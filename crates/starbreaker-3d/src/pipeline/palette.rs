//! Material, paint, tint palette, and DataCore value helpers.
//!
//! `resolve_material` resolves an MTL path from a DataCore entity record.
//! `query_tint_palette` / `query_related_tint_palettes` extract tint colour data.
//! `resolve_paint_override` / `enumerate_paint_variants_for_entity` drive the paint
//! variant system. `try_load_mtl` reads and parses a raw MTL file from the P4k.
//! `query_animation_controller_source` reads the animation controller path.
//! `load_localization_map`, `build_paint_display_name_map`, `populate_palette_display_name`
//! localise paint/palette display names.
//! Also contains `load_layer_details` / `populate_layer_snapshots` for layer-detail textures.

use std::collections::{HashMap, HashSet};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::mtl;

use super::{datacore_path_to_p4k, decode_png, load_diffuse_texture};

pub(crate) fn resolve_mtl_p4k_path(mtl_name: &str, p4k_geom_path: &str) -> String {
    if mtl_name.contains('/') || mtl_name.contains('\\') {
        format!("Data\\{}.mtl", mtl_name.replace('/', "\\"))
    } else {
        let dir = p4k_geom_path
            .rfind('\\')
            .map(|i| &p4k_geom_path[..i])
            .unwrap_or(p4k_geom_path);
        format!("{dir}\\{mtl_name}.mtl")
    }
}

/// Resolve and parse the .mtl material file for a mesh.
pub(crate) fn resolve_material(
    p4k: &MappedP4k,
    datacore_material_path: &str,
    p4k_geom_path: &str,
    metadata_bytes: Option<&[u8]>,
) -> Option<mtl::MtlFile> {
    // 1. Try DataCore material path first
    if !datacore_material_path.is_empty() {
        let base_path = datacore_path_to_p4k(datacore_material_path);
        let mut candidates = vec![base_path.clone()];
        if !base_path.to_ascii_lowercase().ends_with(".mtl") {
            candidates.push(format!("{base_path}.mtl"));
        }
        for candidate in candidates {
            if let Some(mtl) = try_load_mtl(p4k, &candidate) {
                return Some(mtl);
            }
        }
    }

    // 2. Use pre-loaded metadata companion for MtlName fallback
    let metadata = metadata_bytes?;
    let mtl_name = mtl::extract_mtl_name(metadata)?;
    let mtl_p4k_path = resolve_mtl_p4k_path(&mtl_name, p4k_geom_path);
    try_load_mtl(p4k, &mtl_p4k_path)
}

