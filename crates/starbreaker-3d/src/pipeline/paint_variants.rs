//! Hull paint variants: the looks a paint item selects by its `@Tag`.
//!
//! A paint item's `AttachDef.Tags` carry one `@Tag` that names a tagged node
//! of the hull's `SGeometryResourceParams`; that node holds the paint's
//! material and/or tint palette.
//!
//! - [`tagged_paint_nodes`] / [`find_tagged_paint_node`]: the tagged nodes of
//!   one `SGeometryResourceParams` component Value.
//! - [`resolve_paint_override`]: the palette and material of the paint the
//!   default loadout equips.
//! - [`enumerate_paint_variants_for_entity`]: every paint variant of a hull
//!   its paint port accepts ([`retain_accepted_paints`], port data from
//!   `paint_port.rs`), written to `paints.json` / `palettes.json`.

use std::collections::{HashMap, HashSet};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::material_variants::MaterialNode;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::mtl;

use super::datacore_path_to_p4k;
use super::finish_geometry::{component_geometry_path, own_geometry_path, resolve_finish_geometry};
use super::palette::{extract_subgeometry_palette, get_value_array, get_value_field, get_value_string, try_load_mtl};

/// One tagged node: the `@Tag` that selects it, its position in its list, its
/// material path (DataCore form) and the node carrying its `Palette`.
#[derive(Debug, Clone)]
pub(crate) struct PaintNode<'v, 'a> {
    pub tag: &'a str,
    pub index: usize,
    pub material_path: Option<String>,
    pub palette_holder: Option<&'v Value<'a>>,
    /// The node's own geometry file, when it names one other than the
    /// component's ([`own_geometry_path`]); `materialVariants` entries have none.
    pub geometry_path: Option<String>,
}

/// The tagged paint nodes of one `SGeometryResourceParams` component, in
/// data order: `Geometry.SubGeometry[]` entries (each a whole geometry node
/// with its own material and palette), then `Material.materialVariants[]`
/// entries (a material, resolved through any nested variants the way
/// [`MaterialNode::resolve`] does, plus an optional palette). `index` is the
/// position within its own list. Untagged entries (the default look) are skipped.
pub(crate) fn tagged_paint_nodes<'v, 'a>(component: &'v Value<'a>) -> Vec<PaintNode<'v, 'a>> {
    let mut nodes = sub_geometry_paint_nodes(component);
    let variants = get_value_field(component, "Material").and_then(|m| get_value_array(m, "materialVariants"));
    for (index, variant) in variants.into_iter().flatten().enumerate() {
        let tag = get_value_string(variant, "Tags").unwrap_or("").trim();
        if tag.is_empty() {
            continue;
        }
        let material_path = MaterialNode::from_value(variant)
            .resolve(&[tag.to_owned()])
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        nodes.push(PaintNode { tag, index, material_path, palette_holder: Some(variant), geometry_path: None });
    }
    nodes
}

fn sub_geometry_paint_nodes<'v, 'a>(component: &'v Value<'a>) -> Vec<PaintNode<'v, 'a>> {
    let mut nodes = Vec::new();
    let subs = get_value_field(component, "Geometry").and_then(|g| get_value_array(g, "SubGeometry"));
    for (index, sub) in subs.into_iter().flatten().enumerate() {
        let tag = get_value_string(sub, "Tags").unwrap_or("").trim();
        if tag.is_empty() {
            continue;
        }
        let geometry = get_value_field(sub, "Geometry");
        let material_path = geometry
            .and_then(|g| get_value_field(g, "Material"))
            .and_then(|m| get_value_string(m, "path"))
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        let geometry_path = geometry.and_then(|g| own_geometry_path(component, g));
        nodes.push(PaintNode { tag, index, material_path, palette_holder: geometry, geometry_path });
    }
    nodes
}

/// The node `tag` selects (any case), if the component has one.
pub(crate) fn find_tagged_paint_node<'v, 'a>(component: &'v Value<'a>, tag: &str) -> Option<PaintNode<'v, 'a>> {
    tagged_paint_nodes(component).into_iter().find(|n| n.tag.eq_ignore_ascii_case(tag))
}

pub(super) fn geometry_components<'a>(db: &'a Database, record: &Record) -> Vec<Value<'a>> {
    // The whole component Value: querying SubGeometry directly can truncate the array.
    db.compile_path::<Value>(record.struct_id(), "Components[SGeometryResourceParams]")
        .ok()
        .and_then(|c| db.query::<Value>(&c, record).ok())
        .unwrap_or_default()
}

