//! Dressing a character in hand-picked items, the way the engine equips
//! them: each item goes on a free port that accepts it ([`PortDef::accepts`]:
//! its `AttachDef.Type` and subtype, and the tags the port requires),
//! searched over the body's ports and those of the items already worn
//! (`Char_Armor_Arms` → an undersuit's `Armor_Arms`, `Char_Clothing_Torso_0`
//! → the body's `Clothing_Torso_0`). Only ports that skin their item to the
//! character are worn on; hands and pockets accept armour too, to carry it.
//! The result is the [`LoadoutEntry`] tree
//! a loadout XML would give ([`super::loadout`]), so `HiddenParts` and the
//! clothing layers apply to picked items too.
//!
//! Bone ports flagged `body_attach` wear rigid props (a backpack on its core),
//! which go on no skin port, as garments go on no bone port.
//!
//! - [`place_items`]: the placement (pure) onto what is already worn
//!   ([`Seed`]); [`wearable`]: an item's type and ports from DataCore;
//!   [`dress_entries`]: picked item names on a CHF character (its own items —
//!   body, head — and a loadout's are the worn items); a pick the wearer's
//!   gender has no mesh for is reported, not worn; [`dress`] the same with
//!   another placement ([`super::stand_in`]: a backpack with no core goes on
//!   a stand-in core).

use starbreaker_chf::{ChfData, ItemPort};
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;

use super::CharacterError;
use super::loadout::{LoadoutEntry, rigid_geometry, worn_geometry};
use super::props::PortHelper;

/// An item port: its name, the item types it accepts, the tags an item
/// must have to go on it (`RequiredPortTags`) and, for a port that hangs its
/// item on a bone, the bone's helper ([`super::props`]); `None`: the port
/// skins its item to the character.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortDef {
    pub name: String,
    pub types: Vec<PortType>,
    pub required_tags: Vec<String>,
    pub bone: Option<PortHelper>,
    /// `MaxSize`: the largest item size the port takes (not a wear gate,
    /// see [`PortDef::accepts`]; it picks a stand-in core's weight,
    /// [`super::stand_in`]).
    pub max_size: Option<i32>,
}

/// An item type a port accepts; `sub_types` empty accepts any subtype.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortType {
    pub name: String,
    pub sub_types: Vec<String>,
}

/// An item to wear: its entity class name, `AttachDef` type, subtype and
/// tags, its own ports, and whether it is a rigid prop (worn on a bone, not
/// skinned to the character).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Wearable {
    pub item: String,
    pub item_type: String,
    pub sub_type: String,
    pub tags: Vec<String>,
    pub ports: Vec<PortDef>,
    pub rigid: bool,
    /// `AttachDef.Size`.
    pub size: Option<i32>,
}

impl PortDef {
    /// Whether `item` fits this port: one of its types (any case) with a
    /// matching subtype when it lists any, every tag the port requires on the
    /// item (a required `$tag` is the item's `tag`), and the port's
    /// attachment the item's kind: rigid props hang on bones, garments are
    /// skinned.
    ///
    /// Not checked: `MinSize` / `MaxSize`, which do not gate wear (the
    /// undersuit's necksock port is size 0..0 and takes a size-1 necksock),
    /// and the item's own `RequiredTags`, which the engine matches against
    /// tags from up the hierarchy, not just the port's (`ctl_hat_01_01_01`
    /// requires `Protos`, which neither the head's hat port nor the head has).
    pub fn accepts(&self, item: &Wearable) -> bool {
        self.types.iter().any(|t| {
            t.name.eq_ignore_ascii_case(&item.item_type)
                && (t.sub_types.is_empty() || t.sub_types.iter().any(|s| s.eq_ignore_ascii_case(&item.sub_type)))
        }) && self.required_tags.iter().all(|tag| {
            let tag = tag.strip_prefix('$').unwrap_or(tag);
            item.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
        }) && self.bone.is_some() == item.rigid
    }
}

/// Where the items went: one root entry (the body's port, item kept) with
/// the worn items nested under it, and `"<item>: <reason>"` for each item
/// that could not be worn.
#[derive(Debug, Default, PartialEq)]
pub struct Placement {
    pub entries: Vec<LoadoutEntry>,
    pub unplaced: Vec<String>,
    /// The ports left free, in tree order.
    pub free_ports: Vec<PortDef>,
    /// Items worn on a stand-in core ([`super::stand_in`]).
    pub stand_ins: Vec<super::stand_in::StandIn>,
}

