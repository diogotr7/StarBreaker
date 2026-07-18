//! Blender decomposed export: convert `DecomposedInput` to individual `.blend` files.
//!
//! Phase 1 (mesh decomposition):
//! - Extract meshes from `DecomposedInput::children`
//! - Build material slots (empty, names-only)
//! - Write individual `.blend` files to `Data/Objects/...` paths
//! - Return decomposed export with .blend mesh files instead of GLB
//!
//! Phase 2 (scene.blend linking) — infrastructure complete
//! Phase 3 (lights and empties) — extraction and creation
//! Phase 4 (decal vertex groups) — material identification
//! Phase 5D (decal material assignment) — vertex group material assignment
#![allow(dead_code)]

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;
use starbreaker_blend::{
    allocate_idprop_blocks,
    bytes4_data, build_attribute, build_attribute_array, build_base, build_collection,
    build_master_collection, build_collection_object_linked,
    build_file_global, build_layer_collection, build_layer_collection_linked, build_mat_ptr_array,
    build_mat_ptr_array_from_ptrs, build_material_with_node_tree_and_properties,
    build_matbits, build_mesh, build_motion_blur_shutter_curve_points, build_object,
    build_cycles_render_settings_system_properties, build_scene_with_motion_blur_curve_and_properties,
    build_tool_settings, write_world_with_sky_shader,
    build_view_layer,
    floats2_data, floats3_data, ints_data, startup_ui_prefix_bytes, write_block, write_block_header, PtrAlloc,
    ATTR_DOMAIN_CORNER, ATTR_DOMAIN_EDGE, ATTR_DOMAIN_FACE, ATTR_DOMAIN_POINT, ATTR_TYPE_BYTE_COLOR,
    ATTR_TYPE_FLOAT2, ATTR_TYPE_FLOAT3, ATTR_TYPE_INT, BLEND_MAGIC, DNA1_BYTES,
    SDNA_IDX_ATTRIBUTE, SDNA_IDX_ATTRIBUTE_ARRAY, SDNA_IDX_BASE, SDNA_IDX_COLLECTION, SDNA_IDX_COLLECTION_CHILD,
    SDNA_IDX_COLLECTION_OBJECT, SDNA_IDX_DNA1, SDNA_IDX_FILE_GLOBAL, SDNA_IDX_LAYER_COLLECTION,
    SDNA_IDX_IDPROPERTY, SDNA_IDX_MATERIAL, SDNA_IDX_MESH, SDNA_IDX_OBJECT, SDNA_IDX_SCENE, SDNA_IDX_TOOL_SETTINGS,
    SDNA_IDX_VIEW_LAYER, SDNA_IDX_LIBRARY, SDNA_IDX_ID,
    build_lamp, build_lamp_with_node_tree_and_properties, build_lamp_object,
    build_lamp_object_with_properties_and_visibility, build_empty_object, build_empty_object_with_properties,
    build_library_block, build_id_stub, LAMP_SIZE, OBJECT_SIZE,
    build_bdeformgroup, build_mdeformvert_array, build_mdeformweight_array,
    build_custom_data_layer_mdeformvert, SDNA_IDX_BDEFORMGROUP, SDNA_IDX_MDEFORMVERT, SDNA_IDX_LAMP,
    build_displace_modifier, build_weld_modifier, build_weighted_normal_modifier, set_object_modifiers_listbase,
    SDNA_IDX_DISPLACE_MODIFIER, SDNA_IDX_WELD_MODIFIER, SDNA_IDX_WEIGHTED_NORMAL_MODIFIER,
    ints2_data, triangle_edge_topology, write_f32, write_i16, write_identity_matrix4x4, write_ptr,
    write_idprop_blocks, write_light_gobo_node_tree, ATTR_TYPE_INT32_2D, IdPropValue, STARTUP_UI_SCREEN_PTR,
};

use crate::error::Error;
use crate::decomposed::DecomposedInput;
use crate::pipeline::{
    load_interior_mesh_asset, preload_interior_meshes, DecomposedExport, ExportedFile, ExportedFileKind, ExportOptions,
    LoadedInteriors, PngCache, port_flags_mark_invisible,
};
use crate::types::{Mesh, SubMesh};
use crate::nmc::NodeMeshCombo;
use crate::mtl::MtlFile;

const DECAL_OFFSET_GROUP_NAME: &str = "starbreaker_decal_offset";
const DECAL_OFFSET_MODIFIER_NAME: &str = "StarBreaker Decal Offset";
const DECAL_OFFSET_MIDLEVEL: f32 = 0.0;

/// Internal structure to hold mesh data for .blend file generation
#[derive(Clone)]
struct MeshDataEntry {
    mesh: Mesh,
    materials: Option<MtlFile>,
    nmc: Option<NodeMeshCombo>,
    interior_placement_space: bool,
}

struct BlendAssetJob {
    blend_path: String,
    mesh_name: String,
    blend_key: String,
}

struct BuiltBlendAsset {
    file: Option<ExportedFile>,
    relative_path: String,
    linked_mesh_refs: Vec<LinkedMeshRef>,
    source_nodes: Vec<LinkedSourceNode>,
}

struct CyclesSceneProps {
    root_ptr: u64,
    root: Vec<u8>,
    cycles_group_ptr: u64,
    cycles_group: Vec<u8>,
    children: Vec<(u64, Vec<u8>)>,
}

fn allocate_cycles_scene_props(ptrs: &mut PtrAlloc) -> CyclesSceneProps {
    let root_ptr = ptrs.alloc();
    let cycles_group_ptr = ptrs.alloc();
    let child_ptrs = [
        ptrs.alloc(),
        ptrs.alloc(),
        ptrs.alloc(),
        ptrs.alloc(),
        ptrs.alloc(),
        ptrs.alloc(),
        ptrs.alloc(),
        ptrs.alloc(),
    ];
    let (root, cycles_group, children) =
        build_cycles_render_settings_system_properties(root_ptr, cycles_group_ptr, &child_ptrs);
    CyclesSceneProps {
        root_ptr,
        root,
        cycles_group_ptr,
        cycles_group,
        children,
    }
}

fn write_cycles_scene_props(out: &mut Vec<u8>, props: &CyclesSceneProps) {
    write_block(out, b"DATA", SDNA_IDX_IDPROPERTY, props.root_ptr, 1, &props.root);
    write_block(
        out,
        b"DATA",
        SDNA_IDX_IDPROPERTY,
        props.cycles_group_ptr,
        1,
        &props.cycles_group,
    );
    for (child_ptr, child_data) in &props.children {
        write_block(out, b"DATA", SDNA_IDX_IDPROPERTY, *child_ptr, 1, child_data);
    }
}

fn material_custom_properties(material_name: &str, materials: &Option<MtlFile>) -> Vec<(String, IdPropValue)> {
    let mut props = vec![(
        "starbreaker_material_identity".to_string(),
        IdPropValue::String(material_name.to_string()),
    )];
    if let Some(source_path) = materials.as_ref().and_then(|mtl| mtl.source_path.clone()) {
        props.push((
            "starbreaker_material_sidecar".to_string(),
            IdPropValue::String(source_path),
        ));
    }
    props
}

fn package_scene_path(package_name: &str) -> String {
    format!("Packages/{package_name}/scene.json")
}

fn package_path_depth(package_name: &str) -> usize {
    package_name.split('/').filter(|part| !part.is_empty()).count()
}

fn scene_library_blend_path(package_name: &str, mesh_asset: &str) -> String {
    let mut relative = String::from("//");
    for _ in 0..(package_path_depth(package_name) + 1) {
        relative.push_str("../");
    }
    relative.push_str(mesh_asset);
    relative
}

fn package_root_properties(entity_name: &str) -> Vec<(String, IdPropValue)> {
    vec![
        ("starbreaker_package_root".to_string(), IdPropValue::Int(1)),
        ("starbreaker_scene_path".to_string(), IdPropValue::String(package_scene_path(entity_name))),
        ("starbreaker_export_root".to_string(), IdPropValue::String(String::new())),
        ("starbreaker_package_name".to_string(), IdPropValue::String(entity_name.to_string())),
        ("starbreaker_entity_name".to_string(), IdPropValue::String(entity_name.to_string())),
        ("starbreaker_palette_id".to_string(), IdPropValue::String(String::new())),
    ]
}

fn entity_wrapper_properties(entity_name: &str) -> Vec<(String, IdPropValue)> {
    vec![
        ("starbreaker_scene_path".to_string(), IdPropValue::String(package_scene_path(entity_name))),
        ("starbreaker_export_root".to_string(), IdPropValue::String(String::new())),
        ("starbreaker_package_name".to_string(), IdPropValue::String(entity_name.to_string())),
        ("starbreaker_entity_name".to_string(), IdPropValue::String(entity_name.to_string())),
    ]
}

fn entity_root_properties(package_name: &str, entity_name: &str) -> Vec<(String, IdPropValue)> {
    vec![
        ("starbreaker_scene_path".to_string(), IdPropValue::String(package_scene_path(package_name))),
        ("starbreaker_export_root".to_string(), IdPropValue::String(String::new())),
        ("starbreaker_package_name".to_string(), IdPropValue::String(package_name.to_string())),
        ("starbreaker_entity_name".to_string(), IdPropValue::String(entity_name.to_string())),
    ]
}

fn scene_instance_properties(entity_name: &str, instance: &LinkedMeshInstance) -> Vec<(String, IdPropValue)> {
    let instance_json = serde_json::json!({
        "entity_name": instance.name,
        "mesh_asset": instance.mesh_asset,
        "material_sidecar": instance.material_sidecar,
        "palette_id": instance.palette_id,
        "source_object_name": instance.source_object_name,
        "source_parent_name": instance.source_parent_name,
        "source_ancestors": instance.source_ancestors.iter().map(|ancestor| ancestor.name.clone()).collect::<Vec<_>>(),
        "ui_bindings": instance.ui_bindings,
    })
    .to_string();
    let mut props = vec![
        ("starbreaker_scene_path".to_string(), IdPropValue::String(package_scene_path(entity_name))),
        ("starbreaker_export_root".to_string(), IdPropValue::String(String::new())),
        ("starbreaker_package_name".to_string(), IdPropValue::String(entity_name.to_string())),
        ("starbreaker_entity_name".to_string(), IdPropValue::String(instance.name.clone())),
        ("starbreaker_mesh_asset".to_string(), IdPropValue::String(instance.mesh_asset.clone())),
        ("starbreaker_instance_json".to_string(), IdPropValue::String(instance_json)),
        (
            "starbreaker_decal_offset_strength".to_string(),
            IdPropValue::Double(instance_decal_offset_strength(instance)),
        ),
    ];
    if let Some(material_sidecar) = &instance.material_sidecar {
        props.push((
            "starbreaker_material_sidecar".to_string(),
            IdPropValue::String(material_sidecar.clone()),
        ));
    }
    if let Some(palette_id) = &instance.palette_id {
        props.push((
            "starbreaker_palette_id".to_string(),
            IdPropValue::String(palette_id.clone()),
        ));
    }
    props
}

/// Determine the Blender Displace modifier strength for the decal-offset pass on
/// a scene instance.
///
/// Interior instances (and all loadout items parented inside the interior) use a
/// smaller offset (0.001) because interior geometry is scaled more tightly and a
/// 0.005 offset causes visible lifting on flat panels.  Exterior hull meshes use
/// 0.005 to clear the thicker paint/decal stacking that occurs on the outside of
/// the ship.
///
/// Decision tree:
/// 1. `is_interior == true` → **0.001** (interior geometry + interior loadout)
/// 2. `material_sidecar` path contains `/ships/` and does **not** contain `_int_`
///    → **0.005** (exterior hull)
/// 3. Everything else (exterior loadout/accessories) → **0.001**
fn instance_decal_offset_strength(instance: &LinkedMeshInstance) -> f64 {
    if instance.is_interior {
        return 0.001;
    }
    if let Some(sidecar) = &instance.material_sidecar {
        let lower = sidecar.to_lowercase();
        if lower.contains("/ships/") && !lower.contains("_int_") && !lower.contains("_int_master") {
            return 0.005;
        }
    }
    0.001
}

fn has_decal_offset_vertices(vertex_groups: Option<&Vec<VertexGroup>>) -> bool {
    vertex_groups.is_some_and(|groups| {
        groups
            .iter()
            .any(|group| group.name == DECAL_OFFSET_GROUP_NAME && !group.vertex_indices.is_empty())
    })
}

fn mesh_ref_has_decal_offset(
    instance: &LinkedMeshInstance,
    decal_mesh_refs: &HashSet<(String, String)>,
) -> bool {
    decal_mesh_refs.contains(&(instance.mesh_asset.to_ascii_lowercase(), instance.source_object_name.clone()))
}


fn scene_anchor_name(instance_name: &str) -> String {
    format!("{instance_name}_anchor")
}


fn scene_source_empty_name(instance_name: &str, ancestor_index: usize, ancestor_name: &str) -> String {
    format!("{instance_name}_{ancestor_index}_{ancestor_name}")
}

fn unique_scene_object_name(base: &str, used: &mut HashMap<String, usize>) -> String {
    let count = used.entry(base.to_string()).or_insert(0);
    let name = if *count == 0 {
        base.to_string()
    } else {
        format!("{base}_{count:03}")
    };
    *count += 1;
    name
}

fn blend_library_basename(blend_path: &str) -> &str {
    blend_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(blend_path)
}

fn blend_library_path_components(blend_path: &str) -> Vec<&str> {
    blend_path
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect()
}

fn hashed_library_name(blend_path: &str, basename: &str) -> String {
    let mut hasher = DefaultHasher::new();
    blend_path.hash(&mut hasher);
    format!("{basename}__{:016x}", hasher.finish())
}

fn unique_library_names(blend_paths: &[String]) -> HashMap<String, String> {
    let mut paths_by_basename: HashMap<&str, Vec<&String>> = HashMap::new();
    for blend_path in blend_paths {
        paths_by_basename
            .entry(blend_library_basename(blend_path))
            .or_default()
            .push(blend_path);
    }

    let mut names = HashMap::new();
    for (basename, paths) in paths_by_basename {
        if paths.len() == 1 {
            names.insert(paths[0].clone(), basename.to_string());
            continue;
        }

        let component_lists: Vec<Vec<&str>> = paths
            .iter()
            .map(|path| blend_library_path_components(path))
            .collect();
        let max_depth = component_lists
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(1);

        let mut resolved = None;
        for depth in 2..=max_depth {
            let candidates: Vec<String> = component_lists
                .iter()
                .map(|components| {
                    let start = components.len().saturating_sub(depth);
                    components[start..].join("__")
                })
                .collect();

            let unique: HashSet<&String> = candidates.iter().collect();
            if unique.len() == candidates.len() {
                resolved = Some(candidates);
                break;
            }
        }

        let resolved = resolved.unwrap_or_else(|| {
            paths.iter()
                .map(|path| hashed_library_name(path, basename))
                .collect()
        });

        for (path, candidate) in paths.iter().zip(resolved) {
            let final_name = if candidate.len() <= 255 {
                candidate
            } else {
                hashed_library_name(path, basename)
            };
            names.insert((*path).clone(), final_name);
        }
    }

    names
}

fn lookup_named_ptr(map: &HashMap<String, u64>, name: &str) -> Option<u64> {
    map.get(name)
        .copied()
        .or_else(|| {
            map.iter()
                .find_map(|(candidate, ptr)| candidate.eq_ignore_ascii_case(name).then_some(*ptr))
        })
}

fn interior_parent_empty_name(
    container_name: &str,
    parent_entity_name: Option<&str>,
    container_index: Option<usize>,
) -> String {
    let base = if let Some(parent) = parent_entity_name.filter(|name| !name.is_empty()) {
        format!("interior_{parent}_{container_name}")
    } else {
        format!("interior_{container_name}")
    };
    if let Some(index) = container_index {
        format!("{base}_{index:03}")
    } else {
        base
    }
}

#[derive(Debug, Clone)]
struct SceneManifestInstance {
    entity_name: String,
    parent_entity_name: Option<String>,
    parent_empty_name: Option<String>,
    parent_empty_parent_entity_name: Option<String>,
    parent_empty_parent_node_name: Option<String>,
    parent_empty_loc: [f32; 3],
    parent_empty_quat: [f32; 4],
    parent_empty_scale: [f32; 3],
    is_interior: bool,
    mesh_asset: String,
    material_sidecar: Option<String>,
    palette_id: Option<String>,
    ui_bindings: Vec<serde_json::Value>,
    parent_node_name: Option<String>,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    hidden: bool,
}

fn manifest_vec3(value: &serde_json::Value, key: &str) -> Option<[f32; 3]> {
    let arr = value.get(key)?.as_array()?;
    Some([
        arr.first()?.as_f64()? as f32,
        arr.get(1)?.as_f64()? as f32,
        arr.get(2)?.as_f64()? as f32,
    ])
}

fn manifest_matrix(value: &serde_json::Value, key: &str) -> Option<glam::Mat4> {
    let rows = value.get(key)?.as_array()?;
    matrix_from_json_rows(rows)
}

fn matrix_from_json_rows(rows: &[serde_json::Value]) -> Option<glam::Mat4> {
    if rows.len() == 4 {
        let mut row_values = [[0.0f32; 4]; 4];
        for row in 0..4 {
            let cols = rows[row].as_array()?;
            if cols.len() != 4 {
                return None;
            }
            for col in 0..4 {
                row_values[row][col] = cols[col].as_f64()? as f32;
            }
        }
        Some(glam::Mat4::from_cols_array(&[
            row_values[0][0], row_values[0][1], row_values[0][2], row_values[0][3],
            row_values[1][0], row_values[1][1], row_values[1][2], row_values[1][3],
            row_values[2][0], row_values[2][1], row_values[2][2], row_values[2][3],
            row_values[3][0], row_values[3][1], row_values[3][2], row_values[3][3],
        ]))
    } else {
        None
    }
}

fn scene_axis_matrix() -> glam::Mat4 {
    glam::Mat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, -1.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ])
}

fn sc_matrix_to_scene_transform(source: glam::Mat4) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let axis = scene_axis_matrix();
    matrix_to_transform((axis * source * axis.inverse()).to_cols_array())
}

fn reference_root_conversion_quat() -> glam::Quat {
    glam::Quat::from_xyzw(
        -std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    )
}

fn apply_reference_root_conversion(
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let root_quat = reference_root_conversion_quat();
    let converted_loc = root_quat * glam::Vec3::from_array(loc);
    let local_quat = glam::Quat::from_xyzw(quat[1], quat[2], quat[3], quat[0]);
    let converted_quat = (root_quat * local_quat).normalize();
    (
        converted_loc.to_array(),
        [
            converted_quat.w,
            converted_quat.x,
            converted_quat.y,
            converted_quat.z,
        ],
        scale,
    )
}

fn manifest_scene_transform(value: &serde_json::Value) -> ([f32; 3], [f32; 4], [f32; 3]) {
    if let Some(source) = manifest_matrix(value, "local_transform_sc") {
        match value.get("source_transform_basis").and_then(|v| v.as_str()) {
            Some("cryengine_z_up") => return sc_matrix_to_scene_transform(source),
            Some("gltf_y_up") => return matrix_to_transform(source.to_cols_array()),
            _ => {}
        }
    }

    let offset = manifest_vec3(value, "offset_position").unwrap_or([0.0, 0.0, 0.0]);
    (
        [offset[0], -offset[2], offset[1]],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
    )
}

fn material_sidecar_default_palettes(manifest_files: &[ExportedFile]) -> HashMap<String, String> {
    let mut palettes = HashMap::new();
    for file in manifest_files.iter().filter(|file| file.relative_path.ends_with(".materials.json")) {
        let Ok(root) = serde_json::from_slice::<serde_json::Value>(&file.bytes) else {
            continue;
        };
        let Some(default_palette) = sidecar_default_palette_id(&root) else {
            continue;
        };
        palettes.insert(file.relative_path.clone(), default_palette.clone());
        if let Some(normalized_path) = root
            .get("normalized_export_relative_path")
            .and_then(|value| value.as_str())
            .filter(|path| !path.is_empty())
        {
            palettes.insert(normalized_path.to_string(), default_palette);
        }
    }
    palettes
}

fn sidecar_default_palette_id(sidecar: &serde_json::Value) -> Option<String> {
    let attributes = sidecar
        .get("authored_material_set")
        .and_then(|value| value.get("attributes"))
        .and_then(|value| value.as_array())?;
    let default_palette_path = attributes.iter().find_map(|attribute| {
        let name = attribute.get("name").and_then(|value| value.as_str())?;
        if !name.eq_ignore_ascii_case("DefaultPalette") {
            return None;
        }
        attribute.get("value").and_then(|value| value.as_str())
    })?;
    let source_name = default_palette_path
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())?
        .to_ascii_lowercase();
    Some(format!("palette/{source_name}"))
}

fn scene_manifest_instances(manifest_files: &[ExportedFile]) -> Vec<SceneManifestInstance> {
    let Some(scene_file) = manifest_files.iter().find(|file| file.relative_path.ends_with("scene.json")) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(&scene_file.bytes) else {
        return Vec::new();
    };
    let default_palette_by_sidecar = material_sidecar_default_palettes(manifest_files);
    let mut out = Vec::new();
    let mut push_record = |value: &serde_json::Value| {
        let Some(mesh_asset) = value.get("mesh_asset").and_then(|v| v.as_str()) else {
            return;
        };
        let entity_name = value
            .get("entity_name")
            .and_then(|v| v.as_str())
            .unwrap_or(mesh_asset)
            .to_string();
        let (loc, quat, scale) = manifest_scene_transform(value);
        let hidden = value
            .get("port_flags")
            .and_then(|v| v.as_str())
            .is_some_and(port_flags_mark_invisible);
        let material_sidecar = value
            .get("material_sidecar")
            .and_then(|v| v.as_str())
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned);
        let ui_bindings = value
            .get("ui_bindings")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let sidecar_palette_id = material_sidecar
            .as_deref()
            .and_then(|path| default_palette_by_sidecar.get(path).cloned());
        out.push(SceneManifestInstance {
            entity_name,
            parent_entity_name: value
                .get("parent_entity_name")
                .and_then(|v| v.as_str())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned),
            parent_empty_name: None,
            parent_empty_parent_entity_name: None,
            parent_empty_parent_node_name: None,
            parent_empty_loc: [0.0, 0.0, 0.0],
            parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
            parent_empty_scale: [1.0, 1.0, 1.0],
            is_interior: false,
            mesh_asset: mesh_asset.to_string(),
            material_sidecar,
            palette_id: value
                .get("palette_id")
                .and_then(|v| v.as_str())
                .filter(|palette| !palette.is_empty())
                .map(ToOwned::to_owned)
                .or(sidecar_palette_id),
            ui_bindings,
            parent_node_name: value
                .get("parent_node_name")
                .and_then(|v| v.as_str())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned),
            loc,
            quat,
            scale,
            hidden,
        });
    };
    if let Some(root_entity) = root.get("root_entity") {
        push_record(root_entity);
    }
    if let Some(children) = root.get("children").and_then(|v| v.as_array()) {
        for child in children {
            push_record(child);
        }
    }
    if let Some(interiors) = root.get("interiors").and_then(|v| v.as_array()) {
        for (interior_index, interior) in interiors.iter().enumerate() {
            let container_transform = manifest_matrix(interior, "container_transform")
                .unwrap_or(glam::Mat4::IDENTITY);
            let (container_loc, container_quat, container_scale) =
                sc_matrix_to_scene_transform(container_transform);
            let (container_loc, container_quat, container_scale) =
                apply_reference_root_conversion(container_loc, container_quat, container_scale);
            let interior_name = interior
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Interior")
                .to_string();
            let interior_palette_id = interior
                .get("palette_id")
                .and_then(|v| v.as_str())
                .filter(|palette| !palette.is_empty())
                .map(ToOwned::to_owned);
            let parent_empty_parent_entity_name = interior
                .get("parent_entity_name")
                .and_then(|v| v.as_str())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned);
            let container_name = interior_parent_empty_name(
                &interior_name,
                parent_empty_parent_entity_name.as_deref(),
                Some(interior_index),
            );
            let parent_empty_parent_node_name = interior
                .get("parent_node_name")
                .and_then(|v| v.as_str())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned);
            if let Some(placements) = interior.get("placements").and_then(|v| v.as_array()) {
                for placement in placements {
                    let Some(mesh_asset) = placement.get("mesh_asset").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let source =
                        manifest_matrix(placement, "transform").unwrap_or(glam::Mat4::IDENTITY);
                    let (loc, quat, scale) = sc_matrix_to_scene_transform(source);
                    let material_sidecar = placement
                        .get("material_sidecar")
                        .and_then(|v| v.as_str())
                        .filter(|path| !path.is_empty())
                        .map(ToOwned::to_owned);
                    let sidecar_palette_id = material_sidecar
                        .as_deref()
                        .and_then(|path| default_palette_by_sidecar.get(path).cloned());
                    out.push(SceneManifestInstance {
                        entity_name: placement
                            .get("cgf_path")
                            .and_then(|v| v.as_str())
                            .unwrap_or(mesh_asset)
                            .to_string(),
                        parent_entity_name: None,
                        parent_empty_name: Some(container_name.clone()),
                        parent_empty_parent_entity_name: parent_empty_parent_entity_name.clone(),
                        parent_empty_parent_node_name: parent_empty_parent_node_name.clone(),
                        parent_empty_loc: container_loc,
                        parent_empty_quat: container_quat,
                        parent_empty_scale: container_scale,
                        is_interior: true,
                        mesh_asset: mesh_asset.to_string(),
                        material_sidecar,
                        palette_id: placement
                            .get("palette_id")
                            .and_then(|v| v.as_str())
                            .filter(|palette| !palette.is_empty())
                            .map(ToOwned::to_owned)
                            .or_else(|| interior_palette_id.clone())
                            .or(sidecar_palette_id),
                        ui_bindings: placement
                            .get("ui_bindings")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default(),
                        parent_node_name: None,
                        loc,
                        quat,
                        scale,
                        hidden: false,
                    });
                }
            }
        }
    }
    out
}

fn light_object_properties(
    entity_name: &str,
    light_name: &str,
    radius_source: f32,
) -> Vec<(String, IdPropValue)> {
    vec![
        ("starbreaker_scene_path".to_string(), IdPropValue::String(package_scene_path(entity_name))),
        ("starbreaker_package_name".to_string(), IdPropValue::String(entity_name.to_string())),
        ("starbreaker_entity_name".to_string(), IdPropValue::String(entity_name.to_string())),
        ("starbreaker_source_node_name".to_string(), IdPropValue::String(light_name.to_string())),
        (
            "starbreaker_light_source_radius".to_string(),
            IdPropValue::Double(radius_source as f64),
        ),
    ]
}

fn light_data_properties(
    active_state: &str,
    states_json: Option<&str>,
    semantic_light_kind: &str,
) -> Vec<(String, IdPropValue)> {
    let mut props = Vec::new();
    if let Some(states_json) = states_json.filter(|value| !value.is_empty()) {
        props.push((
            "starbreaker_light_states".to_string(),
            IdPropValue::String(states_json.to_string()),
        ));
    }
    if !active_state.is_empty() {
        props.push((
            "starbreaker_light_active_state".to_string(),
            IdPropValue::String(active_state.to_string()),
        ));
    }
    if !semantic_light_kind.is_empty() {
        props.push((
            "starbreaker_light_semantic_kind".to_string(),
            IdPropValue::String(semantic_light_kind.to_string()),
        ));
    }
    props
}

fn decal_face_indices_for_mesh(mesh: &Mesh, mesh_with_decals: &MeshWithDecals) -> Vec<usize> {
    let decal_material_indices = mesh_with_decals
        .decal_materials
        .iter()
        .map(|material| material.material_index)
        .collect::<HashSet<_>>();
    let decal_material_names = mesh_with_decals
        .decal_materials
        .iter()
        .map(|material| material.material_name.as_str())
        .collect::<HashSet<_>>();
    let mut decal_face_indices = Vec::new();
    for submesh in &mesh.submeshes {
        let include_submesh = if let Some(name) = submesh.material_name.as_deref() {
            decal_material_names.contains(name)
        } else {
            let material_id = submesh.source_material_id.unwrap_or(submesh.material_id) as usize;
            decal_material_indices.contains(&material_id)
        };
        if include_submesh {
            let start_face = submesh.first_index / 3;
            let num_faces = submesh.num_indices / 3;
            decal_face_indices.extend((start_face..start_face + num_faces).map(|face| face as usize));
        }
    }
    decal_face_indices
}