/// Resolve paint-based palette and material overrides from an equipped paint item.
///
/// Finds the paint item in the loadout (hardpoint_paint), extracts the `@Tag` from its
/// AttachDef.Tags, matches it against the entity's tagged paint nodes, and returns the
/// overridden palette and material. Falls through to the originals if no paint is found.
pub(crate) fn resolve_paint_override(
    db: &Database,
    p4k: &MappedP4k,
    entity_record: &Record,
    root_node: &starbreaker_datacore::loadout::LoadoutNode,
    default_palette: Option<mtl::TintPalette>,
    default_mtl: Option<mtl::MtlFile>,
) -> (Option<mtl::TintPalette>, Option<mtl::MtlFile>) {
    let paint_node = root_node
        .children
        .iter()
        .find(|c| c.item_port_name.to_lowercase().contains("paint"));
    let Some(paint_node) = paint_node else {
        return (default_palette, default_mtl);
    };

    let tags = db
        .compile_path::<String>(
            paint_node.record.struct_id(),
            "Components[SAttachableComponentParams].AttachDef.Tags",
        )
        .ok()
        .and_then(|c| db.query_single::<String>(&c, &paint_node.record).ok().flatten())
        .unwrap_or_default();

    // Extract @Tag from the tags string (e.g., "Paint_Gladius @GladiusPirate" → "GladiusPirate").
    let Some(paint_tag) = super::paint_port::paint_selector(&tags) else {
        log::info!("  paint '{}' has no @Tag in '{tags}', using default palette", paint_node.entity_name);
        return (default_palette, default_mtl);
    };
    log::info!("  paint '{}' selects tag '{paint_tag}'", paint_node.entity_name);

    let components = geometry_components(db, entity_record);
    let Some(node) = components.iter().find_map(|c| find_tagged_paint_node(c, paint_tag)) else {
        log::warn!("  paint tag '{paint_tag}' not found in the hull's tagged paint nodes");
        return (default_palette, default_mtl);
    };
    log::info!("  matched paint node [{}] tag='{}'", node.index, node.tag);

    let palette = node
        .palette_holder
        .and_then(|holder| extract_subgeometry_palette(holder, Some(paint_node.entity_name.clone())));
    if let Some(p) = &palette {
        log::info!(
            "  paint palette: primary=[{:.2},{:.2},{:.2}] secondary=[{:.2},{:.2},{:.2}]",
            p.primary[0], p.primary[1], p.primary[2], p.secondary[0], p.secondary[1], p.secondary[2],
        );
    }

    let override_info = mtl::PaintOverrideInfo {
        paint_item_name: paint_node.entity_name.clone(),
        subgeometry_tag: node.tag.to_string(),
        subgeometry_index: node.index,
        material_path: node.material_path.as_deref().map(|path| datacore_path_to_p4k(path).replace('\\', "/")),
    };
    let mut mtl = match node.material_path.as_deref() {
        Some(mtl_path) => {
            log::info!("  paint material override: {mtl_path}");
            try_load_mtl(p4k, &datacore_path_to_p4k(mtl_path)).or_else(|| default_mtl.clone())
        }
        None => default_mtl.clone(),
    };
    if let Some(materials) = mtl.as_mut() {
        materials.paint_override = Some(override_info);
    }
    (palette.or(default_palette), mtl)
}

/// The `paints.json` ids of a tag: the sanitized tag (the display-name key)
/// and the canonical id without a `paint_` prefix.
/// E.g. "Paint_Aurora_Mk2_Pink_Green_Purple" → ("paint_aurora_mk2_pink_green_purple",
/// "aurora_mk2_pink_green_purple").
pub(super) fn paint_variant_ids(tag: &str) -> (String, String) {
    let sanitized: String = tag
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch.to_ascii_lowercase() } else { '_' })
        .collect();
    let canonical = sanitized.strip_prefix("paint_").unwrap_or(&sanitized).to_string();
    (sanitized, canonical)
}

/// The paint variants of the tagged nodes of `components`, without their
/// materials loaded. Duplicate tags (some entities repeat entries) keep the first.
pub(crate) fn paint_variants_from_components(
    components: &[Value],
    display_names: &HashMap<String, String>,
) -> Vec<mtl::PaintVariant> {
    let mut variants: Vec<mtl::PaintVariant> = Vec::new();
    for node in components.iter().flat_map(tagged_paint_nodes) {
        if variants.iter().any(|v| v.subgeometry_tag.eq_ignore_ascii_case(node.tag)) {
            continue;
        }
        let (sanitized, canonical) = paint_variant_ids(node.tag);
        let display_name = display_names.get(sanitized.as_str()).cloned();
        let palette = node
            .palette_holder
            .and_then(|holder| extract_subgeometry_palette(holder, Some(canonical.clone())))
            .map(|mut palette| {
                if palette.display_name.is_none() {
                    palette.display_name = display_name.clone();
                }
                palette
            });
        variants.push(mtl::PaintVariant {
            subgeometry_tag: node.tag.to_string(),
            palette_id: Some(format!("palette/{canonical}")),
            palette,
            display_name,
            // P4K-relative, stored with forward slashes for the output JSON.
            material_path: node.material_path.as_deref().map(|p| datacore_path_to_p4k(p).replace('\\', "/")),
            materials: None,
            geometry_path: node.geometry_path.clone(),
            extra_parts: Vec::new(),
        });
    }
    variants
}