/// Resolve paint-based palette and material overrides from an equipped paint item.
///
/// Finds the paint item in the loadout (hardpoint_paint), extracts the `@Tag` from its
/// AttachDef.Tags, matches it against the entity's SubGeometry entries, and returns the
/// overridden palette and material. Falls through to the originals if no paint is found.
pub(crate) fn resolve_paint_override(
    db: &Database,
    p4k: &MappedP4k,
    entity_record: &Record,
    root_node: &starbreaker_datacore::loadout::LoadoutNode,
    default_palette: Option<mtl::TintPalette>,
    default_mtl: Option<mtl::MtlFile>,
) -> (Option<mtl::TintPalette>, Option<mtl::MtlFile>) {
    // Find paint item in loadout children.
    let paint_node = root_node
        .children
        .iter()
        .find(|c| c.item_port_name.to_lowercase().contains("paint"));
    let Some(paint_node) = paint_node else {
        return (default_palette, default_mtl);
    };

    // Query the paint item's AttachDef.Tags to find the @SubGeometry selector.
    let tags = db
        .compile_path::<String>(
            paint_node.record.struct_id(),
            "Components[SAttachableComponentParams].AttachDef.Tags",
        )
        .ok()
        .and_then(|c| {
            db.query_single::<String>(&c, &paint_node.record)
                .ok()
                .flatten()
        })
        .unwrap_or_default();

    // Extract @Tag from the tags string (e.g., "Paint_Gladius @GladiusPirate" → "GladiusPirate").
    let subgeo_tag = tags.split_whitespace().find_map(|t| t.strip_prefix('@'));
    let Some(subgeo_tag) = subgeo_tag else {
        log::info!(
            "  paint '{}' has no @Tag in '{tags}', using default palette",
            paint_node.entity_name
        );
        return (default_palette, default_mtl);
    };
    log::info!(
        "  paint '{}' selects SubGeometry tag '{subgeo_tag}'",
        paint_node.entity_name
    );

    // Query SubGeometry entries via the full SGeometryResourceParams component Value tree
    // (same approach as loadout.rs query_sub_geometry — querying SubGeometry directly
    // can truncate the array).
    use starbreaker_datacore::query::value::Value;
    let compiled = match db.compile_path::<Value>(
        entity_record.struct_id(),
        "Components[SGeometryResourceParams]",
    ) {
        Ok(c) => c,
        Err(_) => return (default_palette, default_mtl),
    };
    let components = db
        .query::<Value>(&compiled, entity_record)
        .unwrap_or_default();

    for component in &components {
        let geom_node = match get_value_field(component, "Geometry") {
            Some(g) => g,
            None => continue,
        };
        let sub_arr = match get_value_array(geom_node, "SubGeometry") {
            Some(a) => a,
            None => continue,
        };

        for (idx, sub) in sub_arr.iter().enumerate() {
            let tag = get_value_string(sub, "Tags").unwrap_or("");
            if !tag.eq_ignore_ascii_case(subgeo_tag) {
                continue;
            }
            log::info!("  matched SubGeometry[{idx}] tag='{tag}'");

            // Extract palette from this SubGeometry's Geometry.Palette.RootRecord.root.
            let palette = get_value_field(sub, "Geometry").and_then(|geometry| {
                extract_subgeometry_palette(geometry, Some(paint_node.entity_name.clone()))
            });

            if let Some(ref p) = palette {
                log::info!(
                    "  paint palette: primary=[{:.2},{:.2},{:.2}] secondary=[{:.2},{:.2},{:.2}]",
                    p.primary[0],
                    p.primary[1],
                    p.primary[2],
                    p.secondary[0],
                    p.secondary[1],
                    p.secondary[2],
                );
            }

            // Extract material override.
            let mtl_path = get_value_field(sub, "Geometry")
                .and_then(|g| get_value_field(g, "Material"))
                .and_then(|m| get_value_string(m, "path"))
                .filter(|p| !p.is_empty());

            let override_info = mtl::PaintOverrideInfo {
                paint_item_name: paint_node.entity_name.clone(),
                subgeometry_tag: tag.to_string(),
                subgeometry_index: idx,
                material_path: mtl_path.map(|path| datacore_path_to_p4k(path).replace('\\', "/")),
            };

            let mut mtl = if let Some(mtl_path) = mtl_path {
                log::info!("  paint material override: {mtl_path}");
                let p4k_path = datacore_path_to_p4k(mtl_path);
                try_load_mtl(p4k, &p4k_path).or_else(|| default_mtl.clone())
            } else {
                default_mtl.clone()
            };

            if let Some(materials) = mtl.as_mut() {
                materials.paint_override = Some(override_info);
            }

            return (palette.or(default_palette), mtl);
        }
    }

    log::warn!("  paint tag '{subgeo_tag}' not found in SubGeometry entries");
    (default_palette, default_mtl)
}

/// Enumerate all available paint variants for a ship entity by inspecting every
/// SubGeometry entry that carries a @Tag.  For each entry we read the variant's
/// material path and load the MTL file, then derive a stable `paint/…` ID from
/// the sanitized tag string.  Entries without a tag (the default material) are
/// skipped.  Duplicate tags (some entities repeat SubGeometry entries) are
/// de-duplicated.
pub(crate) fn enumerate_paint_variants_for_entity(
    db: &Database,
    p4k: &MappedP4k,
    entity_record: &Record,
    display_names: &HashMap<String, String>,
) -> Vec<mtl::PaintVariant> {
    use starbreaker_datacore::query::value::Value;

    let compiled = match db.compile_path::<Value>(
        entity_record.struct_id(),
        "Components[SGeometryResourceParams]",
    ) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let components = db
        .query::<Value>(&compiled, entity_record)
        .unwrap_or_default();

    let mut variants: Vec<mtl::PaintVariant> = Vec::new();

    for component in &components {
        let geom_node = match get_value_field(component, "Geometry") {
            Some(g) => g,
            None => continue,
        };
        let sub_arr = match get_value_array(geom_node, "SubGeometry") {
            Some(a) => a,
            None => continue,
        };

        for sub in sub_arr {
            let tag = get_value_string(sub, "Tags").unwrap_or("").trim();
            // Skip the default (no-paint) SubGeometry entry.
            if tag.is_empty() {
                continue;
            }
            // Skip duplicates — some entities repeat SubGeometry entries.
            if variants
                .iter()
                .any(|v| v.subgeometry_tag.eq_ignore_ascii_case(tag))
            {
                continue;
            }

            // Derive the P4K-relative material path, normalised to backslashes for P4K lookup
            // and stored with forward slashes for output JSON.
            let p4k_mtl_path: Option<String> = get_value_field(sub, "Geometry")
                .and_then(|g| get_value_field(g, "Material"))
                .and_then(|m| get_value_string(m, "path"))
                .filter(|p| !p.is_empty())
                .map(|p| datacore_path_to_p4k(p));

            // Load the material file for this variant using the backslash P4K path.
            let materials = p4k_mtl_path.as_deref().and_then(|p| try_load_mtl(p4k, p));

            // Store forward-slash version for output.
            let material_path = p4k_mtl_path.map(|p| p.replace('\\', "/"));

            // Derive a stable canonical palette_id directly from the SubGeometry tag.
            // E.g. "Paint_Aurora_Mk2_Pink_Green_Purple" → "palette/aurora_mk2_pink_green_purple".
            let sanitized_tag: String = tag
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                        ch.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            let canonical_tag = sanitized_tag
                .strip_prefix("paint_")
                .unwrap_or(sanitized_tag.as_str());
            let palette_id = Some(format!("palette/{canonical_tag}"));
            // Try to look up a localized display name using the sanitized tag.
            let display_name = display_names.get(sanitized_tag.as_str()).cloned();
            let palette = get_value_field(sub, "Geometry")
                .and_then(|geometry| {
                    extract_subgeometry_palette(geometry, Some(canonical_tag.to_string()))
                })
                .map(|mut palette| {
                    if palette.display_name.is_none() {
                        palette.display_name = display_name.clone();
                    }
                    palette
                });

            log::info!(
                "  paint variant: tag={tag:?}, material={:?}, palette_id={:?}, display={:?}",
                material_path,
                palette_id,
                display_name,
            );

            variants.push(mtl::PaintVariant {
                subgeometry_tag: tag.to_string(),
                palette_id,
                palette,
                display_name,
                material_path,
                materials,
            });
        }
    }

    extend_with_palette_only_paint_variants(db, entity_record, display_names, &mut variants);

    variants
}

