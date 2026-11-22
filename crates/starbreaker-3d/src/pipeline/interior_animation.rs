//! Socpak interior animation export: promotes animated interior entities to
//! decomposed `children` so their skeletal `.caf` clips are extracted.
//!
//! Hangar doors/elevators are `.cga` (animated geometry) with a sibling
//! `.chrparams` referencing `.caf` clips — the same mechanism ships use. The
//! socpak interior path emits them as static placements, so their animations are
//! never extracted. [`extract_animated_interior_children`] detects them, loads
//! them as [`EntityPayload`] children (carrying `skeleton_source_path`), and
//! removes the static duplicates so the existing decomposed animation pipeline
//! produces their clips.

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::{Record, StringId, StringId2};
use starbreaker_p4k::MappedP4k;

use crate::types::{EntityPayload, ResolvedNode};

use super::{
    datacore_path_to_p4k, load_child_payload_asset, ExportOptions, LoadedChildPayload,
    LoadedInteriors,
};

/// The sibling `.chrparams` path for an animated `.cga` geometry, or `None` for
/// static `.cgf` (or non-geometry) paths. Case of the stem is preserved.
pub(crate) fn chrparams_path_for_cga(cgf_path: &str) -> Option<String> {
    if cgf_path.to_ascii_lowercase().ends_with(".cga") {
        let stem = &cgf_path[..cgf_path.len() - ".cga".len()];
        Some(format!("{stem}.chrparams"))
    } else {
        None
    }
}

/// Synthetic placeholder DataCore record for an interior geometry that has no
/// entity record (socpak `IncludedObjects` are raw geometry placements).
/// `load_child_payload_asset` only falls back to the record when `geometry_path`
/// fails to load, so an empty record is sufficient.
fn placeholder_record() -> Record {
    Record {
        name_offset: StringId2(-1),
        file_name_offset: StringId(0),
        tag_offset: StringId2(-1),
        struct_index: 0,
        id: starbreaker_common::CigGuid::EMPTY,
        instance_index: 0,
        struct_size: 0,
    }
}

fn cga_entity_name(cgf_path: &str) -> String {
    let base = cgf_path.rsplit(['/', '\\']).next().unwrap_or(cgf_path);
    base.get(..base.len().saturating_sub(".cga".len())).unwrap_or(base).to_string()
}