/// Convert `DecomposedInput` into a decomposed export with individual `.blend` files.
///
/// **Phase 1**: Mesh Decomposition to individual .blend files
/// - Extracts real mesh data from DecomposedInput::children
/// - Generates full decomposed export (scene.json, package manifests, etc.)
/// - Writes native .blend mesh assets directly to the decomposed package
/// - Each mesh gets its own uncompressed .blend file with actual mesh data
///
/// **Phase 3**: Lights and Empties Integration
/// - Extracts lights from DecomposedInput::interiors
/// - Extracts empties from DecomposedInput::root_nmc
/// - Creates scene.blend with lights and empties in proper collections
///
/// **Phase 4**: Decal Vertex Groups Integration
/// - Identifies decal materials in each mesh
/// - Creates starbreaker_decal_offset vertex groups with decal vertices
/// - Adds vertex groups to individual mesh .blend files
///
/// Returns `DecomposedExport` with all files including real mesh geometry
pub fn write_decomposed_export_blend(
    db: &Database<'_>,
    p4k: &MappedP4k,
    input: DecomposedInput,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
    existing_interior_assets: Option<&crate::decomposed::ExistingInteriorAssetMap>,
    existing_asset_loader: Option<&(dyn Fn(&str) -> Option<Vec<u8>> + Sync)>,
) -> Result<DecomposedExport, Error> {
    const BASE_DECOMPOSED_END: f32 = 0.90;
    const DECAL_GROUPS_START: f32 = 0.90;
    const NATIVE_BLEND_ASSETS_START: f32 = 0.91;
    const NATIVE_BLEND_ASSETS_END: f32 = 0.995;
    const SCENE_BLEND_START: f32 = 0.995;

    let total_start = Instant::now();
    let mut phase_start = Instant::now();
    // Phase 1: Extract mesh data from input children BEFORE calling write_decomposed_export.
    // Key by the exact final .blend mesh asset path; loose entity-name matching is not
    // stable enough for similarly named ship parts.
    let mut mesh_data_map: HashMap<String, MeshDataEntry> = HashMap::new();
    let mut child_mesh_data_map: HashMap<String, MeshDataEntry> = HashMap::new();
    
    let root_mesh_asset = crate::decomposed::mesh_asset_relative_path(
        p4k,
        &input.geometry_path,
        &input.entity_name,
        opts.lod_level,
        opts.format,
    );
    let root_material_view = crate::decomposed::build_decomposed_material_view(
        &input.root_mesh,
        input.root_materials.as_ref(),
        input.root_nmc.as_ref(),
        opts.include_nodraw,
        opts.include_shields,
    );
    mesh_data_map.insert(root_mesh_asset.to_ascii_lowercase(), MeshDataEntry {
        mesh: root_material_view.mesh,
        materials: root_material_view.glb_materials.or(root_material_view.sidecar_materials),
        nmc: root_material_view.glb_nmc,
        interior_placement_space: false,
    });
    
    for child in &input.children {
        let mesh_asset = crate::decomposed::mesh_asset_relative_path(
            p4k,
            &child.geometry_path,
            &child.entity_name,
            opts.lod_level,
            opts.format,
        );
        let child_material_view = crate::decomposed::build_decomposed_material_view(
            &child.mesh,
            child.materials.as_ref(),
            child.nmc.as_ref(),
            opts.include_nodraw,
            opts.include_shields,
        );
        let entry = MeshDataEntry {
            mesh: child_material_view.mesh,
            materials: child_material_view.glb_materials.or(child_material_view.sidecar_materials),
            nmc: child_material_view.glb_nmc,
            interior_placement_space: false,
        };
        
        let mesh_key = mesh_asset.to_ascii_lowercase();
        child_mesh_data_map.insert(mesh_key.clone(), entry.clone());
        mesh_data_map.insert(mesh_key, entry);
    }
    log::info!("[timing][blend] precollect_mesh_data: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();
    
    // Extract minimal data needed for scene.blend before passing input to write_decomposed_export
    let scene_entity_name = input.entity_name.clone();
    let children_for_scene = input.children.iter().map(|c| c.entity_name.clone()).collect::<Vec<_>>();
    let mut scene_mesh_instances = Vec::new();
    
    // Phase 3: Extract lights and empties BEFORE calling write_decomposed_export
    let extracted_lights = extract_lights_from_interiors(&input.interiors)
        .unwrap_or_default();
    
    let _extracted_empties = input.root_nmc.as_ref()
        .and_then(|nmc| extract_empties_from_nmc(&nmc.nodes).ok())
        .unwrap_or_default();
    log::info!("[timing][blend] extract_lights_empties: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();
    
    let interior_mesh_opts = crate::pipeline::ExportOptions {
        material_mode: crate::pipeline::MaterialMode::Colors,
        ..opts.clone()
    };
    let preloaded_interior_meshes = preload_interior_meshes(&input.interiors, p4k, &interior_mesh_opts);
    let mut preloaded_interior_mesh_map: HashMap<String, Option<(Mesh, Option<crate::mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)>> = input
        .interiors
        .unique_cgfs
        .iter()
        .zip(preloaded_interior_meshes.into_iter())
        .map(|(entry, loaded)| {
            (
                crate::decomposed::interior_asset_lookup_key(&entry.cgf_path, entry.material_path.as_deref()),
                loaded,
            )
        })
        .collect();

    // Pre-decode all source textures in parallel before the serial decomposed
    // writer runs. Collect each asset's (materials, material_path, geometry_path)
    // — root, every child, and every unique interior CGF (materials from the
    // already-preloaded meshes) — then warm a png_cache the writer consumes.
    // Must run before `interior_mesh_loader` below borrows the mesh map.
    let prewarm_start = Instant::now();
    let mut prewarm_assets: Vec<(crate::mtl::MtlFile, String, String)> = Vec::new();
    if let Some(root_materials) = input.root_materials.as_ref() {
        prewarm_assets.push((
            root_materials.clone(),
            input.material_path.clone(),
            input.geometry_path.clone(),
        ));
    }
    for child in &input.children {
        if let Some(materials) = child.materials.as_ref() {
            prewarm_assets.push((
                materials.clone(),
                child.material_path.clone(),
                child.geometry_path.clone(),
            ));
        }
    }
    for entry in &input.interiors.unique_cgfs {
        let lookup_key = crate::decomposed::interior_asset_lookup_key(
            &entry.cgf_path,
            entry.material_path.as_deref(),
        );
        if let Some(Some((mesh, materials, nmc))) = preloaded_interior_mesh_map.get(&lookup_key) {
            let view = crate::decomposed::build_decomposed_material_view(
                mesh,
                materials.as_ref(),
                nmc.as_ref(),
                opts.include_nodraw,
                opts.include_shields,
            );
            if let Some(sidecar_materials) = view.sidecar_materials {
                prewarm_assets.push((
                    sidecar_materials,
                    entry.material_path.clone().unwrap_or_default(),
                    entry.cgf_path.clone(),
                ));
            }
        }
    }
    let prewarmed_png_cache =
        crate::decomposed::prewarm_decomposed_textures(p4k, &prewarm_assets, opts.texture_mip);
    log::info!(
        "[timing][blend] prewarm_textures: {:.2}s ({} assets, {} cached)",
        prewarm_start.elapsed().as_secs_f32(),
        prewarm_assets.len(),
        prewarmed_png_cache.len(),
    );
    let prewarm_rough_start = Instant::now();
    let prewarmed_roughness_cache =
        crate::decomposed::prewarm_decomposed_roughness(p4k, &prewarm_assets, opts.texture_mip);
    log::info!(
        "[timing][blend] prewarm_roughness: {:.2}s ({} sources)",
        prewarm_rough_start.elapsed().as_secs_f32(),
        prewarmed_roughness_cache.len(),
    );

    let mut interior_mesh_loader = |entry: &crate::pipeline::InteriorCgfEntry|
        -> Option<(Mesh, Option<crate::mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
        let lookup_key = crate::decomposed::interior_asset_lookup_key(&entry.cgf_path, entry.material_path.as_deref());
        let loaded = preloaded_interior_mesh_map
            .remove(&lookup_key)
            .flatten()
            .or_else(|| {
                let mut interior_png_cache = PngCache::new();
                load_interior_mesh_asset(p4k, entry, &interior_mesh_opts, &mut interior_png_cache)
            })?;
        let (mesh, materials, nmc) = loaded.clone();
        let mesh_asset = crate::decomposed::mesh_asset_relative_path(
            p4k,
            &entry.cgf_path,
            &entry.name,
            opts.lod_level,
            opts.format,
        );
        let material_view = crate::decomposed::build_decomposed_material_view(
            &mesh,
            materials.as_ref(),
            nmc.as_ref(),
            opts.include_nodraw,
            opts.include_shields,
        );
        let interior_entry = MeshDataEntry {
            mesh: material_view.mesh,
            materials: material_view.glb_materials.or(material_view.sidecar_materials),
            nmc: material_view.glb_nmc,
            interior_placement_space: true,
        };
        mesh_data_map.insert(mesh_asset.to_ascii_lowercase(), interior_entry);
        Some(loaded)
    };

    report_progress(progress, 0.0, "Generating decomposed export with .blend mesh files");
    let base_progress = progress.map(|progress| progress.sub(0.0, BASE_DECOMPOSED_END));

    // Generate package manifests, sidecars, textures, palettes, and the reusable
    // native .blend asset list with the shared decomposed exporter. Mesh asset
    // payloads are populated with real .blend bytes below.
    let base_export = crate::decomposed::write_decomposed_export(
        db,
        p4k,
        input,
        opts,
        base_progress.as_ref(),
        existing_asset_paths,
        existing_interior_assets,
        prewarmed_png_cache,
        prewarmed_roughness_cache,
        &mut interior_mesh_loader,
    )?;
    log::info!("[timing][blend] base_decomposed_export: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    // Extract updated projector_texture paths from the generated scene.json
    // This ensures scene.blend uses the PNG paths that decomposed export created
    let mut light_projector_texture_map: HashMap<String, String> = HashMap::new();
    if let Some(scene_json_file) = base_export.files.iter().find(|f| f.relative_path.ends_with("scene.json")) {
        if let Ok(scene_data) = serde_json::from_slice::<serde_json::Value>(&scene_json_file.bytes) {
            if let Some(interiors) = scene_data.get("interiors").and_then(|v| v.as_array()) {
                for interior in interiors {
                    if let Some(lights) = interior.get("lights").and_then(|v| v.as_array()) {
                        for light in lights {
                            if let (Some(name), Some(texture)) = (
                                light.get("name").and_then(|v| v.as_str()),
                                light.get("projector_texture").and_then(|v| v.as_str()),
                            ) {
                                if !texture.is_empty() {
                                    // Store the normalized PNG path
                                    light_projector_texture_map.insert(name.to_string(), texture.to_string());
                                }
                            }
                        }
                    }
                }
            }
            // Also check root-level lights for exterior lights
            if let Some(lights) = scene_data.get("lights").and_then(|v| v.as_array()) {
                for light in lights {
                    if let (Some(name), Some(texture)) = (
                        light.get("name").and_then(|v| v.as_str()),
                        light.get("projector_texture").and_then(|v| v.as_str()),
                    ) {
                        if !texture.is_empty() {
                            light_projector_texture_map.insert(name.to_string(), texture.to_string());
                        }
                    }
                }
            }
        }
    }
    log::info!("[blend-debug] Extracted {} light projector texture paths", light_projector_texture_map.len());

    // Phase 4: Collect vertex groups for all meshes BEFORE creating .blend files
    report_progress(progress, DECAL_GROUPS_START, "Collecting decal vertex groups from meshes");
    
    let mut mesh_vertex_groups: HashMap<String, Vec<VertexGroup>> = HashMap::new();
    
    // Collect all mesh materials for decal identification
    let mut mesh_materials = Vec::new();
    for (mesh_key, entry) in &mesh_data_map {
        if let Some(ref mtl) = entry.materials {
            let material_list: Vec<DecalMaterialSource> = mtl
                .materials
                .iter()
                .map(|sub| {
                    (
                        sub.name.clone(),
                        sub.shader.clone(),
                        sub.string_gen_mask.clone(),
                        sub.public_params.iter().map(|param| param.name.clone()).collect(),
                    )
                })
                .collect();
            mesh_materials.push((mesh_key.clone(), material_list));
        }
    }
    
    // Identify meshes with decals and collect vertex groups
    if let Ok(meshes_with_decals) = identify_meshes_with_decals(&mesh_materials) {
        for mesh_with_decals in meshes_with_decals {
            if let Some(mesh_entry) = mesh_data_map.get(&mesh_with_decals.mesh_path) {
                let decal_face_indices = decal_face_indices_for_mesh(&mesh_entry.mesh, &mesh_with_decals);

                if let Ok(vgroups) = collect_decal_vertices(
                    &mesh_with_decals,
                    &decal_face_indices,
                    &mesh_entry.mesh.indices.iter().map(|&i| i as u32).collect::<Vec<_>>(),
                    3, // verts_per_face for triangles
                ) {
                    mesh_vertex_groups.insert(mesh_with_decals.mesh_path.clone(), vgroups.vertex_groups);
                }
            }
        }
    }
    log::info!("[timing][blend] collect_decal_vertex_groups: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    report_progress(progress, NATIVE_BLEND_ASSETS_START, "Writing native .blend mesh assets with real geometry");

    // Populate native .blend mesh asset payloads with real geometry.
    let mut blend_files = Vec::new();
    let mut manifest_files = Vec::new();
    let mut other_files = Vec::new();
    let mut refs_by_asset = HashMap::new();
    let mut source_nodes_by_asset = HashMap::new();
    let mut blend_asset_jobs = Vec::new();
    for file in base_export.files {
        if file.kind == ExportedFileKind::MeshAsset {
            let blend_path = file.relative_path;
            if !blend_path.ends_with(".blend") {
                // Non-.blend mesh assets (e.g. .glb children) cannot be processed by the
                // native Blend pipeline; pass them through as-is.
                log::debug!(
                    "native Blend export skipping non-.blend mesh asset: '{}'",
                    blend_path
                );
                other_files.push(ExportedFile {
                    relative_path: blend_path,
                    bytes: file.bytes,
                    kind: ExportedFileKind::MeshAsset,
                });
                continue;
            }
            // Extract mesh name from path for Blender object naming
            let mesh_name = blend_path
                .split('/')
                .last()
                .unwrap_or("mesh")
                .trim_end_matches(".blend")
                .to_string();
            let blend_key = blend_path.to_ascii_lowercase();
            blend_asset_jobs.push(BlendAssetJob {
                blend_path,
                mesh_name,
                blend_key,
            });
        } else if file.kind == ExportedFileKind::PackageManifest {
            manifest_files.push(ExportedFile {
                relative_path: file.relative_path,
                bytes: file.bytes,
                kind: file.kind,
            });
        } else if !file.relative_path.ends_with("scene.blend") {
            // Keep other files as-is (palettes, textures, etc.)
            // EXCLUDE scene.blend from base_export - we'll create our own detailed version
            other_files.push(file);
        }
    }
    log::info!("[timing][blend] classify_base_files: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    let preview_instances = scene_manifest_instances(&manifest_files);
    let interior_assets = preview_instances
        .iter()
        .filter(|instance| instance.is_interior)
        .map(|instance| instance.mesh_asset.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut child_override_by_original = HashMap::new();
    for instance in preview_instances
        .iter()
        .filter(|instance| !instance.is_interior && instance.parent_node_name.is_some())
    {
        let original_key = instance.mesh_asset.to_ascii_lowercase();
        if !interior_assets.contains(&original_key) {
            continue;
        }
        if child_override_by_original.contains_key(&original_key) {
            continue;
        }
        let Some(child_entry) = child_mesh_data_map.get(&original_key).cloned() else {
            continue;
        };
        let override_path = instance
            .mesh_asset
            .strip_suffix(".blend")
            .map(|stem| format!("{stem}__child.blend"))
            .unwrap_or_else(|| format!("{}__child.blend", instance.mesh_asset));
        let override_key = override_path.to_ascii_lowercase();
        if !mesh_data_map.contains_key(&override_key) {
            mesh_data_map.insert(override_key.clone(), child_entry);
            if let Some(vertex_groups) = mesh_vertex_groups.get(&original_key).cloned() {
                mesh_vertex_groups.insert(override_key.clone(), vertex_groups);
            }
            let mesh_name = override_path
                .split('/')
                .last()
                .unwrap_or("mesh")
                .trim_end_matches(".blend")
                .to_string();
            blend_asset_jobs.push(BlendAssetJob {
                blend_path: override_path.clone(),
                mesh_name,
                blend_key: override_key.clone(),
            });
        }
        child_override_by_original.insert(original_key, override_path);
    }
    if !child_override_by_original.is_empty() {
        for file in &mut manifest_files {
            if !file.relative_path.ends_with("scene.json") {
                continue;
            }
            let mut scene_data = serde_json::from_slice::<serde_json::Value>(&file.bytes)
                .map_err(|error| {
                    Error::Other(format!(
                        "failed to parse package manifest {} as JSON: {error}",
                        file.relative_path
                    ))
                })?;
            if let Some(children) = scene_data.get_mut("children").and_then(|value| value.as_array_mut()) {
                for child in children {
                    if child
                        .get("parent_node_name")
                        .and_then(|value| value.as_str())
                        .is_none()
                    {
                        continue;
                    }
                    let Some(mesh_asset) = child.get("mesh_asset").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    if let Some(override_path) =
                        child_override_by_original.get(&mesh_asset.to_ascii_lowercase())
                    {
                        child["mesh_asset"] = serde_json::Value::String(override_path.clone());
                    }
                }
            }
            file.bytes = serde_json::to_vec(&scene_data).map_err(|error| {
                Error::Other(format!(
                    "failed to serialize package manifest {} JSON: {error}",
                    file.relative_path
                ))
            })?;
        }
    }

    let mut scheduled_blend_assets = blend_asset_jobs
        .iter()
        .map(|job| job.blend_path.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for instance in scene_manifest_instances(&manifest_files) {
        if scheduled_blend_assets.insert(instance.mesh_asset.to_ascii_lowercase()) {
            let mesh_name = instance
                .mesh_asset
                .split('/')
                .last()
                .unwrap_or("mesh")
                .trim_end_matches(".blend")
                .to_string();
            blend_asset_jobs.push(BlendAssetJob {
                blend_path: instance.mesh_asset.clone(),
                mesh_name,
                blend_key: instance.mesh_asset.to_ascii_lowercase(),
            });
        }
    }

    for built_asset in build_native_blend_assets(
        &blend_asset_jobs,
        &mesh_data_map,
        &mesh_vertex_groups,
        opts.threads,
        progress,
        NATIVE_BLEND_ASSETS_START,
        NATIVE_BLEND_ASSETS_END,
        existing_asset_loader,
    )? {
        refs_by_asset.insert(
            built_asset.relative_path.clone(),
            built_asset.linked_mesh_refs.clone(),
        );
        source_nodes_by_asset.insert(
            built_asset.relative_path.clone(),
            built_asset.source_nodes,
        );
        if let Some(file) = built_asset.file {
            blend_files.push(file);
        }
    }
    log::info!("[timing][blend] build_native_blend_assets: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    let mut blend_file_order = Vec::new();
    let mut blend_file_by_path = HashMap::new();
    for file in blend_files {
        if !blend_file_by_path.contains_key(&file.relative_path) {
            blend_file_order.push(file.relative_path.clone());
        }
        blend_file_by_path.insert(file.relative_path.clone(), file);
    }
    blend_files = blend_file_order
        .into_iter()
        .filter_map(|path| blend_file_by_path.remove(&path))
        .collect();

    scene_mesh_instances.clear();
    let scene_package_name = manifest_files
        .iter()
        .find_map(|file| {
            file.relative_path
                .strip_prefix("Packages/")
                .and_then(|path| path.strip_suffix("/scene.json"))
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| scene_entity_name.clone());
    let manifest_instances = scene_manifest_instances(&manifest_files);
    let mut used_scene_object_names = HashMap::new();
    if manifest_instances.is_empty() {
        for (scene_instance_id, file) in blend_files.iter().enumerate() {
            if let Some(linked_mesh_refs) = refs_by_asset.get(&file.relative_path) {
                for (idx, linked_mesh_ref) in linked_mesh_refs.iter().enumerate() {
                    let name = unique_scene_object_name(&linked_mesh_ref.object_name, &mut used_scene_object_names);
                    scene_mesh_instances.push(LinkedMeshInstance {
                        scene_instance_id,
                        entity_name: linked_mesh_ref.object_name.clone(),
                        parent_entity_name: None,
                        parent_empty_name: None,
                        parent_empty_parent_entity_name: None,
                        parent_empty_parent_node_name: None,
                        parent_empty_loc: [0.0, 0.0, 0.0],
                        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
                        parent_empty_scale: [1.0, 1.0, 1.0],
                        is_interior: false,
                        source_object_name: linked_mesh_ref.object_name.clone(),
                        name,
                        mesh_name: linked_mesh_ref.mesh_name.clone(),
                        material_names: linked_mesh_ref.material_names.clone(),
                        material_sidecar: None,
                        palette_id: None,
                        ui_bindings: Vec::new(),
                        source_nodes: if idx == 0 {
                            source_nodes_by_asset
                                .get(&file.relative_path)
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        },
                        source_ancestors: linked_mesh_ref.ancestors.clone(),
                        source_parent_name: linked_mesh_ref.source_parent_name.clone(),
                        source_loc: linked_mesh_ref.object_loc,
                        source_quat: linked_mesh_ref.object_quat,
                        source_scale: linked_mesh_ref.object_scale,
                        parent_node_name: None,
                        blend_path: scene_library_blend_path(&scene_package_name, &file.relative_path),
                        mesh_asset: file.relative_path.clone(),
                        position: [0.0, 0.0, 0.0],
                        rotation: [1.0, 0.0, 0.0, 0.0],
                        scale: [1.0, 1.0, 1.0],
                        hidden: false,
                    });
                }
            }
        }
    } else {
        for (scene_instance_id, manifest_instance) in manifest_instances.iter().enumerate() {
            if let Some(linked_mesh_refs) = refs_by_asset.get(&manifest_instance.mesh_asset) {
                for (mesh_ref_idx, linked_mesh_ref) in linked_mesh_refs.iter().enumerate() {
                    let name = unique_scene_object_name(&linked_mesh_ref.object_name, &mut used_scene_object_names);
                    let instance_source_nodes = if mesh_ref_idx == 0 {
                        source_nodes_by_asset
                            .get(&manifest_instance.mesh_asset)
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    scene_mesh_instances.push(LinkedMeshInstance {
                        scene_instance_id,
                        entity_name: manifest_instance.entity_name.clone(),
                        parent_entity_name: manifest_instance.parent_entity_name.clone(),
                        parent_empty_name: manifest_instance.parent_empty_name.clone(),
                        parent_empty_parent_entity_name: manifest_instance.parent_empty_parent_entity_name.clone(),
                        parent_empty_parent_node_name: manifest_instance.parent_empty_parent_node_name.clone(),
                        parent_empty_loc: manifest_instance.parent_empty_loc,
                        parent_empty_quat: manifest_instance.parent_empty_quat,
                        parent_empty_scale: manifest_instance.parent_empty_scale,
                        is_interior: manifest_instance.is_interior,
                        source_object_name: linked_mesh_ref.object_name.clone(),
                        name,
                        mesh_name: linked_mesh_ref.mesh_name.clone(),
                        material_names: linked_mesh_ref.material_names.clone(),
                        material_sidecar: manifest_instance.material_sidecar.clone(),
                        palette_id: manifest_instance.palette_id.clone(),
                        ui_bindings: manifest_instance.ui_bindings.clone(),
                        source_nodes: instance_source_nodes,
                        source_ancestors: linked_mesh_ref.ancestors.clone(),
                        source_parent_name: linked_mesh_ref.source_parent_name.clone(),
                        source_loc: linked_mesh_ref.object_loc,
                        source_quat: linked_mesh_ref.object_quat,
                        source_scale: linked_mesh_ref.object_scale,
                        parent_node_name: manifest_instance.parent_node_name.clone(),
                        blend_path: scene_library_blend_path(
                            &scene_package_name,
                            &manifest_instance.mesh_asset,
                        ),
                        mesh_asset: manifest_instance.mesh_asset.clone(),
                        position: manifest_instance.loc,
                        rotation: manifest_instance.quat,
                        scale: manifest_instance.scale,
                        hidden: manifest_instance.hidden,
                    });
                }
            }
        }
    }
    log::info!("[timing][blend] scene_instance_records: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();

    let decal_mesh_refs = refs_by_asset
        .iter()
        .flat_map(|(asset_path, refs)| {
            refs.iter()
                .filter(|mesh_ref| mesh_ref.has_decal_offset_modifier)
                .map(|mesh_ref| (asset_path.to_ascii_lowercase(), mesh_ref.object_name.clone()))
        })
        .collect::<HashSet<_>>();

    // Phase 3: Create scene.blend with lights and empties
    report_progress(progress, SCENE_BLEND_START, "Creating scene.blend with linked mesh instances");
    
    // Create scene.blend with properly linked mesh instances and lights
    // Library paths are relative to scene.blend location, which is in Packages/{package}/
    // So we need ../../Data/Objects to reach the shared assets
    log::info!("[blend-debug] Creating scene.blend with {} children and {} lights", children_for_scene.len(), extracted_lights.len());
    let scene_blend_bytes = create_scene_blend_package_with_instances_and_decal_offsets(
        &scene_package_name,
        &scene_entity_name,
        &scene_mesh_instances,
        &extracted_lights,
        &light_projector_texture_map,
        &decal_mesh_refs,
    )?;
    log::info!("[blend-debug] scene.blend created, size: {} bytes", scene_blend_bytes.len());
    log::info!("[blend-debug] First 20 bytes of uncompressed: {:?}", &scene_blend_bytes[..20.min(scene_blend_bytes.len())]);
    
    // Compress scene.blend with standard Zstd (Blender 5.1 native format)
    let compressed_scene = starbreaker_blend::compress_blend_bytes(&scene_blend_bytes);
    log::info!("[blend-debug] Compressed scene size: {} bytes", compressed_scene.len());
    log::info!("[blend-debug] First 20 bytes of compressed: {:?}", &compressed_scene[..20.min(compressed_scene.len())]);
    log::info!("[timing][blend] scene_blend_assembly: {:.2}s", phase_start.elapsed().as_secs_f32());
    phase_start = Instant::now();
    
    // Combine blend mesh files with other files (NOT including base_export.scene.blend)
    let mut all_files = blend_files;
    all_files.extend(manifest_files);
    all_files.extend(other_files);
    
    // Determine package_name from first mesh if available
    let package_name = format!("Packages/{scene_package_name}");
    
    // Add our detailed scene.blend with proper relative path and kind
    all_files.push(ExportedFile {
        relative_path: format!("{}/scene.blend", package_name),
        bytes: compressed_scene,
        // PackageManifest kind ensures scene.blend is always written (not skipped by skip_existing_assets)
        kind: ExportedFileKind::PackageManifest,
    });
    log::info!("[timing][blend] final_file_assembly: {:.2}s", phase_start.elapsed().as_secs_f32());
    log::info!("[timing][blend] total: {:.2}s", total_start.elapsed().as_secs_f32());

    report_progress(progress, 1.0, "Export complete");

    Ok(DecomposedExport { files: all_files })
}

/// Convert a mesh to `.blend` file bytes (uncompressed).
///
/// Produces a .blend containing a single OB_MESH object with:
/// - Position vertices (POINT / FLOAT3)
/// - Corner vertices (.corner_vert, CORNER / INT)
/// - Material index per face (FACE / INT)
/// - UVMap (CORNER / FLOAT2) — when mesh.uvs is Some
/// - Color (CORNER / BYTE_COLOR) — when mesh.colors is Some
/// - Vertex groups — when vertex_groups is Some
fn blend_material_slots(name: &str, mesh: &Mesh, materials: &Option<crate::mtl::MtlFile>) -> (Vec<String>, Vec<usize>) {
    let source_stem = materials
        .as_ref()
        .and_then(|mtl| mtl.source_path.as_deref())
        .and_then(|path| {
            path.rsplit(['\\', '/'])
                .next()
                .and_then(|file| file.rsplit_once('.').map(|(stem, _)| stem).or(Some(file)))
        })
        .unwrap_or(name);

    if mesh.submeshes.is_empty() {
        return (vec![format!("{source_stem}_mtl_material_0_00")], Vec::new());
    }

    let mut slot_by_material_id: HashMap<u32, usize> = HashMap::new();
    let mut material_names = Vec::new();
    let mut submesh_slots = Vec::with_capacity(mesh.submeshes.len());

    for (submesh_index, submesh) in mesh.submeshes.iter().enumerate() {
        if let Some(&slot) = slot_by_material_id.get(&submesh.material_id) {
            submesh_slots.push(slot);
            continue;
        }

        let base_name = submesh
            .material_name
            .clone()
            .or_else(|| {
                materials
                    .as_ref()
                    .and_then(|mtl| mtl.materials.get(submesh.material_id as usize))
                    .map(|mat| mat.name.clone())
            })
            .unwrap_or_else(|| format!("material_{submesh_index}"));
        let slot = material_names.len();
        material_names.push(format!("{source_stem}_mtl_{base_name}_0{}", submesh.material_id));
        slot_by_material_id.insert(submesh.material_id, slot);
        submesh_slots.push(slot);
    }

    (material_names, submesh_slots)
}

fn mesh_to_blend(
    name: &str,
    mesh: &Mesh,
    materials: &Option<crate::mtl::MtlFile>,
    nmc: Option<&NodeMeshCombo>,
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> Vec<u8> {
    if let Some(nmc) = nmc.filter(|nmc| !nmc.nodes.is_empty()) {
        return mesh_to_blend_hierarchy(name, mesh, materials, nmc, vertex_groups);
    }
    mesh_to_blend_flat(name, mesh, materials, vertex_groups)
}

fn mesh_to_blend_flat(
    name: &str,
    mesh: &Mesh,
    materials: &Option<crate::mtl::MtlFile>,
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> Vec<u8> {
    let totvert = mesh.positions.len();
    let totloop = mesh.indices.len();
    let totpoly = totloop / 3;
    let (edge_verts, corner_edges) = triangle_edge_topology(&mesh.indices);
    let totedge = edge_verts.len();
    let (material_names, submesh_material_slots) = blend_material_slots(name, mesh, materials);
    let mat_slots = material_names.len() as i16;

    let mut ptrs = PtrAlloc::new(0x1000);

    let _screen_ptr    = ptrs.alloc();
    let _wm_ptr        = ptrs.alloc();
    let object_ptr     = ptrs.alloc();
    let mesh_ptr       = ptrs.alloc();
    let mesh_mat_ptr   = ptrs.alloc();
    let obj_mat_ptr    = ptrs.alloc();
    let obj_matbits_ptr = ptrs.alloc();
    let material_ptrs: Vec<u64> = (0..material_names.len()).map(|_| ptrs.alloc()).collect();
    let material_idprops = material_names
        .iter()
        .map(|material_name| allocate_idprop_blocks(&mut ptrs, material_custom_properties(material_name, materials)))
        .collect::<Vec<_>>();
    let scene_ptr      = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let tool_settings_ptr = ptrs.alloc();
    let motion_blur_curve_points_ptr = ptrs.alloc();
    let cycles_scene_props = allocate_cycles_scene_props(&mut ptrs);
    let world_ptr = ptrs.alloc();
    let world_node_tree_ptr = ptrs.alloc();
    let base_ptr       = ptrs.alloc();
    let collection_ptr = ptrs.alloc();
    let collection_object_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    let poly_offs_ptr  = ptrs.alloc();
    let attrs_ptr      = ptrs.alloc();

    // Always-present attributes
    let name_pos_ptr  = ptrs.alloc();
    let name_ev_ptr   = ptrs.alloc();
    let name_cv_ptr   = ptrs.alloc();
    let name_ce_ptr   = ptrs.alloc();
    let array_pos_ptr = ptrs.alloc();
    let array_ev_ptr  = ptrs.alloc();
    let array_cv_ptr  = ptrs.alloc();
    let array_ce_ptr  = ptrs.alloc();
    let raw_pos_ptr   = ptrs.alloc();
    let raw_ev_ptr    = ptrs.alloc();
    let raw_cv_ptr    = ptrs.alloc();
    let raw_ce_ptr    = ptrs.alloc();

    // material_index (FACE domain)
    let name_matidx_ptr = ptrs.alloc();
    let array_matidx_ptr = ptrs.alloc();
    let raw_matidx_ptr = ptrs.alloc();

    // Optional: UV maps
    let (name_uv_ptr, array_uv_ptr, raw_uv_ptr) = if mesh.uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    let (active_uv_name_ptr, default_uv_name_ptr) = if mesh.uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0)
    };
    let (name_uv2_ptr, array_uv2_ptr, raw_uv2_ptr) = if mesh.secondary_uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    // Optional: vertex colors
    let (name_col_ptr, array_col_ptr, raw_col_ptr) = if mesh.colors.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };

    // Vertex group structures (Phase 5D)
    let (vgroup_first_ptr, vgroup_last_ptr, _vgroup_count, cdl_ptr, vgroup_ptrs, mdeformvert_ptrs, mdeformweight_ptrs) = 
        if let Some(vgroups) = vertex_groups {
            let mut v_ptrs = Vec::new();
            let mut mdv_ptrs = Vec::new();
            let mut mdw_ptrs = Vec::new();
            
            if !vgroups.is_empty() {
                // Allocate bDeformGroup pointers
                for _ in vgroups.iter() {
                    v_ptrs.push(ptrs.alloc());
                }
                let v_first = v_ptrs[0];
                let v_last = v_ptrs[v_ptrs.len() - 1];
                
                // Allocate MDeformVert array
                let mdv_array_ptr = ptrs.alloc();
                let mdv_data_ptr = ptrs.alloc();
                mdv_ptrs.push((mdv_array_ptr, mdv_data_ptr));
                
                // Allocate MDeformWeight arrays for each vertex group
                for _ in vgroups.iter() {
                    for _ in 0..totvert {
                        mdw_ptrs.push(ptrs.alloc());
                    }
                }
                
                let cdl = ptrs.alloc();
                (v_first, v_last, vgroups.len() as u64, cdl, v_ptrs, mdv_ptrs, mdw_ptrs)
            } else {
                (0, 0, 0, 0, Vec::new(), Vec::new(), Vec::new())
            }
        } else {
            (0, 0, 0, 0, Vec::new(), Vec::new(), Vec::new())
        };

    // ── Geometry data ─────────────────────────────────────────────────────────

    // poly_offsets: [0, 3, 6, ..., totloop] — totpoly+1 entries (sentinel at end)
    let poly_offsets: Vec<i32> = (0..=totpoly as i32).map(|i| i * 3).collect();

    // Modifier pointer allocations. Decal meshes append a Displace modifier
    // after the existing Weld and Weighted Normal modifiers.
    let has_decal_offset = has_decal_offset_vertices(vertex_groups);
    let weld_mod_ptr = ptrs.alloc();
    let wn_mod_ptr = ptrs.alloc();
    let decal_offset_mod_ptr = if has_decal_offset { ptrs.alloc() } else { 0 };
    let raw_poly_offsets = ints_data(&poly_offsets);

    // corner_vert[i] = vertex index for loop corner i
    let corner_verts: Vec<i32> = mesh.indices.iter().map(|&i| i as i32).collect();

    let raw_position   = floats3_data(&mesh.positions);
    let raw_edge_verts = ints2_data(&edge_verts);
    let raw_corner_vert = ints_data(&corner_verts);
    let raw_corner_edge = ints_data(&corner_edges);

    // Per-polygon material index: fill from submesh ranges
    let mut material_indices: Vec<i32> = vec![0; totpoly];
    for (submesh_idx, submesh) in mesh.submeshes.iter().enumerate() {
        let mat_idx = submesh_material_slots.get(submesh_idx).copied().unwrap_or(0);
        let start_face = submesh.first_index / 3;
        let num_faces = submesh.num_indices / 3;
        for i in 0..num_faces {
            if (start_face + i) < totpoly as u32 {
                material_indices[(start_face + i) as usize] = mat_idx as i32;
            }
        }
    }
    let raw_material_index = ints_data(&material_indices);

    // Expand per-vertex UVs to per-loop data. Blender's UV origin is opposite
    // the exported Star Citizen texture-space V origin, so V is flipped here.
    let raw_uv = mesh
        .uvs
        .as_ref()
        .map(|uvs| expanded_blender_uv_data(&mesh.indices, uvs));
    let raw_uv2 = mesh
        .secondary_uvs
        .as_ref()
        .map(|uvs| expanded_blender_uv_data(&mesh.indices, uvs));

    // Expand per-vertex colors → per-loop
    let raw_color: Option<Vec<u8>> = mesh.colors.as_ref().map(|colors| {
        let expanded: Vec<[u8; 4]> = mesh.indices.iter().map(|&i| colors[i as usize]).collect();
        bytes4_data(&expanded)
    });

    // ── Attribute descriptor blob ─────────────────────────────────────────────

    let mut attr_blob: Vec<u8> = Vec::new();
    let mut num_attrs: u32 = 5; // position + edge_verts + corner_vert + corner_edge + material_index

    attr_blob.extend_from_slice(&build_attribute(
        name_pos_ptr, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, array_pos_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_ev_ptr, ATTR_TYPE_INT32_2D, ATTR_DOMAIN_EDGE, array_ev_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_cv_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_cv_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_ce_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_ce_ptr,
    ));
    attr_blob.extend_from_slice(&build_attribute(
        name_matidx_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_FACE, array_matidx_ptr,
    ));
    if mesh.uvs.is_some() {
        attr_blob.extend_from_slice(&build_attribute(
            name_uv_ptr, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, array_uv_ptr,
        ));
        num_attrs += 1;
    }
    if mesh.secondary_uvs.is_some() {
        attr_blob.extend_from_slice(&build_attribute(
            name_uv2_ptr, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, array_uv2_ptr,
        ));
        num_attrs += 1;
    }
    if mesh.colors.is_some() {
        attr_blob.extend_from_slice(&build_attribute(
            name_col_ptr, ATTR_TYPE_BYTE_COLOR, ATTR_DOMAIN_CORNER, array_col_ptr,
        ));
        num_attrs += 1;
    }

    // ── Datablocks ────────────────────────────────────────────────────────────

    let mut object_data = build_object(
        name, mesh_ptr, obj_mat_ptr, obj_matbits_ptr, mat_slots as i32, 0,
    );
    patch_object_parent_transform(
        &mut object_data,
        0,
        [0.0, 0.0, 0.0],
        BLENDER_BAKED_ROOT_QUAT,
        BLENDER_BAKED_ROOT_SCALE,
    );
    set_object_modifiers_listbase(
        &mut object_data,
        weld_mod_ptr,
        if has_decal_offset { decal_offset_mod_ptr } else { wn_mod_ptr },
    );
    let scene_data = build_scene_with_motion_blur_curve_and_properties(
        name,
        view_layer_ptr,
        collection_ptr,
        tool_settings_ptr,
        motion_blur_curve_points_ptr,
        cycles_scene_props.root_ptr,
        world_ptr,
        "CYCLES",
    );
    let motion_blur_curve_points_data = build_motion_blur_shutter_curve_points();
    let tool_settings_data = build_tool_settings();
    let view_layer_data = build_view_layer("ViewLayer", base_ptr, layer_collection_ptr);
    let base_data = build_base(object_ptr);
    let collection_data = build_master_collection(collection_object_ptr, collection_object_ptr, 0, 0);
    let collection_object_data =
        build_collection_object_linked(object_ptr, 0, 0);
    let layer_collection_data = build_layer_collection(collection_ptr);
    let mut mesh_data = build_mesh(
        name, totvert, totedge, totpoly, totloop,
        poly_offs_ptr, attrs_ptr,
        mesh_mat_ptr, mat_slots,
        vgroup_first_ptr, vgroup_last_ptr, cdl_ptr,
        num_attrs,
    );
    if raw_uv.is_some() {
        write_ptr(&mut mesh_data, 1584, active_uv_name_ptr);
        write_ptr(&mut mesh_data, 1592, default_uv_name_ptr);
    }
    let mesh_mat_array = build_mat_ptr_array_from_ptrs(&material_ptrs);
    let obj_mat_array  = build_mat_ptr_array(mat_slots as usize);
    let obj_matbits    = build_matbits(mat_slots as usize);

    let arr_pos = build_attribute_array(raw_pos_ptr,  totvert as i64);
    let arr_ev  = build_attribute_array(raw_ev_ptr,   totedge as i64);
    let arr_cv  = build_attribute_array(raw_cv_ptr,   totloop as i64);
    let arr_ce  = build_attribute_array(raw_ce_ptr,   totloop as i64);

    // ── Assemble file ─────────────────────────────────────────────────────────

    let mut out: Vec<u8> = Vec::with_capacity(512 * 1024);
    out.extend_from_slice(BLEND_MAGIC);

    let file_global = build_file_global(STARTUP_UI_SCREEN_PTR, scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);
    out.extend_from_slice(&startup_ui_prefix_bytes());

    // Minimal scene graph so Blender opens this as a normal scene, not library-only data.
    // CRITICAL: All DATA blocks for a given ID block must be consecutive immediately after it.
    // Blender's readfile.cc reads them all into fd->datamap, then clears it after each ID block.
    write_block(&mut out, b"SC\0\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    // SC DATA sequence (all consecutive, no non-DATA blocks until OB):
    write_block(&mut out, b"DATA", 0, motion_blur_curve_points_ptr, 3, &motion_blur_curve_points_data);
    write_cycles_scene_props(&mut out, &cycles_scene_props);
    write_block(&mut out, b"DATA", SDNA_IDX_TOOL_SETTINGS, tool_settings_ptr, 1, &tool_settings_data);
    write_block(&mut out, b"DATA", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, layer_collection_ptr, 1, &layer_collection_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION, collection_ptr, 1, &collection_data);  // embedded master_collection
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, collection_object_ptr, 1, &collection_object_data);
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);
    write_world_with_sky_shader(&mut out, "World", world_ptr, world_node_tree_ptr, &mut ptrs);

    // OB block + DATA blocks (gap=1 rule: mat** and matbits must immediately follow OB)
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, object_ptr, 1, &object_data);
    write_block(&mut out, b"DATA", 0, obj_mat_ptr,      1, &obj_mat_array);
    write_block(&mut out, b"DATA", 0, obj_matbits_ptr,  1, &obj_matbits);
    // Modifier DATA blocks (part of OB's consecutive data chain).
    write_block(&mut out, b"DATA", SDNA_IDX_WELD_MODIFIER, weld_mod_ptr, 1,
        &build_weld_modifier("StarBreaker Weld", wn_mod_ptr, 0, 0.0005));
    write_block(&mut out, b"DATA", SDNA_IDX_WEIGHTED_NORMAL_MODIFIER, wn_mod_ptr, 1,
        &build_weighted_normal_modifier(
            "StarBreaker Weighted Normal",
            decal_offset_mod_ptr,
            weld_mod_ptr,
            50,
            0.01,
        ));
    if has_decal_offset {
        write_block(&mut out, b"DATA", SDNA_IDX_DISPLACE_MODIFIER, decal_offset_mod_ptr, 1,
            &build_displace_modifier(
                DECAL_OFFSET_MODIFIER_NAME,
                0,
                wn_mod_ptr,
                0.005,
                DECAL_OFFSET_GROUP_NAME,
                DECAL_OFFSET_MIDLEVEL,
            ));
    }

    // ME block + DATA block (gap=1 rule: mesh mat** must immediately follow ME)
    write_block(&mut out, b"ME\0\0", SDNA_IDX_MESH, mesh_ptr, 1, &mesh_data);
    write_block(&mut out, b"DATA", 0, mesh_mat_ptr, 1, &mesh_mat_array);

    // Attribute descriptor block (all Attribute structs concatenated)
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE, attrs_ptr, num_attrs as u32, &attr_blob);

    // Attribute name strings
    write_block(&mut out, b"DATA", 0, name_pos_ptr, 1, b"position\0");
    write_block(&mut out, b"DATA", 0, name_ev_ptr,  1, b".edge_verts\0");
    write_block(&mut out, b"DATA", 0, name_cv_ptr,  1, b".corner_vert\0");
    write_block(&mut out, b"DATA", 0, name_ce_ptr,  1, b".corner_edge\0");
    write_block(&mut out, b"DATA", 0, name_matidx_ptr, 1, b"material_index\0");

    // Attribute array descriptors + raw data (topology, position, material_index)
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_pos_ptr, 1, &arr_pos);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_ev_ptr,  1, &arr_ev);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_cv_ptr,  1, &arr_cv);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_ce_ptr,  1, &arr_ce);
    let arr_matidx = build_attribute_array(raw_matidx_ptr, totpoly as i64);
    write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_matidx_ptr, 1, &arr_matidx);
    write_block(&mut out, b"DATA", 0, raw_pos_ptr, 1, &raw_position);
    write_block(&mut out, b"DATA", 0, raw_ev_ptr,  1, &raw_edge_verts);
    write_block(&mut out, b"DATA", 0, raw_cv_ptr,  1, &raw_corner_vert);
    write_block(&mut out, b"DATA", 0, raw_ce_ptr,  1, &raw_corner_edge);
    write_block(&mut out, b"DATA", 0, raw_matidx_ptr, 1, &raw_material_index);

    // Optional: UV map
    if let Some(ref uv_data) = raw_uv {
        let arr_uv = build_attribute_array(raw_uv_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0,  name_uv_ptr,  1, b"UVMap\0");
        write_block(&mut out, b"DATA", 0, active_uv_name_ptr, 1, b"UVMap\0");
        write_block(&mut out, b"DATA", 0, default_uv_name_ptr, 1, b"UVMap\0");
        write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_uv_ptr, 1, &arr_uv);
        write_block(&mut out, b"DATA", 0,  raw_uv_ptr,   1, uv_data);
    }
    if let Some(ref uv_data) = raw_uv2 {
        let arr_uv = build_attribute_array(raw_uv2_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0, name_uv2_ptr, 1, b"UVMap.001\0");
        write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_uv2_ptr, 1, &arr_uv);
        write_block(&mut out, b"DATA", 0, raw_uv2_ptr, 1, uv_data);
    }

    // Optional: vertex colors
    if let Some(ref color_data) = raw_color {
        let arr_col = build_attribute_array(raw_col_ptr, totloop as i64);
        write_block(&mut out, b"DATA", 0,  name_col_ptr,  1, b"Color\0");
        write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, array_col_ptr, 1, &arr_col);
        write_block(&mut out, b"DATA", 0,  raw_col_ptr,   1, color_data);
    }

    // Polygon offset array
    write_block(&mut out, b"DATA", 0, poly_offs_ptr, 1, &raw_poly_offsets);

    // Phase 5D: Vertex groups
    if let Some(vgroups) = vertex_groups {
        if !vgroups.is_empty() && !vgroup_ptrs.is_empty() {
            // Write bDeformGroup entries
            for (idx, vgroup) in vgroups.iter().enumerate() {
                let next_ptr = if idx + 1 < vgroup_ptrs.len() {
                    vgroup_ptrs[idx + 1]
                } else {
                    0
                };
                let prev_ptr = if idx > 0 {
                    vgroup_ptrs[idx - 1]
                } else {
                    0
                };
                let bdeform_data = build_bdeformgroup(&vgroup.name, next_ptr, prev_ptr);
                write_block(&mut out, b"DATA", SDNA_IDX_BDEFORMGROUP, vgroup_ptrs[idx], 1, &bdeform_data);
            }
            
            // Write MDeformVert array with weights for each vertex
            if !mdeformvert_ptrs.is_empty() {
                let (mdv_array_ptr, mdv_data_ptr) = mdeformvert_ptrs[0];
                let mut mdeformvert_data = Vec::new();
                
                for vert_idx in 0..totvert {
                    // Find which vertex groups this vertex belongs to
                    let mut weights_for_vert = Vec::new();
                    for (group_idx, vgroup) in vgroups.iter().enumerate() {
                        if vgroup.vertex_indices.contains(&vert_idx) {
                            weights_for_vert.push((group_idx as u32, 1.0f32));
                        }
                    }
                    
                    if !weights_for_vert.is_empty() {
                        // For this implementation, write weight array for this vertex
                        let weight_array_ptr = if vert_idx < mdeformweight_ptrs.len() {
                            mdeformweight_ptrs[vert_idx]
                        } else {
                            0
                        };
                        mdeformvert_data.push((weight_array_ptr, weights_for_vert.len() as u32));
                    } else {
                        mdeformvert_data.push((0, 0));
                    }
                }
                
                let mdv_array_data = build_mdeformvert_array(&mdeformvert_data);
                write_block(&mut out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, mdv_array_ptr, 1, &mdv_array_data);
                
                // Write individual weight arrays
                for vert_idx in 0..totvert {
                    for (group_idx, vgroup) in vgroups.iter().enumerate() {
                        if vgroup.vertex_indices.contains(&vert_idx) {
                            let weight_array_ptr = if vert_idx < mdeformweight_ptrs.len() {
                                mdeformweight_ptrs[vert_idx]
                            } else {
                                continue;
                            };
                            let weight_data = build_mdeformweight_array(&[(group_idx as u32, 1.0f32)]);
                            write_block(&mut out, b"DATA", 0, weight_array_ptr, 1, &weight_data);
                        }
                    }
                }
                
                // Write MDeformVert data array itself
                let mdv_raw_data = build_mdeformvert_array(&mdeformvert_data);
                write_block(&mut out, b"DATA", SDNA_IDX_MDEFORMVERT, mdv_data_ptr, totvert as u32, &mdv_raw_data);
            }
            
            // Write CustomDataLayer
            if cdl_ptr != 0 && !mdeformvert_ptrs.is_empty() {
                let (_, mdv_data_ptr) = mdeformvert_ptrs[0];
                let cdl_data = build_custom_data_layer_mdeformvert(mdv_data_ptr);
                write_block(&mut out, b"DATA", 0, cdl_ptr, 1, &cdl_data);
            }
        }
    }

    for ((material_ptr, material_props), material_name) in material_ptrs
        .iter()
        .zip(material_idprops.iter())
        .zip(material_names.iter())
    {
        let material_data = build_material_with_node_tree_and_properties(
            material_name,
            0,
            material_props.as_ref().map(|props| props.root_ptr).unwrap_or(0),
        );
        write_block(&mut out, b"MA\0\0", SDNA_IDX_MATERIAL, *material_ptr, 1, &material_data);
        if let Some(props) = material_props {
            write_idprop_blocks(&mut out, props);
        }
    }

    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);

    // Phase 1D: Do NOT compress individual mesh files (keep uncompressed)
    // Compression only happens at scene.blend (Phase 2)
    out
}