/// Keep the variants whose tag an accepted paint item selects (`accepted`
/// holds lower-case `@Tag` selectors); `None` keeps every variant.
pub(crate) fn retain_accepted_paints(variants: &mut Vec<mtl::PaintVariant>, accepted: Option<&HashSet<String>>) {
    if let Some(accepted) = accepted {
        variants.retain(|v| accepted.contains(&v.subgeometry_tag.to_ascii_lowercase()));
    }
}

/// Enumerate the paint variants of a hull from its tagged paint nodes, keep
/// those its paint port accepts, and load each one's material file.
pub fn enumerate_paint_variants_for_entity(
    db: &Database,
    p4k: &MappedP4k,
    entity_record: &Record,
    root_node: &starbreaker_datacore::loadout::LoadoutNode,
    display_names: &HashMap<String, String>,
) -> Vec<mtl::PaintVariant> {
    let components = geometry_components(db, entity_record);
    let mut variants = paint_variants_from_components(&components, display_names);
    let accepted = super::paint_port::accepted_paint_tags(db, p4k, entity_record, root_node);
    let before = variants.len();
    retain_accepted_paints(&mut variants, accepted.as_ref());
    if variants.len() != before {
        log::info!("  paint port accepts {} of {before} tagged paint nodes", variants.len());
    }
    for variant in &mut variants {
        variant.materials = variant.material_path.as_deref().and_then(|p| try_load_mtl(p4k, &p.replace('/', "\\")));
    }
    if let Some(base) = components.iter().find_map(component_geometry_path) {
        resolve_finish_geometry(p4k, base, &mut variants);
    }
    for variant in &variants {
        log::info!(
            "  paint variant: tag={:?}, material={:?}, palette_id={:?}, display={:?}",
            variant.subgeometry_tag, variant.material_path, variant.palette_id, variant.display_name,
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

    fn material(path: &'static str) -> Value<'static> {
        object("GlobalResourceMaterial", vec![("path", Value::String(path))])
    }

    /// A tint palette reference; `None` is an unset `RootRecord`. Colours are
    /// arbitrary synthetic values.
    fn palette(name: Option<&'static str>) -> Value<'static> {
        let root = match name {
            Some(name) => object(
                "TintPaletteTree",
                vec![(
                    "root",
                    object(
                        "TintPalette",
                        vec![
                            ("name", Value::String(name)),
                            ("entryA", object("TintEntry", vec![("tintColor", object("SRGB8", vec![("r", Value::UInt8(1)), ("g", Value::UInt8(2)), ("b", Value::UInt8(3))]))])),
                        ],
                    ),
                )],
            ),
            None => Value::Null,
        };
        object("TintPaletteRef", vec![("RootRecord", root)])
    }

    fn material_variant(tags: &'static str, path: &'static str, pal: Option<&'static str>) -> Value<'static> {
        object(
            "SMaterialNodeParams",
            vec![("Material", material(path)), ("Palette", palette(pal)), ("Tags", Value::String(tags)), ("materialVariants", Value::Array(vec![]))],
        )
    }

    fn sub_geometry(tags: &'static str, path: &'static str, pal: Option<&'static str>) -> Value<'static> {
        object(
            "SGeometryNodeParams",
            vec![
                ("Tags", Value::String(tags)),
                ("Geometry", object("SGeometryDataParams", vec![("Material", material(path)), ("Palette", palette(pal))])),
            ],
        )
    }

    /// One `SGeometryResourceParams` component Value.
    fn component(subs: Vec<Value<'static>>, variants: Vec<Value<'static>>) -> Value<'static> {
        object(
            "SGeometryResourceParams",
            vec![
                ("Geometry", object("SGeometryNodeParams", vec![("Tags", Value::String("")), ("SubGeometry", Value::Array(subs))])),
                (
                    "Material",
                    object(
                        "SMaterialNodeParams",
                        vec![("Material", material("")), ("Palette", palette(None)), ("Tags", Value::String("")), ("materialVariants", Value::Array(variants))],
                    ),
                ),
            ],
        )
    }

    fn tags(variants: &[mtl::PaintVariant]) -> Vec<&str> {
        variants.iter().map(|v| v.subgeometry_tag.as_str()).collect()
    }

    /// A tagged node whose data names a geometry file of its own.
    fn sub_geometry_with_file(tags: &'static str, file: &'static str) -> Value<'static> {
        object(
            "SGeometryNodeParams",
            vec![
                ("Tags", Value::String(tags)),
                (
                    "Geometry",
                    object(
                        "SGeometryDataParams",
                        vec![
                            ("Geometry", object("GlobalResourceGeometry", vec![("path", Value::String(file))])),
                            ("Material", material("")),
                            ("Palette", palette(Some("synthetic_gold"))),
                        ],
                    ),
                ),
            ],
        )
    }

    /// A finish drawn from its own `.cdf` keeps that file on its variant, so
    /// the export can take the finish's material and extra parts from it.
    #[test]
    fn a_sub_geometry_node_with_its_own_file_keeps_it() {
        let c = component(vec![sub_geometry_with_file("Gold01", "gun/cosmetics/gun_gold.cdf"), sub_geometry("Red01", "", Some("synthetic_red"))], vec![]);
        let variants = paint_variants_from_components(&[c], &HashMap::new());
        assert_eq!(variants[0].geometry_path.as_deref(), Some("gun/cosmetics/gun_gold.cdf"));
        assert_eq!(variants[1].geometry_path, None, "a palette-only node draws the base's geometry");
    }

    #[test]
    fn sub_geometry_entries_are_paint_variants() {
        let c = component(
            vec![sub_geometry("", "", None), sub_geometry("HullPaint_Red", "hull/red.mtl", Some("synthetic_red")), sub_geometry("HullPaint_Red", "hull/dup.mtl", None)],
            vec![],
        );
        let variants = paint_variants_from_components(&[c], &HashMap::new());
        assert_eq!(tags(&variants), ["HullPaint_Red"], "untagged and repeated entries are skipped");
        assert_eq!(variants[0].material_path.as_deref(), Some("Data/hull/red.mtl"));
        assert!(variants[0].palette.is_some());
    }

    /// Hulls whose `SubGeometry` is empty keep their paints in
    /// `Material.materialVariants[]`: tag → material, with an optional palette.
    #[test]
    fn material_variants_are_paint_variants() {
        let c = component(
            vec![],
            vec![material_variant("HullPaint_Blue", "hull/blue.mtl", None), material_variant("HullPaint_Gold", "hull/gold.mtl", Some("synthetic_gold"))],
        );
        let variants = paint_variants_from_components(&[c], &HashMap::new());
        assert_eq!(tags(&variants), ["HullPaint_Blue", "HullPaint_Gold"]);
        assert_eq!(variants[0].material_path.as_deref(), Some("Data/hull/blue.mtl"));
        assert!(variants[0].palette.is_none(), "an unset RootRecord is no palette");
        assert!(variants[1].palette.is_some(), "the variant's own palette");
        assert_eq!(variants[1].palette_id.as_deref(), Some("palette/hullpaint_gold"));
    }

    /// The equipped paint's `@Tag` may name a materialVariants entry only.
    #[test]
    fn a_tag_found_only_in_material_variants_resolves() {
        let c = component(vec![], vec![material_variant("HullPaint_Blue", "hull/blue.mtl", Some("synthetic_blue"))]);
        let node = find_tagged_paint_node(&c, "hullpaint_blue").expect("materialVariants entry");
        assert_eq!(node.material_path.as_deref(), Some("hull/blue.mtl"));
        assert!(node.palette_holder.and_then(|h| extract_subgeometry_palette(h, None)).is_some());
    }

    /// A materialVariants entry without its own material keeps the hull's
    /// (palette-only paint).
    #[test]
    fn a_palette_only_material_variant_has_no_material() {
        let c = component(vec![], vec![material_variant("HullPaint_Plain", "", Some("synthetic_plain"))]);
        let variants = paint_variants_from_components(&[c], &HashMap::new());
        assert_eq!(variants[0].material_path, None);
        assert!(variants[0].palette.is_some());
    }

    /// Hulls sharing one geometry carry every sibling's tags; each lists only
    /// the paints its port accepts, and tags no accepted paint selects (a
    /// port-requirement tag) are not variants. No paint port: nothing filtered.
    #[test]
    fn the_paint_port_keeps_only_the_paints_it_accepts() {
        let c = component(
            vec![],
            vec![
                material_variant("HullA_Red", "a_red.mtl", None),
                material_variant("HullB_Red", "b_red.mtl", None),
                material_variant("Family_Blue", "blue.mtl", None),
                material_variant("HullA_Paint", "a.mtl", None),
            ],
        );
        let all = paint_variants_from_components(&[c], &HashMap::new());
        let accepted: std::collections::HashSet<String> = ["hulla_red", "family_blue"].iter().map(|t| t.to_string()).collect();

        let mut kept = all.clone();
        retain_accepted_paints(&mut kept, Some(&accepted));
        assert_eq!(tags(&kept), ["HullA_Red", "Family_Blue"]);

        let mut unfiltered = all.clone();
        retain_accepted_paints(&mut unfiltered, None);
        assert_eq!(unfiltered.len(), 4);
    }

}
