use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use gltf_json as json;
use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::gltf::{offset_to_gltf_matrix, GlbBuilder, GlbInput, GlbLoaders, GlbMetadata, GlbOptions, PackedMeshInfo};
use crate::mtl::{MtlFile, SemanticTextureBinding, SubMaterial, TextureSemanticRole, TintPalette};
use crate::nmc::NodeMeshCombo;
use crate::pipeline::{
    DecomposedExport, ExportOptions, ExportedFile, ExportedFileKind, InteriorCgfEntry,
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
    pub children: Vec<EntityPayload>,
    pub interiors: LoadedInteriors,
    /// All available paint variants for this entity, populated from SubGeometry entries.
    pub paint_variants: Vec<crate::mtl::PaintVariant>,
}

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
struct DecomposedMaterialView {
    mesh: Mesh,
    sidecar_materials: Option<MtlFile>,
    glb_materials: Option<MtlFile>,
    glb_nmc: Option<NodeMeshCombo>,
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
    let duplicate_offset = offset_rotation == [0.0, 0.0, 0.0]
        && (rotated_offset - parent_translation).abs().max_element() <= 5e-4;
    let local_translation = if duplicate_offset {
        glam::Vec3::ZERO
    } else {
        desired_translation
    };
    glam::Mat4::from_rotation_translation(parent_rotation.inverse() * desired_rotation, local_translation)
        .to_cols_array()
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
            Some(resolve_no_rotation_local_matrix(
                builder.compute_node_world_matrix(target_idx as usize),
                child.offset_position,
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
                entity_name: child.entity_name.clone(),
                parent_node_name: child.parent_node_name.clone(),
                parent_entity_name: child.parent_entity_name.clone(),
                no_rotation: child.no_rotation,
                offset_position: child.offset_position,
                offset_rotation: child.offset_rotation,
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

fn build_decomposed_material_view(
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
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    };

    if include_nodraw {
        let filtered_mesh = filter_mesh_geometry(mesh, Some(materials), nmc, include_nodraw, include_shields);
        let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
        return DecomposedMaterialView {
            mesh: filtered_mesh,
            sidecar_materials: Some(materials.clone()),
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    }

    let mut material_id_map = Vec::with_capacity(materials.materials.len());
    let mut filtered_materials = Vec::with_capacity(materials.materials.len());
    for material in &materials.materials {
        if material.should_hide() {
            material_id_map.push(None);
        } else {
            material_id_map.push(Some(filtered_materials.len() as u32));
            filtered_materials.push(material.clone());
        }
    }

    let mut dropped_out_of_range = false;
    let mut filtered_mesh = mesh.clone();
    filtered_mesh.submeshes = mesh
        .submeshes
        .iter()
        .filter_map(|submesh| {
            if submesh_is_excluded_helper(submesh, nmc, include_nodraw, include_shields) {
                return None;
            }

            let Some(mapped) = material_id_map.get(submesh.material_id as usize) else {
                dropped_out_of_range = true;
                return None;
            };
            let Some(new_material_id) = *mapped else {
                return None;
            };

            let mut filtered = submesh.clone();
            filtered.material_id = new_material_id;
            if let Some(material) = filtered_materials.get(new_material_id as usize) {
                filtered.material_name = Some(material.name.clone());
            }
            Some(filtered)
        })
        .collect();

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
        let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);
        return DecomposedMaterialView {
            mesh: filtered_mesh,
            sidecar_materials: Some(materials.clone()),
            glb_materials: None,
            glb_nmc: filtered_nmc,
        };
    }

    let filtered_materials = MtlFile {
        materials: filtered_materials,
        source_path: materials.source_path.clone(),
        paint_override: materials.paint_override.clone(),
        material_set: materials.material_set.clone(),
    };

    let (filtered_mesh, filtered_nmc) = filter_nmc_hierarchy(filtered_mesh, nmc, include_nodraw, include_shields);

    DecomposedMaterialView {
        mesh: filtered_mesh,
        sidecar_materials: Some(filtered_materials.clone()),
        glb_materials: Some(filtered_materials),
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
                if materials
                    .materials
                    .get(submesh.material_id as usize)
                    .is_some_and(crate::mtl::SubMaterial::should_hide)
                    && !include_nodraw
                {
                    return false;
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
    load_interior_mesh: &mut dyn FnMut(
        &InteriorCgfEntry,
    ) -> Option<(Mesh, Option<MtlFile>, Option<NodeMeshCombo>)>,
) -> Result<DecomposedExport, Error> {
    let mut files = BTreeMap::new();
    let mut texture_cache: HashMap<(String, TextureFlavor), String> = HashMap::new();
    let mut png_cache = PngCache::new();
    let mut palette_records = BTreeMap::new();
    let mut livery_usage = BTreeMap::new();
    let package_name = package_directory_name(&input.entity_name, opts.lod_level, opts.texture_mip);
    let scene_manifest_path = package_relative_path(&package_name, "scene.json");
    let palettes_manifest_path = package_relative_path(&package_name, "palettes.json");
    let liveries_manifest_path = package_relative_path(&package_name, "liveries.json");
    report_progress(progress, 0.05, "Writing root assets");
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
            opts.texture_mip,
            existing_asset_paths,
        )
    });
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
                opts.texture_mip,
                existing_asset_paths,
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

