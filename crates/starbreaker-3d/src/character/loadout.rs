//! Character loadouts: what a character wears.
//!
//! - [`parse_loadout`]: the engine's item-port loadout XML (CryXmlB,
//!   `Scripts/Loadouts/Character/…`) → [`LoadoutEntry`] trees:
//!   `<Item portName itemName><Items>…</Items></Item>`.
//! - [`resolve_garments`]: the entries worn by a character of a gender. An
//!   entry on the character's own root port is its body (the CHF's wins; an
//!   empty `itemName` means "keep it") and is walked, not worn. Every other
//!   item that has a `.skin` for the gender is a [`Garment`]: geometry from
//!   its gender sub-geometry node, material from its `materialVariants`
//!   resolved against the gender and the loadout's published tags (as head
//!   items), else the node's own. An item without a `.skin` for the gender
//!   that its parent item hangs on a bone port is a [`Prop`] (a backpack on
//!   its core); others are skipped. Ports an item lists in
//!   `HiddenParts` ([`super::clothing`]) drop everything worn under them;
//!   [`Garments::hidden_ports`] reports them for the CHF's own items. A
//!   stand-in core ([`LoadoutEntry::stand_in`], [`super::stand_in`]) is no
//!   garment: it is resolved to its `.skin` and mount bones only, and hides
//!   nothing.
//! - [`worn_geometry`] / [`pick_worn_variant`]: the shared rule for what a
//!   wearable item is worn as (its gendered `.skin` and material);
//!   [`rigid_geometry`] / [`pick_rigid_variant`] the same for props. Single
//!   pieces export through the entity pipeline's `--geometry-tag` instead.

use starbreaker_common::CigGuid;
use starbreaker_cryxml::CryXmlNode;
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::{EntityIndex, SubGeometryVariant};
use starbreaker_datacore::types::Record;
use starbreaker_datacore::material_variants::{item_tags, material_node, published_tags};

use super::CharacterError;
use super::clothing::{Clothing, clothing_params};
use super::props::{PortHelper, Prop};

/// One `<Item>` of a loadout and the items attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutEntry {
    /// The port on the parent item (`Armor_Undersuit`).
    pub port: String,
    /// Entity class name; empty keeps what the character has there.
    pub item: String,
    pub children: Vec<LoadoutEntry>,
    /// The item is a stand-in core ([`super::stand_in`]): worn only for the
    /// mount bone its bone port hangs a prop on; its mesh is not written.
    pub stand_in: bool,
}

/// Parse a loadout XML (CryXmlB) into its top-level entries.
pub fn parse_loadout(bytes: &[u8]) -> Result<Vec<LoadoutEntry>, CharacterError> {
    let xml = starbreaker_cryxml::from_bytes(bytes).map_err(|e| CharacterError::Loadout(format!("CryXmlB decode: {e:?}")))?;
    let root = xml.root();
    if xml.node_tag(root) != "Loadout" {
        return Err(CharacterError::Loadout(format!("root <{}>, expected <Loadout>", xml.node_tag(root))));
    }
    Ok(items(&xml, root))
}

/// The `<Item>`s under `node`'s `<Items>` children.
fn items(xml: &starbreaker_cryxml::CryXml<'_>, node: &CryXmlNode) -> Vec<LoadoutEntry> {
    xml.node_children(node)
        .filter(|n| xml.node_tag(n) == "Items")
        .flat_map(|list| xml.node_children(list).filter(|n| xml.node_tag(n) == "Item"))
        .map(|item| {
            let attr = |name: &str| xml.node_attributes(item).find(|(k, _)| *k == name).map(|(_, v)| v.to_owned()).unwrap_or_default();
            LoadoutEntry { port: attr("portName"), item: attr("itemName"), children: items(xml, item), stand_in: false }
        })
        .collect()
}

/// A worn item resolved for a gender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Garment {
    /// The item record (`EntityClassDefinition`) GUID.
    pub record: CigGuid,
    /// Entity class name.
    pub item: String,
    /// The port it is worn on.
    pub port: String,
    /// `.skin` path of the gender's sub-geometry.
    pub geometry: String,
    /// `.mtl` path.
    pub material: String,
    pub clothing: Clothing,
}

/// The garments of a loadout, its props and the ports they hide.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Garments {
    pub garments: Vec<Garment>,
    /// Rigid items worn on a bone port of their parent item.
    pub props: Vec<Prop>,
    /// Every port a worn item hides (`HiddenParts`).
    pub hidden_ports: Vec<String>,
    /// Stand-in cores ([`LoadoutEntry::stand_in`]): worn for their mount
    /// bones only.
    pub stand_ins: Vec<super::stand_in::StandInCore>,
}

