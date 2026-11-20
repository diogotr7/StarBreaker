//! Item finishes that are separate entities on the base item's geometry.
//!
//! Many FPS weapon finishes are their own `EntityClassDefinition`: the base's
//! geometry file with the finish entity's own top-level `Palette` and/or
//! `Material`, and no tagged node in the base. Exporting the base emits each
//! such sibling as a paint variant so a viewer can switch to it.
//!
//! - [`finish_variants_from_siblings`]: the siblings whose look (effective
//!   palette and material) is not already one of the base's looks.
//! - [`same_item`]: whether two `AttachDef`s are one item in two finishes
//!   (same type, subtype, size and manufacturer).
//! - [`enumerate_sibling_finishes`]: finds the siblings in DataCore (same
//!   geometry file, [`same_item`]) and loads their materials.

use std::collections::HashMap;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::mtl;

use super::datacore_path_to_p4k;
use super::paint_variants::{geometry_components, paint_variant_ids, tagged_paint_nodes};
use super::palette::{extract_subgeometry_palette, get_value_field, get_value_string, localization_key_from_value, try_load_mtl};

/// One sibling finish entity: its short record name and its
/// `SGeometryResourceParams` component Value.
pub(crate) struct Sibling<'v, 'a> {
    pub name: String,
    pub component: &'v Value<'a>,
}

/// A look: the tint palette's name and the material path (lower case),
/// with an unset palette or material already replaced by the base's.
type Look = (Option<String>, String);

/// The name of the tint palette a node's `Palette.RootRecord` names.
fn palette_name(holder: &Value) -> Option<String> {
    let root = get_value_field(get_value_field(get_value_field(holder, "Palette")?, "RootRecord")?, "root")?;
    get_value_string(root, "name").map(str::to_ascii_lowercase)
}

fn material_path<'a>(data: &Value<'a>) -> &'a str {
    get_value_field(data, "Material").and_then(|m| get_value_string(m, "path")).unwrap_or("")
}

/// The component's top-level `Geometry.Geometry` (`SGeometryDataParams`).
fn top_data<'v, 'a>(component: &'v Value<'a>) -> Option<&'v Value<'a>> {
    get_value_field(get_value_field(component, "Geometry")?, "Geometry")
}

fn effective(palette: Option<String>, material: &str, base: &Look) -> Look {
    let material = if material.is_empty() { base.1.clone() } else { material.to_ascii_lowercase() };
    (palette.or_else(|| base.0.clone()), material)
}

/// The paint variants of the siblings of `base` whose look is new, in the
/// given order; `display_name` localizes a sibling's record name. Known
/// looks are the base's default, its tagged paint nodes, and earlier siblings.
pub(crate) fn finish_variants_from_siblings(
    base: &Value,
    siblings: &[Sibling],
    display_name: &dyn Fn(&str) -> Option<String>,
) -> Vec<mtl::PaintVariant> {
    let base_data = top_data(base);
    let base_look: Look = (base_data.and_then(palette_name), base_data.map(material_path).unwrap_or("").to_ascii_lowercase());
    let mut known: Vec<Look> = vec![base_look.clone()];
    for node in tagged_paint_nodes(base) {
        let palette = node.palette_holder.and_then(palette_name);
        known.push(effective(palette, node.material_path.as_deref().unwrap_or(""), &base_look));
    }

    let mut variants = Vec::new();
    for sibling in siblings {
        let Some(data) = top_data(sibling.component) else { continue };
        let material = material_path(data);
        let look = effective(palette_name(data), material, &base_look);
        if known.contains(&look) {
            continue;
        }
        known.push(look);
        let (_, canonical) = paint_variant_ids(&sibling.name);
        let display_name = display_name(&sibling.name);
        let palette = extract_subgeometry_palette(data, Some(canonical.clone())).map(|mut palette| {
            palette.display_name = palette.display_name.or_else(|| display_name.clone());
            palette
        });
        variants.push(mtl::PaintVariant {
            subgeometry_tag: sibling.name.clone(),
            palette_id: Some(format!("palette/{canonical}")),
            palette,
            display_name,
            material_path: (!material.is_empty()).then(|| datacore_path_to_p4k(material).replace('\\', "/")),
            materials: None,
            geometry_path: None,
            extra_parts: Vec::new(),
        });
    }
    variants
}

/// Whether `other` is the same item as `base` in another finish: the same
/// `AttachDef` type, subtype and size, from the same manufacturer. Other
/// items drawn with the same geometry file (a creature's weapon, another
/// maker's gadget) are not finishes of it.
pub(crate) fn same_item(base: &Value, other: &Value) -> bool {
    let maker = |item: &Value| match get_value_field(item, "Manufacturer") {
        Some(Value::Object { record_id, .. }) => *record_id,
        _ => None,
    };
    maker(base).is_some()
        && maker(base) == maker(other)
        && ["Type", "SubType", "Size"].iter().all(|name| get_value_field(base, name) == get_value_field(other, name))
}