/// What a character already wears, as a tree of items with their ports:
/// the CHF's own items (`item` empty: kept as they are) and a loadout's.
#[derive(Debug, Clone, PartialEq)]
pub struct Seed {
    /// The port on the parent item.
    pub port: String,
    /// Entity class name; empty keeps what the character has there.
    pub item: String,
    /// The item's wear ports.
    pub ports: Vec<PortDef>,
    pub children: Vec<Seed>,
}

/// Place `items` on what `worn` already wears and on each other: a worn
/// item's port is free unless one of its children fills it (port names
/// match in any case); repeatedly, every unplaced item takes the first free
/// port (in tree order) that accepts its type, until no item moves.
pub fn place_items(worn: &Seed, items: &[Wearable]) -> Placement {
    struct Node {
        port: String,
        item: String,
        parent: Option<usize>,
    }
    struct Slot<'p> {
        owner: usize,
        port: &'p PortDef,
        taken: bool,
    }
    fn seed<'s>(s: &'s Seed, parent: Option<usize>, nodes: &mut Vec<Node>, slots: &mut Vec<Slot<'s>>) {
        nodes.push(Node { port: s.port.clone(), item: s.item.clone(), parent });
        let id = nodes.len() - 1;
        for port in &s.ports {
            let taken = s.children.iter().any(|c| c.port.eq_ignore_ascii_case(&port.name));
            slots.push(Slot { owner: id, port, taken });
        }
        for child in &s.children {
            seed(child, Some(id), nodes, slots);
        }
    }
    let mut nodes = Vec::new();
    let mut slots: Vec<Slot> = Vec::new();
    seed(worn, None, &mut nodes, &mut slots);
    let mut pending: Vec<&Wearable> = items.iter().collect();
    loop {
        let mut placed = None;
        for (i, w) in pending.iter().enumerate() {
            let accepts = |s: &Slot| !s.taken && s.port.accepts(w);
            if let Some(slot) = slots.iter().position(accepts) {
                placed = Some((i, slot));
                break;
            }
        }
        let Some((i, slot)) = placed else { break };
        let w = pending.remove(i);
        slots[slot].taken = true;
        nodes.push(Node { port: slots[slot].port.name.clone(), item: w.item.clone(), parent: Some(slots[slot].owner) });
        let owner = nodes.len() - 1;
        slots.extend(w.ports.iter().map(|port| Slot { owner, port, taken: false }));
    }
    fn build(nodes: &[Node], id: usize) -> LoadoutEntry {
        LoadoutEntry {
            port: nodes[id].port.clone(),
            item: nodes[id].item.clone(),
            children: (0..nodes.len()).filter(|&c| nodes[c].parent == Some(id)).map(|c| build(nodes, c)).collect(),
            stand_in: false,
        }
    }
    Placement {
        entries: vec![build(&nodes, 0)],
        unplaced: pending.iter().map(|w| format!("{}: no free port accepts {}", w.item, w.item_type)).collect(),
        free_ports: slots.iter().filter(|s| !s.taken).map(|s| s.port.clone()).collect(),
        stand_ins: Vec::new(),
    }
}

/// An item's `AttachDef` (type, subtype, tags) and its item
/// ports, by exact entity class name (any case); `None` for unknown names or
/// items without an attach type.
pub fn wearable(db: &Database, index: &EntityIndex, name: &str) -> Result<Option<Wearable>, CharacterError> {
    let Some(record) = index.find_record(name) else { return Ok(None) };
    let attach = |field: &str| first_text(db, record, &format!("Components[SAttachableComponentParams].AttachDef.{field}"));
    let Some(item_type) = attach("Type") else { return Ok(None) };
    Ok(Some(Wearable {
        item: name.to_owned(),
        item_type,
        sub_type: attach("SubType").unwrap_or_default(),
        tags: words(attach("Tags").as_deref()),
        ports: ports(db, record)?,
        rigid: false,
        size: first_int(db, record, "Components[SAttachableComponentParams].AttachDef.Size"),
    }))
}