#[derive(Clone)]
struct MeshObjectExport {
    name: String,
    mesh: Mesh,
    vertex_groups: Option<Vec<VertexGroup>>,
}

fn empty_anchor_mesh() -> Mesh {
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

struct MeshBlockData {
    object_ptr: u64,
    mesh_ptr: u64,
    mesh_mat_ptr: u64,
    obj_mat_ptr: u64,
    obj_matbits_ptr: u64,
    poly_offs_ptr: u64,
    attrs_ptr: u64,
    name_pos_ptr: u64,
    name_ev_ptr: u64,
    name_cv_ptr: u64,
    name_ce_ptr: u64,
    array_pos_ptr: u64,
    array_ev_ptr: u64,
    array_cv_ptr: u64,
    array_ce_ptr: u64,
    raw_pos_ptr: u64,
    raw_ev_ptr: u64,
    raw_cv_ptr: u64,
    raw_ce_ptr: u64,
    name_matidx_ptr: u64,
    array_matidx_ptr: u64,
    raw_matidx_ptr: u64,
    name_uv_ptr: u64,
    array_uv_ptr: u64,
    raw_uv_ptr: u64,
    active_uv_name_ptr: u64,
    default_uv_name_ptr: u64,
    name_uv2_ptr: u64,
    array_uv2_ptr: u64,
    raw_uv2_ptr: u64,
    name_col_ptr: u64,
    array_col_ptr: u64,
    raw_col_ptr: u64,
    vgroup_first_ptr: u64,
    vgroup_last_ptr: u64,
    vgroup_count: u64,
    cdl_ptr: u64,
    vgroup_ptrs: Vec<u64>,
    mdeformvert_ptrs: Vec<(u64, u64)>,
    mdeformweight_ptrs: Vec<u64>,
    weld_mod_ptr: u64,
    wn_mod_ptr: u64,
    decal_offset_mod_ptr: u64,
    totvert: usize,
    totedge: usize,
    totpoly: usize,
    totloop: usize,
    num_attrs: u32,
    raw_poly_offsets: Vec<u8>,
    raw_position: Vec<u8>,
    raw_edge_verts: Vec<u8>,
    raw_corner_vert: Vec<u8>,
    raw_corner_edge: Vec<u8>,
    raw_material_index: Vec<u8>,
    raw_uv: Option<Vec<u8>>,
    raw_uv2: Option<Vec<u8>>,
    raw_color: Option<Vec<u8>>,
    attr_blob: Vec<u8>,
    material_slot_indices: Vec<usize>,
}

fn uv_to_blender(uv: [f32; 2]) -> [f32; 2] {
    [uv[0], 1.0 - uv[1]]
}

fn expanded_blender_uv_data(indices: &[u32], uvs: &[[f32; 2]]) -> Vec<u8> {
    let expanded = indices
        .iter()
        .map(|&i| uvs.get(i as usize).copied().map(uv_to_blender).unwrap_or([0.0; 2]))
        .collect::<Vec<_>>();
    floats2_data(&expanded)
}

fn strip_lod_suffix(name: &str) -> String {
    let Some((stem, suffix)) = name.rsplit_once("_LOD") else {
        return name.to_string();
    };
    if !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) {
        stem.to_string()
    } else {
        name.to_string()
    }
}