fn query_string(db: &Database, record: &Record, path: &str) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), path)
        .ok()
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten())
        .filter(|s| !s.is_empty())
}

fn short_name(db: &Database, record: &Record) -> String {
    let full = db.resolve_string2(record.name_offset);
    full.rsplit('.').next().unwrap_or(full).to_string()
}

const GEOMETRY_PATH: &str = "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path";
const ATTACH_DEF: &str = "Components[SAttachableComponentParams].AttachDef";
const LOCALIZED_NAME: &str = "Components[SAttachableComponentParams].AttachDef.Localization.Name";

fn localized_name(db: &Database, record: &Record, localization: &HashMap<String, String>) -> Option<String> {
    let compiled = db.compile_path::<Value>(record.struct_id(), LOCALIZED_NAME).ok()?;
    let key = localization_key_from_value(&db.query_single::<Value>(&compiled, record).ok().flatten()?)?;
    localization.get(&key).cloned()
}

/// The finish variants of an item: other items drawn with the same geometry
/// file that are the same item ([`same_item`]) and whose look is new. Vehicles
/// (their paints come through their paint port) and non-items have none.
pub fn enumerate_sibling_finishes(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    localization: &HashMap<String, String>,
) -> Vec<mtl::PaintVariant> {
    if query_string(db, record, "Components[VehicleComponentParams].vehicleDefinition").is_some() {
        return Vec::new();
    }
    let Ok(attach_def) = db.compile_path::<Value>(record.struct_id(), ATTACH_DEF) else {
        return Vec::new();
    };
    let attach = |r: &Record| db.query_single::<Value>(&attach_def, r).ok().flatten();
    let (Some(geometry), Some(item)) = (query_string(db, record, GEOMETRY_PATH), attach(record)) else {
        return Vec::new();
    };
    let normalize = |p: &str| p.replace('\\', "/").to_ascii_lowercase();
    let geometry = normalize(&geometry);
    let Ok(geometry_rooted) = db.compile_rooted::<String>(&format!("EntityClassDefinition.{GEOMETRY_PATH}")) else {
        return Vec::new();
    };
    let own_name = short_name(db, record);

    let mut found: Vec<(String, Vec<Value>, &Record)> = Vec::new();
    for other in db.records_by_type_name("EntityClassDefinition") {
        if !db.is_main_record(other) {
            continue;
        }
        let same_geometry = db
            .query_single::<String>(&geometry_rooted, other)
            .ok()
            .flatten()
            .is_some_and(|p| normalize(&p) == geometry);
        if !same_geometry {
            continue;
        }
        let name = short_name(db, other);
        if name.eq_ignore_ascii_case(&own_name) || !attach(other).is_some_and(|o| same_item(&item, &o)) {
            continue;
        }
        found.push((name, geometry_components(db, other), other));
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));

    let base = geometry_components(db, record);
    let Some(base) = base.first() else { return Vec::new() };
    let siblings: Vec<Sibling> = found
        .iter()
        .filter_map(|(name, components, _)| Some(Sibling { name: name.clone(), component: components.first()? }))
        .collect();
    let display_name = |name: &str| -> Option<String> {
        let (_, _, record) = found.iter().find(|(n, _, _)| n == name)?;
        localized_name(db, record, localization)
    };

    let mut variants = finish_variants_from_siblings(base, &siblings, &display_name);
    for variant in &mut variants {
        variant.materials = variant.material_path.as_deref().and_then(|p| try_load_mtl(p4k, &p.replace('/', "\\")));
        log::info!(
            "  finish variant: entity={:?}, material={:?}, own palette={}, display={:?}",
            variant.subgeometry_tag,
            variant.material_path,
            variant.palette.is_some(),
            variant.display_name,
        );
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(type_name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        Value::Object { type_name, fields, record_id: None }
    }

    /// A palette reference named `name`; `None` is an unset `RootRecord`.
    /// Colours are arbitrary synthetic values.
    fn palette(name: Option<&'static str>) -> Value<'static> {
        let root = match name {
            Some(name) => object(
                "TintPaletteTree",
                vec![("root", object("TintPalette", vec![("name", Value::String(name)), ("entryA", object("TintEntry", vec![]))]))],
            ),
            None => Value::Null,
        };
        object("TintPaletteRef", vec![("RootRecord", root)])
    }

    fn data(material: &'static str, pal: Option<&'static str>) -> Value<'static> {
        object(
            "SGeometryDataParams",
            vec![
                ("Geometry", object("GlobalResourceGeometry", vec![("path", Value::String("weapon/base.cdf"))])),
                ("Material", object("GlobalResourceMaterial", vec![("path", Value::String(material))])),
                ("Palette", palette(pal)),
            ],
        )
    }

    fn component(material: &'static str, pal: Option<&'static str>, subs: Vec<(&'static str, &'static str, Option<&'static str>)>) -> Value<'static> {
        let subs = subs
            .into_iter()
            .map(|(tag, m, p)| object("SGeometryNodeParams", vec![("Tags", Value::String(tag)), ("Geometry", data(m, p))]))
            .collect();
        object(
            "SGeometryResourceParams",
            vec![("Geometry", object("SGeometryNodeParams", vec![("Tags", Value::String("")), ("Geometry", data(material, pal)), ("SubGeometry", Value::Array(subs))]))],
        )
    }

    fn names(variants: &[mtl::PaintVariant]) -> Vec<&str> {
        variants.iter().map(|v| v.subgeometry_tag.as_str()).collect()
    }

    fn base() -> Value<'static> {
        component("", Some("synthetic_default"), vec![("Red01", "", Some("synthetic_red")), ("Fire01", "weapon/fire.mtl", None)])
    }

    #[test]
    fn a_sibling_with_its_own_palette_is_a_finish() {
        let base = base();
        let own = component("", Some("synthetic_gold"), vec![]);
        let variants = finish_variants_from_siblings(&base, &[Sibling { name: "weapon_gold".into(), component: &own }], &|_| None);
        assert_eq!(names(&variants), ["weapon_gold"]);
        assert_eq!(variants[0].palette_id.as_deref(), Some("palette/weapon_gold"));
        assert!(variants[0].palette.is_some(), "its own palette goes to palettes.json");
        assert_eq!(variants[0].material_path, None, "the base material");
    }

    #[test]
    fn a_sibling_with_its_own_material_is_a_finish() {
        let base = base();
        let own = component("weapon/uee.mtl", Some("synthetic_grey"), vec![]);
        let variants = finish_variants_from_siblings(&base, &[Sibling { name: "weapon_uee".into(), component: &own }], &|n| Some(format!("loc:{n}")));
        assert_eq!(variants[0].material_path.as_deref(), Some("Data/weapon/uee.mtl"));
        assert_eq!(variants[0].display_name.as_deref(), Some("loc:weapon_uee"));
    }

    /// Siblings that only select (or repeat) a look the base already has —
    /// the default, a tagged node (an unset palette or material inherits the
    /// base's), or an earlier sibling — add nothing.
    #[test]
    fn siblings_repeating_a_known_look_are_skipped() {
        let base = base();
        let same_default = component("", None, vec![]);
        let same_red = component("", Some("synthetic_red"), vec![]);
        let same_fire = component("weapon/FIRE.mtl", Some("synthetic_default"), vec![]);
        let gold = component("", Some("synthetic_gold"), vec![]);
        let gold_again = component("", Some("synthetic_gold"), vec![]);
        let siblings = [
            Sibling { name: "weapon_store".into(), component: &same_default },
            Sibling { name: "weapon_red".into(), component: &same_red },
            Sibling { name: "weapon_reward".into(), component: &same_fire },
            Sibling { name: "weapon_gold".into(), component: &gold },
            Sibling { name: "weapon_gold_reward".into(), component: &gold_again },
        ];
        assert_eq!(names(&finish_variants_from_siblings(&base, &siblings, &|_| None)), ["weapon_gold"]);
    }

    fn attach(sub_type: &'static str, size: i32, manufacturer: Option<u8>) -> Value<'static> {
        let manufacturer = match manufacturer {
            Some(id) => Value::Object { type_name: "SCItemManufacturer", fields: vec![], record_id: Some(synthetic_guid(id)) },
            None => Value::Null,
        };
        object(
            "SItemDefinition",
            vec![
                ("Type", Value::Enum("WeaponPersonal")),
                ("SubType", Value::Enum(sub_type)),
                ("Size", Value::Int32(size)),
                ("Manufacturer", manufacturer),
            ],
        )
    }

    fn synthetic_guid(id: u8) -> starbreaker_datacore::types::CigGuid {
        let mut bytes = [0u8; 16];
        bytes[0] = id;
        starbreaker_datacore::types::CigGuid::from_bytes(bytes)
    }

    /// A finish is the same item: another maker's item, an item of another
    /// size or subtype, or one with no maker is not, even on the same geometry.
    #[test]
    fn only_the_same_item_from_the_same_maker_is_a_finish() {
        let base = attach("Medium", 2, Some(1));
        assert!(same_item(&base, &attach("Medium", 2, Some(1))));
        assert!(!same_item(&base, &attach("Medium", 1, None)), "a maker-less item of another size");
        assert!(!same_item(&base, &attach("Medium", 2, None)), "no maker");
        assert!(!same_item(&base, &attach("Medium", 2, Some(2))), "another maker");
        assert!(!same_item(&base, &attach("Medium", 3, Some(1))), "another size");
        assert!(!same_item(&base, &attach("Small", 2, Some(1))), "another subtype");
    }
}
