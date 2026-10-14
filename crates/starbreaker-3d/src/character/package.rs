//! CHF character → decomposed package (the Blender import path).
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

/// Export a CHF character as a decomposed package (in memory; the caller
/// writes [`DecomposedExport::files`] under its export root). `name` is the
/// root scene instance (and so the package directory) name. `opts.kind`
/// must be `Decomposed` with `format: Blend`. `animation` is what the
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
    let root_mesh = rig.head_mesh()?;
    let (root_mtl, root_nmc) = (rig.head_materials(), rig.head_source.nmc.clone().filter(|_| !rig.head_hidden));
    let children = rig.part_payloads(name, |source| character_geometry_path(name, &source.skin_path))?;

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
        geometry_path: character_geometry_path(name, assets.head_skin_path()),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        material_path: root_mtl.as_ref().and_then(|m| m.source_path.clone()).unwrap_or_default(),
        root_mesh,
        root_materials: root_mtl,
        root_nmc,
        root_palette: None,
        available_palettes: Vec::new(),
        root_bones: rig.bones(),
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children,
        interiors: Default::default(),
        paint_variants: Vec::new(),
        animations: rig.animations.clone(),
    };
    let mut export = write_decomposed_export_blend(db, p4k, input, opts, None, None, None, None)
        .map_err(|e| CharacterError::Customization(format!("package export: {e}")))?;

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
    scene.bytes = serde_json::to_vec_pretty(&value).expect("serialize scene.json");
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_geometry_paths_are_per_character() {
        assert_eq!(
            character_geometry_path("Mascu Line", r"Data\Objects\A\b.skin"),
            "Characters/mascu_line/b.skin"
        );
        assert_ne!(character_geometry_path("a", "Objects/b.skin"), character_geometry_path("c", "Objects/b.skin"));
    }
}