/// The garments `entries` put on a character of `gender` whose root (body)
/// port is `root_port` (see the module docs).
pub fn resolve_garments(db: &Database, entries: &[LoadoutEntry], gender: &str, root_port: &str) -> Garments {
    let index = EntityIndex::new(db);
    // Every named item with its port path (root first) and parent item.
    type Worn<'e> = (Vec<String>, &'e LoadoutEntry, Option<&'e LoadoutEntry>);
    let mut worn: Vec<Worn> = Vec::new();
    fn walk<'e>(entries: &'e [LoadoutEntry], path: &[String], parent: Option<&'e LoadoutEntry>, out: &mut Vec<Worn<'e>>) {
        for entry in entries {
            let mut here = path.to_vec();
            here.push(entry.port.clone());
            out.push((here.clone(), entry, parent));
            walk(&entry.children, &here, Some(entry), out);
        }
    }
    walk(entries, &[], None, &mut worn);
    let records: Vec<_> = worn
        .iter()
        .filter(|(path, e, _)| !e.item.is_empty() && !(path.len() == 1 && e.port.eq_ignore_ascii_case(root_port)))
        .filter_map(|(path, e, parent)| match index.find_record(&e.item) {
            Some(record) => Some((path, *e, *parent, record)),
            None => {
                eprintln!("loadout item {}: no entity record, skipped", e.item);
                None
            }
        })
        .collect();
    // A stand-in core is not worn: its tags select nothing.
    let mut context = vec![gender.to_owned()];
    for (_, _, _, record) in records.iter().filter(|(_, e, _, _)| !e.stand_in) {
        context.extend(published_tags(&item_tags(db, record)));
    }

    let mut all: Vec<(&Vec<String>, Garment)> = Vec::new();
    let mut props: Vec<(&Vec<String>, Prop)> = Vec::new();
    let mut stand_ins = Vec::new();
    for (path, entry, parent, record) in records {
        if entry.stand_in {
            match stand_in_core(db, entry, record, gender) {
                Some(core) => stand_ins.push(core),
                None => eprintln!("stand-in core {}: no {gender} .skin, skipped", entry.item),
            }
            continue;
        }
        // A garment must fit the wearer: the other gender's mesh does not.
        let Some(worn) = worn_geometry(db, record, gender, &context).filter(|w| !w.other_gender) else {
            match worn_prop(db, &index, entry, parent, record, gender, &context) {
                Some(prop) => props.push((path, prop)),
                None => eprintln!("loadout item {}: no {gender} .skin or prop on a bone port, skipped", entry.item),
            }
            continue;
        };
        all.push((
            path,
            Garment {
                record: record.id,
                item: entry.item.clone(),
                port: entry.port.clone(),
                geometry: worn.geometry,
                material: worn.material,
                clothing: clothing_params(db, record),
            },
        ));
    }
    let hidden_ports: Vec<String> = all.iter().flat_map(|(_, g)| g.clothing.hidden_ports.iter().cloned()).collect();
    let shown = |path: &[String], item: &str| {
        let hidden = path.iter().any(|p| hidden_ports.iter().any(|h| h.eq_ignore_ascii_case(p)));
        if hidden {
            eprintln!("loadout item {item}: on a hidden port, skipped");
        }
        !hidden
    };
    let garments = all.into_iter().filter(|(path, g)| shown(path, &g.item)).map(|(_, g)| g).collect();
    let props = props.into_iter().filter(|(path, p)| shown(path, &p.item)).map(|(_, p)| p).collect();
    Garments { garments, props, hidden_ports, stand_ins }
}

/// The stand-in core `entry` resolved for `gender`: its `.skin` (for the
/// mount bones), the bones of the bone ports items are worn on, and those
/// items.
fn stand_in_core(db: &Database, entry: &LoadoutEntry, record: &Record, gender: &str) -> Option<super::stand_in::StandInCore> {
    let geometry = worn_geometry(db, record, gender, &[gender.to_owned()]).filter(|w| !w.other_gender)?.geometry;
    fn bones(helper: &PortHelper, out: &mut Vec<String>) {
        if !out.iter().any(|b| b.eq_ignore_ascii_case(&helper.bone)) {
            out.push(helper.bone.clone());
        }
        helper.sub_helpers.iter().for_each(|sub| bones(sub, out));
    }
    // Only the ports something is worn on: the core's other bone ports
    // (weapon, magazine holsters) carry nothing.
    let mut mount = Vec::new();
    for port in super::wear::ports(db, record).ok()? {
        if !entry.children.iter().any(|c| c.port.eq_ignore_ascii_case(&port.name)) {
            continue;
        }
        if let Some(helper) = &port.bone {
            bones(helper, &mut mount);
        }
    }
    Some(super::stand_in::StandInCore {
        item: entry.item.clone(),
        port: entry.port.clone(),
        geometry,
        bones: mount,
        items: entry.children.iter().map(|c| c.item.clone()).filter(|i| !i.is_empty()).collect(),
    })
}