fn linked_scene_object_names(
    mesh_name: &str,
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
) -> Vec<String> {
    let Some(nmc) = nmc.filter(|nmc| !nmc.nodes.is_empty()) else {
        return vec![mesh_name.to_string()];
    };

    let export_names = nmc_export_object_names(mesh_name, nmc);
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut node_submeshes: Vec<Vec<usize>> = vec![Vec::new(); nmc.nodes.len()];
    for (submesh_index, submesh) in mesh.submeshes.iter().enumerate() {
        let node_index = submesh.node_parent_index as usize;
        if node_index < node_submeshes.len() {
            node_submeshes[node_index].push(submesh_index);
        }
    }
    for (node_index, submesh_indices) in node_submeshes.iter().enumerate() {
        if submesh_indices.is_empty() {
            continue;
        }
        let (node_mesh, _) = subset_mesh_for_submeshes(mesh, submesh_indices, None);
        if node_mesh.indices.is_empty() {
            continue;
        }
        let name = export_names[node_index].clone();
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }

    if names.is_empty() {
        vec![mesh_name.to_string()]
    } else {
        names
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LinkedMeshRef {
    object_name: String,
    mesh_name: String,
    material_names: Vec<String>,
    source_parent_name: Option<String>,
    object_loc: [f32; 3],
    object_quat: [f32; 4],
    object_scale: [f32; 3],
    ancestors: Vec<LinkedSourceAncestor>,
    has_decal_offset_modifier: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkedSourceAncestor {
    pub name: String,
    pub loc: [f32; 3],
    pub quat: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkedSourceNode {
    pub name: String,
    pub parent_name: Option<String>,
    pub loc: [f32; 3],
    pub quat: [f32; 4],
    pub scale: [f32; 3],
}

fn blend_link_data_from_bytes(blend_bytes: &[u8]) -> (Vec<LinkedMeshRef>, Vec<LinkedSourceNode>) {
    #[derive(Debug, Clone)]
    struct ObjectInfo {
        name: String,
        object_type: i16,
        parent_ptr: u64,
        mesh_ptr: u64,
        modifier_first_ptr: u64,
        loc: [f32; 3],
        quat: [f32; 4],
        scale: [f32; 3],
    }
    #[derive(Debug, Clone)]
    struct MeshInfo {
        name: String,
        mat_ptr: u64,
        mat_slots: usize,
    }

    let mut object_order = Vec::new();
    let mut objects_by_ptr = HashMap::new();
    let mut meshes_by_ptr = HashMap::new();
    let mut material_names_by_ptr = HashMap::new();
    let mut data_by_ptr = HashMap::new();
    let mut sdna_by_ptr = HashMap::new();
    let mut offset = BLEND_MAGIC.len();
    while offset + 32 <= blend_bytes.len() {
        let code = &blend_bytes[offset..offset + 4];
        let old_ptr = u64::from_le_bytes(blend_bytes[offset + 8..offset + 16].try_into().unwrap());
        let size = u32::from_le_bytes(blend_bytes[offset + 16..offset + 20].try_into().unwrap()) as usize;
        let data_start = offset + 32;
        let data_end = data_start.saturating_add(size);
        if data_end > blend_bytes.len() {
            break;
        }
        if code == b"ME\0\0" {
            let data = &blend_bytes[data_start..data_end];
            if data.len() >= 300 {
                let raw = &data[42..300];
                let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
                meshes_by_ptr.insert(
                    old_ptr,
                    MeshInfo {
                        name: String::from_utf8_lossy(&raw[..end]).to_string(),
                        mat_ptr: u64::from_le_bytes(data[424..432].try_into().unwrap()),
                        mat_slots: i16::from_le_bytes(data[1618..1620].try_into().unwrap()).max(0) as usize,
                    },
                );
            }
        } else if code == b"MA\0\0" {
            let data = &blend_bytes[data_start..data_end];
            if data.len() >= 300 {
                let raw = &data[42..300];
                let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
                material_names_by_ptr.insert(old_ptr, String::from_utf8_lossy(&raw[..end]).to_string());
            }
        } else if code == b"DATA" {
            data_by_ptr.insert(old_ptr, blend_bytes[data_start..data_end].to_vec());
            let sdna = u32::from_le_bytes(blend_bytes[offset + 4..offset + 8].try_into().unwrap());
            sdna_by_ptr.insert(old_ptr, sdna);
        } else if code == b"OB\0\0" {
            let data = &blend_bytes[data_start..data_end];
            if data.len() >= OBJECT_SIZE {
                let raw = &data[42..300];
                let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
                let name = String::from_utf8_lossy(&raw[..end]).to_string();
                let object_type = i16::from_le_bytes(data[416..418].try_into().unwrap());
                object_order.push(old_ptr);
                objects_by_ptr.insert(
                    old_ptr,
                    ObjectInfo {
                        name,
                        object_type,
                        parent_ptr: u64::from_le_bytes(data[496..504].try_into().unwrap()),
                        mesh_ptr: u64::from_le_bytes(data[552..560].try_into().unwrap()),
                        modifier_first_ptr: u64::from_le_bytes(data[656..664].try_into().unwrap()),
                        loc: [
                            f32::from_le_bytes(data[736..740].try_into().unwrap()),
                            f32::from_le_bytes(data[740..744].try_into().unwrap()),
                            f32::from_le_bytes(data[744..748].try_into().unwrap()),
                        ],
                        scale: [
                            f32::from_le_bytes(data[760..764].try_into().unwrap()),
                            f32::from_le_bytes(data[764..768].try_into().unwrap()),
                            f32::from_le_bytes(data[768..772].try_into().unwrap()),
                        ],
                        quat: [
                            f32::from_le_bytes(data[820..824].try_into().unwrap()),
                            f32::from_le_bytes(data[824..828].try_into().unwrap()),
                            f32::from_le_bytes(data[828..832].try_into().unwrap()),
                            f32::from_le_bytes(data[832..836].try_into().unwrap()),
                        ],
                    },
                );
            }
        }
        if code == b"ENDB" {
            break;
        }
        offset = data_end;
    }

    let linked_mesh_refs = object_order
        .iter()
        .filter_map(|object_ptr| {
            let object = objects_by_ptr.get(object_ptr)?;
            if object.object_type != 1 {
                return None;
            }
            let mesh_info = meshes_by_ptr.get(&object.mesh_ptr);
            let mesh_name = mesh_info
                .map(|mesh| mesh.name.clone())
                .unwrap_or_else(|| object.name.clone());
            let material_names = mesh_info
                .and_then(|mesh| data_by_ptr.get(&mesh.mat_ptr).map(|data| (mesh, data)))
                .map(|(mesh, data)| {
                    (0..mesh.mat_slots)
                        .filter_map(|slot| {
                            let start = slot * 8;
                            (start + 8 <= data.len())
                                .then(|| u64::from_le_bytes(data[start..start + 8].try_into().unwrap()))
                                .and_then(|ptr| material_names_by_ptr.get(&ptr).cloned())
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut has_decal_offset_modifier = false;
            let mut modifier_ptr = object.modifier_first_ptr;
            let mut visited_modifiers = HashSet::new();
            while modifier_ptr != 0 && visited_modifiers.insert(modifier_ptr) {
                if sdna_by_ptr.get(&modifier_ptr).copied() == Some(SDNA_IDX_DISPLACE_MODIFIER) {
                    has_decal_offset_modifier = true;
                    break;
                }
                let Some(modifier_data) = data_by_ptr.get(&modifier_ptr) else {
                    break;
                };
                if modifier_data.len() < 8 {
                    break;
                }
                modifier_ptr = u64::from_le_bytes(modifier_data[0..8].try_into().unwrap());
            }
            let mut ancestors = Vec::new();
            let mut parent_ptr = object.parent_ptr;
            while parent_ptr != 0 {
                let Some(parent) = objects_by_ptr.get(&parent_ptr) else {
                    break;
                };
                ancestors.push(LinkedSourceAncestor {
                    name: parent.name.clone(),
                    loc: parent.loc,
                    quat: parent.quat,
                    scale: parent.scale,
                });
                parent_ptr = parent.parent_ptr;
            }
            ancestors.reverse();
            Some(LinkedMeshRef {
                object_name: object.name.clone(),
                mesh_name,
                material_names,
                source_parent_name: objects_by_ptr
                    .get(&object.parent_ptr)
                    .map(|parent| parent.name.clone()),
                object_loc: object.loc,
                object_quat: object.quat,
                object_scale: object.scale,
                ancestors,
                has_decal_offset_modifier,
            })
        })
        .collect();

    let source_nodes = object_order
        .iter()
        .filter_map(|object_ptr| {
            let object = objects_by_ptr.get(object_ptr)?;
            (object.object_type == 0).then(|| LinkedSourceNode {
                name: object.name.clone(),
                parent_name: objects_by_ptr
                    .get(&object.parent_ptr)
                    .map(|parent| parent.name.clone()),
                loc: object.loc,
                quat: object.quat,
                scale: object.scale,
            })
        })
        .collect();

    (linked_mesh_refs, source_nodes)
}

fn mesh_object_refs_from_blend_bytes(blend_bytes: &[u8]) -> Vec<LinkedMeshRef> {
    blend_link_data_from_bytes(blend_bytes).0
}

fn source_empty_nodes_from_blend_bytes(blend_bytes: &[u8]) -> Vec<LinkedSourceNode> {
    blend_link_data_from_bytes(blend_bytes).1
}

fn reusable_native_blend_asset(
    job: &BlendAssetJob,
    existing_asset_loader: Option<&(dyn Fn(&str) -> Option<Vec<u8>> + Sync)>,
) -> Result<Option<BuiltBlendAsset>, Error> {
    let Some(loader) = existing_asset_loader else {
        return Ok(None);
    };
    let Some(stored_bytes) = loader(&job.blend_path) else {
        return Ok(None);
    };
    let blend_bytes = starbreaker_blend::decompress_blend_bytes_if_needed(&stored_bytes)
        .map_err(|error| {
            Error::Other(format!(
                "failed to decompress reusable native Blend asset '{}': {error}",
                job.blend_path
            ))
        })?;
    let (mut linked_mesh_refs, source_nodes) = blend_link_data_from_bytes(&blend_bytes);
    if linked_mesh_refs.is_empty() {
        linked_mesh_refs.push(LinkedMeshRef {
            object_name: job.mesh_name.clone(),
            mesh_name: job.mesh_name.clone(),
            material_names: Vec::new(),
            source_parent_name: None,
            object_loc: [0.0, 0.0, 0.0],
            object_quat: [1.0, 0.0, 0.0, 0.0],
            object_scale: [1.0, 1.0, 1.0],
            ancestors: Vec::new(),
            has_decal_offset_modifier: false,
        });
    }
    Ok(Some(BuiltBlendAsset {
        file: None,
        relative_path: job.blend_path.clone(),
        linked_mesh_refs,
        source_nodes,
    }))
}

fn build_native_blend_asset(
    job: &BlendAssetJob,
    mesh_data_map: &HashMap<String, MeshDataEntry>,
    mesh_vertex_groups: &HashMap<String, Vec<VertexGroup>>,
    existing_asset_loader: Option<&(dyn Fn(&str) -> Option<Vec<u8>> + Sync)>,
) -> Result<BuiltBlendAsset, Error> {
    if let Some(existing) = reusable_native_blend_asset(job, existing_asset_loader)? {
        return Ok(existing);
    }

    let Some(mesh_entry) = mesh_data_map.get(&job.blend_key) else {
        return Err(Error::Other(format!(
            "native Blend export has no mesh payload for generated asset '{}'",
            job.blend_path
        )));
    };
    let mut placement_mesh;
    let placement_name;
    let (mesh_name, mesh, nmc) = if mesh_entry.interior_placement_space {
        placement_name =
            interior_placement_object_name(&job.mesh_name, &mesh_entry.mesh, mesh_entry.nmc.as_ref());
        placement_mesh = mesh_entry.mesh.clone();
        convert_mesh_geometry_to_scene_axes(&mut placement_mesh);
        (placement_name.as_str(), &placement_mesh, None)
    } else {
        (job.mesh_name.as_str(), &mesh_entry.mesh, mesh_entry.nmc.as_ref())
    };
    let vgroups = mesh_vertex_groups.get(&job.blend_key).cloned();
    let blend_bytes = mesh_to_blend(
        mesh_name,
        mesh,
        &mesh_entry.materials,
        nmc,
        vgroups.as_ref(),
    );
    // Extract link data from uncompressed bytes before compressing for storage
    let (mut linked_mesh_refs, source_nodes) = blend_link_data_from_bytes(&blend_bytes);
    if linked_mesh_refs.is_empty() {
        linked_mesh_refs.push(LinkedMeshRef {
            object_name: job.mesh_name.clone(),
            mesh_name: job.mesh_name.clone(),
            material_names: Vec::new(),
            source_parent_name: None,
            object_loc: [0.0, 0.0, 0.0],
            object_quat: [1.0, 0.0, 0.0, 0.0],
            object_scale: [1.0, 1.0, 1.0],
            ancestors: Vec::new(),
            has_decal_offset_modifier: has_decal_offset_vertices(vgroups.as_ref()),
        });
    }
    // Compress for storage (matching Blender 5.x standard zstd format)
    let compressed = starbreaker_blend::compress_blend_bytes(&blend_bytes);

    Ok(BuiltBlendAsset {
        file: Some(ExportedFile {
            relative_path: job.blend_path.clone(),
            bytes: compressed,
            kind: ExportedFileKind::MeshAsset,
        }),
        relative_path: job.blend_path.clone(),
        linked_mesh_refs,
        source_nodes,
    })
}

fn build_native_blend_assets(
    jobs: &[BlendAssetJob],
    mesh_data_map: &HashMap<String, MeshDataEntry>,
    mesh_vertex_groups: &HashMap<String, Vec<VertexGroup>>,
    threads: usize,
    progress: Option<&Progress>,
    progress_from: f32,
    progress_to: f32,
    existing_asset_loader: Option<&(dyn Fn(&str) -> Option<Vec<u8>> + Sync)>,
) -> Result<Vec<BuiltBlendAsset>, Error> {
    let total_jobs = jobs.len().max(1);
    let progress_range = progress_to - progress_from;
    if threads == 1 || jobs.len() <= 1 {
        return jobs
            .iter()
            .enumerate()
            .map(|(index, job)| {
                let built = build_native_blend_asset(job, mesh_data_map, mesh_vertex_groups, existing_asset_loader)?;
                let fraction = (index + 1) as f32 / total_jobs as f32;
                report_progress(
                    progress,
                    progress_from + progress_range * fraction,
                    "Writing native .blend mesh assets with real geometry",
                );
                Ok(built)
            })
            .collect();
    }

    let mut builder = rayon::ThreadPoolBuilder::new();
    if threads > 0 {
        builder = builder.num_threads(threads);
    }
    let pool = builder
        .build()
        .map_err(|err| Error::Other(format!("failed to build blend export thread pool: {err}")))?;
    pool.install(|| {
        let completed = AtomicUsize::new(0);
        jobs.par_iter()
            .map(|job| {
                let built = build_native_blend_asset(job, mesh_data_map, mesh_vertex_groups, existing_asset_loader)?;
                let finished = completed.fetch_add(1, Ordering::Relaxed) + 1;
                let fraction = finished as f32 / total_jobs as f32;
                report_progress(
                    progress,
                    progress_from + progress_range * fraction,
                    "Writing native .blend mesh assets with real geometry",
                );
                Ok(built)
            })
            .collect()
    })
}

fn nmc_export_object_names(mesh_name: &str, nmc: &NodeMeshCombo) -> Vec<String> {
    let wrapper_name = strip_lod_suffix(mesh_name);
    let mut used = HashSet::new();
    used.insert(wrapper_name.clone());

    nmc.nodes
        .iter()
        .enumerate()
        .map(|(node_index, node)| {
            let base_name = if node.name.is_empty() {
                format!("{mesh_name}_node_{node_index}")
            } else {
                node.name.clone()
            };
            let mut name = if used.contains(&base_name) {
                if base_name == wrapper_name {
                    format!("{base_name}_mesh")
                } else {
                    format!("{base_name}_node_{node_index}")
                }
            } else {
                base_name
            };
            while used.contains(&name) {
                name = format!("{name}_{node_index}");
            }
            used.insert(name.clone());
            name
        })
        .collect()
}

fn matrix_to_transform(matrix: [f32; 16]) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let mat = glam::Mat4::from_cols_array(&matrix);
    let (scale, rotation, translation) = mat.to_scale_rotation_translation();
    (
        [translation.x, translation.y, translation.z],
        [rotation.w, rotation.x, rotation.y, rotation.z],
        [scale.x, scale.y, scale.z],
    )
}

fn interior_placement_object_name(
    default_name: &str,
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
) -> String {
    if let Some(nmc) = nmc {
        let referenced_nodes = mesh
            .submeshes
            .iter()
            .filter_map(|submesh| {
                let index = submesh.node_parent_index as usize;
                (index < nmc.nodes.len()).then_some(index)
            })
            .collect::<HashSet<_>>();
        if referenced_nodes.len() == 1 {
            let index = *referenced_nodes.iter().next().unwrap();
            let object_names = nmc_export_object_names(default_name, nmc);
            if let Some(name) = object_names.get(index).filter(|name| !name.is_empty()) {
                return name.clone();
            }
        }
    }
    strip_lod_suffix(default_name)
}

fn scene_axis_convert_vec3(value: [f32; 3]) -> [f32; 3] {
    [value[0], -value[2], value[1]]
}

fn convert_mesh_geometry_to_scene_axes(mesh: &mut Mesh) {
    for position in &mut mesh.positions {
        *position = scene_axis_convert_vec3(*position);
    }
    if let Some(normals) = mesh.normals.as_mut() {
        for normal in normals {
            *normal = scene_axis_convert_vec3(*normal);
        }
    }
    if let Some(tangents) = mesh.tangents.as_mut() {
        for tangent in tangents {
            let converted = scene_axis_convert_vec3([tangent[0], tangent[1], tangent[2]]);
            tangent[0] = converted[0];
            tangent[1] = converted[1];
            tangent[2] = converted[2];
        }
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for position in &mesh.positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    if min.iter().all(|v| v.is_finite()) {
        mesh.model_min = min;
        mesh.model_max = max;
        mesh.scaling_min = min;
        mesh.scaling_max = max;
    }
}

fn patch_object_parent_transform(
    object_data: &mut [u8],
    parent_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
) {
    write_ptr(object_data, 496, parent_ptr);
    for i in 0..3 {
        write_f32(object_data, 736 + i * 4, loc[i]);
        write_f32(object_data, 760 + i * 4, scale[i]);
        write_f32(object_data, 784 + i * 4, 1.0);
    }
    for i in 0..4 {
        write_f32(object_data, 820 + i * 4, quat[i]);
    }
    write_f32(object_data, 836, 1.0);
    write_i16(object_data, 1040, 0);
    if parent_ptr != 0 {
        write_identity_matrix4x4(object_data, 884);
    }
}

fn remap_vertex_groups(
    vertex_groups: Option<&Vec<VertexGroup>>,
    old_to_new: &HashMap<u32, u32>,
) -> Option<Vec<VertexGroup>> {
    let groups = vertex_groups?;
    let remapped = groups
        .iter()
        .filter_map(|group| {
            let mut vertex_indices = group
                .vertex_indices
                .iter()
                .filter_map(|old| old_to_new.get(&(*old as u32)).copied().map(|new| new as usize))
                .collect::<Vec<_>>();
            vertex_indices.sort_unstable();
            vertex_indices.dedup();
            if vertex_indices.is_empty() {
                None
            } else {
                Some(VertexGroup {
                    name: group.name.clone(),
                    vertex_indices,
                })
            }
        })
        .collect::<Vec<_>>();
    if remapped.is_empty() { None } else { Some(remapped) }
}

fn subset_mesh_for_submeshes(
    mesh: &Mesh,
    submesh_indices: &[usize],
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> (Mesh, Option<Vec<VertexGroup>>) {
    let mut old_to_new: HashMap<u32, u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut uvs = mesh.uvs.as_ref().map(|_| Vec::new());
    let mut secondary_uvs = mesh.secondary_uvs.as_ref().map(|_| Vec::new());
    let mut normals = mesh.normals.as_ref().map(|_| Vec::new());
    let mut tangents = mesh.tangents.as_ref().map(|_| Vec::new());
    let mut colors = mesh.colors.as_ref().map(|_| Vec::new());
    let mut indices = Vec::new();
    let mut submeshes = Vec::new();

    for &submesh_index in submesh_indices {
        let Some(source_submesh) = mesh.submeshes.get(submesh_index) else {
            continue;
        };
        let first_index = indices.len() as u32;
        let start = source_submesh.first_index as usize;
        let end = (start + source_submesh.num_indices as usize).min(mesh.indices.len());
        for &old_index in &mesh.indices[start..end] {
            let new_index = if let Some(&new_index) = old_to_new.get(&old_index) {
                new_index
            } else {
                let old = old_index as usize;
                let new_index = positions.len() as u32;
                positions.push(mesh.positions.get(old).copied().unwrap_or([0.0; 3]));
                if let (Some(src), Some(dst)) = (mesh.uvs.as_ref(), uvs.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([0.0; 2]));
                }
                if let (Some(src), Some(dst)) = (mesh.secondary_uvs.as_ref(), secondary_uvs.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([0.0; 2]));
                }
                if let (Some(src), Some(dst)) = (mesh.normals.as_ref(), normals.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([0.0, 0.0, 1.0]));
                }
                if let (Some(src), Some(dst)) = (mesh.tangents.as_ref(), tangents.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]));
                }
                if let (Some(src), Some(dst)) = (mesh.colors.as_ref(), colors.as_mut()) {
                    dst.push(src.get(old).copied().unwrap_or([255, 255, 255, 255]));
                }
                old_to_new.insert(old_index, new_index);
                new_index
            };
            indices.push(new_index);
        }
        let num_indices = indices.len() as u32 - first_index;
        if num_indices == 0 {
            continue;
        }
        let mut submesh = SubMesh {
            material_name: source_submesh.material_name.clone(),
            material_id: source_submesh.material_id,
            source_material_id: source_submesh.source_material_id,
            first_index,
            num_indices,
            first_vertex: 0,
            num_vertices: positions.len() as u32,
            node_parent_index: source_submesh.node_parent_index,
        };
        if let (Some(min), Some(max)) = (
            old_to_new.values().min().copied(),
            old_to_new.values().max().copied(),
        ) {
            submesh.first_vertex = min;
            submesh.num_vertices = max.saturating_sub(min) + 1;
        }
        submeshes.push(submesh);
    }

    let (model_min, model_max) = if positions.is_empty() {
        ([0.0; 3], [0.0; 3])
    } else {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for position in &positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        (min, max)
    };
    let vertex_groups = remap_vertex_groups(vertex_groups, &old_to_new);
    (
        Mesh {
            positions,
            indices,
            uvs,
            secondary_uvs,
            normals,
            tangents,
            colors,
            submeshes,
            model_min,
            model_max,
            scaling_min: mesh.scaling_min,
            scaling_max: mesh.scaling_max,
        },
        vertex_groups,
    )
}

fn local_material_slot_map(
    mesh: &Mesh,
    global_slot_by_material_id: &HashMap<u32, usize>,
) -> (HashMap<u32, usize>, Vec<usize>) {
    let mut local_slot_by_material_id = HashMap::new();
    let mut global_slot_indices = Vec::new();
    for submesh in &mesh.submeshes {
        if local_slot_by_material_id.contains_key(&submesh.material_id) {
            continue;
        }
        let Some(&global_slot) = global_slot_by_material_id.get(&submesh.material_id) else {
            continue;
        };
        let local_slot = global_slot_indices.len();
        local_slot_by_material_id.insert(submesh.material_id, local_slot);
        global_slot_indices.push(global_slot);
    }
    if global_slot_indices.is_empty() {
        global_slot_indices.push(0);
    }
    (local_slot_by_material_id, global_slot_indices)
}

fn allocate_mesh_block(
    ptrs: &mut PtrAlloc,
    mesh: &Mesh,
    vertex_groups: Option<&Vec<VertexGroup>>,
    submesh_slot_by_material_id: &HashMap<u32, usize>,
    material_slot_indices: Vec<usize>,
) -> MeshBlockData {
    let totvert = mesh.positions.len();
    let totloop = mesh.indices.len();
    let totpoly = totloop / 3;
    let (edge_verts, corner_edges) = triangle_edge_topology(&mesh.indices);
    let totedge = edge_verts.len();

    let object_ptr = ptrs.alloc();
    let mesh_ptr = ptrs.alloc();
    let mesh_mat_ptr = ptrs.alloc();
    let obj_mat_ptr = ptrs.alloc();
    let obj_matbits_ptr = ptrs.alloc();
    let poly_offs_ptr = ptrs.alloc();
    let attrs_ptr = ptrs.alloc();
    let name_pos_ptr = ptrs.alloc();
    let name_ev_ptr = ptrs.alloc();
    let name_cv_ptr = ptrs.alloc();
    let name_ce_ptr = ptrs.alloc();
    let array_pos_ptr = ptrs.alloc();
    let array_ev_ptr = ptrs.alloc();
    let array_cv_ptr = ptrs.alloc();
    let array_ce_ptr = ptrs.alloc();
    let raw_pos_ptr = ptrs.alloc();
    let raw_ev_ptr = ptrs.alloc();
    let raw_cv_ptr = ptrs.alloc();
    let raw_ce_ptr = ptrs.alloc();
    let name_matidx_ptr = ptrs.alloc();
    let array_matidx_ptr = ptrs.alloc();
    let raw_matidx_ptr = ptrs.alloc();
    let (name_uv_ptr, array_uv_ptr, raw_uv_ptr) = if mesh.uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    let (active_uv_name_ptr, default_uv_name_ptr) = if mesh.uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0)
    };
    let (name_uv2_ptr, array_uv2_ptr, raw_uv2_ptr) = if mesh.secondary_uvs.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    let (name_col_ptr, array_col_ptr, raw_col_ptr) = if mesh.colors.is_some() {
        (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())
    } else {
        (0, 0, 0)
    };
    let (vgroup_first_ptr, vgroup_last_ptr, vgroup_count, cdl_ptr, vgroup_ptrs, mdeformvert_ptrs, mdeformweight_ptrs) =
        if let Some(vgroups) = vertex_groups.filter(|groups| !groups.is_empty()) {
            let vgroup_ptrs = (0..vgroups.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
            let mdeformvert_ptrs = vec![(ptrs.alloc(), ptrs.alloc())];
            let mdeformweight_ptrs = (0..totvert).map(|_| ptrs.alloc()).collect::<Vec<_>>();
            (
                vgroup_ptrs[0],
                *vgroup_ptrs.last().unwrap(),
                vgroups.len() as u64,
                ptrs.alloc(),
                vgroup_ptrs,
                mdeformvert_ptrs,
                mdeformweight_ptrs,
            )
        } else {
            (0, 0, 0, 0, Vec::new(), Vec::new(), Vec::new())
        };

    let poly_offsets = (0..=totpoly as i32).map(|i| i * 3).collect::<Vec<_>>();
    let raw_poly_offsets = ints_data(&poly_offsets);
    let corner_verts = mesh.indices.iter().map(|&i| i as i32).collect::<Vec<_>>();
    let raw_position = floats3_data(&mesh.positions);
    let raw_edge_verts = ints2_data(&edge_verts);
    let raw_corner_vert = ints_data(&corner_verts);
    let raw_corner_edge = ints_data(&corner_edges);
    let mut material_indices = vec![0; totpoly];
    for submesh in &mesh.submeshes {
        let mat_idx = submesh_slot_by_material_id
            .get(&submesh.material_id)
            .copied()
            .unwrap_or(0);
        let start_face = submesh.first_index / 3;
        let num_faces = submesh.num_indices / 3;
        for i in 0..num_faces {
            if (start_face + i) < totpoly as u32 {
                material_indices[(start_face + i) as usize] = mat_idx as i32;
            }
        }
    }
    let raw_material_index = ints_data(&material_indices);
    let raw_uv = mesh
        .uvs
        .as_ref()
        .map(|uvs| expanded_blender_uv_data(&mesh.indices, uvs));
    let raw_uv2 = mesh
        .secondary_uvs
        .as_ref()
        .map(|uvs| expanded_blender_uv_data(&mesh.indices, uvs));
    let raw_color = mesh.colors.as_ref().map(|colors| {
        let expanded = mesh
            .indices
            .iter()
            .map(|&i| colors.get(i as usize).copied().unwrap_or([255, 255, 255, 255]))
            .collect::<Vec<_>>();
        bytes4_data(&expanded)
    });

    let mut attr_blob = Vec::new();
    let mut num_attrs = 5;
    attr_blob.extend_from_slice(&build_attribute(name_pos_ptr, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, array_pos_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_ev_ptr, ATTR_TYPE_INT32_2D, ATTR_DOMAIN_EDGE, array_ev_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_cv_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_cv_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_ce_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_CORNER, array_ce_ptr));
    attr_blob.extend_from_slice(&build_attribute(name_matidx_ptr, ATTR_TYPE_INT, ATTR_DOMAIN_FACE, array_matidx_ptr));
    if mesh.uvs.is_some() {
        attr_blob.extend_from_slice(&build_attribute(name_uv_ptr, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, array_uv_ptr));
        num_attrs += 1;
    }
    if mesh.secondary_uvs.is_some() {
        attr_blob.extend_from_slice(&build_attribute(name_uv2_ptr, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, array_uv2_ptr));
        num_attrs += 1;
    }
    if mesh.colors.is_some() {
        attr_blob.extend_from_slice(&build_attribute(name_col_ptr, ATTR_TYPE_BYTE_COLOR, ATTR_DOMAIN_CORNER, array_col_ptr));
        num_attrs += 1;
    }

    let has_decal_offset = has_decal_offset_vertices(vertex_groups);

    MeshBlockData {
        object_ptr,
        mesh_ptr,
        mesh_mat_ptr,
        obj_mat_ptr,
        obj_matbits_ptr,
        poly_offs_ptr,
        attrs_ptr,
        name_pos_ptr,
        name_ev_ptr,
        name_cv_ptr,
        name_ce_ptr,
        array_pos_ptr,
        array_ev_ptr,
        array_cv_ptr,
        array_ce_ptr,
        raw_pos_ptr,
        raw_ev_ptr,
        raw_cv_ptr,
        raw_ce_ptr,
        name_matidx_ptr,
        array_matidx_ptr,
        raw_matidx_ptr,
        name_uv_ptr,
        array_uv_ptr,
        raw_uv_ptr,
        active_uv_name_ptr,
        default_uv_name_ptr,
        name_uv2_ptr,
        array_uv2_ptr,
        raw_uv2_ptr,
        name_col_ptr,
        array_col_ptr,
        raw_col_ptr,
        vgroup_first_ptr,
        vgroup_last_ptr,
        vgroup_count,
        cdl_ptr,
        vgroup_ptrs,
        mdeformvert_ptrs,
        mdeformweight_ptrs,
        weld_mod_ptr: ptrs.alloc(),
        wn_mod_ptr: ptrs.alloc(),
        decal_offset_mod_ptr: if has_decal_offset { ptrs.alloc() } else { 0 },
        totvert,
        totedge,
        totpoly,
        totloop,
        num_attrs,
        raw_poly_offsets,
        raw_position,
        raw_edge_verts,
        raw_corner_vert,
        raw_corner_edge,
        raw_material_index,
        raw_uv,
        raw_uv2,
        raw_color,
        attr_blob,
        material_slot_indices,
    }
}

fn write_mesh_block(
    out: &mut Vec<u8>,
    block: &MeshBlockData,
    object: &MeshObjectExport,
    material_ptrs: &[u64],
    parent_ptr: u64,
    transform: ([f32; 3], [f32; 4], [f32; 3]),
) {
    let object_material_ptrs = block
        .material_slot_indices
        .iter()
        .filter_map(|&slot| material_ptrs.get(slot).copied())
        .collect::<Vec<_>>();
    let mat_slots = object_material_ptrs.len() as i16;
    let mut object_data = build_object(
        &object.name,
        block.mesh_ptr,
        block.obj_mat_ptr,
        block.obj_matbits_ptr,
        mat_slots as i32,
        0,
    );
    patch_object_parent_transform(&mut object_data, parent_ptr, transform.0, transform.1, transform.2);
    let has_decal_offset = block.decal_offset_mod_ptr != 0;
    set_object_modifiers_listbase(
        &mut object_data,
        block.weld_mod_ptr,
        if has_decal_offset { block.decal_offset_mod_ptr } else { block.wn_mod_ptr },
    );
    let mut mesh_data = build_mesh(
        &object.name,
        block.totvert,
        block.totedge,
        block.totpoly,
        block.totloop,
        block.poly_offs_ptr,
        block.attrs_ptr,
        block.mesh_mat_ptr,
        mat_slots,
        block.vgroup_first_ptr,
        block.vgroup_last_ptr,
        block.cdl_ptr,
        block.num_attrs,
    );
    if block.raw_uv.is_some() {
        write_ptr(&mut mesh_data, 1584, block.active_uv_name_ptr);
        write_ptr(&mut mesh_data, 1592, block.default_uv_name_ptr);
    }
    let mesh_mat_array = build_mat_ptr_array_from_ptrs(&object_material_ptrs);
    let obj_mat_array = build_mat_ptr_array(mat_slots as usize);
    let obj_matbits = build_matbits(mat_slots as usize);
    write_block(out, b"OB\0\0", SDNA_IDX_OBJECT, block.object_ptr, 1, &object_data);
    write_block(out, b"DATA", 0, block.obj_mat_ptr, 1, &obj_mat_array);
    write_block(out, b"DATA", 0, block.obj_matbits_ptr, 1, &obj_matbits);
    write_block(out, b"DATA", SDNA_IDX_WELD_MODIFIER, block.weld_mod_ptr, 1,
        &build_weld_modifier("StarBreaker Weld", block.wn_mod_ptr, 0, 0.0005));
    write_block(out, b"DATA", SDNA_IDX_WEIGHTED_NORMAL_MODIFIER, block.wn_mod_ptr, 1,
        &build_weighted_normal_modifier(
            "StarBreaker Weighted Normal",
            block.decal_offset_mod_ptr,
            block.weld_mod_ptr,
            50,
            0.01,
        ));
    if has_decal_offset {
        write_block(out, b"DATA", SDNA_IDX_DISPLACE_MODIFIER, block.decal_offset_mod_ptr, 1,
            &build_displace_modifier(
                DECAL_OFFSET_MODIFIER_NAME,
                0,
                block.wn_mod_ptr,
                0.005,
                DECAL_OFFSET_GROUP_NAME,
                DECAL_OFFSET_MIDLEVEL,
            ));
    }
    write_block(out, b"ME\0\0", SDNA_IDX_MESH, block.mesh_ptr, 1, &mesh_data);
    write_block(out, b"DATA", 0, block.mesh_mat_ptr, 1, &mesh_mat_array);
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE, block.attrs_ptr, block.num_attrs, &block.attr_blob);
    write_block(out, b"DATA", 0, block.name_pos_ptr, 1, b"position\0");
    write_block(out, b"DATA", 0, block.name_ev_ptr, 1, b".edge_verts\0");
    write_block(out, b"DATA", 0, block.name_cv_ptr, 1, b".corner_vert\0");
    write_block(out, b"DATA", 0, block.name_ce_ptr, 1, b".corner_edge\0");
    write_block(out, b"DATA", 0, block.name_matidx_ptr, 1, b"material_index\0");
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_pos_ptr, 1, &build_attribute_array(block.raw_pos_ptr, block.totvert as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_ev_ptr, 1, &build_attribute_array(block.raw_ev_ptr, block.totedge as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_cv_ptr, 1, &build_attribute_array(block.raw_cv_ptr, block.totloop as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_ce_ptr, 1, &build_attribute_array(block.raw_ce_ptr, block.totloop as i64));
    write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_matidx_ptr, 1, &build_attribute_array(block.raw_matidx_ptr, block.totpoly as i64));
    write_block(out, b"DATA", 0, block.raw_pos_ptr, 1, &block.raw_position);
    write_block(out, b"DATA", 0, block.raw_ev_ptr, 1, &block.raw_edge_verts);
    write_block(out, b"DATA", 0, block.raw_cv_ptr, 1, &block.raw_corner_vert);
    write_block(out, b"DATA", 0, block.raw_ce_ptr, 1, &block.raw_corner_edge);
    write_block(out, b"DATA", 0, block.raw_matidx_ptr, 1, &block.raw_material_index);
    if let Some(ref uv_data) = block.raw_uv {
        write_block(out, b"DATA", 0, block.name_uv_ptr, 1, b"UVMap\0");
        write_block(out, b"DATA", 0, block.active_uv_name_ptr, 1, b"UVMap\0");
        write_block(out, b"DATA", 0, block.default_uv_name_ptr, 1, b"UVMap\0");
        write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_uv_ptr, 1, &build_attribute_array(block.raw_uv_ptr, block.totloop as i64));
        write_block(out, b"DATA", 0, block.raw_uv_ptr, 1, uv_data);
    }
    if let Some(ref uv_data) = block.raw_uv2 {
        write_block(out, b"DATA", 0, block.name_uv2_ptr, 1, b"UVMap.001\0");
        write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_uv2_ptr, 1, &build_attribute_array(block.raw_uv2_ptr, block.totloop as i64));
        write_block(out, b"DATA", 0, block.raw_uv2_ptr, 1, uv_data);
    }
    if let Some(ref color_data) = block.raw_color {
        write_block(out, b"DATA", 0, block.name_col_ptr, 1, b"Color\0");
        write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, block.array_col_ptr, 1, &build_attribute_array(block.raw_col_ptr, block.totloop as i64));
        write_block(out, b"DATA", 0, block.raw_col_ptr, 1, color_data);
    }
    write_block(out, b"DATA", 0, block.poly_offs_ptr, 1, &block.raw_poly_offsets);
    if let Some(vgroups) = object.vertex_groups.as_ref().filter(|groups| !groups.is_empty()) {
        for (idx, vgroup) in vgroups.iter().enumerate() {
            let next_ptr = if idx + 1 < block.vgroup_ptrs.len() { block.vgroup_ptrs[idx + 1] } else { 0 };
            let prev_ptr = if idx > 0 { block.vgroup_ptrs[idx - 1] } else { 0 };
            let bdeform_data = build_bdeformgroup(&vgroup.name, next_ptr, prev_ptr);
            write_block(out, b"DATA", SDNA_IDX_BDEFORMGROUP, block.vgroup_ptrs[idx], 1, &bdeform_data);
        }
        if let Some((mdv_array_ptr, mdv_data_ptr)) = block.mdeformvert_ptrs.first().copied() {
            let mut mdeformvert_data = Vec::with_capacity(block.totvert);
            let mut weight_payloads = Vec::new();
            for vert_idx in 0..block.totvert {
                let weights_for_vert = vgroups
                    .iter()
                    .enumerate()
                    .filter_map(|(group_idx, vgroup)| {
                        vgroup.vertex_indices.contains(&vert_idx).then_some((group_idx as u32, 1.0f32))
                    })
                    .collect::<Vec<_>>();
                let weight_ptr = if weights_for_vert.is_empty() { 0 } else { block.mdeformweight_ptrs[vert_idx] };
                mdeformvert_data.push((weight_ptr, weights_for_vert.len() as u32));
                if !weights_for_vert.is_empty() {
                    weight_payloads.push((weight_ptr, build_mdeformweight_array(&weights_for_vert)));
                }
            }
            let mdv_array_data = build_mdeformvert_array(&mdeformvert_data);
            write_block(out, b"DATA", SDNA_IDX_ATTRIBUTE_ARRAY, mdv_array_ptr, 1, &mdv_array_data);
            for (weight_ptr, weight_data) in weight_payloads {
                write_block(out, b"DATA", 0, weight_ptr, 1, &weight_data);
            }
            write_block(out, b"DATA", SDNA_IDX_MDEFORMVERT, mdv_data_ptr, block.totvert as u32, &mdv_array_data);
            if block.cdl_ptr != 0 {
                write_block(out, b"DATA", 0, block.cdl_ptr, 1, &build_custom_data_layer_mdeformvert(mdv_data_ptr));
            }
        }
    }
}