/// The ports of `record` that items are worn on
/// (`SItemPortContainerComponentParams.Ports`): those whose attachment is a
/// skin (the item is skinned to the character), and bone ports flagged
/// `body_attach` (the item hangs on a bone of the body: a core's backpack
/// port), with their helper. Carry ports (hands attach to a bone but not to
/// the body, pockets hold an entity) accept armour too, but nothing is worn
/// there. A record type without item ports has none.
pub(crate) fn ports(db: &Database, record: &Record) -> Result<Vec<PortDef>, CharacterError> {
    let Ok(path) = db.compile_path::<Value>(record.struct_id(), "Components[SItemPortContainerComponentParams].Ports") else {
        return Ok(Vec::new());
    };
    // Ports are read from their inline fields: expanding the records they
    // reference (default items, tags) can exceed the materialization limit.
    let values = db
        .query_no_references(&path, record)
        .map_err(|e| CharacterError::Loadout(format!("item ports of {}: {e}", record.id)))?;
    Ok(values
        .iter()
        .flat_map(|ports| match ports {
            Value::Array(items) => items.iter().collect::<Vec<_>>(),
            other => vec![other],
        })
        .filter_map(|port| {
            let Some(Value::Object { type_name, .. }) = field(port, "AttachmentImplementation") else { return None };
            let bone = match *type_name {
                SKIN_ATTACHMENT => None,
                BONE_ATTACHMENT if words(field(port, "Flags").and_then(text).as_deref()).iter().any(|f| f.eq_ignore_ascii_case("body_attach")) => {
                    Some(port_helper(field(field(port, "AttachmentImplementation")?, "Helper")?)?)
                }
                _ => return None,
            };
            let name = text(field(port, "Name")?)?;
            let types = match field(port, "Types") {
                Some(Value::Array(types)) => types
                    .iter()
                    .filter_map(|t| {
                        let sub_types = match field(t, "SubTypes") {
                            Some(Value::Array(subs)) => subs.iter().filter_map(text).collect(),
                            _ => Vec::new(),
                        };
                        Some(PortType { name: text(field(t, "Type")?)?, sub_types })
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let required_tags = words(field(port, "RequiredPortTags").and_then(text).as_deref());
            let max_size = field(port, "MaxSize").and_then(int);
            Some(PortDef { name, types, required_tags, bone, max_size })
        })
        .collect())
}

/// A DataCore tag string's tags (space-separated).
fn words(s: Option<&str>) -> Vec<String> {
    s.map(|s| s.split_whitespace().map(str::to_owned).collect()).unwrap_or_default()
}

/// The port attachment that skins the attached item to the character.
const SKIN_ATTACHMENT: &str = "SItemPortDefAttachmentImplementationSkin";
/// The port attachment that hangs the attached item on a bone.
const BONE_ATTACHMENT: &str = "SItemPortDefAttachmentImplementationBone";

/// A bone port's helper tree (`SItemPortDefHelperNode`: `Tags`, `Helper`,
/// `SubHelpers`); `None` when it names no bone.
fn port_helper(node: &Value) -> Option<PortHelper> {
    let helper = field(node, "Helper")?;
    let offset = field(helper, "Offset");
    let vec3 = |name: &str| {
        let v = offset.and_then(|o| field(o, name));
        ["x", "y", "z"].map(|axis| match v.and_then(|v| field(v, axis)) {
            Some(Value::Float(f)) => *f,
            Some(Value::Double(f)) => *f as f32,
            _ => 0.0,
        })
    };
    Some(PortHelper {
        tags: words(field(node, "Tags").and_then(text).as_deref()),
        bone: text(field(helper, "Name")?)?,
        position: vec3("Position"),
        rotation: vec3("Rotation"),
        item_helper: field(helper, "ItemOffsetHelperName").and_then(text).unwrap_or_default(),
        sub_helpers: match field(node, "SubHelpers") {
            Some(Value::Array(subs)) => subs.iter().filter_map(port_helper).collect(),
            _ => Vec::new(),
        },
    })
}

fn first_text(db: &Database, record: &Record, path: &str) -> Option<String> {
    let compiled = db.compile_path::<Value>(record.struct_id(), path).ok()?;
    db.query::<Value>(&compiled, record).ok()?.iter().find_map(text)
}

fn first_int(db: &Database, record: &Record, path: &str) -> Option<i32> {
    let compiled = db.compile_path::<Value>(record.struct_id(), path).ok()?;
    db.query::<Value>(&compiled, record).ok()?.iter().find_map(int)
}

fn int(v: &Value) -> Option<i32> {
    match *v {
        Value::Int8(i) => Some(i.into()),
        Value::Int16(i) => Some(i.into()),
        Value::Int32(i) => Some(i),
        Value::UInt8(i) => Some(i.into()),
        Value::UInt16(i) => Some(i.into()),
        Value::UInt32(i) => i32::try_from(i).ok(),
        _ => None,
    }
}

pub(crate) fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) | Value::Enum(s) if !s.is_empty() => Some((*s).to_owned()),
        _ => None,
    }
}

