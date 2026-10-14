//! Dressing a character in hand-picked items, the way the engine equips
//! them: each item goes on a free port that accepts its `AttachDef.Type`,
//! searched over the body's ports and those of the items already worn
//! (`Char_Armor_Arms` → an undersuit's `Armor_Arms`, `Char_Clothing_Torso_0`
//! → the body's `Clothing_Torso_0`). Only ports that skin their item to the
//! character are worn on; hands and pockets accept armour too, to carry it.
//! The result is the [`LoadoutEntry`] tree
//! a loadout XML would give ([`super::loadout`]), so `HiddenParts` and the
//! clothing layers apply to picked items too.
//!
//! - [`place_items`]: the placement (pure) onto what is already worn
//!   ([`Seed`]); [`wearable`]: an item's type and ports from DataCore;
//!   [`dress_entries`]: picked item names on a CHF character (its own items —
//!   body, head — and a loadout's are the worn items).

use starbreaker_chf::{ChfData, ItemPort};
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;

use super::loadout::LoadoutEntry;

/// An item port: its name and the item types it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDef {
    pub name: String,
    pub types: Vec<String>,
}

/// An item to wear: its entity class name, `AttachDef.Type` and own ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wearable {
    pub item: String,
    pub item_type: String,
    pub ports: Vec<PortDef>,
}

/// Where the items went: one root entry (the body's port, item kept) with
/// the worn items nested under it, and `"<item>: <reason>"` for each item
/// that could not be worn.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub entries: Vec<LoadoutEntry>,
    pub unplaced: Vec<String>,
}

/// What a character already wears, as a tree of items with their ports:
/// the CHF's own items (`item` empty: kept as they are) and a loadout's.
#[derive(Debug, Clone, PartialEq, Eq)]
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
            let accepts = |s: &Slot| !s.taken && s.port.types.iter().any(|t| t.eq_ignore_ascii_case(&w.item_type));
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
        }
    }
    Placement {
        entries: vec![build(&nodes, 0)],
        unplaced: pending.iter().map(|w| format!("{}: no free port accepts {}", w.item, w.item_type)).collect(),
    }
}

/// An item's `AttachDef.Type` and its item ports, by exact entity class
/// name (any case); `None` for unknown names or items without an attach type.
pub fn wearable(db: &Database, index: &EntityIndex, name: &str) -> Option<Wearable> {
    let record = index.find_record(name)?;
    let item_type = first_text(db, record, "Components[SAttachableComponentParams].AttachDef.Type")?;
    Some(Wearable { item: name.to_owned(), item_type, ports: ports(db, record) })
}

/// The ports of `record` that items are worn on
/// (`SItemPortContainerComponentParams.Ports` whose attachment is a skin:
/// the item is skinned to the character). Carry ports (hands attach to a
/// bone, pockets hold an entity) accept armour too, but nothing is worn
/// there.
fn ports(db: &Database, record: &Record) -> Vec<PortDef> {
    let Ok(path) = db.compile_path::<Value>(record.struct_id(), "Components[SItemPortContainerComponentParams].Ports") else {
        return Vec::new();
    };
    let values = db.query::<Value>(&path, record).unwrap_or_default();
    values
        .iter()
        .flat_map(|ports| match ports {
            Value::Array(items) => items.iter().collect::<Vec<_>>(),
            other => vec![other],
        })
        .filter(|port| {
            matches!(field(port, "AttachmentImplementation"), Some(Value::Object { type_name, .. }) if *type_name == SKIN_ATTACHMENT)
        })
        .filter_map(|port| {
            let name = text(field(port, "Name")?)?;
            let types = match field(port, "Types") {
                Some(Value::Array(types)) => types.iter().filter_map(|t| text(field(t, "Type")?)).collect(),
                _ => Vec::new(),
            };
            Some(PortDef { name, types })
        })
        .collect()
}

/// The port attachment that skins the attached item to the character.
const SKIN_ATTACHMENT: &str = "SItemPortDefAttachmentImplementationSkin";

fn first_text(db: &Database, record: &Record, path: &str) -> Option<String> {
    let compiled = db.compile_path::<Value>(record.struct_id(), path).ok()?;
    db.query::<Value>(&compiled, record).ok()?.iter().find_map(text)
}

fn text(v: &Value) -> Option<String> {
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
/// the loadout's and the placed items by name). Unknown names are reported
/// in `unplaced`.
pub fn dress_entries(db: &Database, chf: &ChfData, loadout: &[LoadoutEntry], items: &[String]) -> Placement {
    let index = EntityIndex::new(db);
    let mut worn = chf_seed(db, &chf.itemport);
    let mut other_roots = Vec::new();
    for entry in loadout {
        if entry.port.eq_ignore_ascii_case(&worn.port) {
            merge_entries(db, &index, &mut worn, &entry.children);
        } else {
            other_roots.push(entry.clone());
        }
    }
    let mut wearables = Vec::new();
    let mut unknown = Vec::new();
    for name in items {
        match wearable(db, &index, name) {
            Some(w) => wearables.push(w),
            None => unknown.push(format!("{name}: no such wearable item")),
        }
    }
    let mut placement = place_items(&worn, &wearables);
    placement.entries.extend(other_roots);
    placement.unplaced.extend(unknown);
    placement
}

/// The CHF's item tree as worn items (kept: `item` empty) with their ports.
fn chf_seed(db: &Database, port: &ItemPort) -> Seed {
    Seed {
        port: port.name.to_string(),
        item: String::new(),
        ports: db.record_by_id(&port.id).map(|r| ports(db, r)).unwrap_or_default(),
        children: port.children.iter().map(|c| chf_seed(db, c)).collect(),
    }
}

/// Lay a loadout's `entries` over `worn`: an entry on a port something is
/// worn on replaces it (a named item, with its own ports and nothing under
/// it yet) or keeps it (empty name); other entries are added.
fn merge_entries(db: &Database, index: &EntityIndex, worn: &mut Seed, entries: &[LoadoutEntry]) {
    let item_ports = |name: &str| index.find_record(name).map(|r| ports(db, r)).unwrap_or_default();
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
            child.ports = item_ports(&entry.item);
            child.children.clear();
        }
        merge_entries(db, index, child, &entry.children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, ty: &str) -> PortDef {
        PortDef { name: name.into(), types: vec![ty.into()] }
    }
    fn item(name: &str, ty: &str, ports: Vec<PortDef>) -> Wearable {
        Wearable { item: name.into(), item_type: ty.into(), ports }
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

    #[test]
    fn port_types_match_case_insensitively() {
        let p = place_items(&body("body", vec![port("Clothing_Torso_0", "char_clothing_torso_0")]), &[item("shirt", "Char_Clothing_Torso_0", vec![])]);
        assert!(p.unplaced.is_empty());
    }
}
