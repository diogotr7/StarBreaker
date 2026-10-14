//! Tests for authored normals in the `.blend` writers: the `custom_normal`
//! corner attribute (flat and NMC-hierarchy meshes), the Weld / Weighted
//! Normal modifiers they replace, and the scene wrappers that mirror them.

use super::*;
use starbreaker_blend::custom_normals::decode_corner_normals;
use starbreaker_blend::{ATTR_TYPE_INT16_2D, SDNA_IDX_WEIGHTED_NORMAL_MODIFIER, SDNA_IDX_WELD_MODIFIER};
use starbreaker_blend::reader::{cstr, i16_at, i32_at, ptr_at, BlendFile, Block};

use crate::types::SubMesh;

/// A bent quad (two triangles) with non-default, unit normals.
fn bent_quad(normals: Option<Vec<[f32; 3]>>) -> Mesh {
    let mut mesh = Mesh::empty();
    mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.3], [0.0, 1.0, 0.0]];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    mesh.normals = normals;
    mesh.submeshes = vec![SubMesh {
        material_name: Some("mat".to_string()),
        material_id: 0,
        source_material_id: None,
        first_index: 0,
        num_indices: 6,
        first_vertex: 0,
        num_vertices: 4,
        node_parent_index: 0,
    }];
    mesh
}

fn tilted_normals() -> Vec<[f32; 3]> {
    [[0.2f32, 0.1, 1.0], [-0.3, 0.0, 1.0], [0.0, 0.4, 1.0], [0.5, -0.5, 1.0]]
        .iter()
        .map(|n| {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            [n[0] / l, n[1] / l, n[2] / l]
        })
        .collect()
}

fn mesh_named<'a>(file: &'a BlendFile, name: &str) -> Block<'a> {
    file.ids(b"ME").find(|me| me.id_name() == name).unwrap_or_else(|| panic!("no mesh {name}"))
}

fn object_named<'a>(file: &'a BlendFile, name: &str) -> Block<'a> {
    file.ids(b"OB").find(|ob| ob.id_name() == name).unwrap_or_else(|| panic!("no object {name}"))
}

/// The mesh's `custom_normal` attribute: `(type, domain, short2 values)`.
fn custom_normal_attribute(file: &BlendFile, me: Block<'_>) -> Option<(i16, u8, Vec<[i16; 2]>)> {
    let attrs = file.block(ptr_at(me.data, 456))?;
    attrs.data.chunks_exact(24).take(i32_at(me.data, 464) as usize).find_map(|a| {
        let name = file.block(ptr_at(a, 0)).map(|b| cstr(b.data, 0, b.data.len()).to_string())?;
        (name == "custom_normal").then(|| {
            let array = file.block(ptr_at(a, 16)).expect("attribute array");
            let raw = file.block(ptr_at(array.data, 0)).expect("attribute data");
            let values = raw.data.chunks_exact(4).map(|c| [i16_at(c, 0), i16_at(c, 2)]).collect();
            (i16_at(a, 8), a[10], values)
        })
    })
}

/// The object's modifier SDNA indices in stack order, checking the links.
fn modifier_stack(file: &BlendFile, ob: Block<'_>) -> Vec<u32> {
    let modifiers = file.list(ob.data, 656);
    let mut prev = 0;
    for m in &modifiers {
        assert_eq!(ptr_at(m.data, 8), prev, "modifier prev link");
        prev = m.old_ptr;
    }
    assert_eq!(ptr_at(ob.data, 664), prev, "Object.modifiers.last");
    modifiers.iter().map(|m| m.sdna).collect()
}

fn assert_decodes_to(mesh: &Mesh, values: &[[i16; 2]]) {
    let decoded = decode_corner_normals(&mesh.positions, &mesh.indices, values).unwrap();
    let normals = mesh.normals.as_ref().unwrap();
    for (corner, n) in decoded.iter().enumerate() {
        let want = normals[mesh.indices[corner] as usize];
        let err = (0..3).map(|i| (n[i] - want[i]).abs()).fold(0.0, f32::max);
        assert!(err < 1e-3, "corner {corner}: {n:?} vs {want:?}");
    }
}

#[test]
fn flat_mesh_writes_normals_as_custom_normal_corner_shorts() {
    let mesh = bent_quad(Some(tilted_normals()));
    let bytes = mesh_to_blend("quad", &mesh, &None, None, None, &[], &[]).unwrap();
    let file = BlendFile::parse(&bytes).unwrap();
    let (kind, domain, values) = custom_normal_attribute(&file, mesh_named(&file, "quad")).expect("custom_normal");
    assert_eq!((kind, domain), (ATTR_TYPE_INT16_2D, ATTR_DOMAIN_CORNER));
    assert_eq!(values.len(), mesh.indices.len());
    assert_decodes_to(&mesh, &values);
}

#[test]
fn flat_mesh_with_normals_drops_weld_and_weighted_normal() {
    let bytes = mesh_to_blend("quad", &bent_quad(Some(tilted_normals())), &None, None, None, &[], &[]).unwrap();
    let file = BlendFile::parse(&bytes).unwrap();
    assert!(modifier_stack(&file, object_named(&file, "quad")).is_empty());
}