fn extend_with_palette_only_paint_variants(
    db: &Database,
    entity_record: &Record,
    display_names: &HashMap<String, String>,
    variants: &mut Vec<mtl::PaintVariant>,
) {
    use starbreaker_datacore::QueryResultExt;
    use starbreaker_datacore::query::value::Value;

    let family_keys = tint_palette_family_keys(db.resolve_string2(entity_record.name_offset));
    if family_keys.is_empty() {
        return;
    }

    let Ok(tags_compiled) = db
        .compile_rooted::<String>(
            "EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Tags",
        )
        .optional()
    else {
        return;
    };
    let Ok(geometry_compiled) = db
        .compile_rooted::<Value>("EntityClassDefinition.Components[SGeometryResourceParams]")
        .optional()
    else {
        return;
    };

    let mut seen_palette_ids: HashSet<String> = variants
        .iter()
        .filter_map(|variant| variant.palette_id.clone())
        .collect();

    for record in db.records_by_type_name("EntityClassDefinition") {
        if !db.is_main_record(record) {
            continue;
        }
        let file_path = db.resolve_string(record.file_name_offset).to_lowercase();
        if !file_path.contains("entities/scitem/ships/paints/") {
            continue;
        }

        let Some(tags) = tags_compiled
            .as_ref()
            .and_then(|compiled| db.query_single::<String>(compiled, record).ok().flatten())
        else {
            continue;
        };
        if !paint_attach_tags_match_family(&tags, &family_keys) {
            continue;
        }

        let full_name = db.resolve_string2(record.name_offset);
        let short_name = full_name
            .rsplit('.')
            .next()
            .unwrap_or(full_name)
            .to_lowercase();
        let canonical_tag = short_name
            .strip_prefix("paint_")
            .unwrap_or(short_name.as_str())
            .to_string();
        let palette_id = format!("palette/{canonical_tag}");
        if !seen_palette_ids.insert(palette_id.clone()) {
            continue;
        }

        let display_name = display_names
            .get(&short_name)
            .cloned()
            .or_else(|| display_names.get(&canonical_tag).cloned());
        let components = geometry_compiled
            .as_ref()
            .and_then(|compiled| db.query::<Value>(compiled, record).ok())
            .unwrap_or_default();
        let palette = components
            .iter()
            .filter_map(|component| get_value_field(component, "Geometry"))
            .find_map(|geometry| extract_subgeometry_palette(geometry, Some(canonical_tag.clone())))
            .map(|mut palette| {
                if palette.display_name.is_none() {
                    palette.display_name = display_name.clone();
                }
                palette
            });

        if palette.is_none() {
            continue;
        }

        let subgeometry_tag = tags
            .split_whitespace()
            .find_map(|token| token.strip_prefix('@'))
            .map(str::to_string);

        variants.push(mtl::PaintVariant {
            subgeometry_tag: subgeometry_tag.unwrap_or_else(|| short_name.clone()),
            palette_id: Some(palette_id),
            palette,
            display_name,
            material_path: None,
            materials: None,
        });
    }
}