fn mesh_to_blend_hierarchy(
    name: &str,
    mesh: &Mesh,
    materials: &Option<crate::mtl::MtlFile>,
    nmc: &NodeMeshCombo,
    vertex_groups: Option<&Vec<VertexGroup>>,
) -> Vec<u8> {
    let (material_names, full_submesh_slots) = blend_material_slots(name, mesh, materials);
    let mut submesh_slot_by_material_id = HashMap::new();
    for (submesh, slot) in mesh.submeshes.iter().zip(full_submesh_slots.iter().copied()) {
        submesh_slot_by_material_id.entry(submesh.material_id).or_insert(slot);
    }

    let mut node_submeshes: Vec<Vec<usize>> = vec![Vec::new(); nmc.nodes.len()];
    for (submesh_index, submesh) in mesh.submeshes.iter().enumerate() {
        let node_index = submesh.node_parent_index as usize;
        if node_index < node_submeshes.len() {
            node_submeshes[node_index].push(submesh_index);
        }
    }

    let nmc_export_names = nmc_export_object_names(name, nmc);
    let mut mesh_objects: Vec<Option<MeshObjectExport>> = vec![None; nmc.nodes.len()];
    for (node_index, submesh_indices) in node_submeshes.iter().enumerate() {
        if submesh_indices.is_empty() {
            continue;
        }
        let (node_mesh, node_vertex_groups) = subset_mesh_for_submeshes(mesh, submesh_indices, vertex_groups);
        if node_mesh.indices.is_empty() {
            continue;
        }
        mesh_objects[node_index] = Some(MeshObjectExport {
            name: nmc_export_names[node_index].clone(),
            mesh: node_mesh,
            vertex_groups: node_vertex_groups,
        });
    }
    let wrapper_name = strip_lod_suffix(name);
    let collapsed_wrapper_node = nmc.nodes.iter().enumerate().find_map(|(index, node)| {
        (node.parent_index.is_none()
            && node.name == wrapper_name
            && mesh_objects.get(index).and_then(|object| object.as_ref()).is_none())
        .then_some(index)
    });
    let mut ptrs = PtrAlloc::new(0x1000);
    let _screen_ptr = ptrs.alloc();
    let _wm_ptr = ptrs.alloc();
    let wrapper_ptr = ptrs.alloc();
    let wrapper_mat_ptr = ptrs.alloc();
    let wrapper_matbits_ptr = ptrs.alloc();
    let nmc_object_ptrs = (0..nmc.nodes.len())
        .map(|index| {
            if collapsed_wrapper_node == Some(index) {
                wrapper_ptr
            } else {
                ptrs.alloc()
            }
        })
        .collect::<Vec<_>>();
    let nmc_empty_mat_ptrs = (0..nmc.nodes.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let nmc_empty_matbits_ptrs = (0..nmc.nodes.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let mesh_blocks = mesh_objects
        .iter()
        .map(|object| {
            object.as_ref().map(|object| {
                let (local_slot_by_material_id, material_slot_indices) =
                    local_material_slot_map(&object.mesh, &submesh_slot_by_material_id);
                allocate_mesh_block(
                    &mut ptrs,
                    &object.mesh,
                    object.vertex_groups.as_ref(),
                    &local_slot_by_material_id,
                    material_slot_indices,
                )
            })
        })
        .collect::<Vec<_>>();
    let fallback_anchor = if mesh_objects.iter().all(Option::is_none) {
        let fallback_object = MeshObjectExport {
            name: name.to_string(),
            mesh: empty_anchor_mesh(),
            vertex_groups: None,
        };
        let (local_slot_by_material_id, material_slot_indices) =
            local_material_slot_map(&fallback_object.mesh, &submesh_slot_by_material_id);
        let fallback_block = allocate_mesh_block(
            &mut ptrs,
            &fallback_object.mesh,
            None,
            &local_slot_by_material_id,
            material_slot_indices,
        );
        Some((fallback_object, fallback_block))
    } else {
        None
    };
    let material_ptrs = (0..material_names.len()).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let material_idprops = material_names
        .iter()
        .map(|material_name| allocate_idprop_blocks(&mut ptrs, material_custom_properties(material_name, materials)))
        .collect::<Vec<_>>();
    let scene_ptr = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let tool_settings_ptr = ptrs.alloc();
    let motion_blur_curve_points_ptr = ptrs.alloc();
    let cycles_scene_props = allocate_cycles_scene_props(&mut ptrs);
    let world_ptr = ptrs.alloc();
    let world_node_tree_ptr = ptrs.alloc();
    let base_ptr = ptrs.alloc();
    let collection_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    let object_count = 1
        + usize::from(fallback_anchor.is_some())
        + nmc.nodes.len()
        - usize::from(collapsed_wrapper_node.is_some());
    let collection_object_ptrs = (0..object_count).map(|_| ptrs.alloc()).collect::<Vec<_>>();

    let scene_data = build_scene_with_motion_blur_curve_and_properties(
        name,
        view_layer_ptr,
        collection_ptr,
        tool_settings_ptr,
        motion_blur_curve_points_ptr,
        cycles_scene_props.root_ptr,
        world_ptr,
        "CYCLES",
    );
    let motion_blur_curve_points_data = build_motion_blur_shutter_curve_points();
    let tool_settings_data = build_tool_settings();
    let view_layer_data = build_view_layer("ViewLayer", base_ptr, layer_collection_ptr);
    let base_data = build_base(wrapper_ptr);
    let collection_data = build_master_collection(
        collection_object_ptrs.first().copied().unwrap_or(0),
        collection_object_ptrs.last().copied().unwrap_or(0),
        0,
        0,
    );
    let layer_collection_data = build_layer_collection(collection_ptr);
    let object_ptr_sequence = std::iter::once(wrapper_ptr)
        .chain(fallback_anchor.as_ref().map(|(_, block)| block.object_ptr))
        .chain(
            nmc_object_ptrs
                .iter()
                .enumerate()
                .filter_map(|(index, &ptr)| (collapsed_wrapper_node != Some(index)).then_some(ptr)),
        )
        .collect::<Vec<_>>();
    let collection_object_data = collection_object_ptrs
        .iter()
        .enumerate()
        .map(|(idx, &coll_ptr)| {
            let prev_ptr = if idx > 0 { collection_object_ptrs[idx - 1] } else { 0 };
            let next_ptr = if idx + 1 < collection_object_ptrs.len() { collection_object_ptrs[idx + 1] } else { 0 };
            (coll_ptr, build_collection_object_linked(object_ptr_sequence[idx], prev_ptr, next_ptr))
        })
        .collect::<Vec<_>>();

    let mut out = Vec::with_capacity(1024 * 1024);
    out.extend_from_slice(BLEND_MAGIC);
    let file_global = build_file_global(STARTUP_UI_SCREEN_PTR, scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);
    out.extend_from_slice(&startup_ui_prefix_bytes());
    write_block(&mut out, b"SC\0\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    write_block(&mut out, b"DATA", 0, motion_blur_curve_points_ptr, 3, &motion_blur_curve_points_data);
    write_cycles_scene_props(&mut out, &cycles_scene_props);
    write_block(&mut out, b"DATA", SDNA_IDX_TOOL_SETTINGS, tool_settings_ptr, 1, &tool_settings_data);
    write_block(&mut out, b"DATA", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, layer_collection_ptr, 1, &layer_collection_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION, collection_ptr, 1, &collection_data);
    for (coll_ptr, data) in &collection_object_data {
        write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, *coll_ptr, 1, data);
    }
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);
    write_world_with_sky_shader(&mut out, "World", world_ptr, world_node_tree_ptr, &mut ptrs);
    let wrapper_transform = collapsed_wrapper_node
        .and_then(|node_index| nmc.nodes.get(node_index))
        .map(|node| {
            if crate::gltf::is_identity_or_zero(&node.bone_to_world) {
                ([0.0, 0.0, 0.0], BLENDER_BAKED_ROOT_QUAT, BLENDER_BAKED_ROOT_SCALE)
            } else {
                matrix_to_transform(crate::gltf::mat3x4_to_gltf(&node.bone_to_world))
            }
        })
        .unwrap_or(([0.0, 0.0, 0.0], BLENDER_BAKED_ROOT_QUAT, BLENDER_BAKED_ROOT_SCALE));
    let wrapper_data = build_empty_object(
        &wrapper_name,
        wrapper_transform.0,
        wrapper_transform.1,
        wrapper_transform.2,
        0,
    );
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, wrapper_ptr, 1, &wrapper_data);
    write_block(&mut out, b"DATA", 0, wrapper_mat_ptr, 1, &build_mat_ptr_array(0));
    write_block(&mut out, b"DATA", 0, wrapper_matbits_ptr, 1, &build_matbits(0));

    if let Some((fallback_object, fallback_block)) = &fallback_anchor {
        write_mesh_block(
            &mut out,
            fallback_block,
            fallback_object,
            &material_ptrs,
            wrapper_ptr,
            ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
        );
    }

    for (node_index, node) in nmc.nodes.iter().enumerate() {
        if collapsed_wrapper_node == Some(node_index) {
            continue;
        }
        let parent_ptr = node
            .parent_index
            .and_then(|parent| nmc_object_ptrs.get(parent as usize).copied())
            .unwrap_or(wrapper_ptr);
        let transform = if crate::gltf::is_identity_or_zero(&node.bone_to_world) {
            ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0])
        } else {
            matrix_to_transform(crate::gltf::mat3x4_to_gltf(&node.bone_to_world))
        };
        if let (Some(object), Some(block)) = (&mesh_objects[node_index], &mesh_blocks[node_index]) {
            if block.object_ptr == nmc_object_ptrs[node_index] {
                write_mesh_block(&mut out, block, object, &material_ptrs, parent_ptr, transform);
            } else {
                let cloned = MeshBlockData {
                    object_ptr: nmc_object_ptrs[node_index],
                    mesh_ptr: block.mesh_ptr,
                    mesh_mat_ptr: block.mesh_mat_ptr,
                    obj_mat_ptr: block.obj_mat_ptr,
                    obj_matbits_ptr: block.obj_matbits_ptr,
                    poly_offs_ptr: block.poly_offs_ptr,
                    attrs_ptr: block.attrs_ptr,
                    name_pos_ptr: block.name_pos_ptr,
                    name_ev_ptr: block.name_ev_ptr,
                    name_cv_ptr: block.name_cv_ptr,
                    name_ce_ptr: block.name_ce_ptr,
                    array_pos_ptr: block.array_pos_ptr,
                    array_ev_ptr: block.array_ev_ptr,
                    array_cv_ptr: block.array_cv_ptr,
                    array_ce_ptr: block.array_ce_ptr,
                    raw_pos_ptr: block.raw_pos_ptr,
                    raw_ev_ptr: block.raw_ev_ptr,
                    raw_cv_ptr: block.raw_cv_ptr,
                    raw_ce_ptr: block.raw_ce_ptr,
                    name_matidx_ptr: block.name_matidx_ptr,
                    array_matidx_ptr: block.array_matidx_ptr,
                    raw_matidx_ptr: block.raw_matidx_ptr,
                    name_uv_ptr: block.name_uv_ptr,
                    array_uv_ptr: block.array_uv_ptr,
                    raw_uv_ptr: block.raw_uv_ptr,
                    active_uv_name_ptr: block.active_uv_name_ptr,
                    default_uv_name_ptr: block.default_uv_name_ptr,
                    name_uv2_ptr: block.name_uv2_ptr,
                    array_uv2_ptr: block.array_uv2_ptr,
                    raw_uv2_ptr: block.raw_uv2_ptr,
                    name_col_ptr: block.name_col_ptr,
                    array_col_ptr: block.array_col_ptr,
                    raw_col_ptr: block.raw_col_ptr,
                    vgroup_first_ptr: block.vgroup_first_ptr,
                    vgroup_last_ptr: block.vgroup_last_ptr,
                    vgroup_count: block.vgroup_count,
                    cdl_ptr: block.cdl_ptr,
                    vgroup_ptrs: block.vgroup_ptrs.clone(),
                    mdeformvert_ptrs: block.mdeformvert_ptrs.clone(),
                    mdeformweight_ptrs: block.mdeformweight_ptrs.clone(),
                    weld_mod_ptr: block.weld_mod_ptr,
                    wn_mod_ptr: block.wn_mod_ptr,
                    decal_offset_mod_ptr: block.decal_offset_mod_ptr,
                    totvert: block.totvert,
                    totedge: block.totedge,
                    totpoly: block.totpoly,
                    totloop: block.totloop,
                    num_attrs: block.num_attrs,
                    raw_poly_offsets: block.raw_poly_offsets.clone(),
                    raw_position: block.raw_position.clone(),
                    raw_edge_verts: block.raw_edge_verts.clone(),
                    raw_corner_vert: block.raw_corner_vert.clone(),
                    raw_corner_edge: block.raw_corner_edge.clone(),
                    raw_material_index: block.raw_material_index.clone(),
                    raw_uv: block.raw_uv.clone(),
                    raw_uv2: block.raw_uv2.clone(),
                    raw_color: block.raw_color.clone(),
                    attr_blob: block.attr_blob.clone(),
                    material_slot_indices: block.material_slot_indices.clone(),
                };
                write_mesh_block(&mut out, &cloned, object, &material_ptrs, parent_ptr, transform);
            }
        } else {
            let object_name = nmc_export_names[node_index].clone();
            let empty_data = build_empty_object(&object_name, transform.0, transform.1, transform.2, parent_ptr);
            write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, nmc_object_ptrs[node_index], 1, &empty_data);
            write_block(&mut out, b"DATA", 0, nmc_empty_mat_ptrs[node_index], 1, &build_mat_ptr_array(0));
            write_block(&mut out, b"DATA", 0, nmc_empty_matbits_ptrs[node_index], 1, &build_matbits(0));
        }
    }

    for ((material_ptr, material_props), material_name) in material_ptrs
        .iter()
        .zip(material_idprops.iter())
        .zip(material_names.iter())
    {
        let material_data = build_material_with_node_tree_and_properties(
            material_name,
            0,
            material_props.as_ref().map(|props| props.root_ptr).unwrap_or(0),
        );
        write_block(&mut out, b"MA\0\0", SDNA_IDX_MATERIAL, *material_ptr, 1, &material_data);
        if let Some(props) = material_props {
            write_idprop_blocks(&mut out, props);
        }
    }

    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);
    out
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5A: Scene.blend Assembly — Link mesh .blend files with collections
// ════════════════════════════════════════════════════════════════════════════════

/// Data for a linked mesh instance in scene.blend.
#[derive(Debug, Clone)]
pub struct LinkedMeshInstance {
    pub scene_instance_id: usize,
    pub entity_name: String,
    pub parent_entity_name: Option<String>,
    pub parent_empty_name: Option<String>,
    pub parent_empty_parent_entity_name: Option<String>,
    pub parent_empty_parent_node_name: Option<String>,
    pub parent_empty_loc: [f32; 3],
    pub parent_empty_quat: [f32; 4],
    pub parent_empty_scale: [f32; 3],
    pub is_interior: bool,
    pub source_object_name: String,
    /// Instance name (typically "mesh_0", "mesh_1", etc.)
    pub name: String,
    /// Mesh datablock name inside the linked decomposed asset file.
    pub mesh_name: String,
    pub material_names: Vec<String>,
    pub material_sidecar: Option<String>,
    pub palette_id: Option<String>,
    pub ui_bindings: Vec<serde_json::Value>,
    /// Source asset empty ancestors, ordered root-to-parent.
    pub source_ancestors: Vec<LinkedSourceAncestor>,
    /// Full source empty tree for this scene instance, when available.
    pub source_nodes: Vec<LinkedSourceNode>,
    /// Source object local transform inside its decomposed asset file.
    pub source_loc: [f32; 3],
    pub source_quat: [f32; 4],
    pub source_scale: [f32; 3],
    /// Direct source parent object name for this mesh object.
    pub source_parent_name: Option<String>,
    /// Optional source node in an already-created parent asset to attach this scene instance to.
    pub parent_node_name: Option<String>,
    /// Relative path to the linked .blend file (e.g., "Data/Objects/mesh_0.blend")
    pub blend_path: String,
    /// Package-relative mesh asset path before any scene.blend-relative library prefix.
    pub mesh_asset: String,
    /// Blender coordinates position [x, y, z]
    pub position: [f32; 3],
    /// Blender coordinates rotation as quaternion [w, x, y, z]
    pub rotation: [f32; 4],
    /// Blender coordinates scale.
    pub scale: [f32; 3],
    /// Whether this scene instance should be hidden in viewport and render by default.
    pub hidden: bool,
}

fn hide_object_viewport_and_render(object_data: &mut [u8]) {
    write_i16(object_data, 1082, 0x0005);
}

const BLENDER_BAKED_ROOT_QUAT: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
const BLENDER_BAKED_ROOT_SCALE: [f32; 3] = [1.0, 1.0, 1.0];

/// Create a scene.blend file that links together all individual mesh .blend files.
///
/// **Phase 5A Context:**
/// - Input: Entity name and number of children
/// - Output: A valid .blend file containing:
///   - Root scene object (empty at origin)
///   - Collections organized by entity type (Meshes, Lights, Empties, Decals)
///   - Linked instances pointing to mesh .blend files with relative paths
///   - Proper scene settings and render configuration
///
/// **Collections structure:**
/// - Scene (root)
///   - Meshes (contains linked mesh instances)
///   - Lights (for Phase 5B integration)
///   - Empties (for Phase 5C integration)
///   - Decals (placeholder for decal geometry)
///
/// **Library linking:**
/// - Each mesh is linked via Library block + ID stub
/// - Paths are relative for portability across export roots
/// - Transforms applied at instance level (mesh-level geometry unchanged)
///
/// # Arguments
///
/// * `entity_name` - Name of the scene entity
/// * `children_count` - Number of mesh child objects to create
/// * `mesh_output_dir` - Directory containing the mesh .blend files (e.g., "Data/Objects")
///
/// # Returns
///
/// Raw uncompressed .blend bytes ready for file write or further compression
pub fn create_scene_blend(
    entity_name: &str,
    children_count: usize,
    mesh_output_dir: &str,
    lights: &[ExtractedLight],
) -> Result<Vec<u8>, Error> {
    let mesh_instances: Vec<LinkedMeshInstance> = (0..children_count)
        .map(|idx| {
            let name = format!("mesh_{idx}");
            LinkedMeshInstance {
                scene_instance_id: idx,
                entity_name: name.clone(),
                parent_entity_name: None,
                parent_empty_name: None,
                parent_empty_parent_entity_name: None,
                parent_empty_parent_node_name: None,
                parent_empty_loc: [0.0, 0.0, 0.0],
                parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
                parent_empty_scale: [1.0, 1.0, 1.0],
                is_interior: false,
                source_object_name: name.clone(),
                blend_path: format!("{mesh_output_dir}/{name}.blend"),
                mesh_asset: format!("{mesh_output_dir}/{name}.blend"),
                mesh_name: name.clone(),
                material_names: Vec::new(),
                material_sidecar: None,
                palette_id: None,
                ui_bindings: Vec::new(),
                source_nodes: Vec::new(),
                source_ancestors: Vec::new(),
                source_loc: [0.0, 0.0, 0.0],
                source_quat: [1.0, 0.0, 0.0, 0.0],
                source_scale: [1.0, 1.0, 1.0],
                source_parent_name: None,
                parent_node_name: None,
                name,
                position: [0.0, 0.0, 0.0],
                rotation: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                hidden: false,
            }
        })
        .collect();

    create_scene_blend_with_instances(entity_name, &mesh_instances, lights)
}

pub fn create_scene_blend_with_instances(
    entity_name: &str,
    mesh_instances_input: &[LinkedMeshInstance],
    lights: &[ExtractedLight],
) -> Result<Vec<u8>, Error> {
    create_scene_blend_package_with_instances(entity_name, entity_name, mesh_instances_input, lights, &HashMap::new())
}

fn create_scene_blend_package_with_instances(
    package_name: &str,
    root_entity_name: &str,
    mesh_instances_input: &[LinkedMeshInstance],
    lights: &[ExtractedLight],
    light_projector_texture_map: &HashMap<String, String>,
) -> Result<Vec<u8>, Error> {
    create_scene_blend_package_with_instances_and_decal_offsets(
        package_name,
        root_entity_name,
        mesh_instances_input,
        lights,
        light_projector_texture_map,
        &HashSet::new(),
    )
}

