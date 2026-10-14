//! Clothing layers: which parts of worn items stay visible.
//!
//! Every wearable item (`SCItemClothingParams`) claims body zones —
//! `Chunks[].MeshChunk`, the `EMeshChunks` enum (`torso01_zone`,
//! `l_hand_zone`, …) — at a `Layer` (body 0, undersuit 3, armour 4), and
//! may hide whole item ports (`HiddenParts`: an undersuit hides the
//! `Clothing_*` ports, a full helmet `Head_ItemPort`).
//!
//! A mesh names the zone of each submesh: the `.skinm` descriptor's
//! `node_parent_index` indexes the mesh's extra-word table, each word a
//! [`zone_hash`] (FNV-1a-32 of the UPPERCASE zone name with its NUL).
//!
//! [`hidden_zones`] is the engine's rule (client
//! `ItemPortContainer::RefreshMeshChunkVisibility` over the predicates
//! `CSCItemClothing::EnableOverlay` registers per chunk): per zone, the
//! layers that claim it from the top down. The top layer's chunks are
//! evaluated; every lower layer is hidden unless an evaluated, visible
//! chunk above lists it in its `VisibleLayers` (then its chunks are
//! evaluated too). A chunk is visible when its `VisibilityConditions`
//! hold ([`chunk_visible`]). A hidden chunk lists nothing, so it still
//! covers the layers below.
//!
//! - [`Clothing`] / [`ClothingChunk`] / [`VisibilityCondition`] /
//!   [`PortVisibility`]: an item's layers, tags and ports, from
//!   [`clothing_params`]; [`Worn`]: an item on its port.

use std::collections::{BTreeMap, HashMap, HashSet};

use starbreaker_common::CigGuid;
use starbreaker_datacore::Database;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;

/// The zone hash a mesh stores for `name` (see the module docs).
pub fn zone_hash(name: &str) -> u32 {
    name.to_ascii_uppercase()
        .bytes()
        .chain(std::iter::once(0))
        .fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// One test of a [`VisibilityCondition`] (`SCItemClothingPortVisibilityConditionParams`).
/// Each tag list matches a tag set holding all of its tags; an empty list
/// matches nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortVisibility {
    /// The ports whose items are tested; none: the tags of everything worn.
    pub ports: Vec<String>,
    /// Matching hides the chunk outright.
    pub disable: Vec<CigGuid>,
    /// Matching fails the condition.
    pub skip: Vec<CigGuid>,
    /// Must match for the condition to hold.
    pub active: Vec<CigGuid>,
}

/// One of a chunk's `VisibilityConditions`: holds when every entry does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibilityCondition {
    pub port_visibility: Vec<PortVisibility>,
}

/// One zone an item claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClothingChunk {
    /// `EMeshChunks` name (`l_hand_zone`).
    pub zone: String,
    pub layer: i32,
    /// Lower layers that stay visible under this chunk (while it is visible).
    pub visible_layers: Vec<i32>,
    /// When the chunk is visible (none: always).
    pub conditions: Vec<VisibilityCondition>,
}

/// An item's `SCItemClothingParams`, tags and ports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clothing {
    pub chunks: Vec<ClothingChunk>,
    /// Item ports this item hides (`Head_ItemPort`, …).
    pub hidden_ports: Vec<String>,
    /// The item's tags: the entity's `tags` and the clothing's `OverlayTags`.
    pub tags: Vec<CigGuid>,
    /// The item ports it defines (`SItemPortContainerComponentParams`).
    pub ports: Vec<String>,
}

/// A worn item: its clothing on the port it is attached to.
#[derive(Debug, Clone, Copy)]
pub struct Worn<'a> {
    pub port: &'a str,
    pub clothing: &'a Clothing,
}

/// What the conditions test: the worn items by port, the ports that
/// exist, and every worn item's tags.
struct Wearing<'a> {
    occupied: HashMap<String, &'a [CigGuid]>,
    existing: HashSet<String>,
    all_tags: HashSet<CigGuid>,
}