fn paint_attach_tags_match_family(tags: &str, family_keys: &[String]) -> bool {
    let tokens: HashSet<String> = tags
        .split_whitespace()
        .map(|token| token.to_lowercase())
        .collect();
    family_keys
        .iter()
        .map(|key| format!("paint_{key}"))
        .any(|candidate| tokens.contains(&candidate))
}

/// Helper: get an object field from a DataCore Value.
fn get_value_field<'v, 'a>(
    val: &'v starbreaker_datacore::query::value::Value<'a>,
    name: &str,
) -> Option<&'v starbreaker_datacore::query::value::Value<'a>> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
    } else {
        None
    }
}

/// Helper: get a string field from a DataCore Value.
fn get_value_string<'a>(
    val: &starbreaker_datacore::query::value::Value<'a>,
    name: &str,
) -> Option<&'a str> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let starbreaker_datacore::query::value::Value::String(s) = v {
                    return Some(s);
                }
            }
        }
    }
    None
}

pub fn query_animation_controller_source(
    db: &Database,
    record: &Record,
) -> Option<crate::animation::AnimationControllerSource> {
    let compiled = db
        .compile_path::<Value>(record.struct_id(), "Components[SAnimationControllerParams]")
        .ok()?;
    let component = db.query_single::<Value>(&compiled, record).ok().flatten()?;
    let animation_database = get_value_string(&component, "AnimationDatabase")?.to_string();
    let animation_controller = get_value_string(&component, "AnimationController")?.to_string();
    Some(crate::animation::AnimationControllerSource {
        animation_database,
        animation_controller,
    })
}

/// Helper: get an array field from a DataCore Value.
fn get_value_array<'v, 'a>(
    val: &'v starbreaker_datacore::query::value::Value<'a>,
    name: &str,
) -> Option<&'v Vec<starbreaker_datacore::query::value::Value<'a>>> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let starbreaker_datacore::query::value::Value::Array(arr) = v {
                    return Some(arr);
                }
            }
        }
    }
    None
}

/// Helper: get a u8 field from a DataCore Value.
fn get_value_u8(val: &starbreaker_datacore::query::value::Value, name: &str) -> Option<u8> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let starbreaker_datacore::query::value::Value::UInt8(n) = v {
                    return Some(*n);
                }
            }
        }
    }
    None
}

/// Helper: get an f32-like field from a DataCore Value.
fn get_value_f32(val: &starbreaker_datacore::query::value::Value, name: &str) -> Option<f32> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                return match v {
                    starbreaker_datacore::query::value::Value::Float(n) => Some(*n),
                    starbreaker_datacore::query::value::Value::Double(n) => Some(*n as f32),
                    starbreaker_datacore::query::value::Value::UInt8(n) => Some(*n as f32),
                    starbreaker_datacore::query::value::Value::UInt16(n) => Some(*n as f32),
                    starbreaker_datacore::query::value::Value::UInt32(n) => Some(*n as f32),
                    _ => None,
                };
            }
        }
    }
    None
}

fn extract_subgeometry_palette(
    geometry: &starbreaker_datacore::query::value::Value,
    source_name: Option<String>,
) -> Option<mtl::TintPalette> {
    let palette_ref = get_value_field(geometry, "Palette")?;
    let root_record = get_value_field(palette_ref, "RootRecord")?;
    let root = get_value_field(root_record, "root")?;

    let read_entry = |entry_name: &str| -> [f32; 3] {
        read_palette_rgb_entry(get_value_field(root, entry_name))
    };
    let read_finish = |entry_name: &str| -> mtl::TintPaletteFinishEntry {
        let entry = get_value_field(root, entry_name);
        mtl::TintPaletteFinishEntry {
            specular: entry.and_then(|value| read_rgb_value_field(value, "specColor")),
            glossiness: entry.and_then(|value| get_value_f32(value, "glossiness")),
        }
    };

    Some(mtl::TintPalette {
        source_name,
        display_name: None,
        primary: read_entry("entryA"),
        secondary: read_entry("entryB"),
        tertiary: read_entry("entryC"),
        glass: read_entry("glassColor"),
        decal_color_r: read_rgb_value_field(root, "decalColorR"),
        decal_color_g: read_rgb_value_field(root, "decalColorG"),
        decal_color_b: read_rgb_value_field(root, "decalColorB"),
        decal_texture: get_value_string(root, "decalTexture").map(str::to_string),
        finish: mtl::TintPaletteFinish {
            primary: read_finish("entryA"),
            secondary: read_finish("entryB"),
            tertiary: read_finish("entryC"),
            glass: read_finish("glassColor"),
        },
    })
}