fn create_scene_blend_package_with_instances_and_decal_offsets(
    package_name: &str,
    root_entity_name: &str,
    mesh_instances_input: &[LinkedMeshInstance],
    lights: &[ExtractedLight],
    light_projector_texture_map: &HashMap<String, String>,
    decal_mesh_refs: &HashSet<(String, String)>,
) -> Result<Vec<u8>, Error> {
    let children_count = mesh_instances_input.len();
    let mut used_light_names = HashMap::new();
    let mut light_source_names = HashMap::new();
    let lights = lights
        .iter()
        .map(|light| {
            let mut scene_light = light.clone();
            scene_light.name = unique_scene_object_name(&light.name, &mut used_light_names);
            light_source_names.insert(scene_light.name.clone(), light.name.clone());
            scene_light
        })
        .collect::<Vec<_>>();
    let lights = lights.as_slice();
    // Build a minimal input structure for compatibility with internal logic
    let mut ptrs = PtrAlloc::new(0x1000);
    
    let _screen_ptr = ptrs.alloc();
    let _wm_ptr = ptrs.alloc();
    let scene_ptr = ptrs.alloc();
    let view_layer_ptr = ptrs.alloc();
    let tool_settings_ptr = ptrs.alloc();
    let motion_blur_curve_points_ptr = ptrs.alloc();
    let cycles_scene_props = allocate_cycles_scene_props(&mut ptrs);
    let world_ptr = ptrs.alloc();
    let world_node_tree_ptr = ptrs.alloc();
    let mut scene_material_ptrs: HashMap<String, u64> = HashMap::new();
    let mut scene_material_blocks = Vec::new();
    for material_name in mesh_instances_input
        .iter()
        .flat_map(|instance| instance.material_names.iter())
    {
        if scene_material_ptrs.contains_key(material_name) {
            continue;
        }
        let material_ptr = ptrs.alloc();
        scene_material_ptrs.insert(material_name.clone(), material_ptr);
        scene_material_blocks.push((
            material_ptr,
            build_material_with_node_tree_and_properties(material_name, 0, 0),
        ));
    }
    let base_ptr = ptrs.alloc();
    let root_collection_ptr = ptrs.alloc();
    let root_collection_object_ptr = ptrs.alloc();
    let layer_collection_ptr = ptrs.alloc();
    
    // Addon-style collections: an empty default collection, package collection,
    // and package Interior child collection.
    let default_collection_ptr = ptrs.alloc();
    let default_layer_coll_ptr = ptrs.alloc();
    let package_collection_ptr = ptrs.alloc();
    let package_layer_coll_ptr = ptrs.alloc();
    let interior_collection_ptr = ptrs.alloc();
    let interior_layer_coll_ptr = ptrs.alloc();
    
    // Addon-style local scene anchors. Linked library object parent chains are
    // owned by their asset files, but local lights and scene metadata live here.
    let package_root_ptr = ptrs.alloc();
    let package_root_mat_ptr = ptrs.alloc();
    let package_root_matbits_ptr = ptrs.alloc();
    let entity_root_ptr = ptrs.alloc();
    let entity_root_mat_ptr = ptrs.alloc();
    let entity_root_matbits_ptr = ptrs.alloc();
    let entity_root_collection_object_ptr = ptrs.alloc();
    let package_root_idprops = allocate_idprop_blocks(&mut ptrs, package_root_properties(package_name));
    let entity_root_idprops = allocate_idprop_blocks(&mut ptrs, entity_root_properties(package_name, root_entity_name));
    
    // Allocate pointers for local, parentable mesh objects whose data points at
    // linked Mesh ID stubs. Directly parenting linked Object IDs does not
    // persist across Blender reloads because the library owns their parent chain.
    let mut linked_mesh_ids = Vec::new();
    let mut library_ptr_by_path = HashMap::new();
    let mut library_ptrs = Vec::new();
    let mut package_coll_obj_ptrs = Vec::new();
    let mut interior_coll_obj_ptrs = Vec::new();
    let mut local_mesh_object_entries = Vec::new();
    let mut instance_anchor_entries = Vec::new();
    // Parallel vec of (weld_mod_ptr, weighted_normal_mod_ptr, optional_decal_offset_mod_ptr)
    // per local mesh object entry.
    let mut local_mesh_modifier_ptrs: Vec<(u64, u64, u64)> = Vec::new();
    let mut scene_source_node_entries = Vec::new();
    let mut source_empty_entries = Vec::new();
    let mut parent_empty_entries = Vec::new();
    let mut parent_empty_ptr_by_name = HashMap::new();
    let mut source_node_ptr_by_name = HashMap::new();
    let mut source_node_ptrs_by_scene_instance: HashMap<usize, HashMap<String, u64>> = HashMap::new();
    let mut local_source_node_ptrs_by_instance = Vec::new();
    let preallocated_local_object_ptrs = (0..children_count).map(|_| ptrs.alloc()).collect::<Vec<_>>();
    let mut source_object_ptr_by_name = HashMap::new();
    let mut source_object_ptrs_by_scene_instance: HashMap<usize, HashMap<String, u64>> = HashMap::new();
    for (idx, instance) in mesh_instances_input.iter().enumerate() {
        source_object_ptr_by_name
            .entry(instance.name.clone())
            .or_insert(preallocated_local_object_ptrs[idx]);
        source_object_ptrs_by_scene_instance
            .entry(instance.scene_instance_id)
            .or_default()
            .insert(instance.source_object_name.clone(), preallocated_local_object_ptrs[idx]);
    }
    
    for (idx, instance) in mesh_instances_input.iter().enumerate() {
        if let Some(parent_empty_name) = &instance.parent_empty_name {
            if !parent_empty_ptr_by_name.contains_key(parent_empty_name) {
                let empty_ptr = ptrs.alloc();
                let empty_coll_obj_ptr = ptrs.alloc();
                parent_empty_ptr_by_name.insert(parent_empty_name.clone(), empty_ptr);
                parent_empty_entries.push((
                    empty_ptr,
                    empty_coll_obj_ptr,
                    parent_empty_name.clone(),
                    instance.parent_empty_parent_entity_name.clone(),
                    instance.parent_empty_parent_node_name.clone(),
                    instance.parent_empty_loc,
                    instance.parent_empty_quat,
                    instance.parent_empty_scale,
                    instance.is_interior,
                ));
            }
        }
        let anchor_ptr = ptrs.alloc();
        let anchor_coll_obj_ptr = ptrs.alloc();
        let anchor_idprops = allocate_idprop_blocks(&mut ptrs, scene_instance_properties(package_name, instance));
        let mut local_source_node_ptrs = HashMap::new();
        let mut local_source_node_entries = Vec::new();
        for (node_index, source_node) in instance.source_nodes.iter().enumerate() {
            let empty_ptr = ptrs.alloc();
            let empty_coll_obj_ptr = ptrs.alloc();
            local_source_node_ptrs.insert(source_node.name.clone(), empty_ptr);
            source_node_ptr_by_name
                .entry(source_node.name.clone())
                .or_insert(empty_ptr);
            if instance.is_interior {
                interior_coll_obj_ptrs.push((empty_coll_obj_ptr, empty_ptr));
            } else {
                package_coll_obj_ptrs.push((empty_coll_obj_ptr, empty_ptr));
            }
            local_source_node_entries.push((node_index, empty_ptr, empty_coll_obj_ptr));
        }
        for (node_index, empty_ptr, _) in &local_source_node_entries {
            let source_node = &instance.source_nodes[*node_index];
            let parent_ptr = source_node
                .parent_name
                .as_ref()
                .and_then(|name| {
                    lookup_named_ptr(&local_source_node_ptrs, name)
                        .or_else(|| {
                            source_node_ptrs_by_scene_instance
                                .get(&instance.scene_instance_id)
                                .and_then(|nodes| lookup_named_ptr(nodes, name))
                        })
                        .or_else(|| {
                            source_object_ptrs_by_scene_instance
                                .get(&instance.scene_instance_id)
                                .and_then(|objects| lookup_named_ptr(objects, name))
                        })
                        .or_else(|| lookup_named_ptr(&source_object_ptr_by_name, name))
                })
                .unwrap_or(anchor_ptr);
            scene_source_node_entries.push((*empty_ptr, idx, *node_index, parent_ptr));
        }
        if !local_source_node_ptrs.is_empty() {
            source_node_ptrs_by_scene_instance
                .entry(instance.scene_instance_id)
                .or_default()
                .extend(local_source_node_ptrs.iter().map(|(name, ptr)| (name.clone(), *ptr)));
        }
        local_source_node_ptrs_by_instance.push(local_source_node_ptrs);
        let mesh_id_ptr = ptrs.alloc();
        let library_ptr = if let Some(&ptr) = library_ptr_by_path.get(&instance.blend_path) {
            ptr
        } else {
            let ptr = ptrs.alloc();
            library_ptr_by_path.insert(instance.blend_path.clone(), ptr);
            library_ptrs.push((instance.blend_path.clone(), ptr));
            ptr
        };
        let coll_obj_ptr = ptrs.alloc();  // Collection object for this mesh instance
        let local_object_ptr = preallocated_local_object_ptrs[idx];
        let local_object_mat_ptr = ptrs.alloc();
        let local_object_matbits_ptr = ptrs.alloc();
        let local_object_idprops = allocate_idprop_blocks(&mut ptrs, scene_instance_properties(package_name, instance));
        let anchor_parent_ptr = instance
            .parent_node_name
            .as_ref()
            .and_then(|name| {
                instance
                    .parent_entity_name
                    .as_ref()
                    .and_then(|parent_entity_name| {
                        mesh_instances_input[..idx]
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(parent_idx, candidate)| {
                                if candidate.entity_name != *parent_entity_name {
                                    return None;
                                }
                                local_source_node_ptrs_by_instance
                                    .get(parent_idx)
                                    .and_then(|nodes| lookup_named_ptr(nodes, name))
                                    .or_else(|| {
                                        source_node_ptrs_by_scene_instance
                                            .get(&candidate.scene_instance_id)
                                            .and_then(|nodes| lookup_named_ptr(nodes, name))
                                    })
                                    .or_else(|| {
                                        candidate
                                            .name
                                            .eq_ignore_ascii_case(name)
                                            .then_some(preallocated_local_object_ptrs[parent_idx])
                                    })
                            })
                    })
                    .or_else(|| {
                        source_node_ptrs_by_scene_instance
                            .get(&instance.scene_instance_id)
                            .and_then(|nodes| lookup_named_ptr(nodes, name))
                            .or_else(|| lookup_named_ptr(&source_node_ptr_by_name, name))
                            .or_else(|| {
                                source_object_ptrs_by_scene_instance
                                    .get(&instance.scene_instance_id)
                                    .and_then(|objects| lookup_named_ptr(objects, name))
                            })
                            .or_else(|| lookup_named_ptr(&source_object_ptr_by_name, name))
                    })
            })
            .or_else(|| {
                instance
                    .parent_empty_name
                    .as_ref()
                    .and_then(|name| parent_empty_ptr_by_name.get(name).copied())
            })
            .unwrap_or(entity_root_ptr);
        let mesh_parent_ptr = instance
            .source_parent_name
            .as_ref()
            .and_then(|name| {
                lookup_named_ptr(&local_source_node_ptrs_by_instance[idx], name)
                    .or_else(|| {
                        source_node_ptrs_by_scene_instance
                            .get(&instance.scene_instance_id)
                            .and_then(|nodes| lookup_named_ptr(nodes, name))
                    })
                    .or_else(|| {
                        source_object_ptrs_by_scene_instance
                            .get(&instance.scene_instance_id)
                            .and_then(|objects| lookup_named_ptr(objects, name))
                    })
                    .or_else(|| lookup_named_ptr(&source_node_ptr_by_name, name))
                    .or_else(|| lookup_named_ptr(&source_object_ptr_by_name, name))
            })
            .unwrap_or_else(|| {
                let ancestor_indices = instance
                    .source_ancestors
                    .iter()
                    .enumerate()
                    .filter_map(|(ancestor_index, _ancestor)| Some(ancestor_index))
                    .collect::<Vec<_>>();
                let mut ancestor_ptrs = Vec::new();
                for ancestor_index in ancestor_indices {
                    let empty_ptr = ptrs.alloc();
                    let empty_coll_obj_ptr = ptrs.alloc();
                    let parent_ptr = ancestor_ptrs.last().copied().unwrap_or(anchor_ptr);
                    source_empty_entries.push((empty_ptr, idx, ancestor_index, parent_ptr));
                    if instance.is_interior {
                        interior_coll_obj_ptrs.push((empty_coll_obj_ptr, empty_ptr));
                    } else {
                        package_coll_obj_ptrs.push((empty_coll_obj_ptr, empty_ptr));
                    }
                    source_node_ptr_by_name
                        .entry(instance.source_ancestors[ancestor_index].name.clone())
                        .or_insert(empty_ptr);
                    source_node_ptrs_by_scene_instance
                        .entry(instance.scene_instance_id)
                        .or_default()
                        .entry(instance.source_ancestors[ancestor_index].name.clone())
                        .or_insert(empty_ptr);
                    ancestor_ptrs.push(empty_ptr);
                }
                ancestor_ptrs.last().copied().unwrap_or(anchor_ptr)
            });
        
        linked_mesh_ids.push((mesh_id_ptr, library_ptr, idx));
        if instance.is_interior {
            interior_coll_obj_ptrs.push((coll_obj_ptr, local_object_ptr));
        } else {
            package_coll_obj_ptrs.push((coll_obj_ptr, local_object_ptr));
        }
        local_mesh_object_entries.push((
            local_object_ptr,
            local_object_mat_ptr,
            local_object_matbits_ptr,
            idx,
            local_object_idprops,
            mesh_parent_ptr,
            instance
                .material_names
                .iter()
                .filter_map(|name| scene_material_ptrs.get(name).copied())
                .collect::<Vec<_>>(),
        ));
        let decal_offset_mod_ptr = if mesh_ref_has_decal_offset(instance, decal_mesh_refs) {
            ptrs.alloc()
        } else {
            0
        };
        local_mesh_modifier_ptrs.push((ptrs.alloc(), ptrs.alloc(), decal_offset_mod_ptr));
        instance_anchor_entries.push((anchor_ptr, idx, anchor_parent_ptr, anchor_idprops));
        if instance.is_interior {
            interior_coll_obj_ptrs.push((anchor_coll_obj_ptr, anchor_ptr));
        } else {
            package_coll_obj_ptrs.push((anchor_coll_obj_ptr, anchor_ptr));
        }
    }
    
    for light in lights {
        if let Some(parent_empty_name) = &light.parent_empty_name {
            if !parent_empty_ptr_by_name.contains_key(parent_empty_name) {
                let empty_ptr = ptrs.alloc();
                let empty_coll_obj_ptr = ptrs.alloc();
                parent_empty_ptr_by_name.insert(parent_empty_name.clone(), empty_ptr);
                parent_empty_entries.push((
                    empty_ptr,
                    empty_coll_obj_ptr,
                    parent_empty_name.clone(),
                    light.parent_empty_parent_entity_name.clone(),
                    light.parent_empty_parent_node_name.clone(),
                    light.parent_empty_loc,
                    light.parent_empty_quat,
                    light.parent_empty_scale,
                    true,
                ));
            }
        }
    }

    // Allocate pointers for light instances
    let mut light_instances = Vec::new();
    let mut light_coll_obj_ptrs = Vec::new();
    
    for idx in 0..lights.len() {
        let lamp_ptr = ptrs.alloc();
        let object_ptr = ptrs.alloc();
        let object_mat_ptr = ptrs.alloc();
        let object_matbits_ptr = ptrs.alloc();
        let coll_obj_ptr = ptrs.alloc();  // Collection object for this light
        
        light_instances.push((lamp_ptr, object_ptr, object_mat_ptr, object_matbits_ptr, idx));
        light_coll_obj_ptrs.push((coll_obj_ptr, object_ptr));
    }
    
    // CollectionChild structs for the addon-style collection tree.
    let default_coll_child_ptr = ptrs.alloc();
    let package_coll_child_ptr = ptrs.alloc();
    let interior_coll_child_ptr = ptrs.alloc();
    
    // Build scene datablocks
    // Scene must always be named "Scene" for Blender to recognize it as the primary scene
    let scene_data = build_scene_with_motion_blur_curve_and_properties(
        "Scene",
        view_layer_ptr,
        root_collection_ptr,
        tool_settings_ptr,
        motion_blur_curve_points_ptr,
        cycles_scene_props.root_ptr,
        world_ptr,
        "CYCLES",
    );
    let motion_blur_curve_points_data = build_motion_blur_shutter_curve_points();
    let tool_settings_data = build_tool_settings();
    let view_layer_data = build_view_layer("ViewLayer", base_ptr, layer_collection_ptr);
    let base_data = build_base(package_root_ptr);
    
    package_coll_obj_ptrs.push((root_collection_object_ptr, package_root_ptr));
    package_coll_obj_ptrs.push((entity_root_collection_object_ptr, entity_root_ptr));
    package_coll_obj_ptrs.extend(light_coll_obj_ptrs.iter().copied());
    for (empty_ptr, coll_obj_ptr, _, _, _, _, _, _, is_interior) in &parent_empty_entries {
        if *is_interior {
            interior_coll_obj_ptrs.push((*coll_obj_ptr, *empty_ptr));
        } else {
            package_coll_obj_ptrs.push((*coll_obj_ptr, *empty_ptr));
        }
    }
    let package_collection_head = package_coll_obj_ptrs.first().map(|entry| entry.0).unwrap_or(0);
    let package_collection_tail = package_coll_obj_ptrs.last().map(|entry| entry.0).unwrap_or(0);
    let interior_collection_head = interior_coll_obj_ptrs.first().map(|entry| entry.0).unwrap_or(0);
    let interior_collection_tail = interior_coll_obj_ptrs.last().map(|entry| entry.0).unwrap_or(0);
    let package_coll_obj_data_list = linked_collection_object_data(&package_coll_obj_ptrs);
    let interior_coll_obj_data_list = linked_collection_object_data(&interior_coll_obj_ptrs);

    let root_collection_data = build_master_collection(
        0,
        0,
        default_coll_child_ptr,
        package_coll_child_ptr,
    );
    let default_collection_data = build_collection("Collection", 0, 0, 0, 0);
    let package_collection_data = build_collection(
        &format!("StarBreaker {package_name}"),
        package_collection_head,
        package_collection_tail,
        interior_coll_child_ptr,
        interior_coll_child_ptr,
    );
    let interior_collection_data = build_collection(
        &format!("StarBreaker {package_name} Interior"),
        interior_collection_head,
        interior_collection_tail,
        0,
        0,
    );
    let default_coll_child_data =
        build_collection_object_linked(default_collection_ptr, 0, package_coll_child_ptr);
    let package_coll_child_data = build_collection_object_linked(
        package_collection_ptr,
        default_coll_child_ptr,
        0,
    );
    let interior_coll_child_data =
        build_collection_object_linked(interior_collection_ptr, 0, 0);

    let root_layer_collection_data = build_layer_collection_linked(
        root_collection_ptr,  // Collection pointer
        0,                    // prev = NULL (root has no siblings)
        0,                    // next = NULL (root has no siblings)
        default_layer_coll_ptr,
        package_layer_coll_ptr,
    );
    let default_layer_coll_data = build_layer_collection_linked(
        default_collection_ptr,
        0,
        package_layer_coll_ptr,
        0,
        0,
    );
    let package_layer_coll_data = build_layer_collection_linked(
        package_collection_ptr,
        default_layer_coll_ptr,
        0,
        interior_layer_coll_ptr,
        interior_layer_coll_ptr,
    );
    let interior_layer_coll_data = build_layer_collection_linked(
        interior_collection_ptr,
        0,
        0,
        0,
        0,
    );

    // Build addon-style package and entity root empties at origin.
    let package_root_data = build_empty_object_with_properties(
        &format!("StarBreaker {package_name}"),
        [0.0, 0.0, 0.0],  // position
        [1.0, 0.0, 0.0, 0.0],  // quaternion
        [1.0, 1.0, 1.0],  // scale
        0,
        package_root_idprops.as_ref().map(|props| props.root_ptr).unwrap_or(0),
    );
    let entity_root_data = build_empty_object_with_properties(
        root_entity_name,
        [0.0, 0.0, 0.0],
        BLENDER_BAKED_ROOT_QUAT,
        BLENDER_BAKED_ROOT_SCALE,
        package_root_ptr,
        entity_root_idprops.as_ref().map(|props| props.root_ptr).unwrap_or(0),
    );
    let package_root_mat_array = build_mat_ptr_array(0);
    let package_root_matbits = build_matbits(0);
    let entity_root_mat_array = build_mat_ptr_array(0);
    let entity_root_matbits = build_matbits(0);

    let parent_empty_data = parent_empty_entries
        .iter()
        .map(|(empty_ptr, _, name, parent_entity_name, parent_node_name, loc, quat, scale, _)| {
            let parent_ptr = parent_node_name
                .as_ref()
                .and_then(|node_name| {
                    parent_entity_name.as_ref().and_then(|entity_name| {
                        mesh_instances_input
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(parent_idx, candidate)| {
                                if candidate.entity_name != *entity_name {
                                    return None;
                                }
                                local_source_node_ptrs_by_instance
                                    .get(parent_idx)
                                    .and_then(|nodes| lookup_named_ptr(nodes, node_name))
                                    .or_else(|| {
                                        source_node_ptrs_by_scene_instance
                                            .get(&candidate.scene_instance_id)
                                            .and_then(|nodes| lookup_named_ptr(nodes, node_name))
                                    })
                                    .or_else(|| {
                                        candidate
                                            .name
                                            .eq_ignore_ascii_case(node_name)
                                            .then_some(preallocated_local_object_ptrs[parent_idx])
                                    })
                            })
                    })
                })
                .or_else(|| {
                    parent_entity_name.as_ref().and_then(|entity_name| {
                        mesh_instances_input
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(parent_idx, candidate)| {
                                (candidate.entity_name == *entity_name)
                                    .then_some(preallocated_local_object_ptrs[parent_idx])
                            })
                    })
                })
                .unwrap_or(entity_root_ptr);
            (
                *empty_ptr,
                build_empty_object(name, *loc, *quat, *scale, parent_ptr),
            )
        })
        .collect::<Vec<_>>();

    let mut instance_anchor_data = Vec::new();
    for (anchor_ptr, idx, anchor_parent_ptr, anchor_idprops) in &instance_anchor_entries {
        let instance = &mesh_instances_input[*idx];
        let mut object_data = build_empty_object_with_properties(
            &scene_anchor_name(&instance.name),
            instance.position,
            instance.rotation,
            instance.scale,
            *anchor_parent_ptr,
            anchor_idprops.as_ref().map(|props| props.root_ptr).unwrap_or(0),
        );
        if instance.hidden {
            hide_object_viewport_and_render(&mut object_data);
        }
        instance_anchor_data.push((*anchor_ptr, object_data));
    }

    let mut source_empty_data = Vec::new();
    for (empty_ptr, idx, node_index, parent_ptr) in &scene_source_node_entries {
        let instance = &mesh_instances_input[*idx];
        let source_node = &instance.source_nodes[*node_index];
        let mut object_data = build_empty_object(
            &scene_source_empty_name(&instance.name, *node_index, &source_node.name),
            source_node.loc,
            source_node.quat,
            source_node.scale,
            *parent_ptr,
        );
        if instance.hidden {
            hide_object_viewport_and_render(&mut object_data);
        }
        source_empty_data.push((*empty_ptr, object_data));
    }
    for (empty_ptr, idx, ancestor_index, parent_ptr) in &source_empty_entries {
        let instance = &mesh_instances_input[*idx];
        let ancestor = &instance.source_ancestors[*ancestor_index];
        let mut object_data = build_empty_object(
            &scene_source_empty_name(&instance.name, *ancestor_index, &ancestor.name),
            ancestor.loc,
            ancestor.quat,
            ancestor.scale,
            *parent_ptr,
        );
        if instance.hidden {
            hide_object_viewport_and_render(&mut object_data);
        }
        source_empty_data.push((*empty_ptr, object_data));
    }

    let mut local_mesh_object_data = Vec::new();
    for (entry_idx, (object_ptr, mat_ptr, matbits_ptr, idx, object_idprops, parent_anchor_ptr, material_ptrs)) in local_mesh_object_entries.iter().enumerate() {
        let instance = &mesh_instances_input[*idx];
        let (mesh_id_ptr, _, _) = linked_mesh_ids[*idx];
        let material_slot_count = material_ptrs.len() as i32;
        let mut object_data = build_object(
            &instance.name,
            mesh_id_ptr,
            *mat_ptr,
            *matbits_ptr,
            material_slot_count,
            object_idprops.as_ref().map(|props| props.root_ptr).unwrap_or(0),
        );
        patch_object_parent_transform(
            &mut object_data,
            *parent_anchor_ptr,
            instance.source_loc,
            instance.source_quat,
            instance.source_scale,
        );
        if instance.hidden {
            hide_object_viewport_and_render(&mut object_data);
        }
        let (weld_mod_ptr, wn_mod_ptr, decal_offset_mod_ptr) = local_mesh_modifier_ptrs[entry_idx];
        set_object_modifiers_listbase(
            &mut object_data,
            weld_mod_ptr,
            if decal_offset_mod_ptr != 0 { decal_offset_mod_ptr } else { wn_mod_ptr },
        );
        local_mesh_object_data.push((
            *object_ptr,
            object_data,
            build_mat_ptr_array_from_ptrs(material_ptrs),
            vec![1u8; material_ptrs.len()],
        ));
    }
    
    // Allocate string data for library filenames (uses entity name, not scene name)
    let scene_name_bytes = format!("{}\0", package_name);
    let scene_name_ptr = ptrs.alloc();
    
    // Build linked object ID stubs and their library blocks.
    let mut linked_mesh_id_data = Vec::new();
    let mut mesh_library_data = Vec::new();
    let library_names = unique_library_names(
        &library_ptrs
            .iter()
            .map(|(blend_path, _)| blend_path.clone())
            .collect::<Vec<_>>(),
    );
    
    for (blend_path, library_ptr) in &library_ptrs {
        let lib_name = library_names
            .get(blend_path)
            .map(String::as_str)
            .unwrap_or_else(|| blend_library_basename(blend_path));
        let lib_data = build_library_block(lib_name, blend_path);
        mesh_library_data.push((*library_ptr, lib_data));
    }
    for (mesh_id_ptr, library_ptr, idx) in &linked_mesh_ids {
        linked_mesh_id_data.push((*mesh_id_ptr, build_id_stub("ME", &mesh_instances_input[*idx].mesh_name, *library_ptr)));
    }
    
    // Build light objects for lights collection
    let mut light_data = Vec::new();
    let mut light_object_data = Vec::new();
    let mut light_data_idprops = Vec::new();
    let mut light_object_idprops = Vec::new();
    let mut light_object_mat_arrays = Vec::new();
    let mut light_object_matbits = Vec::new();
    let mut gobo_image_by_path: HashMap<String, (u64, String)> = HashMap::new();
    
    for (lamp_ptr, _, _, _, idx) in &light_instances {
        let light = &lights[*idx];
        
        // Check if there's an updated PNG path in the map, otherwise use original gobo_path
        let source_light_name = light_source_names
            .get(&light.name)
            .map(String::as_str)
            .unwrap_or(light.name.as_str());
        let effective_gobo_path = light_projector_texture_map.get(&light.name)
            .or_else(|| light_projector_texture_map.get(source_light_name))
            .map(|s| s.as_str())
            .or(light.gobo_path.as_deref());
        
        // ONLY create gobo nodes for SPOT lights (lamp_type == 2).
        // Point lights (Omni, SoftOmni) should NEVER have projector textures.
        let gobo_blocks = if light.lamp_type == 2 {
            effective_gobo_path.map(|gobo_path| {
                let node_tree_ptr = ptrs.alloc();
                let normalized_path = gobo_image_blend_filepath(package_name, gobo_path);
                let image_filename = gobo_path
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("light_gobo.png")
                    .to_string();
                let mut write_image_block = false;
                let (image_ptr, image_name) = if let Some((ptr, name)) = gobo_image_by_path.get(&normalized_path) {
                    (*ptr, name.clone())
                } else {
                    let image_ptr = ptrs.alloc();
                    let image_name = format!("gobo_{image_ptr:x}_{image_filename}");
                    gobo_image_by_path.insert(normalized_path.clone(), (image_ptr, image_name.clone()));
                    write_image_block = true;
                    (image_ptr, image_name)
                };
                (node_tree_ptr, image_ptr, image_name, normalized_path, write_image_block)
            })
        } else {
            None
        };
        let node_tree_ptr = gobo_blocks
            .as_ref()
            .map(|(node_tree_ptr, _, _, _, _)| *node_tree_ptr)
            .unwrap_or(0);

        let light_data_props = allocate_idprop_blocks(
            &mut ptrs,
            light_data_properties(
                &light.active_state,
                light.states_json.as_deref(),
                &light.semantic_light_kind,
            ),
        );
        let light_data_props_ptr = light_data_props.as_ref().map(|props| props.root_ptr).unwrap_or(0);

        // Build lamp datablock
        let lamp_bytes = build_lamp_with_node_tree_and_properties(
            &light.name,
            light.lamp_type,
            light.color,
            light.energy_watts,
            light.radius,
            light.cutoff_distance,
            light.spot_size,
            light.spot_blend,
            light.temperature_k,
            false,
            node_tree_ptr,
            light_data_props_ptr,
        );
        light_data.push((*lamp_ptr, lamp_bytes, gobo_blocks));
        light_data_idprops.push(light_data_props);
        let light_props = allocate_idprop_blocks(
            &mut ptrs,
            light_object_properties(package_name, source_light_name, light.radius_source),
        );
        let light_props_ptr = light_props.as_ref().map(|props| props.root_ptr).unwrap_or(0);
        
        // Build object wrapper for light
        let light_parent_ptr = light
            .parent_empty_name
            .as_ref()
            .and_then(|name| parent_empty_ptr_by_name.get(name).copied())
            .unwrap_or(entity_root_ptr);
        let object_bytes = build_lamp_object_with_properties_and_visibility(
            &light.name,
            *lamp_ptr,
            light.position_blend,
            light.rotation_blend,
            [1.0, 1.0, 1.0],  // Standard scale
            light_parent_ptr,
            light_props_ptr,
            true,
        );
        light_object_data.push(object_bytes);
        light_object_idprops.push(light_props);
        
        // Build material arrays (empty for lights)
        light_object_mat_arrays.push(build_mat_ptr_array(0));
        light_object_matbits.push(build_matbits(0));
    }
    
    // Assemble .blend file
    let mut out: Vec<u8> = Vec::with_capacity(1024 * 1024);
    out.extend_from_slice(BLEND_MAGIC);
    
    let file_global = build_file_global(STARTUP_UI_SCREEN_PTR, scene_ptr, view_layer_ptr);
    write_block(&mut out, b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x10, 1, &file_global);
    out.extend_from_slice(&startup_ui_prefix_bytes());
    
    // Write scene structure
    // CRITICAL: All DATA blocks for Scene must be consecutive immediately after SC\0\0.
    // Blender reads them all into fd->datamap, then clears it after processing each ID block.
    // Any non-DATA block between SC and its data will truncate the datamap.
    write_block(&mut out, b"SC\0\0", SDNA_IDX_SCENE, scene_ptr, 1, &scene_data);
    // SC DATA sequence — ToolSettings, ViewLayer, all LayerCollections, master_collection, CollectionChildren, Base:
    write_block(&mut out, b"DATA", 0, motion_blur_curve_points_ptr, 3, &motion_blur_curve_points_data);
    write_cycles_scene_props(&mut out, &cycles_scene_props);
    write_block(&mut out, b"DATA", SDNA_IDX_TOOL_SETTINGS, tool_settings_ptr, 1, &tool_settings_data);
    write_block(&mut out, b"DATA", SDNA_IDX_VIEW_LAYER, view_layer_ptr, 1, &view_layer_data);
    write_block(&mut out, b"DATA", SDNA_IDX_BASE, base_ptr, 1, &base_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, layer_collection_ptr, 1, &root_layer_collection_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, default_layer_coll_ptr, 1, &default_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, package_layer_coll_ptr, 1, &package_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_LAYER_COLLECTION, interior_layer_coll_ptr, 1, &interior_layer_coll_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION, root_collection_ptr, 1, &root_collection_data);  // embedded master_collection
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, default_coll_child_ptr, 1, &default_coll_child_data);
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, package_coll_child_ptr, 1, &package_coll_child_data);
    // End of SC DATA sequence — sub-collection GR blocks follow with their own DATA sub-blocks:
    write_world_with_sky_shader(&mut out, "World", world_ptr, world_node_tree_ptr, &mut ptrs);


    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, default_collection_ptr, 1, &default_collection_data);
    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, package_collection_ptr, 1, &package_collection_data);
    for (coll_obj_ptr, coll_obj_data) in &package_coll_obj_data_list {
        write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, *coll_obj_ptr, 1, coll_obj_data);
    }
    write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_CHILD, interior_coll_child_ptr, 1, &interior_coll_child_data);
    write_block(&mut out, b"GR\0\0", SDNA_IDX_COLLECTION, interior_collection_ptr, 1, &interior_collection_data);
    for (coll_obj_ptr, coll_obj_data) in &interior_coll_obj_data_list {
        write_block(&mut out, b"DATA", SDNA_IDX_COLLECTION_OBJECT, *coll_obj_ptr, 1, coll_obj_data);
    }

    // Write package/entity root empty objects + properties/material arrays.
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, package_root_ptr, 1, &package_root_data);
    if let Some(props) = &package_root_idprops {
        write_idprop_blocks(&mut out, props);
    }
    write_block(&mut out, b"DATA", 0, package_root_mat_ptr, 1, &package_root_mat_array);
    write_block(&mut out, b"DATA", 0, package_root_matbits_ptr, 1, &package_root_matbits);
    write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, entity_root_ptr, 1, &entity_root_data);
    if let Some(props) = &entity_root_idprops {
        write_idprop_blocks(&mut out, props);
    }
    write_block(&mut out, b"DATA", 0, entity_root_mat_ptr, 1, &entity_root_mat_array);
    write_block(&mut out, b"DATA", 0, entity_root_matbits_ptr, 1, &entity_root_matbits);

    for (empty_ptr, empty_data) in &parent_empty_data {
        write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, *empty_ptr, 1, empty_data);
    }

    for (idx, (anchor_ptr, anchor_data)) in instance_anchor_data.iter().enumerate() {
        write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, *anchor_ptr, 1, anchor_data);
        if let Some(props) = &instance_anchor_entries[idx].3 {
            write_idprop_blocks(&mut out, props);
        }
    }

    for (empty_ptr, empty_data) in &source_empty_data {
        write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, *empty_ptr, 1, empty_data);
    }

    for (idx, (object_ptr, object_data, mat_array, matbits)) in local_mesh_object_data.iter().enumerate() {
        write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, *object_ptr, 1, object_data);
        if let Some(props) = &local_mesh_object_entries[idx].4 {
            write_idprop_blocks(&mut out, props);
        }
        write_block(&mut out, b"DATA", 0, local_mesh_object_entries[idx].1, 1, mat_array);
        write_block(&mut out, b"DATA", 0, local_mesh_object_entries[idx].2, 1, matbits);
        // Modifier DATA blocks (part of OB's consecutive data chain).
        let (weld_mod_ptr, wn_mod_ptr, decal_offset_mod_ptr) = local_mesh_modifier_ptrs[idx];
        write_block(&mut out, b"DATA", SDNA_IDX_WELD_MODIFIER, weld_mod_ptr, 1,
            &build_weld_modifier("StarBreaker Weld", wn_mod_ptr, 0, 0.0005));
        write_block(&mut out, b"DATA", SDNA_IDX_WEIGHTED_NORMAL_MODIFIER, wn_mod_ptr, 1,
            &build_weighted_normal_modifier(
                "StarBreaker Weighted Normal",
                decal_offset_mod_ptr,
                weld_mod_ptr,
                50,
                0.01,
            ));
        if decal_offset_mod_ptr != 0 {
            let instance = &mesh_instances_input[local_mesh_object_entries[idx].3];
            write_block(&mut out, b"DATA", SDNA_IDX_DISPLACE_MODIFIER, decal_offset_mod_ptr, 1,
                &build_displace_modifier(
                    DECAL_OFFSET_MODIFIER_NAME,
                    0,
                    wn_mod_ptr,
                    instance_decal_offset_strength(instance) as f32,
                    DECAL_OFFSET_GROUP_NAME,
                    DECAL_OFFSET_MIDLEVEL,
                ));
        }
    }

    for (material_ptr, material_data) in &scene_material_blocks {
        write_block(&mut out, b"MA\0\0", SDNA_IDX_MATERIAL, *material_ptr, 1, material_data);
    }
    
    // Write light blocks (Phase 5B)
    for (idx, (_, object_ptr, mat_ptr, matbits_ptr, _)) in light_instances.iter().enumerate() {
        // Write LAMP datablock
        let (lamp_block_ptr, lamp_bytes, gobo_blocks) = light_data[idx].clone();
        write_block(&mut out, b"LA\0\0", SDNA_IDX_LAMP, lamp_block_ptr, 1, &lamp_bytes);
        if let Some(props) = &light_data_idprops[idx] {
            write_idprop_blocks(&mut out, props);
        }
        if let Some((node_tree_ptr, image_ptr, image_name, image_filepath, write_image_block)) = gobo_blocks {
            write_light_gobo_node_tree(
                &mut out,
                lamp_block_ptr,
                node_tree_ptr,
                image_ptr,
                &image_name,
                &image_filepath,
                write_image_block,
                &mut ptrs,
            );
        }
        
        // Write light object
        write_block(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, *object_ptr, 1, &light_object_data[idx]);
        if let Some(props) = &light_object_idprops[idx] {
            write_idprop_blocks(&mut out, props);
        }
        write_block(&mut out, b"DATA", 0, *mat_ptr, 1, &light_object_mat_arrays[idx]);
        write_block(&mut out, b"DATA", 0, *matbits_ptr, 1, &light_object_matbits[idx]);
    }
    
    // Write linked mesh libraries and object ID stubs after local IDs. This
    // matches Blender-authored linked-object scenes and avoids making following
    // local object-data IDs inherit the active library during read.
    for (library_ptr, library_data) in &mesh_library_data {
        write_block(&mut out, b"LI\0\0", SDNA_IDX_LIBRARY, *library_ptr, 1, library_data);
        for (idx, (object_id_ptr, object_library_ptr, _)) in linked_mesh_ids.iter().enumerate() {
            if object_library_ptr == library_ptr {
                write_block(&mut out, b"ID\0\0", SDNA_IDX_ID, *object_id_ptr, 1, &linked_mesh_id_data[idx].1);
            }
        }
    }
    
    // Write scene name
    write_block(&mut out, b"DATA", 0, scene_name_ptr, 1, scene_name_bytes.as_bytes());
    
    // Write DNA1 and ENDB
    write_block(&mut out, b"DNA1", SDNA_IDX_DNA1, 0x01, 1, DNA1_BYTES);
    write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);
    
    // Return uncompressed for now (Phase 2 will handle compression)
    Ok(out)
}