    report_progress(progress, 0.15, "Writing child assets");

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
                opts.texture_mip,
                existing_asset_paths,
            )
        });
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
            source_transform_basis: Some("cryengine_z_up".to_string()),
            local_transform_sc: Some(resolved_transform.local_transform_sc),
            resolved_no_rotation: resolved_transform.resolved_no_rotation,
            no_rotation: child.no_rotation,
            offset_position: child.offset_position,
            offset_rotation: child.offset_rotation,
        });

        if child_count > 0 {
            let fraction = (index + 1) as f32 / child_count as f32;
            report_progress(progress, 0.15 + 0.40 * fraction, "Writing child assets");
        }
    }
    if child_count == 0 {
        report_progress(progress, 0.55, "Writing interior assets");
    }

    let mut interior_asset_cache: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut interior_records = Vec::with_capacity(input.interiors.containers.len());
    let container_count = input.interiors.containers.len();
    for (index, container) in input.interiors.containers.iter().enumerate() {
        let palette_id = container
            .palette
            .as_ref()
            .map(|palette| register_palette(&mut palette_records, palette));
        let mut placements = Vec::with_capacity(container.placements.len());
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
            let cache_key = format!(
                "{}|{}",
                entry.cgf_path.to_lowercase(),
                entry.material_path.as_deref().unwrap_or("").to_lowercase()
            );
            let (mesh_asset, material_sidecar) = if let Some(cached) = interior_asset_cache.get(&cache_key) {
                cached.clone()
            } else {
                let Some((mesh, materials, _nmc)) = load_interior_mesh(entry) else {
                    log::warn!("failed to build decomposed interior asset for {}", entry.cgf_path);
                    continue;
                };
                let interior_material_view = build_decomposed_material_view(
                    &mesh,
                    materials.as_ref(),
                    None,
                    opts.include_nodraw,
                    opts.include_shields,
                );
                let requested_mesh_asset = mesh_asset_relative_path(p4k, &entry.cgf_path, &entry.name, opts.lod_level);
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
                    if let Some(requested_path) = requested_material_sidecar.as_ref() {
                        if files.contains_key(requested_path)
                            || existing_asset_paths.is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
                        {
                            return requested_path.clone();
                        }
                    }
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
                        opts.texture_mip,
                        existing_asset_paths,
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
                        existing_asset_paths,
                    )?
                };
                interior_asset_cache.insert(cache_key, (mesh_asset.clone(), material_sidecar.clone()));
                (mesh_asset, material_sidecar)
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
        }

        interior_records.push(InteriorContainerRecord {
            name: container.name.clone(),
            palette_id,
            container_transform: container.container_transform,
            placements,
            lights: container
                .lights
                .iter()
                .map(|light| {
                    // Extract the projector (gobo) DDS from P4k into the
                    // decomposed package so it is self-contained. We keep the
                    // original block format (BC6H / BC7 / DXT) by re-emitting
                    // the raw DDS — gobo textures are frequently HDR BC6H,
                    // which our RGBA decoder does not support today.
                    let projector_texture_export = light
                        .projector_texture
                        .as_deref()
                        .and_then(|src| {
                            let normalized = normalize_source_path(p4k, src);
                            let relative = replace_extension(&normalized, ".dds");
                            let lookup_key = relative.to_ascii_lowercase();
                            if existing_asset_paths
                                .is_some_and(|paths| paths.contains(&lookup_key))
                                || files.contains_key(&relative)
                            {
                                return Some(relative);
                            }
                            let bytes = crate::pipeline::load_raw_dds_file(p4k, src)?;
                            Some(insert_binary_file(&mut files, relative, bytes))
                        });
                    serde_json::json!({
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
                                    }),
                                )
                            })
                            .collect::<serde_json::Map<_, _>>(),
                    })
                })
                .collect(),
        });

        if container_count > 0 {
            let fraction = (index + 1) as f32 / container_count as f32;
            report_progress(progress, 0.55 + 0.30 * fraction, "Writing interior assets");
        }
    }
    if container_count == 0 {
        report_progress(progress, 0.85, "Writing manifests");
    }

    let scene_manifest = build_scene_manifest_value(
        &input.entity_name,
        &package_name,
        &normalize_source_path(p4k, &input.geometry_path),
        &normalize_source_path(p4k, &input.material_path),
        &root_mesh_asset,
        root_material_sidecar.as_deref(),
        root_palette_id.as_deref(),
        &child_instances,
        &interior_records,
        opts,
    );
    report_progress(progress, 0.95, "Writing manifests");
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
    } else if relative_path.ends_with(".glb") {
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
    child_instances: &[SceneInstanceRecord],
    interiors: &[InteriorContainerRecord],
    opts: &ExportOptions,
) -> serde_json::Value {
    serde_json::json!({
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
    })
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
    })
}

