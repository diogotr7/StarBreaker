use gltf_json as json;
use json::validation::Checked;

use crate::error::Error;
use crate::nmc::NodeMeshCombo;
use crate::types::{MaterialTextures, Mesh};

use crate::glb_builder::GlbBuilder;

/// All input data for building a GLB file.
pub struct GlbInput {
    pub root_mesh: Option<Mesh>,
    pub root_materials: Option<crate::mtl::MtlFile>,
    pub root_textures: Option<MaterialTextures>,
    pub root_nmc: Option<NodeMeshCombo>,
    pub root_palette: Option<crate::mtl::TintPalette>,
    pub skeleton_bones: Vec<crate::skeleton::Bone>,
    pub children: Vec<crate::types::EntityPayload>,
    pub interiors: crate::pipeline::LoadedInteriors,
}

pub struct GlbLoaders<'a> {
    pub load_textures: &'a mut dyn FnMut(Option<&crate::mtl::MtlFile>) -> Option<MaterialTextures>,
    pub load_interior_mesh: &'a mut dyn FnMut(
        &crate::pipeline::InteriorCgfEntry,
    ) -> Option<(Mesh, Option<crate::mtl::MtlFile>, Option<NodeMeshCombo>)>,
}

pub struct GlbOptions {
    pub include_tangents: bool,
    pub experimental_textures: bool,
    pub metadata: GlbMetadata,
    pub fallback_palette: Option<crate::mtl::TintPalette>,
}

pub struct GlbMetadata {
    pub entity_name: Option<String>,
    pub geometry_path: Option<String>,
    pub material_path: Option<String>,
    pub export_options: ExportOptionsMetadata,
}

pub struct ExportOptionsMetadata {
    pub texture_mip: u32,
    pub lod_level: u32,
    pub include_interior: bool,
    pub include_tangents: bool,
    pub include_lights: bool,
    pub include_textures: bool,
    pub include_materials: bool,
}


pub(crate) fn mat3x4_to_gltf(m: &[[f32; 4]; 3]) -> [f32; 16] {
    [
        m[0][0], m[1][0], m[2][0], 0.0, // column 0
        m[0][1], m[1][1], m[2][1], 0.0, // column 1
        m[0][2], m[1][2], m[2][2], 0.0, // column 2
        m[0][3], m[1][3], m[2][3], 1.0, // column 3 (translation)
    ]
}

/// Add a BufferView + Accessor pair for vertex attribute data.
/// Returns the accessor index, or None if `byte_length` is 0.
pub(crate) fn add_vertex_accessor(
    buffer_views: &mut Vec<json::buffer::View>,
    accessors: &mut Vec<json::Accessor>,
    byte_offset: usize,
    byte_length: usize,
    count: usize,
    accessor_type: json::accessor::Type,
    min_max: Option<(&[f32], &[f32])>,
) -> Option<u32> {
    if byte_length == 0 {
        return None;
    }
    let bv_idx = buffer_views.len() as u32;
    buffer_views.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_offset: Some(json::validation::USize64(byte_offset as u64)),
        byte_length: json::validation::USize64(byte_length as u64),
        byte_stride: None,
        target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
        name: None,
        extensions: None,
        extras: Default::default(),
    });
    let acc_idx = accessors.len() as u32;
    accessors.push(json::Accessor {
        buffer_view: Some(json::Index::new(bv_idx)),
        byte_offset: Some(json::validation::USize64(0)),
        count: json::validation::USize64(count as u64),
        component_type: Checked::Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::F32,
        )),
        type_: Checked::Valid(accessor_type),
        min: min_max.map(|(min, _)| serde_json::Value::Array(min.iter().map(|&v| serde_json::Value::from(v)).collect())),
        max: min_max.map(|(_, max)| serde_json::Value::Array(max.iter().map(|&v| serde_json::Value::from(v)).collect())),
        name: None,
        normalized: false,
        sparse: None,
        extensions: None,
        extras: Default::default(),
    });
    Some(acc_idx)
}


pub(crate) fn is_identity_or_zero(m: &[[f32; 4]; 3]) -> bool {
    let all_zero = m.iter().all(|row| row.iter().all(|&v| v == 0.0));
    if all_zero {
        return true;
    }
    let identity: [[f32; 4]; 3] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    for r in 0..3 {
        for c in 0..4 {
            if (m[r][c] - identity[r][c]).abs() > 1e-6 {
                return false;
            }
        }
    }
    true
}

