use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::Instant;

use gltf_json as json;
use rayon::prelude::*;
use starbreaker_common::progress::{Progress, report as report_progress};
use starbreaker_dds;
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::gltf::{GlbBuilder, PackedMeshInfo, offset_to_gltf_matrix};
use crate::mtl::{
    MtlFile, SemanticTextureBinding, ShaderFamily, SubMaterial, TextureSemanticRole, TintPalette,
};
use crate::nmc::NodeMeshCombo;
use crate::pipeline::{
    DecomposedExport, ExportFormat, ExportOptions, ExportedFile, ExportedFileKind,
    InteriorCgfEntry, LoadedInteriors, MaterialMode, PngCache, RoughnessCache,
};
use crate::skeleton::Bone;
use crate::types::{EntityPayload, Mesh, UiBinding};

pub(crate) struct DecomposedInput {
    pub entity_name: String,
    pub geometry_path: String,
    pub material_path: String,
    pub assembly_kind: Option<String>,
    pub weapon_assembly: Option<serde_json::Value>,
    pub weapon_assembly_diagnostics: Option<serde_json::Value>,
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
    Roughness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextureExportRef {
    role: String,
    source_path: String,
    export_path: String,
    export_kind: String,
    texture_identity: Option<String>,
    alpha_semantic: Option<String>,
    alpha_channel: Option<String>,
    derived_from_texture_identity: Option<String>,
    derived_from_semantic: Option<String>,
    derived_from_channel: Option<String>,
    value_channel: Option<String>,
    value_transform: Option<String>,
    packed_texture_format: Option<String>,
    packed_channel_semantics: Option<BTreeMap<String, String>>,
    constant_channel_values: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextureDerivationStatus {
    role: String,
    source_path: String,
    export_kind: String,
    texture_identity: Option<String>,
    derived_from_texture_identity: Option<String>,
    derived_from_semantic: Option<String>,
    derived_from_channel: Option<String>,
    value_channel: Option<String>,
    value_transform: Option<String>,
    packed_texture_format: Option<String>,
    packed_channel_semantics: Option<BTreeMap<String, String>>,
    constant_channel_values: Option<BTreeMap<String, String>>,
    status: String,
    reason: Option<String>,
    export_path: Option<String>,
    requested_mip: Option<u32>,
    selected_mip: Option<u32>,
    mip_selection: Option<String>,
    alpha_mip_format: Option<String>,
    alpha_mip_layout: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    alpha_mip_count: Option<u32>,
    smoothness_min: Option<u8>,
    smoothness_max: Option<u8>,
    smoothness_mean: Option<u8>,
    roughness_min: Option<u8>,
    roughness_max: Option<u8>,
    roughness_mean: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerTextureExport {
    source_material_path: String,
    diffuse_export_path: Option<String>,
    normal_export_path: Option<String>,
    roughness_export_path: Option<String>,
    roughness_texture: Option<TextureExportRef>,
    ddna_derivations: Vec<TextureDerivationStatus>,
    slot_exports: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExtractedMaterialEntry {
    slot_exports: Vec<serde_json::Value>,
    direct_texture_exports: Vec<TextureExportRef>,
    layer_exports: Vec<LayerTextureExport>,
    derived_texture_exports: Vec<TextureExportRef>,
    ddna_derivations: Vec<TextureDerivationStatus>,
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
    ui_bindings: Vec<UiBinding>,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedChildTransform {
    local_transform_sc: [[f32; 4]; 4],
    resolved_no_rotation: bool,
}

fn identity_flat_4x4() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
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
    glam::Mat4::from_rotation_translation(
        parent_rotation.inverse() * desired_rotation,
        local_translation,
    )
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

fn docking_host_world_matrix(builder: &GlbBuilder, target_idx: usize) -> Option<[f32; 16]> {
    let parent_of = node_parent_indices(&builder.nodes_json);
    let parent_idx = parent_of.get(target_idx).and_then(|idx| *idx)?;
    let siblings = builder.nodes_json.get(parent_idx)?.children.as_ref()?;
    siblings
        .iter()
        .filter_map(|sibling| {
            let index = sibling.value();
            let name = builder
                .nodes_json
                .get(index)?
                .name
                .as_deref()?
                .to_ascii_lowercase();
            Some((index, name))
        })
        .find(|(_, name)| name.contains("docking_host"))
        .or_else(|| {
            siblings
                .iter()
                .filter_map(|sibling| {
                    let index = sibling.value();
                    let name = builder
                        .nodes_json
                        .get(index)?
                        .name
                        .as_deref()?
                        .to_ascii_lowercase();
                    Some((index, name))
                })
                .find(|(_, name)| name.contains("docking_door"))
        })
        .map(|(index, _)| builder.compute_node_world_matrix(index))
}

fn child_docking_vehicle_translation(nmc: Option<&NodeMeshCombo>) -> Option<glam::Vec3> {
    nmc?.nodes.iter().find_map(|node| {
        node.name
            .to_ascii_lowercase()
            .contains("docking_vehicle")
            .then(|| {
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

    let scene_nodes =
        if let Some(root_nmc) = input.root_nmc.as_ref().filter(|nmc| !nmc.nodes.is_empty()) {
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

    let mut load_textures =
        |_materials: Option<&crate::mtl::MtlFile>, _palette: Option<&crate::mtl::TintPalette>| None;
    let mut resolved = Vec::with_capacity(input.children.len());

    for child in &input.children {
        let resolved_local_matrix = if child.no_rotation {
            let target_idx = builder
                .node_name_to_idx
                .get(&child.parent_node_name.to_lowercase())
                .copied()
                .or_else(|| {
                    builder
                        .node_name_to_idx
                        .get(&child.parent_entity_name.to_lowercase())
                        .copied()
                })
                .or_else(|| scene_nodes.first().map(|node| node.value() as u32))
                .unwrap_or(0);
            let mut offset_position = child.offset_position;
            if let Some(docking_offset) =
                docking_entity_attachment_offset(&builder, target_idx as usize, child)
            {
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
                attach_def_type: child.attach_def_type.clone(),
                parent_node_name: child.parent_node_name.clone(),
                parent_entity_name: child.parent_entity_name.clone(),
                no_rotation: child.no_rotation,
                offset_position: child.offset_position,
                offset_rotation: child.offset_rotation,
                detach_direction: child.detach_direction,
                port_flags: child.port_flags.clone(),
                ui_bindings: child.ui_bindings.clone(),
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
    ui_bindings: Vec<UiBinding>,
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
    let blender_material_names =
        preferred_blender_material_names(&materials.materials, source_stem);
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
    if !matches!(
        material.shader_family(),
        ShaderFamily::Illum | ShaderFamily::HardSurface
    ) {
        return false;
    }
    let emissive_factor = material.emissive_factor();
    let has_emissive_energy =
        emissive_factor.iter().any(|component| *component > 0.0) || material.glow > 0.0;
    let has_emissive_texture = material.texture_slots.iter().any(|slot| {
        let lowered = slot.path.to_ascii_lowercase();
        lowered.contains("glow") || lowered.contains("emissive")
    });
    has_emissive_energy || has_emissive_texture
}

fn should_export_engine_glow_targets(child: &crate::types::EntityPayload) -> bool {
    child
        .entity_category
        .as_deref()
        .is_some_and(|category| category.eq_ignore_ascii_case("Thruster"))
        && child
            .attach_def_type
            .as_deref()
            .is_some_and(|attach_def_type| attach_def_type.eq_ignore_ascii_case("MainThruster"))
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
        let (filtered_mesh, filtered_nmc) =
            filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
        return DecomposedMaterialView {
            mesh: filtered_mesh,
            sidecar_materials: None,
            sidecar_original_indices: Vec::new(),
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    };

    if include_nodraw {
        let filtered_mesh =
            filter_mesh_geometry(mesh, Some(materials), nmc, include_nodraw, include_shields);
        let (filtered_mesh, filtered_nmc) =
            filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
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
                if material.is_nodraw {
                    "NoDraw"
                } else {
                    "opacity"
                }
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
        .filter_map(|(orig_idx, mapped)| {
            if mapped.is_some() {
                Some(orig_idx as u32)
            } else {
                None
            }
        })
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
        let (filtered_mesh, filtered_nmc) =
            filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
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

    let (filtered_mesh, filtered_nmc) =
        filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);

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
            .is_some_and(|node| {
                helper_name_is_excluded(&node.name, include_nodraw, include_shields)
            })
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
    db: &Database<'_>,
    p4k: &MappedP4k,
    input: DecomposedInput,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
    existing_interior_assets: Option<&ExistingInteriorAssetMap>,
    png_cache: PngCache,
    roughness_cache: RoughnessCache,
    mtl_cache: HashMap<String, Option<MtlFile>>,
    load_interior_mesh: &mut dyn FnMut(
        &InteriorCgfEntry,
    )
        -> Option<(Mesh, Option<MtlFile>, Option<NodeMeshCombo>)>,
) -> Result<DecomposedExport, Error> {
    const ROOT_ASSETS_START: f32 = 0.01;
    const CHILD_ASSETS_START: f32 = 0.16;
    const CHILD_ASSETS_END: f32 = 0.38;
    const INTERIOR_ASSETS_END: f32 = 0.99;

    let mut files = OutputFiles::new();
    let root_manufacturer_id: Option<String> =
        derive_manufacturer_id(export_entity_basename(&input.entity_name));
    // Per-ship UI values derived once from the root vehicle's DataCore
    // records (power pools → pip stacks, temps); every binding render below
    // receives them so screens show this ship's data, not static defaults.
    let ui_ship_data = crate::ui_pipeline::UiShipData::derive(db, &input.entity_name);
    // Localization (`global.ini`) is multi-MB and expensive to parse.  Load it
    // exactly once per export and share a reference with every UI binding render
    // — this is the documented contract of `UiLocData::load`.  Previously it was
    // reloaded inside every child / interior-placement binding group, which on a
    // capital ship (hundreds of UI-bearing entities) dominated export time.
    // Skip the load entirely when no binding needs it, preserving the fast path
    // for UI-free exports.
    let any_ui_bindings = input
        .children
        .iter()
        .any(|child| !child.ui_bindings.is_empty())
        || input.interiors.containers.iter().any(|container| {
            container
                .placements
                .iter()
                .any(|placement| !placement.ui_bindings.is_empty())
        });
    let ui_loc_data = any_ui_bindings.then(|| crate::ui_pipeline::UiLocData::load(p4k));
    // The default-value registry feeding every UI binding render is a pure
    // function of the localization map and the ship's derived values, both
    // constant across the export — build it once and share it by reference,
    // instead of rebuilding (clone + re-merge the localization map) per render.
    let ui_defaults_registry = ui_loc_data
        .as_ref()
        .map(|loc_data| crate::ui_pipeline::build_default_registry(loc_data, &ui_ship_data));
    // Pre-render each distinct UI screen once (hundreds of placements/children
    // re-use a small set of screens). Built before the child/interior loops so
    // both consume the same cache; the loops then look up renders instead of
    // rendering. See `prerender_ui_bindings`.
    let ui_render_cache: HashMap<UiRenderKey, Result<Vec<u8>, String>> =
        match (ui_loc_data.as_ref(), ui_defaults_registry.as_ref()) {
            (Some(loc_data), Some(defaults_registry)) => {
                let render_start = Instant::now();
                let mut all_bindings: Vec<&UiBinding> = Vec::new();
                for child in &input.children {
                    all_bindings.extend(child.ui_bindings.iter());
                }
                for container in &input.interiors.containers {
                    for placement in &container.placements {
                        all_bindings.extend(placement.ui_bindings.iter());
                    }
                }
                let cache = prerender_ui_bindings(
                    &all_bindings,
                    db,
                    p4k,
                    opts.texture_mip,
                    &input.entity_name,
                    root_manufacturer_id.as_deref(),
                    loc_data,
                    defaults_registry,
                    &ui_ship_data,
                );
                log::info!(
                    "[timing][decomposed] prerender_ui: {:.2}s ({} bindings, {} unique)",
                    render_start.elapsed().as_secs_f32(),
                    all_bindings.len(),
                    cache.len(),
                );
                cache
            }
            _ => HashMap::new(),
        };
    let mut texture_cache: HashMap<(String, TextureFlavor), String> = HashMap::new();
    // Memo for the DDNA→roughness derivation (exported ref + status metadata);
    // shared across root/paint/child/interior sidecars so each DDNA source is
    // decoded and statistics-scanned exactly once per export.
    let mut ddna_status_cache: HashMap<String, (Option<TextureExportRef>, TextureDerivationStatus)> =
        HashMap::new();
    // Seeded from the prewarm's source-material resolution (see
    // `prewarm_decomposed_textures`); the single blend caller passes the
    // prewarmed memo, a second caller passes `HashMap::new()`.
    let mut mtl_cache = mtl_cache;
    // Caller may pre-fill this with parallel-decoded textures (see
    // `prewarm_decomposed_textures`); otherwise it starts empty.
    let mut png_cache = png_cache;
    // Prewarmed, consult-only DDNA→roughness decode cache (see
    // `prewarm_decomposed_roughness`); passed by `&` downstream.
    let roughness_cache = roughness_cache;
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
    let root_palette_id = input
        .root_palette
        .as_ref()
        .map(|palette| register_palette(&mut palette_records, palette));
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
        if opts.ui_only_files {
            projected_material_sidecar_path(
                p4k,
                materials,
                &input.material_path,
                &input.geometry_path,
                &input.entity_name,
                opts.texture_mip,
                &mut mtl_cache,
            )
        } else {
            write_material_sidecar(
                &mut files,
                p4k,
                &mut png_cache,
                &mut texture_cache,
                &mut ddna_status_cache,
                &roughness_cache,
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
        }
    }).map(|path| normalize_material_source_for_manifest(&path));
    let mut engine_glow_targets = Vec::new();
    register_livery_usage(
        &mut livery_usage,
        root_palette_id.as_deref(),
        input.root_palette.as_ref(),
        &input.entity_name,
        root_material_sidecar.as_deref(),
    );
    log::info!(
        "[timing][decomposed] root_assets: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );
    phase_start = Instant::now();

    // Export material sidecars for each paint variant and build the paints.json manifest.
    let mut paint_variant_json: Vec<serde_json::Value> = Vec::new();
    for variant in &input.paint_variants {
        register_paint_variant_palette(&mut palette_records, variant);
        let Some(palette_id) = variant.palette_id.as_ref() else {
            continue;
        };
        let sidecar_path = variant.materials.as_ref().map(|materials| {
            let variant_material_path = variant
                .material_path
                .as_deref()
                .unwrap_or(&input.material_path);
            if opts.ui_only_files {
                projected_material_sidecar_path(
                    p4k,
                    materials,
                    variant_material_path,
                    &input.geometry_path,
                    &input.entity_name,
                    opts.texture_mip,
                    &mut mtl_cache,
                )
            } else {
                let identity: Vec<u32> = (0..materials.materials.len() as u32).collect();
                write_material_sidecar(
                    &mut files,
                    p4k,
                    &mut png_cache,
                    &mut texture_cache,
                    &mut ddna_status_cache,
                    &roughness_cache,
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
            }
        }).map(|path| normalize_material_source_for_manifest(&path));
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
    log::info!(
        "[timing][decomposed] paint_variants: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );
    phase_start = Instant::now();

    report_progress(progress, CHILD_ASSETS_START, "Writing child assets");

    let resolved_child_transforms = resolve_child_instance_transforms(&input);
    let mut child_instances = Vec::with_capacity(input.children.len());
    let child_count = input.children.len();
    // Render every child's UI bindings up front in one parallel pass.  The bulk
    // of child-export time on a UI-heavy capital ship is MFD/screen rendering,
    // and each render is single-threaded, so the unit of parallel work is one
    // *binding*, not one child.  A handful of children (cockpit, consoles) own
    // most bindings while the rest own none, so parallelising per child leaves
    // most cores idle.  Flatten all bindings into one work list and render them
    // in a single balanced `par_iter`, then regroup the results per child in
    // their original order — the serial loop below consumes them at the same
    // point and in the same order as before, so the merged `files` map stays
    // byte-identical to serial production.
    let ui_render_start = Instant::now();
    let child_ui_jobs: Vec<(usize, &UiBinding)> = input
        .children
        .iter()
        .enumerate()
        .flat_map(|(child_index, child)| {
            child
                .ui_bindings
                .iter()
                .map(move |binding| (child_index, binding))
        })
        .collect();
    let mut precomputed_child_ui: Vec<(Vec<UiBinding>, Vec<(String, Vec<u8>)>)> =
        vec![(Vec::new(), Vec::new()); input.children.len()];
    if !child_ui_jobs.is_empty() {
        // Localization is shared by reference; the caller loaded it because at
        // least one binding (these) exists, so it is always `Some` here.
        let loc_data = ui_loc_data
            .as_ref()
            .expect("UiLocData must be loaded when UI bindings are present");
        let defaults_registry = ui_defaults_registry
            .as_ref()
            .expect("default registry must be built when UI bindings are present");
        let rendered: Vec<(usize, UiBinding, Option<(String, Vec<u8>)>)> = child_ui_jobs
            .par_iter()
            .map(|(child_index, binding)| {
                let (binding, file_record) = generated_ui_binding_record(
                    binding,
                    db,
                    p4k,
                    opts.texture_mip,
                    &input.entity_name,
                    &input.geometry_path,
                    &scene_manifest_path,
                    root_manufacturer_id.as_deref(),
                    loc_data,
                    defaults_registry,
                    &ui_ship_data,
                    &ui_render_cache,
                );
                (*child_index, binding, file_record)
            })
            .collect();
        // `par_iter().collect()` preserves job order, so pushing here regroups
        // each child's bindings (and Some(file_record)s) in their original order.
        for (child_index, binding, file_record) in rendered {
            let entry = &mut precomputed_child_ui[child_index];
            if let Some(file_record) = file_record {
                entry.1.push(file_record);
            }
            entry.0.push(binding);
        }
    }
    let t_ui = ui_render_start.elapsed();
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
            if opts.ui_only_files {
                projected_material_sidecar_path(
                    p4k,
                    materials,
                    &child.material_path,
                    &child.geometry_path,
                    &child.entity_name,
                    opts.texture_mip,
                    &mut mtl_cache,
                )
            } else {
                write_material_sidecar(
                    &mut files,
                    p4k,
                    &mut png_cache,
                    &mut texture_cache,
                    &mut ddna_status_cache,
                    &roughness_cache,
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
            }
        }).map(|path| normalize_material_source_for_manifest(&path));
        if should_export_engine_glow_targets(child) {
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
        // Consume this child's pre-rendered UI bindings (computed in parallel
        // above) and merge their generated PNG records into `files` at the same
        // point — and in the same child order — the serial path used, keeping
        // output byte-identical.
        let (ui_bindings, ui_file_records) = std::mem::take(&mut precomputed_child_ui[index]);
        for (export_path, png_bytes) in ui_file_records {
            if !files.contains_key(&export_path) {
                insert_binary_file(&mut files, export_path, png_bytes);
            }
        }

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
            ui_bindings,
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
    log::info!(
        "[timing][decomposed] child_assets: {:.2}s ({} children, parallel ui render {:.2}s)",
        phase_start.elapsed().as_secs_f32(),
        child_count,
        t_ui.as_secs_f32(),
    );
    phase_start = Instant::now();

    let mut interior_asset_cache: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut failed_interior_asset_cache: HashSet<String> = HashSet::new();
    let mut interior_records = Vec::with_capacity(input.interiors.containers.len());
    let mut interior_placement_elapsed = std::time::Duration::ZERO;
    let mut interior_asset_resolve_elapsed = std::time::Duration::ZERO;
    let mut interior_ui_binding_elapsed = std::time::Duration::ZERO;
    let mut interior_light_elapsed = std::time::Duration::ZERO;
    let container_count = input.interiors.containers.len();
    let total_interior_placements = input
        .interiors
        .containers
        .iter()
        .map(|container| container.placements.len())
        .sum::<usize>();
    let mut processed_interior_placements = 0usize;

    // Render every interior placement's UI bindings up front in one parallel
    // pass spanning ALL containers, indexed `[container][placement]`.  Binding
    // work clusters in a few containers (cockpit, control rooms) while most hold
    // none, so a per-container `par_iter` (the previous structure) leaves cores
    // idle between containers.  Flattening across containers lets the pool
    // balance individual renders; the serial loop below consumes each
    // container's slice in order, keeping the merged `files` map byte-identical.
    let ui_binding_start = Instant::now();
    let mut precomputed_interior_ui: Vec<Vec<(Vec<UiBinding>, Vec<(String, Vec<u8>)>)>> = input
        .interiors
        .containers
        .iter()
        .map(|container| vec![(Vec::new(), Vec::new()); container.placements.len()])
        .collect();
    let interior_ui_jobs: Vec<(usize, usize)> = input
        .interiors
        .containers
        .iter()
        .enumerate()
        .flat_map(|(container_index, container)| {
            container
                .placements
                .iter()
                .enumerate()
                .filter(|(_, placement)| !placement.ui_bindings.is_empty())
                .map(move |(placement_index, _)| (container_index, placement_index))
        })
        .collect();
    if !interior_ui_jobs.is_empty() {
        let rendered: Vec<((usize, usize), (Vec<UiBinding>, Vec<(String, Vec<u8>)>))> =
            interior_ui_jobs
                .par_iter()
                .map(|&(container_index, placement_index)| {
                    let placement = &input.interiors.containers[container_index].placements
                        [placement_index];
                    let records = generate_ui_binding_records_detached(
                        &placement.ui_bindings,
                        db,
                        p4k,
                        opts.texture_mip,
                        &input.entity_name,
                        &input.geometry_path,
                        &scene_manifest_path,
                        root_manufacturer_id.as_deref(),
                        ui_loc_data.as_ref(),
                        ui_defaults_registry.as_ref(),
                        &ui_ship_data,
                        &ui_render_cache,
                    );
                    ((container_index, placement_index), records)
                })
                .collect();
        for ((container_index, placement_index), records) in rendered {
            precomputed_interior_ui[container_index][placement_index] = records;
        }
    }
    interior_ui_binding_elapsed += ui_binding_start.elapsed();

    // Normalize each unique interior CGF's paths + cache key exactly once.
    // Placements re-reference `unique_cgfs` by `mesh_index`, so on a capital
    // ship this collapses ~24k redundant `normalize_source_path` calls (one per
    // placement, twice) down to one per distinct CGF. Values are identical to
    // the per-placement recomputation, so output is byte-identical.
    struct PrecomputedCgf {
        normalized_cgf_path: String,
        normalized_material_path: Option<String>,
        cache_key: String,
    }
    let precomputed_cgfs: Vec<PrecomputedCgf> = input
        .interiors
        .unique_cgfs
        .iter()
        .map(|entry| {
            let normalized_cgf_path = normalize_source_path(p4k, &entry.cgf_path);
            let normalized_material_path = entry
                .material_path
                .as_deref()
                .map(|path| normalize_source_path(p4k, path));
            let cache_key =
                interior_asset_lookup_key(&normalized_cgf_path, normalized_material_path.as_deref());
            PrecomputedCgf {
                normalized_cgf_path,
                normalized_material_path,
                cache_key,
            }
        })
        .collect();

    for (index, container) in input.interiors.containers.iter().enumerate() {
        let palette_id = container
            .palette
            .as_ref()
            .map(|palette| register_palette(&mut palette_records, palette));
        let mut placements = Vec::with_capacity(container.placements.len());
        let precomputed_placement_ui = std::mem::take(&mut precomputed_interior_ui[index]);
        let placement_start = Instant::now();
        for (placement, (ui_bindings, file_records)) in container
            .placements
            .iter()
            .zip(precomputed_placement_ui.into_iter())
        {
            let asset_resolve_start = Instant::now();
            let entry = &input.interiors.unique_cgfs[placement.mesh_index];
            // Per-placement palette override (loadout-attached children like
            // fire-extinguisher tanks with their own `kegr_red_black` palette)
            // takes precedence over the container's palette. Register it in
            // the manifest so the addon can look it up by id.
            let placement_palette_id = placement
                .palette
                .as_ref()
                .map(|palette| register_palette(&mut palette_records, palette));
            let effective_palette_id = placement_palette_id
                .clone()
                .or_else(|| palette_id.clone());
            let effective_palette_ref = placement
                .palette
                .as_ref()
                .or(container.palette.as_ref());
            let precomp = &precomputed_cgfs[placement.mesh_index];
            let cache_key = precomp.cache_key.clone();
            if failed_interior_asset_cache.contains(&cache_key) {
                interior_asset_resolve_elapsed += asset_resolve_start.elapsed();
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
            let (mesh_asset, material_sidecar) =
                if let Some(cached) = interior_asset_cache.get(&cache_key) {
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
                        interior_asset_resolve_elapsed += asset_resolve_start.elapsed();
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
                        projected_material_sidecar_path(
                            p4k,
                            materials,
                            entry.material_path.as_deref().unwrap_or(""),
                            &entry.cgf_path,
                            &entry.name,
                            opts.texture_mip,
                            &mut mtl_cache,
                        )
                    }).map(|path| normalize_material_source_for_manifest(&path));
                    let material_sidecar = interior_material_view.sidecar_materials.as_ref().map(|materials| {
                        if opts.ui_only_files {
                            projected_material_sidecar_path(
                                p4k,
                                materials,
                                entry.material_path.as_deref().unwrap_or(""),
                                &entry.cgf_path,
                                &entry.name,
                                opts.texture_mip,
                                &mut mtl_cache,
                            )
                        } else {
                            write_material_sidecar(
                                &mut files,
                                p4k,
                                &mut png_cache,
                                &mut texture_cache,
                                &mut ddna_status_cache,
                                &roughness_cache,
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
                        }
                    }).map(|path| normalize_material_source_for_manifest(&path));
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
            interior_asset_resolve_elapsed += asset_resolve_start.elapsed();

            register_livery_usage(
                &mut livery_usage,
                effective_palette_id.as_deref(),
                effective_palette_ref,
                &entry.name,
                material_sidecar.as_deref(),
            );

            for (export_path, png_bytes) in file_records {
                if !files.contains_key(&export_path) {
                    insert_binary_file(&mut files, export_path, png_bytes);
                }
            }

            placements.push(InteriorPlacementRecord {
                cgf_path: precomp.normalized_cgf_path.clone(),
                material_path: precomp.normalized_material_path.clone(),
                mesh_asset,
                material_sidecar,
                entity_class_guid: None,
                ui_bindings,
                transform: placement.transform,
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
                    if export_texture_asset(
                        &mut files,
                        p4k,
                        &mut png_cache,
                        &mut texture_cache,
                        src,
                        TextureFlavor::Generic,
                        opts.texture_mip,
                        existing_asset_paths,
                    )
                    .is_some()
                    {
                        return Some(texture_relative_path(
                            p4k,
                            src,
                            TextureFlavor::Generic,
                            opts.texture_mip,
                        ));
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
        "[timing][decomposed] interior_asset_resolve: {:.2}s",
        interior_asset_resolve_elapsed.as_secs_f32()
    );
    log::info!(
        "[timing][decomposed] interior_ui_bindings: {:.2}s",
        interior_ui_binding_elapsed.as_secs_f32()
    );
    log::info!(
        "[timing][decomposed] interior_placement_other: {:.2}s",
        interior_placement_elapsed
            .saturating_sub(interior_asset_resolve_elapsed)
            .saturating_sub(interior_ui_binding_elapsed)
            .as_secs_f32()
    );
    log::info!(
        "[timing][decomposed] interior_lights: {:.2}s",
        interior_light_elapsed.as_secs_f32()
    );
    log::info!(
        "[timing][decomposed] interior_assets: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );
    phase_start = Instant::now();

    let root_animations = if opts.include_animations {
        let mut clips: Vec<serde_json::Value> = Vec::new();
        // Map from clip name → index in `clips`, used to merge same-named clips
        // from different child skeletons (e.g. landing_gear_extend from front/left/right CHRs).
        let mut name_to_index = std::collections::HashMap::<String, usize>::new();

        let mut append_from_skeleton = |skeleton_path: &str, include_unmatched: bool, allow_bone_subset_fallback: bool, namespace: Option<&str>| {
            match crate::animation::extract_animations_for_skeleton_json(p4k, skeleton_path, include_unmatched, allow_bone_subset_fallback) {
                Ok(Some(serde_json::Value::Array(values))) => {
                    for mut clip in values {
                        let original = clip
                            .get("name")
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .to_string();
                        // Socpak interior children (`namespace = Some(entity stem)`):
                        // key each clip by entity-type so different entities'
                        // identically-named clips (every door's `door_open`) don't
                        // merge, and attach a human-readable panel label. Instances of
                        // the same entity-type share the key and still merge.
                        let name = match namespace {
                            Some(stem) if !original.is_empty() => {
                                let key = interior_clip_key(stem, &original);
                                if let Some(object) = clip.as_object_mut() {
                                    object.insert(
                                        "name".to_string(),
                                        serde_json::Value::String(key.clone()),
                                    );
                                    object.insert(
                                        "display_name".to_string(),
                                        serde_json::Value::String(humanize_animation_label(
                                            stem, &original,
                                        )),
                                    );
                                }
                                key
                            }
                            _ => original,
                        };
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
            append_from_skeleton(skeleton_path, true, false, None);
        }
        // With no root skeleton (the socpak interior case — ships always have a
        // root rig), children are independent interior entities; namespace their
        // clips per entity-type so distinct mechanisms stay separate.
        let namespace_children = input.root_skeleton_source_path.is_none();
        for child in &input.children {
            if let Some(skeleton_path) = child.skeleton_source_path.as_deref() {
                let namespace = namespace_children.then_some(child.entity_name.as_str());
                append_from_skeleton(skeleton_path, false, true, namespace);
            }
        }

        // Component animation tracks (doors, ladders, beds, …) live in a shared
        // .dba; the root-skeleton sweep stamps the blocks whose nodes are not in
        // the hull NMC with the exterior CGA and no source_node_name. Resolve
        // those against the entity's own interior + child rigs so each track is
        // stamped with the geometry that actually owns its node.
        let rig_paths: Vec<&str> = input
            .interiors
            .unique_cgfs
            .iter()
            .map(|entry| entry.cgf_path.as_str())
            .chain(
                input
                    .children
                    .iter()
                    .filter_map(|child| child.skeleton_source_path.as_deref()),
            )
            .collect();
        let restamped = restamp_unresolved_animation_channels(&mut clips, p4k, rig_paths);
        if restamped > 0 {
            log::info!(
                "[anim] re-stamped {restamped} unresolved animation channel(s) from interior/child rigs"
            );
        }

        if clips.is_empty() {
            None
        } else {
            if let Some(source) = input.root_animation_controller.as_ref() {
                if let Err(error) =
                    crate::animation::annotate_animation_fragments_json(p4k, &mut clips, source)
                {
                    log::warn!("[anim] failed to annotate Mannequin fragments: {error}");
                }
            }
            // Phase 35: split each clip into a lightweight index record
            // (kept inline in `scene.json`) and a heavy sidecar body
            // written to `Packages/<entity>/animations/<clip>.json`.
            // Deduplicate sidecar filenames in case two clips end up
            // sanitizing to the same name.
            let mut index_records: Vec<serde_json::Value> = Vec::with_capacity(clips.len());
            let mut used_filenames: std::collections::HashSet<String> =
                std::collections::HashSet::new();
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
    log::info!(
        "[timing][decomposed] animations: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );
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
        input.assembly_kind.as_deref(),
        input.weapon_assembly.as_ref(),
        opts,
    );
    report_progress(progress, INTERIOR_ASSETS_END, "Writing manifests");
    insert_ui_export_stamp(&mut files);
    insert_json_file(&mut files, scene_manifest_path, scene_manifest);
    if !opts.ui_only_files {
        finalize_palette_records(
            &mut palette_records,
            &mut files,
            p4k,
            &mut png_cache,
            &mut texture_cache,
            opts.texture_mip,
            existing_asset_paths,
        );
        if let Some(diagnostics) = input.weapon_assembly_diagnostics.as_ref() {
            insert_json_file(
                &mut files,
                package_relative_path(&package_name, "weapon_assembly_diagnostics.json"),
                diagnostics.clone(),
            );
        }
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
    }
    log::info!("[timing][decomposed] manifests: {:.2}s", phase_start.elapsed().as_secs_f32());
    log::info!("[timing][decomposed] total: {:.2}s", total_start.elapsed().as_secs_f32());

    Ok(DecomposedExport {
        files: files
            .into_files()
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
    assembly_kind: Option<&str>,
    weapon_assembly: Option<&serde_json::Value>,
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
    if let Some(kind) = assembly_kind {
        manifest["assembly_kind"] = serde_json::json!(kind);
    }
    if let Some(weapon_assembly) = weapon_assembly {
        manifest["weapon_assembly"] = weapon_assembly.clone();
    }
    if !engine_glow_targets.is_empty() {
        manifest["controls"] = serde_json::json!({
            "engine_glow": {
                "label": "Engine Glow",
                "units": "emission_strength",
                "min_strength": 0.0,
                "max_strength": 200.0,
                "default_strength": 0.0,
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
        "ui_bindings": instance.ui_bindings.iter().map(ui_binding_json).collect::<Vec<_>>(),
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
                "ui_bindings": placement.ui_bindings.iter().map(ui_binding_json).collect::<Vec<_>>(),
                "transform": placement.transform,
                "palette_id": placement.palette_id,
            })
        }).collect::<Vec<_>>(),
        "lights": container.lights,
    })
}

fn ui_binding_json(binding: &UiBinding) -> serde_json::Value {
    serde_json::json!({
        "binding_kind": binding.binding_kind,
        "source_entity_name": binding.source_entity_name,
        "helper_name": binding.helper_name,
        "default_view": binding.default_view,
        "default_state_is_off": binding.default_state_is_off,
        "default_state_name": binding.default_state_name,
        "default_light_color": binding.default_light_color,
        "default_light_intensity_milli": binding.default_light_intensity_milli,
        "canvas_guid": binding.canvas_guid,
        "canvas_record_name": binding.canvas_record_name,
        "canvas_record_path": binding.canvas_record_path,
        "content_canvas_guid": binding.content_canvas_guid,
        "content_canvas_record_name": binding.content_canvas_record_name,
        "screen_name_loc_key": binding.screen_name_loc_key,
        "transit_location_loc_key": binding.transit_location_loc_key,
        "dashboard_view_index": binding.dashboard_view_index,
        "dashboard_screen_slot": binding.dashboard_screen_slot,
        "owner_source_file": binding.owner_source_file,
        "runtime_image_source": binding.runtime_image_source,
        "generated_image_path": binding.generated_image_path,
        "generated_context_manifest_path": binding.generated_context_manifest_path,
        "generated_resolved_source_path": binding.generated_resolved_source_path,
        "generated_backend": binding.generated_backend,
        "generated_provenance": binding.generated_provenance,
        "generated_confidence": binding.generated_confidence,
        "ui_screen_aspect_w_over_h": binding.ui_screen_aspect_w_over_h,
    })
}

fn write_mesh_asset(
    files: &mut OutputFiles,
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
    let requested_path =
        mesh_asset_relative_path(p4k, geometry_path, fallback_name, lod_level, format);
    if existing_asset_paths
        .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
    {
        return Ok(requested_path);
    }
    Ok(insert_binary_file(files, requested_path, Vec::new()))
}

fn write_material_sidecar(
    files: &mut OutputFiles,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    ddna_status_cache: &mut HashMap<String, (Option<TextureExportRef>, TextureDerivationStatus)>,
    roughness_cache: &RoughnessCache,
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
    let source_material_path = canonical_material_source_path(
        p4k,
        materials,
        material_path,
        geometry_path,
        mtl_cache,
    );
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
                ddna_status_cache,
                roughness_cache,
                material,
                texture_mip,
                existing_asset_paths,
                mtl_cache,
                &source_material_path,
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
    files: &mut OutputFiles,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    ddna_status_cache: &mut HashMap<String, (Option<TextureExportRef>, TextureDerivationStatus)>,
    roughness_cache: &RoughnessCache,
    material: &SubMaterial,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
    mtl_cache: &mut HashMap<String, Option<MtlFile>>,
    source_material_path: &str,
) -> ExtractedMaterialEntry {
    let mut derived_texture_exports = Vec::new();
    let mut ddna_derivations = Vec::new();
    for path in normal_gloss_ddna_source_paths(material) {
        let (roughness_texture, derivation_status) = export_ddna_roughness_asset_with_status(
            files,
            p4k,
            texture_cache,
            ddna_status_cache,
            roughness_cache,
            &path,
            texture_mip,
            existing_asset_paths,
        );
        if let Some(roughness_texture) = roughness_texture {
            derived_texture_exports.push(roughness_texture);
        }
        ddna_derivations.push(derivation_status);
    }
    let ddna_smoothness_sources = exported_ddna_smoothness_sources(&ddna_derivations);

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
                material,
                source_material_path,
                texture_mip,
                existing_asset_paths,
                &ddna_smoothness_sources,
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
                alpha_channel: None,
                derived_from_texture_identity: None,
                derived_from_semantic: None,
                derived_from_channel: None,
                value_channel: None,
                value_transform: None,
                packed_texture_format: None,
                packed_channel_semantics: None,
                constant_channel_values: None,
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
                alpha_semantic: ddna_alpha_semantic_for_exported_source(
                    p4k,
                    path,
                    TextureSemanticRole::NormalGloss,
                    &ddna_smoothness_sources,
                )
                .map(str::to_string),
                alpha_channel: ddna_alpha_channel_for_exported_source(
                    p4k,
                    path,
                    TextureSemanticRole::NormalGloss,
                    &ddna_smoothness_sources,
                )
                .map(str::to_string),
                derived_from_texture_identity: None,
                derived_from_semantic: None,
                derived_from_channel: None,
                value_channel: None,
                value_transform: None,
                packed_texture_format: None,
                packed_channel_semantics: None,
                constant_channel_values: None,
            });
        }
    }

    let layer_exports = material
        .layers
        .iter()
        .map(|layer| {
            let layer_material_path = normalize_source_path(p4k, &layer.path);
            let layer_mtl = load_mtl_cached(p4k, mtl_cache, &layer.path);
            let layer_sub = layer_mtl
                .as_ref()
                .and_then(|mtl| crate::mtl::resolve_layer_submaterial(mtl, &layer.sub_material));
            let mut layer_ddna_derivations = Vec::new();
            let mut roughness_texture = None;
            if let Some(layer_sub) = layer_sub {
                for path in normal_gloss_ddna_source_paths(layer_sub) {
                    let (candidate, derivation_status) = export_ddna_roughness_asset_with_status(
                        files,
                        p4k,
                        texture_cache,
                        ddna_status_cache,
                        roughness_cache,
                        &path,
                        texture_mip,
                        existing_asset_paths,
                    );
                    if roughness_texture.is_none() {
                        roughness_texture = candidate;
                    }
                    layer_ddna_derivations.push(derivation_status);
                }
            }
            let layer_ddna_smoothness_sources =
                exported_ddna_smoothness_sources(&layer_ddna_derivations);
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
                                sub,
                                &layer_material_path,
                                texture_mip,
                                existing_asset_paths,
                                &layer_ddna_smoothness_sources,
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
            let roughness_export_path = roughness_texture
                .as_ref()
                .map(|texture| texture.export_path.clone());

            LayerTextureExport {
                source_material_path: layer_material_path,
                diffuse_export_path,
                normal_export_path,
                roughness_export_path,
                roughness_texture,
                ddna_derivations: layer_ddna_derivations,
                slot_exports,
            }
        })
        .collect::<Vec<_>>();

    ExtractedMaterialEntry {
        slot_exports,
        direct_texture_exports,
        layer_exports,
        derived_texture_exports,
        ddna_derivations,
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
    let blender_material_names =
        preferred_blender_material_names(&materials.materials, source_stem);

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
            if name_counts
                .get(material.name.as_str())
                .copied()
                .unwrap_or_default()
                > 1
            {
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
    let (activation_state, activation_reason) =
        material_activation_state(material, &semantic_slots);
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
    let screen_effects = screen_effects_json(material, &semantic_slots);

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
        "screen_effects": screen_effects,
        "public_params": public_params,
        "direct_textures": extracted.direct_texture_exports.iter().map(texture_ref_json).collect::<Vec<_>>(),
        "derived_textures": extracted.derived_texture_exports.iter().map(texture_ref_json).collect::<Vec<_>>(),
        "ddna_derivations": extracted
            .ddna_derivations
            .iter()
            .map(texture_derivation_status_json)
            .collect::<Vec<_>>(),
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
                "roughness_texture": extracted_layer
                    .and_then(|layer| layer.roughness_texture.as_ref())
                    .map(texture_ref_json),
                "ddna_derivations": extracted_layer
                    .map(|layer| {
                        layer
                            .ddna_derivations
                            .iter()
                            .map(texture_derivation_status_json)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
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
    files: &mut OutputFiles,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    binding: &SemanticTextureBinding,
    material: &SubMaterial,
    source_material_path: &str,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
    ddna_smoothness_sources: &HashSet<String>,
) -> serde_json::Value {
    let source_path = slot_source_path(Some(p4k), binding);
    let export_flavor = slot_texture_flavor(binding.role);
    let generated_ui = generated_ui_texture_for_binding(
        files,
        p4k,
        png_cache,
        texture_cache,
        material,
        binding,
        source_material_path,
        texture_mip,
        existing_asset_paths,
    );
    let export_path = if let Some(generated) = generated_ui.as_ref() {
        Some(generated.export_path.clone())
    } else if binding.is_virtual {
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
        (
            "is_virtual".to_string(),
            serde_json::json!(binding.is_virtual),
        ),
        ("source_path".to_string(), serde_json::json!(source_path)),
        ("export_path".to_string(), serde_json::json!(export_path)),
        (
            "export_kind".to_string(),
            serde_json::json!(
                generated_ui
                    .as_ref()
                    .map(|generated| generated.export_kind.as_str())
                    .unwrap_or_else(|| texture_export_kind(export_flavor))
            ),
        ),
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
        value.insert(
            "texture_identity".to_string(),
            serde_json::json!(texture_identity),
        );
    }
    if let Some(alpha_semantic) = ddna_alpha_semantic_for_exported_source(
        p4k,
        &binding.path,
        binding.role,
        ddna_smoothness_sources,
    ) {
        value.insert(
            "alpha_semantic".to_string(),
            serde_json::json!(alpha_semantic),
        );
    }
    if let Some(alpha_channel) = ddna_alpha_channel_for_exported_source(
        p4k,
        &binding.path,
        binding.role,
        ddna_smoothness_sources,
    ) {
        value.insert(
            "alpha_channel".to_string(),
            serde_json::json!(alpha_channel),
        );
    }
    if let Some(texture_transform) = texture_transform_json(&binding.authored_child_blocks) {
        value.insert("texture_transform".to_string(), texture_transform);
    }
    if let Some(generated) = generated_ui {
        value.insert(
            "generated_ui".to_string(),
            serde_json::json!({
                "identity": generated.identity_components,
                "frame_selection": "default_on",
                "source_path": generated.source_path,
                "provenance": generated.provenance,
            }),
        );
    }
    serde_json::Value::Object(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedUiTexture {
    export_path: String,
    export_kind: String,
    source_path: String,
    provenance: String,
    identity_components: Vec<String>,
}

fn generated_ui_texture_for_binding(
    _files: &mut OutputFiles,
    _p4k: &MappedP4k,
    _png_cache: &mut PngCache,
    _texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    material: &SubMaterial,
    binding: &SemanticTextureBinding,
    _source_material_path: &str,
    _texture_mip: u32,
    _existing_asset_paths: Option<&HashSet<String>>,
) -> Option<GeneratedUiTexture> {
    if !binding.is_virtual || binding.role != TextureSemanticRole::RenderToTexture {
        return None;
    }
    if !matches!(
        material.shader_family(),
        ShaderFamily::DisplayScreen | ShaderFamily::UiPlane
    ) {
        return None;
    }
    // Canvas-based rendering is handled by the UiBinding path (generated_ui_binding_record).
    // The material RTT texture slot has no static image for this export phase.
    None
}

fn generated_ui_binding_record(
    binding: &UiBinding,
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_entity_name: &str,
    root_geometry_path: &str,
    scene_manifest_path: &str,
    root_manufacturer_id: Option<&str>,
    loc_data: &crate::ui_pipeline::UiLocData,
    defaults_registry: &starbreaker_ui::DefaultValueRegistry,
    ship_data: &crate::ui_pipeline::UiShipData,
    render_cache: &HashMap<UiRenderKey, Result<Vec<u8>, String>>,
) -> (UiBinding, Option<(String, Vec<u8>)>) {
    let mut binding = binding.clone();
    // Reuse the pre-rendered image for this screen if available (most screens
    // recur across placements/children); otherwise render live.
    let render_result = match render_cache.get(&ui_render_key(&binding)) {
        Some(result) => result.clone(),
        None => crate::ui_pipeline::render_ui_binding_png(
            &binding,
            db,
            p4k,
            texture_mip,
            root_manufacturer_id,
            loc_data,
            ship_data,
            Some(defaults_registry),
            Some(root_entity_name),
        ),
    };
    match render_result {
        Ok(png_bytes) => {
            let export_path = generated_ui_binding_path(
                root_entity_name,
                root_geometry_path,
                root_manufacturer_id,
                &binding,
            );
            let file_record = Some((export_path.clone(), png_bytes));
            binding.generated_image_path = Some(export_path);
            binding.generated_context_manifest_path = Some(scene_manifest_path.to_string());
            let selected_style_source = root_manufacturer_id
                .map(|id| format!("manufacturer:{id}"))
                .unwrap_or_else(|| "manufacturer:drak".to_string());
            let selected_swf_source = binding
                .runtime_image_source
                .clone()
                .or_else(|| binding.canvas_widget_canvas_path.clone())
                .or_else(|| binding.canvas_record_path.clone())
                .unwrap_or_else(|| "unknown".to_string());
            binding.generated_resolved_source_path = Some(selected_swf_source.clone());
            binding.generated_backend = Some("starbreaker-ui".to_string());
            let mut fallback_counters = serde_json::Map::new();
            fallback_counters.insert(
                "canvas".to_string(),
                serde_json::json!(if binding.content_canvas_guid.is_none() { 1u32 } else { 0u32 }),
            );
            fallback_counters.insert(
                "style".to_string(),
                serde_json::json!(if binding.canvas_widget_canvas_path.is_none() { 1u32 } else { 0u32 }),
            );
            fallback_counters.insert(
                "swf".to_string(),
                serde_json::json!(if binding.runtime_image_source.is_none() { 1u32 } else { 0u32 }),
            );
            let unresolved_references = [
                binding
                    .canvas_guid
                    .is_none()
                    .then_some("canvas_guid".to_string()),
                binding
                    .content_canvas_guid
                    .is_none()
                    .then_some("content_canvas_guid".to_string()),
                binding
                    .runtime_image_source
                    .is_none()
                    .then_some("runtime_image_source".to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            let mut confidence: i16 = 100;
            if binding.content_canvas_guid.is_none() {
                confidence -= 20;
            }
            if binding.runtime_image_source.is_none() {
                confidence -= 20;
            }
            if binding.canvas_record_name.is_none() {
                confidence -= 10;
            }
            if binding.helper_name.is_none() {
                confidence -= 10;
            }
            if unresolved_references.len() > 2 {
                confidence -= 10;
            }
            let confidence = confidence.clamp(0, 100) as u8;
            binding.generated_confidence = Some(confidence);
            binding.generated_provenance = Some(
                serde_json::to_string(&serde_json::json!({
                    "resolved_canvas_ids": [
                        binding.canvas_guid.clone(),
                        binding.content_canvas_guid.clone(),
                    ],
                    "resolved_canvas_names": [
                        binding.canvas_record_name.clone(),
                        binding.content_canvas_record_name.clone(),
                        binding.helper_name.clone(),
                    ],
                    "selected_style_source": selected_style_source,
                    "selected_swf_source": selected_swf_source,
                    "render_backend": "starbreaker-ui",
                    "fallback_counters": fallback_counters,
                    "unresolved_references": unresolved_references,
                    "confidence": confidence,
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            );
            (binding, file_record)
        }
        Err(e) => {
            log::warn!(
                "ui render failed for helper {:?} (kind {}): {}",
                binding.helper_name, binding.binding_kind, e
            );
            (binding, None)
        }
    }
}

fn generate_ui_binding_records_detached(
    bindings: &[UiBinding],
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_entity_name: &str,
    root_geometry_path: &str,
    scene_manifest_path: &str,
    root_manufacturer_id: Option<&str>,
    loc_data: Option<&crate::ui_pipeline::UiLocData>,
    defaults_registry: Option<&starbreaker_ui::DefaultValueRegistry>,
    ship_data: &crate::ui_pipeline::UiShipData,
    render_cache: &HashMap<UiRenderKey, Result<Vec<u8>, String>>,
) -> (Vec<UiBinding>, Vec<(String, Vec<u8>)>) {
    // Most interior placements carry no UI bindings; skip all binding work for
    // them entirely.
    if bindings.is_empty() {
        return (Vec::new(), Vec::new());
    }
    // Localization (multi-MB `global.ini`) is parsed once per export and the
    // default registry derived from it once too; both are shared by reference
    // across every binding render here.  The caller builds them whenever any
    // binding exists, so they are always `Some` here.
    let loc_data = loc_data
        .expect("UiLocData must be loaded by the caller when UI bindings are present");
    let defaults_registry = defaults_registry
        .expect("default registry must be built by the caller when UI bindings are present");
    let mut generated = bindings
        .par_iter()
        .enumerate()
        .map(|(idx, binding)| {
            let (binding, file_record) = generated_ui_binding_record(
                binding,
                db,
                p4k,
                texture_mip,
                root_entity_name,
                root_geometry_path,
                scene_manifest_path,
                root_manufacturer_id,
                loc_data,
                defaults_registry,
                ship_data,
                render_cache,
            );
            (idx, binding, file_record)
        })
        .collect::<Vec<_>>();

    generated.sort_by_key(|(idx, _, _)| *idx);

    let mut ui_bindings = Vec::with_capacity(generated.len());
    let mut file_records = Vec::new();
    for (_, binding, file_record) in generated {
        if let Some(file_record) = file_record {
            file_records.push(file_record);
        }
        ui_bindings.push(binding);
    }
    (ui_bindings, file_records)
}

fn generated_ui_binding_path(
    root_entity_name: &str,
    root_geometry_path: &str,
    root_manufacturer_id: Option<&str>,
    binding: &UiBinding,
) -> String {
    let ui_type = generated_ui_type_segment(root_geometry_path);
    let manufacturer = root_manufacturer_id
        .map(sanitize_identifier)
        .unwrap_or_else(|| "unknown".to_string());
    let ship_name = generated_ui_ship_name(root_entity_name, root_manufacturer_id);
    let asset_name = generated_ui_asset_name(binding);
    format!(
        "Data/UI/Generated/{ui_type}/{manufacturer}/{ship_name}/{asset_name}.png"
    )
}

fn generated_ui_type_segment(root_geometry_path: &str) -> &'static str {
    let lowered = root_geometry_path.replace('\\', "/").to_ascii_lowercase();
    if lowered.contains("/vehicles/") {
        "vehicle"
    } else {
        "ship"
    }
}

fn generated_ui_ship_name(root_entity_name: &str, root_manufacturer_id: Option<&str>) -> String {
    let base = clean_export_label(export_entity_basename(root_entity_name));
    let Some(manufacturer_id) = root_manufacturer_id else {
        return base;
    };
    let mut parts = base.split_whitespace();
    let Some(first) = parts.next() else {
        return base;
    };
    if !first.eq_ignore_ascii_case(manufacturer_id) {
        return base;
    }
    let remainder = parts.collect::<Vec<_>>().join(" ");
    if remainder.is_empty() {
        base
    } else {
        remainder
    }
}

fn generated_ui_asset_name(binding: &UiBinding) -> String {
    let candidates = [
        binding.content_canvas_record_name.as_deref(),
        binding.canvas_record_name.as_deref(),
        binding.helper_name.as_deref(),
        binding.default_view.as_deref(),
        binding.canvas_widget_url_postfix.as_deref(),
        Some(binding.source_entity_name.as_str()),
        Some(binding.binding_kind.as_str()),
    ];
    for candidate in candidates.into_iter().flatten() {
        let cleaned = sanitize_identifier(candidate);
        if !cleaned.is_empty() {
            // Transit screens render per FLOOR (the loc key is part of the
            // render identity) — suffix the floor so per-floor PNGs don't
            // collide on the shared canvas name.
            if let Some(loc_key) = binding.transit_location_loc_key.as_deref() {
                let floor = sanitize_identifier(loc_key.trim_start_matches('@'));
                if !floor.is_empty() {
                    return format!("{cleaned}_{floor}");
                }
            }
            return cleaned;
        }
    }
    "generated_ui".to_string()
}

/// Derive a short manufacturer id (lowercase, e.g. "drak", "rsi", "aegs") from
/// a root entity name like `DRAK_Clipper` or `RSI_AuroraMk2`.
///
/// Returns `None` when the prefix does not match any known manufacturer code.
/// Production code must NOT hard-code ship/helper names — only manufacturer
/// **codes** appear here, which are stable DataCore identifiers (not specific
/// to any single ship or component).  Per-component manufacturer overrides
/// (e.g. a Bioticorp medical bay installed on a Drake ship) are deferred to
/// Phase A5 of the UI plan and require DataCore record traversal.
fn derive_manufacturer_id(root_entity_name: &str) -> Option<String> {
    let prefix = root_entity_name
        .split(|c: char| c == '_' || c == '-' || c.is_whitespace())
        .next()
        .unwrap_or("");
    if prefix.is_empty() {
        return None;
    }
    let lower = prefix.to_ascii_lowercase();
    // Stable DataCore manufacturer prefixes (entity-record naming convention).
    const KNOWN_PREFIXES: &[&str] = &[
        "drak", "rsi", "aegs", "anvl", "misc", "crus", "orig", "xian", "banu",
        "krgn", "tmbl", "gama", "grin", "btc", "koa", "expl", "cnou", "vncl",
        "espe", "gatc", "argo", "ksar", "kgnp",
    ];
    if KNOWN_PREFIXES.iter().any(|known| *known == lower.as_str()) {
        Some(lower)
    } else {
        None
    }
}

#[cfg(test)]
mod interior_animation_naming_tests {
    use super::{humanize_animation_label, interior_clip_key};

    #[test]
    fn interior_clip_key_and_label() {
        assert_eq!(
            interior_clip_key("ht_hangar_door_top_xl_a", "door_open"),
            "ht_hangar_door_top_xl_a__door_open"
        );
        // different entities never collide → their same-named clips don't merge
        assert_ne!(
            interior_clip_key("ht_hangar_door_top_xl_a", "door_open"),
            interior_clip_key("elev_ht_cargo_pad_xl", "door_open")
        );
        // same entity-type + action → identical key → instances still merge
        assert_eq!(
            interior_clip_key("elevator_door_double_a", "door_open"),
            interior_clip_key("elevator_door_double_a", "door_open")
        );
        assert_eq!(
            humanize_animation_label("ht_hangar_door_top_xl_a", "door_open"),
            "Ht Hangar Door Top Xl A — Door Open"
        );
    }
}

#[cfg(test)]
mod manufacturer_id_tests {
    use super::{
        derive_manufacturer_id, generated_ui_asset_name, generated_ui_binding_path,
        generated_ui_ship_name, generated_ui_type_segment,
    };
    use crate::types::UiBinding;

    #[test]
    fn drake_prefix_is_recognised() {
        assert_eq!(derive_manufacturer_id("DRAK_Clipper"), Some("drak".into()));
        assert_eq!(derive_manufacturer_id("drak_pitbull"), Some("drak".into()));
    }

    #[test]
    fn rsi_aegs_anvl_recognised() {
        assert_eq!(derive_manufacturer_id("RSI_AuroraMk2"), Some("rsi".into()));
        assert_eq!(derive_manufacturer_id("AEGS_Gladius"), Some("aegs".into()));
        assert_eq!(derive_manufacturer_id("ANVL_Hawk"), Some("anvl".into()));
    }

    #[test]
    fn unknown_prefix_returns_none() {
        assert_eq!(derive_manufacturer_id("Vehicle_Screen_MFD"), None);
        assert_eq!(derive_manufacturer_id(""), None);
    }

    #[test]
    fn ui_generated_path_is_structured() {
        let binding = UiBinding {
            binding_kind: "physical".into(),
            source_entity_name: "DRAK_Clipper_Screen".into(),
            helper_name: Some("mesh_end_screen_plane".into()),
            default_view: None,
            default_state_is_off: false,
            default_state_name: None,
            default_light_color: None,
            default_light_intensity_milli: None,
            canvas_guid: None,
            canvas_record_name: None,
            canvas_record_path: None,
            canvas_widget_canvas_path: None,
            canvas_widget_url_postfix: None,
            canvas_widget_url_optional: None,
            canvas_variable_binding: None,
            content_canvas_guid: None,
            content_canvas_record_name: None,
            screen_name_loc_key: None,
            transit_location_loc_key: None,
            dashboard_view_index: None,
            dashboard_screen_slot: None,
            owner_source_file: None,
            runtime_image_source: None,
            generated_image_path: None,
            generated_context_manifest_path: None,
            generated_resolved_source_path: None,
            generated_backend: None,
            generated_provenance: None,
            generated_confidence: None,
            ui_screen_aspect_w_over_h: None,
        };

        assert_eq!(generated_ui_type_segment("Objects/Spaceships/Ships/DRAK/Clipper/exterior/test.cga"), "ship");
        assert_eq!(generated_ui_ship_name("DRAK_Clipper", Some("drak")), "Clipper");
        assert_eq!(generated_ui_asset_name(&binding), "mesh_end_screen_plane");
        assert_eq!(
            generated_ui_binding_path(
                "DRAK_Clipper",
                "Objects/Spaceships/Ships/DRAK/Clipper/exterior/test.cga",
                Some("drak"),
                &binding,
            ),
            "Data/UI/Generated/ship/drak/Clipper/mesh_end_screen_plane.png"
        );
    }
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
    files: &mut OutputFiles,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    source_path: &str,
    flavor: TextureFlavor,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<String> {
    let normalized_source = normalize_source_path(p4k, source_path);
    let cache_key = texture_cache_key(&normalized_source, flavor);
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
        TextureFlavor::Generic => crate::pipeline::cached_load_keyed(
            p4k,
            source_path,
            texture_mip,
            "",
            png_cache,
            crate::pipeline::load_diffuse_texture,
        ),
        TextureFlavor::Normal => crate::pipeline::cached_load_keyed(
            p4k,
            source_path,
            texture_mip,
            "@n",
            png_cache,
            crate::pipeline::load_normal_texture,
        ),
        TextureFlavor::Roughness => crate::pipeline::cached_load_keyed(
            p4k,
            source_path,
            texture_mip,
            "@r",
            png_cache,
            crate::pipeline::load_roughness_texture,
        ),
    }?;

    let stored_path = insert_binary_file(files, requested_path, bytes);
    texture_cache.insert(cache_key, stored_path.clone());
    Some(stored_path)
}

fn texture_cache_key(normalized_source: &str, flavor: TextureFlavor) -> (String, TextureFlavor) {
    (normalized_source.to_ascii_lowercase(), flavor)
}

fn register_palette(
    records: &mut BTreeMap<String, PaletteRecord>,
    palette: &TintPalette,
) -> String {
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
    files: &mut OutputFiles,
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
    let entry = usages
        .entry(palette_id.to_string())
        .or_insert_with(|| LiveryUsage {
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

fn projected_material_sidecar_path(
    p4k: &MappedP4k,
    materials: &MtlFile,
    material_path: &str,
    geometry_path: &str,
    fallback_name: &str,
    texture_mip: u32,
    mtl_cache: &mut HashMap<String, Option<MtlFile>>,
) -> String {
    let source_material_path = canonical_material_source_path(
        p4k,
        materials,
        material_path,
        geometry_path,
        mtl_cache,
    );
    material_sidecar_relative_path(&source_material_path, fallback_name, texture_mip)
}

fn canonical_material_source_path(
    p4k: &MappedP4k,
    materials: &MtlFile,
    material_path: &str,
    geometry_path: &str,
    _mtl_cache: &mut HashMap<String, Option<MtlFile>>,
) -> String {
    let source_material_path = material_source_path(p4k, materials, material_path, geometry_path)
        .replace('\\', "/");
    normalize_material_source_for_manifest(&source_material_path)
}

fn normalize_material_source_for_manifest(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if normalized
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data/"))
    {
        format!("Data/{}", normalized[5..].to_ascii_lowercase())
    } else {
        normalized.to_ascii_lowercase()
    }
}

fn material_source_request(materials: &MtlFile, material_path: &str, geometry_path: &str) -> String {
    if let Some(source_path) = materials.source_path.as_ref() {
        source_path.clone()
    } else if !material_path.is_empty() {
        if material_path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.'))
        {
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

fn requested_material_source_path(
    p4k: &MappedP4k,
    material_path: Option<&str>,
    geometry_path: &str,
) -> String {
    let request = if let Some(material_path) = material_path.filter(|path| !path.is_empty()) {
        if material_path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.'))
        {
            material_path.to_string()
        } else {
            format!("{material_path}.mtl")
        }
    } else {
        replace_extension(geometry_path, ".mtl")
    };
    normalize_source_path(p4k, &request)
}

fn existing_asset_set_contains(
    existing_asset_paths: Option<&HashSet<String>>,
    relative_path: &str,
) -> bool {
    existing_asset_paths.is_some_and(|paths| paths.contains(&relative_path.to_ascii_lowercase()))
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

    let material_source = normalize_material_source_for_manifest(
        &requested_material_source_path(p4k, entry.material_path.as_deref(), &entry.cgf_path),
    );
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
        format!(
            "Data/generated/{}{}",
            sanitize_identifier(fallback_name),
            extension
        )
    } else {
        let _ = p4k;
        replace_extension(&normalize_requested_source_path(geometry_path), extension)
    };
    insert_stem_suffix(&base, &format!("_LOD{lod}"))
}

fn material_sidecar_relative_path(source_material_path: &str, fallback_name: &str, mip: u32) -> String {
    let normalized_source_material_path = normalize_material_source_for_manifest(source_material_path);
    let base = if normalized_source_material_path.is_empty() {
        format!("Data/generated/{}.materials.json", sanitize_identifier(fallback_name))
    } else {
        replace_extension(&normalized_source_material_path, ".materials.json")
    };
    insert_stem_suffix(&base, &format!("_TEX{mip}"))
}

fn texture_relative_path(
    p4k: &MappedP4k,
    source_path: &str,
    flavor: TextureFlavor,
    mip: u32,
) -> String {
    let normalized = normalize_source_path(p4k, source_path);
    texture_relative_path_from_normalized(&normalized, flavor, mip)
}

fn texture_relative_path_from_normalized(
    normalized: &str,
    flavor: TextureFlavor,
    mip: u32,
) -> String {
    let base = match flavor {
        TextureFlavor::Generic => replace_extension(&normalized, ".png"),
        TextureFlavor::Normal => replace_extension(&normalized, ".png"),
        TextureFlavor::Roughness => {
            insert_stem_suffix(&replace_extension(&normalized, ".png"), "_roughness")
        }
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
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // IHDR chunk: 2x2 8-bit RGBA
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
        0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE,
        // IDAT chunk: white 2x2 image (zlib compressed, then CRC)
        0x00, 0x00, 0x00, 0x1B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0xF8, 0xFF, 0xFF, 0x3F,
        0x03, 0x03, 0x03, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x09, 0x00, 0x01, 0xBE, 0xCE, 0x66, 0xA9,
        // IEND chunk
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

fn export_gobo_as_exr(p4k: &MappedP4k, source_path: &str, texture_mip: u32) -> Option<Vec<u8>> {
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

    let channels: AnyChannels<FlatSamples> = AnyChannels::sort(
        vec![
            AnyChannel::new("R", FlatSamples::F32(r_channel)),
            AnyChannel::new("G", FlatSamples::F32(g_channel)),
            AnyChannel::new("B", FlatSamples::F32(b_channel)),
        ]
        .into(),
    );

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

/// Marker written next to the generated UI PNGs on every export that produced
/// them. The starbreaker-ui visual guard reads it to hard-fail comparisons
/// against PNGs that predate the current build (the stale-export trap —
/// `crates/starbreaker-ui/docs/ui-process-improvements.md` ledger item 20).
const UI_EXPORT_STAMP_PATH: &str = "Data/UI/Generated/.export_stamp.json";

/// Insert `.export_stamp.json` when this export produced any generated UI
/// file. Skipped otherwise so an export without UI screens cannot pass off
/// another ship's stale PNGs as fresh. Never fails: every field degrades to a
/// sentinel ("unknown" / 0) rather than erroring the export.
fn insert_ui_export_stamp(files: &mut OutputFiles) {
    let has_generated_ui = files
        .iter()
        .any(|(path, _)| path.starts_with("Data/UI/Generated/"));
    if !has_generated_ui {
        return;
    }
    let written_at_epoch_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let git_describe = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let binary_built_at_epoch_s = std::env::current_exe()
        .and_then(|path| path.metadata())
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    insert_json_file(
        files,
        UI_EXPORT_STAMP_PATH.to_string(),
        serde_json::json!({
            "written_at_epoch_s": written_at_epoch_s,
            "git_describe": git_describe,
            "binary_built_at_epoch_s": binary_built_at_epoch_s,
        }),
    );
}

/// The export's output file map plus an index that makes per-segment case
/// canonicalization O(path depth) instead of O(files) (the old code scanned
/// every key for every inserted segment — quadratic over an export). `case_index`
/// maps a lowercase path prefix to the canonical-cased final segment of the
/// FIRST key that introduced it, exactly matching the previous "first key wins"
/// behaviour.
pub(crate) struct OutputFiles {
    files: BTreeMap<String, Vec<u8>>,
    case_index: HashMap<String, String>,
}

impl OutputFiles {
    pub(crate) fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            case_index: HashMap::new(),
        }
    }
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.files.contains_key(key)
    }
    #[allow(dead_code)] // used by tests and the Phase-2 merge/logging
    pub(crate) fn get(&self, key: &str) -> Option<&Vec<u8>> {
        self.files.get(key)
    }
    #[allow(dead_code)] // used by tests and the Phase-2 merge/logging
    pub(crate) fn len(&self) -> usize {
        self.files.len()
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Vec<u8>)> {
        self.files.iter()
    }
    /// Consume the wrapper, yielding the underlying path→bytes map (no copy).
    pub(crate) fn into_files(self) -> BTreeMap<String, Vec<u8>> {
        self.files
    }
    /// Canonicalize `requested_path`'s segment case against prior inserts, then
    /// insert `bytes` (deduping identical content, hashing genuine collisions).
    /// Returns the stored path.
    pub(crate) fn insert_canonical(&mut self, requested_path: String, bytes: Vec<u8>) -> String {
        let requested_path = self.canonicalize_case(&requested_path);
        if let Some(existing) = self.files.get(&requested_path) {
            if existing == &bytes {
                return requested_path;
            }
        }
        let mut candidate = requested_path.clone();
        while let Some(existing) = self.files.get(&candidate) {
            if existing == &bytes {
                return candidate;
            }
            candidate = hashed_variant_path(&requested_path, &bytes);
        }
        self.record_case(&candidate);
        self.files.insert(candidate.clone(), bytes);
        candidate
    }
    /// O(depth) replacement for the old `canonicalize_output_path_case` scan.
    fn canonicalize_case(&self, requested_path: &str) -> String {
        let mut lower_prefix = String::new();
        let mut parts = Vec::new();
        for (depth, part) in requested_path.split('/').enumerate() {
            if depth > 0 {
                lower_prefix.push('/');
            }
            lower_prefix.push_str(&part.to_ascii_lowercase());
            let canonical = self
                .case_index
                .get(&lower_prefix)
                .cloned()
                .unwrap_or_else(|| part.to_string());
            parts.push(canonical);
        }
        parts.join("/")
    }
    /// Record each prefix of a freshly stored path so later paths adopt its case.
    fn record_case(&mut self, stored_path: &str) {
        let mut lower_prefix = String::new();
        for (depth, part) in stored_path.split('/').enumerate() {
            if depth > 0 {
                lower_prefix.push('/');
            }
            lower_prefix.push_str(&part.to_ascii_lowercase());
            self.case_index
                .entry(lower_prefix.clone())
                .or_insert_with(|| part.to_string());
        }
    }
}

fn insert_json_file(
    files: &mut OutputFiles,
    requested_path: String,
    value: serde_json::Value,
) -> String {
    let bytes = serde_json::to_vec_pretty(&value).unwrap_or_else(|_| b"{}".to_vec());
    files.insert_canonical(requested_path, bytes)
}

fn insert_binary_file(files: &mut OutputFiles, requested_path: String, bytes: Vec<u8>) -> String {
    files.insert_canonical(requested_path, bytes)
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

fn screen_effects_json(
    material: &SubMaterial,
    semantic_slots: &[SemanticTextureBinding],
) -> serde_json::Value {
    if !matches!(
        material.shader_family(),
        ShaderFamily::DisplayScreen | ShaderFamily::UiPlane | ShaderFamily::Monitor
    ) {
        return serde_json::Value::Null;
    }

    let pixel_layout = semantic_slots
        .iter()
        .find(|binding| binding.role == TextureSemanticRole::ScreenPixelLayout);
    let mut effects = serde_json::Map::new();
    effects.insert("apply_crt".to_string(), serde_json::json!(pixel_layout.is_some()));

    if let Some(binding) = pixel_layout {
        effects.insert("source".to_string(), serde_json::json!(binding.role.as_str()));
        effects.insert("pixel_layout_slot".to_string(), serde_json::json!(binding.slot));
        effects.insert("pixel_layout_source_path".to_string(), serde_json::json!(binding.path));
    }

    if let (Some(x), Some(y)) = (
        material.public_param_f32(&["PixelGridTilingX"]),
        material.public_param_f32(&["PixelGridTilingY"]),
    ) {
        effects.insert(
            "pixel_grid_tiling".to_string(),
            serde_json::json!({ "x": x, "y": y }),
        );
    }

    serde_json::Value::Object(effects)
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

fn has_base_color_source(
    material: &SubMaterial,
    semantic_slots: &[SemanticTextureBinding],
) -> bool {
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
        (
            "source_path".to_string(),
            serde_json::json!(texture_ref.source_path),
        ),
        (
            "export_path".to_string(),
            serde_json::json!(texture_ref.export_path),
        ),
        (
            "export_kind".to_string(),
            serde_json::json!(texture_ref.export_kind),
        ),
    ]);
    if let Some(texture_identity) = &texture_ref.texture_identity {
        value.insert(
            "texture_identity".to_string(),
            serde_json::json!(texture_identity),
        );
    }
    if let Some(alpha_semantic) = &texture_ref.alpha_semantic {
        value.insert(
            "alpha_semantic".to_string(),
            serde_json::json!(alpha_semantic),
        );
    }
    if let Some(alpha_channel) = &texture_ref.alpha_channel {
        value.insert(
            "alpha_channel".to_string(),
            serde_json::json!(alpha_channel),
        );
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
    if let Some(derived_from_channel) = &texture_ref.derived_from_channel {
        value.insert(
            "derived_from_channel".to_string(),
            serde_json::json!(derived_from_channel),
        );
    }
    if let Some(value_channel) = &texture_ref.value_channel {
        value.insert(
            "value_channel".to_string(),
            serde_json::json!(value_channel),
        );
    }
    if let Some(value_transform) = &texture_ref.value_transform {
        value.insert(
            "value_transform".to_string(),
            serde_json::json!(value_transform),
        );
    }
    if let Some(packed_texture_format) = &texture_ref.packed_texture_format {
        value.insert(
            "packed_texture_format".to_string(),
            serde_json::json!(packed_texture_format),
        );
    }
    if let Some(packed_channel_semantics) = &texture_ref.packed_channel_semantics {
        value.insert(
            "packed_channel_semantics".to_string(),
            serde_json::json!(packed_channel_semantics),
        );
    }
    if let Some(constant_channel_values) = &texture_ref.constant_channel_values {
        value.insert(
            "constant_channel_values".to_string(),
            serde_json::json!(constant_channel_values),
        );
    }
    serde_json::Value::Object(value)
}

fn texture_derivation_status_json(status: &TextureDerivationStatus) -> serde_json::Value {
    let mut value = serde_json::Map::from_iter([
        ("role".to_string(), serde_json::json!(status.role)),
        (
            "source_path".to_string(),
            serde_json::json!(status.source_path),
        ),
        (
            "export_kind".to_string(),
            serde_json::json!(status.export_kind),
        ),
        ("status".to_string(), serde_json::json!(status.status)),
    ]);
    if let Some(texture_identity) = &status.texture_identity {
        value.insert(
            "texture_identity".to_string(),
            serde_json::json!(texture_identity),
        );
    }
    if let Some(texture_identity) = &status.derived_from_texture_identity {
        value.insert(
            "derived_from_texture_identity".to_string(),
            serde_json::json!(texture_identity),
        );
    }
    if let Some(derived_from_semantic) = &status.derived_from_semantic {
        value.insert(
            "derived_from_semantic".to_string(),
            serde_json::json!(derived_from_semantic),
        );
    }
    if let Some(derived_from_channel) = &status.derived_from_channel {
        value.insert(
            "derived_from_channel".to_string(),
            serde_json::json!(derived_from_channel),
        );
    }
    if let Some(value_channel) = &status.value_channel {
        value.insert(
            "value_channel".to_string(),
            serde_json::json!(value_channel),
        );
    }
    if let Some(value_transform) = &status.value_transform {
        value.insert(
            "value_transform".to_string(),
            serde_json::json!(value_transform),
        );
    }
    if let Some(packed_texture_format) = &status.packed_texture_format {
        value.insert(
            "packed_texture_format".to_string(),
            serde_json::json!(packed_texture_format),
        );
    }
    if let Some(packed_channel_semantics) = &status.packed_channel_semantics {
        value.insert(
            "packed_channel_semantics".to_string(),
            serde_json::json!(packed_channel_semantics),
        );
    }
    if let Some(constant_channel_values) = &status.constant_channel_values {
        value.insert(
            "constant_channel_values".to_string(),
            serde_json::json!(constant_channel_values),
        );
    }
    if let Some(reason) = &status.reason {
        value.insert("reason".to_string(), serde_json::json!(reason));
    }
    if let Some(export_path) = &status.export_path {
        value.insert("export_path".to_string(), serde_json::json!(export_path));
    }
    if let Some(requested_mip) = status.requested_mip {
        value.insert(
            "requested_mip".to_string(),
            serde_json::json!(requested_mip),
        );
    }
    if let Some(selected_mip) = status.selected_mip {
        value.insert("selected_mip".to_string(), serde_json::json!(selected_mip));
    }
    if let Some(mip_selection) = &status.mip_selection {
        value.insert(
            "mip_selection".to_string(),
            serde_json::json!(mip_selection),
        );
    }
    if let Some(alpha_mip_format) = &status.alpha_mip_format {
        value.insert(
            "alpha_mip_format".to_string(),
            serde_json::json!(alpha_mip_format),
        );
    }
    if let Some(alpha_mip_layout) = &status.alpha_mip_layout {
        value.insert(
            "alpha_mip_layout".to_string(),
            serde_json::json!(alpha_mip_layout),
        );
    }
    if let Some(width) = status.width {
        value.insert("width".to_string(), serde_json::json!(width));
    }
    if let Some(height) = status.height {
        value.insert("height".to_string(), serde_json::json!(height));
    }
    if let Some(alpha_mip_count) = status.alpha_mip_count {
        value.insert(
            "alpha_mip_count".to_string(),
            serde_json::json!(alpha_mip_count),
        );
    }
    if let Some(smoothness_min) = status.smoothness_min {
        value.insert(
            "smoothness_min".to_string(),
            serde_json::json!(smoothness_min),
        );
    }
    if let Some(smoothness_max) = status.smoothness_max {
        value.insert(
            "smoothness_max".to_string(),
            serde_json::json!(smoothness_max),
        );
    }
    if let Some(smoothness_mean) = status.smoothness_mean {
        value.insert(
            "smoothness_mean".to_string(),
            serde_json::json!(smoothness_mean),
        );
    }
    if let Some(roughness_min) = status.roughness_min {
        value.insert(
            "roughness_min".to_string(),
            serde_json::json!(roughness_min),
        );
    }
    if let Some(roughness_max) = status.roughness_max {
        value.insert(
            "roughness_max".to_string(),
            serde_json::json!(roughness_max),
        );
    }
    if let Some(roughness_mean) = status.roughness_mean {
        value.insert(
            "roughness_mean".to_string(),
            serde_json::json!(roughness_mean),
        );
    }
    serde_json::Value::Object(value)
}

fn ddna_texture_identity(path: &str) -> Option<&'static str> {
    if crate::mtl::texture_path_has_file_stem_token(path, &["ddna"]) {
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

fn ddna_alpha_channel(path: &str, role: TextureSemanticRole) -> Option<&'static str> {
    if ddna_alpha_semantic(path, role).is_some() {
        Some("a")
    } else {
        None
    }
}

fn exported_ddna_smoothness_sources(derivations: &[TextureDerivationStatus]) -> HashSet<String> {
    derivations
        .iter()
        .filter(|derivation| {
            derivation.export_kind == "derived_ddna_alpha"
                && derivation.status == "exported"
                && derivation.derived_from_texture_identity.as_deref() == Some("ddna_normal")
                && derivation.derived_from_semantic.as_deref() == Some("smoothness")
                && derivation.derived_from_channel.as_deref() == Some("a")
        })
        .map(|derivation| derivation.source_path.to_ascii_lowercase())
        .collect()
}

fn ddna_alpha_semantic_for_exported_source(
    p4k: &MappedP4k,
    path: &str,
    role: TextureSemanticRole,
    exported_sources: &HashSet<String>,
) -> Option<&'static str> {
    let semantic = ddna_alpha_semantic(path, role)?;
    let source_path = normalize_source_path(p4k, path).to_ascii_lowercase();
    exported_sources.contains(&source_path).then_some(semantic)
}

fn ddna_alpha_channel_for_exported_source(
    p4k: &MappedP4k,
    path: &str,
    role: TextureSemanticRole,
    exported_sources: &HashSet<String>,
) -> Option<&'static str> {
    if ddna_alpha_semantic_for_exported_source(p4k, path, role, exported_sources).is_some() {
        Some("a")
    } else {
        None
    }
}

fn normal_gloss_ddna_source_paths(material: &SubMaterial) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for binding in material.semantic_texture_slots() {
        if !matches!(binding.role, TextureSemanticRole::NormalGloss) {
            continue;
        }
        if ddna_texture_identity(&binding.path).is_none() {
            continue;
        }
        if seen.insert(normalize_requested_source_path(&binding.path).to_ascii_lowercase()) {
            paths.push(binding.path);
        }
    }
    if let Some(path) = material.normal_tex.as_deref()
        && ddna_texture_identity(path).is_some()
        && seen.insert(normalize_requested_source_path(path).to_ascii_lowercase())
    {
        paths.push(path.to_string());
    }
    paths
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
        value.insert(
            "children".to_string(),
            authored_blocks_json(&texmod.children),
        );
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

/// List every `(raw texture path, flavor)` pair that `export_texture_asset`
/// would decode for `materials`, covering each submaterial's non-virtual
/// semantic slots plus direct diffuse (Generic) and normal-gloss (Normal)
/// textures. Pure and P4K-free: paths are the verbatim slot paths, matching the
/// keys `export_texture_asset` passes to `cached_load_keyed`. May contain
/// duplicates; callers dedupe. Layer/decal textures are intentionally omitted —
/// because the decode cache is flavor-aware, a missing path only forfeits
/// pre-decode speedup, never correctness (it decodes serially as before).
fn texture_load_keys_from_materials(materials: &MtlFile) -> Vec<(String, TextureFlavor)> {
    let mut keys = Vec::new();
    for material in &materials.materials {
        for binding in material.semantic_texture_slots() {
            if binding.is_virtual {
                continue;
            }
            keys.push((binding.path.clone(), slot_texture_flavor(binding.role)));
        }
        if let Some(path) = material.diffuse_tex.as_deref() {
            keys.push((path.to_string(), TextureFlavor::Generic));
        }
        if let Some(path) = material.normal_tex.as_deref() {
            keys.push((path.to_string(), TextureFlavor::Normal));
        }
    }
    keys
}

/// Identity of a UI binding's rendered image. Two bindings with equal
/// `UiRenderKey` produce byte-identical PNGs within one export, because the
/// render depends only on these `UiBindingView` fields — every other input
/// (localization, ship data, manufacturer, root entity) is constant per export.
/// `f32` aspect is stored as raw bits so the key is `Hash + Eq`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UiRenderKey {
    canvas_guid: Option<String>,
    content_canvas_guid: Option<String>,
    binding_kind: String,
    helper_name: Option<String>,
    dashboard_view_index: Option<u32>,
    dashboard_screen_slot: Option<u32>,
    screen_name_loc_key: Option<String>,
    /// Per-floor transit location: two lift-call consoles on different floors
    /// render different headings, so the loc key is part of render identity.
    transit_location_loc_key: Option<String>,
    owner_source_file: Option<String>,
    screen_aspect_bits: Option<u32>,
}

/// Build the render-identity key for `binding` (see [`UiRenderKey`]).
fn ui_render_key(binding: &UiBinding) -> UiRenderKey {
    UiRenderKey {
        canvas_guid: binding.canvas_guid.clone(),
        content_canvas_guid: binding.content_canvas_guid.clone(),
        binding_kind: binding.binding_kind.clone(),
        helper_name: binding.helper_name.clone(),
        dashboard_view_index: binding.dashboard_view_index,
        dashboard_screen_slot: binding.dashboard_screen_slot,
        screen_name_loc_key: binding.screen_name_loc_key.clone(),
        transit_location_loc_key: binding.transit_location_loc_key.clone(),
        owner_source_file: binding.owner_source_file.clone(),
        screen_aspect_bits: binding.ui_screen_aspect_w_over_h.map(f32::to_bits),
    }
}

/// Render each DISTINCT UI binding once, in parallel, returning a map from
/// render key to the rendered PNG (or the render error). Bindings that share a
/// [`UiRenderKey`] render identically, so a representative is rendered once and
/// every instance reuses the result. This collapses the per-instance render
/// work (hundreds of screens, mostly duplicates) to one render per unique
/// screen. Output is unchanged: the per-binding record builder still computes
/// each binding's own export path and provenance.
fn prerender_ui_bindings(
    bindings: &[&UiBinding],
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_entity_name: &str,
    root_manufacturer_id: Option<&str>,
    loc_data: &crate::ui_pipeline::UiLocData,
    defaults_registry: &starbreaker_ui::DefaultValueRegistry,
    ship_data: &crate::ui_pipeline::UiShipData,
) -> HashMap<UiRenderKey, Result<Vec<u8>, String>> {
    // One representative binding per unique key (first wins; any is equivalent).
    let mut representatives: HashMap<UiRenderKey, &UiBinding> = HashMap::new();
    for binding in bindings {
        representatives.entry(ui_render_key(binding)).or_insert(binding);
    }
    representatives
        .par_iter()
        .map(|(key, binding)| {
            let result = crate::ui_pipeline::render_ui_binding_png(
                binding,
                db,
                p4k,
                texture_mip,
                root_manufacturer_id,
                loc_data,
                ship_data,
                Some(defaults_registry),
                Some(root_entity_name),
            );
            (key.clone(), result)
        })
        .collect()
}

/// Decode, in parallel, every source texture the decomposed sidecar writer will
/// need, returning a pre-filled `png_cache` and the source-material `mtl_cache`
/// built while resolving slot paths (so the writer can reuse it instead of
/// re-parsing the same `.mtl` files). `assets` carries one
/// `(materials, material_path, geometry_path)` per root/child/interior asset.
///
/// For each asset we resolve the same canonical source `.mtl` the serial writer
/// uses (so the slot paths match exactly), collect `(path, flavor)` keys, then
/// decode the unique set across the rayon pool. Keys are written in the exact
/// `cached_load_keyed` format (`""` for Generic, `"@n"` for Normal), so the
/// serial `export_texture_asset` calls hit the cache and skip decoding. Because
/// the cache is flavor-aware and the values are identical to what the serial
/// path would compute, this never changes output; an unenumerated path simply
/// decodes serially as before.
pub(crate) fn prewarm_decomposed_textures(
    p4k: &MappedP4k,
    assets: &[(MtlFile, String, String)],
    texture_mip: u32,
) -> (PngCache, HashMap<String, Option<MtlFile>>) {
    // Resolve source materials + collect keys serially (cheap .mtl parses,
    // deduped via a local mtl cache). Texture DECODE is the expensive part and
    // is parallelized below.
    let mut mtl_cache: HashMap<String, Option<MtlFile>> = HashMap::new();
    let mut requests: HashSet<(String, TextureFlavor)> = HashSet::new();
    for (materials, material_path, geometry_path) in assets {
        let source_material_path =
            canonical_material_source_path(p4k, materials, material_path, geometry_path, &mut mtl_cache);
        let (sidecar_materials, _indices) = canonical_sidecar_materials_from_source(
            p4k,
            &source_material_path,
            materials,
            &[],
            &mut mtl_cache,
        );
        for key in texture_load_keys_from_materials(&sidecar_materials) {
            requests.insert(key);
        }
    }
    let jobs: Vec<(String, TextureFlavor)> = requests.into_iter().collect();
    let png_cache: PngCache = jobs
        .par_iter()
        .map(|(path, flavor)| match flavor {
            TextureFlavor::Generic => (
                crate::pipeline::png_cache_key(path, texture_mip, ""),
                crate::pipeline::load_diffuse_texture(p4k, path, texture_mip),
            ),
            TextureFlavor::Normal => (
                crate::pipeline::png_cache_key(path, texture_mip, "@n"),
                crate::pipeline::load_normal_texture(p4k, path, texture_mip),
            ),
            TextureFlavor::Roughness => (
                crate::pipeline::png_cache_key(path, texture_mip, "@r"),
                crate::pipeline::load_roughness_texture(p4k, path, texture_mip),
            ),
        })
        .collect();
    // Return the source-material memo too: the serial writer seeds its own
    // `mtl_cache` from it so each `.mtl` is parsed once per export, not again
    // during sidecar writing.
    (png_cache, mtl_cache)
}

/// Parallel pre-decode of the DDNA→roughness sources the decomposed writer will
/// process, returning a cache keyed by RAW `binding.path` (via the same
/// `normal_gloss_ddna_source_paths` enumerator the writer uses, so keys match
/// `export_ddna_roughness_asset_with_status`'s `source_path` exactly). Holds ONLY
/// the pure decode `Result`; the serial writer still performs all
/// `files`/`texture_cache`/`ddna_status_cache` inserts, so output is unchanged and
/// a cache miss simply decodes serially as before.
///
/// NOTE: layer submaterials resolved via `resolve_layer_submaterial` (the
/// `.mtl`-layer branch in `extract_material_entry`) are NOT enumerated here and
/// still decode serially on their first touch — byte-safe, partial speedup.
pub(crate) fn prewarm_decomposed_roughness(
    p4k: &MappedP4k,
    assets: &[(MtlFile, String, String)],
    texture_mip: u32,
) -> RoughnessCache {
    use rayon::prelude::*;
    let mut sources: HashSet<String> = HashSet::new();
    for (materials, _material_path, _geometry_path) in assets {
        for material in &materials.materials {
            for src in normal_gloss_ddna_source_paths(material) {
                sources.insert(src);
            }
        }
    }
    let jobs: Vec<String> = sources.into_iter().collect();
    jobs.par_iter()
        .map(|src| {
            (
                src.clone(),
                crate::pipeline::load_roughness_texture_result(p4k, src, texture_mip),
            )
        })
        .collect()
}

fn texture_export_kind(flavor: TextureFlavor) -> &'static str {
    match flavor {
        TextureFlavor::Generic => "source",
        TextureFlavor::Normal => "source",
        TextureFlavor::Roughness => "derived_ddna_alpha",
    }
}

fn ddna_roughness_texture_ref(
    p4k: &MappedP4k,
    source_path: &str,
    export_path: String,
) -> TextureExportRef {
    TextureExportRef {
        role: "roughness".to_string(),
        source_path: normalize_source_path(p4k, source_path),
        export_path,
        export_kind: texture_export_kind(TextureFlavor::Roughness).to_string(),
        texture_identity: None,
        alpha_semantic: None,
        alpha_channel: None,
        derived_from_texture_identity: ddna_texture_identity(source_path).map(str::to_string),
        derived_from_semantic: ddna_alpha_semantic(source_path, TextureSemanticRole::NormalGloss)
            .map(str::to_string),
        derived_from_channel: ddna_alpha_channel(source_path, TextureSemanticRole::NormalGloss)
            .map(str::to_string),
        value_channel: Some("r".to_string()),
        value_transform: Some("sqrt_one_minus".to_string()),
        packed_texture_format: Some("roughness_grayscale".to_string()),
        packed_channel_semantics: Some(roughness_grayscale_channel_semantics()),
        constant_channel_values: Some(roughness_grayscale_constant_channels()),
    }
}

fn roughness_grayscale_channel_semantics() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("r".to_string(), "roughness".to_string()),
        ("g".to_string(), "roughness".to_string()),
        ("b".to_string(), "roughness".to_string()),
        ("a".to_string(), "unused".to_string()),
    ])
}

fn roughness_grayscale_constant_channels() -> BTreeMap<String, String> {
    BTreeMap::from([("a".to_string(), "1.0".to_string())])
}

fn export_ddna_roughness_asset_with_status(
    files: &mut OutputFiles,
    p4k: &MappedP4k,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    ddna_status_cache: &mut HashMap<String, (Option<TextureExportRef>, TextureDerivationStatus)>,
    roughness_cache: &RoughnessCache,
    source_path: &str,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> (Option<TextureExportRef>, TextureDerivationStatus) {
    let Some(_) = ddna_texture_identity(source_path) else {
        return (
            None,
            ddna_roughness_derivation_status(
                p4k,
                source_path,
                "missing",
                Some("not_ddna_normal"),
                None,
                Some(texture_mip),
                None,
                Some("unavailable"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
        );
    };

    // The derivation result — the exported ref AND the rich status metadata —
    // is a pure function of the source path (the mip is constant across one
    // export). Without this memo every sidecar referencing the same DDNA paid a
    // FULL mip-level decode + smoothness-statistics pass + PNG encode
    // (`load_roughness_texture_result` runs before the byte-level
    // `texture_cache` check because the status needs the decoded statistics),
    // which turned capital-ship exports from minutes into hours. Keyed by the
    // RAW source path so a cache hit replays exactly what an uncached repeat
    // call would have produced.
    if let Some(cached) = ddna_status_cache.get(source_path) {
        return cached.clone();
    }

    let normalized_source = normalize_source_path(p4k, source_path);
    let cache_key = texture_cache_key(&normalized_source, TextureFlavor::Roughness);
    let requested_path =
        texture_relative_path(p4k, source_path, TextureFlavor::Roughness, texture_mip);

    // Consult the prewarmed pure-decode cache (keyed by RAW source path) before
    // decoding. This is the ONLY roughness_cache use — the `ddna_status_cache`
    // memo above and all `files`/`texture_cache` inserts below stay serial, so a
    // cache hit replays exactly what a cold decode would produce.
    let result = match roughness_cache
        .get(source_path)
        .cloned()
        .unwrap_or_else(|| {
            crate::pipeline::load_roughness_texture_result(p4k, source_path, texture_mip)
        }) {
        Ok(loaded) => {
            let selected_mip = loaded.selected_mip;
            let requested_mip = loaded.requested_mip;
            let mip_selection = loaded.mip_selection;
            let alpha_mip_format = loaded.alpha_mip_format;
            let alpha_mip_layout = loaded.alpha_mip_layout;
            let width = loaded.width;
            let height = loaded.height;
            let alpha_mip_count = loaded.alpha_mip_count;
            let smoothness_min = loaded.smoothness_min;
            let smoothness_max = loaded.smoothness_max;
            let smoothness_mean = loaded.smoothness_mean;
            let roughness_min = loaded.roughness_min;
            let roughness_max = loaded.roughness_max;
            let roughness_mean = loaded.roughness_mean;
            let stored_path = if let Some(cached_path) = texture_cache.get(&cache_key) {
                cached_path.clone()
            } else if existing_asset_paths
                .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
            {
                texture_cache.insert(cache_key, requested_path.clone());
                requested_path
            } else {
                let stored_path = insert_binary_file(files, requested_path, loaded.grayscale_png);
                texture_cache.insert(cache_key, stored_path.clone());
                stored_path
            };
            (
                Some(ddna_roughness_texture_ref(
                    p4k,
                    source_path,
                    stored_path.clone(),
                )),
                ddna_roughness_derivation_status(
                    p4k,
                    source_path,
                    "exported",
                    None,
                    Some(stored_path),
                    Some(requested_mip),
                    Some(selected_mip),
                    Some(mip_selection),
                    Some(alpha_mip_format),
                    Some(alpha_mip_layout),
                    Some(width),
                    Some(height),
                    Some(alpha_mip_count),
                    Some(smoothness_min),
                    Some(smoothness_max),
                    Some(smoothness_mean),
                    Some(roughness_min),
                    Some(roughness_max),
                    Some(roughness_mean),
                ),
            )
        }
        Err(error) => (
            None,
            ddna_roughness_derivation_status(
                p4k,
                source_path,
                "missing",
                Some(error.as_str()),
                None,
                Some(texture_mip),
                None,
                Some("unavailable"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
        ),
    };
    ddna_status_cache.insert(source_path.to_string(), result.clone());
    result
}

fn ddna_roughness_derivation_status(
    p4k: &MappedP4k,
    source_path: &str,
    status: &str,
    reason: Option<&str>,
    export_path: Option<String>,
    requested_mip: Option<u32>,
    selected_mip: Option<u32>,
    mip_selection: Option<&str>,
    alpha_mip_format: Option<&str>,
    alpha_mip_layout: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
    alpha_mip_count: Option<u32>,
    smoothness_min: Option<u8>,
    smoothness_max: Option<u8>,
    smoothness_mean: Option<u8>,
    roughness_min: Option<u8>,
    roughness_max: Option<u8>,
    roughness_mean: Option<u8>,
) -> TextureDerivationStatus {
    TextureDerivationStatus {
        role: "roughness".to_string(),
        source_path: normalize_source_path(p4k, source_path),
        export_kind: texture_export_kind(TextureFlavor::Roughness).to_string(),
        texture_identity: None,
        derived_from_texture_identity: ddna_texture_identity(source_path).map(str::to_string),
        derived_from_semantic: ddna_alpha_semantic(source_path, TextureSemanticRole::NormalGloss)
            .map(str::to_string),
        derived_from_channel: ddna_alpha_channel(source_path, TextureSemanticRole::NormalGloss)
            .map(str::to_string),
        value_channel: Some("r".to_string()),
        value_transform: Some("sqrt_one_minus".to_string()),
        packed_texture_format: Some("roughness_grayscale".to_string()),
        packed_channel_semantics: Some(roughness_grayscale_channel_semantics()),
        constant_channel_values: Some(roughness_grayscale_constant_channels()),
        status: status.to_string(),
        reason: reason.map(str::to_string),
        export_path,
        requested_mip,
        selected_mip,
        mip_selection: mip_selection.map(str::to_string),
        alpha_mip_format: alpha_mip_format.map(str::to_string),
        alpha_mip_layout: alpha_mip_layout.map(str::to_string),
        width,
        height,
        alpha_mip_count,
        smoothness_min,
        smoothness_max,
        smoothness_mean,
        roughness_min,
        roughness_max,
        roughness_mean,
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

/// Per-entity-type clip key for socpak interior animations. Different entity
/// types never collide (so their identically-named clips — e.g. every door's
/// `door_open` — no longer merge into one), while instances of the same
/// entity-type share a key (so they still merge into a single clip). The double
/// underscore separates the entity stem from the action unambiguously.
fn interior_clip_key(entity_stem: &str, clip_name: &str) -> String {
    format!("{entity_stem}__{clip_name}")
}

/// Human-readable panel label for a socpak interior animation: the title-cased
/// entity stem plus the title-cased clip action. Purely generic (split on `_`,
/// capitalise each token) — no asset-specific token stripping.
fn humanize_animation_label(entity_stem: &str, clip_name: &str) -> String {
    fn titlecase(value: &str) -> String {
        value
            .split('_')
            .filter(|token| !token.is_empty())
            .map(|token| {
                let mut chars = token.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    format!("{} — {}", titlecase(entity_stem), titlecase(clip_name))
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
        if !existing_variants
            .iter()
            .any(|variant| *variant == incoming_value)
        {
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

/// Parse an animation channel bone-key (e.g. `"0x6B65934C"` or a decimal
/// string) into its raw CRC32 hash.
fn parse_animation_channel_hash(key: &str) -> Option<u32> {
    let trimmed = key.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    trimmed.parse::<u32>().ok()
}

/// Walk every channel variant in a `bones` map, invoking `f(bone_key, channel_obj)`.
fn for_each_animation_channel(
    clips: &mut [serde_json::Value],
    mut f: impl FnMut(&str, &mut serde_json::Map<String, serde_json::Value>),
) {
    for clip in clips {
        let Some(bones) = clip.get_mut("bones").and_then(|b| b.as_object_mut()) else {
            continue;
        };
        for (bone_key, channel_value) in bones.iter_mut() {
            if let Some(obj) = channel_value.as_object_mut() {
                f(bone_key, obj);
            } else if let Some(variants) = channel_value.as_array_mut() {
                for variant in variants {
                    if let Some(obj) = variant.as_object_mut() {
                        f(bone_key, obj);
                    }
                }
            }
        }
    }
}

/// Collect the bone-key hashes of every channel that still lacks a
/// `source_node_name` (i.e. the rig it was stamped with could not name it).
fn collect_unresolved_channel_hashes(
    clips: &[serde_json::Value],
) -> std::collections::HashSet<u32> {
    let mut unresolved = std::collections::HashSet::new();
    for clip in clips {
        let Some(bones) = clip.get("bones").and_then(|b| b.as_object()) else {
            continue;
        };
        for (bone_key, channel_value) in bones {
            let variants: Vec<&serde_json::Value> = match channel_value {
                serde_json::Value::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            for variant in variants {
                let resolved = variant
                    .get("source_node_name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty());
                if !resolved {
                    if let Some(hash) = parse_animation_channel_hash(bone_key) {
                        unresolved.insert(hash);
                    }
                }
            }
        }
    }
    unresolved
}

/// Owner of a channel hash among the entity's own rigs.
///
/// Because the hash is `crc32(node_name)`, a hash present in several rigs means
/// those rigs share a node with the *same name* — so the name is unambiguous
/// even when the owning CGA is not.
enum ChannelOwner {
    /// Exactly one rig owns the node: both `source_node_name` and
    /// `source_skeleton_path` are safe to rewrite.
    Unique { node_name: String, source_path: String },
    /// Several rigs contain a node with the same name (e.g. a generic `shutter_*`
    /// node shared by multiple door CGAs). The name is certain; the owning CGA
    /// is not, so only `source_node_name` is stamped.
    SharedName { node_name: String },
    /// A genuine hash collision between two *different* names — never guessed.
    Conflicting,
}

impl ChannelOwner {
    fn merge_candidate(&mut self, name: String, source_path: String) {
        match self {
            ChannelOwner::Unique { node_name, source_path: existing_path } => {
                if *node_name != name {
                    *self = ChannelOwner::Conflicting;
                } else if *existing_path != source_path {
                    // Same node name, different rig → name is certain, path is not.
                    *self = ChannelOwner::SharedName { node_name: name };
                }
                // else: exact same (name, rig) → no change.
            }
            ChannelOwner::SharedName { node_name } => {
                if *node_name != name {
                    *self = ChannelOwner::Conflicting;
                }
            }
            ChannelOwner::Conflicting => {}
        }
    }
}

/// Re-stamp animation channels that the root rig could not name with the
/// correct owning rig, chosen from the entity's *own* interior/child geometry.
///
/// Star Citizen ships keep their per-component animation tracks (doors, ladders,
/// beds, …) in a shared `.dba`. The root-skeleton extraction sweeps every block
/// (`include_unmatched`), so component blocks whose nodes are not in the root
/// NMC end up stamped with the exterior hull CGA and no `source_node_name`. This
/// pass resolves each such channel's CRC32 hash against the NMC/rig node names of
/// the interior + child CGAs we are already exporting, and rewrites
/// `source_node_name` + `source_skeleton_path` to the rig that actually owns the
/// node. It only fills genuinely-unresolved channels and never overrides a hash
/// owned by more than one rig, so it cannot mis-bind an already-correct track.
///
/// Returns the number of channels re-stamped.
fn restamp_unresolved_animation_channels<'a>(
    clips: &mut [serde_json::Value],
    p4k: &starbreaker_p4k::MappedP4k,
    rig_source_paths: impl IntoIterator<Item = &'a str>,
) -> usize {
    let unresolved = collect_unresolved_channel_hashes(clips);
    if unresolved.is_empty() {
        return 0;
    }

    // Build hash -> owner from the entity's own rigs, restricted to the hashes
    // that are actually unresolved so we never parse more than necessary.
    let mut owner_by_hash: std::collections::HashMap<u32, ChannelOwner> =
        std::collections::HashMap::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    for rig_path in rig_source_paths {
        if rig_path.is_empty() || !seen_paths.insert(rig_path.to_string()) {
            continue;
        }
        let p4k_path = crate::pipeline::datacore_path_to_p4k(rig_path);
        let Some(names) = p4k
            .entry_case_insensitive(&p4k_path)
            .and_then(|e| p4k.read(e).ok())
            .and_then(|data| crate::skeleton::parse_rig_node_names(&data))
        else {
            continue;
        };
        let normalized_source = normalize_source_path(p4k, rig_path);
        for name in names {
            let hash = crate::animation::bone_name_hash(&name);
            if !unresolved.contains(&hash) {
                continue;
            }
            match owner_by_hash.get_mut(&hash) {
                None => {
                    owner_by_hash.insert(
                        hash,
                        ChannelOwner::Unique {
                            node_name: name,
                            source_path: normalized_source.clone(),
                        },
                    );
                }
                Some(owner) => owner.merge_candidate(name, normalized_source.clone()),
            }
        }
    }

    if owner_by_hash.is_empty() {
        return 0;
    }

    apply_channel_owner_restamp(clips, &owner_by_hash)
}

/// Apply a hash→owner map to every still-unresolved channel. Pure (no I/O) so
/// it can be unit-tested. Only fills channels that lack a `source_node_name`
/// and whose hash has a unique owner.
fn apply_channel_owner_restamp(
    clips: &mut [serde_json::Value],
    owner_by_hash: &std::collections::HashMap<u32, ChannelOwner>,
) -> usize {
    let mut restamped = 0usize;
    for_each_animation_channel(clips, |bone_key, channel_obj| {
        let already_named = channel_obj
            .get("source_node_name")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if already_named {
            return;
        }
        let Some(hash) = parse_animation_channel_hash(bone_key) else {
            return;
        };
        match owner_by_hash.get(&hash) {
            Some(ChannelOwner::Unique { node_name, source_path }) => {
                channel_obj.insert(
                    "source_node_name".to_string(),
                    serde_json::Value::String(node_name.clone()),
                );
                channel_obj.insert(
                    "source_skeleton_path".to_string(),
                    serde_json::Value::String(source_path.clone()),
                );
                restamped += 1;
            }
            Some(ChannelOwner::SharedName { node_name }) => {
                // Name is certain across rigs; the owning CGA is not, so leave
                // the existing source_skeleton_path in place.
                channel_obj.insert(
                    "source_node_name".to_string(),
                    serde_json::Value::String(node_name.clone()),
                );
                restamped += 1;
            }
            Some(ChannelOwner::Conflicting) | None => {}
        }
    });
    restamped
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
    fn collect_unresolved_channel_hashes_collects_only_unnamed() {
        let clips = vec![serde_json::json!({
            "name": "door_open",
            "bones": {
                "0x00000001": { "has_rotation": true, "source_node_name": "shutter_15" },
                "0x00000002": { "has_rotation": true, "source_skeleton_path": "Exterior/Hull.cga" },
            }
        })];
        let unresolved = collect_unresolved_channel_hashes(&clips);
        assert!(!unresolved.contains(&1), "named channel must not be unresolved");
        assert!(unresolved.contains(&2), "channel without source_node_name is unresolved");
    }

    #[test]
    fn apply_channel_owner_restamp_handles_unique_shared_and_conflicting() {
        let mut clips = vec![serde_json::json!({
            "name": "door_open",
            "bones": {
                "0x00000002": { "has_rotation": true, "source_skeleton_path": "Exterior/Hull.cga" },
                "0x00000003": { "has_rotation": true, "source_skeleton_path": "Exterior/Hull.cga" },
                "0x00000005": { "has_rotation": true, "source_skeleton_path": "Exterior/Hull.cga" },
                "0x00000004": { "has_rotation": true, "source_node_name": "already" },
            }
        })];
        let mut owner = std::collections::HashMap::new();
        // Unique: one rig owns it → both fields rewritten.
        owner.insert(
            2u32,
            ChannelOwner::Unique {
                node_name: "lever_01".to_string(),
                source_path: "Interior/Bridge/Avionics_door.cga".to_string(),
            },
        );
        // SharedName: same node name in several door CGAs → name only, path kept.
        owner.insert(3u32, ChannelOwner::SharedName { node_name: "shutter_15".to_string() });
        // Conflicting: a genuine cross-name hash collision → never guessed.
        owner.insert(5u32, ChannelOwner::Conflicting);

        let restamped = apply_channel_owner_restamp(&mut clips, &owner);
        assert_eq!(restamped, 2);

        let bones = clips[0]["bones"].as_object().unwrap();
        // Unique owner: both fields rewritten to the owning interior CGA.
        assert_eq!(bones["0x00000002"]["source_node_name"], "lever_01");
        assert_eq!(
            bones["0x00000002"]["source_skeleton_path"],
            "Interior/Bridge/Avionics_door.cga"
        );
        // Shared name: name set, original skeleton path left untouched.
        assert_eq!(bones["0x00000003"]["source_node_name"], "shutter_15");
        assert_eq!(bones["0x00000003"]["source_skeleton_path"], "Exterior/Hull.cga");
        // Conflicting: left unresolved.
        assert!(bones["0x00000005"].get("source_node_name").is_none());
        // Already-named channel: untouched.
        assert_eq!(bones["0x00000004"]["source_node_name"], "already");
    }

    #[test]
    fn channel_owner_merge_demotes_to_shared_then_conflicting() {
        // Same name from a second rig → SharedName.
        let mut owner = ChannelOwner::Unique {
            node_name: "shutter_15".to_string(),
            source_path: "a.cga".to_string(),
        };
        owner.merge_candidate("shutter_15".to_string(), "b.cga".to_string());
        assert!(matches!(owner, ChannelOwner::SharedName { .. }));
        // A different name colliding on the same hash → Conflicting.
        owner.merge_candidate("totally_other".to_string(), "c.cga".to_string());
        assert!(matches!(owner, ChannelOwner::Conflicting));
    }

    #[test]
    fn merge_animation_channel_values_promotes_duplicates_to_variant_array() {
        let mut existing = serde_json::json!({"position": [[1.0, 2.0, 3.0]]});
        let incoming = serde_json::json!({"position": [[4.0, 5.0, 6.0]]});

        merge_animation_channel_values(
            &mut existing,
            incoming.clone(),
            "landing_gear_retract",
            "0x2522C378",
        );

        let arr = existing
            .as_array()
            .expect("channel entry should become a variant array");
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

        merge_animation_channel_values(
            &mut existing,
            incoming,
            "landing_gear_retract",
            "0x2522C378",
        );

        let arr = existing
            .as_array()
            .expect("channel entry should remain an array");
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
        let index_count = submeshes
            .iter()
            .map(|submesh| submesh.first_index + submesh.num_indices)
            .max()
            .unwrap_or(0) as usize;
        Mesh {
            positions: Vec::new(),
            indices: (0..index_count as u32).collect(),
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
            replace_extension(
                &normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"),
                ".png"
            ),
            "Data/Objects/Ships/Test/hull_diff.png"
        );
        assert_eq!(
            replace_extension(
                &normalize_requested_source_path("Objects/Ships/Test/hull_ddna.dds"),
                ".png"
            ),
            "Data/Objects/Ships/Test/hull_ddna.png"
        );
    }

    #[test]
    fn roughness_texture_relative_path_marks_ddna_alpha_derivation() {
        assert_eq!(
            texture_relative_path_from_normalized(
                "Data/Objects/FPS_Weapons/Test/brfl_fps_behr_p6lr_ddna.dds",
                TextureFlavor::Roughness,
                2,
            ),
            "Data/Objects/FPS_Weapons/Test/brfl_fps_behr_p6lr_ddna_roughness_TEX2.png"
        );
        assert_eq!(
            texture_export_kind(TextureFlavor::Roughness),
            "derived_ddna_alpha"
        );
    }

    #[test]
    fn ddna_texture_identity_uses_filename_tokens() {
        assert_eq!(
            ddna_texture_identity("Data/Objects/Test/panel-ddna.tif"),
            Some("ddna_normal")
        );
        assert_eq!(
            ddna_texture_identity("Data/Objects/Test_ddna_cache/panel_diff.tif"),
            None
        );
    }

    #[test]
    fn normal_gloss_ddna_sources_include_semantic_slots_outside_texslot2() {
        let mut material = sample_submaterial();
        material.shader = "HardSurface".into();
        material.normal_tex = None;
        material.texture_slots = vec![mtl::TextureSlotBinding {
            slot: "TexSlot1".into(),
            path: "Objects/FPS_Weapons/Test/brfl_fps_behr_p6lr_ddna.tif".into(),
            is_virtual: false,
        }];

        assert_eq!(
            normal_gloss_ddna_source_paths(&material),
            vec!["Objects/FPS_Weapons/Test/brfl_fps_behr_p6lr_ddna.tif".to_string()]
        );
    }

    #[test]
    fn normal_gloss_ddna_sources_deduplicate_normalized_paths() {
        let mut material = sample_submaterial();
        material.shader = "HardSurface".into();
        material.normal_tex =
            Some("Data\\Objects\\FPS_Weapons\\Test\\brfl_fps_behr_p6lr_ddna.tif".into());
        material.texture_slots = vec![mtl::TextureSlotBinding {
            slot: "TexSlot1".into(),
            path: "Objects/FPS_Weapons/Test/brfl_fps_behr_p6lr_ddna.tif".into(),
            is_virtual: false,
        }];

        assert_eq!(
            normal_gloss_ddna_source_paths(&material),
            vec!["Objects/FPS_Weapons/Test/brfl_fps_behr_p6lr_ddna.tif".to_string()]
        );
    }

    #[test]
    fn texture_cache_key_normalizes_case_by_flavor() {
        assert_eq!(
            texture_cache_key("Data/Objects/Test/PANEL_DDNA.dds", TextureFlavor::Roughness),
            (
                "data/objects/test/panel_ddna.dds".to_string(),
                TextureFlavor::Roughness
            )
        );
        assert_ne!(
            texture_cache_key("Data/Objects/Test/PANEL_DDNA.dds", TextureFlavor::Normal),
            texture_cache_key("Data/Objects/Test/PANEL_DDNA.dds", TextureFlavor::Roughness)
        );
    }

    #[test]
    #[ignore = "needs SC_DATA_P4K"]
    fn prewarmed_roughness_matches_cold_and_emits_png() {
        let Ok(p) = std::env::var("SC_DATA_P4K") else {
            return;
        };
        let p4k = MappedP4k::open(&p).unwrap();
        // Pick a DDNA source that actually decodes so the PNG-emission assertion
        // is meaningful (a source that errors emits no roughness PNG on either path).
        let src = p4k
            .entries()
            .iter()
            .map(|e| e.name.clone())
            .find(|n| {
                n.to_ascii_lowercase().ends_with("_ddna.dds")
                    && crate::pipeline::load_roughness_texture_result(&p4k, n, 0).is_ok()
            })
            .expect("a decodable ddna source");

        // Cold path: empty prewarm cache, writer decodes serially.
        let mut f0 = OutputFiles::new();
        let mut tc0 = HashMap::new();
        let mut sc0 = HashMap::new();
        let empty = RoughnessCache::new();
        let cold = export_ddna_roughness_asset_with_status(
            &mut f0, &p4k, &mut tc0, &mut sc0, &empty, &src, 0, None,
        );

        // Prewarmed path: cache pre-seeded with the pure decode result.
        let warm_cache: RoughnessCache = std::iter::once((
            src.clone(),
            crate::pipeline::load_roughness_texture_result(&p4k, &src, 0),
        ))
        .collect();
        let mut f1 = OutputFiles::new();
        let mut tc1 = HashMap::new();
        let mut sc1 = HashMap::new();
        let warm = export_ddna_roughness_asset_with_status(
            &mut f1, &p4k, &mut tc1, &mut sc1, &warm_cache, &src, 0, None,
        );

        assert_eq!(cold.0, warm.0, "exported ref must match the cold path");
        assert_eq!(cold.1, warm.1, "derivation status must match the cold path");
        let rel = texture_relative_path(&p4k, &src, TextureFlavor::Roughness, 0);
        assert!(
            f1.contains_key(&rel),
            "roughness PNG must be inserted on the prewarmed path"
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
        assert_eq!(
            normalize_package_subdir("vehicle/test"),
            Some("vehicle/test".to_string())
        );
        assert_eq!(
            normalize_package_subdir("../ship"),
            Some("ship".to_string())
        );
        assert_eq!(normalize_package_subdir(""), None);
    }

    #[test]
    fn ui_render_key_ignores_non_view_fields_but_tracks_view_fields() {
        let base = UiBinding {
            binding_kind: "mfd".to_string(),
            canvas_guid: Some("guid-a".to_string()),
            helper_name: Some("screen_a".to_string()),
            ..Default::default()
        };
        // Same view fields, different non-view field (canvas_record_name, used
        // only for provenance) -> equal key.
        let mut same = base.clone();
        same.canvas_record_name = Some("SomeOtherRecordName".to_string());
        assert_eq!(ui_render_key(&base), ui_render_key(&same));
        // Different view field (helper_name) -> different key.
        let mut different = base.clone();
        different.helper_name = Some("screen_b".to_string());
        assert_ne!(ui_render_key(&base), ui_render_key(&different));
    }

    #[test]
    fn texture_load_keys_collects_direct_diffuse_and_normal() {
        let materials = MtlFile {
            materials: vec![sample_submaterial()],
            source_path: None,
            paint_override: None,
            material_set: Default::default(),
        };
        let keys = texture_load_keys_from_materials(&materials);
        assert!(keys.contains(&(
            "Objects/Ships/Test/hull_diff.dds".to_string(),
            TextureFlavor::Generic
        )));
        assert!(keys.contains(&(
            "Objects/Ships/Test/hull_ddna.dds".to_string(),
            TextureFlavor::Normal
        )));
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
                alpha_channel: None,
                derived_from_texture_identity: None,
                derived_from_semantic: None,
                derived_from_channel: None,
                value_channel: None,
                value_transform: None,
                packed_texture_format: None,
                packed_channel_semantics: None,
                constant_channel_values: None,
            }],
            layer_exports: vec![LayerTextureExport {
                source_material_path: "Data/libs/materials/metal/test_layer.mtl".into(),
                diffuse_export_path: Some("Data/libs/materials/metal/test_layer.png".into()),
                normal_export_path: Some("Data/libs/materials/metal/test_layer.png".into()),
                roughness_export_path: Some(
                    "Data/libs/materials/metal/test_layer_roughness_TEX0.png".into(),
                ),
                roughness_texture: Some(TextureExportRef {
                    role: "roughness".into(),
                    source_path: "Data/libs/materials/metal/test_layer_ddna.dds".into(),
                    export_path: "Data/libs/materials/metal/test_layer_roughness_TEX0.png".into(),
                    export_kind: "derived_ddna_alpha".into(),
                    texture_identity: None,
                    alpha_semantic: None,
                    alpha_channel: None,
                    derived_from_texture_identity: Some("ddna_normal".into()),
                    derived_from_semantic: Some("smoothness".into()),
                    derived_from_channel: Some("a".into()),
                    value_channel: Some("r".into()),
                    value_transform: Some("sqrt_one_minus".into()),
                    packed_texture_format: Some("roughness_grayscale".into()),
                    packed_channel_semantics: Some(roughness_grayscale_channel_semantics()),
                    constant_channel_values: Some(roughness_grayscale_constant_channels()),
                }),
                ddna_derivations: vec![TextureDerivationStatus {
                    role: "roughness".into(),
                    source_path: "Data/libs/materials/metal/test_layer_ddna.dds".into(),
                    export_kind: "derived_ddna_alpha".into(),
                    texture_identity: None,
                    derived_from_texture_identity: Some("ddna_normal".into()),
                    derived_from_semantic: Some("smoothness".into()),
                    derived_from_channel: Some("a".into()),
                    value_channel: Some("r".into()),
                    value_transform: Some("sqrt_one_minus".into()),
                    packed_texture_format: Some("roughness_grayscale".into()),
                    packed_channel_semantics: Some(roughness_grayscale_channel_semantics()),
                    constant_channel_values: Some(roughness_grayscale_constant_channels()),
                    status: "exported".into(),
                    reason: None,
                    export_path: Some(
                        "Data/libs/materials/metal/test_layer_roughness_TEX0.png".into(),
                    ),
                    requested_mip: Some(0),
                    selected_mip: Some(0),
                    mip_selection: Some("requested".into()),
                    alpha_mip_format: Some("bc4_unorm".into()),
                    alpha_mip_layout: Some("numbered_sibling".into()),
                    width: Some(256),
                    height: Some(128),
                    alpha_mip_count: Some(8),
                    smoothness_min: Some(12),
                    smoothness_max: Some(240),
                    smoothness_mean: Some(128),
                    roughness_min: Some(125),
                    roughness_max: Some(250),
                    roughness_mean: Some(180),
                }],
                slot_exports: vec![serde_json::json!({
                    "slot": "TexSlot3",
                    "role": "normal_gloss",
                    "is_virtual": false,
                    "source_path": "Data/libs/materials/metal/test_layer_ddna.dds",
                    "export_path": "Data/libs/materials/metal/test_layer.png",
                    "export_kind": "source",
                    "texture_identity": "ddna_normal",
                    "alpha_semantic": "smoothness",
                    "alpha_channel": "a",
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
            derived_texture_exports: vec![TextureExportRef {
                role: "roughness".into(),
                source_path: "Data/libs/materials/metal/test_layer_ddna.dds".into(),
                export_path: "Data/libs/materials/metal/test_layer_roughness_TEX0.png".into(),
                export_kind: "derived_ddna_alpha".into(),
                texture_identity: None,
                alpha_semantic: None,
                alpha_channel: None,
                derived_from_texture_identity: Some("ddna_normal".into()),
                derived_from_semantic: Some("smoothness".into()),
                derived_from_channel: Some("a".into()),
                value_channel: Some("r".into()),
                value_transform: Some("sqrt_one_minus".into()),
                packed_texture_format: Some("roughness_grayscale".into()),
                packed_channel_semantics: Some(roughness_grayscale_channel_semantics()),
                constant_channel_values: Some(roughness_grayscale_constant_channels()),
            }],
            ddna_derivations: vec![TextureDerivationStatus {
                role: "roughness".into(),
                source_path: "Data/libs/materials/metal/test_layer_ddna.dds".into(),
                export_kind: "derived_ddna_alpha".into(),
                texture_identity: None,
                derived_from_texture_identity: Some("ddna_normal".into()),
                derived_from_semantic: Some("smoothness".into()),
                derived_from_channel: Some("a".into()),
                value_channel: Some("r".into()),
                value_transform: Some("sqrt_one_minus".into()),
                packed_texture_format: Some("roughness_grayscale".into()),
                packed_channel_semantics: Some(roughness_grayscale_channel_semantics()),
                constant_channel_values: Some(roughness_grayscale_constant_channels()),
                status: "missing".into(),
                reason: Some("missing_alpha_mips".into()),
                export_path: None,
                requested_mip: Some(0),
                selected_mip: None,
                mip_selection: Some("unavailable".into()),
                alpha_mip_format: None,
                alpha_mip_layout: None,
                width: None,
                height: None,
                alpha_mip_count: None,
                smoothness_min: None,
                smoothness_max: None,
                smoothness_mean: None,
                roughness_min: None,
                roughness_max: None,
                roughness_mean: None,
            }],
        }];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0],
        );

        assert_eq!(
            value["source_material_path"],
            serde_json::json!("Data/Objects/Ships/Test/hull.mtl")
        );
        assert!(value.get("geometry_path").is_none());
        assert_eq!(
            value["authored_material_set"]["attributes"][0]["name"],
            serde_json::json!("DefaultPalette")
        );
        assert_eq!(
            value["authored_material_set"]["public_params"][0]["name"],
            serde_json::json!("RootGlowScale")
        );
        assert_eq!(
            value["authored_material_set"]["child_blocks"][0]["tag"],
            serde_json::json!("VertexDeform")
        );
        assert_eq!(
            value["paint_override"]["subgeometry_tag"],
            serde_json::json!("BlackGold")
        );
        assert_eq!(
            value["submaterials"][0]["blender_material_name"],
            serde_json::json!("hull:hull_panel")
        );
        assert_eq!(
            value["submaterials"][0]["shader_family"],
            serde_json::json!("LayerBlend_V2")
        );
        assert_eq!(
            value["submaterials"][0]["authored_attributes"][0]["name"],
            serde_json::json!("MtlFlags")
        );
        assert_eq!(
            value["submaterials"][0]["authored_public_params"][0]["name"],
            serde_json::json!("WearBlendBase")
        );
        assert_eq!(
            value["submaterials"][0]["authored_child_blocks"][0]["tag"],
            serde_json::json!("VertexDeform")
        );
        assert_eq!(
            value["submaterials"][0]["texture_slots"][0]["authored_child_blocks"][0]["tag"],
            serde_json::json!("TexMod")
        );
        assert_eq!(
            value["submaterials"][0]["palette_routing"]["material_channel"]["name"],
            serde_json::json!("secondary")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["palette_channel"]["name"],
            serde_json::json!("primary")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["name"],
            serde_json::json!("Primary")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["submaterial_name"],
            serde_json::json!("paint")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["shader_family"],
            serde_json::json!("Layer")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["authored_attributes"]
                [0]["name"],
            serde_json::json!("MatTemplate")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["authored_public_params"]
                [0]["name"],
            serde_json::json!("WearGlossiness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["resolved_material"]["authored_child_blocks"]
                [0]["tag"],
            serde_json::json!("VertexDeform")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["authored_attributes"][0]["name"],
            serde_json::json!("CustomBlendMode")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["authored_child_blocks"][0]["tag"],
            serde_json::json!("CustomAnimation")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["role"],
            serde_json::json!("normal_gloss")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["texture_identity"],
            serde_json::json!("ddna_normal")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["alpha_semantic"],
            serde_json::json!("smoothness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["alpha_channel"],
            serde_json::json!("a")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["texture_slots"][0]["texture_transform"]
                ["scale"],
            serde_json::json!([2.0, 3.0])
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_export_path"],
            serde_json::json!("Data/libs/materials/metal/test_layer_roughness_TEX0.png")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["role"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["export_kind"],
            serde_json::json!("derived_ddna_alpha")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["derived_from_texture_identity"],
            serde_json::json!("ddna_normal")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["derived_from_semantic"],
            serde_json::json!("smoothness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["derived_from_channel"],
            serde_json::json!("a")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["value_transform"],
            serde_json::json!("sqrt_one_minus")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["value_channel"],
            serde_json::json!("r")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["packed_texture_format"],
            serde_json::json!("roughness_grayscale")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["packed_channel_semantics"]
                ["r"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["packed_channel_semantics"]
                ["g"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["packed_channel_semantics"]
                ["b"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["roughness_texture"]["constant_channel_values"]
                ["a"],
            serde_json::json!("1.0")
        );
        let gloss_mult = value["submaterials"][0]["layer_manifest"][0]["gloss_mult"]
            .as_f64()
            .expect("gloss_mult should be numeric");
        assert!((gloss_mult - 0.7).abs() < 1e-6);
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["layer_snapshot"]["shader"],
            serde_json::json!("Layer")
        );
        let wear_glossiness =
            value["submaterials"][0]["layer_manifest"][0]["layer_snapshot"]["wear_glossiness"]
                .as_f64()
                .expect("wear_glossiness should be numeric");
        assert!((wear_glossiness - 0.91).abs() < 1e-6);
        assert_eq!(
            value["submaterials"][0]["public_params"]["WearBlendBase"],
            serde_json::json!(0.5)
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["role"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["export_kind"],
            serde_json::json!("derived_ddna_alpha")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["derived_from_texture_identity"],
            serde_json::json!("ddna_normal")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["derived_from_semantic"],
            serde_json::json!("smoothness")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["derived_from_channel"],
            serde_json::json!("a")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["value_transform"],
            serde_json::json!("sqrt_one_minus")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["value_channel"],
            serde_json::json!("r")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["packed_texture_format"],
            serde_json::json!("roughness_grayscale")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["packed_channel_semantics"]["r"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["packed_channel_semantics"]["g"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["derived_textures"][0]["constant_channel_values"]["a"],
            serde_json::json!("1.0")
        );
        assert_eq!(
            value["submaterials"][0]["ddna_derivations"][0]["status"],
            serde_json::json!("missing")
        );
        assert_eq!(
            value["submaterials"][0]["ddna_derivations"][0]["reason"],
            serde_json::json!("missing_alpha_mips")
        );
        assert_eq!(
            value["submaterials"][0]["ddna_derivations"][0]["requested_mip"],
            serde_json::json!(0)
        );
        assert_eq!(
            value["submaterials"][0]["ddna_derivations"][0]["mip_selection"],
            serde_json::json!("unavailable")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["status"],
            serde_json::json!("exported")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["value_transform"],
            serde_json::json!("sqrt_one_minus")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["value_channel"],
            serde_json::json!("r")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["packed_texture_format"],
            serde_json::json!("roughness_grayscale")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["packed_channel_semantics"]
                ["r"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["packed_channel_semantics"]
                ["g"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["packed_channel_semantics"]
                ["b"],
            serde_json::json!("roughness")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["constant_channel_values"]
                ["a"],
            serde_json::json!("1.0")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["requested_mip"],
            serde_json::json!(0)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["selected_mip"],
            serde_json::json!(0)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["mip_selection"],
            serde_json::json!("requested")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["width"],
            serde_json::json!(256)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["height"],
            serde_json::json!(128)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["alpha_mip_count"],
            serde_json::json!(8)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["alpha_mip_format"],
            serde_json::json!("bc4_unorm")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["alpha_mip_layout"],
            serde_json::json!("numbered_sibling")
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["smoothness_min"],
            serde_json::json!(12)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["smoothness_max"],
            serde_json::json!(240)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["smoothness_mean"],
            serde_json::json!(128)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["roughness_min"],
            serde_json::json!(125)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["roughness_max"],
            serde_json::json!(250)
        );
        assert_eq!(
            value["submaterials"][0]["layer_manifest"][0]["ddna_derivations"][0]["roughness_mean"],
            serde_json::json!(180)
        );
        assert_eq!(
            value["submaterials"][0]["virtual_inputs"][0],
            serde_json::json!("$TintPaletteDecal")
        );
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
            ddna_derivations: Vec::new(),
        }];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0],
        );

        assert_eq!(
            value["submaterials"][0]["decoded_feature_flags"]["has_iridescence"],
            serde_json::json!(true)
        );
        assert_eq!(
            value["submaterials"][0]["texture_slots"][0]["role"],
            serde_json::json!("iridescence")
        );
        assert_eq!(
            value["submaterials"][0]["public_params"]["IridescenceIntensity"],
            serde_json::json!(0.75)
        );
        assert_eq!(
            value["submaterials"][0]["authored_public_params"][0]["name"],
            serde_json::json!("IridescenceIntensity")
        );
    }

    #[test]
    fn material_sidecar_json_marks_screen_crt_from_pixel_layout_slot() {
        let mut material = sample_submaterial();
        material.shader = "UIPlane".into();
        material.texture_slots = vec![crate::mtl::TextureSlotBinding {
            slot: "TexSlot17".into(),
            path: "Data/EngineAssets/Textures/ScreenInterference/pixel_layout_crt.tif".into(),
            is_virtual: false,
        }];
        material.public_params = vec![
            crate::mtl::PublicParam {
                name: "PixelGridTilingX".into(),
                value: "300".into(),
            },
            crate::mtl::PublicParam {
                name: "PixelGridTilingY".into(),
                value: "120".into(),
            },
        ];

        let materials = MtlFile {
            materials: vec![material],
            source_path: Some("Data/Materials/UI/rtt_comms_opaque_hightech.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };
        let extracted = vec![ExtractedMaterialEntry {
            slot_exports: vec![serde_json::json!({
                "slot": "TexSlot17",
                "role": "screen_pixel_layout",
                "is_virtual": false,
                "source_path": "Data/EngineAssets/Textures/ScreenInterference/pixel_layout_crt.tif",
                "export_path": null,
                "export_kind": "source",
                "authored_attributes": [],
                "authored_child_blocks": [],
            })],
            direct_texture_exports: Vec::new(),
            layer_exports: Vec::new(),
            derived_texture_exports: Vec::new(),
            ddna_derivations: Vec::new(),
        }];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Materials/UI/rtt_comms_opaque_hightech.mtl",
            "Data/Materials/UI/rtt_comms_opaque_hightech.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0],
        );

        let effects = &value["submaterials"][0]["screen_effects"];
        assert_eq!(effects["apply_crt"], serde_json::json!(true));
        assert_eq!(effects["source"], serde_json::json!("screen_pixel_layout"));
        assert_eq!(effects["pixel_layout_slot"], serde_json::json!("TexSlot17"));
        assert_eq!(effects["pixel_grid_tiling"], serde_json::json!({"x": 300.0, "y": 120.0}));
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
        let extracted = vec![
            ExtractedMaterialEntry::default(),
            ExtractedMaterialEntry::default(),
        ];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
            &[0, 1],
        );

        assert_eq!(
            value["submaterials"][0]["blender_material_name"],
            serde_json::json!("hull:hull_panel_0")
        );
        assert_eq!(
            value["submaterials"][1]["blender_material_name"],
            serde_json::json!("hull:hull_panel_1")
        );
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
                entity_names: ["child_a".to_string(), "child_b".to_string()]
                    .into_iter()
                    .collect(),
                material_sidecars: [
                    "Data/Objects/A.materials.json".to_string(),
                    "Data/Objects/B.materials.json".to_string(),
                ]
                .into_iter()
                .collect(),
            },
        );

        let value = build_livery_manifest_value(&records);
        assert_eq!(
            value["liveries"][0]["palette_source_name"],
            serde_json::json!("vehicle.palette.test")
        );
        assert_eq!(
            value["liveries"][0]["entity_names"]
                .as_array()
                .map(|items| items.len()),
            Some(2)
        );
        assert_eq!(
            value["liveries"][0]["material_sidecars"]
                .as_array()
                .map(|items| items.len()),
            Some(2)
        );
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
        assert_eq!(
            value["palettes"][0]["id"],
            serde_json::json!("palette/vehicle_palette_test")
        );
        assert_eq!(
            value["palettes"][0]["source_name"],
            serde_json::json!("vehicle.palette.test")
        );
        assert_eq!(
            value["palettes"][0]["display_name"],
            serde_json::json!("Vehicle Palette Test")
        );
        assert_eq!(
            value["palettes"][0]["glass"]
                .as_array()
                .map(|items| items.len()),
            Some(3)
        );
        assert_eq!(
            value["palettes"][0]["decal"]["source_path"],
            serde_json::json!("Data/Textures/branding/test_decal.png")
        );
        assert_eq!(
            value["palettes"][0]["decal"]["red"]
                .as_array()
                .map(|items| items.len()),
            Some(3)
        );
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
        assert_eq!(
            palette_id,
            "palette/vulture_coramor_2956_purple_pink_green_iridecence"
        );
        assert_eq!(
            value["palettes"][0]["id"],
            serde_json::json!("palette/vulture_coramor_2956_purple_pink_green_iridecence")
        );
        assert_eq!(
            value["palettes"][0]["source_name"],
            serde_json::json!("coramor_2956_purple_pink_green_iridecence")
        );
        assert_eq!(
            value["palettes"][0]["display_name"],
            serde_json::json!("Vulture Heartthrob Livery")
        );
    }

    #[test]
    fn material_source_request_prefers_loaded_source_path() {
        let materials = MtlFile {
            materials: Vec::new(),
            source_path: Some("Data\\Objects\\Ships\\Test\\canonical.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        };

        let path = material_source_request(
            &materials,
            "Data/objects/ships/test/canonical",
            "Data/Objects/Ships/Test/hull.skin",
        );

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

        let path = material_source_request(
            &materials,
            "Data/objects/ships/test/canonical",
            "Data/Objects/Ships/Test/hull.skin",
        );

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
        assert_eq!(
            view.mesh.submeshes[0].material_name.as_deref(),
            Some("hull")
        );
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

        let filtered =
            build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, false);
        assert_eq!(filtered.mesh.submeshes.len(), 2);
        assert_eq!(
            filtered.glb_nmc.as_ref().map(|combo| combo.nodes.len()),
            Some(2)
        );
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

        let filtered =
            build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, false);
        assert_eq!(filtered.mesh.submeshes.len(), 2);
        assert_eq!(
            filtered.glb_nmc.as_ref().map(|combo| combo.nodes.len()),
            Some(2)
        );
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

        let filtered =
            build_decomposed_material_view(&mesh, Some(&materials), Some(&nmc), false, false);

        assert_eq!(filtered.mesh.submeshes.len(), 1);
        assert_eq!(
            filtered.glb_nmc.as_ref().map(|combo| combo.nodes.len()),
            Some(2)
        );
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
    fn ui_export_stamp_written_when_generated_ui_files_exist() {
        let mut files = OutputFiles::new();
        insert_binary_file(
            &mut files,
            "Data/UI/Generated/ship/test/Test/screen.png".to_string(),
            vec![0u8],
        );
        insert_ui_export_stamp(&mut files);
        let stamp = files
            .get(UI_EXPORT_STAMP_PATH)
            .expect("stamp written alongside generated UI files");
        let value: serde_json::Value = serde_json::from_slice(stamp).expect("stamp parses");
        assert!(value["written_at_epoch_s"].as_u64().is_some_and(|s| s > 0));
        assert!(value["git_describe"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(value["binary_built_at_epoch_s"].is_u64());
    }

    #[test]
    fn ui_export_stamp_skipped_without_generated_ui_files() {
        let mut files = OutputFiles::new();
        insert_binary_file(
            &mut files,
            "Packages/Test/scene.json".to_string(),
            b"{}".to_vec(),
        );
        insert_ui_export_stamp(&mut files);
        assert!(!files.contains_key(UI_EXPORT_STAMP_PATH));
    }

    #[test]
    fn output_files_canonicalizes_segment_case_from_first_insert() {
        let mut of = OutputFiles::new();
        // First insert establishes "Data/Foo" casing.
        of.insert_canonical("Data/Foo/a.bin".to_string(), vec![1]);
        // A later path with different case on existing segments adopts the first casing.
        let p = of.insert_canonical("data/foo/b.bin".to_string(), vec![2]);
        assert_eq!(p, "Data/Foo/b.bin");
        // Identical bytes at an existing path dedupe to that path.
        let p2 = of.insert_canonical("Data/Foo/a.bin".to_string(), vec![1]);
        assert_eq!(p2, "Data/Foo/a.bin");
        assert_eq!(of.len(), 2);
    }

    #[test]
    fn insert_binary_file_reuses_identical_content_and_hashes_collisions() {
        let mut files = OutputFiles::new();
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
            local_transform_sc: Some(crate::socpak::build_container_transform(
                [1.0, 2.0, 3.0],
                [0.0, 90.0, 0.0],
            )),
            resolved_no_rotation: false,
            no_rotation: false,
            offset_position: [1.0, 2.0, 3.0],
            offset_rotation: [0.0, 90.0, 0.0],
            detach_direction: [0.0, 0.0, -1.0],
            port_flags: "invisible uneditable".into(),
            ui_bindings: Vec::new(),
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
                material_sidecar: Some(
                    "Data/Objects/Ships/Test/interior_panel.materials.json".into(),
                ),
                entity_class_guid: Some("1234".into()),
                ui_bindings: Vec::new(),
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
            None,
            None,
            &ExportOptions::default(),
        );

        assert_eq!(
            value["root_entity"]["mesh_asset"],
            serde_json::json!("Data/Objects/Ships/Test/root.glb")
        );
        assert_eq!(
            value["children"][0]["mesh_asset"],
            serde_json::json!("Data/Objects/Ships/Test/child.glb")
        );
        assert_eq!(
            value["children"][0]["parent_node_name"],
            serde_json::json!("hardpoint_weapon_left")
        );
        assert_eq!(
            value["children"][0]["source_transform_basis"],
            serde_json::json!("gltf_y_up")
        );
        assert!(value["children"][0]["local_transform_sc"].is_array());
        assert_eq!(
            value["children"][0]["resolved_no_rotation"],
            serde_json::json!(false)
        );
        assert_eq!(
            value["interiors"][0]["parent_entity_name"],
            serde_json::json!("child_entity")
        );
        assert_eq!(
            value["interiors"][0]["parent_node_name"],
            serde_json::json!("child_root")
        );
        assert_eq!(
            value["interiors"][0]["placements"][0]["mesh_asset"],
            serde_json::json!("Data/Objects/Ships/Test/interior_panel.glb")
        );
        assert_eq!(
            value["package_rule"]["package_dir"],
            serde_json::json!("Packages/ARGO MOLE")
        );
        assert_eq!(
            value["package_rule"]["normalized_p4k_relative_paths"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn scene_manifest_includes_weapon_assembly_section_when_present() {
        let weapon_assembly = serde_json::json!({
            "root": { "entity_name": "BEHR_P4AR" },
            "parts": [
                {
                    "role": "magazine",
                    "attach_slot": "magazine",
                    "attach_bone": "magAttach",
                    "transform_source": "alias"
                }
            ],
            "unresolved_slots": [],
        });

        let value = build_scene_manifest_value(
            "BEHR_P4AR",
            "BEHR_P4AR_LOD0_MIP0",
            "Data/Objects/Weapons/FPS_Weapons/BEHR/P4AR/p4ar.cdf",
            "Data/Objects/Weapons/FPS_Weapons/BEHR/P4AR/p4ar.mtl",
            "Data/Objects/Weapons/FPS_Weapons/BEHR/P4AR/p4ar.blend",
            None,
            None,
            None,
            &[],
            &[],
            &[],
            Some("fps_weapon"),
            Some(&weapon_assembly),
            &ExportOptions::default(),
        );

        assert_eq!(value["assembly_kind"], serde_json::json!("fps_weapon"));
        assert_eq!(
            value["weapon_assembly"]["parts"][0]["attach_bone"],
            serde_json::json!("magAttach")
        );
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
                mesh_asset: "Data/Objects/Spaceships/Thrusters/DRAK/test_thruster_LOD0.blend"
                    .into(),
                source_material_index: 7,
                submaterial_name: "Glow_Thrusters".into(),
                blender_material_name: "root:Glow_Thrusters".into(),
            }],
            None,
            None,
            &ExportOptions::default(),
        );

        assert_eq!(
            value["controls"]["engine_glow"]["default_strength"],
            serde_json::json!(0.0)
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
    fn engine_glow_targets_require_main_thruster_attach_def_type() {
        let child = EntityPayload {
            mesh: sample_mesh(vec![]),
            materials: None,
            textures: None,
            nmc: None,
            palette: None,
            geometry_path: String::new(),
            material_path: String::new(),
            bones: Vec::new(),
            skeleton_source_path: None,
            entity_name: "thruster".into(),
            entity_category: Some("Thruster".into()),
            attach_def_type: Some("MainThruster".into()),
            parent_node_name: "bone".into(),
            parent_entity_name: "parent".into(),
            no_rotation: false,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            detach_direction: [0.0; 3],
            port_flags: String::new(),
            ui_bindings: Vec::new(),
        };
        assert!(should_export_engine_glow_targets(&child));

        let maneuver_child = EntityPayload {
            attach_def_type: Some("ManneuverThruster".into()),
            ..child
        };
        assert!(!should_export_engine_glow_targets(&maneuver_child));
    }

    #[test]
    fn resolve_no_rotation_local_matrix_suppresses_duplicate_zero_rotation_offset() {
        let parent_world =
            glam::Mat4::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)).to_cols_array();
        let resolved =
            resolve_no_rotation_local_matrix(parent_world, [3.0, 0.0, 0.0], [0.0, 0.0, 0.0]);

        assert_eq!(resolved[12], 0.0);
        assert_eq!(resolved[13], 0.0);
        assert_eq!(resolved[14], 0.0);
    }

    #[test]
    fn resolve_no_rotation_local_matrix_treats_tiny_rotation_as_zero() {
        let parent_world =
            glam::Mat4::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)).to_cols_array();
        let resolved =
            resolve_no_rotation_local_matrix(parent_world, [3.0, 0.0, 0.0], [1e-7, 0.0, 0.0]);

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
            world_to_bone: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
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
                    [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                ),
                test_nmc_node(
                    "hardpoint_docking_module",
                    Some(0),
                    [
                        [0.0, -1.0, 0.0, -18.9],
                        [1.0, 0.0, 0.0, -18.38946],
                        [0.0, 0.0, 1.0, 5.51615],
                    ],
                ),
                test_nmc_node(
                    "hardpoint_docking_host",
                    Some(0),
                    [
                        [1.0, 0.0, 0.0, -19.15725],
                        [0.0, 1.0, 0.0, -18.37498],
                        [0.0, 0.0, 1.0, 7.30487],
                    ],
                ),
            ],
            material_indices: Vec::new(),
        };
        let child_nmc = NodeMeshCombo {
            nodes: vec![test_nmc_node(
                "hardpoint_docking_vehicle",
                None,
                [
                    [1.0, 0.0, 0.0, 2.98941],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 1.0474],
                ],
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
            geometry_path: "Objects/Spaceships/Ships/DRAK/command_module/exterior/test.cga"
                .to_string(),
            material_path: String::new(),
            bones: Vec::new(),
            skeleton_source_path: None,
            entity_name: "DRAK_Test_Command_Module".to_string(),
            entity_category: None,
            attach_def_type: None,
            parent_node_name: "hardpoint_docking_module".to_string(),
            parent_entity_name: "root".to_string(),
            no_rotation: true,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            detach_direction: [0.0; 3],
            port_flags: "Docking_Request_Accepting".to_string(),
            ui_bindings: Vec::new(),
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
        assert_eq!(
            view.mesh.submeshes[0].material_id, 0,
            "compacted material_id for hull"
        );
        assert_eq!(
            view.mesh.submeshes[0].source_material_id,
            Some(1),
            "source index for hull"
        );

        // GLB submesh 1 → decal (original index 2)
        assert_eq!(
            view.mesh.submeshes[1].material_id, 1,
            "compacted material_id for decal"
        );
        assert_eq!(
            view.mesh.submeshes[1].source_material_id,
            Some(2),
            "source index for decal"
        );
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
            sidecar_a
                .materials
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            sidecar_b
                .materials
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            "sidecar material lists should be identical"
        );
        assert_eq!(
            view_a.sidecar_original_indices, view_b.sidecar_original_indices,
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
        let path_a = material_sidecar_relative_path(
            "Data/Objects/Ships/Drak/Clipper/hull.mtl",
            "fallback",
            0,
        );
        let path_b = material_sidecar_relative_path(
            "Data/Objects/Ships/Drak/Clipper/hull.mtl",
            "other_fallback",
            0,
        );

        assert_eq!(path_a, path_b, "same source .mtl must produce the same sidecar path");
        assert_eq!(path_a, "Data/objects/ships/drak/clipper/hull_TEX0.materials.json");
    }

    #[test]
    fn material_sidecar_relative_path_encodes_mip_level() {
        let path0 = material_sidecar_relative_path("Data/Objects/Ships/Test/hull.mtl", "f", 0);
        let path2 = material_sidecar_relative_path("Data/Objects/Ships/Test/hull.mtl", "f", 2);

        assert_eq!(path0, "Data/objects/ships/test/hull_TEX0.materials.json");
        assert_eq!(path2, "Data/objects/ships/test/hull_TEX2.materials.json");
        assert_ne!(path0, path2);
    }

    #[test]
    fn material_sidecar_relative_path_normalizes_case() {
        let path = material_sidecar_relative_path("Data/Objects/Spaceships/Ships/DRAK/Clipper/EXTERIOR/DRAK_CLIPPER_EXT.mtl", "f", 0);

        assert_eq!(path, "Data/objects/spaceships/ships/drak/clipper/exterior/drak_clipper_ext_TEX0.materials.json");
    }
}