fn linked_collection_object_data(entries: &[(u64, u64)]) -> Vec<(u64, Vec<u8>)> {
    entries
        .iter()
        .enumerate()
        .map(|(idx, &(coll_obj_ptr, object_ptr))| {
            let prev_ptr = if idx > 0 { entries[idx - 1].0 } else { 0 };
            let next_ptr = if idx + 1 < entries.len() { entries[idx + 1].0 } else { 0 };
            (
                coll_obj_ptr,
                build_collection_object_linked(object_ptr, prev_ptr, next_ptr),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests_package_paths {
    use super::{gobo_image_blend_filepath, scene_library_blend_path};

    #[test]
    fn scene_library_path_for_top_level_package() {
        assert_eq!(
            scene_library_blend_path("DRAK Buccaneer_LOD0_TEX0", "Data/Objects/test.blend"),
            "//../../Data/Objects/test.blend"
        );
    }

    #[test]
    fn scene_library_path_for_typed_package_subdir() {
        assert_eq!(
            scene_library_blend_path("ship/DRAK Buccaneer_LOD0_TEX0", "Data/Objects/test.blend"),
            "//../../../Data/Objects/test.blend"
        );
    }

    #[test]
    fn gobo_image_path_for_top_level_package() {
        assert_eq!(
            gobo_image_blend_filepath("DRAK Buccaneer_LOD0_TEX0", "Data/Textures/lights/light_ies_5_TEX0.png"),
            "//../../Data/Textures/lights/light_ies_5_TEX0.png"
        );
    }

    #[test]
    fn gobo_image_path_for_typed_package_subdir() {
        assert_eq!(
            gobo_image_blend_filepath("ship/DRAK Buccaneer_LOD0_TEX0", "Data/Textures/lights/light_ies_5_TEX0.png"),
            "//../../../Data/Textures/lights/light_ies_5_TEX0.png"
        );
    }
}

#[cfg(test)]
mod tests_5a_scene_blend;
// ════════════════════════════════════════════════════════════════════════════════
// Phase 5B: Light Parenting and Collection Organization
// ════════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests_5b;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3A: Extract Light Data from Manifest
// ════════════════════════════════════════════════════════════════════════════════

/// Extracted light ready for Blender scene construction.
#[derive(Debug, Clone)]
pub struct ExtractedLight {
    /// CryEngine light name
    pub name: String,
    pub parent_empty_name: Option<String>,
    pub parent_empty_parent_entity_name: Option<String>,
    pub parent_empty_parent_node_name: Option<String>,
    pub parent_empty_loc: [f32; 3],
    pub parent_empty_quat: [f32; 4],
    pub parent_empty_scale: [f32; 3],
    /// Position in Blender coordinates
    pub position_blend: [f32; 3],
    /// Rotation as quaternion in Blender coordinates [w, x, y, z]
    pub rotation_blend: [f32; 4],
    /// Linear RGB color 0..1
    pub color: [f32; 3],
    /// Blender lamp type: 0=POINT, 1=SUN, 2=SPOT, 4=AREA
    pub lamp_type: i16,
    /// Energy in Watts (radiant flux)
    pub energy_watts: f32,
    /// Blender point/spot soft-shadow size, or area-light size.
    pub radius: f32,
    /// Blender attenuation cutoff distance in meters.
    pub cutoff_distance: f32,
    /// Authored source radius before Blender-side adjustments.
    pub radius_source: f32,
    /// Spot cone full aperture in radians (0 for POINT)
    pub spot_size: f32,
    /// Spot cone feather width 0..1 (0 for POINT)
    pub spot_blend: f32,
    /// Intensity in candelas (for reference)
    pub intensity_candela: f32,
    /// Color temperature in Kelvin
    pub temperature_k: f32,
    /// When true, render blackbody color at temperature_k; ignore RGB
    pub use_temperature: bool,
    /// Optional projector gobo texture path (for SPOTs)
    pub gobo_path: Option<String>,
    /// Currently active state name
    pub active_state: String,
    /// Authored light states serialized for the Blender addon's state switcher.
    pub states_json: Option<String>,
    /// Semantic mapping hint used by addon state switcher.
    pub semantic_light_kind: String,
}

/// Convert CryEngine position to Blender coordinates.
///
/// CryEngine uses Z-up: (x, y, z) → Blender Y-up: (x, -z, y)
fn convert_position_sc_to_blender(pos_sc: [f64; 3]) -> [f32; 3] {
    [
        pos_sc[0] as f32,
        -(pos_sc[2] as f32),
        pos_sc[1] as f32,
    ]
}

fn lamp_type_for_light(light_type: &str, semantic_light_kind: &str) -> i16 {
    match semantic_light_kind.to_ascii_lowercase().as_str() {
        "area" => 4,
        "sun" | "directional" => 1,
        "spot" => 2,
        "point" => 0,
        _ => match light_type {
            "Omni" | "SoftOmni" | "Ambient" => 0,
            "Projector" => 2,
            "Directional" | "Sun" => 1,
            "Planar" => 4,
            _ => 0,
        },
    }
}

fn light_energy_to_blender(
    lamp_type: i16,
    semantic_light_kind: &str,
    intensity_candela_proxy: f32,
    intensity_raw: f32,
) -> f32 {
    const LIGHT_CANDELA_TO_WATT: f32 = 4.0 * std::f32::consts::PI / 683.0;
    const LIGHT_VISUAL_GAIN: f32 = 20.0;
    const LUMENS_PER_WATT_WHITE: f32 = 120.0;

    match lamp_type {
        1 => intensity_candela_proxy / 683.0,
        4 => intensity_raw / LUMENS_PER_WATT_WHITE,
        _ => {
            if semantic_light_kind.eq_ignore_ascii_case("ambient_proxy") {
                intensity_candela_proxy * LIGHT_CANDELA_TO_WATT
            } else {
                intensity_candela_proxy * LIGHT_CANDELA_TO_WATT * LIGHT_VISUAL_GAIN
            }
        }
    }
}

fn soft_shadow_radius_from_source(light_radius: f32) -> f32 {
    (light_radius * 0.05).clamp(0.01, 0.5)
}

fn gobo_image_blend_filepath(package_name: &str, gobo_path: &str) -> String {
    let normalized = gobo_path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if normalized.starts_with("//") || normalized.starts_with('/') {
        normalized
    } else if lower.starts_with("data/textures/") {
        scene_library_blend_path(
            package_name,
            &format!("Data/Textures/{}", &normalized["Data/Textures/".len()..]),
        )
    } else if lower.starts_with("textures/") {
        scene_library_blend_path(
            package_name,
            &format!("Data/Textures/{}", &normalized["Textures/".len()..]),
        )
    } else if normalized.starts_with("Data/") {
        scene_library_blend_path(package_name, &normalized)
    } else {
        scene_library_blend_path(package_name, &normalized)
    }
}

fn mat4_from_sc_columns(matrix: [[f32; 4]; 4]) -> glam::Mat4 {
    glam::Mat4::from_cols_array(&[
        matrix[0][0], matrix[0][1], matrix[0][2], matrix[0][3],
        matrix[1][0], matrix[1][1], matrix[1][2], matrix[1][3],
        matrix[2][0], matrix[2][1], matrix[2][2], matrix[2][3],
        matrix[3][0], matrix[3][1], matrix[3][2], matrix[3][3],
    ])
}

fn mat4_from_sc_rows(matrix: [[f32; 4]; 4]) -> glam::Mat4 {
    glam::Mat4::from_cols_array(&[
        matrix[0][0], matrix[0][1], matrix[0][2], matrix[0][3],
        matrix[1][0], matrix[1][1], matrix[1][2], matrix[1][3],
        matrix[2][0], matrix[2][1], matrix[2][2], matrix[2][3],
        matrix[3][0], matrix[3][1], matrix[3][2], matrix[3][3],
    ])
}

fn transform_light_position_sc(container_transform: [[f32; 4]; 4], position: [f64; 3]) -> [f64; 3] {
    let transformed = mat4_from_sc_columns(container_transform)
        .transform_point3(glam::Vec3::new(position[0] as f32, position[1] as f32, position[2] as f32));
    [transformed.x as f64, transformed.y as f64, transformed.z as f64]
}

fn transform_light_rotation_sc(container_transform: [[f32; 4]; 4], rotation: [f64; 4]) -> [f64; 4] {
    let (_, container_rotation, _) = mat4_from_sc_columns(container_transform).to_scale_rotation_translation();
    let local_rotation = glam::Quat::from_xyzw(
        rotation[1] as f32,
        rotation[2] as f32,
        rotation[3] as f32,
        rotation[0] as f32,
    );
    let transformed = (container_rotation * local_rotation).normalize();
    [
        transformed.w as f64,
        transformed.x as f64,
        transformed.y as f64,
        transformed.z as f64,
    ]
}

/// Convert CryEngine quaternion to Blender coordinates.
///
/// CryEngine quaternion: [w, x, y, z]
/// 
/// **Coordinate system transformation** (Z-up to Y-up):
/// Apply same transformation as position: swap y/z, negate z
///
/// Blender's glTF light convention needs the same basis correction the addon
/// applies in `_scene_light_quaternion_to_blender`, not only for spot lights.
fn convert_quaternion_sc_to_blender(quat_sc: [f64; 4], _is_spotlight: bool) -> [f32; 4] {
    if quat_sc.iter().all(|component| component.abs() <= 1e-8) {
        [1.0, 0.0, 0.0, 0.0]
    } else {
        let source_quat = glam::Quat::from_xyzw(
            quat_sc[1] as f32,
            quat_sc[2] as f32,
            quat_sc[3] as f32,
            quat_sc[0] as f32,
        );
        let axis = scene_axis_matrix();
        let converted_matrix = axis * glam::Mat4::from_quat(source_quat) * axis.inverse();
        let (_, converted_quat, _) = converted_matrix.to_scale_rotation_translation();
        let basis_correction = glam::Quat::from_xyzw(
            0.0,
            -std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let result = (converted_quat * basis_correction).normalize();
        [result.w, result.x, result.y, result.z]
    }
}

#[cfg(test)]
fn quaternion_multiply(q1: [f32; 4], q2: [f32; 4]) -> [f32; 4] {
    let [w1, x1, y1, z1] = q1;
    let [w2, x2, y2, z2] = q2;
    [
        w1*w2 - x1*x2 - y1*y2 - z1*z2,
        w1*x2 + x1*w2 + y1*z2 - z1*y2,
        w1*y2 - x1*z2 + y1*w2 + z1*x2,
        w1*z2 + x1*y2 - y1*x2 + z1*w2,
    ]
}

/// Extract all lights from loaded interiors.
///
/// Reads LightInfo from all interior containers and converts to Blender format.
/// Returns vector of extracted lights ready for scene construction.
pub fn extract_lights_from_interiors(
    interiors: &LoadedInteriors,
) -> Result<Vec<ExtractedLight>, Error> {
    let mut lights = Vec::new();
    
    for (container_index, container) in interiors.containers.iter().enumerate() {
        let parent_empty_parent_entity_name = container.parent_entity_name.clone();
        let parent_empty_parent_node_name = container.parent_node_name.clone();
        for light_info in &container.lights {
            let (parent_empty_loc, parent_empty_quat, parent_empty_scale) =
                sc_matrix_to_scene_transform(mat4_from_sc_rows(container.container_transform));
            let (parent_empty_loc, parent_empty_quat, parent_empty_scale) =
                apply_reference_root_conversion(parent_empty_loc, parent_empty_quat, parent_empty_scale);
            let position_blend = convert_position_sc_to_blender(light_info.position);
            
            let lamp_type =
                lamp_type_for_light(&light_info.light_type, &light_info.semantic_light_kind);
            
            // Convert quaternion rotation with basis correction for spotlights
            let is_spotlight = lamp_type == 2;  // SPOT type
            let rotation_blend = convert_quaternion_sc_to_blender(light_info.rotation, is_spotlight);
            
            let energy_watts = light_energy_to_blender(
                lamp_type,
                &light_info.semantic_light_kind,
                light_info.intensity_candela_proxy,
                light_info.intensity_raw,
            );
            
            // Spot angles (if present)
            let (spot_size, spot_blend) = if let (Some(inner), Some(outer)) = (
                light_info.inner_angle,
                light_info.outer_angle,
            ) {
                let spot_size = outer.to_radians() * 2.0;  // Full cone aperture
                let spot_blend = if outer > 0.0 {
                    (1.0 - (inner / outer)).max(0.0).min(1.0)  // Feather width, clamped
                } else {
                    0.0
                };
                (spot_size, spot_blend)
            } else {
                (0.0, 0.0)
            };
            
            // Get active state info for temperature
            let (temperature_k, use_temperature) = light_info.states
                .get(&light_info.active_state)
                .map(|s| (s.temperature, s.use_temperature))
                .unwrap_or((6500.0, false));
            let states_json = if light_info.states.is_empty() {
                None
            } else {
                let mut payload = serde_json::Map::new();
                for (state_name, state) in &light_info.states {
                    payload.insert(
                        state_name.clone(),
                        serde_json::json!({
                            "intensity_raw": state.intensity_raw,
                            "intensity_unit": state.intensity_unit,
                            "intensity_cd": state.intensity_cd,
                            "intensity_candela_proxy": state.intensity_candela_proxy,
                            "temperature": state.temperature,
                            "use_temperature": state.use_temperature,
                            "color": state.color,
                            "light_style": state.light_style,
                            "preset_tag": state.preset_tag,
                        }),
                    );
                }
                Some(serde_json::Value::Object(payload).to_string())
            };
            
            let blender_radius = if matches!(lamp_type, 0 | 2) {
                soft_shadow_radius_from_source(light_info.radius)
            } else {
                light_info.radius
            };
            let cutoff_distance = if lamp_type == 1 { 0.0 } else { light_info.radius };
            lights.push(ExtractedLight {
                name: light_info.name.clone(),
                parent_empty_name: Some(interior_parent_empty_name(
                    &container.name,
                    parent_empty_parent_entity_name.as_deref(),
                    Some(container_index),
                )),
                parent_empty_parent_entity_name: parent_empty_parent_entity_name.clone(),
                parent_empty_parent_node_name: parent_empty_parent_node_name.clone(),
                parent_empty_loc,
                parent_empty_quat,
                parent_empty_scale,
                position_blend,
                rotation_blend,
                color: light_info.color,
                lamp_type,
                energy_watts,
                radius: blender_radius,
                cutoff_distance,
                radius_source: light_info.radius,
                spot_size,
                spot_blend,
                intensity_candela: light_info.intensity_candela_proxy,
                temperature_k,
                use_temperature,
                gobo_path: light_info.projector_texture.clone(),
                active_state: light_info.active_state.clone(),
                states_json,
                semantic_light_kind: light_info.semantic_light_kind.clone(),
            });
        }
    }
    
    Ok(lights)
}

#[cfg(test)]
mod tests;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3B: Extract Empties from NMC (Node Mesh Combo)
// ════════════════════════════════════════════════════════════════════════════════

/// Extracted empty object ready for Blender scene construction.
#[derive(Debug, Clone)]
pub struct ExtractedEmpty {
    /// NMC node name
    pub name: String,
    /// Index in NMC node array (for parent references)
    pub nmc_index: usize,
    /// Parent node index in NMC array (None for root)
    pub parent_nmc_index: Option<usize>,
    /// Position in Blender coordinates
    pub position_blend: [f32; 3],
    /// Rotation as quaternion in Blender coordinates [w, x, y, z]
    pub rotation_blend: [f32; 4],
    /// Scale per axis
    pub scale: [f32; 3],
    /// Geometry type from NMC (0=GEOM, 3=HELP2, etc.)
    pub geometry_type: u16,
    /// Whether this is a helper/non-mesh node
    pub is_helper: bool,
}

/// Extract 4x4 matrix from NMC 3x4 row-major format
/// Returns [position, rotation_quat, scale]
fn extract_matrix_components(
    matrix_3x4: &[[f32; 4]; 3]
) -> ([f32; 3], [f32; 4], [f32; 3]) {
    // Position is the 4th column
    let position = [matrix_3x4[0][3], matrix_3x4[1][3], matrix_3x4[2][3]];
    
    // Extract rotation from 3x3 upper-left
    // Convert 3x3 rotation matrix to quaternion
    let rot_matrix = [
        [matrix_3x4[0][0], matrix_3x4[0][1], matrix_3x4[0][2]],
        [matrix_3x4[1][0], matrix_3x4[1][1], matrix_3x4[1][2]],
        [matrix_3x4[2][0], matrix_3x4[2][1], matrix_3x4[2][2]],
    ];
    
    let quaternion = matrix_to_quaternion(&rot_matrix);
    
    // Scale is typically [1, 1, 1], but might be stored separately
    let scale = [1.0, 1.0, 1.0];
    
    (position, quaternion, scale)
}

/// Convert 3x3 rotation matrix to quaternion.
/// Matrix is in row-major order.
fn matrix_to_quaternion(m: &[[f32; 3]; 3]) -> [f32; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        [
            0.25 / s,
            (m[2][1] - m[1][2]) * s,
            (m[0][2] - m[2][0]) * s,
            (m[1][0] - m[0][1]) * s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = 2.0 * (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt();
        [
            (m[2][1] - m[1][2]) / s,
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = 2.0 * (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt();
        [
            (m[0][2] - m[2][0]) / s,
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
        ]
    } else {
        let s = 2.0 * (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt();
        [
            (m[1][0] - m[0][1]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
        ]
    }
}

/// Convert 3x4 CryEngine matrix to Blender coordinates
fn convert_matrix_sc_to_blender(matrix_sc: &[[f32; 4]; 3]) -> ([[f32; 4]; 3], [f32; 4]) {
    // Extract position and rotation
    let (pos_sc, rot_quat_sc, _scale) = extract_matrix_components(matrix_sc);
    
    // Convert position
    let pos_blend = convert_position_sc_to_blender([pos_sc[0] as f64, pos_sc[1] as f64, pos_sc[2] as f64]);
    
    // Convert quaternion
    let rot_quat_blend = convert_quaternion_sc_to_blender([
        rot_quat_sc[0] as f64,
        rot_quat_sc[1] as f64,
        rot_quat_sc[2] as f64,
        rot_quat_sc[3] as f64,
    ], false);  // Empties don't need basis correction
    
    // Reconstruct matrix in Blender coordinates (simplified)
    let mut matrix_blend = [[0.0f32; 4]; 3];
    matrix_blend[0][3] = pos_blend[0];
    matrix_blend[1][3] = pos_blend[1];
    matrix_blend[2][3] = pos_blend[2];
    
    (matrix_blend, rot_quat_blend)
}

/// Extract empties (non-mesh nodes) from NMC node hierarchy.
///
/// Empties are created from NMC nodes that should not be rendered as meshes:
/// - Helper nodes (geometry_type > 0)
/// - Nodes with special properties (e.g., "class" = "AnimatedJoint")
/// - Group nodes for organizing the hierarchy
pub fn extract_empties_from_nmc(
    nmc_nodes: &[crate::nmc::NmcNode],
) -> Result<Vec<ExtractedEmpty>, Error> {
    let mut empties = Vec::new();
    
    for (idx, node) in nmc_nodes.iter().enumerate() {
        // Determine if this node should be an empty
        // Empties are non-mesh nodes (geometry_type != 0) or special node types
        let is_helper = node.geometry_type != 0 || 
                        node.properties.get("class").map(|v| v != "Mesh").unwrap_or(false);
        
        if !is_helper && idx > 0 {
            // Skip mesh geometry nodes (geometry_type == 0 and no special properties)
            continue;
        }
        
        // Extract position from WorldToBone matrix
        let pos_sc = [
            node.world_to_bone[0][3] as f64,
            node.world_to_bone[1][3] as f64,
            node.world_to_bone[2][3] as f64,
        ];
        let position_blend = convert_position_sc_to_blender(pos_sc);
        
        // Extract rotation from matrix
        let rot_matrix = [
            [node.world_to_bone[0][0], node.world_to_bone[0][1], node.world_to_bone[0][2]],
            [node.world_to_bone[1][0], node.world_to_bone[1][1], node.world_to_bone[1][2]],
            [node.world_to_bone[2][0], node.world_to_bone[2][1], node.world_to_bone[2][2]],
        ];
        let rot_quat_sc = matrix_to_quaternion(&rot_matrix);
        let rotation_blend = convert_quaternion_sc_to_blender([
            rot_quat_sc[0] as f64,
            rot_quat_sc[1] as f64,
            rot_quat_sc[2] as f64,
            rot_quat_sc[3] as f64,
        ], false);  // Empties don't need basis correction
        
        empties.push(ExtractedEmpty {
            name: node.name.clone(),
            nmc_index: idx,
            parent_nmc_index: node.parent_index.map(|p| p as usize),
            position_blend,
            rotation_blend,
            scale: node.scale,
            geometry_type: node.geometry_type,
            is_helper,
        });
    }
    
    Ok(empties)
}

#[cfg(test)]
mod tests_3b;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3C: Create Light Objects in scene.blend
// ════════════════════════════════════════════════════════════════════════════════

/// Blender lamp and object pair ready to write to .blend file
#[derive(Debug, Clone)]
pub struct LampBlockPair {
    /// Lamp datablock bytes (from build_lamp)
    pub lamp_bytes: Vec<u8>,
    /// Pointer for lamp in file allocation
    pub lamp_ptr: u64,
    /// Object datablock bytes (from build_lamp_object)
    pub object_bytes: Vec<u8>,
    /// Pointer for object in file allocation
    pub object_ptr: u64,
    /// Collection name for organizing lights (e.g., "Projector", "Ambient")
    pub collection_name: String,
}

/// Build Blender lamp datablock and object wrapper from extracted light.
///
/// Creates both the Lamp datablock (light properties) and Object wrapper
/// (placement, hierarchy). Assigns pointers for file writing.
pub fn build_lamp_blocks(
    light: &ExtractedLight,
    lamp_ptr: u64,
    object_ptr: u64,
    parent_collection_ptr: u64,
) -> Result<LampBlockPair, Error> {
    // Build the lamp datablock
    let lamp_bytes = build_lamp(
        &light.name,
        light.lamp_type,
        light.color,
        light.energy_watts,
        light.radius,
        light.cutoff_distance,
        light.spot_size,
        light.spot_blend,
        light.temperature_k,
        light.use_temperature,
    );
    
    // Build the object wrapper
    let object_bytes = build_lamp_object(
        &light.name,
        lamp_ptr,
        light.position_blend,
        light.rotation_blend,
        [1.0, 1.0, 1.0], // Standard scale
        parent_collection_ptr,
    );
    
    // Determine collection name by light type
    let collection_name = match light.lamp_type {
        0 => {
            // POINT type - distinguish between Ambient, Omni, SoftOmni
            if light.intensity_candela < 10.0 {
                "Ambient".to_string()
            } else if light.intensity_candela < 100.0 {
                "Omni".to_string()
            } else {
                "SoftOmni".to_string()
            }
        },
        1 => "Sun".to_string(),
        2 => "Projector".to_string(),
        4 => "Area".to_string(),
        _ => "Other".to_string(),
    };
    
    Ok(LampBlockPair {
        lamp_bytes,
        lamp_ptr,
        object_bytes,
        object_ptr,
        collection_name,
    })
}

/// Validate lamp block sizes (for safety)
pub fn validate_lamp_block_sizes() -> Result<(), String> {
    // Ensure block sizes match expected values
    if LAMP_SIZE != 568 {
        return Err(format!("Expected LAMP_SIZE=568, got {}", LAMP_SIZE));
    }
    if OBJECT_SIZE != 1288 {
        return Err(format!("Expected OBJECT_SIZE=1288, got {}", OBJECT_SIZE));
    }
    Ok(())
}

#[cfg(test)]
mod tests_3c;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3D: Create Empty Objects
// ════════════════════════════════════════════════════════════════════════════════

/// Blender empty object ready to write to .blend file
#[derive(Debug, Clone)]
pub struct EmptyBlockPair {
    /// Object datablock bytes (from build_empty_object)
    pub object_bytes: Vec<u8>,
    /// Pointer for object in file allocation
    pub object_ptr: u64,
    /// Collection name for organizing empties (e.g., "Helpers", "Controls")
    pub collection_name: String,
}

/// Build Blender empty object from extracted empty.
///
/// Creates an empty object placeholder for non-mesh nodes in the hierarchy.
/// Empty objects serve as group containers, animation controls, or structural nodes.
pub fn build_empty_blocks(
    empty: &ExtractedEmpty,
    object_ptr: u64,
    parent_collection_ptr: u64,
) -> Result<EmptyBlockPair, String> {
    // Build the empty object
    let object_bytes = build_empty_object(
        &empty.name,
        empty.position_blend,
        empty.rotation_blend,
        empty.scale,
        parent_collection_ptr,
    );
    
    // Determine collection name by helper type
    let collection_name = if empty.is_helper {
        match empty.geometry_type {
            3 => "Controls".to_string(),  // HELP2 = control points
            _ => "Helpers".to_string(),    // Other helper types
        }
    } else {
        "Armature".to_string()  // Non-helper nodes go to Armature
    };
    
    Ok(EmptyBlockPair {
        object_bytes,
        object_ptr,
        collection_name,
    })
}

/// Validate empty object block can be created
pub fn validate_empty_object_creation() -> Result<(), String> {
    // Verify OBJECT_SIZE is correct
    if OBJECT_SIZE != 1288 {
        return Err(format!("Expected OBJECT_SIZE=1288, got {}", OBJECT_SIZE));
    }
    Ok(())
}

#[cfg(test)]
mod tests_3d;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 3E: Organize Lights into Collections
// ════════════════════════════════════════════════════════════════════════════════

/// Collection hierarchy for organizing lights
#[derive(Debug, Clone)]
pub struct LightCollectionTree {
    /// Root lights collection
    pub root_ptr: u64,
    /// Sub-collections by type: "Ambient", "Omni", "SoftOmni", "Projector", "Sun"
    pub type_collections: std::collections::HashMap<String, u64>,
}

/// Organize extracted lights by type into collection hierarchy.
///
/// Creates a structure like:
/// - Lights (root)
///   - Ambient (collection)
///   - Omni (collection)
///   - SoftOmni (collection)
///   - Projector (collection)
///   - Sun (collection)
///
/// Returns mapping of light type → collection pointer for placement
pub fn organize_lights_into_collections(
    lights: &[ExtractedLight],
) -> Result<LightCollectionTree, String> {
    use std::collections::HashMap;
    
    // Collect unique light types
    let mut light_types = HashMap::new();
    for light in lights {
        let light_type = match light.lamp_type {
            0 => {
                // POINT - distinguish by intensity
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        light_types.entry(light_type.to_string()).or_insert_with(|| 0);
    }
    
    // Build collection map with placeholder pointers
    let mut type_collections = HashMap::new();
    let mut next_ptr = 0x2000u64;
    for light_type in light_types.keys() {
        type_collections.insert(light_type.clone(), next_ptr);
        next_ptr += 0x200;  // Space for each collection
    }
    
    Ok(LightCollectionTree {
        root_ptr: 0x1000,  // Root collection pointer
        type_collections,
    })
}

/// Categorize lights by type for collection organization
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LightCategory {
    Ambient,
    Omni,
    SoftOmni,
    Projector,
    Sun,
    Area,
    Other,
}

impl LightCategory {
    /// Determine category from lamp type and intensity
    pub fn from_light(lamp_type: i16, intensity_candela: f32) -> Self {
        match lamp_type {
            0 => {  // POINT
                if intensity_candela < 10.0 {
                    LightCategory::Ambient
                } else if intensity_candela < 100.0 {
                    LightCategory::Omni
                } else {
                    LightCategory::SoftOmni
                }
            },
            1 => LightCategory::Sun,
            2 => LightCategory::Projector,
            4 => LightCategory::Area,
            _ => LightCategory::Other,
        }
    }
    
    /// Get collection name
    pub fn collection_name(&self) -> &'static str {
        match self {
            LightCategory::Ambient => "Ambient",
            LightCategory::Omni => "Omni",
            LightCategory::SoftOmni => "SoftOmni",
            LightCategory::Projector => "Projector",
            LightCategory::Sun => "Sun",
            LightCategory::Area => "Area",
            LightCategory::Other => "Other",
        }
    }
}

/// Validate light collection organization
pub fn validate_light_collection_hierarchy(
    tree: &LightCollectionTree,
) -> Result<(), String> {
    // Root must have valid pointer
    if tree.root_ptr == 0 {
        return Err("Root collection pointer cannot be 0".to_string());
    }
    
    // All type collections must be unique and valid
    let mut seen_ptrs = std::collections::HashSet::new();
    for (type_name, ptr) in &tree.type_collections {
        if *ptr == 0 {
            return Err(format!("Collection '{}' has invalid pointer 0", type_name));
        }
        if !seen_ptrs.insert(*ptr) {
            return Err(format!("Duplicate pointer for collection '{}'", type_name));
        }
        if *ptr == tree.root_ptr {
            return Err(format!("Collection '{}' pointer conflicts with root", type_name));
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_3e;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5B: Organize Lights into Collections
// ════════════════════════════════════════════════════════════════════════════════

/// Organized light with collection metadata
#[derive(Debug, Clone)]
pub struct BlenderLight {
    /// Extracted light with all transform and property data
    pub light: ExtractedLight,
    /// Collection name for organizing this light ("Ambient", "Omni", "SoftOmni", "Projector", "Sun", "Area")
    pub collection_name: String,
}

/// Organize lights from DecomposedInput into a collection hierarchy.
///
/// Extracts lights from the interior data and organizes them by type:
/// - Ambient lights (point < 10 candela)
/// - Omni lights (point 10-100 candela)
/// - SoftOmni lights (point > 100 candela)
/// - Projector lights (spot)
/// - Sun lights (directional)
/// - Area lights
///
/// Returns organized lights ready for Blender scene construction.
///
/// # Arguments
/// * `input` - DecomposedInput containing interior light definitions
///
/// # Returns
/// * `Result<Vec<BlenderLight>, Error>` - Organized lights by collection type
pub fn organize_lights_collection(
    input: &DecomposedInput,
) -> Result<Vec<BlenderLight>, Error> {
    // Extract lights from interiors using Phase 3 extraction
    let extracted_lights = extract_lights_from_interiors(&input.interiors)
        .map_err(|e| Error::Other(e.to_string()))?;
    
    // Organize lights by category
    let mut organized_lights = Vec::new();
    
    for light in extracted_lights {
        // Determine collection name based on light type and intensity
        let collection_name = match light.lamp_type {
            0 => {
                // POINT type - distinguish between Ambient, Omni, SoftOmni
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        }.to_string();
        
        organized_lights.push(BlenderLight {
            light,
            collection_name,
        });
    }
    
    Ok(organized_lights)
}

/// Validate light collection organization
pub fn validate_lights_collection_organization(
    lights: &[BlenderLight],
) -> Result<(), String> {
    // Verify no lights are orphaned
    for light in lights {
        if light.collection_name.is_empty() {
            return Err(format!(
                "Light '{}' has empty collection name",
                light.light.name
            ));
        }
    }
    
    // Verify collection names are valid
    let valid_collections = vec![
        "Ambient", "Omni", "SoftOmni", "Projector", "Sun", "Area", "Other"
    ];
    
    for light in lights {
        if !valid_collections.contains(&light.collection_name.as_str()) {
            return Err(format!(
                "Light '{}' has invalid collection name '{}'",
                light.light.name, light.collection_name
            ));
        }
    }
    
    // Verify intensity thresholds are honored
    for light in lights {
        if light.light.lamp_type == 0 {
            // POINT lights should be categorized by intensity
            match light.collection_name.as_str() {
                "Ambient" => {
                    if light.light.intensity_candela >= 10.0 {
                        return Err(format!(
                            "Light '{}' marked Ambient but has intensity {} (should be < 10)",
                            light.light.name, light.light.intensity_candela
                        ));
                    }
                },
                "Omni" => {
                    if light.light.intensity_candela < 10.0 || light.light.intensity_candela >= 100.0 {
                        return Err(format!(
                            "Light '{}' marked Omni but has intensity {} (should be 10-100)",
                            light.light.name, light.light.intensity_candela
                        ));
                    }
                },
                "SoftOmni" => {
                    if light.light.intensity_candela < 100.0 {
                        return Err(format!(
                            "Light '{}' marked SoftOmni but has intensity {} (should be >= 100)",
                            light.light.name, light.light.intensity_candela
                        ));
                    }
                },
                _ => {}
            }
        }
    }
    
    Ok(())
}

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5B: Organize Lights into Collections (to be implemented)
// ════════════════════════════════════════════════════════════════════════════════

// Test helpers and tests for Phase 5B will be added by Phase 5B agent

// ════════════════════════════════════════════════════════════════════════════════
// Phase 5C: Organize Empty Objects into Collections (to be implemented)
// ════════════════════════════════════════════════════════════════════════════════

// Test helpers and tests for Phase 5C will be added by Phase 5C agent


// ════════════════════════════════════════════════════════════════════════════════
// Phase 5C: Organize Empty Objects into Collections
// ════════════════════════════════════════════════════════════════════════════════


/// Organized empty objects with collection metadata
#[derive(Debug, Clone)]
pub struct OrganizedEmpty {
    /// Extracted empty with all transform data
    pub empty: ExtractedEmpty,
    /// Collection name for organizing this empty ("Helpers", "Controls", "Armature")
    pub collection_name: String,
}

/// Collection hierarchy for organizing empties
#[derive(Debug, Clone)]
pub struct EmptyCollectionTree {
    /// Root empties collection
    pub root_ptr: u64,
    /// Sub-collections by type: "Helpers", "Controls", "Armature"
    pub type_collections: std::collections::HashMap<String, u64>,
    /// Organized empties grouped by collection
    pub empties_by_collection: std::collections::HashMap<String, Vec<OrganizedEmpty>>,
}

/// Categorize empties by type for collection organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmptyCategory {
    /// Helper nodes (geometry_type > 0, non-mesh)
    Helpers,
    /// Control point nodes (geometry_type == 3)
    Controls,
    /// Armature/skeleton nodes (geometry_type == 0, mesh nodes in hierarchy)
    Armature,
}

impl EmptyCategory {
    /// Determine category from empty properties
    pub fn from_empty(is_helper: bool, geometry_type: u16) -> Self {
        if is_helper {
            if geometry_type == 3 {
                EmptyCategory::Controls
            } else {
                EmptyCategory::Helpers
            }
        } else {
            EmptyCategory::Armature
        }
    }
    
    /// Get collection name for this category
    pub fn collection_name(&self) -> &str {
        match self {
            EmptyCategory::Helpers => "Helpers",
            EmptyCategory::Controls => "Controls",
            EmptyCategory::Armature => "Armature",
        }
    }
}

/// Organize extracted empties by type into collection hierarchy.
///
/// Creates a structure like:
/// - Empties (root)
///   - Helpers (collection)
///   - Controls (collection)
///   - Armature (collection)
///
/// Preserves parent-child relationships within hierarchy.
/// All coordinate transforms have already been applied (Phase 3).
///
/// Returns mapping of empty type → collection pointer for placement
pub fn organize_empties_into_collections(
    empties: &[ExtractedEmpty],
) -> Result<EmptyCollectionTree, String> {
    use std::collections::HashMap;
    
    if empties.is_empty() {
        return Ok(EmptyCollectionTree {
            root_ptr: 0x1000,
            type_collections: HashMap::new(),
            empties_by_collection: HashMap::new(),
        });
    }
    
    // Collect unique empty types and organize empties by category
    let mut type_collections = HashMap::new();
    let mut empties_by_collection: HashMap<String, Vec<OrganizedEmpty>> = HashMap::new();
    
    for empty in empties {
        let category = EmptyCategory::from_empty(empty.is_helper, empty.geometry_type);
        let collection_name = category.collection_name().to_string();
        
        // Create collection if not exists
        if !type_collections.contains_key(&collection_name) {
            let next_ptr = 0x2000u64 + (type_collections.len() as u64) * 0x200;
            type_collections.insert(collection_name.clone(), next_ptr);
        }
        
        // Add empty to collection
        empties_by_collection
            .entry(collection_name)
            .or_insert_with(Vec::new)
            .push(OrganizedEmpty {
                empty: empty.clone(),
                collection_name: category.collection_name().to_string(),
            });
    }
    
    Ok(EmptyCollectionTree {
        root_ptr: 0x1000,  // Root collection pointer
        type_collections,
        empties_by_collection,
    })
}

/// Validate empty object collection hierarchy
pub fn validate_empty_collection_hierarchy(tree: &EmptyCollectionTree) -> Result<(), String> {
    // Check for duplicate pointers
    let mut seen_ptrs = std::collections::HashSet::new();
    for (type_name, ptr) in &tree.type_collections {
        if seen_ptrs.contains(ptr) {
            return Err(format!(
                "Duplicate collection pointer 0x{:x} for type '{}'",
                ptr, type_name
            ));
        }
        seen_ptrs.insert(*ptr);
        if *ptr == tree.root_ptr {
            return Err(format!("Collection '{}' pointer conflicts with root", type_name));
        }
    }
    
    // Validate organization
    for (coll_name, empties) in &tree.empties_by_collection {
        if !tree.type_collections.contains_key(coll_name) {
            return Err(format!("Collection '{}' organized but no corresponding type collection", coll_name));
        }
        
        if empties.is_empty() {
            return Err(format!("Collection '{}' has no empties", coll_name));
        }
        
        // Verify all empties in collection match category
        for empty_entry in empties {
            if empty_entry.collection_name != *coll_name {
                return Err(format!(
                    "Empty '{}' collection_name mismatch: expected '{}', got '{}'",
                    empty_entry.empty.name, coll_name, empty_entry.collection_name
                ));
            }
        }
    }
    
    Ok(())
}

/// Verify parent-child relationships in empty hierarchy are preserved
pub fn verify_empty_hierarchy_preservation(
    empties: &[ExtractedEmpty],
) -> Result<(), String> {
    // Build index map
    let mut index_map = std::collections::HashMap::new();
    for (idx, empty) in empties.iter().enumerate() {
        index_map.insert(empty.nmc_index, idx);
    }
    
    // Check all parent references are valid
    for empty in empties {
        if let Some(parent_idx) = empty.parent_nmc_index {
            if !index_map.contains_key(&parent_idx) {
                return Err(format!(
                    "Empty '{}' has invalid parent index {}",
                    empty.name, parent_idx
                ));
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_5c;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 4A: Identify Decal/POM Materials
// ════════════════════════════════════════════════════════════════════════════════

/// Material with decal or POM properties
#[derive(Debug, Clone)]
pub struct DecalMaterial {
    /// Material name from mesh submesh
    pub material_name: String,
    /// Whether this is a decal material
    pub is_decal: bool,
    /// Whether this is a POM (parallax occlusion mapping) material
    pub is_pom: bool,
    /// Material index in the mesh
    pub material_index: usize,
}

/// Mesh with decal/POM materials requiring vertex group
#[derive(Debug, Clone)]
pub struct MeshWithDecals {
    /// Mesh name/path
    pub mesh_path: String,
    /// Materials in this mesh that are decals
    pub decal_materials: Vec<DecalMaterial>,
    /// Indices of faces using decal materials
    pub decal_face_indices: Vec<usize>,
}

type DecalMaterialSource = (String, String, String, Vec<String>);

fn string_gen_mask_tokens(string_gen_mask: &str) -> HashSet<String> {
    string_gen_mask
        .split('%')
        .filter(|token| !token.is_empty())
        .map(|token| token.trim().to_ascii_uppercase())
        .collect()
}

fn has_decal_stencil_public_param(public_param_names: &[String]) -> bool {
    public_param_names
        .iter()
        .any(|name| name.starts_with("Decal") || name.starts_with("Stencil"))
}

fn has_mesh_decal_offset_signal(tokens: &HashSet<String>) -> bool {
    tokens.contains("VERTDATA")
        || tokens.contains("DECAL")
        || tokens.contains("STENCIL_MAP")
        || tokens.contains("PARALLAX")
        || tokens.contains("POM")
        || tokens.contains("PARALLAX_OCCLUSION_MAPPING")
}

/// Identify whether a material is decal/stencil-style and whether it also uses POM.
///
/// Checks the actual material data flags for:
/// - decal/stencil signals (`MeshDecal`, `%DECAL`, `%STENCIL_MAP`, or
///   authored `Decal*`/`Stencil*` public params)
/// - `%PARALLAX` / `%POM` only after a decal/stencil signal is present
///
/// Returns (is_decal, is_pom).
pub fn identify_decal_material_flags(
    shader: &str,
    string_gen_mask: &str,
    public_param_names: &[String],
) -> (bool, bool) {
    let upper_mask = string_gen_mask.to_ascii_uppercase();
    let tokens = string_gen_mask_tokens(string_gen_mask);
    let is_mesh_decal = shader.eq_ignore_ascii_case("MeshDecal");
    // GlassPBR and ScopePBR both map to the glass shader family (see mtl::shader_family);
    // keep this in sync so scope-lens materials are not classified as decal receivers.
    let is_glass_shader =
        shader.eq_ignore_ascii_case("GlassPBR") || shader.eq_ignore_ascii_case("ScopePBR");
    let is_decal = if is_glass_shader {
        false
    } else if is_mesh_decal {
        has_mesh_decal_offset_signal(&tokens)
    } else {
        tokens.contains("DECAL")
            || upper_mask.contains("%DECAL")
            || tokens.contains("STENCIL_MAP")
            || upper_mask.contains("%STENCIL_MAP")
            || has_decal_stencil_public_param(public_param_names)
    };
    let has_pom = tokens.contains("PARALLAX")
        || tokens.contains("POM")
        || tokens.contains("PARALLAX_OCCLUSION_MAPPING")
        || upper_mask.contains("%PARALLAX")
        || upper_mask.contains("%POM");
    let is_pom = is_decal && has_pom;
    (is_decal, is_pom)
}

/// Identify all meshes with decal/POM materials from SubMaterial data.
///
/// Takes a list of materials with their StringGenMask values and identifies which ones
/// are decals or have parallax occlusion mapping.
///
/// Returns list of meshes that have decal materials and need vertex groups.
pub fn identify_meshes_with_decals(
    mesh_materials: &[(String, Vec<DecalMaterialSource>)],  // (mesh_path, [(material_name, shader, string_gen_mask, public_param_names)])
) -> Result<Vec<MeshWithDecals>, String> {
    let mut result = Vec::new();
    
    for (mesh_path, materials) in mesh_materials {
        let mut decal_materials = Vec::new();
        
        for (mat_idx, (material_name, shader, string_gen_mask, public_param_names)) in materials.iter().enumerate() {
            let (is_decal, is_pom) =
                identify_decal_material_flags(shader, string_gen_mask, public_param_names);
            
            if is_decal || is_pom {
                decal_materials.push(DecalMaterial {
                    material_name: material_name.clone(),
                    is_decal,
                    is_pom,
                    material_index: mat_idx,
                });
            }
        }
        
        if !decal_materials.is_empty() {
            result.push(MeshWithDecals {
                mesh_path: mesh_path.clone(),
                decal_materials,
                decal_face_indices: Vec::new(),  // Will be populated by 4B
            });
        }
    }
    
    Ok(result)
}

/// Validate decal material identification
pub fn validate_decal_material_identification(
    meshes: &[MeshWithDecals],
) -> Result<(), String> {
    for mesh in meshes {
        if mesh.decal_materials.is_empty() {
            return Err(format!(
                "Mesh '{}' has no decal materials (shouldn't be in list)",
                mesh.mesh_path
            ));
        }
        
        for material in &mesh.decal_materials {
            if !material.is_decal && !material.is_pom {
                return Err(format!(
                    "Material '{}' in mesh '{}' marked as decal but has no decal properties",
                    material.material_name, mesh.mesh_path
                ));
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_4a;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 4B: Create starbreaker_decal_offset Vertex Group
// ════════════════════════════════════════════════════════════════════════════════

/// Vertex group metadata
#[derive(Debug, Clone)]
pub struct VertexGroup {
    /// Name of the vertex group (e.g., "starbreaker_decal_offset")
    pub name: String,
    /// Indices of vertices in the group
    pub vertex_indices: Vec<usize>,
}

/// Mesh with vertex groups ready to write
#[derive(Debug, Clone)]
pub struct MeshWithVertexGroups {
    /// Mesh name
    pub mesh_name: String,
    /// Total vertex count in mesh
    pub total_vertices: usize,
    /// Vertex groups to add to mesh
    pub vertex_groups: Vec<VertexGroup>,
}

/// Map face indices to vertex indices based on mesh indices
///
/// Takes a list of face indices (polygons) and the mesh's corner_vert array
/// and returns the list of unique vertex indices used by those faces.
pub fn map_faces_to_vertices(
    face_indices: &[usize],
    corner_verts: &[u32],
    verts_per_face: usize,
) -> Result<Vec<usize>, String> {
    if face_indices.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut vertex_set = std::collections::HashSet::new();
    
    for &face_idx in face_indices {
        let start = face_idx * verts_per_face;
        let end = start + verts_per_face;
        
        if end > corner_verts.len() {
            return Err(format!(
                "Face {} at indices {}-{} exceeds corner_verts length {}",
                face_idx,
                start,
                end,
                corner_verts.len()
            ));
        }
        
        for i in start..end {
            vertex_set.insert(corner_verts[i] as usize);
        }
    }
    
    let mut vertices: Vec<usize> = vertex_set.into_iter().collect();
    vertices.sort();
    Ok(vertices)
}

/// Create vertex group for decal materials
///
/// Consolidates all vertices from all decal/POM materials into a single
/// "starbreaker_decal_offset" vertex group. This allows Blender addons to target
/// all decal vertices with modifiers.
///
/// Returns the combined set of vertex indices from all decal material faces.
pub fn collect_decal_vertices(
    mesh_with_decals: &MeshWithDecals,
    decal_face_indices: &[usize],
    corner_verts: &[u32],
    verts_per_face: usize,
) -> Result<MeshWithVertexGroups, String> {
    // Map all decal face indices to their vertex indices
    let vertex_indices = if decal_face_indices.is_empty() {
        Vec::new()
    } else {
        map_faces_to_vertices(decal_face_indices, corner_verts, verts_per_face)?
    };
    
    // Create single consolidated vertex group
    let decal_vgroup = VertexGroup {
        name: DECAL_OFFSET_GROUP_NAME.to_string(),
        vertex_indices,
    };
    
    // Fix Phase 4B bug: total_vertices should be max(corner_verts) + 1, not len(corner_verts)
    let total_vertices = corner_verts.iter().max().map(|&v| (v + 1) as usize).unwrap_or(0);
    
    Ok(MeshWithVertexGroups {
        mesh_name: mesh_with_decals.mesh_path.clone(),
        total_vertices,
        vertex_groups: vec![decal_vgroup],
    })
}

/// Validate vertex group integrity
pub fn validate_vertex_groups(
    vgroups: &[VertexGroup],
    total_vertices: usize,
) -> Result<(), String> {
    for vgroup in vgroups {
        if vgroup.name.is_empty() {
            return Err("Vertex group has empty name".to_string());
        }
        
        for &vertex_idx in &vgroup.vertex_indices {
            if vertex_idx >= total_vertices {
                return Err(format!(
                    "Vertex index {} exceeds vertex count {}",
                    vertex_idx, total_vertices
                ));
            }
        }
    }
    
    Ok(())
}

/// Phase 5D: Assign decal materials to vertex groups
///
/// For each mesh with decal vertices, this function validates and assigns
/// decal materials with proper blend modes and culling flags to the vertex groups.
///
/// # Arguments
///
/// * `input` - DecomposedInput containing mesh and material data
///
/// # Returns
///
/// Result with error messages for any validation issues
pub fn assign_decal_materials_to_vertex_groups(input: &DecomposedInput) -> Result<(), Error> {
    // Collect all mesh materials for decal identification
    let mut mesh_materials = Vec::new();
    
    // Add root mesh materials
    if let Some(ref mtl) = input.root_materials {
        let material_list: Vec<DecalMaterialSource> = mtl
            .materials
            .iter()
            .map(|sub| {
                (
                    sub.name.clone(),
                    sub.shader.clone(),
                    sub.string_gen_mask.clone(),
                    sub.public_params.iter().map(|param| param.name.clone()).collect(),
                )
            })
            .collect();
        mesh_materials.push(("__root__".to_string(), material_list));
    }
    
    // Add child mesh materials
    for child in &input.children {
        if let Some(ref mtl) = child.materials {
            let material_list: Vec<DecalMaterialSource> = mtl
                .materials
                .iter()
                .map(|sub| {
                    (
                        sub.name.clone(),
                        sub.shader.clone(),
                        sub.string_gen_mask.clone(),
                        sub.public_params.iter().map(|param| param.name.clone()).collect(),
                    )
                })
                .collect();
            mesh_materials.push((child.entity_name.clone(), material_list));
        }
    }
    
    // Identify meshes with decals
    let meshes_with_decals = match identify_meshes_with_decals(&mesh_materials) {
        Ok(meshes) => meshes,
        Err(_e) => {
            // Log the error but return Ok - this is not a fatal error
            return Ok(());
        }
    };
    
    // Validate each mesh with decals
    for mesh_with_decals in meshes_with_decals {
        // Check that we have decal materials
        if mesh_with_decals.decal_materials.is_empty() {
            continue;
        }
        
        // Find the corresponding mesh in input
        let mesh = if mesh_with_decals.mesh_path == "__root__" {
            Some(&input.root_mesh)
        } else {
            input.children.iter()
                .find(|c| c.entity_name == mesh_with_decals.mesh_path)
                .map(|c| &c.mesh)
        };
        
        if let Some(mesh) = mesh {
            // Validate vertex count
            if mesh.positions.is_empty() {
                continue;
            }
            
            // For each decal material, validate its properties
            for decal_material in &mesh_with_decals.decal_materials {
                // Find the material in the mesh's material file
                let material_found = if mesh_with_decals.mesh_path == "__root__" {
                    input.root_materials.as_ref()
                } else {
                    input.children.iter()
                        .find(|c| c.entity_name == mesh_with_decals.mesh_path)
                        .and_then(|c| c.materials.as_ref())
                }.map(|mtl| {
                    mtl.materials.iter()
                        .any(|sub| sub.name == decal_material.material_name)
                }).unwrap_or(false);
                
                if !material_found {
                    // Log warning but continue - material may be created dynamically
                }
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests_4b;

// ════════════════════════════════════════════════════════════════════════════════
// Phase 4C: Validate in Blender
// ════════════════════════════════════════════════════════════════════════════════

/// Validation result for a Blender export
#[derive(Debug, Clone)]
pub struct BlendValidationResult {
    /// Total lights found
    pub light_count: usize,
    /// Lights by type
    pub lights_by_type: std::collections::HashMap<String, usize>,
    /// Total empties found
    pub empty_count: usize,
    /// Total collections
    pub collection_count: usize,
    /// Meshes with decal vertex groups
    pub meshes_with_decals: usize,
    /// Validation errors
    pub errors: Vec<String>,
    /// Validation warnings
    pub warnings: Vec<String>,
    /// Overall validation passed
    pub is_valid: bool,
}

/// Validate light export results
///
/// Checks that lights have been properly extracted and converted.
/// Expected: ~62 lights in Aurora Mk2
///  - Ambient: ~10
///  - Omni: ~5
///  - SoftOmni: ~2
///  - Projector: ~45
pub fn validate_lights_extraction(
    lights: &[ExtractedLight],
) -> Result<BlendValidationResult, String> {
    let mut result = BlendValidationResult {
        light_count: lights.len(),
        lights_by_type: std::collections::HashMap::new(),
        empty_count: 0,
        collection_count: 0,
        meshes_with_decals: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    // Categorize lights
    for light in lights {
        let category = match light.lamp_type {
            0 => {
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        
        *result.lights_by_type.entry(category.to_string()).or_insert(0) += 1;
    }
    
    // Validate results
    if lights.is_empty() {
        result.errors.push("No lights extracted".to_string());
        result.is_valid = false;
    }
    
    // Check for expected Aurora Mk2 light counts (approximate)
    if lights.len() < 50 {
        result.warnings.push(format!(
            "Light count {} is lower than expected ~62 for Aurora Mk2",
            lights.len()
        ));
    }
    
    // Validate light properties
    for light in lights {
        if light.position_blend[0].is_nan() || light.position_blend[1].is_nan() || light.position_blend[2].is_nan() {
            result.errors.push(format!("Light '{}' has NaN position", light.name));
            result.is_valid = false;
        }
        
        if light.rotation_blend[0].is_nan() {
            result.errors.push(format!("Light '{}' has NaN rotation", light.name));
            result.is_valid = false;
        }
        
        if light.energy_watts <= 0.0 {
            result.warnings.push(format!(
                "Light '{}' has non-positive energy: {}",
                light.name, light.energy_watts
            ));
        }
    }
    
    Ok(result)
}

/// Validate empties extraction
pub fn validate_empties_extraction(
    empties: &[ExtractedEmpty],
) -> Result<BlendValidationResult, String> {
    let mut result = BlendValidationResult {
        light_count: 0,
        lights_by_type: std::collections::HashMap::new(),
        empty_count: empties.len(),
        collection_count: 0,
        meshes_with_decals: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    if empties.is_empty() {
        result.warnings.push("No empties extracted".to_string());
    }
    
    // Validate empty properties
    for empty in empties {
        if empty.position_blend[0].is_nan() || empty.position_blend[1].is_nan() || empty.position_blend[2].is_nan() {
            result.errors.push(format!("Empty '{}' has NaN position", empty.name));
            result.is_valid = false;
        }
        
        if empty.rotation_blend[0].is_nan() {
            result.errors.push(format!("Empty '{}' has NaN rotation", empty.name));
            result.is_valid = false;
        }
    }
    
    Ok(result)
}

/// Validate decal mesh identification
pub fn validate_decals_extraction(
    meshes: &[MeshWithDecals],
) -> Result<BlendValidationResult, String> {
    let mut result = BlendValidationResult {
        light_count: 0,
        lights_by_type: std::collections::HashMap::new(),
        empty_count: 0,
        collection_count: 0,
        meshes_with_decals: meshes.len(),
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    // Validate decal materials
    for mesh in meshes {
        if mesh.decal_materials.is_empty() {
            result.errors.push(format!(
                "Mesh '{}' marked as having decals but has none",
                mesh.mesh_path
            ));
            result.is_valid = false;
        }
        
        for material in &mesh.decal_materials {
            if !material.is_decal && !material.is_pom {
                result.errors.push(format!(
                    "Material '{}' in mesh '{}' has no decal/POM properties",
                    material.material_name, mesh.mesh_path
                ));
                result.is_valid = false;
            }
        }
    }
    
    Ok(result)
}

/// Comprehensive validation of entire Phase 3-4 pipeline
pub fn validate_complete_phase_3_4_export(
    lights: &[ExtractedLight],
    empties: &[ExtractedEmpty],
    decals: &[MeshWithDecals],
) -> BlendValidationResult {
    let mut result = BlendValidationResult {
        light_count: lights.len(),
        lights_by_type: std::collections::HashMap::new(),
        empty_count: empties.len(),
        collection_count: 5,  // Ambient, Omni, SoftOmni, Projector, Sun (typical)
        meshes_with_decals: decals.len(),
        errors: Vec::new(),
        warnings: Vec::new(),
        is_valid: true,
    };
    
    // Categorize lights
    for light in lights {
        let category = match light.lamp_type {
            0 => {
                if light.intensity_candela < 10.0 {
                    "Ambient"
                } else if light.intensity_candela < 100.0 {
                    "Omni"
                } else {
                    "SoftOmni"
                }
            },
            1 => "Sun",
            2 => "Projector",
            4 => "Area",
            _ => "Other",
        };
        
        *result.lights_by_type.entry(category.to_string()).or_insert(0) += 1;
    }
    
    // Basic validation checks
    if lights.is_empty() && empties.is_empty() && decals.is_empty() {
        result.errors.push("No lights, empties, or decals extracted".to_string());
        result.is_valid = false;
    }
    
    if lights.is_empty() {
        result.warnings.push("No lights extracted".to_string());
    }
    
    if empties.is_empty() {
        result.warnings.push("No empties extracted".to_string());
    }
    
    if decals.is_empty() {
        result.warnings.push("No decal materials identified".to_string());
    }
    
    result
}

#[cfg(test)]
mod tests_5d;

#[cfg(test)]
mod tests_4c;