fn read_palette_rgb_entry(entry: Option<&starbreaker_datacore::query::value::Value>) -> [f32; 3] {
    let rgb = entry
        .and_then(|value| get_value_field(value, "tintColor"))
        .or(entry);
    let r = rgb
        .and_then(|value| get_value_u8(value, "r"))
        .unwrap_or(128);
    let g = rgb
        .and_then(|value| get_value_u8(value, "g"))
        .unwrap_or(128);
    let b = rgb
        .and_then(|value| get_value_u8(value, "b"))
        .unwrap_or(128);
    [
        srgb_to_linear(r as f32 / 255.0),
        srgb_to_linear(g as f32 / 255.0),
        srgb_to_linear(b as f32 / 255.0),
    ]
}

fn read_rgb_value_field(
    value: &starbreaker_datacore::query::value::Value,
    field_name: &str,
) -> Option<[f32; 3]> {
    let rgb = get_value_field(value, field_name)?;
    let r = get_value_u8(rgb, "r")?;
    let g = get_value_u8(rgb, "g")?;
    let b = get_value_u8(rgb, "b")?;
    Some([
        srgb_to_linear(r as f32 / 255.0),
        srgb_to_linear(g as f32 / 255.0),
        srgb_to_linear(b as f32 / 255.0),
    ])
}

/// Query the default tint palette colors from a DataCore entity.
///
/// Strategy:
/// 1. Try querying through the entity's Reference path (follows the Reference to the
///    correct TintPaletteTree record, works when RootRecord is populated).
/// 2. Fallback: search for a TintPaletteTree record matching the entity name.
pub(crate) fn query_tint_palette(db: &Database, record: &Record) -> Option<mtl::TintPalette> {
    let entity_name = db.resolve_string2(record.name_offset);
    let short_name = entity_name
        .rsplit('.')
        .next()
        .unwrap_or(entity_name)
        .to_lowercase();

    // Strategy 1: Query through the entity's Reference path.
    // This follows Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord
    // through the Reference to the TintPaletteTree and reads colors directly.
    let base = "Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord.root";
    if let Some(palette) = query_tint_from_path(db, record, base, Some(short_name.clone())) {
        return Some(palette);
    }

    // Strategy 2: Find TintPaletteTree record by entity name convention.
    let tpt_si = db.struct_id("TintPaletteTree")?;
    // Find an exact match first (e.g., "rsi_zeus_cl"), not a substring match
    // that could pick up a paint variant like "aegs_gladius_black_grey_grey_geometric".
    let palette_record = db.records_of_type(tpt_si).find(|r| {
        let name = db.resolve_string2(r.name_offset).to_lowercase();
        let rec_short = name.rsplit('.').next().unwrap_or(&name);
        rec_short == short_name
    })?;

    query_tint_from_record(db, palette_record, Some(short_name))
}

pub(crate) fn query_related_tint_palettes(
    db: &Database,
    record: &Record,
    default_palette: Option<&mtl::TintPalette>,
) -> Vec<mtl::TintPalette> {
    let Some(tpt_si) = db.struct_id("TintPaletteTree") else {
        return Vec::new();
    };
    let family_keys = tint_palette_family_keys(db.resolve_string2(record.name_offset));
    if family_keys.is_empty() {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut palettes = Vec::new();
    if let Some(palette) = default_palette.cloned() {
        if let Some(source_name) = palette.source_name.clone() {
            seen.insert(source_name);
        }
        palettes.push(palette);
    }
    for palette_record in db.records_of_type(tpt_si) {
        let full_name = db
            .resolve_string2(palette_record.name_offset)
            .to_lowercase();
        let short_name = full_name.rsplit('.').next().unwrap_or(&full_name);
        if !tint_palette_matches_family(short_name, &family_keys) {
            continue;
        }
        if !seen.insert(short_name.to_string()) {
            continue;
        }
        if let Some(palette) =
            query_tint_from_record(db, palette_record, Some(short_name.to_string()))
        {
            palettes.push(palette);
        }
    }

    palettes.sort_by(|left, right| left.source_name.cmp(&right.source_name));
    palettes
}

pub(crate) fn tint_palette_family_keys(name: &str) -> Vec<String> {
    let short_name = name
        .rsplit('.')
        .next()
        .unwrap_or(name)
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .to_lowercase();
    if short_name.is_empty() {
        return Vec::new();
    }

    let mut keys = vec![short_name.clone()];
    if let Some((_, remainder)) = short_name.split_once('_')
        && !remainder.is_empty()
    {
        keys.push(remainder.to_string());
    }
    keys.sort();
    keys.dedup();
    keys
}

pub(crate) fn tint_palette_matches_family(short_name: &str, family_keys: &[String]) -> bool {
    family_keys
        .iter()
        .any(|key| short_name == key || short_name.starts_with(&format!("{key}_")))
}

pub(crate) fn load_localization_map(p4k: &MappedP4k) -> HashMap<String, String> {
    let data = p4k
        .read_file("Data\\Localization\\english\\global.ini")
        .unwrap_or_default();
    parse_localization(&data)
}

fn parse_localization(data: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(data);
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some((raw_key, value)) = line.split_once('=') {
            let key = raw_key.trim().to_lowercase();
            let value = value.trim().to_string();
            // If the key ends with ",p" (plural-form suffix used by Star Citizen's
            // localization format), also insert it under the bare key so lookups
            // using the base form resolve correctly.  The live key with the suffix
            // is inserted too, so explicit plural lookups still work.
            if let Some(base) = key.strip_suffix(",p") {
                map.entry(base.to_string()).or_insert_with(|| value.clone());
            }
            map.insert(key, value);
        }
    }
    map
}

