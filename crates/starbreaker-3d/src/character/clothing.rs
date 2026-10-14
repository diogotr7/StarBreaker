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
//! [`hidden_zones`]: a submesh of item A in zone Z is hidden when another
//! worn item claims Z at a higher layer that A's chunk for Z does not list
//! in `VisibleLayers`. (`VisibilityConditions` — state-dependent, e.g. a
//! helmet's visor — are not evaluated: every chunk counts as worn.)
//!
//! - [`Clothing`] / [`ClothingChunk`]: an item's layers, from
//!   [`clothing_params`].

use std::collections::HashSet;

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

/// One zone an item claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClothingChunk {
    /// `EMeshChunks` name (`l_hand_zone`).
    pub zone: String,
    pub layer: i32,
    /// Higher layers that do not hide this item's part of the zone.
    pub visible_layers: Vec<i32>,
}

/// An item's `SCItemClothingParams`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clothing {
    pub chunks: Vec<ClothingChunk>,
    /// Item ports this item hides (`Head_ItemPort`, …).
    pub hidden_ports: Vec<String>,
}

/// For each of the worn `items`, the [`zone_hash`]es of its zones the other
/// items cover (see the module docs).
pub fn hidden_zones(items: &[&Clothing]) -> Vec<HashSet<u32>> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            item.chunks
                .iter()
                .filter(|chunk| {
                    items.iter().enumerate().any(|(j, other)| {
                        j != i
                            && other.chunks.iter().any(|c| {
                                c.zone.eq_ignore_ascii_case(&chunk.zone)
                                    && c.layer > chunk.layer
                                    && !chunk.visible_layers.contains(&c.layer)
                            })
                    })
                })
                .map(|chunk| zone_hash(&chunk.zone))
                .collect()
        })
        .collect()
}

/// The `SCItemClothingParams` of an item record (empty when it has none).
pub fn clothing_params(db: &Database, record: &Record) -> Clothing {
    let Ok(path) = db.compile_path::<Value>(record.struct_id(), "Components[SCItemClothingParams]") else {
        return Clothing::default();
    };
    let mut clothing = Clothing::default();
    for params in db.query::<Value>(&path, record).unwrap_or_default() {
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
            if !array(chunk, "VisibilityConditions").is_empty() {
                log::debug!("clothing chunk {zone}: visibility conditions not evaluated");
            }
            clothing.chunks.push(ClothingChunk { zone, layer, visible_layers });
        }
    }
    clothing
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
mod tests {
    use super::*;

    fn item(layer: i32, zones: &[&str]) -> Clothing {
        Clothing {
            chunks: zones.iter().map(|z| ClothingChunk { zone: (*z).into(), layer, visible_layers: Vec::new() }).collect(),
            hidden_ports: Vec::new(),
        }
    }

    /// Hashes as `m_body.skinm` / the undersuit store them.
    #[test]
    fn zone_hashes_match_the_meshes() {
        assert_eq!(zone_hash("l_hand_zone"), 3335329176);
        assert_eq!(zone_hash("r_hand_zone"), 998859158);
        assert_eq!(zone_hash("torso01_zone"), zone_hash("TORSO01_ZONE"), "case-insensitive");
    }

    /// An undersuit (3) covers the body (0) where both claim a zone; armour
    /// (4) on the hands covers the undersuit's gloves and the body's hands.
    #[test]
    fn higher_layers_hide_lower_ones_per_zone() {
        let body = item(0, &["torso01_zone", "l_hand_zone", "head_zone"]);
        let suit = item(3, &["torso01_zone", "l_hand_zone"]);
        let arms = item(4, &["l_hand_zone", "omega_arms_zone"]);
        let hidden = hidden_zones(&[&body, &suit, &arms]);
        let set = |zones: &[&str]| zones.iter().map(|z| zone_hash(z)).collect::<HashSet<u32>>();
        assert_eq!(hidden[0], set(&["torso01_zone", "l_hand_zone"]), "the head zone stays");
        assert_eq!(hidden[1], set(&["l_hand_zone"]));
        assert!(hidden[2].is_empty(), "nothing covers the top layer");
    }

    #[test]
    fn same_layers_and_visible_layers_do_not_hide() {
        let mut shirt = item(1, &["torso01_zone"]);
        let jacket = item(1, &["torso01_zone"]);
        let vest = item(2, &["torso01_zone"]);
        shirt.chunks[0].visible_layers = vec![2];
        let hidden = hidden_zones(&[&shirt, &jacket, &vest]);
        assert!(hidden[0].is_empty(), "layer 2 is in the shirt's VisibleLayers; layer 1 is its own");
        assert_eq!(hidden[1], [zone_hash("torso01_zone")].into(), "the jacket has no exception");
    }
}