fn interior_container_json(container: &InteriorContainerRecord) -> serde_json::Value {
    serde_json::json!({
        "name": container.name,
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
    mesh: &Mesh,
    materials: Option<&MtlFile>,
    nmc: Option<&NodeMeshCombo>,
    bones: &[Bone],
    lod_level: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<String, Error> {
    fn no_textures(
        _: Option<&crate::mtl::MtlFile>,
        _: Option<&crate::mtl::TintPalette>,
    ) -> Option<crate::types::MaterialTextures> {
        None
    }

    fn no_interiors(
        _: &crate::pipeline::InteriorCgfEntry,
    ) -> Option<(Mesh, Option<MtlFile>, Option<NodeMeshCombo>)> {
        None
    }

    let mut no_textures_fn = no_textures;
    let mut no_interiors_fn = no_interiors;
    let requested_path = mesh_asset_relative_path(p4k, geometry_path, fallback_name, lod_level);
    if existing_asset_paths
        .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
    {
        return Ok(requested_path);
    }
    let glb = crate::gltf::write_glb(
        GlbInput {
            root_mesh: Some(mesh.clone()),
            root_materials: materials.cloned(),
            root_textures: None,
            root_nmc: nmc.cloned(),
            root_palette: None,
            skeleton_bones: bones.to_vec(),
            children: Vec::new(),
            interiors: LoadedInteriors::default(),
        },
        &mut GlbLoaders {
            load_textures: &mut no_textures_fn,
            load_interior_mesh: &mut no_interiors_fn,
        },
        &GlbOptions {
            material_mode: MaterialMode::None,
            preserve_textureless_decal_primitives: true,
            metadata: GlbMetadata {
                entity_name: Some(fallback_name.to_string()),
                geometry_path: (!geometry_path.is_empty()).then_some(geometry_path.to_string()),
                material_path: None,
                export_options: crate::gltf::ExportOptionsMetadata {
                    kind: "Decomposed".to_string(),
                    material_mode: "None".to_string(),
                    format: "Glb".to_string(),
                    lod_level: 0,
                    texture_mip: 0,
                    include_attachments: false,
                    include_interior: false,
                },
            },
            fallback_palette: None,
        },
    )?;
    Ok(insert_binary_file(files, requested_path, glb))
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
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> String {
    let source_material_path = material_source_path(p4k, materials, material_path, geometry_path);
    let relative_path = material_sidecar_relative_path(&source_material_path, fallback_name, texture_mip);
    let extracted = materials
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
            )
        })
        .collect::<Vec<_>>();
    let value = build_material_sidecar_value(
        materials,
        &source_material_path,
        &relative_path,
        palettes_manifest_path,
        &extracted,
    );
    insert_json_file(files, relative_path, value)
}

fn extract_material_entry(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    material: &SubMaterial,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
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
            let layer_mtl = crate::pipeline::try_load_mtl(p4k, &crate::pipeline::datacore_path_to_p4k(&layer.path));
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
        "submaterials": materials.materials.iter().enumerate().map(|(index, material)| {
            build_submaterial_json(
                material,
                source_material_path,
                source_stem,
                &blender_material_names[index],
                index,
                &extracted[index],
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

fn mesh_asset_relative_path(p4k: &MappedP4k, geometry_path: &str, fallback_name: &str, lod: u32) -> String {
    let base = if geometry_path.is_empty() {
        format!("Data/generated/{}.glb", sanitize_identifier(fallback_name))
    } else {
        replace_extension(&normalize_source_path(p4k, geometry_path), ".glb")
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

fn normalize_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = crate::pipeline::datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| normalize_requested_source_path(path))
}

fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
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
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("glass".into()),
                material_id: 2,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
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
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("broken".into()),
                material_id: 9,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
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
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
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
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 0,
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
            source_transform_basis: Some("cryengine_z_up".into()),
            local_transform_sc: Some(crate::socpak::build_container_transform([1.0, 2.0, 3.0], [0.0, 90.0, 0.0])),
            resolved_no_rotation: false,
            no_rotation: false,
            offset_position: [1.0, 2.0, 3.0],
            offset_rotation: [0.0, 90.0, 0.0],
        };
        let interior = InteriorContainerRecord {
            name: "interior_main".into(),
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
            &[child],
            &[interior],
            &ExportOptions::default(),
        );

        assert_eq!(value["root_entity"]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/root.glb"));
        assert_eq!(value["children"][0]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/child.glb"));
        assert_eq!(value["children"][0]["parent_node_name"], serde_json::json!("hardpoint_weapon_left"));
        assert_eq!(value["children"][0]["source_transform_basis"], serde_json::json!("cryengine_z_up"));
        assert!(value["children"][0]["local_transform_sc"].is_array());
        assert_eq!(value["children"][0]["resolved_no_rotation"], serde_json::json!(false));
        assert_eq!(value["interiors"][0]["placements"][0]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/interior_panel.glb"));
        assert_eq!(value["package_rule"]["package_dir"], serde_json::json!("Packages/ARGO MOLE"));
        assert_eq!(value["package_rule"]["normalized_p4k_relative_paths"], serde_json::json!(true));
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
    fn normalized_relative_paths_join_beneath_selected_base_directory() {
        let base_dir = std::path::PathBuf::from("/tmp/export-root");
        let texture_path = replace_extension(
            &normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"),
            ".png",
        );
        let full_path = base_dir.join(texture_path);

        assert_eq!(
            full_path.to_string_lossy(),
            "/tmp/export-root/Data/Objects/Ships/Test/hull_diff.png"
        );
    }
}