pub(crate) fn build_paint_display_name_map(
    db: &Database,
    localization: &HashMap<String, String>,
) -> HashMap<String, String> {
    use starbreaker_datacore::QueryResultExt;

    if localization.is_empty() {
        return HashMap::new();
    }

    let Ok(loc_compiled) = db
        .compile_rooted::<Value>(
            "EntityClassDefinition.Components[SAttachableComponentParams].AttachDef.Localization.Name",
        )
        .optional()
    else {
        return HashMap::new();
    };

    let mut display_names = HashMap::new();
    for record in db.records_by_type_name("EntityClassDefinition") {
        if !db.is_main_record(record) {
            continue;
        }
        let file_path = db.resolve_string(record.file_name_offset).to_lowercase();
        if !file_path.contains("entities/scitem/ships/paints/") {
            continue;
        }
        let Some(display_name) = loc_compiled
            .as_ref()
            .and_then(|compiled| db.query_single::<Value>(compiled, record).ok().flatten())
            .and_then(|value| localization_key_from_value(&value))
            .and_then(|key| localization.get(&key).cloned())
        else {
            continue;
        };

        let full_name = db.resolve_string2(record.name_offset);
        let short_name = full_name
            .rsplit('.')
            .next()
            .unwrap_or(full_name)
            .to_lowercase();
        display_names.insert(short_name.clone(), display_name.clone());
        if let Some(stripped) = short_name.strip_prefix("paint_") {
            display_names
                .entry(stripped.to_string())
                .or_insert(display_name);
        }
    }

    display_names
}

fn localization_key_from_value(value: &Value) -> Option<String> {
    let key = match value {
        Value::String(text) | Value::Locale(text) => text.to_string(),
        _ => return None,
    };
    if key.is_empty() || key == "@LOC_UNINITIALIZED" || key == "@LOC_EMPTY" {
        return None;
    }
    Some(key.strip_prefix('@').unwrap_or(&key).to_lowercase())
}

pub(crate) fn populate_palette_display_name(
    palette: &mut mtl::TintPalette,
    display_names: &HashMap<String, String>,
) {
    if palette.display_name.is_some() {
        return;
    }
    let Some(source_name) = palette.source_name.as_deref() else {
        return;
    };
    let key = source_name
        .rsplit('.')
        .next()
        .unwrap_or(source_name)
        .to_lowercase();
    if let Some(display_name) = display_names.get(&key) {
        palette.display_name = Some(display_name.clone());
    }
}

