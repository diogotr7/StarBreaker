//! Tests for `physics_grid`: the grid volume's shapes in the vehicle entity's frame.
use super::*;
use crate::ivo::physics::{PhysicsEntry, PhysicsGeometry};
use crate::nmc::NmcNode;

fn node(geometry_type: u16, t: [f32; 3]) -> NmcNode {
    NmcNode {
        name: "Grid".into(),
        parent_index: None,
        world_to_bone: [[1.0, 0.0, 0.0, -t[0]], [0.0, 1.0, 0.0, -t[1]], [0.0, 0.0, 1.0, -t[2]]],
        bone_to_world: [[1.0, 0.0, 0.0, t[0]], [0.0, 1.0, 0.0, t[1]], [0.0, 0.0, 1.0, t[2]]],
        scale: [1.0; 3],
        geometry_type,
        properties: Default::default(),
    }
}

fn trimesh(node: Option<u16>) -> PhysicsEntry {
    PhysicsEntry {
        node,
        part: 1,
        geometry: PhysicsGeometry::Trimesh {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
            material_ids: vec![0; 4],
        },
    }
}

#[test]
fn whole_object_volume_without_nodes_stays_in_the_entity_frame() {
    let grid = grid_json("Data/Objects/grid.cgf", &[trimesh(None)], &[]).unwrap().unwrap();
    assert_eq!(grid["source_path"], "Data/Objects/grid.cgf");
    let shapes = grid["shapes"].as_array().unwrap();
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0]["kind"], "trimesh");
    assert_eq!(shapes[0]["vertices"][1], serde_json::json!([1.0, 0.0, 0.0]));
    assert_eq!(shapes[0]["triangles"][0], serde_json::json!([0, 2, 1]));
}

#[test]
fn whole_object_volume_follows_the_single_geometry_node() {
    let grid = grid_json("g", &[trimesh(None)], &[node(0, [2.0, 3.0, 4.0])]).unwrap().unwrap();
    assert_eq!(grid["shapes"][0]["vertices"][1], serde_json::json!([3.0, 3.0, 4.0]));
}

#[test]
fn per_node_box_is_moved_into_the_entity_frame() {
    let entry = PhysicsEntry {
        node: Some(0),
        part: 1,
        geometry: PhysicsGeometry::Box {
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            center: [1.0, 0.0, 0.0],
            half_size: [1.0, 2.0, 3.0],
        },
    };
    let grid = grid_json("g", &[entry], &[node(3, [0.0, 10.0, 0.0])]).unwrap().unwrap();
    let shape = &grid["shapes"][0];
    assert_eq!(shape["kind"], "box");
    assert_eq!(shape["center"], serde_json::json!([1.0, 10.0, 0.0]));
    assert_eq!(shape["half_size"], serde_json::json!([1.0, 2.0, 3.0]));
}

#[test]
fn damage_data_is_not_a_volume_and_an_empty_volume_is_none() {
    let entry = PhysicsEntry { node: None, part: 1, geometry: PhysicsGeometry::DamageMesh { bytes: 8 } };
    assert!(grid_json("g", &[entry], &[]).unwrap().is_none());
}

#[test]
fn per_node_entry_outside_the_nmc_is_an_error() {
    assert!(grid_json("g", &[trimesh(Some(4))], &[node(0, [0.0; 3])]).is_err());
}

#[test]
fn grid_node_transforms_compose_down_the_parent_hierarchy() {
    // NMC `bone_to_world` is the node's LOCAL matrix (lib.rs `flatten_nmc_to_world`): a grid node 2 m below a
    // helper that is 10 m up sits at 8 m in the entity frame.
    let helper = node(3, [0.0, 0.0, 10.0]);
    let mut child = node(0, [0.0, 0.0, -2.0]);
    child.parent_index = Some(0);
    let grid = grid_json("g", &[trimesh(Some(1))], &[helper.clone(), child.clone()]).unwrap().unwrap();
    assert_eq!(grid["shapes"][0]["vertices"][3], serde_json::json!([0.0, 0.0, 9.0]));
    // Whole-object physics on the single geometry node composes the same way.
    let grid = grid_json("g", &[trimesh(None)], &[helper, child]).unwrap().unwrap();
    assert_eq!(grid["shapes"][0]["vertices"][3], serde_json::json!([0.0, 0.0, 9.0]));
}

#[test]
fn zero_node_matrix_is_identity() {
    // An all-zero NMC matrix means "no transform" everywhere else in the exporter (`is_identity_or_zero`).
    let mut zero = node(0, [0.0; 3]);
    zero.bone_to_world = [[0.0; 4]; 3];
    let grid = grid_json("g", &[trimesh(Some(0))], &[zero]).unwrap().unwrap();
    assert_eq!(grid["shapes"][0]["vertices"][1], serde_json::json!([1.0, 0.0, 0.0]));
}