/// `entry` worn as a rigid prop: its parent item's port it is on hangs it on
/// a bone ([`super::wear`]'s bone ports), it has rigid geometry for the
/// gender ([`rigid_geometry`]), and it takes the port helper its tags select.
/// `None` otherwise (an item on the body itself: the CHF's ports are not
/// looked up here).
fn worn_prop(
    db: &Database,
    index: &EntityIndex,
    entry: &LoadoutEntry,
    parent: Option<&LoadoutEntry>,
    record: &Record,
    gender: &str,
    context: &[String],
) -> Option<Prop> {
    let parent_name = parent?.item.as_str();
    let parent = index.find_record(parent_name)?;
    let ports = super::wear::ports(db, parent).ok()?;
    let bone = ports.into_iter().find(|p| p.name.eq_ignore_ascii_case(&entry.port))?.bone?;
    let geometry = rigid_geometry(index, record, gender, context)?;
    let tags: Vec<String> = item_tags(db, record).split_whitespace().map(str::to_owned).collect();
    let helper = PortHelper { sub_helpers: Vec::new(), ..bone.select(&tags).clone() };
    Some(Prop {
        record: record.id,
        item: entry.item.clone(),
        on: parent_name.to_owned(),
        port: entry.port.clone(),
        helper,
        geometry: geometry.geometry,
        material: geometry.material,
    })
}

/// A wearable item's worn mesh: the gendered `.skin` and its `.mtl` (empty:
/// the `.skin`'s own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WornGeometry {
    pub geometry: String,
    pub material: String,
    /// The mesh is the other gender's: the item has none for the one asked.
    pub other_gender: bool,
}

/// The variant a wearer of `gender` wears: the `.skin` of the node tagged
/// with exactly the gender, else the other gender's, else none.
pub fn pick_worn_variant<'v>(variants: &'v [SubGeometryVariant], gender: &str) -> Option<&'v SubGeometryVariant> {
    let worn = |g: &str| {
        variants.iter().find(|v| v.tag.eq_ignore_ascii_case(g) && v.geometry_path.to_ascii_lowercase().ends_with(".skin"))
    };
    let other = if gender.eq_ignore_ascii_case("female") { "Male" } else { "Female" };
    worn(gender).or_else(|| worn(other))
}

/// The worn mesh of `record` for `gender` ([`pick_worn_variant`]; flagged
/// when it is the other gender's); its material from the item's
/// `materialVariants` resolved against `context` (gender + published tags),
/// else the node's own. `None` for records that are not worn.
pub fn worn_geometry(db: &Database, record: &Record, gender: &str, context: &[String]) -> Option<WornGeometry> {
    let variants = starbreaker_datacore::loadout::sub_geometry_variants(db, record);
    Some(variant_geometry(db, record, pick_worn_variant(&variants, gender)?, gender, context))
}

/// The variant a rigid prop (worn on a bone, not skinned) is worn as by a
/// wearer of `gender`: the non-`.skin` geometry of the node tagged with
/// exactly the gender, else the item's `own` geometry (its top-level node:
/// most backpacks have no gender nodes, and those that do repeat it), else
/// none.
pub fn pick_rigid_variant<'v>(
    variants: &'v [SubGeometryVariant],
    own: Option<&'v SubGeometryVariant>,
    gender: &str,
) -> Option<&'v SubGeometryVariant> {
    let rigid = |v: &&SubGeometryVariant| !v.geometry_path.is_empty() && !v.geometry_path.to_ascii_lowercase().ends_with(".skin");
    variants.iter().filter(rigid).find(|v| v.tag.eq_ignore_ascii_case(gender)).or(own.filter(rigid))
}

