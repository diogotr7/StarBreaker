//! CHF material customization resolved against DataCore.
//!
//! A CHF's `materials` section overrides the character's materials: each
//! entry names a material slot (`Head Material`, `BodyMaterial`,
//! `EyeMaterial`, `HairDyeMaterial`, …) and may carry a material GUID, and
//! each of its submaterials carries texture overrides (GUID per zero-based
//! shader texture slot: index 4 = `$TexSlot5` blemish mask, 9 = `$TexSlot10`
//! blemish ID, 12–14 makeup, 8 tattoo), float params (`FrecklesAmount`,
//! `BaseMelanin`, …) and colours (`BodyColor` → the skin tone, `EyeColor`,
//! `DyeColor` / `BaseTintColor` for hair, …).
//!
//! Each slot customizes the item on one attachment port: the slot hash is
//! `crc32(lowercase(port) + "_skinitemport")` ([`port_material_slot`]), and
//! the game's serialization presets apply it to that item's submaterial 0.
//! [`bind_item_materials`] records that target as `applies_to`.
//!
//! GUIDs resolve through the `materialLookupTable` of the DataCore record of
//! type `SCharacterGenerationParams` (`DefaultCharacterGenerationParams`):
//! `materials[{guid, filePath}]` (every per-character head `.mtl`, the body
//! `.mtl`s, …) and `textures[{guid, filePath}]` ([`MaterialLookup`]).
//! [`resolve_chf_materials`] produces [`CustomizedMaterial`]s with names
//! de-hashed and paths resolved.

use std::collections::{BTreeMap, HashMap};

use starbreaker_chf::MaterialDefinition;
use starbreaker_common::{CigGuid, NameHash};
use starbreaker_datacore::Database;
use starbreaker_datacore::query::value::Value;
use serde::Serialize;

use super::CharacterError;

/// GUID → file path tables from `SCharacterGenerationParams.materialLookupTable`.
#[derive(Debug, Default, Clone)]
pub struct MaterialLookup {
    pub materials: HashMap<CigGuid, String>,
    pub textures: HashMap<CigGuid, String>,
}

impl MaterialLookup {
    /// Load the lookup table from the first `SCharacterGenerationParams`
    /// record carrying one.
    pub fn load(db: &Database) -> Result<Self, CharacterError> {
        let missing = |what: &str| CharacterError::Customization(format!("DataCore: {what}"));
        let sid = db
            .struct_id("SCharacterGenerationParams")
            .ok_or_else(|| missing("no SCharacterGenerationParams type"))?;
        let path = db
            .compile_path::<Value>(sid, "materialLookupTable")
            .map_err(|e| missing(&format!("materialLookupTable path: {e:?}")))?;
        for record in db.records_of_type(sid) {
            let Ok(values) = db.query::<Value>(&path, record) else { continue };
            for table in &values {
                let lookup = Self {
                    materials: entries(table, "materials"),
                    textures: entries(table, "textures"),
                };
                if !lookup.materials.is_empty() {
                    return Ok(lookup);
                }
            }
        }
        Err(missing("no populated materialLookupTable"))
    }
}

/// `{guid, filePath}` entries of one lookup-table array.
fn entries(table: &Value, field: &str) -> HashMap<CigGuid, String> {
    let Some(Value::Array(items)) = object_field(table, field) else { return HashMap::new() };
    items
        .iter()
        .filter_map(|e| match (object_field(e, "guid"), object_field(e, "filePath")) {
            (Some(Value::Guid(g)), Some(Value::String(p))) if !p.is_empty() => Some((*g, (*p).to_owned())),
            _ => None,
        })
        .collect()
}

fn object_field<'v, 'a>(v: &'v Value<'a>, name: &str) -> Option<&'v Value<'a>> {
    match v {
        Value::Object { fields, .. } => fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v),
        _ => None,
    }
}

/// One CHF material slot with its overrides resolved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomizedMaterial {
    /// Slot name (`Head Material`, `BodyMaterial`, …) or `0x…` if unknown.
    pub name: String,
    /// Raw slot hash (see [`port_material_slot`]).
    #[serde(skip)]
    pub slot: u32,
    /// The `.mtl` the CHF selects for this slot, when it selects one.
    pub mtl_path: Option<String>,
    /// The item material these overrides apply to (set by [`bind_item_materials`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<MaterialTarget>,
    pub submaterials: Vec<CustomizedSubmaterial>,
}

/// A submaterial of an item's `.mtl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaterialTarget {
    pub mtl_path: String,
    pub submaterial_index: u32,
}

/// CHF material slot hash of the item on `port`.
pub fn port_material_slot(port: &str) -> u32 {
    crc32fast::hash(format!("{}_skinitemport", port.to_lowercase()).as_bytes())
}

/// Bind each slot to the material of the item on its port (`(port, mtl)`
/// pairs), submaterial 0 as in the game's serialization presets.
pub fn bind_item_materials(materials: &mut [CustomizedMaterial], items: &[(&str, &str)]) {
    for material in materials {
        if let Some((_, mtl)) = items.iter().find(|(port, _)| port_material_slot(port) == material.slot) {
            material.applies_to = Some(MaterialTarget { mtl_path: (*mtl).to_owned(), submaterial_index: 0 });
        }
    }
}