/// Convert an sRGB 0.0-1.0 component to linear.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Read tint palette colors from a path through an entity record.
fn query_tint_from_path(
    db: &Database,
    record: &Record,
    base: &str,
    source_name: Option<String>,
) -> Option<mtl::TintPalette> {
    let query_rgb = |entry: &str| -> [f32; 3] {
        let mut rgb = [0.5f32; 3];
        let entry_base = if entry.eq_ignore_ascii_case("glassColor") {
            format!("{base}.{entry}")
        } else {
            format!("{base}.{entry}.tintColor")
        };
        for (i, ch) in ["r", "g", "b"].iter().enumerate() {
            let path = format!("{entry_base}.{ch}");
            if let Ok(compiled) = db.compile_path::<u8>(record.struct_id(), &path)
                && let Ok(Some(v)) = db.query_single::<u8>(&compiled, record)
            {
                // DataCore stores palette colors as SRGB8 — convert to linear for glTF PBR.
                rgb[i] = srgb_to_linear(v as f32 / 255.0);
            }
        }
        rgb
    };
    let query_finish_rgb = |entry: &str| -> Option<[f32; 3]> {
        let mut rgb = [0.0f32; 3];
        let mut found = false;
        for (i, ch) in ["r", "g", "b"].iter().enumerate() {
            let path = format!("{base}.{entry}.specColor.{ch}");
            if let Ok(compiled) = db.compile_path::<u8>(record.struct_id(), &path)
                && let Ok(Some(v)) = db.query_single::<u8>(&compiled, record)
            {
                found = true;
                rgb[i] = srgb_to_linear(v as f32 / 255.0);
            }
        }
        found.then_some(rgb)
    };
    let query_glossiness = |entry: &str| -> Option<f32> {
        let path = format!("{base}.{entry}.glossiness");
        let compiled = db.compile_path::<f32>(record.struct_id(), &path).ok()?;
        db.query_single::<f32>(&compiled, record).ok().flatten()
    };
    let query_palette_rgb = |entry: &str| -> Option<[f32; 3]> {
        let mut rgb = [0.0f32; 3];
        let mut found = false;
        for (i, ch) in ["r", "g", "b"].iter().enumerate() {
            let path = format!("{base}.{entry}.{ch}");
            if let Ok(compiled) = db.compile_path::<u8>(record.struct_id(), &path)
                && let Ok(Some(v)) = db.query_single::<u8>(&compiled, record)
            {
                found = true;
                rgb[i] = srgb_to_linear(v as f32 / 255.0);
            }
        }
        found.then_some(rgb)
    };
    let query_string = |entry: &str| -> Option<String> {
        let path = format!("{base}.{entry}");
        let compiled = db.compile_path::<String>(record.struct_id(), &path).ok()?;
        db.query_single::<String>(&compiled, record).ok().flatten()
    };

    // Quick check: can we even query this path?
    let test_path = format!("{base}.entryA.tintColor.r");
    let compiled = db.compile_path::<u8>(record.struct_id(), &test_path).ok()?;
    let _val = db.query_single::<u8>(&compiled, record).ok().flatten()?;

    Some(mtl::TintPalette {
        source_name,
        display_name: None,
        primary: query_rgb("entryA"),
        secondary: query_rgb("entryB"),
        tertiary: query_rgb("entryC"),
        glass: query_rgb("glassColor"),
        decal_color_r: query_palette_rgb("decalColorR"),
        decal_color_g: query_palette_rgb("decalColorG"),
        decal_color_b: query_palette_rgb("decalColorB"),
        decal_texture: query_string("decalTexture"),
        finish: mtl::TintPaletteFinish {
            primary: mtl::TintPaletteFinishEntry {
                specular: query_finish_rgb("entryA"),
                glossiness: query_glossiness("entryA"),
            },
            secondary: mtl::TintPaletteFinishEntry {
                specular: query_finish_rgb("entryB"),
                glossiness: query_glossiness("entryB"),
            },
            tertiary: mtl::TintPaletteFinishEntry {
                specular: query_finish_rgb("entryC"),
                glossiness: query_glossiness("entryC"),
            },
            glass: mtl::TintPaletteFinishEntry {
                specular: query_finish_rgb("glassColor"),
                glossiness: query_glossiness("glassColor"),
            },
        },
    })
}

/// Read tint palette colors from a TintPaletteTree record directly.
pub(crate) fn query_tint_from_record(
    db: &Database,
    record: &Record,
    source_name: Option<String>,
) -> Option<mtl::TintPalette> {
    query_tint_from_path(db, record, "root", source_name)
}

pub(crate) fn try_load_mtl(p4k: &MappedP4k, p4k_path: &str) -> Option<mtl::MtlFile> {
    let entry = p4k.entry_case_insensitive(p4k_path)?;
    let data = p4k.read(entry).ok()?;
    let mut mtl = mtl::parse_mtl(&data).ok()?;
    mtl.source_path = Some(p4k_path.to_string());
    populate_layer_snapshots(p4k, &mut mtl);
    Some(mtl)
}

fn populate_layer_snapshots(p4k: &MappedP4k, mtl: &mut mtl::MtlFile) {
    for material in &mut mtl.materials {
        let parent_surface_type = material.surface_type.clone();
        for layer in &mut material.layers {
            if layer.snapshot.is_none() || layer.resolved_material.is_none() {
                if let Some((snapshot, resolved_material)) =
                    load_layer_details(p4k, layer, &parent_surface_type)
                {
                    if layer.snapshot.is_none() {
                        layer.snapshot = Some(snapshot);
                    }
                    if layer.resolved_material.is_none() {
                        layer.resolved_material = Some(resolved_material);
                    }
                }
            }
        }
    }
}

