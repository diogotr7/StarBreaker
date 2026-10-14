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
//!   items), else the node's own. Items without a `.skin` for the gender
//!   (bone-attached props) are skipped. Ports an item lists in
//!   `HiddenParts` ([`super::clothing`]) drop everything worn under them;
//!   [`Garments::hidden_ports`] reports them for the CHF's own items.
//! - [`worn_geometry`] / [`pick_worn_variant`]: the shared rule for what a
//!   wearable item is worn as (its gendered `.skin` and material). Single
//!   pieces export through the entity pipeline's `--geometry-tag` instead.

use starbreaker_common::CigGuid;
use starbreaker_cryxml::CryXmlNode;
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::{EntityIndex, SubGeometryVariant};
use starbreaker_datacore::types::Record;
use starbreaker_datacore::material_variants::{item_tags, material_node, published_tags};

use super::CharacterError;
use super::clothing::{Clothing, clothing_params};

/// One `<Item>` of a loadout and the items attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutEntry {
    /// The port on the parent item (`Armor_Undersuit`).
    pub port: String,
    /// Entity class name; empty keeps what the character has there.
    pub item: String,
    pub children: Vec<LoadoutEntry>,
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
            LoadoutEntry { port: attr("portName"), item: attr("itemName"), children: items(xml, item) }
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

/// The garments of a loadout and the ports they hide.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Garments {
    pub garments: Vec<Garment>,
    /// Every port a worn item hides (`HiddenParts`).
    pub hidden_ports: Vec<String>,
}

/// The garments `entries` put on a character of `gender` whose root (body)
/// port is `root_port` (see the module docs).
pub fn resolve_garments(db: &Database, entries: &[LoadoutEntry], gender: &str, root_port: &str) -> Garments {
    let index = EntityIndex::new(db);
    // Every named item with its port path (root first).
    let mut worn: Vec<(Vec<String>, &LoadoutEntry)> = Vec::new();
    fn walk<'e>(entries: &'e [LoadoutEntry], path: &[String], out: &mut Vec<(Vec<String>, &'e LoadoutEntry)>) {
        for entry in entries {
            let mut here = path.to_vec();
            here.push(entry.port.clone());
            out.push((here.clone(), entry));
            walk(&entry.children, &here, out);
        }
    }
    walk(entries, &[], &mut worn);
    let records: Vec<_> = worn
        .iter()
        .filter(|(path, e)| !e.item.is_empty() && !(path.len() == 1 && e.port.eq_ignore_ascii_case(root_port)))
        .filter_map(|(path, e)| match index.find_record(&e.item) {
            Some(record) => Some((path, *e, record)),
            None => {
                eprintln!("loadout item {}: no entity record, skipped", e.item);
                None
            }
        })
        .collect();
    let mut context = vec![gender.to_owned()];
    for (_, _, record) in &records {
        context.extend(published_tags(&item_tags(db, record)));
    }

    let mut all: Vec<(&Vec<String>, Garment)> = Vec::new();
    for (path, entry, record) in records {
        // A garment must fit the wearer: the other gender's mesh does not.
        let Some(worn) = worn_geometry(db, record, gender, &context).filter(|w| !w.other_gender) else {
            eprintln!("loadout item {}: no {gender} .skin, skipped", entry.item);
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
    let garments = all
        .into_iter()
        .filter(|(path, g)| {
            let hidden = path.iter().any(|p| hidden_ports.iter().any(|h| h.eq_ignore_ascii_case(p)));
            if hidden {
                eprintln!("loadout item {}: on a hidden port, skipped", g.item);
            }
            !hidden
        })
        .map(|(_, g)| g)
        .collect();
    Garments { garments, hidden_ports }
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
    let variant = pick_worn_variant(&variants, gender)?;
    let material = material_node(db, record)
        .and_then(|node| node.resolve(context).map(str::to_owned))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| variant.material_path.clone());
    Some(WornGeometry {
        geometry: variant.geometry_path.clone(),
        material,
        other_gender: !variant.tag.eq_ignore_ascii_case(gender),
    })
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

    #[test]
    fn non_wearables_have_no_worn_variant() {
        assert!(pick_worn_variant(&[v("front left", "a.cga")], "Male").is_none());
        assert!(pick_worn_variant(&[v("male", "m_prop.cgf")], "Male").is_none(), "a gender node without a .skin");
        assert!(pick_worn_variant(&[], "Male").is_none());
    }
}