/// One submaterial's overrides.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomizedSubmaterial {
    pub name: String,
    /// `TexSlotN` → texture path (shader slot = CHF index + 1).
    pub textures: BTreeMap<String, String>,
    pub params: BTreeMap<String, f32>,
    /// Colours as sRGB RGBA in 0..1.
    pub colors: BTreeMap<String, [f32; 4]>,
}

fn name_of(h: NameHash) -> String {
    h.name().map(str::to_owned).unwrap_or_else(|| format!("0x{:08X}", h.value()))
}

/// Resolve the CHF's material overrides. Texture GUIDs the table does not
/// know are dropped (and reported on stderr).
pub fn resolve_chf_materials(lookup: &MaterialLookup, materials: &[MaterialDefinition]) -> Vec<CustomizedMaterial> {
    materials
        .iter()
        .map(|m| CustomizedMaterial {
            name: name_of(m.name),
            slot: m.name.value(),
            applies_to: None,
            mtl_path: (!m.guid.is_empty()).then(|| lookup.materials.get(&m.guid).cloned()).flatten(),
            submaterials: m
                .sub_materials
                .iter()
                .map(|s| CustomizedSubmaterial {
                    name: name_of(s.name),
                    textures: s
                        .textures
                        .iter()
                        .filter_map(|t| match lookup.textures.get(&t.guid) {
                            Some(p) => Some((format!("TexSlot{}", t.index as u32 + 1), p.clone())),
                            None => {
                                eprintln!("CHF texture {} (slot index {}) not in the lookup table", t.guid, t.index);
                                None
                            }
                        })
                        .collect(),
                    params: s.material_params.iter().map(|p| (name_of(p.name), p.value)).collect(),
                    colors: s
                        .material_colors
                        .iter()
                        .map(|c| (name_of(c.name), [c.value.r, c.value.g, c.value.b, c.value.a].map(|x| x as f32 / 255.0)))
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use starbreaker_chf::{MaterialParam, SubMaterial, Texture};
    use starbreaker_common::ColorRgba;

    fn guid(b: u8) -> CigGuid {
        CigGuid::from_bytes([b; 16])
    }

    #[test]
    fn resolves_material_and_texture_guids_and_names() {
        let lookup = MaterialLookup {
            materials: [(guid(1), "Objects/x/head.mtl".to_owned())].into(),
            textures: [(guid(2), "Objects/x/blemish_mask.tif".to_owned())].into(),
        };
        let chf = vec![MaterialDefinition {
            name: NameHash(0xa98beb34), // "Head Material" (stored hash != crc32c of the display name)
            guid: guid(1),
            mtl_flags: 0,
            sub_materials: vec![SubMaterial {
                name: NameHash::from_string("shader_head"),
                textures: vec![Texture { index: 4, guid: guid(2) }, Texture { index: 9, guid: guid(9) }],
                material_params: vec![MaterialParam { name: NameHash::from_string("FrecklesAmount"), value: 0.5 }],
                material_colors: vec![MaterialParam { name: NameHash(0xbd530797), value: ColorRgba::new(255, 0, 51, 255) }],
            }],
        }];
        let r = resolve_chf_materials(&lookup, &chf);
        assert_eq!(r[0].name, "Head Material");
        assert_eq!(r[0].mtl_path.as_deref(), Some("Objects/x/head.mtl"));
        let s = &r[0].submaterials[0];
        assert_eq!(s.name, "shader_head");
        assert_eq!(s.textures.get("TexSlot5").map(String::as_str), Some("Objects/x/blemish_mask.tif"));
        assert!(!s.textures.contains_key("TexSlot10"), "unknown GUID must be dropped");
        assert_eq!(s.params["FrecklesAmount"], 0.5);
        assert_eq!(s.colors["BodyColor"], [1.0, 0.0, 0.2, 1.0]);
    }

    /// CHF material slots are `crc32(lowercase(port) + "_skinitemport")`.
    #[test]
    fn port_material_slots_match_chf_slot_hashes() {
        assert_eq!(port_material_slot("hair_itemport"), 0x6c836947); // HairDyeMaterial
        assert_eq!(port_material_slot("Eyebrow_ItemPort"), 0x078ac8bd); // EyebrowDyeMaterial
        assert_eq!(port_material_slot("head_itemport"), 0xa98beb34); // Head Material
    }

    #[test]
    fn item_materials_bind_to_their_port_slot() {
        let mut materials = vec![
            CustomizedMaterial { name: "HairDyeMaterial".into(), slot: 0x6c836947, mtl_path: None, applies_to: None, submaterials: vec![] },
            CustomizedMaterial { name: "BeardDyeMaterial".into(), slot: 0x9b274d93, mtl_path: None, applies_to: None, submaterials: vec![] },
        ];
        bind_item_materials(&mut materials, &[("hair_itemport", "Objects/hair/f_hair_36.mtl")]);
        assert_eq!(
            materials[0].applies_to,
            Some(MaterialTarget { mtl_path: "Objects/hair/f_hair_36.mtl".into(), submaterial_index: 0 })
        );
        assert_eq!(materials[1].applies_to, None, "no beard item, no binding");
    }

    #[test]
    fn empty_guid_selects_no_material() {
        let lookup = MaterialLookup::default();
        let chf = vec![MaterialDefinition {
            name: NameHash(0xa047885e),
            guid: CigGuid::EMPTY,
            mtl_flags: 0,
            sub_materials: vec![],
        }];
        assert_eq!(resolve_chf_materials(&lookup, &chf)[0].mtl_path, None);
    }
}