fn field<'v, 'a>(v: &'v Value<'a>, name: &str) -> Option<&'v Value<'a>> {
    match v {
        Value::Object { fields, .. } => fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v),
        _ => None,
    }
}

/// `items` (exact entity class names) put on the CHF character dressed in
/// `loadout`: they go on the ports of the character's own items (its body,
/// its head's hat and eyewear ports) and of the loadout's, as the engine
/// equips them. The result is the whole tree (CHF items kept as they are,
/// the loadout's and the placed items by name). Unknown names, and items
/// with no mesh for the character's gender, are reported in `unplaced`.
/// Errors when DataCore lacks the CHF's body record or a query fails.
pub fn dress_entries(db: &Database, chf: &ChfData, loadout: &[LoadoutEntry], items: &[String]) -> Result<Placement, CharacterError> {
    dress(db, chf, loadout, items, |_, worn, wearables| Ok(place_items(worn, wearables)))
}

/// [`dress_entries`] with the placement `place` makes of the wearables on
/// the worn tree (given the entity index).
pub(crate) fn dress(
    db: &Database,
    chf: &ChfData,
    loadout: &[LoadoutEntry],
    items: &[String],
    place: impl FnOnce(&EntityIndex, &Seed, &[Wearable]) -> Result<Placement, CharacterError>,
) -> Result<Placement, CharacterError> {
    let index = EntityIndex::new(db);
    let gender = super::gender_tag(chf.dna.gender_hash);
    if db.record_by_id(&chf.itemport.id).is_none() {
        return Err(CharacterError::Loadout(format!("the CHF's body record {} is not in DataCore", chf.itemport.id)));
    }
    let mut worn = chf_seed(db, &chf.itemport)?;
    let mut other_roots = Vec::new();
    for entry in loadout {
        if entry.port.eq_ignore_ascii_case(&worn.port) {
            merge_entries(db, &index, &mut worn, &entry.children)?;
        } else {
            other_roots.push(entry.clone());
        }
    }
    let mut wearables = Vec::new();
    let mut unwearable = Vec::new();
    for name in items {
        match wearable(db, &index, name)? {
            None => unwearable.push(format!("{name}: no such wearable item")),
            Some(w) => {
                let record = index.find_record(name).expect("wearable found it");
                let context = [gender.to_owned()];
                let fits = |g: Option<super::loadout::WornGeometry>| g.is_some_and(|g| !g.other_gender);
                if fits(worn_geometry(db, record, gender, &context)) {
                    wearables.push(w);
                } else if rigid_geometry(&index, record, gender, &context).is_some() {
                    wearables.push(Wearable { rigid: true, ..w });
                } else {
                    unwearable.push(format!("{name}: no {gender} mesh"));
                }
            }
        }
    }
    let mut placement = place(&index, &worn, &wearables)?;
    placement.entries.extend(other_roots);
    placement.unplaced.extend(unwearable);
    Ok(placement)
}

/// The CHF's item tree as worn items (kept: `item` empty) with their ports;
/// a port with no item has none.
fn chf_seed(db: &Database, port: &ItemPort) -> Result<Seed, CharacterError> {
    Ok(Seed {
        port: port.name.to_string(),
        item: String::new(),
        ports: match db.record_by_id(&port.id) {
            Some(record) => ports(db, record)?,
            None => Vec::new(),
        },
        children: port.children.iter().map(|c| chf_seed(db, c)).collect::<Result<_, _>>()?,
    })
}