impl<'a> Wearing<'a> {
    fn new(items: &[Worn<'a>]) -> Self {
        let occupied = items.iter().map(|w| (w.port.to_ascii_lowercase(), w.clothing.tags.as_slice())).collect();
        let existing = items
            .iter()
            .flat_map(|w| w.clothing.ports.iter().map(|p| p.to_ascii_lowercase()).chain([w.port.to_ascii_lowercase()]))
            .collect();
        let all_tags = items.iter().flat_map(|w| w.clothing.tags.iter().copied()).collect();
        Self { occupied, existing, all_tags }
    }
}

fn matches(required: &[CigGuid], tags: &dyn Fn(&CigGuid) -> bool) -> bool {
    !required.is_empty() && required.iter().all(tags)
}

/// `(disable, skip, active)` of one entry against what is worn.
fn entry_matches(entry: &PortVisibility, wearing: &Wearing<'_>) -> (bool, bool, bool) {
    let against = |tags: &dyn Fn(&CigGuid) -> bool| {
        (matches(&entry.disable, tags), matches(&entry.skip, tags), matches(&entry.active, tags))
    };
    if entry.ports.is_empty() {
        return against(&|t| wearing.all_tags.contains(t));
    }
    let mut all = (true, true, true);
    for port in &entry.ports {
        let port = port.to_ascii_lowercase();
        let (disable, skip, active) = match wearing.occupied.get(&port) {
            Some(tags) => against(&|t| tags.contains(t)),
            // An empty port (without skip tags): no active tags hold, any
            // others skip; nothing is there to disable.
            None if wearing.existing.contains(&port) && entry.skip.is_empty() => {
                (false, !entry.active.is_empty(), entry.active.is_empty())
            }
            None => (false, false, false),
        };
        all = (all.0 && disable, all.1 && skip, all.2 && active);
        if !all.0 && !all.2 {
            break;
        }
    }
    all
}

/// Whether a chunk is visible: no conditions, or a condition whose
/// entries all match `active` and not all `skip` (any condition whose
/// entries all match `disable` hides it; so does an empty condition).
#[cfg(test)]
fn chunk_visible(chunk: &ClothingChunk, wearing_items: &[Worn<'_>]) -> bool {
    visible(chunk, &Wearing::new(wearing_items))
}

fn visible(chunk: &ClothingChunk, wearing: &Wearing<'_>) -> bool {
    if chunk.conditions.is_empty() {
        return true;
    }
    for condition in &chunk.conditions {
        if condition.port_visibility.is_empty() {
            return false;
        }
        let (mut disable, mut skip, mut active) = (true, true, true);
        for entry in &condition.port_visibility {
            let (d, s, a) = entry_matches(entry, wearing);
            (disable, skip, active) = (disable && d, skip && s, active && a);
            if !disable && !active {
                break;
            }
        }
        if disable {
            return false;
        }
        if active && !skip {
            return true;
        }
    }
    false
}

/// For each worn item, the [`zone_hash`]es of its zones that are not
/// drawn (see the module docs).
pub fn hidden_zones(items: &[Worn<'_>]) -> Vec<HashSet<u32>> {
    let wearing = Wearing::new(items);
    // zone -> layer -> (item, chunk)
    let mut zones: HashMap<u32, BTreeMap<i32, Vec<(usize, &ClothingChunk)>>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        for chunk in &item.clothing.chunks {
            zones.entry(zone_hash(&chunk.zone)).or_default().entry(chunk.layer).or_default().push((i, chunk));
        }
    }
    let mut hidden = vec![HashSet::new(); items.len()];
    for (zone, layers) in zones {
        let mut kept: HashSet<i32> = HashSet::new();
        let mut evaluated = false;
        for (layer, chunks) in layers.iter().rev() {
            let shown = !evaluated || kept.contains(layer);
            for (i, chunk) in chunks {
                if shown && visible(chunk, &wearing) {
                    kept.extend(&chunk.visible_layers);
                } else {
                    hidden[*i].insert(zone);
                }
            }
            evaluated = true;
        }
    }
    hidden
}

/// The clothing of an item record: its `SCItemClothingParams` (none when
/// it has no such component), tags and ports.
pub fn clothing_params(db: &Database, record: &Record) -> Clothing {
    let mut clothing = Clothing {
        tags: query(db, record, "tags").iter().flat_map(values).filter_map(tag_id).collect(),
        ports: query(db, record, "Components[SItemPortContainerComponentParams].Ports.Name")
            .iter()
            .flat_map(values)
            .filter_map(|v| if let Value::String(s) = v { Some((*s).to_owned()) } else { None })
            .collect(),
        ..Clothing::default()
    };
    for params in query(db, record, "Components[SCItemClothingParams]") {
        if let Some(overlay) = field(&params, "OverlayTags") {
            clothing.tags.extend(array(overlay, "tags").iter().filter_map(tag_id));
        }
        for part in array(&params, "HiddenParts") {
            if let Some(Value::String(port)) = field(part, "PortName") {
                clothing.hidden_ports.push((*port).to_owned());
            }
        }
        for chunk in array(&params, "Chunks") {
            let zone = match field(chunk, "MeshChunk") {
                Some(Value::Enum(z) | Value::String(z)) => (*z).to_owned(),
                _ => continue,
            };
            let layer = match field(chunk, "Layer") {
                Some(Value::Int32(l)) => *l,
                _ => continue,
            };
            let visible_layers =
                array(chunk, "VisibleLayers").iter().filter_map(|v| if let Value::Int32(l) = v { Some(*l) } else { None }).collect();
            let conditions = array(chunk, "VisibilityConditions").iter().map(condition).collect();
            clothing.chunks.push(ClothingChunk { zone, layer, visible_layers, conditions });
        }
    }
    clothing
}

fn condition(v: &Value<'_>) -> VisibilityCondition {
    let tags = |entry: &Value<'_>, name: &str| array(entry, name).iter().filter_map(tag_id).collect();
    VisibilityCondition {
        port_visibility: array(v, "PortVisibility")
            .iter()
            .map(|entry| PortVisibility {
                ports: array(entry, "Ports").iter().filter_map(|p| if let Value::String(s) = p { Some((*s).to_owned()) } else { None }).collect(),
                disable: tags(entry, "DisableTags"),
                skip: tags(entry, "SkipTags"),
                active: tags(entry, "ActiveTags"),
            })
            .collect(),
    }
}

/// A `Tag` reference: its record id (materialized or not).
fn tag_id(v: &Value<'_>) -> Option<CigGuid> {
    match v {
        Value::Guid(g) => Some(*g),
        Value::Object { record_id, .. } => *record_id,
        _ => None,
    }
}

fn query<'a>(db: &'a Database, record: &Record, path: &str) -> Vec<Value<'a>> {
    db.compile_path::<Value>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query::<Value>(&compiled, record).ok())
        .unwrap_or_default()
}

/// A query result's values: an array's items, or the value itself.
fn values<'v, 'a>(v: &'v Value<'a>) -> Vec<&'v Value<'a>> {
    match v {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    }
}

fn field<'v, 'a>(v: &'v Value<'a>, name: &str) -> Option<&'v Value<'a>> {
    match v {
        Value::Object { fields, .. } => fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v),
        _ => None,
    }
}

fn array<'v, 'a>(v: &'v Value<'a>, name: &str) -> &'v [Value<'a>] {
    match field(v, name) {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

#[cfg(test)]
mod tests;