#[test]
fn flat_mesh_without_normals_keeps_weld_and_weighted_normal() {
    let bytes = mesh_to_blend("quad", &bent_quad(None), &None, None, None, &[], &[]).unwrap();
    let file = BlendFile::parse(&bytes).unwrap();
    assert!(custom_normal_attribute(&file, mesh_named(&file, "quad")).is_none());
    assert_eq!(
        modifier_stack(&file, object_named(&file, "quad")),
        vec![SDNA_IDX_WELD_MODIFIER, SDNA_IDX_WEIGHTED_NORMAL_MODIFIER]
    );
}

#[test]
fn decal_mesh_with_normals_keeps_only_the_displace_modifier() {
    let groups = vec![VertexGroup { name: DECAL_OFFSET_GROUP_NAME.to_string(), vertex_indices: vec![0, 1, 2, 3] }];
    let mesh = bent_quad(Some(tilted_normals()));
    let bytes = mesh_to_blend("decal", &mesh, &None, None, Some(&groups), &[], &[]).unwrap();
    let file = BlendFile::parse(&bytes).unwrap();
    assert_eq!(modifier_stack(&file, object_named(&file, "decal")), vec![SDNA_IDX_DISPLACE_MODIFIER]);
}

#[test]
fn mismatched_normal_count_is_an_error() {
    let mesh = bent_quad(Some(vec![[0.0, 0.0, 1.0]; 3]));
    assert!(mesh_to_blend("quad", &mesh, &None, None, None, &[], &[]).is_err());
}

#[test]
fn hierarchy_meshes_write_custom_normals_per_node() {
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    let node = |name: &str, parent_index: Option<u16>, geometry_type: u16| crate::nmc::NmcNode {
        name: name.to_string(),
        parent_index,
        world_to_bone: identity,
        bone_to_world: identity,
        scale: [1.0; 3],
        geometry_type,
        properties: HashMap::new(),
    };
    let nmc = NodeMeshCombo { nodes: vec![node("root", None, 3), node("geo", Some(0), 0)], material_indices: vec![] };
    let mut mesh = bent_quad(Some(tilted_normals()));
    mesh.submeshes[0].node_parent_index = 1;
    let bytes = mesh_to_blend("asset_LOD0", &mesh, &None, Some(&nmc), None, &[], &[]).unwrap();
    let file = BlendFile::parse(&bytes).unwrap();
    let (kind, domain, values) = custom_normal_attribute(&file, mesh_named(&file, "geo")).expect("custom_normal");
    assert_eq!((kind, domain), (ATTR_TYPE_INT16_2D, ATTR_DOMAIN_CORNER));
    assert_decodes_to(&mesh, &values);
    assert!(modifier_stack(&file, object_named(&file, "geo")).is_empty());
}

#[test]
fn link_data_reports_meshes_with_custom_normals() {
    let with = mesh_to_blend("with", &bent_quad(Some(tilted_normals())), &None, None, None, &[], &[]).unwrap();
    let without = mesh_to_blend("without", &bent_quad(None), &None, None, None, &[], &[]).unwrap();
    assert!(blend_link_data_from_bytes(&with).0[0].has_custom_normals);
    assert!(!blend_link_data_from_bytes(&without).0[0].has_custom_normals);
}

fn scene_instance() -> LinkedMeshInstance {
    LinkedMeshInstance {
        scene_instance_id: 0,
        ui_bindings: Vec::new(),
        entity_name: "Hull".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "HullMesh".to_string(),
        name: "HullMesh".to_string(),
        mesh_name: "HullMesh".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_ancestors: Vec::new(),
        source_nodes: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/hull.blend".to_string(),
        mesh_asset: "Data/Objects/hull.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    }
}

fn scene_wrapper_stack(custom_normal_refs: &HashSet<(String, String)>) -> Vec<u32> {
    let bytes = create_scene_blend_package_with_instances_and_decal_offsets(
        "Test",
        "Test",
        &[scene_instance()],
        &[],
        &HashMap::new(),
        &HashSet::new(),
        custom_normal_refs,
        None,
    )
    .unwrap();
    let file = BlendFile::parse(&bytes).unwrap();
    modifier_stack(&file, object_named(&file, "HullMesh"))
}

#[test]
fn scene_wrapper_of_a_mesh_with_custom_normals_has_no_normal_modifiers() {
    let refs = HashSet::from([("data/objects/hull.blend".to_string(), "HullMesh".to_string())]);
    assert!(scene_wrapper_stack(&refs).is_empty());
}

#[test]
fn scene_wrapper_of_a_mesh_without_custom_normals_keeps_normal_modifiers() {
    assert_eq!(
        scene_wrapper_stack(&HashSet::new()),
        vec![SDNA_IDX_WELD_MODIFIER, SDNA_IDX_WEIGHTED_NORMAL_MODIFIER]
    );
}