/// Lay a loadout's `entries` over `worn`: an entry on a port something is
/// worn on replaces it (a named item, with its own ports and nothing under
/// it yet) or keeps it (empty name); other entries are added.
fn merge_entries(db: &Database, index: &EntityIndex, worn: &mut Seed, entries: &[LoadoutEntry]) -> Result<(), CharacterError> {
    for entry in entries {
        let slot = match worn.children.iter().position(|c| c.port.eq_ignore_ascii_case(&entry.port)) {
            Some(i) => i,
            None => {
                worn.children.push(Seed { port: entry.port.clone(), item: String::new(), ports: Vec::new(), children: Vec::new() });
                worn.children.len() - 1
            }
        };
        let child = &mut worn.children[slot];
        if !entry.item.is_empty() {
            child.item = entry.item.clone();
            // An item DataCore lacks is reported when the outfit is built.
            child.ports = match index.find_record(&entry.item) {
                Some(record) => ports(db, record)?,
                None => Vec::new(),
            };
            child.children.clear();
        }
        merge_entries(db, index, child, &entry.children)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, ty: &str) -> PortDef {
        PortDef { name: name.into(), types: vec![PortType { name: ty.into(), sub_types: Vec::new() }], ..Default::default() }
    }
    fn item(name: &str, ty: &str, ports: Vec<PortDef>) -> Wearable {
        Wearable { item: name.into(), item_type: ty.into(), ports, ..Default::default() }
    }
    fn body_ports() -> Vec<PortDef> {
        vec![port("Armor_Undersuit", "Char_Armor_Undersuit"), port("Clothing_Torso_0", "Char_Clothing_Torso_0")]
    }
    fn suit() -> Wearable {
        item("suit", "Char_Armor_Undersuit", vec![port("Armor_Arms", "Char_Armor_Arms"), port("Armor_Helmet", "Char_Armor_Helmet")])
    }
    /// The body, nothing worn yet.
    fn body(port: &str, ports: Vec<PortDef>) -> Seed {
        Seed { port: port.into(), item: String::new(), ports, children: Vec::new() }
    }
    /// The body already wearing the undersuit (from a loadout).
    fn body_in_suit() -> Seed {
        let suit = suit();
        let worn = Seed { port: "Armor_Undersuit".into(), item: suit.item, ports: suit.ports, children: Vec::new() };
        Seed { children: vec![worn], ..body("body_itemport", body_ports()) }
    }

    /// Picked items go on the ports of items already worn (a loadout's
    /// undersuit); the worn items stay in the tree as they are.
    #[test]
    fn items_go_on_ports_of_worn_items() {
        let p = place_items(&body_in_suit(), &[item("arms", "Char_Armor_Arms", vec![])]);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        let suit = &p.entries[0].children[0];
        assert_eq!((suit.port.as_str(), suit.item.as_str()), ("Armor_Undersuit", "suit"));
        assert_eq!(suit.children[0].item, "arms");
    }

    /// A port a worn item already fills is taken (case-insensitive name).
    #[test]
    fn ports_worn_items_fill_are_taken() {
        let mut seed = body_in_suit();
        seed.children[0].children.push(Seed { port: "armor_helmet".into(), item: "h1".into(), ports: Vec::new(), children: Vec::new() });
        let p = place_items(&seed, &[item("h2", "Char_Armor_Helmet", vec![])]);
        assert_eq!(p.unplaced.len(), 1, "{:?}", p.unplaced);
        assert_eq!(p.entries[0].children[0].children.len(), 1);
    }

    /// Items go on the port that accepts their type, nested under the item
    /// that has the port, whatever order they are listed in.
    #[test]
    fn items_nest_on_accepting_ports_in_any_order() {
        let p = place_items(&body("body_itemport", body_ports()), &[item("arms", "Char_Armor_Arms", vec![]), suit()]);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        let root = &p.entries[0];
        assert_eq!((root.port.as_str(), root.item.as_str()), ("body_itemport", ""));
        let suit = &root.children[0];
        assert_eq!((suit.port.as_str(), suit.item.as_str()), ("Armor_Undersuit", "suit"));
        assert_eq!((suit.children[0].port.as_str(), suit.children[0].item.as_str()), ("Armor_Arms", "arms"));
    }

    /// One item per port: the second helmet is reported, not worn.
    #[test]
    fn a_taken_port_reports_the_next_item() {
        let p = place_items(&body("body", body_ports()), &[suit(), item("h1", "Char_Armor_Helmet", vec![]), item("h2", "Char_Armor_Helmet", vec![])]);
        assert_eq!(p.entries[0].children[0].children.len(), 1);
        assert_eq!(p.unplaced.len(), 1);
        assert!(p.unplaced[0].starts_with("h2:"), "{:?}", p.unplaced);
    }

    #[test]
    fn items_no_port_accepts_are_reported() {
        let p = place_items(&body("body", body_ports()), &[item("arms", "Char_Armor_Arms", vec![])]);
        assert!(p.entries[0].children.is_empty());
        assert!(p.unplaced[0].contains("Char_Armor_Arms"), "{:?}", p.unplaced);
    }

    /// A port's `RequiredPortTags` must all be on the item (any case): the
    /// undersuit's necksock port takes the undersuit necksock, not the
    /// helmet's.
    #[test]
    fn ports_take_only_items_with_their_required_tags() {
        let necksock_port = PortDef { required_tags: vec!["necksock_undersuit".into()], ..port("universal_necksock_undersuit", "Char_Flair") };
        let sock = |name: &str, tags: &[&str]| Wearable { tags: tags.iter().map(|t| t.to_string()).collect(), ..item(name, "Char_Flair", vec![]) };
        let p = place_items(
            &body("body", vec![necksock_port]),
            &[sock("helmet_sock", &["necksock", "uneditable"]), sock("suit_sock", &["Necksock_Undersuit", "uneditable"])],
        );
        assert_eq!(p.entries[0].children.iter().map(|c| c.item.as_str()).collect::<Vec<_>>(), ["suit_sock"]);
        assert!(p.unplaced[0].starts_with("helmet_sock:"), "{:?}", p.unplaced);
    }

    /// A port listing subtypes takes only items of one of them; one listing
    /// none takes any.
    #[test]
    fn port_subtypes_restrict_items() {
        let typed = PortDef { types: vec![PortType { name: "Char_Flair".into(), sub_types: vec!["Badge".into()] }], ..port("flair", "") };
        let of = |sub: &str| Wearable { sub_type: sub.into(), ..item(sub, "Char_Flair", vec![]) };
        let p = place_items(&body("body", vec![typed]), &[of("Patch"), of("badge")]);
        assert_eq!(p.entries[0].children[0].item, "badge");
        assert_eq!(p.unplaced.len(), 1);
        let p = place_items(&body("body", vec![port("flair", "Char_Flair")]), &[of("Patch")]);
        assert!(p.unplaced.is_empty());
    }

    #[test]
    fn port_types_match_case_insensitively() {
        let p = place_items(&body("body", vec![port("Clothing_Torso_0", "char_clothing_torso_0")]), &[item("shirt", "Char_Clothing_Torso_0", vec![])]);
        assert!(p.unplaced.is_empty());
    }

    /// A port's required `$tag` is the item's `tag`: the core's backpack port
    /// requires `$backpack`, the backpack carries `backpack`.
    #[test]
    fn dollar_required_tags_match_plain_item_tags() {
        let bp_port = PortDef { required_tags: vec!["$backpack".into()], bone: Some(Default::default()), ..port("backpack", "Char_Armor_Backpack") };
        let bp = Wearable { tags: vec!["Backpack".into()], rigid: true, ..item("bp", "Char_Armor_Backpack", vec![]) };
        let p = place_items(&body("body", vec![bp_port]), &[bp]);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
    }

    /// Rigid props go on bone ports only and skinned garments on skin ports
    /// only: a backpack is not skinned to the body, an undersuit is not hung
    /// on a bone.
    #[test]
    fn rigid_items_go_on_bone_ports_and_skinned_items_on_skin_ports() {
        let bone_port = PortDef { bone: Some(Default::default()), ..port("backpack", "Char_Armor_Backpack") };
        let skin_port = port("backpack_skin", "Char_Armor_Backpack");
        let rigid = Wearable { rigid: true, ..item("bp", "Char_Armor_Backpack", vec![]) };
        let skinned = item("bp_skin", "Char_Armor_Backpack", vec![]);
        let p = place_items(&body("body", vec![skin_port, bone_port]), &[rigid, skinned]);
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        let on = |port: &str| p.entries[0].children.iter().find(|c| c.port == port).map(|c| c.item.as_str());
        assert_eq!(on("backpack"), Some("bp"));
        assert_eq!(on("backpack_skin"), Some("bp_skin"));
    }
}