/// The mesh of `record` worn as a rigid prop by `gender`
/// ([`pick_rigid_variant`]), its material as [`worn_geometry`]'s. `None` for
/// records with no rigid geometry.
pub fn rigid_geometry(index: &EntityIndex, record: &Record, gender: &str, context: &[String]) -> Option<WornGeometry> {
    let db = index.db;
    let variants = starbreaker_datacore::loadout::sub_geometry_variants(db, record);
    let (geometry, material) = index.query_geometry_paths(record);
    let own = geometry.map(|geometry_path| SubGeometryVariant {
        tag: gender.to_owned(),
        geometry_path,
        material_path: material.unwrap_or_default(),
        deformer_type: String::new(),
    });
    Some(variant_geometry(db, record, pick_rigid_variant(&variants, own.as_ref(), gender)?, gender, context))
}

/// `variant`'s geometry with the item's `materialVariants` material resolved
/// against `context`, else the node's own.
fn variant_geometry(db: &Database, record: &Record, variant: &SubGeometryVariant, gender: &str, context: &[String]) -> WornGeometry {
    let material = material_node(db, record)
        .and_then(|node| node.resolve(context).map(str::to_owned))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| variant.material_path.clone());
    WornGeometry {
        geometry: variant.geometry_path.clone(),
        material,
        other_gender: !variant.tag.eq_ignore_ascii_case(gender),
    }
}

/// The tint palette `record` is worn with by a wearer of `gender`: its
/// `gender` sub-geometry node's (where armour pieces keep theirs: their
/// top-level geometry is the storage crate), else its default — the same
/// rule as `entity export --geometry-tag`.
pub fn worn_palette(db: &Database, record: &Record, gender: &str) -> Option<crate::mtl::TintPalette> {
    crate::pipeline::query_geometry_variant_palette(db, record, gender)
        .or_else(|| crate::pipeline::query_tint_palette(db, record))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(tag: &str, geometry: &str) -> SubGeometryVariant {
        SubGeometryVariant { tag: tag.into(), geometry_path: geometry.into(), material_path: String::new(), deformer_type: String::new() }
    }

    /// The worn mesh is the `.skin` of the node tagged with exactly the
    /// gender; deeper nodes (skin tones, stored props) are not it.
    #[test]
    fn worn_variant_is_the_exact_gender_skin() {
        let variants = [
            v("heldentity", "crate.cgf"),
            v("male inventorystoredentity", "m_arms_prop.cgf"),
            v("male", "m_arms.skin"),
            v("male body cau", "m_other.skin"),
            v("female", "f_arms.skin"),
        ];
        assert_eq!(pick_worn_variant(&variants, "Male").unwrap().geometry_path, "m_arms.skin");
        assert_eq!(pick_worn_variant(&variants, "female").unwrap().geometry_path, "f_arms.skin");
    }

    /// No worn mesh for the gender: the other gender's, so a female-only
    /// garment still exports as itself.
    #[test]
    fn worn_variant_falls_back_to_the_other_gender() {
        let variants = [v("heldentity", "crate.cgf"), v("female", "f_top.skin")];
        assert_eq!(pick_worn_variant(&variants, "Male").unwrap().geometry_path, "f_top.skin");
    }

    /// A prop is worn as the gender node's non-`.skin` geometry (a
    /// backpack's `.cga`), not as a `.skin` and not as a nested stored node;
    /// an item with no gender node is worn as its own geometry (most
    /// backpacks have none).
    #[test]
    fn rigid_variant_is_the_gender_nodes_else_the_items_own() {
        let variants = [
            v("heldentity", "pack.cga"),
            v("female", "f_pack.cga"),
            v("female inventorystoredentity", "f_stored.cga"),
            v("male", "m_pack.cga"),
        ];
        let own = v("", "pack.cga");
        assert_eq!(pick_rigid_variant(&variants, Some(&own), "Male").unwrap().geometry_path, "m_pack.cga");
        assert_eq!(pick_rigid_variant(&variants, Some(&own), "Female").unwrap().geometry_path, "f_pack.cga");
        assert_eq!(pick_rigid_variant(&[v("heldentity", "held.cga")], Some(&own), "Male").unwrap().geometry_path, "pack.cga");
        assert!(pick_rigid_variant(&[v("male", "m_arms.skin")], None, "Male").is_none(), "a garment is not a prop");
        assert!(pick_rigid_variant(&[], Some(&v("", "arms.skin")), "Male").is_none(), "nor is a .skin of its own");
        assert!(pick_rigid_variant(&[], None, "Male").is_none());
    }

    #[test]
    fn non_wearables_have_no_worn_variant() {
        assert!(pick_worn_variant(&[v("front left", "a.cga")], "Male").is_none());
        assert!(pick_worn_variant(&[v("male", "m_prop.cgf")], "Male").is_none(), "a gender node without a .skin");
        assert!(pick_worn_variant(&[], "Male").is_none());
    }
}