/// Detect animated `.cga` interior entities (those with a sibling `.chrparams`
/// in the P4k), load each as an `EntityPayload` child with its
/// `skeleton_source_path` set, and remove the static placements that became
/// children (so geometry isn't duplicated). Returns the animated children to add
/// to the decomposed `DecomposedInput.children`.
pub(crate) fn extract_animated_interior_children(
    db: &Database,
    p4k: &MappedP4k,
    interiors: &mut LoadedInteriors,
    opts: &ExportOptions,
) -> Vec<EntityPayload> {
    use rayon::prelude::*;

    // Snapshot per-CGF info so the container loop can mutate placements while
    // still reading the shared unique-CGF table.
    let cgf_info: Vec<(String, Option<String>)> = interiors
        .unique_cgfs
        .iter()
        .map(|e| (e.cgf_path.clone(), e.material_path.clone()))
        .collect();

    // Phase 1: detect animated placements and remove them from the static
    // interior placements (de-dup geometry). Records each placement's world
    // transform + per-object palette and an index into `unique_keys`; geometry,
    // materials and textures are loaded once per unique (geometry, material) pair
    // in phase 2 rather than once per placement (a hangar repeats the same door /
    // elevator dozens of times, and TEX0 texture decode dominates the load).
    struct AnimPlacement {
        unique_index: usize,
        offset_position: [f32; 3],
        offset_rotation: [f32; 3],
        palette: Option<crate::mtl::TintPalette>,
        entity_name: String,
    }
    let mut unique_keys: Vec<(String, Option<String>)> = Vec::new();
    let mut key_index: std::collections::HashMap<(String, Option<String>), usize> =
        std::collections::HashMap::new();
    let mut anim_placements: Vec<AnimPlacement> = Vec::new();

    for container in &mut interiors.containers {
        let container_mat = glam::Mat4::from_cols_array_2d(&container.container_transform);
        let mut kept = Vec::with_capacity(container.placements.len());
        for placement in std::mem::take(&mut container.placements) {
            let Some((cgf_path, material_path)) = cgf_info.get(placement.mesh_index).cloned() else {
                kept.push(placement);
                continue;
            };
            let Some(chrparams) = chrparams_path_for_cga(&cgf_path) else {
                kept.push(placement);
                continue;
            };
            if p4k
                .entry_case_insensitive(&datacore_path_to_p4k(&chrparams))
                .is_none()
            {
                kept.push(placement);
                continue;
            }

            // Animated: place at the placement's world transform decomposed into
            // CryEngine-style offset position + Euler-degree rotation.
            let world = container_mat * glam::Mat4::from_cols_array_2d(&placement.transform);
            let (_scale, rot, trans) = world.to_scale_rotation_translation();
            let (rx, ry, rz) = rot.to_euler(glam::EulerRot::XYZ);

            let key = (cgf_path.clone(), material_path.clone());
            let unique_index = *key_index.entry(key.clone()).or_insert_with(|| {
                let i = unique_keys.len();
                unique_keys.push(key);
                i
            });

            anim_placements.push(AnimPlacement {
                unique_index,
                offset_position: [trans.x, trans.y, trans.z],
                offset_rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                palette: placement.palette.clone(),
                entity_name: cga_entity_name(&cgf_path),
            });
        }
        container.placements = kept;
    }

    if anim_placements.is_empty() {
        return Vec::new();
    }

    // Phase 2: load each unique geometry once, in parallel (palette-independent —
    // the per-object tint palette is applied downstream from the EntityPayload's
    // `palette` field, so the same loaded asset is shared across placements).
    let loaded: Vec<Option<LoadedChildPayload>> = unique_keys
        .par_iter()
        .map(|(geometry_path, material_path)| {
            let node = ResolvedNode {
                entity_name: cga_entity_name(geometry_path),
                attachment_name: String::new(),
                no_rotation: false,
                offset_position: [0.0; 3],
                offset_rotation: [0.0; 3],
                detach_direction: [0.0; 3],
                port_flags: String::new(),
                nmc: None,
                bones: Vec::new(),
                has_geometry: true,
                record: placeholder_record(),
                geometry_path: Some(geometry_path.clone()),
                material_path: material_path.clone(),
                allows_child_object_containers: false,
                children: Vec::new(),
            };
            load_child_payload_asset(&node, db, p4k, opts, opts.material_mode, None)
        })
        .collect();

    // Phase 3: build one EntityPayload per placement, sharing the loaded geometry.
    let mut children = Vec::with_capacity(anim_placements.len());
    for ap in anim_placements {
        let Some(loaded_payload) = loaded[ap.unique_index].as_ref() else {
            continue;
        };
        children.push(EntityPayload {
            mesh: loaded_payload.mesh.clone(),
            materials: loaded_payload.materials.clone(),
            textures: loaded_payload.textures.clone(),
            nmc: loaded_payload.nmc.clone(),
            // Keep the placement's per-object tint palette if it had one.
            palette: ap.palette.or_else(|| loaded_payload.palette.clone()),
            geometry_path: loaded_payload.geometry_path.clone(),
            material_path: loaded_payload.material_path.clone(),
            bones: loaded_payload.bones.clone(),
            skeleton_source_path: loaded_payload.skeleton_source_path.clone(),
            entity_name: ap.entity_name,
            entity_category: None,
            attach_def_type: None,
            parent_node_name: String::new(),
            parent_entity_name: String::new(),
            no_rotation: false,
            offset_position: ap.offset_position,
            offset_rotation: ap.offset_rotation,
            detach_direction: [0.0; 3],
            port_flags: String::new(),
            ui_bindings: Vec::new(),
            parent_index: None,
        });
    }
    children
}

#[cfg(test)]
mod tests {
    use super::{cga_entity_name, chrparams_path_for_cga};

    #[test]
    fn chrparams_path_for_cga_maps_only_cga_geometry() {
        assert_eq!(
            chrparams_path_for_cga(
                "objects/buildingsets/human/hightech/delta/int/hangar/elev/ht_hangar_ship_elev_xl_a.cga"
            )
            .as_deref(),
            Some("objects/buildingsets/human/hightech/delta/int/hangar/elev/ht_hangar_ship_elev_xl_a.chrparams")
        );
        // static .cgf geometry has no animation chrparams
        assert_eq!(chrparams_path_for_cga("objects/walls/hangar_wall_a.cgf"), None);
        // case-insensitive extension match, stem case preserved
        assert_eq!(
            chrparams_path_for_cga("Objects/Elev/Door_A.CGA").as_deref(),
            Some("Objects/Elev/Door_A.chrparams")
        );
        // not geometry
        assert_eq!(chrparams_path_for_cga("foo/bar.dds"), None);
    }

    #[test]
    fn cga_entity_name_strips_dir_and_extension() {
        assert_eq!(cga_entity_name("a/b/ht_hangar_ship_elev_xl_a.cga"), "ht_hangar_ship_elev_xl_a");
        assert_eq!(cga_entity_name("Door_A.CGA"), "Door_A");
    }
}