pub fn write_glb(
    input: GlbInput,
    loaders: &mut GlbLoaders<'_>,
    opts: &GlbOptions,
) -> Result<Vec<u8>, Error> {
    let mut builder = GlbBuilder::new();

    log::info!("[mem-phase] start write_glb");
    // ---- Pack root entity (drop textures after packing) ----
    let mut scene_nodes = if let Some(root_mesh) = input.root_mesh {
        let root_packed = builder.pack_mesh(
            &root_mesh,
            input.root_materials.as_ref(),
            input.root_textures.as_ref(),
            input.root_palette.as_ref(),
            None,
            opts.include_tangents,
            opts.experimental_textures,
        );
        drop(input.root_textures);

        // ---- Build root NMC scene graph ----
        if let Some(nmc) = input.root_nmc.as_ref().filter(|n| !n.nodes.is_empty()) {
            let root_nodes = builder.build_nmc_hierarchy(&root_packed, nmc, &root_mesh.submeshes, true);
            root_nodes
                .iter()
                .map(|&i| json::Index::new(i))
                .collect::<Vec<_>>()
        } else {
            // Flat path — single node with root mesh
            builder.nodes_json.push(json::Node {
                mesh: Some(json::Index::new(root_packed.mesh_idx)),
                ..Default::default()
            });
            vec![json::Index::new(0)]
        }
    } else {
        Vec::new()
    };

    log::info!("[mem-phase] root packed, bin={}MB", builder.bin.len() / 1_048_576);
    // ---- Skeleton bone nodes ----
    builder.attach_skeleton_bones(&input.skeleton_bones, &scene_nodes);

    // ---- Attach child entities ----
    let num_children = input.children.len();
    for (i, child) in input.children.into_iter().enumerate() {
        builder.attach_child_entity(child, &scene_nodes, opts.include_tangents, opts.experimental_textures, opts.fallback_palette.as_ref(), loaders.load_textures);
        if (i + 1) % 20 == 0 || i + 1 == num_children {
            log::info!("[mem-phase] children {}/{}, bin={}MB", i + 1, num_children, builder.bin.len() / 1_048_576);
        }
    }

    log::info!("[mem-phase] children done, bin={}MB", builder.bin.len() / 1_048_576);
    // ---- Interior mesh instancing ----
    let (interior_scene_nodes, all_lights) = builder.attach_interiors(
        &input.interiors, opts.include_tangents, opts.experimental_textures, opts.fallback_palette.as_ref(), loaders.load_textures, loaders.load_interior_mesh,
    );
    scene_nodes.extend(interior_scene_nodes);
    log::info!("[mem-phase] interiors done, bin={}MB", builder.bin.len() / 1_048_576);

    // ---- Entity + palette extras on root node ----
    {
        let mut map = serde_json::Map::new();
        if let Some(ref entity_name) = opts.metadata.entity_name {
            map.insert("entity_name".into(), serde_json::json!(entity_name));
            if let Some(ref gp) = opts.metadata.geometry_path {
                map.insert("geometry_path".into(), serde_json::json!(gp));
            }
            if let Some(ref mp) = opts.metadata.material_path {
                map.insert("material_path".into(), serde_json::json!(mp));
            }
        }
        if let Some(ref pal) = input.root_palette {
            map.insert("tint_palette".into(), serde_json::json!({
                "primary": pal.primary,
                "secondary": pal.secondary,
                "tertiary": pal.tertiary,
                "glass": pal.glass
            }));
        }
        if !map.is_empty() {
            if let Some(first) = scene_nodes.first() {
                let idx = first.value() as usize;
                if idx < builder.nodes_json.len() {
                    builder.nodes_json[idx].extras = Some(
                        serde_json::value::RawValue::from_string(
                            serde_json::to_string(&serde_json::Value::Object(map))?
                        )?.into()
                    );
                }
            }
        }
    }

    builder.finalize(scene_nodes, all_lights, &opts.metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Multiply a 3x4 row-major matrix A by a 3x4 row-major matrix B,
    /// treating B as a 4x4 matrix with an implicit `[0,0,0,1]` bottom row.
    fn mul_3x4(a: &[[f32; 4]; 3], b: &[[f32; 4]; 3]) -> [[f32; 4]; 3] {
        let mut result = [[0.0f32; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                result[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            }
            result[r][3] = a[r][0] * b[0][3] + a[r][1] * b[1][3] + a[r][2] * b[2][3] + a[r][3];
        }
        result
    }
    use crate::nmc::NmcNode;
    use crate::types::SubMesh;

    fn triangle_mesh() -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![SubMesh {
                material_name: Some("test".into()),
                material_id: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            }],
            model_min: [0.0; 3],
            model_max: [1.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [1.0; 3],
        }
    }

    fn default_opts() -> GlbOptions {
        GlbOptions {
            include_tangents: true,
            experimental_textures: false,
            metadata: GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: ExportOptionsMetadata {
                    texture_mip: 0,
                    lod_level: 0,
                    include_interior: false,
                    include_tangents: true,
                    include_lights: false,
                    include_textures: false,
                    include_materials: true,
                },
            },
            fallback_palette: None,
        }
    }

    fn call_write_glb(mesh: Mesh) -> Result<Vec<u8>, crate::error::Error> {
        write_glb(
            GlbInput {
                root_mesh: Some(mesh),
                root_materials: None,
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
    }

    fn write_glb_simple(mesh: &Mesh, nmc: Option<&NodeMeshCombo>) -> Result<Vec<u8>, crate::error::Error> {
        write_glb(
            GlbInput {
                root_mesh: Some(mesh.clone()),
                root_materials: None,
                root_textures: None,
                root_nmc: nmc.cloned(),
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
    }

    #[test]
    fn write_glb_produces_valid_header() {
        let glb = call_write_glb(triangle_mesh()).expect("write_glb failed");
        assert!(glb.len() >= 12);
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
    }

    #[test]
    fn write_glb_has_json_and_bin_chunks() {
        let glb = call_write_glb(triangle_mesh()).expect("write_glb failed");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_type = u32::from_le_bytes(glb[16..20].try_into().unwrap());
        assert_eq!(json_type, 0x4E4F534A, "first chunk should be JSON");

        let json_bytes = &glb[20..20 + json_len];
        let json_str = std::str::from_utf8(json_bytes).expect("JSON not valid UTF-8");
        assert!(
            json_str.contains("\"meshes\""),
            "JSON should contain meshes"
        );

        let bin_offset = 20 + json_len;
        if glb.len() > bin_offset + 8 {
            let bin_type =
                u32::from_le_bytes(glb[bin_offset + 4..bin_offset + 8].try_into().unwrap());
            assert_eq!(bin_type, 0x004E4942, "second chunk should be BIN");
        }
    }

    #[test]
    fn mat3x4_to_gltf_identity() {
        let m: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let g = mat3x4_to_gltf(&m);
        #[rustfmt::skip]
        let expected = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(g, expected);
    }

    #[test]
    fn mat3x4_to_gltf_translation() {
        let m: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, -3.0],
        ];
        let g = mat3x4_to_gltf(&m);
        // Column-major: last 4 elements should be the translation + 1.0
        assert_eq!(g[12], 5.0);
        assert_eq!(g[13], 10.0);
        assert_eq!(g[14], -3.0);
        assert_eq!(g[15], 1.0);
    }

    #[test]
    fn mul_3x4_identity_times_identity() {
        let id: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let result = mul_3x4(&id, &id);
        for r in 0..3 {
            for c in 0..4 {
                assert!(
                    (result[r][c] - id[r][c]).abs() < 1e-6,
                    "mismatch at [{r}][{c}]"
                );
            }
        }
    }

    #[test]
    fn mul_3x4_translation_composition() {
        // A translates by (1,0,0), B translates by (0,2,0)
        let a: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let b: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let result = mul_3x4(&a, &b);
        // A * B should translate by (1,2,0)
        assert!((result[0][3] - 1.0).abs() < 1e-6);
        assert!((result[1][3] - 2.0).abs() < 1e-6);
        assert!((result[2][3] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn is_identity_or_zero_detects_zero() {
        let zero = [[0.0f32; 4]; 3];
        assert!(is_identity_or_zero(&zero));
    }

    #[test]
    fn is_identity_or_zero_detects_identity() {
        let id: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        assert!(is_identity_or_zero(&id));
    }

    #[test]
    fn is_identity_or_zero_rejects_translation() {
        let m: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        assert!(!is_identity_or_zero(&m));
    }

    /// Build a mesh with two submeshes assigned to different NMC nodes,
    /// and verify the scene graph path produces hierarchical nodes.
    #[test]
    fn write_glb_scene_graph_produces_hierarchical_nodes() {
        // 6 vertices, 2 triangles = 2 submeshes on different nodes
        let mesh = Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            model_min: [0.0; 3],
            model_max: [3.0, 1.0, 0.0],
            scaling_min: [0.0; 3],
            scaling_max: [3.0, 1.0, 0.0],
            submeshes: vec![
                SubMesh {
                    material_name: Some("mat_a".into()),
                    material_id: 0,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 0, // belongs to root node
                },
                SubMesh {
                    material_name: Some("mat_b".into()),
                    material_id: 0,
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 1, // belongs to child node
                },
            ],
        };

        let nmc = NodeMeshCombo {
            nodes: vec![
                NmcNode {
                    name: "Root".into(),
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
                    properties: std::collections::HashMap::new(),
                },
                NmcNode {
                    name: "Child".into(),
                    parent_index: Some(0),
                    world_to_bone: [
                        [1.0, 0.0, 0.0, -5.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 5.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: std::collections::HashMap::new(),
                },
            ],
            material_indices: vec![0, 0],
        };

        let glb =
            write_glb_simple(&mesh, Some(&nmc)).expect("write_glb with NMC failed");

        // Verify GLB header
        assert_eq!(&glb[0..4], b"glTF");

        // Parse JSON to verify structure
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_bytes = &glb[20..20 + json_len];
        let json_str = std::str::from_utf8(json_bytes).expect("JSON not valid UTF-8");
        let root: serde_json::Value = serde_json::from_str(json_str.trim()).expect("invalid JSON");

        // Should have 2 meshes (one per node with geometry)
        let meshes = root["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 2, "expected 2 meshes for 2 geometry nodes");

        // Should have 3 nodes (2 content + 1 coordinate root)
        let nodes = root["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "expected 3 nodes (2 content + coord root)");

        // First node (Root) should have name "Root" and children [1]
        assert_eq!(nodes[0]["name"], "Root");
        let children = nodes[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], 1);

        // Second node (Child) should have name "Child" and a matrix with translation
        assert_eq!(nodes[1]["name"], "Child");
        // Root node: bone_to_world is identity → is_identity_or_zero → matrix=None
        assert!(
            nodes[0]["matrix"].is_null(),
            "root identity should be omitted"
        );
        // Child: bone_to_world (local transform) has tx=5
        let matrix = nodes[1]["matrix"].as_array().unwrap();
        assert_eq!(matrix.len(), 16);
        // Column-major: element [12] is translation X
        assert_eq!(matrix[12].as_f64().unwrap(), 5.0);

        // Scene should reference the coordinate root node which wraps the content
        let scene_nodes = root["scenes"][0]["nodes"].as_array().unwrap();
        assert_eq!(scene_nodes.len(), 1);
        // Coordinate root is at index 2 (after the 2 content nodes)
        assert_eq!(
            nodes[scene_nodes[0].as_u64().unwrap() as usize]["name"],
            "CryEngine_Z_up"
        );
    }

    /// Verify that a non-trivial child transform produces a matrix in the output.
    #[test]
    fn write_glb_scene_graph_nontrivial_transform() {
        let mesh = Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            model_min: [0.0; 3],
            model_max: [1.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [1.0; 3],
            submeshes: vec![SubMesh {
                material_name: None,
                material_id: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            }],
        };

        let nmc = NodeMeshCombo {
            nodes: vec![
                NmcNode {
                    name: "Root".into(),
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
                    properties: std::collections::HashMap::new(),
                },
                NmcNode {
                    name: "Offset".into(),
                    parent_index: Some(0),
                    world_to_bone: [
                        [1.0, 0.0, 0.0, -10.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 10.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: std::collections::HashMap::new(),
                },
            ],
            material_indices: vec![0],
        };

        let glb = write_glb_simple(&mesh, Some(&nmc)).expect("write_glb failed");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_bytes = &glb[20..20 + json_len];
        let root: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(json_bytes).unwrap().trim()).unwrap();

        let nodes = root["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "2 content nodes + 1 coord root");

        // Root has no geometry (submesh belongs to node 1)
        assert!(nodes[0]["mesh"].is_null(), "root should have no mesh");

        // Offset node has the mesh
        assert!(!nodes[1]["mesh"].is_null(), "child should have mesh");

        // Only 1 mesh (only one node has geometry)
        let meshes = root["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 1);

        // Child local transform = child.b2w = tx(10) → non-identity → should have matrix
        let matrix = nodes[1]["matrix"].as_array().unwrap();
        assert_eq!(matrix.len(), 16);
        // Column-major: element [12] is translation X
        assert_eq!(matrix[12].as_f64().unwrap(), 10.0);
    }
}
