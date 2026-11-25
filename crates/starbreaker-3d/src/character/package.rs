//! CHF character → decomposed GLB or Blender package.
//!
//! [`export_character_package`] writes the same `Packages/<name>/scene.json`
//! + `Data/...` package as entity exports (`.blend` mesh assets, material
//! sidecars, PNG textures), so the Blender addon imports a character with its
//! real materials. The meshes are the character rig's
//! (`HeadAssets::character_rig`: DNA head, DNA eyes/teeth, wrapped
//! eye-details, CHF items, and the body and a loadout's garments on the
//! actor skeleton, zones they cover left out — [`super::outfit`]) — each
//! part's pipeline-loaded
//! [`super::part_source::PartSource`] (submesh material ids, every vertex
//! stream, the part's `.mtl`) reshaped to the neutral-identity shape, the
//! same meshes the character GLB writes; the root carries the rig's
//! skeleton (the scene.blend armature every skinned part binds to) and the
//! head its blendshapes as shape keys. The head uses the `.mtl` the CHF
//! selects (`Head Material`, resolved through DataCore).
//!
//! Reshaped parts are the character's own geometry, so their mesh assets
//! live in `Characters/<name>/` ([`character_geometry_path`]). With a
//! [`CharacterAnimation`] the package carries its clip as Blender actions
//! (armature in scene.blend; a face clip's blendshapes as the head's shape
//! keys, animated with the head asset).
//!
//! With `ExportOptions::keep_covered_zones` the body and garments are
//! written with every submesh, one entity per clothing zone
//! (`CharacterRig::zone_payloads`, marked `clothing_zone`), and the
//! `character` object lists the worn items' clothing (`outfit`), so a
//! consumer can dress the character in any combination itself.
//!
//! The root scene instance gains a `character` object: the resolved CHF
//! material customization (skin tone, blemishes, makeup, eye and hair
//! colours — see [`super::customization`]) and the HumanSkin_V2 wrinkle
//! slot layout, for the importer's skin material. The CHF's override
//! textures are exported as package PNGs; each customized submaterial maps
//! them in `texture_exports` (`TexSlotN` → package-relative PNG path).

use serde_json::json;
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;
use std::sync::Arc;

use super::customization::{MaterialLookup, bind_item_materials, resolve_chf_materials};
use super::identity_context::HeadAssets;
use super::items::{load_head_items, resolve_head_items};
use super::part_source::{PartSource, geometry_path};
use super::wrinkles::SLOT_LAYOUT;
use super::clips::CharacterAnimation;
use super::clothing::clothing_json;
use super::loadout::LoadoutEntry;
use super::outfit::Outfit;
use super::{CharacterError, gender_tag};
use crate::decomposed::{DecomposedInput, texture_png};
use crate::pipeline::{DecomposedExport, ExportOptions, ExportedFile, ExportedFileKind, PngCache, write_decomposed_export_blend};

/// Package geometry path of a reshaped part of `character`: the shape is
/// this character's own, so it lives in `Characters/<character>/` instead of
/// at the shared source path (another character would overwrite it). Only
/// the file name is kept: the source directories are deep, and Windows
/// cannot open library paths past 260 characters.
fn character_geometry_path(character: &str, p4k_path: &str) -> String {
    let path = geometry_path(p4k_path);
    let file = path.rsplit('/').next().unwrap_or(&path);
    format!("Characters/{}/{file}", crate::decomposed::sanitize_identifier(character))
}

/// A zone entity of a package written with `keep_covered_zones`.
struct ZoneEntity {
    entity: String,
    part: String,
    zone: Option<u32>,
    covered: bool,
}

/// Mark each zone entity of `scene` (a `scene.json`) with its part, zone
/// hash and whether the outfit covers it (`clothing_zone`), and give the
/// root's `character` the worn items' clothing (`outfit`).
fn annotate_zones(scene: &mut serde_json::Value, zones: &[ZoneEntity], outfit: Vec<serde_json::Value>) {
    for child in scene["children"].as_array_mut().into_iter().flatten() {
        let Some(z) = zones.iter().find(|z| child["entity_name"] == z.entity.as_str()) else { continue };
        child["clothing_zone"] = json!({"part": z.part, "hash": z.zone, "covered": z.covered});
    }
    scene["root_entity"]["character"]["outfit"] = serde_json::Value::Array(outfit);
}

