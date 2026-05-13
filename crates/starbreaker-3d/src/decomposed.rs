use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::Instant;

use gltf_json as json;
use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_dds;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::gltf::{offset_to_gltf_matrix, GlbBuilder, PackedMeshInfo};
use crate::mtl::{MtlFile, SemanticTextureBinding, ShaderFamily, SubMaterial, TextureSemanticRole, TintPalette};
use crate::nmc::NodeMeshCombo;
use crate::pipeline::{
    DecomposedExport, ExportFormat, ExportOptions, ExportedFile, ExportedFileKind, InteriorCgfEntry,
    LoadedInteriors, MaterialMode,
    PngCache,
};
use crate::skeleton::Bone;
use crate::types::{EntityPayload, Mesh};

pub(crate) struct DecomposedInput {
    pub entity_name: String,
    pub geometry_path: String,
    pub material_path: String,
    pub root_mesh: Mesh,
    pub root_materials: Option<MtlFile>,
    pub root_nmc: Option<NodeMeshCombo>,
    pub root_palette: Option<TintPalette>,
    pub available_palettes: Vec<TintPalette>,
    pub root_bones: Vec<Bone>,
    pub root_skeleton_source_path: Option<String>,
    pub root_animation_controller: Option<crate::animation::AnimationControllerSource>,
    pub children: Vec<EntityPayload>,
    pub interiors: LoadedInteriors,
    /// All available paint variants for this entity, populated from SubGeometry entries.
    pub paint_variants: Vec<crate::mtl::PaintVariant>,
}