fn load_layer_details(
    p4k: &MappedP4k,
    layer: &mtl::MatLayer,
    parent_surface_type: &str,
) -> Option<(mtl::MatLayerSnapshot, mtl::ResolvedLayerMaterial)> {
    let p4k_path = datacore_path_to_p4k(&layer.path);
    let entry = p4k.entry_case_insensitive(&p4k_path)?;
    let data = p4k.read(entry).ok()?;
    let layer_mtl = mtl::parse_mtl(&data).ok()?;
    let material = mtl::resolve_layer_submaterial(&layer_mtl, &layer.sub_material)?;

    // Prefer the parent submaterial's SurfaceType (e.g. the hard-surface
    // material carries ``metal_dense`` / ``rubber_dense`` etc. and
    // expresses the intended PBR class). Layer sub-mtls sometimes
    // declare their own SurfaceType that reflects the *sampling*
    // material rather than the parent's intent (e.g. a rubber panel
    // whose "Primary" layer is ``ship_lf_panel_rubber_a_base.mtl`` with
    // SurfaceType=metal_shell). Trusting the parent avoids false
    // metallic classifications in that case, while still falling back
    // to the layer's SurfaceType when the parent is unset.
    let effective_surface_type = if !parent_surface_type.is_empty() {
        parent_surface_type
    } else {
        material.surface_type.as_str()
    };

    let specular_texture_mean = if material.public_param_f32(&["TintMode"]).unwrap_or(0.0) > 0.0 {
        load_layer_specular_texture_mean(p4k, material)
    } else {
        None
    };

    Some((
        mtl::MatLayerSnapshot {
            shader: material.shader.clone(),
            diffuse: material.diffuse,
            specular: material.specular,
            shininess: material.shininess,
            wear_specular_color: material.public_param_rgb(&["WearSpecularColor"]),
            wear_glossiness: material.public_param_f32(&["WearGlossiness"]),
            surface_type: if effective_surface_type.is_empty() {
                None
            } else {
                Some(effective_surface_type.to_string())
            },
            metallic: mtl::layer_metallic_with_surface_type(
                material.diffuse,
                material.specular,
                specular_texture_mean,
                Some(effective_surface_type),
            ),
        },
        material.resolved_layer_material(),
    ))
}

fn load_layer_specular_texture_mean(p4k: &MappedP4k, material: &mtl::SubMaterial) -> Option<f32> {
    if material.shader_family() != mtl::ShaderFamily::Layer {
        return None;
    }

    // Layer sub-materials conventionally place their authored F0 texture in
    // TexSlot6. Sampling a lower mip is sufficient for classification and much
    // cheaper than decoding the full-resolution DDS.
    let spec_path = material
        .texture_slots
        .iter()
        .find(|binding| !binding.is_virtual && binding.slot.eq_ignore_ascii_case("TexSlot6"))
        .map(|binding| binding.path.as_str())?;

    let png = load_diffuse_texture(p4k, spec_path, 5)?;
    let image = decode_png(&png)?;
    let pixel_count = (image.width() as u64).saturating_mul(image.height() as u64);
    if pixel_count == 0 {
        return None;
    }

    let rgb_sum: u64 = image
        .pixels()
        .map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]))
        .sum();
    Some(rgb_sum as f32 / (pixel_count as f32 * 255.0 * 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use starbreaker_datacore::query::value::Value;

    fn object(
        type_name: &'static str,
        fields: Vec<(&'static str, Value<'static>)>,
    ) -> Value<'static> {
        Value::Object {
            type_name,
            fields,
            record_id: None,
        }
    }

    fn rgb(r: u8, g: u8, b: u8) -> Value<'static> {
        object(
            "SRGB8",
            vec![
                ("r", Value::UInt8(r)),
                ("g", Value::UInt8(g)),
                ("b", Value::UInt8(b)),
            ],
        )
    }

    fn tint_entry(
        r: u8,
        g: u8,
        b: u8,
        spec_r: u8,
        spec_g: u8,
        spec_b: u8,
        glossiness: f32,
    ) -> Value<'static> {
        object(
            "TintEntry",
            vec![
                ("tintColor", rgb(r, g, b)),
                ("specColor", rgb(spec_r, spec_g, spec_b)),
                ("glossiness", Value::Float(glossiness)),
            ],
        )
    }

    #[test]
    fn extract_subgeometry_palette_reads_direct_glass_color_fields() {
        let geometry = object(
            "SGeometryDataParams",
            vec![(
                "Palette",
                object(
                    "TintPaletteRef",
                    vec![(
                        "RootRecord",
                        object(
                            "TintPaletteTree",
                            vec![(
                                "root",
                                object(
                                    "TintPalette",
                                    vec![
                                        ("entryA", tint_entry(55, 55, 55, 59, 59, 59, 180.0)),
                                        ("entryB", tint_entry(45, 45, 45, 59, 59, 59, 250.0)),
                                        ("entryC", tint_entry(211, 152, 28, 59, 59, 59, 250.0)),
                                        ("glassColor", rgb(255, 95, 17)),
                                    ],
                                ),
                            )],
                        ),
                    )],
                ),
            )],
        );

        let palette = extract_subgeometry_palette(&geometry, Some("drak_pitbull".to_string()))
            .expect("palette should parse");

        assert_eq!(
            palette.glass,
            [
                srgb_to_linear(1.0),
                srgb_to_linear(95.0 / 255.0),
                srgb_to_linear(17.0 / 255.0),
            ]
        );
    }
}