/// Export a CHF character as a decomposed package (in memory; the caller
/// writes [`DecomposedExport::files`] under its export root). `name` is the
/// root scene instance (and so the package directory) name. `opts.kind`
/// must be `Decomposed` with `format: Blend` or `Glb`. `animation` is what the
/// package plays (`CharacterAnimation::default()`: the rest pose);
/// `loadout` what the character wears over its body (empty: nothing).
pub fn export_character_package(
    p4k: &MappedP4k,
    db: &Database,
    chf_bytes: &[u8],
    name: &str,
    opts: &ExportOptions,
    animation: &CharacterAnimation<'_>,
    loadout: &[LoadoutEntry],
) -> Result<DecomposedExport, CharacterError> {
    let chf = starbreaker_chf::ChfFile::from_chf(chf_bytes)?
        .parse()
        .map_err(|e| CharacterError::Chf(e.into()))?;
    let gender = gender_tag(chf.dna.gender_hash);
    let mut customization = resolve_chf_materials(&MaterialLookup::load(db)?, &chf.materials);
    let item_defs = resolve_head_items(db, &chf.itemport, gender);
    let items = load_head_items(p4k, &item_defs);
    let assets = HeadAssets::load(p4k, chf.dna.gender_hash, chf.dna.variant_hash)?;
    let outfit = Outfit::load(p4k, db, &chf, loadout);
    // Neutral identity, every part, the head's blendshapes (a face clip's
    // expression blendshapes), the outfit and the clip.
    let mut rig = assets.character_rig(&chf.dna, &items, &outfit, animation)?;

    // The rig's parts are LOD0 pipeline meshes; the writer takes the same LOD.
    let lod0 = ExportOptions { lod_level: 0, ..opts.clone() };
    let opts = &lod0;
    let mut png_cache = PngCache::new();
    let head_mtl = customization
        .iter()
        .find(|m| m.name == "Head Material")
        .and_then(|m| m.mtl_path.clone())
        .unwrap_or_default();
    if !head_mtl.is_empty() {
        rig.head_source = Arc::new(PartSource::load(p4k, assets.head_skin_path(), &head_mtl)?);
    }
    let mut root_mesh = rig.head_mesh()?;
    let (mut root_mtl, mut root_nmc) = (rig.head_materials(), rig.head_source.nmc.clone().filter(|_| !rig.head_hidden));
    let part_path = |source: &PartSource| character_geometry_path(name, &source.skin_path);
    let mut zone_entities = Vec::new();
    let mut children = if opts.keep_covered_zones {
        let zones = rig.zone_payloads(name, part_path)?;
        zone_entities = zones
            .iter()
            .filter(|z| z.zone.is_some())
            .map(|z| ZoneEntity { entity: z.payload.entity_name.clone(), part: z.part.clone(), zone: z.zone, covered: z.covered })
            .collect();
        zones.into_iter().map(|z| z.payload).collect()
    } else {
        rig.part_payloads(name, part_path)?
    };

    // GLB parts carry their own complete bind rig and follow the package leader.
    // Promote a visible skinned part when clothing hides the head: a skeleton-only
    // root is otherwise omitted by the decomposed writer.
    let glb = opts.format == crate::pipeline::ExportFormat::Glb;
    let mut root_geometry = character_geometry_path(name, assets.head_skin_path());
    if glb {
        for child in &mut children { child.bones = rig.bones(); child.nmc = None; }
        root_nmc = None;
        if root_mesh.positions.is_empty() {
            let first = children.iter().position(|p| p.mesh.skin.is_some())
                .ok_or_else(|| CharacterError::Loadout("character has no visible skinned part".into()))?;
            let part = children.remove(first);
            root_mesh = part.mesh; root_mtl = part.materials; root_geometry = part.geometry_path;
        }
    }

    // (port, effective .mtl) per item and the body, for binding the CHF
    // material slots.
    let item_materials: Vec<(String, String)> = items
        .iter()
        .map(|i| (&i.item.port, &i.source))
        .chain(outfit.body.as_ref().map(|b| (&b.item.port, &b.source)))
        .filter_map(|(port, source)| Some((port.clone(), source.materials.as_ref()?.source_path.clone()?)))
        .collect();
    let pairs: Vec<(&str, &str)> = item_materials.iter().map(|(p, m)| (p.as_str(), m.as_str())).collect();
    bind_item_materials(&mut customization, &pairs);

    let input = DecomposedInput {
        entity_name: name.to_owned(),
        geometry_path: root_geometry,
        assembly_kind: glb.then(|| "character".into()),
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        material_path: root_mtl.as_ref().and_then(|m| m.source_path.clone()).unwrap_or_default(),
        root_mesh,
        root_materials: root_mtl,
        root_nmc,
        root_palette: None,
        available_palettes: Vec::new(),
        root_bones: rig.bones(),
        animation_sources: if glb { Vec::new() } else { crate::animation::rig_sources::rig_sources(None, children.iter().filter_map(|child| {
            child.skeleton_source_path.as_ref().map(|path| (child.entity_name.clone(), path.clone()))
        })) },
        root_animation_controller: None,
        children,
        interiors: Default::default(),
        paint_variants: Vec::new(),
        animations: rig.animations.clone(),
        physics_grid: None,
    };
    let mut export = if glb {
        crate::decomposed::write_decomposed_export(db, p4k, input, opts, None, None, None,
            PngCache::new(), Default::default(), Default::default(), &mut |_| None)
    } else {
        write_decomposed_export_blend(db, p4k, input, opts, None, None, None, None)
    }.map_err(|e| CharacterError::Customization(format!("package export: {e}")))?;

    // CHF override textures → package PNGs, mapped per submaterial.
    let mut customization_json = serde_json::to_value(&customization).expect("serialize customization");
    for sub in customization_json
        .as_array_mut()
        .into_iter()
        .flatten()
        .filter_map(|m| m["submaterials"].as_array_mut())
        .flatten()
    {
        let mut exports = serde_json::Map::new();
        let textures = sub["textures"].as_object().cloned().unwrap_or_default();
        for (slot, source) in &textures {
            let Some((path, bytes)) = source.as_str().and_then(|s| texture_png(p4k, &mut png_cache, s, opts.texture_mip))
            else {
                eprintln!("CHF texture {source} ({slot}) could not be exported");
                continue;
            };
            if !export.files.iter().any(|f| f.relative_path.eq_ignore_ascii_case(&path)) {
                export.files.push(ExportedFile { relative_path: path.clone(), bytes, kind: ExportedFileKind::TextureAsset });
            }
            exports.insert(slot.clone(), json!(path));
        }
        sub["texture_exports"] = serde_json::Value::Object(exports);
    }

    // Character metadata on the root scene instance.
    let character = json!({
        "source": "chf",
        "gender": gender,
        "customization": customization_json,
        "wrinkles": {
            "shader": "HumanSkin_V2 %WRINKLE_BLENDING_2",
            "slots": SLOT_LAYOUT.iter().map(|s| json!({"panel": s.panel, "channel": s.channel, "band": s.band, "tile": s.tile})).collect::<Vec<_>>(),
        },
    });
    let scene = export
        .files
        .iter_mut()
        .find(|f| f.relative_path.ends_with("/scene.json"))
        .ok_or_else(|| CharacterError::Customization("package has no scene.json".into()))?;
    let mut value: serde_json::Value = serde_json::from_slice(&scene.bytes)
        .map_err(|e| CharacterError::Customization(format!("scene.json: {e}")))?;
    value["root_entity"]["character"] = character;
    if opts.keep_covered_zones {
        let outfit_json = outfit
            .worn_clothing()
            .into_iter()
            .map(|(port, item, part, clothing)| json!({"port": port, "item": item, "part": part, "clothing": clothing_json(clothing)}))
            .collect();
        annotate_zones(&mut value, &zone_entities, outfit_json);
    }
    scene.bytes = serde_json::to_vec_pretty(&value).expect("serialize scene.json");
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zone entities name their part, zone and whether the outfit covers
    /// them; the root's character lists the worn items' clothing.
    #[test]
    fn zones_and_outfit_annotate_the_scene() {
        let mut scene = json!({
            "root_entity": {"entity_name": "c", "character": {"gender": "male"}},
            "children": [{"entity_name": "suit_zone_00000007"}, {"entity_name": "hair"}],
        });
        let zones = [ZoneEntity { entity: "suit_zone_00000007".into(), part: "suit".into(), zone: Some(7), covered: true }];
        let outfit = vec![json!({"port": "body_itemport", "part": "m_body"})];
        annotate_zones(&mut scene, &zones, outfit.clone());
        assert_eq!(scene["children"][0]["clothing_zone"], json!({"part": "suit", "hash": 7, "covered": true}));
        assert!(scene["children"][1].get("clothing_zone").is_none(), "entities that are no zone stay as they are");
        assert_eq!(scene["root_entity"]["character"]["outfit"], json!(outfit));
        assert_eq!(scene["root_entity"]["character"]["gender"], "male");
    }

    #[test]
    fn character_geometry_paths_are_per_character() {
        assert_eq!(
            character_geometry_path("Mascu Line", r"Data\Objects\A\b.skin"),
            "Characters/mascu_line/b.skin"
        );
        assert_ne!(character_geometry_path("a", "Objects/b.skin"), character_geometry_path("c", "Objects/b.skin"));
    }
}