pub(crate) type ExistingInteriorAssetMap = HashMap<String, (String, Option<String>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TextureFlavor {
    Generic,
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextureExportRef {
    role: String,
    source_path: String,
    export_path: String,
    export_kind: String,
    texture_identity: Option<String>,
    alpha_semantic: Option<String>,
    derived_from_texture_identity: Option<String>,
    derived_from_semantic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerTextureExport {
    source_material_path: String,
    diffuse_export_path: Option<String>,
    normal_export_path: Option<String>,
    roughness_export_path: Option<String>,
    slot_exports: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExtractedMaterialEntry {
    slot_exports: Vec<serde_json::Value>,
    direct_texture_exports: Vec<TextureExportRef>,
    layer_exports: Vec<LayerTextureExport>,
    derived_texture_exports: Vec<TextureExportRef>,
}

#[derive(Debug, Clone)]
pub(crate) struct DecomposedMaterialView {
    pub(crate) mesh: Mesh,
    pub(crate) sidecar_materials: Option<MtlFile>,
    /// Original (pre-filter) index in the source `.mtl` for each entry in `sidecar_materials`.
    /// Empty when `sidecar_materials` is `None`; identity mapping (0, 1, 2, …) when no
    /// materials were hidden.
    sidecar_original_indices: Vec<u32>,
    pub(crate) glb_materials: Option<MtlFile>,
    pub(crate) glb_nmc: Option<NodeMeshCombo>,
}

#[derive(Debug, Clone)]
struct SceneInstanceRecord {
    entity_name: String,
    geometry_path: String,
    material_path: String,
    mesh_asset: String,
    material_sidecar: Option<String>,
    palette_id: Option<String>,
    parent_node_name: Option<String>,
    parent_entity_name: Option<String>,
    source_transform_basis: Option<String>,
    local_transform_sc: Option<[[f32; 4]; 4]>,
    resolved_no_rotation: bool,
    no_rotation: bool,
    offset_position: [f32; 3],
    offset_rotation: [f32; 3],
    detach_direction: [f32; 3],
    port_flags: String,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedChildTransform {
    local_transform_sc: [[f32; 4]; 4],
    resolved_no_rotation: bool,
}

fn identity_flat_4x4() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn empty_scene_graph_mesh() -> Mesh {
    Mesh {
        positions: Vec::new(),
        indices: Vec::new(),
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: Vec::new(),
        model_min: [0.0; 3],
        model_max: [0.0; 3],
        scaling_min: [0.0; 3],
        scaling_max: [0.0; 3],
    }
}

fn flat_4x4_to_rows(flat: [f32; 16]) -> [[f32; 4]; 4] {
    [
        [flat[0], flat[1], flat[2], flat[3]],
        [flat[4], flat[5], flat[6], flat[7]],
        [flat[8], flat[9], flat[10], flat[11]],
        [flat[12], flat[13], flat[14], flat[15]],
    ]
}

fn resolve_no_rotation_local_matrix(
    parent_world_matrix: [f32; 16],
    offset_position: [f32; 3],
    offset_rotation: [f32; 3],
) -> [f32; 16] {
    let parent_world = glam::Mat4::from_cols_array(&parent_world_matrix);
    let parent_rotation = glam::Quat::from_mat4(&parent_world);
    let desired_matrix = glam::Mat4::from_cols_array(
        &offset_to_gltf_matrix(offset_position, offset_rotation).unwrap_or(identity_flat_4x4()),
    );
    let desired_rotation = glam::Quat::from_mat4(&desired_matrix);
    let desired_translation = glam::Vec3::from(offset_position);
    let rotated_offset = parent_world.transform_vector3(desired_translation);
    let parent_translation = parent_world.w_axis.truncate();
    let duplicate_offset = offset_rotation.iter().all(|value| value.abs() <= 1e-6)
        && (rotated_offset - parent_translation).abs().max_element() <= 5e-4;
    let local_translation = if duplicate_offset {
        glam::Vec3::ZERO
    } else {
        desired_translation
    };
    glam::Mat4::from_rotation_translation(parent_rotation.inverse() * desired_rotation, local_translation)
        .to_cols_array()
}

fn node_parent_indices(nodes: &[json::Node]) -> Vec<Option<usize>> {
    let mut parent_of = vec![None; nodes.len()];
    for (parent_index, node) in nodes.iter().enumerate() {
        if let Some(children) = &node.children {
            for child in children {
                let child_index = child.value();
                if child_index < parent_of.len() {
                    parent_of[child_index] = Some(parent_index);
                }
            }
        }
    }
    parent_of
}

fn docking_host_world_matrix(
    builder: &GlbBuilder,
    target_idx: usize,
) -> Option<[f32; 16]> {
    let parent_of = node_parent_indices(&builder.nodes_json);
    let parent_idx = parent_of.get(target_idx).and_then(|idx| *idx)?;
    let siblings = builder.nodes_json.get(parent_idx)?.children.as_ref()?;
    siblings
        .iter()
        .filter_map(|sibling| {
            let index = sibling.value();
            let name = builder.nodes_json.get(index)?.name.as_deref()?.to_ascii_lowercase();
            Some((index, name))
        })
        .find(|(_, name)| name.contains("docking_host"))
        .or_else(|| {
            siblings
                .iter()
                .filter_map(|sibling| {
                    let index = sibling.value();
                    let name = builder.nodes_json.get(index)?.name.as_deref()?.to_ascii_lowercase();
                    Some((index, name))
                })
                .find(|(_, name)| name.contains("docking_door"))
        })
        .map(|(index, _)| builder.compute_node_world_matrix(index))
}

fn child_docking_vehicle_translation(nmc: Option<&NodeMeshCombo>) -> Option<glam::Vec3> {
    nmc?.nodes.iter().find_map(|node| {
        node.name.to_ascii_lowercase().contains("docking_vehicle").then(|| {
            glam::Vec3::new(
                node.bone_to_world[0][3],
                node.bone_to_world[1][3],
                node.bone_to_world[2][3],
            )
        })
    })
}

fn docking_entity_attachment_offset(
    builder: &GlbBuilder,
    target_idx: usize,
    child: &crate::types::EntityPayload,
) -> Option<glam::Vec3> {
    // Vehicle docking entity attachments align the child vehicle attach helper
    // to a sibling docking host/door helper, not to the item-port node origin.
    if !child
        .port_flags
        .split_whitespace()
        .any(|flag| flag.eq_ignore_ascii_case("Docking_Request_Accepting"))
    {
        return None;
    }
    let target_world = glam::Mat4::from_cols_array(&builder.compute_node_world_matrix(target_idx));
    let host_world = glam::Mat4::from_cols_array(&docking_host_world_matrix(builder, target_idx)?);
    let child_attach = child_docking_vehicle_translation(child.nmc.as_ref())?;
    let desired_world = (host_world.w_axis
        - glam::Vec4::new(child_attach.x, child_attach.y, child_attach.z, 0.0))
    .truncate();
    Some(target_world.inverse().transform_point3(desired_world))
}

fn resolve_child_instance_transforms(input: &DecomposedInput) -> Vec<ResolvedChildTransform> {
    let mut builder = GlbBuilder::new();
    let dummy_packed = PackedMeshInfo {
        mesh_idx: 0,
        pos_accessor_idx: 0,
        uv_accessor_idx: None,
        secondary_uv_accessor_idx: None,
        normal_accessor_idx: None,
        color_accessor_idx: None,
        tangent_accessor_idx: None,
        submesh_mat_indices: Vec::new(),
        submesh_idx_accessors: Vec::new(),
    };

    let scene_nodes = if let Some(root_nmc) = input.root_nmc.as_ref().filter(|nmc| !nmc.nodes.is_empty()) {
        builder
            .build_nmc_hierarchy(&dummy_packed, root_nmc, &input.root_mesh.submeshes, false)
            .into_iter()
            .map(json::Index::new)
            .collect::<Vec<_>>()
    } else {
        builder.nodes_json.push(json::Node {
            name: Some(input.entity_name.clone()),
            ..Default::default()
        });
        vec![json::Index::new(0)]
    };

    builder.attach_skeleton_bones(&input.root_bones, &scene_nodes);

    let mut load_textures = |_materials: Option<&crate::mtl::MtlFile>, _palette: Option<&crate::mtl::TintPalette>| {
        None
    };
    let mut resolved = Vec::with_capacity(input.children.len());

    for child in &input.children {
        let resolved_local_matrix = if child.no_rotation {
            let target_idx = builder
                .node_name_to_idx
                .get(&child.parent_node_name.to_lowercase())
                .copied()
                .or_else(|| builder.node_name_to_idx.get(&child.parent_entity_name.to_lowercase()).copied())
                .or_else(|| scene_nodes.first().map(|node| node.value() as u32))
                .unwrap_or(0);
            let mut offset_position = child.offset_position;
            if let Some(docking_offset) = docking_entity_attachment_offset(&builder, target_idx as usize, child) {
                offset_position = [docking_offset.x, docking_offset.y, docking_offset.z];
            }
            Some(resolve_no_rotation_local_matrix(
                builder.compute_node_world_matrix(target_idx as usize),
                offset_position,
                child.offset_rotation,
            ))
        } else {
            None
        };

        let child_idx = builder.attach_child_entity(
            crate::types::EntityPayload {
                mesh: empty_scene_graph_mesh(),
                materials: None,
                textures: None,
                nmc: child.nmc.clone(),
                palette: None,
                geometry_path: child.geometry_path.clone(),
                material_path: child.material_path.clone(),
                bones: child.bones.clone(),
                skeleton_source_path: child.skeleton_source_path.clone(),
                entity_name: child.entity_name.clone(),
                entity_category: child.entity_category.clone(),
                parent_node_name: child.parent_node_name.clone(),
                parent_entity_name: child.parent_entity_name.clone(),
                no_rotation: child.no_rotation,
                offset_position: child.offset_position,
                offset_rotation: child.offset_rotation,
                detach_direction: child.detach_direction,
                port_flags: child.port_flags.clone(),
            },
            &scene_nodes,
            MaterialMode::None,
            None,
            &mut load_textures,
            resolved_local_matrix,
        );

        let local_transform_sc = flat_4x4_to_rows(
            builder.nodes_json[child_idx as usize]
                .matrix
                .unwrap_or_else(identity_flat_4x4),
        );
        resolved.push(ResolvedChildTransform {
            local_transform_sc,
            resolved_no_rotation: child.no_rotation,
        });
    }

    resolved
}

#[derive(Debug, Clone)]
struct InteriorPlacementRecord {
    cgf_path: String,
    material_path: Option<String>,
    mesh_asset: String,
    material_sidecar: Option<String>,
    entity_class_guid: Option<String>,
    transform: [[f32; 4]; 4],
    /// Per-placement tint palette id that overrides the container's palette.
    /// Populated for loadout-attached children that carry their own palette
    /// (e.g. `kegr_red_black` on a fire-extinguisher tank).
    palette_id: Option<String>,
}

#[derive(Debug, Clone)]
struct InteriorContainerRecord {
    name: String,
    parent_entity_name: Option<String>,
    parent_node_name: Option<String>,
    palette_id: Option<String>,
    container_transform: [[f32; 4]; 4],
    placements: Vec<InteriorPlacementRecord>,
    lights: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct PaletteRecord {
    id: String,
    palette: TintPalette,
    decal_texture_export_path: Option<String>,
}

#[derive(Debug, Clone)]
struct LiveryUsage {
    palette_id: String,
    palette_source_name: Option<String>,
    entity_names: BTreeSet<String>,
    material_sidecars: BTreeSet<String>,
}

fn export_entity_basename(name: &str) -> &str {
    let trimmed = name.trim_matches('"');
    trimmed.rsplit('.').next().unwrap_or(trimmed)
}

fn clean_export_label(name: &str) -> String {
    let mut cleaned = String::new();
    let mut last_was_space = false;

    for ch in name.chars() {
        if ch.is_alphanumeric() {
            cleaned.push(ch);
            last_was_space = false;
        } else if ch.is_whitespace() || matches!(ch, '_' | '-') {
            if !cleaned.is_empty() && !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        }
    }

    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        export_entity_basename(name).replace('_', " ")
    } else {
        cleaned.to_string()
    }
}

fn package_directory_name(entity_name: &str, lod: u32, mip: u32) -> String {
    format!(
        "{}_LOD{}_TEX{}",
        clean_export_label(export_entity_basename(entity_name)),
        lod,
        mip,
    )
}

fn package_relative_path(package_name: &str, file_name: &str) -> String {
    format!("Packages/{package_name}/{file_name}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineGlowTargetRecord {
    entity_name: String,
    geometry_path: String,
    mesh_asset: String,
    material_sidecar: String,
    source_material_index: u32,
    submaterial_name: String,
    blender_material_name: String,
}

fn build_thruster_engine_glow_targets(
    mesh: &Mesh,
    materials: Option<&MtlFile>,
    material_sidecar: Option<&str>,
    sidecar_original_indices: &[u32],
    entity_name: &str,
    geometry_path: &str,
    mesh_asset: &str,
) -> Vec<EngineGlowTargetRecord> {
    let Some(materials) = materials else {
        return Vec::new();
    };
    let Some(material_sidecar) = material_sidecar else {
        return Vec::new();
    };
    let source_indices: HashSet<u32> = mesh
        .submeshes
        .iter()
        .map(|submesh| submesh.source_material_id.unwrap_or(submesh.material_id))
        .collect();
    if source_indices.is_empty() {
        return Vec::new();
    }
    let source_stem = materials
        .source_path
        .as_deref()
        .unwrap_or(material_sidecar)
        .rsplit('/')
        .next()
        .unwrap_or(material_sidecar)
        .strip_suffix(".mtl")
        .unwrap_or(material_sidecar);
    let blender_material_names = preferred_blender_material_names(&materials.materials, source_stem);
    materials
        .materials
        .iter()
        .enumerate()
        .filter_map(|(filtered_index, material)| {
            let source_index = sidecar_original_indices
                .get(filtered_index)
                .copied()
                .unwrap_or(filtered_index as u32);
            if !source_indices.contains(&source_index) {
                return None;
            }
            if !is_engine_glow_material(material) {
                return None;
            }
            Some(EngineGlowTargetRecord {
                entity_name: entity_name.to_string(),
                geometry_path: normalize_requested_source_path(geometry_path),
                mesh_asset: mesh_asset.to_string(),
                material_sidecar: material_sidecar.to_string(),
                source_material_index: source_index,
                submaterial_name: material.name.clone(),
                blender_material_name: blender_material_names
                    .get(filtered_index)
                    .cloned()
                    .unwrap_or_else(|| material.name.clone()),
            })
        })
        .collect()
}

fn is_engine_glow_material(material: &SubMaterial) -> bool {
    if !matches!(material.shader_family(), ShaderFamily::Illum | ShaderFamily::HardSurface) {
        return false;
    }
    let emissive_factor = material.emissive_factor();
    let has_emissive_energy = emissive_factor.iter().any(|component| *component > 0.0) || material.glow > 0.0;
    let has_emissive_texture = material.texture_slots.iter().any(|slot| {
        let lowered = slot.path.to_ascii_lowercase();
        lowered.contains("glow") || lowered.contains("emissive")
    });
    has_emissive_energy || has_emissive_texture
}

fn normalize_package_subdir(subdir: &str) -> Option<String> {
    let normalized = subdir.replace('\\', "/");
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

pub(crate) fn build_decomposed_material_view(
    mesh: &Mesh,
    materials: Option<&MtlFile>,
    nmc: Option<&NodeMeshCombo>,
    include_nodraw: bool,
    include_shields: bool,
) -> DecomposedMaterialView {
    let Some(materials) = materials else {
        let filtered_mesh = filter_mesh_geometry(mesh, None, nmc, include_nodraw, include_shields);
        let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
        return DecomposedMaterialView {
            mesh: filtered_mesh,
            sidecar_materials: None,
            sidecar_original_indices: Vec::new(),
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    };

    if include_nodraw {
        let filtered_mesh = filter_mesh_geometry(mesh, Some(materials), nmc, include_nodraw, include_shields);
        let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
        let identity_indices = (0..materials.materials.len() as u32).collect();
        return DecomposedMaterialView {
            mesh: filtered_mesh,
            sidecar_materials: Some(materials.clone()),
            sidecar_original_indices: identity_indices,
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    }

    let mut material_id_map = Vec::with_capacity(materials.materials.len());
    let mut filtered_materials = Vec::with_capacity(materials.materials.len());
    for (orig_idx, material) in materials.materials.iter().enumerate() {
        if material.should_hide() {
            log::debug!(
                "[material-map] filtering out mat_id={} ({}), reason={}",
                orig_idx,
                material.name,
                if material.is_nodraw { "NoDraw" } else { "opacity" }
            );
            material_id_map.push(None);
        } else {
            material_id_map.push(Some(filtered_materials.len() as u32));
            filtered_materials.push(material.clone());
        }
    }

    // Compute which original indices survived the hide-filter.  These become the
    // sidecar indices so that every CGF sharing the same source `.mtl` writes
    // identical sidecar content regardless of which submaterials it actually uses.
    let sidecar_original_indices: Vec<u32> = material_id_map
        .iter()
        .enumerate()
        .filter_map(|(orig_idx, mapped)| if mapped.is_some() { Some(orig_idx as u32) } else { None })
        .collect();

    // Save the non-hidden (post-hide-filter, pre-used-compaction) material list.
    // This is used as sidecar_materials so the sidecar content is stable across
    // all meshes that share the same source .mtl.
    let non_hidden_materials = MtlFile {
        materials: filtered_materials.clone(),
        source_path: materials.source_path.clone(),
        paint_override: materials.paint_override.clone(),
        material_set: materials.material_set.clone(),
    };

    let mut dropped_out_of_range = false;
    let mut filtered_mesh = mesh.clone();
    
    // Collect surviving submeshes first
    let surviving_submeshes: Vec<(usize, crate::types::SubMesh)> = mesh
        .submeshes
        .iter()
        .enumerate()
        .filter_map(|(orig_idx, submesh)| {
            if submesh_is_excluded_helper(submesh, nmc, include_nodraw, include_shields) {
                return None;
            }

            let source_material_id = submesh.source_material_id.unwrap_or(submesh.material_id);
            let Some(mapped) = material_id_map.get(source_material_id as usize) else {
                dropped_out_of_range = true;
                log::debug!(
                    "[submesh-filter] out-of-range: submesh mat_id={}, material_id_map.len={}",
                    source_material_id,
                    material_id_map.len()
                );
                return None;
            };
            let Some(new_material_id) = *mapped else {
                log::debug!(
                    "[submesh-filter] mat_id={} maps to None (material was hidden)",
                    source_material_id
                );
                return None;
            };

            let mut filtered = submesh.clone();
            // Preserve the original source material index before any remapping so the
            // GLB extras `submaterial_index` stays aligned with the sidecar index.
            filtered.source_material_id = Some(source_material_id);
            filtered.material_id = new_material_id;
            if let Some(material) = filtered_materials.get(new_material_id as usize) {
                filtered.material_name = Some(material.name.clone());
            }
            log::debug!(
                "[submesh-kept] source_mat={} -> remapped_mat={} ({}), num_indices={}, first_index={}, num_vertices={}",
                source_material_id,
                new_material_id,
                filtered.material_name.as_deref().unwrap_or("?"),
                submesh.num_indices,
                submesh.first_index,
                submesh.num_vertices
            );
            Some((orig_idx, filtered))
        })
        .collect();
    
    // Rebuild the mesh to actually remove the geometry from filtered submeshes.
    // Only attempt the rebuild when every surviving submesh's index range lies
    // within the mesh's indices buffer — degenerate or partial meshes (no
    // indices at all, or submeshes whose `first_index + num_indices` exceeds
    // the buffer) skip the rebuild and surface the surviving submeshes as-is.
    let needs_rebuild = surviving_submeshes.len() < mesh.submeshes.len();
    let all_ranges_in_bounds = surviving_submeshes.iter().all(|(orig_idx, _)| {
        let sm = &mesh.submeshes[*orig_idx];
        sm.first_index as usize + sm.num_indices as usize <= mesh.indices.len()
    });
    if needs_rebuild && all_ranges_in_bounds {
        let mut new_indices = Vec::new();
        let mut index_offset = 0u32;
        let mut new_submeshes = Vec::new();
        for (orig_idx, mut submesh) in surviving_submeshes {
            let orig_range_start = mesh.submeshes[orig_idx].first_index as usize;
            let orig_range_end = orig_range_start + mesh.submeshes[orig_idx].num_indices as usize;
            new_indices.extend_from_slice(&mesh.indices[orig_range_start..orig_range_end]);
            submesh.first_index = index_offset;
            index_offset += submesh.num_indices;
            new_submeshes.push(submesh);
        }
        filtered_mesh.indices = new_indices;
        filtered_mesh.submeshes = new_submeshes;
        log::debug!(
            "[mesh-rebuild] indices: {} -> {} (removed {} bytes of orphaned geometry)",
            mesh.indices.len(),
            filtered_mesh.indices.len(),
            mesh.indices.len() - filtered_mesh.indices.len()
        );
    } else {
        filtered_mesh.submeshes = surviving_submeshes.into_iter().map(|(_, sm)| sm).collect();
    }
    
    log::debug!(
        "[mesh-filtering-result] submeshes: {} before -> {} after filtering",
        mesh.submeshes.len(),
        filtered_mesh.submeshes.len()
    );
    
    // Keep sidecar + GLB material indices aligned with the surviving primitive
    // set by removing materials no remaining submesh references.
    let used_material_ids: BTreeSet<u32> = filtered_mesh
        .submeshes
        .iter()
        .map(|submesh| submesh.material_id)
        .collect();
    if used_material_ids.len() < filtered_materials.len() {
        let mut compacted = Vec::with_capacity(used_material_ids.len());
        let mut remap: Vec<Option<u32>> = vec![None; filtered_materials.len()];
        for (old_index, material) in filtered_materials.iter().enumerate() {
            if used_material_ids.contains(&(old_index as u32)) {
                let new_index = compacted.len() as u32;
                remap[old_index] = Some(new_index);
                compacted.push(material.clone());
            }
        }

        filtered_mesh.submeshes.retain_mut(|submesh| {
            let Some(Some(new_material_id)) = remap.get(submesh.material_id as usize) else {
                dropped_out_of_range = true;
                return false;
            };
            submesh.material_id = *new_material_id;
            if let Some(material) = compacted.get(*new_material_id as usize) {
                submesh.material_name = Some(material.name.clone());
            }
            true
        });

        filtered_materials = compacted;
    }

    if dropped_out_of_range {
        log::warn!(
            "decomposed mesh references out-of-range material ids; dropping invalid submeshes for {}",
            materials
                .source_path
                .as_deref()
                .unwrap_or("<unknown material source>")
        );
    }

    if filtered_materials.len() == materials.materials.len()
        && filtered_mesh.submeshes.len() == mesh.submeshes.len()
        && !dropped_out_of_range
    {
        let identity_indices = (0..materials.materials.len() as u32).collect();
        let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
        return DecomposedMaterialView {
            mesh: filtered_mesh,
            sidecar_materials: Some(materials.clone()),
            sidecar_original_indices: identity_indices,
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    }

    let glb_materials = MtlFile {
        materials: filtered_materials,
        source_path: materials.source_path.clone(),
        paint_override: materials.paint_override.clone(),
        material_set: materials.material_set.clone(),
    };

    let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);

    DecomposedMaterialView {
        mesh: filtered_mesh,
        sidecar_materials: Some(non_hidden_materials),
        sidecar_original_indices,
        glb_materials: Some(glb_materials),
        glb_nmc: filtered_nmc,
    }
}

fn filter_mesh_geometry(
    mesh: &Mesh,
    materials: Option<&MtlFile>,
    nmc: Option<&NodeMeshCombo>,
    include_nodraw: bool,
    include_shields: bool,
) -> Mesh {
    if include_shields && include_nodraw {
        return mesh.clone();
    }

    let mut filtered_mesh = mesh.clone();
    filtered_mesh.submeshes = mesh
        .submeshes
        .iter()
        .filter(|submesh| {
            if let Some(materials) = materials {
                let source_material_id = submesh.source_material_id.unwrap_or(submesh.material_id);
                if let Some(material) = materials.materials.get(source_material_id as usize) {
                    if material.should_hide() && !include_nodraw {
                        log::debug!(
                            "[geometry-filter] dropping submesh {}: num_indices={}, source_mat_id={} ({}), reason={}",
                            submesh.material_id,
                            submesh.num_indices,
                            source_material_id,
                            material.name,
                            if material.is_nodraw { "NoDraw" } else { "opacity" }
                        );
                        return false;
                    }
                }
            }
            !submesh_is_excluded_helper(submesh, nmc, include_nodraw, include_shields)
        })
        .cloned()
        .collect();
    filtered_mesh
}

fn filter_nmc_hierarchy(
    mut mesh: Mesh,
    nmc: Option<&NodeMeshCombo>,
    include_nodraw: bool,
    include_shields: bool,
) -> (Mesh, Option<NodeMeshCombo>) {
    let Some(nmc) = nmc else {
        return (mesh, None);
    };
    if nmc.nodes.is_empty() {
        return (mesh, None);
    }

    let excluded_nodes = nmc
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            helper_name_is_excluded(&node.name, include_nodraw, include_shields).then_some(index)
        })
        .collect::<std::collections::HashSet<_>>();

    mesh.submeshes.retain(|submesh| {
        let index = submesh.node_parent_index as usize;
        index < nmc.nodes.len() && !excluded_nodes.contains(&index)
    });

    let kept_nodes = (0..nmc.nodes.len())
        .filter(|index| !excluded_nodes.contains(index))
        .collect::<std::collections::BTreeSet<_>>();

    if kept_nodes.is_empty() {
        return (
            mesh,
            Some(NodeMeshCombo {
                nodes: Vec::new(),
                material_indices: Vec::new(),
            }),
        );
    }

    let remap = kept_nodes
        .iter()
        .enumerate()
        .map(|(new_index, old_index)| (*old_index, new_index as u16))
        .collect::<std::collections::HashMap<_, _>>();

    for submesh in &mut mesh.submeshes {
        if let Some(node_parent_index) = remap.get(&(submesh.node_parent_index as usize)) {
            submesh.node_parent_index = *node_parent_index;
        }
    }

    let filtered_nmc = NodeMeshCombo {
        nodes: kept_nodes
            .iter()
            .map(|old_index| {
                let mut node = nmc.nodes[*old_index].clone();
                node.parent_index = node
                    .parent_index
                    .and_then(|parent_index| remap.get(&(parent_index as usize)).copied());
                node
            })
            .collect(),
        material_indices: kept_nodes
            .iter()
            .map(|old_index| *nmc.material_indices.get(*old_index).unwrap_or(&0))
            .collect(),
    };

    (mesh, Some(filtered_nmc))
}

fn submesh_is_excluded_helper(
    submesh: &crate::types::SubMesh,
    nmc: Option<&NodeMeshCombo>,
    include_nodraw: bool,
    include_shields: bool,
) -> bool {
    submesh
        .material_name
        .as_deref()
        .is_some_and(|value| helper_name_is_excluded(value, include_nodraw, include_shields))
        || nmc
            .and_then(|combo| combo.nodes.get(submesh.node_parent_index as usize))
            .is_some_and(|node| helper_name_is_excluded(&node.name, include_nodraw, include_shields))
}

fn helper_name_is_excluded(value: &str, include_nodraw: bool, _include_shields: bool) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .any(|segment| {
            let lowered = segment.to_ascii_lowercase();
            !include_nodraw
                && (lowered == "nodraw"
                    || lowered == "proxy"
                    || lowered.starts_with("proxy")
                    || lowered == "localgrid")
        })
}

pub(crate) fn write_decomposed_export(
    p4k: &MappedP4k,
    input: DecomposedInput,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
    existing_interior_assets: Option<&ExistingInteriorAssetMap>,
    load_interior_mesh: &mut dyn FnMut(
        &InteriorCgfEntry,
    ) -> Option<(Mesh, Option<MtlFile>, Option<NodeMeshCombo>)>,
) -> Result<DecomposedExport, Error> {
    const ROOT_ASSETS_START: f32 = 0.01;
    const CHILD_ASSETS_START: f32 = 0.16;
    const CHILD_ASSETS_END: f32 = 0.38;
    const INTERIOR_ASSETS_END: f32 = 0.99;

    let mut files = BTreeMap::new();
    let mut texture_cache: HashMap<(String, TextureFlavor), String> = HashMap::new();
    let mut mtl_cache: HashMap<String, Option<MtlFile>> = HashMap::new();
    let mut png_cache = PngCache::new();
    let mut palette_records = BTreeMap::new();
    let mut livery_usage = BTreeMap::new();
    let package_leaf = package_directory_name(&input.entity_name, opts.lod_level, opts.texture_mip);
    let package_name = if let Some(subdir) = opts
        .decomposed_package_subdir
        .as_deref()
        .and_then(normalize_package_subdir)
    {
        format!("{subdir}/{package_leaf}")
    } else {
        package_leaf
    };
    let scene_manifest_path = package_relative_path(&package_name, "scene.json");
    let palettes_manifest_path = package_relative_path(&package_name, "palettes.json");
    let liveries_manifest_path = package_relative_path(&package_name, "liveries.json");
    let total_start = Instant::now();
    let mut phase_start = Instant::now();
    report_progress(progress, ROOT_ASSETS_START, "Writing root assets");
    for palette in &input.available_palettes {
        register_palette(&mut palette_records, palette);
    }

    let root_material_view = build_decomposed_material_view(
        &input.root_mesh,
        input.root_materials.as_ref(),
        input.root_nmc.as_ref(),
        opts.include_nodraw,
        opts.include_shields,
    );

    let root_mesh_asset = write_mesh_asset(
        &mut files,
        p4k,
        &input.entity_name,
        &input.geometry_path,
        &root_material_view.mesh,
        root_material_view.glb_materials.as_ref(),
        root_material_view.glb_nmc.as_ref(),
        &input.root_bones,
        opts.lod_level,
        opts.format,
        existing_asset_paths,
    )?;
    let root_material_sidecar = root_material_view.sidecar_materials.as_ref().map(|materials| {
        write_material_sidecar(
            &mut files,
            p4k,
            &mut png_cache,
            &mut texture_cache,
            &palettes_manifest_path,
            &input.entity_name,
            &input.geometry_path,
            &input.material_path,
            materials,
            &root_material_view.sidecar_original_indices,
            opts.texture_mip,
            existing_asset_paths,
            &mut mtl_cache,
        )
    });
    let mut engine_glow_targets = Vec::new();
    let root_palette_id = input
        .root_palette
        .as_ref()
        .map(|palette| register_palette(&mut palette_records, palette));
    register_livery_usage(
        &mut livery_usage,
        root_palette_id.as_deref(),
        input.root_palette.as_ref(),
        &input.entity_name,
        root_material_sidecar.as_deref(),
    );
    log::info!("[timing][decomposed] root_assets: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    // Export material sidecars for each paint variant and build the paints.json manifest.
    let mut paint_variant_json: Vec<serde_json::Value> = Vec::new();
    for variant in &input.paint_variants {
        register_paint_variant_palette(&mut palette_records, variant);
        let Some(palette_id) = variant.palette_id.as_ref() else { continue };
        let sidecar_path = variant.materials.as_ref().map(|materials| {
            let variant_material_path = variant
                .material_path
                .as_deref()
                .unwrap_or(&input.material_path);
            let identity: Vec<u32> = (0..materials.materials.len() as u32).collect();
            write_material_sidecar(
                &mut files,
                p4k,
                &mut png_cache,
                &mut texture_cache,
                &palettes_manifest_path,
                &input.entity_name,
                &input.geometry_path,
                variant_material_path,
                materials,
                &identity,
                opts.texture_mip,
                existing_asset_paths,
                &mut mtl_cache,
            )
        });
        paint_variant_json.push(serde_json::json!({
            "subgeometry_tag": variant.subgeometry_tag,
            "palette_id": palette_id,
            "display_name": variant.display_name,
            "exterior_material_sidecar": sidecar_path,
        }));
    }
    if !paint_variant_json.is_empty() {
        let paints_manifest_path = package_relative_path(&package_name, "paints.json");
        insert_json_file(
            &mut files,
            paints_manifest_path,
            serde_json::json!({
                "version": 1,
                "paint_variants": paint_variant_json,
            }),
        );
    }
    log::info!("[timing][decomposed] paint_variants: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    report_progress(progress, CHILD_ASSETS_START, "Writing child assets");

    let resolved_child_transforms = resolve_child_instance_transforms(&input);
    let mut child_instances = Vec::with_capacity(input.children.len());
    let child_count = input.children.len();
    for (index, child) in input.children.iter().enumerate() {
        let child_material_view = build_decomposed_material_view(
            &child.mesh,
            child.materials.as_ref(),
            child.nmc.as_ref(),
            opts.include_nodraw,
            opts.include_shields,
        );
        let mesh_asset = write_mesh_asset(
            &mut files,
            p4k,
            &child.entity_name,
            &child.geometry_path,
            &child_material_view.mesh,
            child_material_view.glb_materials.as_ref(),
            child_material_view.glb_nmc.as_ref(),
            &child.bones,
            opts.lod_level,
            opts.format,
            existing_asset_paths,
        )?;
        let material_sidecar = child_material_view.sidecar_materials.as_ref().map(|materials| {
            write_material_sidecar(
                &mut files,
                p4k,
                &mut png_cache,
                &mut texture_cache,
                &palettes_manifest_path,
                &child.entity_name,
                &child.geometry_path,
                &child.material_path,
                materials,
                &child_material_view.sidecar_original_indices,
                opts.texture_mip,
                existing_asset_paths,
                &mut mtl_cache,
            )
        });
        if child
            .entity_category
            .as_deref()
            .is_some_and(|category| category.eq_ignore_ascii_case("Thruster"))
        {
            engine_glow_targets.extend(build_thruster_engine_glow_targets(
                &child.mesh,
                child_material_view.sidecar_materials.as_ref(),
                material_sidecar.as_deref(),
                &child_material_view.sidecar_original_indices,
                &child.entity_name,
                &child.geometry_path,
                &mesh_asset,
            ));
        }
        let palette_id = child
            .palette
            .as_ref()
            .map(|palette| register_palette(&mut palette_records, palette));
        register_livery_usage(
            &mut livery_usage,
            palette_id.as_deref(),
            child.palette.as_ref(),
            &child.entity_name,
            material_sidecar.as_deref(),
        );

        let resolved_transform = resolved_child_transforms[index];
        child_instances.push(SceneInstanceRecord {
            entity_name: child.entity_name.clone(),
            geometry_path: normalize_source_path(p4k, &child.geometry_path),
            material_path: normalize_source_path(p4k, &child.material_path),
            mesh_asset,
            material_sidecar,
            palette_id,
            parent_node_name: Some(child.parent_node_name.clone()),
            parent_entity_name: Some(child.parent_entity_name.clone()),
            source_transform_basis: Some("gltf_y_up".to_string()),
            local_transform_sc: Some(resolved_transform.local_transform_sc),
            resolved_no_rotation: resolved_transform.resolved_no_rotation,
            no_rotation: child.no_rotation,
            offset_position: child.offset_position,
            offset_rotation: child.offset_rotation,
            detach_direction: child.detach_direction,
            port_flags: child.port_flags.clone(),
        });

        if child_count > 0 {
            let fraction = (index + 1) as f32 / child_count as f32;
            report_progress(
                progress,
                CHILD_ASSETS_START + (CHILD_ASSETS_END - CHILD_ASSETS_START) * fraction,
                "Writing child assets",
            );
        }
    }
    let mut dedupe = HashSet::new();
    engine_glow_targets.retain(|target| {
        dedupe.insert((
            target.geometry_path.clone(),
            target.mesh_asset.clone(),
            target.material_sidecar.clone(),
            target.source_material_index,
        ))
    });
    engine_glow_targets.sort_by(|a, b| {
        a.geometry_path
            .cmp(&b.geometry_path)
            .then(a.material_sidecar.cmp(&b.material_sidecar))
            .then(a.source_material_index.cmp(&b.source_material_index))
    });
    if child_count == 0 {
        report_progress(progress, CHILD_ASSETS_END, "Writing interior assets");
    }
    log::info!("[timing][decomposed] child_assets: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    let mut interior_asset_cache: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut failed_interior_asset_cache: HashSet<String> = HashSet::new();
    let mut interior_records = Vec::with_capacity(input.interiors.containers.len());
    let mut interior_placement_elapsed = std::time::Duration::ZERO;
    let mut interior_light_elapsed = std::time::Duration::ZERO;
    let container_count = input.interiors.containers.len();
    let total_interior_placements = input
        .interiors
        .containers
        .iter()
        .map(|container| container.placements.len())
        .sum::<usize>();
    let mut processed_interior_placements = 0usize;
    for (index, container) in input.interiors.containers.iter().enumerate() {
        let palette_id = container
            .palette
            .as_ref()
            .map(|palette| register_palette(&mut palette_records, palette));
        let mut placements = Vec::with_capacity(container.placements.len());
        let placement_start = Instant::now();
        for (cgf_idx, transform, placement_palette) in &container.placements {
            let entry = &input.interiors.unique_cgfs[*cgf_idx];
            // Per-placement palette override (loadout-attached children like
            // fire-extinguisher tanks with their own `kegr_red_black` palette)
            // takes precedence over the container's palette. Register it in
            // the manifest so the addon can look it up by id.
            let placement_palette_id = placement_palette
                .as_ref()
                .map(|palette| register_palette(&mut palette_records, palette));
            let effective_palette_id = placement_palette_id
                .clone()
                .or_else(|| palette_id.clone());
            let effective_palette_ref = placement_palette
                .as_ref()
                .or(container.palette.as_ref());
            let normalized_cgf_path = normalize_source_path(p4k, &entry.cgf_path);
            let normalized_material_path = entry
                .material_path
                .as_deref()
                .map(|path| normalize_source_path(p4k, path));
            let cache_key = interior_asset_lookup_key(&normalized_cgf_path, normalized_material_path.as_deref());
            if failed_interior_asset_cache.contains(&cache_key) {
                processed_interior_placements += 1;
                if total_interior_placements > 0 {
                    let fraction =
                        processed_interior_placements as f32 / total_interior_placements as f32;
                    report_progress(
                        progress,
                        CHILD_ASSETS_END + (INTERIOR_ASSETS_END - CHILD_ASSETS_END) * fraction,
                        "Writing interior assets",
                    );
                }
                continue;
            }
            let (mesh_asset, material_sidecar) = if let Some(cached) = interior_asset_cache.get(&cache_key) {
                cached.clone()
            } else {
                let existing_reusable = existing_interior_asset_paths(
                    existing_interior_assets,
                    existing_asset_paths,
                    &cache_key,
                );
                let computed_reusable = if existing_reusable.is_none() {
                    reusable_interior_asset_paths(
                        p4k,
                        entry,
                        opts.lod_level,
                        opts.texture_mip,
                        opts.format,
                        existing_asset_paths,
                    )
                } else {
                    None
                };
                if let Some(reusable) = existing_reusable.or(computed_reusable) {
                    interior_asset_cache.insert(cache_key.clone(), reusable.clone());
                    reusable
                } else {
                    let Some((mesh, materials, _nmc)) = load_interior_mesh(entry) else {
                        log::warn!("failed to build decomposed interior asset for {}", entry.cgf_path);
                        failed_interior_asset_cache.insert(cache_key);
                        processed_interior_placements += 1;
                        if total_interior_placements > 0 {
                            let fraction =
                                processed_interior_placements as f32 / total_interior_placements as f32;
                            report_progress(
                                progress,
                                CHILD_ASSETS_END + (INTERIOR_ASSETS_END - CHILD_ASSETS_END) * fraction,
                                "Writing interior assets",
                            );
                        }
                        continue;
                    };
                    let interior_material_view = build_decomposed_material_view(
                        &mesh,
                        materials.as_ref(),
                        None,
                        opts.include_nodraw,
                        opts.include_shields,
                    );
                    log::debug!(
                        "[interior-asset] {} submeshes: {} before -> {} after filtering",
                        entry.name,
                        mesh.submeshes.len(),
                        interior_material_view.mesh.submeshes.len()
                    );
                    let requested_mesh_asset = mesh_asset_relative_path(
                        p4k,
                        &entry.cgf_path,
                        &entry.name,
                        opts.lod_level,
                        opts.format,
                    );
                    let requested_material_sidecar = interior_material_view.sidecar_materials.as_ref().map(|materials| {
                        let source_material_path = material_source_path(
                            p4k,
                            materials,
                            entry.material_path.as_deref().unwrap_or(""),
                            &entry.cgf_path,
                        );
                        material_sidecar_relative_path(&source_material_path, &entry.name, opts.texture_mip)
                    });
                    let material_sidecar = interior_material_view.sidecar_materials.as_ref().map(|materials| {
                        write_material_sidecar(
                            &mut files,
                            p4k,
                            &mut png_cache,
                            &mut texture_cache,
                            &palettes_manifest_path,
                            &entry.name,
                            &entry.cgf_path,
                            entry.material_path.as_deref().unwrap_or(""),
                            materials,
                            &interior_material_view.sidecar_original_indices,
                            opts.texture_mip,
                            existing_asset_paths,
                            &mut mtl_cache,
                        )
                    });
                    let reuse_existing_mesh_asset = (files.contains_key(&requested_mesh_asset)
                        || existing_asset_paths.is_some_and(|paths| paths.contains(&requested_mesh_asset.to_ascii_lowercase())))
                        && requested_material_sidecar
                            .as_ref()
                            .is_none_or(|requested_path| material_sidecar.as_deref() == Some(requested_path.as_str()));
                    let mesh_asset = if reuse_existing_mesh_asset {
                        requested_mesh_asset
                    } else {
                        write_mesh_asset(
                            &mut files,
                            p4k,
                            &entry.name,
                            &entry.cgf_path,
                            &interior_material_view.mesh,
                            interior_material_view.glb_materials.as_ref(),
                            // Interior meshes already follow the bundled flat-mesh path.
                            // Preserving the raw NMC hierarchy here makes decomposed interiors
                            // diverge from the reference import and can double-apply placement transforms.
                            interior_material_view.glb_nmc.as_ref(),
                            &[],
                            opts.lod_level,
                            opts.format,
                            existing_asset_paths,
                        )?
                    };
                    interior_asset_cache.insert(cache_key, (mesh_asset.clone(), material_sidecar.clone()));
                    (mesh_asset, material_sidecar)
                }
            };

            register_livery_usage(
                &mut livery_usage,
                effective_palette_id.as_deref(),
                effective_palette_ref,
                &entry.name,
                material_sidecar.as_deref(),
            );

            placements.push(InteriorPlacementRecord {
                cgf_path: normalize_source_path(p4k, &entry.cgf_path),
                material_path: entry
                    .material_path
                    .as_ref()
                    .map(|path| normalize_source_path(p4k, path)),
                mesh_asset,
                material_sidecar,
                entity_class_guid: None,
                transform: *transform,
                palette_id: placement_palette_id,
            });
            processed_interior_placements += 1;
            if total_interior_placements > 0 {
                let fraction =
                    processed_interior_placements as f32 / total_interior_placements as f32;
                report_progress(
                    progress,
                    CHILD_ASSETS_END + (INTERIOR_ASSETS_END - CHILD_ASSETS_END) * fraction,
                    "Writing interior assets",
                );
            }
        }
        interior_placement_elapsed += placement_start.elapsed();

        let mut lights = Vec::with_capacity(container.lights.len());
        let light_start = Instant::now();
        for light in &container.lights {
            // Extract the projector (gobo) texture.
            // ONLY for Projector lights (spot lights). Point lights (Omni) should never have gobos.
            // Priority: EXR (for HDR formats like BC6H) -> PNG (for SDR formats) -> white PNG fallback
            // EXR preserves float values >1.0, allowing Blender to sample full HDR energy.
            let projector_texture_export = if light.light_type == "Projector" {
                light.projector_texture.as_deref().and_then(|src| {
                    let normalized = normalize_source_path(p4k, src);
                    let exr_path = replace_extension(&normalized, ".exr");
                    if existing_asset_paths
                        .is_some_and(|paths| paths.contains(&exr_path.to_ascii_lowercase()))
                    {
                        return Some(exr_path);
                    }

                    // Try HDR EXR export first (for BC6H gobos with values >1.0)
                    if let Some(exr_data) = export_gobo_as_exr(p4k, src, opts.texture_mip) {
                        return Some(insert_binary_file(&mut files, exr_path, exr_data));
                    }

                    // Fall back to standard PNG export (for SDR or unsupported formats)
                    if let Some(png_path) = export_texture_asset(
                        &mut files,
                        p4k,
                        &mut png_cache,
                        &mut texture_cache,
                        src,
                        TextureFlavor::Generic,
                        opts.texture_mip,
                        existing_asset_paths,
                    ) {
                        return Some(png_path);
                    }

                    // Both EXR and PNG failed. Log a warning and use white PNG fallback.
                    log::warn!(
                        "Failed to export projector texture '{}' (EXR and PNG both failed). Using white PNG fallback.",
                        src
                    );
                    let normalized = normalize_source_path(p4k, src);
                    let fallback_path = replace_extension(&normalized, ".png");
                    let fallback_png = create_white_png_fallback();
                    Some(insert_binary_file(&mut files, fallback_path, fallback_png))
                })
            } else {
                None
            };
            lights.push(serde_json::json!({
                "name": light.name,
                "position": light.position,
                "transform_basis": light.transform_basis,
                "rotation": light.rotation,
                "direction_sc": light.direction_sc,
                "color": light.color,
                "light_type": light.light_type,
                "semantic_light_kind": light.semantic_light_kind,
                "intensity_raw": light.intensity_raw,
                "intensity_unit": light.intensity_unit,
                "intensity_candela_proxy": light.intensity_candela_proxy,
                "intensity": light.intensity,
                "radius": light.radius,
                "radius_m": light.radius_m,
                "inner_angle": light.inner_angle,
                "outer_angle": light.outer_angle,
                "projector_texture": projector_texture_export,
                "active_state": light.active_state,
                "states": light
                    .states
                    .iter()
                    .map(|(name, s)| {
                        (
                            name.clone(),
                            serde_json::json!({
                                "intensity_raw": s.intensity_raw,
                                "intensity_unit": s.intensity_unit,
                                "intensity_cd": s.intensity_cd,
                                "intensity_candela_proxy": s.intensity_candela_proxy,
                                "temperature": s.temperature,
                                "use_temperature": s.use_temperature,
                                "color": s.color,
                                "light_style": s.light_style,
                                "preset_tag": s.preset_tag,
                            }),
                        )
                    })
                    .collect::<serde_json::Map<_, _>>(),
            }));
        }
        interior_light_elapsed += light_start.elapsed();

        interior_records.push(InteriorContainerRecord {
            name: container.name.clone(),
            parent_entity_name: container.parent_entity_name.clone(),
            parent_node_name: container.parent_node_name.clone(),
            palette_id,
            container_transform: container.container_transform,
            placements,
            lights,
        });

        if container_count > 0 && total_interior_placements == 0 {
            let fraction = (index + 1) as f32 / container_count as f32;
            report_progress(
                progress,
                CHILD_ASSETS_END + (INTERIOR_ASSETS_END - CHILD_ASSETS_END) * fraction,
                "Writing interior assets",
            );
        }
    }
    if container_count == 0 {
        report_progress(progress, INTERIOR_ASSETS_END, "Writing manifests");
    }
    log::info!(
        "[timing][decomposed] interior_placements: {:.2}s",
        interior_placement_elapsed.as_secs_f32()
    );
    log::info!(
        "[timing][decomposed] interior_lights: {:.2}s",
        interior_light_elapsed.as_secs_f32()
    );
    log::info!("[timing][decomposed] interior_assets: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    let root_animations = if opts.include_animations {
        let mut clips: Vec<serde_json::Value> = Vec::new();
        // Map from clip name → index in `clips`, used to merge same-named clips
        // from different child skeletons (e.g. landing_gear_extend from front/left/right CHRs).
        let mut name_to_index = std::collections::HashMap::<String, usize>::new();

        let mut append_from_skeleton = |skeleton_path: &str, include_unmatched: bool, allow_bone_subset_fallback: bool| {
            match crate::animation::extract_animations_for_skeleton_json(p4k, skeleton_path, include_unmatched, allow_bone_subset_fallback) {
                Ok(Some(serde_json::Value::Array(values))) => {
                    for mut clip in values {
                        let name = clip
                            .get("name")
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .to_string();
                        if name.is_empty() {
                            clips.push(clip);
                        } else if let Some(&existing_idx) = name_to_index.get(&name) {
                            // Merge bone channels from this clip into the existing one.
                            if let (Some(serde_json::Value::Object(new_bones)), Some(existing_clip)) =
                                (clip.get_mut("bones").map(|b| b.take()), clips.get_mut(existing_idx))
                            {
                                if let Some(serde_json::Value::Object(existing_bones)) =
                                    existing_clip.get_mut("bones")
                                {
                                    for (k, v) in new_bones {
                                        if let Some(existing_value) = existing_bones.get_mut(&k) {
                                            merge_animation_channel_values(existing_value, v, &name, &k);
                                        } else {
                                            existing_bones.insert(k, v);
                                        }
                                    }
                                }
                            }
                        } else {
                            let idx = clips.len();
                            name_to_index.insert(name, idx);
                            clips.push(clip);
                        }
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => {}
                Err(error) => {
                    log::warn!(
                        "[anim] failed to extract animations for skeleton '{}': {}",
                        skeleton_path,
                        error
                    );
                }
            }
        };

        if let Some(skeleton_path) = input.root_skeleton_source_path.as_deref() {
            append_from_skeleton(skeleton_path, true, false);
        }
        for child in &input.children {
            if let Some(skeleton_path) = child.skeleton_source_path.as_deref() {
                append_from_skeleton(skeleton_path, false, true);
            }
        }

        if clips.is_empty() {
            None
        } else {
            if let Some(source) = input.root_animation_controller.as_ref() {
                if let Err(error) = crate::animation::annotate_animation_fragments_json(p4k, &mut clips, source) {
                    log::warn!("[anim] failed to annotate Mannequin fragments: {error}");
                }
            }
            // Phase 35: split each clip into a lightweight index record
            // (kept inline in `scene.json`) and a heavy sidecar body
            // written to `Packages/<entity>/animations/<clip>.json`.
            // Deduplicate sidecar filenames in case two clips end up
            // sanitizing to the same name.
            let mut index_records: Vec<serde_json::Value> = Vec::with_capacity(clips.len());
            let mut used_filenames: std::collections::HashSet<String> = std::collections::HashSet::new();
            for clip in clips.iter() {
                let raw_name = clip
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("clip")
                    .to_string();
                let mut base = crate::animation::sanitize_clip_filename(&raw_name);
                let mut suffix = 1u32;
                while used_filenames.contains(&base) {
                    suffix += 1;
                    base = format!(
                        "{}_{}",
                        crate::animation::sanitize_clip_filename(&raw_name),
                        suffix
                    );
                }
                used_filenames.insert(base.clone());
                let sidecar_relative = format!("animations/{base}.json");
                let sidecar_path = package_relative_path(&package_name, &sidecar_relative);
                let (index, body) =
                    crate::animation::split_clip_for_sidecar(clip, &sidecar_relative);
                insert_json_file(&mut files, sidecar_path, body);
                index_records.push(index);
            }
            Some(serde_json::Value::Array(index_records))
        }
    } else {
        None
    };
    log::info!("[timing][decomposed] animations: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    let scene_manifest = build_scene_manifest_value(
        &input.entity_name,
        &package_name,
        &normalize_source_path(p4k, &input.geometry_path),
        &normalize_source_path(p4k, &input.material_path),
        &root_mesh_asset,
        root_material_sidecar.as_deref(),
        root_palette_id.as_deref(),
        root_animations.as_ref(),
        &child_instances,
        &interior_records,
        &engine_glow_targets,
        opts,
    );
    report_progress(progress, INTERIOR_ASSETS_END, "Writing manifests");
    finalize_palette_records(
        &mut palette_records,
        &mut files,
        p4k,
        &mut png_cache,
        &mut texture_cache,
        opts.texture_mip,
        existing_asset_paths,
    );
    insert_json_file(&mut files, scene_manifest_path, scene_manifest);
    insert_json_file(
        &mut files,
        palettes_manifest_path.clone(),
        build_palette_manifest_value(&palette_records),
    );
    insert_json_file(
        &mut files,
        liveries_manifest_path,
        build_livery_manifest_value(&livery_usage),
    );
    log::info!("[timing][decomposed] manifests: {:.2}s", phase_start.elapsed().as_secs_f32());
    log::info!("[timing][decomposed] total: {:.2}s", total_start.elapsed().as_secs_f32());

    Ok(DecomposedExport {
        files: files
            .into_iter()
            .map(|(relative_path, bytes)| ExportedFile {
                kind: classify_exported_file_kind(&relative_path),
                relative_path,
                bytes,
            })
            .collect(),
    })
}

fn classify_exported_file_kind(relative_path: &str) -> ExportedFileKind {
    if relative_path.ends_with(".materials.json") {
        ExportedFileKind::MaterialSidecar
    } else if relative_path.ends_with(".glb") || relative_path.ends_with(".blend") {
        ExportedFileKind::MeshAsset
    } else if relative_path.ends_with(".png") {
        ExportedFileKind::TextureAsset
    } else {
        ExportedFileKind::PackageManifest
    }
}

fn build_scene_manifest_value(
    entity_name: &str,
    package_name: &str,
    geometry_path: &str,
    material_path: &str,
    root_mesh_asset: &str,
    root_material_sidecar: Option<&str>,
    root_palette_id: Option<&str>,
    root_animations: Option<&serde_json::Value>,
    child_instances: &[SceneInstanceRecord],
    interiors: &[InteriorContainerRecord],
    engine_glow_targets: &[EngineGlowTargetRecord],
    opts: &ExportOptions,
) -> serde_json::Value {
    let mut manifest = serde_json::json!({
        "version": 1,
        "export_kind": "Decomposed",
        "package_rule": {
            "root": "caller_selected_export_root",
            "package_dir": format!("Packages/{package_name}"),
            "paths_are_relative_to_export_root": true,
            "shared_asset_root": "Data",
            "normalized_p4k_relative_paths": true,
        },
        "root_entity": {
            "entity_name": entity_name,
            "geometry_path": geometry_path,
            "material_path": material_path,
            "mesh_asset": root_mesh_asset,
            "material_sidecar": root_material_sidecar,
            "palette_id": root_palette_id,
        },
        "export_options": {
            "kind": format!("{:?}", opts.kind),
            "format": format!("{:?}", opts.format),
            "material_mode": format!("{:?}", opts.material_mode),
            "lod_level": opts.lod_level,
            "texture_mip": opts.texture_mip,
            "include_attachments": opts.include_attachments,
            "include_interior": opts.include_interior,
            "include_lights": opts.include_lights,
        },
        "children": child_instances.iter().map(scene_instance_json).collect::<Vec<_>>(),
        "interiors": interiors.iter().map(interior_container_json).collect::<Vec<_>>(),
    });

    if let Some(animations) = root_animations {
        manifest["root_entity"]["animations"] = animations.clone();
    }
    if !engine_glow_targets.is_empty() {
        manifest["controls"] = serde_json::json!({
            "engine_glow": {
                "label": "Engine Glow",
                "units": "emission_strength",
                "min_strength": 0.0,
                "max_strength": 200.0,
                "default_strength": 3.0,
                "targets": engine_glow_targets
                    .iter()
                    .map(|target| serde_json::json!({
                        "entity_name": target.entity_name,
                        "geometry_path": target.geometry_path,
                        "mesh_asset": target.mesh_asset,
                        "material_sidecar": target.material_sidecar,
                        "source_material_index": target.source_material_index,
                        "submaterial_name": target.submaterial_name,
                        "blender_material_name": target.blender_material_name,
                    }))
                    .collect::<Vec<_>>(),
            }
        });
    }

    manifest
}

fn build_palette_manifest_value(records: &BTreeMap<String, PaletteRecord>) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "palettes": records.values().map(|record| {
            serde_json::json!({
                "id": record.id,
                "source_name": record.palette.source_name,
                "display_name": record.palette.display_name,
                "primary": record.palette.primary,
                "secondary": record.palette.secondary,
                "tertiary": record.palette.tertiary,
                "glass": record.palette.glass,
                "decal": {
                    "red": record.palette.decal_color_r,
                    "green": record.palette.decal_color_g,
                    "blue": record.palette.decal_color_b,
                    "source_path": record.palette.decal_texture,
                    "export_path": record.decal_texture_export_path,
                },
                "finish": palette_finish_json(&record.palette.finish),
            })
        }).collect::<Vec<_>>(),
    })
}

fn palette_finish_json(finish: &crate::mtl::TintPaletteFinish) -> serde_json::Value {
    serde_json::json!({
        "primary": palette_finish_entry_json(&finish.primary),
        "secondary": palette_finish_entry_json(&finish.secondary),
        "tertiary": palette_finish_entry_json(&finish.tertiary),
        "glass": palette_finish_entry_json(&finish.glass),
    })
}

fn palette_finish_entry_json(entry: &crate::mtl::TintPaletteFinishEntry) -> serde_json::Value {
    serde_json::json!({
        "specular": entry.specular,
        "glossiness": entry.glossiness,
    })
}

fn paint_override_json(info: &crate::mtl::PaintOverrideInfo) -> serde_json::Value {
    serde_json::json!({
        "paint_item_name": info.paint_item_name,
        "subgeometry_tag": info.subgeometry_tag,
        "subgeometry_index": info.subgeometry_index,
        "material_path": info.material_path,
    })
}

fn authored_attributes_json(attributes: &[crate::mtl::AuthoredAttribute]) -> serde_json::Value {
    serde_json::Value::Array(
        attributes
            .iter()
            .map(|attribute| {
                serde_json::json!({
                    "name": attribute.name,
                    "value": attribute.value,
                })
            })
            .collect(),
    )
}

fn authored_blocks_json(blocks: &[crate::mtl::AuthoredBlock]) -> serde_json::Value {
    serde_json::Value::Array(blocks.iter().map(authored_block_json).collect())
}

fn authored_block_json(block: &crate::mtl::AuthoredBlock) -> serde_json::Value {
    serde_json::json!({
        "tag": block.tag,
        "attributes": authored_attributes_json(&block.attributes),
        "children": authored_blocks_json(&block.children),
    })
}

fn raw_public_params_json(params: &[crate::mtl::PublicParam]) -> serde_json::Value {
    serde_json::Value::Array(
        params
            .iter()
            .map(|param| {
                serde_json::json!({
                    "name": param.name,
                    "value": param.value,
                })
            })
            .collect(),
    )
}

fn build_livery_manifest_value(records: &BTreeMap<String, LiveryUsage>) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "liveries": records.values().map(|usage| {
            serde_json::json!({
                "id": usage.palette_id,
                "palette_id": usage.palette_id,
                "palette_source_name": usage.palette_source_name,
                "entity_names": usage.entity_names.iter().cloned().collect::<Vec<_>>(),
                "material_sidecars": usage.material_sidecars.iter().cloned().collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn scene_instance_json(instance: &SceneInstanceRecord) -> serde_json::Value {
    serde_json::json!({
        "entity_name": instance.entity_name,
        "geometry_path": instance.geometry_path,
        "material_path": instance.material_path,
        "mesh_asset": instance.mesh_asset,
        "material_sidecar": instance.material_sidecar,
        "palette_id": instance.palette_id,
        "parent_node_name": instance.parent_node_name,
        "parent_entity_name": instance.parent_entity_name,
        "source_transform_basis": instance.source_transform_basis,
        "local_transform_sc": instance.local_transform_sc,
        "resolved_no_rotation": instance.resolved_no_rotation,
        "no_rotation": instance.no_rotation,
        "offset_position": instance.offset_position,
        "offset_rotation": instance.offset_rotation,
        "detach_direction": instance.detach_direction,
        "port_flags": instance.port_flags,
    })
}

fn interior_container_json(container: &InteriorContainerRecord) -> serde_json::Value {
    serde_json::json!({
        "name": container.name,
        "parent_entity_name": container.parent_entity_name,
        "parent_node_name": container.parent_node_name,
        "palette_id": container.palette_id,
        "container_transform": container.container_transform,
        "placements": container.placements.iter().map(|placement| {
            serde_json::json!({
                "cgf_path": placement.cgf_path,
                "material_path": placement.material_path,
                "mesh_asset": placement.mesh_asset,
                "material_sidecar": placement.material_sidecar,
                "entity_class_guid": placement.entity_class_guid,
                "transform": placement.transform,
                "palette_id": placement.palette_id,
            })
        }).collect::<Vec<_>>(),
        "lights": container.lights,
    })
}

fn write_mesh_asset(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    fallback_name: &str,
    geometry_path: &str,
    _mesh: &Mesh,
    _materials: Option<&MtlFile>,
    _nmc: Option<&NodeMeshCombo>,
    _bones: &[Bone],
    lod_level: u32,
    format: ExportFormat,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<String, Error> {
    let requested_path = mesh_asset_relative_path(p4k, geometry_path, fallback_name, lod_level, format);
    if existing_asset_paths
        .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
    {
        return Ok(requested_path);
    }
    Ok(insert_binary_file(files, requested_path, Vec::new()))
}

fn write_material_sidecar(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    palettes_manifest_path: &str,
    fallback_name: &str,
    geometry_path: &str,
    material_path: &str,
    materials: &MtlFile,
    original_indices: &[u32],
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
    mtl_cache: &mut HashMap<String, Option<MtlFile>>,
) -> String {
    let source_material_path = material_source_path(p4k, materials, material_path, geometry_path);
    let relative_path = material_sidecar_relative_path(&source_material_path, fallback_name, texture_mip);
    if files.contains_key(&relative_path) {
        return relative_path;
    }
    let (sidecar_materials, sidecar_original_indices) = canonical_sidecar_materials_from_source(
        p4k,
        &source_material_path,
        materials,
        original_indices,
        mtl_cache,
    );
    let extracted = sidecar_materials
        .materials
        .iter()
        .map(|material| {
            extract_material_entry(
                files,
                p4k,
                png_cache,
                texture_cache,
                material,
                texture_mip,
                existing_asset_paths,
                mtl_cache,
            )
        })
        .collect::<Vec<_>>();
    let value = build_material_sidecar_value(
        &sidecar_materials,
        &source_material_path,
        &relative_path,
        palettes_manifest_path,
        &extracted,
        &sidecar_original_indices,
    );
    insert_json_file(files, relative_path, value)
}

fn canonical_sidecar_materials_from_source(
    p4k: &MappedP4k,
    source_material_path: &str,
    fallback_materials: &MtlFile,
    fallback_indices: &[u32],
    mtl_cache: &mut HashMap<String, Option<MtlFile>>,
) -> (MtlFile, Vec<u32>) {
    if let Some(parsed) = load_mtl_cached(p4k, mtl_cache, source_material_path) {
        let mut original_indices = Vec::new();
        let mut non_hidden = Vec::new();
        for (idx, material) in parsed.materials.into_iter().enumerate() {
            if material.should_hide() {
                continue;
            }
            original_indices.push(idx as u32);
            non_hidden.push(material);
        }
        let canonical = MtlFile {
            materials: non_hidden,
            source_path: parsed.source_path,
            paint_override: parsed.paint_override,
            material_set: parsed.material_set,
        };
        return (canonical, original_indices);
    }

    // Fallback path: preserve previous behaviour when we can't reload the source file.
    (fallback_materials.clone(), fallback_indices.to_vec())
}

fn load_mtl_cached(
    p4k: &MappedP4k,
    cache: &mut HashMap<String, Option<MtlFile>>,
    material_path: &str,
) -> Option<MtlFile> {
    let p4k_path = crate::pipeline::datacore_path_to_p4k(material_path);
    let cache_key = p4k_path.to_ascii_lowercase();
    if let Some(cached) = cache.get(&cache_key) {
        return cached.clone();
    }
    let loaded = crate::pipeline::try_load_mtl(p4k, &p4k_path);
    cache.insert(cache_key, loaded.clone());
    loaded
}

fn extract_material_entry(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    material: &SubMaterial,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
    mtl_cache: &mut HashMap<String, Option<MtlFile>>,
) -> ExtractedMaterialEntry {
    let semantic_slots = material.semantic_texture_slots();
    let slot_exports = semantic_slots
        .iter()
        .map(|binding| {
            build_slot_export_value(
                files,
                p4k,
                png_cache,
                texture_cache,
                binding,
                texture_mip,
                existing_asset_paths,
            )
        })
        .collect::<Vec<_>>();

    let mut direct_texture_exports = Vec::new();
    if let Some(path) = material.diffuse_tex.as_deref() {
        if let Some(export_path) = export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            path,
            TextureFlavor::Generic,
            texture_mip,
            existing_asset_paths,
        ) {
            direct_texture_exports.push(TextureExportRef {
                role: "diffuse".to_string(),
                source_path: normalize_source_path(p4k, path),
                export_path,
                export_kind: "source".to_string(),
                texture_identity: ddna_texture_identity(path).map(str::to_string),
                alpha_semantic: None,
                derived_from_texture_identity: None,
                derived_from_semantic: None,
            });
        }
    }
    if let Some(path) = material.normal_tex.as_deref() {
        if let Some(export_path) = export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            path,
            TextureFlavor::Normal,
            texture_mip,
            existing_asset_paths,
        ) {
            direct_texture_exports.push(TextureExportRef {
                role: "normal_gloss".to_string(),
                source_path: normalize_source_path(p4k, path),
                export_path,
                export_kind: "source".to_string(),
                texture_identity: ddna_texture_identity(path).map(str::to_string),
                alpha_semantic: ddna_alpha_semantic(path, TextureSemanticRole::NormalGloss).map(str::to_string),
                derived_from_texture_identity: None,
                derived_from_semantic: None,
            });
        }
    }

    let derived_texture_exports = Vec::new();

    let layer_exports = material
        .layers
        .iter()
        .map(|layer| {
            let layer_material_path = normalize_source_path(p4k, &layer.path);
            let layer_mtl = load_mtl_cached(p4k, mtl_cache, &layer.path);
            let layer_sub = layer_mtl
                .as_ref()
                .and_then(|mtl| crate::mtl::resolve_layer_submaterial(mtl, &layer.sub_material));
            let slot_exports = layer_sub
                .map(|sub| {
                    sub.semantic_texture_slots()
                        .iter()
                        .map(|binding| {
                            build_slot_export_value(
                                files,
                                p4k,
                                png_cache,
                                texture_cache,
                                binding,
                                texture_mip,
                                existing_asset_paths,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let diffuse_export_path = layer_sub
                .and_then(|sub| sub.diffuse_tex.as_deref())
                .and_then(|path| {
                    export_texture_asset(
                        files,
                        p4k,
                        png_cache,
                        texture_cache,
                        path,
                        TextureFlavor::Generic,
                        texture_mip,
                        existing_asset_paths,
                    )
                });
            let normal_path = layer_sub.and_then(|sub| sub.normal_tex.as_deref());
            let normal_export_path = normal_path.and_then(|path| {
                export_texture_asset(
                    files,
                    p4k,
                    png_cache,
                    texture_cache,
                    path,
                    TextureFlavor::Normal,
                    texture_mip,
                    existing_asset_paths,
                )
            });
            let roughness_export_path = None;

            LayerTextureExport {
                source_material_path: layer_material_path,
                diffuse_export_path,
                normal_export_path,
                roughness_export_path,
                slot_exports,
            }
        })
        .collect::<Vec<_>>();

    ExtractedMaterialEntry {
        slot_exports,
        direct_texture_exports,
        layer_exports,
        derived_texture_exports,
    }
}

fn build_material_sidecar_value(
    materials: &MtlFile,
    source_material_path: &str,
    relative_path: &str,
    palettes_manifest_path: &str,
    extracted: &[ExtractedMaterialEntry],
    original_indices: &[u32],
) -> serde_json::Value {
    let source_stem = source_material_path
        .rsplit('/')
        .next()
        .unwrap_or(source_material_path)
        .strip_suffix(".mtl")
        .unwrap_or(source_material_path);
    let blender_material_names = preferred_blender_material_names(&materials.materials, source_stem);

    serde_json::json!({
        "version": 1,
        "source_material_path": source_material_path,
        "normalized_export_relative_path": relative_path,
        "authored_material_set": {
            "attributes": authored_attributes_json(&materials.material_set.attributes),
            "public_params": raw_public_params_json(&materials.material_set.public_params),
            "child_blocks": authored_blocks_json(&materials.material_set.child_blocks),
        },
        "palette_contract": {
            "shared_manifest": palettes_manifest_path,
            "scene_instance_field": "palette_id",
        },
        "paint_override": materials.paint_override.as_ref().map(paint_override_json),
        "submaterials": materials.materials.iter().enumerate().map(|(i, material)| {
            let source_index = original_indices.get(i).copied().unwrap_or(i as u32);
            build_submaterial_json(
                material,
                source_material_path,
                source_stem,
                &blender_material_names[i],
                source_index as usize,
                &extracted[i],
            )
        }).collect::<Vec<_>>(),
    })
}

fn preferred_blender_material_names(materials: &[SubMaterial], source_stem: &str) -> Vec<String> {
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for material in materials {
        *name_counts.entry(material.name.as_str()).or_default() += 1;
    }

    materials
        .iter()
        .enumerate()
        .map(|(index, material)| {
            if name_counts.get(material.name.as_str()).copied().unwrap_or_default() > 1 {
                format!("{source_stem}:{}_{}", material.name, index)
            } else {
                format!("{source_stem}:{}", material.name)
            }
        })
        .collect()
}

fn build_submaterial_json(
    material: &SubMaterial,
    source_material_path: &str,
    source_stem: &str,
    blender_material_name: &str,
    index: usize,
    extracted: &ExtractedMaterialEntry,
) -> serde_json::Value {
    let semantic_slots = material.semantic_texture_slots();
    let decoded_flags = material.decoded_string_gen_mask();
    let (activation_state, activation_reason) = material_activation_state(material, &semantic_slots);
    let public_params = material
        .public_params
        .iter()
        .map(|param| (param.name.clone(), string_value_to_json(&param.value)))
        .collect::<serde_json::Map<_, _>>();
    let virtual_inputs = semantic_slots
        .iter()
        .filter(|binding| binding.is_virtual)
        .map(|binding| binding.path.clone())
        .collect::<Vec<_>>();

    serde_json::json!({
        "index": index,
        "submaterial_name": material.name,
        "blender_material_name": blender_material_name,
        "shader": material.shader,
        "shader_family": material.shader_family().as_str(),
        "authored_attributes": authored_attributes_json(&material.authored_attributes),
        "authored_public_params": raw_public_params_json(&material.public_params),
        "authored_child_blocks": authored_blocks_json(&material.authored_child_blocks),
        "activation_state": {
            "state": activation_state,
            "reason": activation_reason,
        },
        "decoded_feature_flags": {
            "tokens": decoded_flags.tokens,
            "has_decal": decoded_flags.has_decal,
            "has_parallax_occlusion_mapping": decoded_flags.has_parallax_occlusion_mapping,
            "has_stencil_map": decoded_flags.has_stencil_map,
            "has_iridescence": decoded_flags.has_iridescence,
            "has_vertex_colors": decoded_flags.has_vertex_colors,
        },
        "texture_slots": extracted.slot_exports,
        "virtual_inputs": virtual_inputs,
        "public_params": public_params,
        "direct_textures": extracted.direct_texture_exports.iter().map(texture_ref_json).collect::<Vec<_>>(),
        "derived_textures": extracted.derived_texture_exports.iter().map(texture_ref_json).collect::<Vec<_>>(),
        "layer_manifest": material.layers.iter().enumerate().map(|(layer_index, layer)| {
            let extracted_layer = extracted.layer_exports.get(layer_index);
            let palette_channel = palette_channel_json(layer.palette_tint, false);
            let layer_snapshot = layer.snapshot.as_ref().map(|snapshot| serde_json::json!({
                "shader": snapshot.shader,
                "diffuse": snapshot.diffuse,
                "specular": snapshot.specular,
                "shininess": snapshot.shininess,
                "wear_specular_color": snapshot.wear_specular_color,
                "wear_glossiness": snapshot.wear_glossiness,
                "surface_type": snapshot.surface_type,
                "metallic": snapshot.metallic,
            }));
            let resolved_material = layer.resolved_material.as_ref().map(|resolved| serde_json::json!({
                "name": resolved.name,
                "shader": resolved.shader,
                "shader_family": resolved.shader_family,
                "authored_attributes": authored_attributes_json(&resolved.authored_attributes),
                "authored_public_params": raw_public_params_json(&resolved.public_params),
                "authored_child_blocks": authored_blocks_json(&resolved.authored_child_blocks),
            }));
            serde_json::json!({
                "index": layer_index,
                "name": layer.name,
                "source_material_path": extracted_layer.map(|layer| layer.source_material_path.clone()).unwrap_or_else(|| layer.path.clone()),
                "submaterial_name": layer.sub_material,
                "resolved_material": resolved_material,
                "authored_attributes": authored_attributes_json(&layer.authored_attributes),
                "authored_child_blocks": authored_blocks_json(&layer.authored_child_blocks),
                "tint_color": layer.tint_color,
                "wear_tint": layer.wear_tint,
                "palette_channel": palette_channel,
                "gloss_mult": layer.gloss_mult,
                "wear_gloss": layer.wear_gloss,
                "uv_tiling": layer.uv_tiling,
                "height_bias": layer.height_bias,
                "height_scale": layer.height_scale,
                "layer_snapshot": layer_snapshot,
                "texture_slots": extracted_layer.map(|layer| layer.slot_exports.clone()).unwrap_or_default(),
                "diffuse_export_path": extracted_layer.and_then(|layer| layer.diffuse_export_path.clone()),
                "normal_export_path": extracted_layer.and_then(|layer| layer.normal_export_path.clone()),
                "roughness_export_path": extracted_layer.and_then(|layer| layer.roughness_export_path.clone()),
            })
        }).collect::<Vec<_>>(),
        "palette_routing": {
            "material_channel": palette_channel_json(material.palette_tint, material.is_glass()),
            "layer_channels": material.layers.iter().enumerate().filter_map(|(layer_index, layer)| {
                let channel = palette_channel_json(layer.palette_tint, false)?;
                Some(serde_json::json!({
                    "index": layer_index,
                    "channel": channel,
                }))
            }).collect::<Vec<_>>(),
        },
        "material_set_identity": {
            "source_path": source_material_path,
            "source_stem": source_stem,
            "submaterial_index": index,
            "submaterial_name": material.name,
        },
        "variant_membership": {
            "palette_routed": material.palette_tint > 0 || material.is_glass(),
            "layer_palette_routed": material.layers.iter().any(|layer| layer.palette_tint > 0),
            "layered": !material.layers.is_empty(),
        },
    })
}

fn build_slot_export_value(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    binding: &SemanticTextureBinding,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> serde_json::Value {
    let source_path = slot_source_path(Some(p4k), binding);
    let export_flavor = slot_texture_flavor(binding.role);
    let export_path = if binding.is_virtual {
        None
    } else {
        export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            &binding.path,
            export_flavor,
            texture_mip,
            existing_asset_paths,
        )
    };

    let mut value = serde_json::Map::from_iter([
        ("slot".to_string(), serde_json::json!(binding.slot)),
        ("role".to_string(), serde_json::json!(binding.role.as_str())),
        ("is_virtual".to_string(), serde_json::json!(binding.is_virtual)),
        ("source_path".to_string(), serde_json::json!(source_path)),
        ("export_path".to_string(), serde_json::json!(export_path)),
        ("export_kind".to_string(), serde_json::json!(texture_export_kind(export_flavor))),
        (
            "authored_attributes".to_string(),
            authored_attributes_json(&binding.authored_attributes),
        ),
        (
            "authored_child_blocks".to_string(),
            authored_blocks_json(&binding.authored_child_blocks),
        ),
    ]);
    if let Some(texture_identity) = ddna_texture_identity(&binding.path) {
        value.insert("texture_identity".to_string(), serde_json::json!(texture_identity));
    }
    if let Some(alpha_semantic) = ddna_alpha_semantic(&binding.path, binding.role) {
        value.insert("alpha_semantic".to_string(), serde_json::json!(alpha_semantic));
    }
    if let Some(texture_transform) = texture_transform_json(&binding.authored_child_blocks) {
        value.insert("texture_transform".to_string(), texture_transform);
    }
    serde_json::Value::Object(value)
}

fn slot_source_path(p4k: Option<&MappedP4k>, binding: &SemanticTextureBinding) -> String {
    if binding.is_virtual {
        binding.path.clone()
    } else {
        p4k.map(|archive| normalize_source_path(archive, &binding.path))
            .unwrap_or_else(|| normalize_requested_source_path(&binding.path))
    }
}

fn export_texture_asset(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    source_path: &str,
    flavor: TextureFlavor,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<String> {
    let normalized_source = normalize_source_path(p4k, source_path);
    let cache_key = (normalized_source.to_lowercase(), flavor);
    if let Some(existing) = texture_cache.get(&cache_key) {
        return Some(existing.clone());
    }

    let requested_path = texture_relative_path(p4k, source_path, flavor, texture_mip);
    if existing_asset_paths
        .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
    {
        texture_cache.insert(cache_key, requested_path.clone());
        return Some(requested_path);
    }

    let bytes = match flavor {
        TextureFlavor::Generic => crate::pipeline::cached_load(
            p4k,
            source_path,
            texture_mip,
            png_cache,
            crate::pipeline::load_diffuse_texture,
        ),
        TextureFlavor::Normal => crate::pipeline::cached_load(
            p4k,
            source_path,
            texture_mip,
            png_cache,
            crate::pipeline::load_normal_texture,
        ),
    }?;

    let stored_path = insert_binary_file(files, requested_path, bytes);
    texture_cache.insert(cache_key, stored_path.clone());
    Some(stored_path)
}

fn register_palette(records: &mut BTreeMap<String, PaletteRecord>, palette: &TintPalette) -> String {
    let id = palette_id(palette);
    register_palette_with_id(records, id.clone(), palette);
    id
}

fn register_palette_with_id(
    records: &mut BTreeMap<String, PaletteRecord>,
    id: String,
    palette: &TintPalette,
) {
    records.entry(id.clone()).or_insert_with(|| PaletteRecord {
        id,
        palette: palette.clone(),
        decal_texture_export_path: None,
    });
}

fn register_paint_variant_palette(
    records: &mut BTreeMap<String, PaletteRecord>,
    variant: &crate::mtl::PaintVariant,
) -> Option<String> {
    let palette_id = variant.palette_id.as_ref()?;
    let palette = variant.palette.as_ref()?;
    register_palette_with_id(records, palette_id.clone(), palette);
    Some(palette_id.clone())
}

fn finalize_palette_records(
    records: &mut BTreeMap<String, PaletteRecord>,
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) {
    for record in records.values_mut() {
        let Some(source_path) = record.palette.decal_texture.as_deref() else {
            continue;
        };
        record.decal_texture_export_path = export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            source_path,
            TextureFlavor::Generic,
            texture_mip,
            existing_asset_paths,
        );
    }
}

fn register_livery_usage(
    usages: &mut BTreeMap<String, LiveryUsage>,
    palette_id: Option<&str>,
    palette: Option<&TintPalette>,
    entity_name: &str,
    material_sidecar: Option<&str>,
) {
    let Some(palette_id) = palette_id else {
        return;
    };
    let entry = usages.entry(palette_id.to_string()).or_insert_with(|| LiveryUsage {
        palette_id: palette_id.to_string(),
        palette_source_name: palette.and_then(|palette| palette.source_name.clone()),
        entity_names: BTreeSet::new(),
        material_sidecars: BTreeSet::new(),
    });
    entry.entity_names.insert(entity_name.to_string());
    if let Some(material_sidecar) = material_sidecar {
        entry.material_sidecars.insert(material_sidecar.to_string());
    }
}

fn material_source_path(
    p4k: &MappedP4k,
    materials: &MtlFile,
    material_path: &str,
    geometry_path: &str,
) -> String {
    normalize_source_path(
        p4k,
        &material_source_request(materials, material_path, geometry_path),
    )
}

fn material_source_request(materials: &MtlFile, material_path: &str, geometry_path: &str) -> String {
    if let Some(source_path) = materials.source_path.as_ref() {
        source_path.clone()
    } else if !material_path.is_empty() {
        if material_path.rsplit('/').next().is_some_and(|name| name.contains('.')) {
            material_path.to_string()
        } else {
            format!("{material_path}.mtl")
        }
    } else if geometry_path.is_empty() {
        "Data/generated/generated.mtl".to_string()
    } else {
        replace_extension(geometry_path, ".mtl")
    }
}

fn requested_material_source_path(p4k: &MappedP4k, material_path: Option<&str>, geometry_path: &str) -> String {
    let request = if let Some(material_path) = material_path.filter(|path| !path.is_empty()) {
        if material_path.rsplit('/').next().is_some_and(|name| name.contains('.')) {
            material_path.to_string()
        } else {
            format!("{material_path}.mtl")
        }
    } else {
        replace_extension(geometry_path, ".mtl")
    };
    normalize_source_path(p4k, &request)
}

fn existing_asset_set_contains(existing_asset_paths: Option<&HashSet<String>>, relative_path: &str) -> bool {
    existing_asset_paths
        .is_some_and(|paths| paths.contains(&relative_path.to_ascii_lowercase()))
}

pub(crate) fn interior_asset_lookup_key(cgf_path: &str, material_path: Option<&str>) -> String {
    format!(
        "{}|{}",
        cgf_path.to_ascii_lowercase(),
        material_path.unwrap_or("").to_ascii_lowercase()
    )
}

fn existing_interior_asset_paths(
    existing_interior_assets: Option<&ExistingInteriorAssetMap>,
    existing_asset_paths: Option<&HashSet<String>>,
    lookup_key: &str,
) -> Option<(String, Option<String>)> {
    let (mesh_asset, material_sidecar) = existing_interior_assets?.get(lookup_key)?;
    if !existing_asset_set_contains(existing_asset_paths, mesh_asset) {
        return None;
    }
    if let Some(sidecar) = material_sidecar.as_deref() {
        if !existing_asset_set_contains(existing_asset_paths, sidecar) {
            return None;
        }
    }
    Some((mesh_asset.clone(), material_sidecar.clone()))
}

fn reusable_interior_asset_paths(
    p4k: &MappedP4k,
    entry: &InteriorCgfEntry,
    lod_level: u32,
    texture_mip: u32,
    format: ExportFormat,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<(String, Option<String>)> {
    let mesh_asset = mesh_asset_relative_path(p4k, &entry.cgf_path, &entry.name, lod_level, format);
    if !existing_asset_set_contains(existing_asset_paths, &mesh_asset) {
        return None;
    }

    let material_source = requested_material_source_path(p4k, entry.material_path.as_deref(), &entry.cgf_path);
    let material_sidecar = material_sidecar_relative_path(&material_source, &entry.name, texture_mip);
    if existing_asset_set_contains(existing_asset_paths, &material_sidecar) {
        Some((mesh_asset, Some(material_sidecar)))
    } else {
        None
    }
}

fn mesh_asset_extension(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Blend => ".blend",
        ExportFormat::Glb | ExportFormat::Stl => ".glb",
    }
}

pub(crate) fn mesh_asset_relative_path(
    p4k: &MappedP4k,
    geometry_path: &str,
    fallback_name: &str,
    lod: u32,
    format: ExportFormat,
) -> String {
    let extension = mesh_asset_extension(format);
    let base = if geometry_path.is_empty() {
        format!("Data/generated/{}{}", sanitize_identifier(fallback_name), extension)
    } else {
        replace_extension(&normalize_source_path(p4k, geometry_path), extension)
    };
    insert_stem_suffix(&base, &format!("_LOD{lod}"))
}

fn material_sidecar_relative_path(source_material_path: &str, fallback_name: &str, mip: u32) -> String {
    let base = if source_material_path.is_empty() {
        format!("Data/generated/{}.materials.json", sanitize_identifier(fallback_name))
    } else {
        replace_extension(source_material_path, ".materials.json")
    };
    insert_stem_suffix(&base, &format!("_TEX{mip}"))
}

fn texture_relative_path(p4k: &MappedP4k, source_path: &str, flavor: TextureFlavor, mip: u32) -> String {
    let normalized = normalize_source_path(p4k, source_path);
    let base = match flavor {
        TextureFlavor::Generic => replace_extension(&normalized, ".png"),
        TextureFlavor::Normal => replace_extension(&normalized, ".png"),
    };
    insert_stem_suffix(&base, &format!("_TEX{mip}"))
}

/// Insert `suffix` immediately before the file extension. For compound
/// extensions like `.materials.json` the suffix lands before the first
/// trailing extension segment so the full compound extension survives.
fn insert_stem_suffix(path: &str, suffix: &str) -> String {
    // Split off the filename from any directory prefix so suffixes never
    // inject into intermediate path components.
    let (dir, file) = match path.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f.to_string()),
        None => (String::new(), path.to_string()),
    };
    // Handle compound extensions by finding the first '.' in the filename.
    let (stem, ext) = match file.find('.') {
        Some(idx) => (&file[..idx], &file[idx..]),
        None => (file.as_str(), ""),
    };
    format!("{dir}{stem}{suffix}{ext}")
}

fn normalize_requested_source_path(path: &str) -> String {
    crate::pipeline::datacore_path_to_p4k(path).replace('\\', "/")
}

pub(crate) fn normalize_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = crate::pipeline::datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| normalize_requested_source_path(path))
}

pub(crate) fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
}

fn create_white_png_fallback() -> Vec<u8> {
    // Create a minimal 2x2 white PNG (1 byte per channel RGBA)
    // This allows Blender to load the image without errors or magenta display.
    // PNG signature + minimal IHDR + IDAT + IEND chunks.
    vec![
        // PNG signature
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        // IHDR chunk: 2x2 8-bit RGBA
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
        0xDE,
        // IDAT chunk: white 2x2 image (zlib compressed, then CRC)
        0x00, 0x00, 0x00, 0x1B, 0x49, 0x44, 0x41, 0x54,
        0x78, 0x9C, 0x62, 0xF8, 0xFF, 0xFF, 0x3F, 0x03,
        0x03, 0x03, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x09,
        0x00, 0x01, 0xBE, 0xCE, 0x66, 0xA9,
        // IEND chunk
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
        0xAE, 0x42, 0x60, 0x82,
    ]
}

fn export_gobo_as_exr(
    p4k: &MappedP4k,
    source_path: &str,
    texture_mip: u32,
) -> Option<Vec<u8>> {
    // Attempt to load and decode the DDS source texture as HDR (BC6H).
    // If successful, export to EXR format so Blender can sample values >1.0.
    // This preserves light energy for gobos that use HDR formats.
    
    // Look up the entry first
    let entry = p4k.entry_case_insensitive(source_path)?;
    let bytes = p4k.read(&entry).ok()?;
    
    // Parse DDS and check if it's BC6H
    let dds = starbreaker_dds::DdsFile::from_bytes(&bytes).ok()?;
    
    // Attempt BC6H float decode
    let (width, height, float_rgb) = match dds.decode_bc6h_to_float_rgb(texture_mip as usize) {
        Ok(Some(result)) => result,
        _ => return None, // Not BC6H or decode failed
    };
    
    if width == 0 || height == 0 {
        return None;
    }
    
    // Build EXR image from float RGB data using the exr crate API.
    use exr::prelude::*;
    use std::io::Cursor;
    
    // Split the interleaved float RGB data into per-channel vectors
    let mut r_channel = Vec::with_capacity((width as usize) * (height as usize));
    let mut g_channel = Vec::with_capacity((width as usize) * (height as usize));
    let mut b_channel = Vec::with_capacity((width as usize) * (height as usize));
    
    for chunk in float_rgb.chunks_exact(3) {
        r_channel.push(chunk[0]);
        g_channel.push(chunk[1]);
        b_channel.push(chunk[2]);
    }
    
    let channels: AnyChannels<FlatSamples> = AnyChannels::sort(vec![
        AnyChannel::new("R", FlatSamples::F32(r_channel)),
        AnyChannel::new("G", FlatSamples::F32(g_channel)),
        AnyChannel::new("B", FlatSamples::F32(b_channel)),
    ].into());
    
    let layer = Layer::new(
        Vec2(width as usize, height as usize),
        LayerAttributes::default(),
        Encoding::FAST_LOSSLESS,
        channels,
    );
    
    let image = Image::from_layer(layer);
    
    let buffer = Vec::new();
    let mut cursor = Cursor::new(buffer);
    match image.write().to_buffered(&mut cursor) {
        Ok(_) => Some(cursor.into_inner()),
        Err(e) => {
            log::warn!("Failed to encode gobo as EXR: {}", e);
            None
        }
    }
}

fn palette_id(palette: &TintPalette) -> String {
    if let Some(source_name) = palette.source_name.as_ref() {
        format!("palette/{}", sanitize_identifier(source_name))
    } else {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_vec3(&mut hasher, &palette.primary);
        hash_vec3(&mut hasher, &palette.secondary);
        hash_vec3(&mut hasher, &palette.tertiary);
        hash_vec3(&mut hasher, &palette.glass);
        hash_finish_entry(&mut hasher, &palette.finish.primary);
        hash_finish_entry(&mut hasher, &palette.finish.secondary);
        hash_finish_entry(&mut hasher, &palette.finish.tertiary);
        hash_finish_entry(&mut hasher, &palette.finish.glass);
        format!("palette/generated-{:016x}", hasher.finish())
    }
}

fn sanitize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn insert_json_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    requested_path: String,
    value: serde_json::Value,
) -> String {
    let bytes = serde_json::to_vec_pretty(&value).unwrap_or_else(|_| b"{}".to_vec());
    insert_binary_file(files, requested_path, bytes)
}

fn insert_binary_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    requested_path: String,
    bytes: Vec<u8>,
) -> String {
    let requested_path = canonicalize_output_path_case(files, &requested_path);
    if let Some(existing) = files.get(&requested_path) {
        if existing == &bytes {
            return requested_path;
        }
    }

    let mut candidate = requested_path.clone();
    while let Some(existing) = files.get(&candidate) {
        if existing == &bytes {
            return candidate;
        }
        candidate = hashed_variant_path(&requested_path, &bytes);
    }
    files.insert(candidate.clone(), bytes);
    candidate
}

fn canonicalize_output_path_case(files: &BTreeMap<String, Vec<u8>>, requested_path: &str) -> String {
    let mut prefixes = String::new();
    let mut canonical_parts = Vec::new();

    for (depth, part) in requested_path.split('/').enumerate() {
        if depth > 0 {
            prefixes.push('/');
        }
        prefixes.push_str(&part.to_ascii_lowercase());

        let canonical_part = files
            .keys()
            .find_map(|existing| existing_segment_case(existing, depth, &prefixes))
            .unwrap_or_else(|| part.to_string());
        canonical_parts.push(canonical_part);
    }

    canonical_parts.join("/")
}

fn existing_segment_case(path: &str, depth: usize, lowercase_prefix: &str) -> Option<String> {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() <= depth {
        return None;
    }
    let existing_prefix = parts[..=depth]
        .iter()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/");
    if existing_prefix == lowercase_prefix {
        Some(parts[depth].to_string())
    } else {
        None
    }
}

fn hashed_variant_path(path: &str, bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some((stem, ext)) = path.rsplit_once('.') {
        format!("{stem}-{hash:08x}.{ext}")
    } else {
        format!("{path}-{hash:08x}")
    }
}

fn material_activation_state(
    material: &SubMaterial,
    semantic_slots: &[SemanticTextureBinding],
) -> (&'static str, &'static str) {
    if material.is_nodraw {
        ("inactive", "nodraw")
    } else if material.should_hide() {
        ("inactive", "semantic_hidden")
    } else if material.is_decal() && !has_base_color_source(material, semantic_slots) {
        ("inactive", "missing_base_color_texture")
    } else {
        ("active", "visible")
    }
}

fn has_base_color_source(material: &SubMaterial, semantic_slots: &[SemanticTextureBinding]) -> bool {
    material.diffuse_tex.is_some()
        || !material.layers.is_empty()
        || semantic_slots.iter().any(|binding| {
            !binding.is_virtual
                && matches!(
                    binding.role,
                    TextureSemanticRole::BaseColor
                        | TextureSemanticRole::AlternateBaseColor
                        | TextureSemanticRole::DecalSheet
                        | TextureSemanticRole::Stencil
                        | TextureSemanticRole::PatternMask
                )
        })
}

fn palette_channel_json(channel: u8, is_glass: bool) -> Option<serde_json::Value> {
    match channel {
        1 => Some(serde_json::json!({ "index": 1, "name": "primary" })),
        2 => Some(serde_json::json!({ "index": 2, "name": "secondary" })),
        3 => Some(serde_json::json!({ "index": 3, "name": "tertiary" })),
        _ if is_glass => Some(serde_json::json!({ "index": 0, "name": "glass" })),
        _ => None,
    }
}

fn texture_ref_json(texture_ref: &TextureExportRef) -> serde_json::Value {
    let mut value = serde_json::Map::from_iter([
        ("role".to_string(), serde_json::json!(texture_ref.role)),
        ("source_path".to_string(), serde_json::json!(texture_ref.source_path)),
        ("export_path".to_string(), serde_json::json!(texture_ref.export_path)),
        ("export_kind".to_string(), serde_json::json!(texture_ref.export_kind)),
    ]);
    if let Some(texture_identity) = &texture_ref.texture_identity {
        value.insert("texture_identity".to_string(), serde_json::json!(texture_identity));
    }
    if let Some(alpha_semantic) = &texture_ref.alpha_semantic {
        value.insert("alpha_semantic".to_string(), serde_json::json!(alpha_semantic));
    }
    if let Some(texture_identity) = &texture_ref.derived_from_texture_identity {
        value.insert(
            "derived_from_texture_identity".to_string(),
            serde_json::json!(texture_identity),
        );
    }
    if let Some(derived_from_semantic) = &texture_ref.derived_from_semantic {
        value.insert(
            "derived_from_semantic".to_string(),
            serde_json::json!(derived_from_semantic),
        );
    }
    serde_json::Value::Object(value)
}

fn ddna_texture_identity(path: &str) -> Option<&'static str> {
    if path.to_ascii_lowercase().contains("_ddna") {
        Some("ddna_normal")
    } else {
        None
    }
}

fn ddna_alpha_semantic(path: &str, role: TextureSemanticRole) -> Option<&'static str> {
    if ddna_texture_identity(path).is_some() && matches!(role, TextureSemanticRole::NormalGloss) {
        Some("smoothness")
    } else {
        None
    }
}

fn texture_transform_json(blocks: &[crate::mtl::AuthoredBlock]) -> Option<serde_json::Value> {
    let texmod = blocks.iter().find(|block| block.tag == "TexMod")?;
    let attributes = texmod
        .attributes
        .iter()
        .map(|attribute| {
            (
                attribute.name.clone(),
                string_value_to_json(&attribute.value),
            )
        })
        .collect::<serde_json::Map<_, _>>();

    let mut value = serde_json::Map::from_iter([(
        "attributes".to_string(),
        serde_json::Value::Object(attributes),
    )]);
    if let Some(scale) = texmod_pair(&texmod.attributes, "TileU", "TileV") {
        value.insert("scale".to_string(), serde_json::json!(scale));
    }
    if let Some(offset) = texmod_pair(&texmod.attributes, "OffsetU", "OffsetV") {
        value.insert("offset".to_string(), serde_json::json!(offset));
    }
    if !texmod.children.is_empty() {
        value.insert("children".to_string(), authored_blocks_json(&texmod.children));
    }
    Some(serde_json::Value::Object(value))
}

fn texmod_pair(
    attributes: &[crate::mtl::AuthoredAttribute],
    first: &str,
    second: &str,
) -> Option<[f32; 2]> {
    let first_value = texmod_float(attributes, first)?;
    let second_value = texmod_float(attributes, second)?;
    Some([first_value, second_value])
}

fn texmod_float(attributes: &[crate::mtl::AuthoredAttribute], name: &str) -> Option<f32> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.value.parse::<f32>().ok())
}

fn slot_texture_flavor(role: TextureSemanticRole) -> TextureFlavor {
    match role {
        TextureSemanticRole::NormalGloss => TextureFlavor::Normal,
        _ => TextureFlavor::Generic,
    }
}

fn texture_export_kind(flavor: TextureFlavor) -> &'static str {
    match flavor {
        TextureFlavor::Generic => "source",
        TextureFlavor::Normal => "source",
    }
}

fn string_value_to_json(value: &str) -> serde_json::Value {
    if value.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(integer) = value.parse::<i64>() {
        return serde_json::json!(integer);
    }
    if let Ok(float) = value.parse::<f64>() {
        return serde_json::json!(float);
    }
    serde_json::json!(value)
}

fn merge_animation_channel_values(
    existing_value: &mut serde_json::Value,
    incoming_value: serde_json::Value,
    clip_name: &str,
    channel_key: &str,
) {
    if *existing_value == incoming_value {
        return;
    }

    if let Some(existing_variants) = existing_value.as_array_mut() {
        if !existing_variants.iter().any(|variant| *variant == incoming_value) {
            existing_variants.push(incoming_value);
        }
        return;
    }

    let previous_value = existing_value.take();
    if previous_value == incoming_value {
        *existing_value = previous_value;
        return;
    }

    log::debug!(
        "[anim] duplicate channel '{}' for clip '{}' while merging skeleton outputs; preserving both variants",
        channel_key,
        clip_name
    );
    *existing_value = serde_json::Value::Array(vec![previous_value, incoming_value]);
}

fn hash_vec3(hasher: &mut std::collections::hash_map::DefaultHasher, values: &[f32; 3]) {
    values[0].to_bits().hash(hasher);
    values[1].to_bits().hash(hasher);
    values[2].to_bits().hash(hasher);
}

fn hash_finish_entry(
    hasher: &mut std::collections::hash_map::DefaultHasher,
    entry: &crate::mtl::TintPaletteFinishEntry,
) {
    entry.specular.is_some().hash(hasher);
    if let Some(specular) = entry.specular.as_ref() {
        hash_vec3(hasher, specular);
    }
    entry.glossiness.is_some().hash(hasher);
    if let Some(glossiness) = entry.glossiness {
        glossiness.to_bits().hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtl;

    #[test]
    fn merge_animation_channel_values_promotes_duplicates_to_variant_array() {
        let mut existing = serde_json::json!({"position": [[1.0, 2.0, 3.0]]});
        let incoming = serde_json::json!({"position": [[4.0, 5.0, 6.0]]});

        merge_animation_channel_values(&mut existing, incoming.clone(), "landing_gear_retract", "0x2522C378");

        let arr = existing.as_array().expect("channel entry should become a variant array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], serde_json::json!({"position": [[1.0, 2.0, 3.0]]}));
        assert_eq!(arr[1], incoming);
    }

    #[test]
    fn merge_animation_channel_values_deduplicates_existing_variant_array() {
        let mut existing = serde_json::json!([
            {"position": [[1.0, 2.0, 3.0]]},
            {"position": [[4.0, 5.0, 6.0]]}
        ]);
        let incoming = serde_json::json!({"position": [[4.0, 5.0, 6.0]]});

        merge_animation_channel_values(&mut existing, incoming, "landing_gear_retract", "0x2522C378");

        let arr = existing.as_array().expect("channel entry should remain an array");
        assert_eq!(arr.len(), 2);
    }

    fn sample_submaterial() -> SubMaterial {
        SubMaterial {
            name: "hull_panel".into(),
            shader: "LayerBlend_V2".into(),
            diffuse: [0.7, 0.7, 0.7],
            opacity: 1.0,
            alpha_test: 0.0,
            string_gen_mask: "%STENCIL_MAP%VERTCOLORS".into(),
            is_nodraw: false,
            specular: [0.04, 0.04, 0.04],
            shininess: 128.0,
            emissive: [0.0, 0.0, 0.0],
            glow: 0.0,
            surface_type: String::new(),
            diffuse_tex: Some("Objects/Ships/Test/hull_diff.dds".into()),
            normal_tex: Some("Objects/Ships/Test/hull_ddna.dds".into()),
            layers: vec![mtl::MatLayer {
                name: "Primary".into(),
                path: "libs/materials/metal/test_layer.mtl".into(),
                sub_material: "paint".into(),
                authored_attributes: vec![mtl::AuthoredAttribute {
                    name: "CustomBlendMode".into(),
                    value: "Additive".into(),
                }],
                authored_child_blocks: vec![mtl::AuthoredBlock {
                    tag: "CustomAnimation".into(),
                    attributes: vec![mtl::AuthoredAttribute {
                        name: "Duration".into(),
                        value: "2.0".into(),
                    }],
                    children: Vec::new(),
                }],
                tint_color: [1.0, 0.5, 0.25],
                wear_tint: [0.2, 0.3, 0.4],
                palette_tint: 1,
                gloss_mult: 0.7,
                wear_gloss: 0.8,
                uv_tiling: 2.0,
                height_bias: 0.05,
                height_scale: 1.1,
                snapshot: Some(mtl::MatLayerSnapshot {
                    shader: "Layer".into(),
                    diffuse: [0.6, 0.6, 0.6],
                    specular: [0.1, 0.2, 0.3],
                    shininess: 233.0,
                    wear_specular_color: Some([0.7, 0.7, 0.7]),
                    wear_glossiness: Some(0.91),
                    surface_type: Some("metal_shell".into()),
                    metallic: 0.0,
                }),
                resolved_material: Some(mtl::ResolvedLayerMaterial {
                    name: "paint".into(),
                    shader: "Layer".into(),
                    shader_family: "Layer".into(),
                    authored_attributes: vec![mtl::AuthoredAttribute {
                        name: "MatTemplate".into(),
                        value: "layer_shell".into(),
                    }],
                    public_params: vec![mtl::PublicParam {
                        name: "WearGlossiness".into(),
                        value: "0.91".into(),
                    }],
                    authored_child_blocks: vec![mtl::AuthoredBlock {
                        tag: "VertexDeform".into(),
                        attributes: vec![mtl::AuthoredAttribute {
                            name: "DividerX".into(),
                            value: "0.5".into(),
                        }],
                        children: Vec::new(),
                    }],
                }),
            }],
            palette_tint: 2,
            texture_slots: vec![
                mtl::TextureSlotBinding {
                    slot: "TexSlot1".into(),
                    path: "Objects/Ships/Test/hull_diff.dds".into(),
                    is_virtual: false,
                },
                mtl::TextureSlotBinding {
                    slot: "TexSlot2".into(),
                    path: "Objects/Ships/Test/hull_ddna.dds".into(),
                    is_virtual: false,
                },
                mtl::TextureSlotBinding {
                    slot: "TexSlot7".into(),
                    path: "$TintPaletteDecal".into(),
                    is_virtual: true,
                },
            ],
            public_params: vec![mtl::PublicParam {
                name: "WearBlendBase".into(),
                value: "0.5".into(),
            }],
            authored_attributes: vec![mtl::AuthoredAttribute {
                name: "MtlFlags".into(),
                value: "524544".into(),
            }],
            authored_textures: vec![mtl::AuthoredTexture {
                slot: "TexSlot1".into(),
                path: "Objects/Ships/Test/hull_diff.dds".into(),
                is_virtual: false,
                attributes: vec![
                    mtl::AuthoredAttribute {
                        name: "Map".into(),
                        value: "TexSlot1".into(),
                    },
                    mtl::AuthoredAttribute {
                        name: "Used".into(),
                        value: "1".into(),
                    },
                ],
                child_blocks: vec![mtl::AuthoredBlock {
                    tag: "TexMod".into(),
                    attributes: vec![mtl::AuthoredAttribute {
                        name: "TileU".into(),
                        value: "2".into(),
                    }],
                    children: Vec::new(),
                }],
            }],
            authored_child_blocks: vec![mtl::AuthoredBlock {
                tag: "VertexDeform".into(),
                attributes: vec![mtl::AuthoredAttribute {
                    name: "DividerX".into(),
                    value: "0.5".into(),
                }],
                children: vec![mtl::AuthoredBlock {
                    tag: "WaveX".into(),
                    attributes: vec![mtl::AuthoredAttribute {
                        name: "Amp".into(),
                        value: "0.25".into(),
                    }],
                    children: Vec::new(),
                }],
            }],
        }
    }

    fn sample_mesh(submeshes: Vec<crate::types::SubMesh>) -> Mesh {
        Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes,
            model_min: [0.0, 0.0, 0.0],
            model_max: [0.0, 0.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [0.0, 0.0, 0.0],
        }
    }

    fn sample_nmc(node_names: &[&str]) -> NodeMeshCombo {
        NodeMeshCombo {
            nodes: node_names
                .iter()
                .map(|name| crate::nmc::NmcNode {
                    name: (*name).to_string(),
                    parent_index: None,
                    world_to_bone: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: Default::default(),
                })
                .collect(),
            material_indices: vec![0; node_names.len()],
        }
    }

    #[test]
    fn normalize_source_paths_keep_data_prefix_and_slashes() {
        assert_eq!(
            normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"),
            "Data/Objects/Ships/Test/hull_diff.dds"
        );
        assert_eq!(
            normalize_requested_source_path("Data\\Objects\\Ships\\Test\\hull_diff.dds"),
            "Data/Objects/Ships/Test/hull_diff.dds"
        );
    }

    #[test]
    fn texture_relative_paths_preserve_source_filenames() {
        assert_eq!(
            replace_extension(&normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"), ".png"),
            "Data/Objects/Ships/Test/hull_diff.png"
        );
        assert_eq!(
            replace_extension(&normalize_requested_source_path("Objects/Ships/Test/hull_ddna.dds"), ".png"),
            "Data/Objects/Ships/Test/hull_ddna.png"
        );
    }

    #[test]
    fn insert_stem_suffix_handles_simple_and_compound_extensions() {
        // Simple extension: suffix lands before the dot.
        assert_eq!(
            insert_stem_suffix("Data/Objects/Test/hull.glb", "_LOD0"),
            "Data/Objects/Test/hull_LOD0.glb"
        );
        assert_eq!(
            insert_stem_suffix("Data/Textures/Test/hull_diff.png", "_TEX2"),
            "Data/Textures/Test/hull_diff_TEX2.png"
        );
        // Compound extension: suffix lands before the FIRST dot so the full
        // .materials.json suffix is preserved.
        assert_eq!(
            insert_stem_suffix("Data/Materials/Test/hull.materials.json", "_TEX1"),
            "Data/Materials/Test/hull_TEX1.materials.json"
        );
        // Directory segments with dots must not be disturbed.
        assert_eq!(
            insert_stem_suffix("Data/foo.bar/hull.glb", "_LOD3"),
            "Data/foo.bar/hull_LOD3.glb"
        );
    }

    #[test]
    fn decomposed_blend_exports_classify_blend_mesh_assets() {
        assert_eq!(
            mesh_asset_extension(ExportFormat::Blend),
            ".blend",
            "native decomposed Blend exports should request .blend mesh asset paths directly",
        );
        assert_eq!(
            classify_exported_file_kind("Data/Objects/Test/hull_LOD0.blend"),
            ExportedFileKind::MeshAsset,
        );
    }

    #[test]
    fn package_directory_name_encodes_lod_and_tex() {
        assert_eq!(
            package_directory_name("EntityClassDefinition.RSI_Aurora_Mk2", 0, 0),
            "RSI Aurora Mk2_LOD0_TEX0"
        );
        assert_eq!(
            package_directory_name("EntityClassDefinition.RSI_Aurora_Mk2", 2, 1),
            "RSI Aurora Mk2_LOD2_TEX1"
        );
    }

    #[test]
    fn normalize_package_subdir_filters_invalid_segments() {
        assert_eq!(normalize_package_subdir("ship"), Some("ship".to_string()));
        assert_eq!(normalize_package_subdir("vehicle/test"), Some("vehicle/test".to_string()));
        assert_eq!(normalize_package_subdir("../ship"), Some("ship".to_string()));
        assert_eq!(normalize_package_subdir(""), None);
    }

    #[test]
    fn material_sidecar_json_preserves_phase_three_semantics() {
        let materials = MtlFile {
            materials: vec![sample_submaterial()],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: Some(crate::mtl::PaintOverrideInfo {
                paint_item_name: "paint_black_gold".into(),
                subgeometry_tag: "BlackGold".into(),
                subgeometry_index: 2,
                material_path: Some("Data/Objects/Ships/Test/hull_variant.mtl".into()),
            }),
            material_set: crate::mtl::MaterialSetAuthoredData {
                attributes: vec![crate::mtl::AuthoredAttribute {
                    name: "DefaultPalette".into(),
                    value: "vehicle_palette_test".into(),
                }],
                public_params: vec![crate::mtl::PublicParam {
                    name: "RootGlowScale".into(),
                    value: "2.0".into(),
                }],
                child_blocks: vec![crate::mtl::AuthoredBlock {
                    tag: "VertexDeform".into(),
                    attributes: vec![crate::mtl::AuthoredAttribute {
                        name: "DividerY".into(),
                        value: "0.25".into(),
                    }],
                    children: Vec::new(),
                }],
            },
        };
        let extracted = vec![ExtractedMaterialEntry {
            slot_exports: vec![serde_json::json!({
                "slot": "TexSlot1",
                "role": "base_color",
                "is_virtual": false,
                "source_path": "Data/Objects/Ships/Test/hull_diff.dds",
                "export_path": "Data/Objects/Ships/Test/hull_diff.png",
                "export_kind": "source",
                "authored_attributes": [
                    {
                        "name": "Map",
                        "value": "TexSlot1",
                    },
                    {
                        "name": "Used",
                        "value": "1",
                    }
                ],
                "authored_child_blocks": [
                    {
                        "tag": "TexMod",
                        "attributes": [
                            {
                                "name": "TileU",
                                "value": "2",
                            }
                        ],
                        "children": [],
                    }
                ],
            })],
            direct_texture_exports: vec![TextureExportRef {
                role: "diffuse".into(),
                source_path: "Data/Objects/Ships/Test/hull_diff.dds".into(),
                export_path: "Data/Objects/Ships/Test/hull_diff.png".into(),
                export_kind: "source".into(),
                texture_identity: None,
                alpha_semantic: None,
                derived_from_texture_identity: None,
                derived_from_semantic: None,
            }],
            layer_exports: vec![LayerTextureExport {
                source_material_path: "Data/libs/materials/metal/test_layer.mtl".into(),
                diffuse_export_path: Some("Data/libs/materials/metal/test_layer.png".into()),
                normal_export_path: Some("Data/libs/materials/metal/test_layer.png".into()),
                roughness_export_path: None,
                slot_exports: vec![serde_json::json!({
                    "slot": "TexSlot3",
                    "role": "normal_gloss",
                    "is_virtual": false,
                    "source_path": "Data/libs/materials/metal/test_layer_ddna.dds",
                    "export_path": "Data/libs/materials/metal/test_layer.png",
                    "export_kind": "source",
                    "texture_identity": "ddna_normal",
                    "alpha_semantic": "smoothness",
                    "texture_transform": {
                        "attributes": {
                            "OffsetU": 0.25,
                            "OffsetV": 0.5,
                            "TileU": 2,
                            "TileV": 3
                        },
                        "offset": [0.25, 0.5],
                        "scale": [2.0, 3.0]
                    },
                })],
            }],
            derived_texture_exports: vec![],
        }];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0],
        );

        assert_eq!(value["source_material_path"], serde_json::json!("Data/Objects/Ships/Test/hull.mtl"));
        assert!(value.get("geometry_path").is_none());
        assert_eq!(value["authored_material_set"]["attributes"][0]["name"], serde_json::json!("DefaultPalette"));
        assert_eq!(value["authored_material_set"]["public_params"][0]["name"], serde_json::json!("RootGlowScale"));
        assert_eq!(value["authored_material_set"]["child_blocks"][0]["tag"], serde_json::json!("VertexDeform"));
        assert_eq!(value["paint_override"]["subgeometry_tag"], serde_json::json!("BlackGold"));
        assert_eq!(
            value["submaterials"][0]["blender_material_name"],
            serde_json::json!("hull:hull_panel")
        );
        assert_eq!(value["submaterials"][0]["shader_family"], serde_json::json!("LayerBlend_V2"));
        assert_eq!(value["submaterials"][0]["authored_attributes"][0]["name"], serde_json::json!("MtlFlags"));
        assert_eq!(value["submaterials"][0]["authored_public_params"][0]["name"], serde_json::json!("WearBlendBase"));
        assert_eq!(value["submaterials"][0]["authored_child_blocks"][0]["tag"], serde_json::json!("VertexDeform"));
        assert_eq!(value["submaterials"][0]["texture_slots"][0]["authored_child_blocks"][0]["tag"], serde_json::json!("TexMod"));
        assert_eq!(value["submaterials"][0]["palette_routing"]["material_channel"]["name"], serde_json::json!("secondary"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["palette_channel"]["name"], serde_json::json!("primary"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["name"], serde_json::json!("Primary"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["submaterial_name"], serde_json::json!("paint"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["shader_family"], serde_json::json!("Layer"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["authored_attributes"][0]["name"], serde_json::json!("MatTemplate"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["authored_public_params"][0]["name"], serde_json::json!("WearGlossiness"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["authored_child_blocks"][0]["tag"], serde_json::json!("VertexDeform"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["authored_attributes"][0]["name"], serde_json::json!("CustomBlendMode"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["authored_child_blocks"][0]["tag"], serde_json::json!("CustomAnimation"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["role"], serde_json::json!("normal_gloss"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["texture_identity"], serde_json::json!("ddna_normal"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["alpha_semantic"], serde_json::json!("smoothness"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["texture_transform"]["scale"], serde_json::json!([2.0, 3.0]));
        let gloss_mult = value["submaterials"][0]["layer_manifest"][0]["gloss_mult"]
            .as_f64()
            .expect("gloss_mult should be numeric");
        assert!((gloss_mult - 0.7).abs() < 1e-6);
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["layer_snapshot"]["shader"], serde_json::json!("Layer"));
        let wear_glossiness = value["submaterials"][0]["layer_manifest"][0]["layer_snapshot"]["wear_glossiness"]
            .as_f64()
            .expect("wear_glossiness should be numeric");
        assert!((wear_glossiness - 0.91).abs() < 1e-6);
        assert_eq!(value["submaterials"][0]["public_params"]["WearBlendBase"], serde_json::json!(0.5));
        assert_eq!(value["submaterials"][0]["derived_textures"], serde_json::json!([]));
        assert_eq!(value["submaterials"][0]["virtual_inputs"][0], serde_json::json!("$TintPaletteDecal"));
    }

    #[test]
    fn material_sidecar_json_preserves_iridescence_support_fields() {
        let mut material = sample_submaterial();
        material.shader = "HardSurface".into();
        material.string_gen_mask = "%IRIDESCENCE".into();
        material.public_params = vec![crate::mtl::PublicParam {
            name: "IridescenceIntensity".into(),
            value: "0.75".into(),
        }];

        let materials = MtlFile {
            materials: vec![material],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let extracted = vec![ExtractedMaterialEntry {
            slot_exports: vec![serde_json::json!({
                "slot": "TexSlot10",
                "role": "iridescence",
                "is_virtual": false,
                "source_path": "Data/Objects/Ships/Test/hull_iridescence.dds",
                "export_path": "Data/Objects/Ships/Test/hull_iridescence.png",
                "export_kind": "source",
                "authored_attributes": [],
                "authored_child_blocks": [],
            })],
            direct_texture_exports: Vec::new(),
            layer_exports: Vec::new(),
            derived_texture_exports: Vec::new(),
        }];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0],
        );

        assert_eq!(value["submaterials"][0]["decoded_feature_flags"]["has_iridescence"], serde_json::json!(true));
        assert_eq!(value["submaterials"][0]["texture_slots"][0]["role"], serde_json::json!("iridescence"));
        assert_eq!(value["submaterials"][0]["public_params"]["IridescenceIntensity"], serde_json::json!(0.75));
        assert_eq!(value["submaterials"][0]["authored_public_params"][0]["name"], serde_json::json!("IridescenceIntensity"));
    }

    #[test]
    fn texture_transform_json_extracts_texmod_scale_and_offset() {
        let value = texture_transform_json(&[crate::mtl::AuthoredBlock {
            tag: "TexMod".into(),
            attributes: vec![
                crate::mtl::AuthoredAttribute {
                    name: "TileU".into(),
                    value: "2".into(),
                },
                crate::mtl::AuthoredAttribute {
                    name: "TileV".into(),
                    value: "3".into(),
                },
                crate::mtl::AuthoredAttribute {
                    name: "OffsetU".into(),
                    value: "0.25".into(),
                },
                crate::mtl::AuthoredAttribute {
                    name: "OffsetV".into(),
                    value: "0.5".into(),
                },
            ],
            children: Vec::new(),
        }])
        .expect("structured texture transform");

        assert_eq!(value["scale"], serde_json::json!([2.0, 3.0]));
        assert_eq!(value["offset"], serde_json::json!([0.25, 0.5]));
        assert_eq!(value["attributes"]["TileU"], serde_json::json!(2));
    }

    #[test]
    fn duplicate_submaterial_names_get_stable_blender_suffixes() {
        let first = sample_submaterial();
        let mut second = sample_submaterial();
        second.shader = "Illum".into();
        second.palette_tint = 0;
        second.layers.clear();

        let materials = MtlFile {
            materials: vec![first, second],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let extracted = vec![ExtractedMaterialEntry::default(), ExtractedMaterialEntry::default()];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0, 1],
        );

        assert_eq!(value["submaterials"][0]["blender_material_name"], serde_json::json!("hull:hull_panel_0"));
        assert_eq!(value["submaterials"][1]["blender_material_name"], serde_json::json!("hull:hull_panel_1"));
    }

    #[test]
    fn virtual_slot_source_paths_preserve_virtual_identifier() {
        let binding = SemanticTextureBinding {
            slot: "TexSlot7".into(),
            role: TextureSemanticRole::TintPaletteDecal,
            path: "$TintPaletteDecal".into(),
            is_virtual: true,
            authored_attributes: Vec::new(),
            authored_child_blocks: Vec::new(),
        };

        assert_eq!(slot_source_path(None, &binding), "$TintPaletteDecal");
    }

    #[test]
    fn livery_manifest_groups_scene_entries_by_shared_palette() {
        let mut records = BTreeMap::new();
        records.insert(
            "palette/test".to_string(),
            LiveryUsage {
                palette_id: "palette/test".to_string(),
                palette_source_name: Some("vehicle.palette.test".to_string()),
                entity_names: ["child_a".to_string(), "child_b".to_string()].into_iter().collect(),
                material_sidecars: [
                    "Data/Objects/A.materials.json".to_string(),
                    "Data/Objects/B.materials.json".to_string(),
                ]
                .into_iter()
                .collect(),
            },
        );

        let value = build_livery_manifest_value(&records);
        assert_eq!(value["liveries"][0]["palette_source_name"], serde_json::json!("vehicle.palette.test"));
        assert_eq!(value["liveries"][0]["entity_names"].as_array().map(|items| items.len()), Some(2));
        assert_eq!(value["liveries"][0]["material_sidecars"].as_array().map(|items| items.len()), Some(2));
    }

    #[test]
    fn palette_manifest_preserves_shared_palette_ids() {
        let mut records = BTreeMap::new();
        let palette = TintPalette {
            source_name: Some("vehicle.palette.test".into()),
            display_name: Some("Vehicle Palette Test".into()),
            primary: [0.1, 0.2, 0.3],
            secondary: [0.3, 0.2, 0.1],
            tertiary: [0.4, 0.5, 0.6],
            glass: [0.6, 0.7, 0.8],
            decal_color_r: Some([0.7, 0.6, 0.5]),
            decal_color_g: Some([0.4, 0.5, 0.6]),
            decal_color_b: Some([0.1, 0.2, 0.3]),
            decal_texture: Some("Data/Textures/branding/test_decal.png".into()),
            finish: crate::mtl::TintPaletteFinish {
                primary: crate::mtl::TintPaletteFinishEntry {
                    specular: Some([0.9, 0.8, 0.7]),
                    glossiness: Some(0.42),
                },
                ..Default::default()
            },
        };
        let palette_id = register_palette(&mut records, &palette);

        let value = build_palette_manifest_value(&records);
        assert_eq!(palette_id, "palette/vehicle_palette_test");
        assert_eq!(value["palettes"][0]["id"], serde_json::json!("palette/vehicle_palette_test"));
        assert_eq!(value["palettes"][0]["source_name"], serde_json::json!("vehicle.palette.test"));
        assert_eq!(value["palettes"][0]["display_name"], serde_json::json!("Vehicle Palette Test"));
        assert_eq!(value["palettes"][0]["glass"].as_array().map(|items| items.len()), Some(3));
        assert_eq!(value["palettes"][0]["decal"]["source_path"], serde_json::json!("Data/Textures/branding/test_decal.png"));
        assert_eq!(value["palettes"][0]["decal"]["red"].as_array().map(|items| items.len()), Some(3));
        let specular = value["palettes"][0]["finish"]["primary"]["specular"]
            .as_array()
            .expect("primary finish specular should be an array");
        assert_eq!(specular.len(), 3);
        assert!((specular[0].as_f64().unwrap() - 0.9).abs() < 1e-6);
        assert!((specular[1].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert!((specular[2].as_f64().unwrap() - 0.7).abs() < 1e-6);
        let glossiness = value["palettes"][0]["finish"]["primary"]["glossiness"]
            .as_f64()
            .expect("primary finish glossiness should be numeric");
        assert!((glossiness - 0.42).abs() < 1e-6);
    }

    #[test]
    fn paint_variant_palette_manifest_uses_variant_palette_id() {
        let mut records = BTreeMap::new();
        let variant = crate::mtl::PaintVariant {
            subgeometry_tag: "Paint_Vulture_coramor_2956_purple_pink_green_iridecence".into(),
            palette_id: Some("palette/vulture_coramor_2956_purple_pink_green_iridecence".into()),
            palette: Some(TintPalette {
                source_name: Some("coramor_2956_purple_pink_green_iridecence".into()),
                display_name: Some("Vulture Heartthrob Livery".into()),
                primary: [0.1, 0.2, 0.3],
                secondary: [0.3, 0.2, 0.1],
                tertiary: [0.4, 0.5, 0.6],
                glass: [0.6, 0.7, 0.8],
                decal_color_r: None,
                decal_color_g: None,
                decal_color_b: None,
                decal_texture: None,
                finish: crate::mtl::TintPaletteFinish::default(),
            }),
            display_name: Some("Vulture Heartthrob Livery".into()),
            material_path: None,
            materials: None,
        };

        let palette_id = register_paint_variant_palette(&mut records, &variant)
            .expect("paint variant palette should register");

        let value = build_palette_manifest_value(&records);
        assert_eq!(palette_id, "palette/vulture_coramor_2956_purple_pink_green_iridecence");
        assert_eq!(value["palettes"][0]["id"], serde_json::json!("palette/vulture_coramor_2956_purple_pink_green_iridecence"));
        assert_eq!(value["palettes"][0]["source_name"], serde_json::json!("coramor_2956_purple_pink_green_iridecence"));
        assert_eq!(value["palettes"][0]["display_name"], serde_json::json!("Vulture Heartthrob Livery"));
    }

    #[test]
    fn material_source_request_prefers_loaded_source_path() {
        let materials = MtlFile {
            materials: Vec::new(),
            source_path: Some("Data\\Objects\\Ships\\Test\\canonical.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };

        let path = material_source_request(&materials, "Data/objects/ships/test/canonical", "Data/Objects/Ships/Test/hull.skin");

        assert_eq!(path, "Data\\Objects\\Ships\\Test\\canonical.mtl");
    }

    #[test]
    fn material_source_request_adds_missing_mtl_extension() {
        let materials = MtlFile {
            materials: Vec::new(),
            source_path: None,
            paint_override: None,
            material_set: Default::default(),
        };

        let path = material_source_request(&materials, "Data/objects/ships/test/canonical", "Data/Objects/Ships/Test/hull.skin");

        assert_eq!(path, "Data/objects/ships/test/canonical.mtl");
    }

    #[test]
    fn decomposed_material_view_excludes_nodraw_and_renumbers_submeshes() {
        let mut nodraw = sample_submaterial();
        nodraw.name = "proxy".into();
        nodraw.shader = "NoDraw".into();
        nodraw.is_nodraw = true;

        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let mut glass = sample_submaterial();
        glass.name = "glass".into();
        glass.shader = "GlassPBR".into();

        let materials = MtlFile {
            materials: vec![nodraw, hull, glass],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("proxy".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("glass".into()),
                material_id: 2,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
                source_material_id: None,
                first_index: 6,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
        ]);

        let view = build_decomposed_material_view(&mesh, Some(&materials), None, false, true);
        let filtered_materials = view.sidecar_materials.expect("filtered sidecar materials");
        let glb_materials = view.glb_materials.expect("filtered glb materials");

        assert_eq!(filtered_materials.materials.len(), 2);
        assert_eq!(
            filtered_materials
                .materials
                .iter()
                .map(|material| material.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hull", "glass"]
        );
        assert_eq!(glb_materials.materials.len(), 2);
        assert_eq!(view.mesh.submeshes.len(), 2);
        assert_eq!(
            view.mesh
                .submeshes
                .iter()
                .map(|submesh| submesh.material_id)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
    }

    #[test]
    fn decomposed_material_view_drops_unused_proxy_after_helper_filter() {
        let mut proxy = sample_submaterial();
        proxy.name = "proxy".into();
        proxy.shader = "Illum".into();
        proxy.is_nodraw = false;

        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let mut decal = sample_submaterial();
        decal.name = "decal".into();

        let materials = MtlFile {
            materials: vec![proxy, hull, decal],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("proxy".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("decal".into()),
                material_id: 2,
                source_material_id: None,
                first_index: 6,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
        ]);
        let nmc = sample_nmc(&["body", "proxy_mount"]);

        let view = build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, true);
        let sidecar = view.sidecar_materials.expect("sidecar materials");
        let glb_materials = view.glb_materials.expect("glb materials (used-only)");

        // Phase 58: sidecar holds the FULL non-hidden set (all 3 — none are hidden/NoDraw).
        assert_eq!(sidecar.materials.len(), 3);
        assert_eq!(
            sidecar
                .materials
                .iter()
                .map(|material| material.name.as_str())
                .collect::<Vec<_>>(),
            vec!["proxy", "hull", "decal"]
        );
        // Original indices are identity (no hidden materials).
        assert_eq!(view.sidecar_original_indices, vec![0u32, 1u32, 2u32]);

        // GLB receives only the used-material compacted set (proxy is excluded by NMC).
        assert_eq!(glb_materials.materials.len(), 2);
        assert_eq!(
            glb_materials
                .materials
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hull", "decal"]
        );
        assert_eq!(view.mesh.submeshes.len(), 2);
        assert_eq!(
            view.mesh
                .submeshes
                .iter()
                .map(|submesh| submesh.material_id)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn decomposed_material_view_preserves_materials_when_nmc_has_no_nodes() {
        let mut hull = sample_submaterial();
        hull.name = "hull".into();
        let mut trim = sample_submaterial();
        trim.name = "trim".into();

        let materials = MtlFile {
            materials: vec![hull, trim],
            source_path: Some("Data/Objects/Ships/Test/interior.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("trim".into()),
                material_id: 1,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
        ]);
        let empty_nmc = NodeMeshCombo {
            nodes: Vec::new(),
            material_indices: Vec::new(),
        };

        let view =
            build_decomposed_material_view(&mesh, Some(&materials), Some(&empty_nmc), false, true);

        assert_eq!(view.mesh.submeshes.len(), 2);
        assert_eq!(
            view.mesh
                .submeshes
                .iter()
                .map(|submesh| submesh.material_id)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(view.glb_nmc.is_none());
    }

    #[test]
    fn decomposed_material_view_drops_out_of_range_submeshes_without_restoring_hidden_materials() {
        let mut nodraw = sample_submaterial();
        nodraw.name = "proxy_shield".into();
        nodraw.shader = "NoDraw".into();
        nodraw.is_nodraw = true;

        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let materials = MtlFile {
            materials: vec![nodraw, hull],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("proxy_shield".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("broken".into()),
                material_id: 9,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
                source_material_id: None,
                first_index: 6,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
        ]);

        let view = build_decomposed_material_view(&mesh, Some(&materials), None, false, true);
        let filtered_materials = view.sidecar_materials.expect("filtered sidecar materials");
        let glb_materials = view.glb_materials.expect("filtered glb materials");

        assert_eq!(filtered_materials.materials.len(), 1);
        assert_eq!(filtered_materials.materials[0].name, "hull");
        assert_eq!(glb_materials.materials.len(), 1);
        assert_eq!(view.mesh.submeshes.len(), 1);
        assert_eq!(view.mesh.submeshes[0].material_id, 0);
        assert_eq!(view.mesh.submeshes[0].material_name.as_deref(), Some("hull"));
    }

    #[test]
    fn decomposed_material_view_preserves_shield_named_submeshes_by_default() {
        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let materials = MtlFile {
            materials: vec![hull],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            },
        ]);
        let nmc = sample_nmc(&["body", "shield_geo"]);

        let filtered = build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, false);
        assert_eq!(filtered.mesh.submeshes.len(), 2);
        assert_eq!(filtered.glb_nmc.as_ref().map(|combo| combo.nodes.len()), Some(2));
    }

    #[test]
    fn decomposed_material_view_preserves_sheild_named_submeshes_by_default() {
        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let materials = MtlFile {
            materials: vec![hull],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            },
        ]);
        let nmc = sample_nmc(&["body", "sheild_arm_a_geo"]);

        let filtered = build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, false);
        assert_eq!(filtered.mesh.submeshes.len(), 2);
        assert_eq!(filtered.glb_nmc.as_ref().map(|combo| combo.nodes.len()), Some(2));
    }

    #[test]
    fn decomposed_material_view_preserves_non_excluded_helper_nodes_without_submeshes() {
        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let materials = MtlFile {
            materials: vec![hull],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![crate::types::SubMesh {
            material_name: Some("hull".into()),
            material_id: 0,
            source_material_id: None,
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 0,
        }]);
        let nmc = sample_nmc(&["body", "hardpoint_weapon_mining"]);

        let filtered = build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, false);

        assert_eq!(filtered.mesh.submeshes.len(), 1);
        assert_eq!(filtered.glb_nmc.as_ref().map(|combo| combo.nodes.len()), Some(2));
        assert_eq!(
            filtered
                .glb_nmc
                .as_ref()
                .and_then(|combo| combo.nodes.get(1))
                .map(|node| node.name.as_str()),
            Some("hardpoint_weapon_mining")
        );
    }

    #[test]
    fn insert_binary_file_reuses_identical_content_and_hashes_collisions() {
        let mut files = BTreeMap::new();
        let first = insert_binary_file(&mut files, "scene.json".to_string(), b"a".to_vec());
        let second = insert_binary_file(&mut files, "scene.json".to_string(), b"a".to_vec());
        let third = insert_binary_file(&mut files, "scene.json".to_string(), b"b".to_vec());

        assert_eq!(first, "scene.json");
        assert_eq!(second, "scene.json");
        assert_ne!(third, "scene.json");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn scene_manifest_uses_relative_asset_paths_for_children_and_interiors() {
        let child = SceneInstanceRecord {
            entity_name: "child_a".into(),
            geometry_path: "Data/Objects/Ships/Test/child.skin".into(),
            material_path: "Data/Objects/Ships/Test/child.mtl".into(),
            mesh_asset: "Data/Objects/Ships/Test/child.glb".into(),
            material_sidecar: Some("Data/Objects/Ships/Test/child.materials.json".into()),
            palette_id: Some("palette/test".into()),
            parent_node_name: Some("hardpoint_weapon_left".into()),
            parent_entity_name: Some("root".into()),
            source_transform_basis: Some("gltf_y_up".into()),
            local_transform_sc: Some(crate::socpak::build_container_transform([1.0, 2.0, 3.0], [0.0, 90.0, 0.0])),
            resolved_no_rotation: false,
            no_rotation: false,
            offset_position: [1.0, 2.0, 3.0],
            offset_rotation: [0.0, 90.0, 0.0],
            detach_direction: [0.0, 0.0, -1.0],
            port_flags: "invisible uneditable".into(),
        };
        let interior = InteriorContainerRecord {
            name: "interior_main".into(),
            parent_entity_name: Some("child_entity".into()),
            parent_node_name: Some("child_root".into()),
            palette_id: Some("palette/interior".into()),
            container_transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            placements: vec![InteriorPlacementRecord {
                cgf_path: "Data/Objects/Ships/Test/interior_panel.cgf".into(),
                material_path: Some("Data/Objects/Ships/Test/interior_panel.mtl".into()),
                mesh_asset: "Data/Objects/Ships/Test/interior_panel.glb".into(),
                material_sidecar: Some("Data/Objects/Ships/Test/interior_panel.materials.json".into()),
                entity_class_guid: Some("1234".into()),
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [4.0, 5.0, 6.0, 1.0],
                ],
                palette_id: None,
            }],
            lights: vec![serde_json::json!({ "name": "light_a" })],
        };

        let value = build_scene_manifest_value(
            "root",
            "ARGO MOLE",
            "Data/Objects/Ships/Test/root.skin",
            "Data/Objects/Ships/Test/root.mtl",
            "Data/Objects/Ships/Test/root.glb",
            Some("Data/Objects/Ships/Test/root.materials.json"),
            Some("palette/root"),
            None,
            &[child],
            &[interior],
            &[],
            &ExportOptions::default(),
        );

        assert_eq!(value["root_entity"]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/root.glb"));
        assert_eq!(value["children"][0]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/child.glb"));
        assert_eq!(value["children"][0]["parent_node_name"], serde_json::json!("hardpoint_weapon_left"));
        assert_eq!(value["children"][0]["source_transform_basis"], serde_json::json!("gltf_y_up"));
        assert!(value["children"][0]["local_transform_sc"].is_array());
        assert_eq!(value["children"][0]["resolved_no_rotation"], serde_json::json!(false));
        assert_eq!(value["interiors"][0]["parent_entity_name"], serde_json::json!("child_entity"));
        assert_eq!(value["interiors"][0]["parent_node_name"], serde_json::json!("child_root"));
        assert_eq!(value["interiors"][0]["placements"][0]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/interior_panel.glb"));
        assert_eq!(value["package_rule"]["package_dir"], serde_json::json!("Packages/ARGO MOLE"));
        assert_eq!(value["package_rule"]["normalized_p4k_relative_paths"], serde_json::json!(true));
    }

    #[test]
    fn scene_manifest_includes_engine_glow_controls() {
        let value = build_scene_manifest_value(
            "root",
            "DRAK Clipper",
            "Data/Objects/Ships/Test/root.skin",
            "Data/Objects/Ships/Test/root.mtl",
            "Data/Objects/Ships/Test/root.glb",
            Some("Data/Objects/Ships/Test/root.materials.json"),
            Some("palette/root"),
            None,
            &[],
            &[],
            &[EngineGlowTargetRecord {
                material_sidecar: "Data/Objects/Ships/Test/root.materials.json".into(),
                entity_name: "DRAK_Clipper_Thruster_Main".into(),
                geometry_path: "Data/Objects/Spaceships/Thrusters/DRAK/test_thruster.cga".into(),
                mesh_asset: "Data/Objects/Spaceships/Thrusters/DRAK/test_thruster_LOD0.blend".into(),
                source_material_index: 7,
                submaterial_name: "Glow_Thrusters".into(),
                blender_material_name: "root:Glow_Thrusters".into(),
            }],
            &ExportOptions::default(),
        );

        assert_eq!(
            value["controls"]["engine_glow"]["default_strength"],
            serde_json::json!(3.0)
        );
        assert_eq!(
            value["controls"]["engine_glow"]["targets"][0]["geometry_path"],
            serde_json::json!("Data/Objects/Spaceships/Thrusters/DRAK/test_thruster.cga")
        );
        assert_eq!(
            value["controls"]["engine_glow"]["targets"][0]["source_material_index"],
            serde_json::json!(7)
        );
    }

    #[test]
    fn engine_glow_targets_follow_datacore_thruster_entity_material_bindings() {
        let mut glow_material = sample_submaterial();
        glow_material.name = "Glow_Thrusters".into();
        glow_material.shader = "Illum".into();
        glow_material.glow = 1.0;

        let materials = MtlFile {
            materials: vec![glow_material],
            source_path: Some("Data/Objects/Ships/Test/root.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let mesh = sample_mesh(vec![crate::types::SubMesh {
            material_name: Some("Glow_Thrusters".into()),
            material_id: 0,
            source_material_id: Some(5),
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 1,
        }]);
        let targets = build_thruster_engine_glow_targets(
            &mesh,
            Some(&materials),
            Some("Data/Objects/Ships/Test/root.materials.json"),
            &[5],
            "DRAK_Test_Thruster_Main",
            "Objects/Spaceships/Thrusters/DRAK/test_thruster.cga",
            "Data/Objects/Spaceships/Thrusters/DRAK/test_thruster_LOD0.blend",
        );

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].entity_name, "DRAK_Test_Thruster_Main");
        assert_eq!(
            targets[0].geometry_path,
            "Data/Objects/Spaceships/Thrusters/DRAK/test_thruster.cga"
        );
        assert_eq!(
            targets[0].mesh_asset,
            "Data/Objects/Spaceships/Thrusters/DRAK/test_thruster_LOD0.blend"
        );
        assert_eq!(targets[0].source_material_index, 5);
        assert_eq!(targets[0].submaterial_name, "Glow_Thrusters");
    }

    #[test]
    fn resolve_no_rotation_local_matrix_suppresses_duplicate_zero_rotation_offset() {
        let parent_world = glam::Mat4::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)).to_cols_array();
        let resolved = resolve_no_rotation_local_matrix(parent_world, [3.0, 0.0, 0.0], [0.0, 0.0, 0.0]);

        assert_eq!(resolved[12], 0.0);
        assert_eq!(resolved[13], 0.0);
        assert_eq!(resolved[14], 0.0);
    }

    #[test]
    fn resolve_no_rotation_local_matrix_treats_tiny_rotation_as_zero() {
        let parent_world = glam::Mat4::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)).to_cols_array();
        let resolved = resolve_no_rotation_local_matrix(parent_world, [3.0, 0.0, 0.0], [1e-7, 0.0, 0.0]);

        assert_eq!(resolved[12], 0.0);
        assert_eq!(resolved[13], 0.0);
        assert_eq!(resolved[14], 0.0);
    }

    fn test_nmc_node(
        name: &str,
        parent_index: Option<u16>,
        bone_to_world: [[f32; 4]; 3],
    ) -> crate::nmc::NmcNode {
        crate::nmc::NmcNode {
            name: name.to_string(),
            parent_index,
            world_to_bone: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            bone_to_world,
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            properties: HashMap::new(),
        }
    }

    fn empty_test_mesh() -> Mesh {
        Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: Vec::new(),
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
        }
    }

    #[test]
    fn docking_entity_attachment_uses_parent_host_and_child_vehicle_attach_point() {
        let root_nmc = NodeMeshCombo {
            nodes: vec![
                test_nmc_node(
                    "root",
                    None,
                    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
                ),
                test_nmc_node(
                    "hardpoint_docking_module",
                    Some(0),
                    [[0.0, -1.0, 0.0, -18.9], [1.0, 0.0, 0.0, -18.38946], [0.0, 0.0, 1.0, 5.51615]],
                ),
                test_nmc_node(
                    "hardpoint_docking_host",
                    Some(0),
                    [[1.0, 0.0, 0.0, -19.15725], [0.0, 1.0, 0.0, -18.37498], [0.0, 0.0, 1.0, 7.30487]],
                ),
            ],
            material_indices: Vec::new(),
        };
        let child_nmc = NodeMeshCombo {
            nodes: vec![test_nmc_node(
                "hardpoint_docking_vehicle",
                None,
                [[1.0, 0.0, 0.0, 2.98941], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0474]],
            )],
            material_indices: Vec::new(),
        };
        let mut builder = GlbBuilder::new();
        let dummy_packed = PackedMeshInfo {
            mesh_idx: 0,
            pos_accessor_idx: 0,
            uv_accessor_idx: None,
            secondary_uv_accessor_idx: None,
            normal_accessor_idx: None,
            color_accessor_idx: None,
            tangent_accessor_idx: None,
            submesh_mat_indices: Vec::new(),
            submesh_idx_accessors: Vec::new(),
        };
        builder.build_nmc_hierarchy(&dummy_packed, &root_nmc, &[], false);
        let target_idx = *builder
            .node_name_to_idx
            .get("hardpoint_docking_module")
            .expect("target node should exist") as usize;
        let child = EntityPayload {
            mesh: empty_test_mesh(),
            materials: None,
            textures: None,
            nmc: Some(child_nmc),
            palette: None,
            geometry_path: "Objects/Spaceships/Ships/DRAK/command_module/exterior/test.cga".to_string(),
            material_path: String::new(),
            bones: Vec::new(),
            skeleton_source_path: None,
            entity_name: "DRAK_Test_Command_Module".to_string(),
            entity_category: None,
            parent_node_name: "hardpoint_docking_module".to_string(),
            parent_entity_name: "root".to_string(),
            no_rotation: true,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            detach_direction: [0.0; 3],
            port_flags: "Docking_Request_Accepting".to_string(),
        };

        let offset = docking_entity_attachment_offset(&builder, target_idx, &child)
            .expect("docking offset should be derived");

        assert!((offset.x - 0.01448).abs() < 0.0001);
        assert!((offset.y - 3.24666).abs() < 0.0001);
        assert!((offset.z - 0.74132).abs() < 0.0001);
    }

    #[test]
    fn normalized_relative_paths_join_beneath_selected_base_directory() {
        let base_dir = std::path::PathBuf::from("/tmp/export-root");
        let texture_path = replace_extension(
            &normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"),
            ".png",
        );
        let full_path = base_dir.join(texture_path);

        // Normalize separators: Path::join uses '\\' on Windows.
        assert_eq!(
            full_path.to_string_lossy().replace('\\', "/"),
            "/tmp/export-root/Data/Objects/Ships/Test/hull_diff.png"
        );
    }

    // --- Phase 58 tests -------------------------------------------------------

    #[test]
    fn source_material_id_is_set_to_original_index_after_hide_filter() {
        // Material 0 is hidden (NoDraw); material 1 and 2 are visible.
        // After filtering, the submesh that referenced material 2 should have:
        //   material_id        = 1  (compacted post-hide index)
        //   source_material_id = Some(2)  (original source index)
        let mut hidden = sample_submaterial();
        hidden.name = "proxy".into();
        hidden.shader = "NoDraw".into();
        hidden.is_nodraw = true;

        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let mut decal = sample_submaterial();
        decal.name = "decal".into();

        let materials = MtlFile {
            materials: vec![hidden.clone(), hull.clone(), decal.clone()],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };

        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("decal".into()),
                material_id: 2,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
        ]);

        let view = build_decomposed_material_view(&mesh, Some(&materials), None, false, false);

        // GLB submesh 0 → hull (original index 1)
        assert_eq!(view.mesh.submeshes[0].material_id, 0, "compacted material_id for hull");
        assert_eq!(view.mesh.submeshes[0].source_material_id, Some(1), "source index for hull");

        // GLB submesh 1 → decal (original index 2)
        assert_eq!(view.mesh.submeshes[1].material_id, 1, "compacted material_id for decal");
        assert_eq!(view.mesh.submeshes[1].source_material_id, Some(2), "source index for decal");
    }

    #[test]
    fn sidecar_original_indices_reflect_source_mtl_positions() {
        // hidden material at index 1; visible at 0 and 2.
        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let mut hidden = sample_submaterial();
        hidden.name = "proxy".into();
        hidden.shader = "NoDraw".into();
        hidden.is_nodraw = true;

        let mut decal = sample_submaterial();
        decal.name = "decal".into();

        let materials = MtlFile {
            materials: vec![hull.clone(), hidden.clone(), decal.clone()],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };

        // Only hull (0) is referenced by the mesh.
        let mesh = sample_mesh(vec![crate::types::SubMesh {
            material_name: Some("hull".into()),
            material_id: 0,
            source_material_id: None,
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 0,
        }]);

        let view = build_decomposed_material_view(&mesh, Some(&materials), None, false, false);

        // sidecar should contain the non-hidden set: hull (orig 0) + decal (orig 2)
        let sidecar = view.sidecar_materials.as_ref().expect("sidecar materials");
        assert_eq!(sidecar.materials.len(), 2, "non-hidden material count");
        assert_eq!(sidecar.materials[0].name, "hull");
        assert_eq!(sidecar.materials[1].name, "decal");

        // original indices: hull→0, decal→2
        assert_eq!(view.sidecar_original_indices, vec![0u32, 2u32]);
    }

    #[test]
    fn two_meshes_sharing_same_mtl_produce_identical_sidecar_content() {
        // Mesh A uses only material 0; Mesh B uses only material 1.
        // Both share the same MtlFile.  After Phase 58 the sidecar for both
        // must be identical (the full non-hidden set with stable original indices).
        let mut mat0 = sample_submaterial();
        mat0.name = "exterior".into();

        let mut mat1 = sample_submaterial();
        mat1.name = "interior".into();

        let materials = MtlFile {
            materials: vec![mat0.clone(), mat1.clone()],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };

        let mesh_a = sample_mesh(vec![crate::types::SubMesh {
            material_name: Some("exterior".into()),
            material_id: 0,
            source_material_id: None,
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 0,
        }]);
        let mesh_b = sample_mesh(vec![crate::types::SubMesh {
            material_name: Some("interior".into()),
            material_id: 1,
            source_material_id: None,
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 0,
        }]);

        let view_a = build_decomposed_material_view(&mesh_a, Some(&materials), None, false, false);
        let view_b = build_decomposed_material_view(&mesh_b, Some(&materials), None, false, false);

        // Both sidecars must contain the same non-hidden set and same original indices.
        let sidecar_a = view_a.sidecar_materials.as_ref().expect("sidecar A");
        let sidecar_b = view_b.sidecar_materials.as_ref().expect("sidecar B");
        assert_eq!(
            sidecar_a.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            sidecar_b.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            "sidecar material lists should be identical"
        );
        assert_eq!(
            view_a.sidecar_original_indices,
            view_b.sidecar_original_indices,
            "sidecar original indices should be identical"
        );
        assert_eq!(view_a.sidecar_original_indices, vec![0u32, 1u32]);
    }

    /// Phase 58 invariant: the sidecar path is derived from the *source .mtl*
    /// file, not from the per-CGF geometry path.  This ensures that two
    /// different CGF meshes sharing the same `.mtl` file always produce the
    /// same sidecar path (before dedup via `insert_json_file`), so identical
    /// content can never accumulate hash-variant files.
    #[test]
    fn material_sidecar_relative_path_uses_source_mtl_not_geometry_path() {
        // Both paths reference the same logical .mtl; the sidecar must resolve
        // to the same output path regardless of the geometry file that triggers it.
        let path_a = material_sidecar_relative_path("Data/Objects/Ships/Drak/Clipper/hull.mtl", "fallback", 0);
        let path_b = material_sidecar_relative_path("Data/Objects/Ships/Drak/Clipper/hull.mtl", "other_fallback", 0);

        assert_eq!(path_a, path_b, "same source .mtl must produce the same sidecar path");
        assert_eq!(path_a, "Data/Objects/Ships/Drak/Clipper/hull_TEX0.materials.json");
    }

    #[test]
    fn material_sidecar_relative_path_encodes_mip_level() {
        let path0 = material_sidecar_relative_path("Data/Objects/Ships/Test/hull.mtl", "f", 0);
        let path2 = material_sidecar_relative_path("Data/Objects/Ships/Test/hull.mtl", "f", 2);

        assert_eq!(path0, "Data/Objects/Ships/Test/hull_TEX0.materials.json");
        assert_eq!(path2, "Data/Objects/Ships/Test/hull_TEX2.materials.json");
        assert_ne!(path0, path2);
    }
}
