//! Phase 5D tests for decal material assignment.

use super::*;
use crate::types::EntityPayload;

#[test]
fn assign_decal_materials_basic() {
    let input = DecomposedInput {
        entity_name: "test_ship".to_string(),
        geometry_path: "/test".to_string(),
        material_path: "/test".to_string(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0, 0.0, 0.0],
            model_max: [1.0, 1.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [1.0, 1.0, 0.0],
        },
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children: vec![],
        interiors: LoadedInteriors::default(),
        paint_variants: vec![],
    };

    let result = assign_decal_materials_to_vertex_groups(&input);
    assert!(result.is_ok());
}

#[test]
fn assign_decal_materials_no_materials() {
    let input = DecomposedInput {
        entity_name: "test_ship".to_string(),
        geometry_path: "/test".to_string(),
        material_path: "/test".to_string(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: Mesh {
            positions: vec![[0.0, 0.0, 0.0]],
            indices: vec![],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0, 0.0, 0.0],
            model_max: [0.0, 0.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [0.0, 0.0, 0.0],
        },
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children: vec![],
        interiors: LoadedInteriors::default(),
        paint_variants: vec![],
    };

    let result = assign_decal_materials_to_vertex_groups(&input);
    assert!(result.is_ok());
}

#[test]
fn assign_decal_materials_empty_mesh() {
    let input = DecomposedInput {
        entity_name: "empty_ship".to_string(),
        geometry_path: "/test".to_string(),
        material_path: "/test".to_string(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: Mesh {
            positions: vec![],
            indices: vec![],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0, 0.0, 0.0],
            model_max: [0.0, 0.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [0.0, 0.0, 0.0],
        },
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children: vec![],
        interiors: LoadedInteriors::default(),
        paint_variants: vec![],
    };

    let result = assign_decal_materials_to_vertex_groups(&input);
    assert!(result.is_ok());
}

#[test]
fn assign_decal_materials_multiple_children() {
    let input = DecomposedInput {
        entity_name: "test_ship".to_string(),
        geometry_path: "/test".to_string(),
        material_path: "/test".to_string(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: Mesh {
            positions: vec![[0.0, 0.0, 0.0]],
            indices: vec![],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0, 0.0, 0.0],
            model_max: [0.0, 0.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [0.0, 0.0, 0.0],
        },
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children: vec![
            EntityPayload {
                entity_name: "child1".to_string(),
                mesh: Mesh {
                    positions: vec![[1.0, 1.0, 1.0]],
                    indices: vec![],
                    uvs: None,
                    secondary_uvs: None,
                    normals: None,
                    tangents: None,
                    colors: None,
                    submeshes: vec![],
                    model_min: [0.0, 0.0, 0.0],
                    model_max: [0.0, 0.0, 0.0],
                    scaling_min: [0.0, 0.0, 0.0],
                    scaling_max: [0.0, 0.0, 0.0],
                },
                materials: None,
                nmc: None,
                palette: None,
                bones: vec![],
                skeleton_source_path: None,
                entity_category: None,
                attach_def_type: None,
                textures: None,
                parent_node_name: "".to_string(),
                parent_entity_name: "".to_string(),
                no_rotation: false,
                offset_position: [0.0, 0.0, 0.0],
                offset_rotation: [0.0, 0.0, 0.0],
                detach_direction: [0.0, 0.0, 0.0],
                port_flags: "".to_string(),
                geometry_path: "/test".to_string(),
                material_path: "/test".to_string(),
                ui_bindings: Vec::new(),
            },
        ],
        interiors: LoadedInteriors::default(),
        paint_variants: vec![],
    };

    let result = assign_decal_materials_to_vertex_groups(&input);
    assert!(result.is_ok());
}

#[test]
fn assign_decal_materials_with_child_mesh() {
    let input = DecomposedInput {
        entity_name: "test_ship".to_string(),
        geometry_path: "/test".to_string(),
        material_path: "/test".to_string(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: Mesh {
            positions: vec![[0.0, 0.0, 0.0]],
            indices: vec![],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0, 0.0, 0.0],
            model_max: [0.0, 0.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [0.0, 0.0, 0.0],
        },
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children: vec![
            EntityPayload {
                entity_name: "child_with_mesh".to_string(),
                mesh: Mesh {
                    positions: vec![
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                    ],
                    indices: vec![0, 1, 2],
                    uvs: None,
                    secondary_uvs: None,
                    normals: None,
                    tangents: None,
                    colors: None,
                    submeshes: vec![],
                    model_min: [0.0, 0.0, 0.0],
                    model_max: [1.0, 1.0, 1.0],
                    scaling_min: [0.0, 0.0, 0.0],
                    scaling_max: [1.0, 1.0, 1.0],
                },
                materials: None,
                nmc: None,
                palette: None,
                bones: vec![],
                skeleton_source_path: None,
                entity_category: None,
                attach_def_type: None,
                textures: None,
                parent_node_name: "".to_string(),
                parent_entity_name: "".to_string(),
                no_rotation: false,
                offset_position: [0.0, 0.0, 0.0],
                offset_rotation: [0.0, 0.0, 0.0],
                detach_direction: [0.0, 0.0, 0.0],
                port_flags: "".to_string(),
                geometry_path: "/test".to_string(),
                material_path: "/test".to_string(),
                ui_bindings: Vec::new(),
            },
        ],
        interiors: LoadedInteriors::default(),
        paint_variants: vec![],
    };

    let result = assign_decal_materials_to_vertex_groups(&input);
    assert!(result.is_ok());
}
