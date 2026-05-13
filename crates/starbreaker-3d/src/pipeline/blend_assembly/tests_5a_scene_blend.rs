//! Phase 5A tests for native scene blend assembly and linked asset handling.

use super::*;
use crate::decomposed::DecomposedInput;
use crate::types::{Mesh, SubMesh};
use crate::pipeline::LoadedInteriors;

#[derive(Debug)]
struct BlendBlock<'a> {
    code: &'a [u8],
    sdna: u32,
    old_ptr: u64,
    data: &'a [u8],
}

fn parse_blend_blocks(bytes: &[u8]) -> Vec<BlendBlock<'_>> {
    let mut blocks = Vec::new();
    let mut offset = BLEND_MAGIC.len();
    while offset + 32 <= bytes.len() {
        let code = &bytes[offset..offset + 4];
        let sdna = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
        let old_ptr = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
        let len = u64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap()) as usize;
        let data_start = offset + 32;
        let data_end = data_start + len;
        blocks.push(BlendBlock {
            code,
            sdna,
            old_ptr,
            data: &bytes[data_start..data_end],
        });
        offset = data_end;
        if code == b"ENDB" {
            break;
        }
    }
    blocks
}

fn object_block_by_name<'a>(blocks: &'a [BlendBlock<'a>], name: &str) -> &'a BlendBlock<'a> {
    blocks
        .iter()
        .find(|block| {
            if block.code != b"OB\0\0" {
                return false;
            }
            let raw = &block.data[42..300];
            let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
            &raw[..end] == name.as_bytes()
        })
        .unwrap_or_else(|| panic!("missing Object block named {name}"))
}

fn mesh_block_by_name<'a>(blocks: &'a [BlendBlock<'a>], name: &str) -> &'a BlendBlock<'a> {
    blocks
        .iter()
        .find(|block| {
            if block.code != b"ME\0\0" {
                return false;
            }
            let raw = &block.data[42..300];
            let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
            &raw[..end] == name.as_bytes()
        })
        .unwrap_or_else(|| panic!("missing Mesh block named {name}"))
}

fn id_block_names(blocks: &[BlendBlock<'_>], code: &[u8; 4]) -> Vec<String> {
    blocks
        .iter()
        .filter(|block| block.code == code)
        .map(|block| cstr_at(block.data, 42, 258).to_string())
        .collect()
}

fn id_block_name(block: &BlendBlock<'_>) -> String {
    cstr_at(block.data, 42, 258).to_string()
}

fn cstr_at(data: &[u8], offset: usize, len: usize) -> &str {
    let raw = &data[offset..offset + len];
    let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
    std::str::from_utf8(&raw[..end]).unwrap()
}

fn test_light(name: &str) -> ExtractedLight {
    ExtractedLight {
        name: name.to_string(),
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        position_blend: [0.0, 0.0, 0.0],
        rotation_blend: [1.0, 0.0, 0.0, 0.0],
        color: [1.0, 1.0, 1.0],
        lamp_type: 0,
        energy_watts: 100.0,
        radius: 10.0,
        cutoff_distance: 10.0,
        radius_source: 10.0,
        spot_size: 0.0,
        spot_blend: 0.0,
        intensity_candela: 5.0,
        temperature_k: 3000.0,
        use_temperature: false,
        gobo_path: None,
        active_state: "default".to_string(),
        states_json: None,
        semantic_light_kind: "point".to_string(),
    }
}

fn test_mesh_with_submeshes(submeshes: Vec<SubMesh>) -> Mesh {
    Mesh {
        positions: vec![[0.0, 0.0, 0.0]; 3],
        indices: vec![0, 1, 2],
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes,
        model_min: [0.0, 0.0, 0.0],
        model_max: [1.0, 1.0, 1.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [1.0, 1.0, 1.0],
    }
}

fn test_submesh(material_id: u32, material_name: &str, first_index: u32) -> SubMesh {
    SubMesh {
        material_name: Some(material_name.to_string()),
        material_id,
        source_material_id: None,
        first_index,
        num_indices: 3,
        first_vertex: 0,
        num_vertices: 3,
        node_parent_index: 0,
    }
}

#[test]
fn blender_uv_writer_flips_v_coordinate() {
    let raw = expanded_blender_uv_data(&[0, 1, 2], &[[0.25, 0.75], [0.5, 0.125], [1.0, 0.0]]);
    let values = raw
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(values, vec![0.25, 0.25, 0.5, 0.875, 1.0, 1.0]);
}

#[test]
fn mesh_to_blend_exports_secondary_uv_map() {
    let mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: Some(vec![[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]]),
        secondary_uvs: Some(vec![[0.25, 0.75], [0.5, 0.125], [1.0, 0.0]]),
        normals: None,
        tangents: None,
        colors: None,
        submeshes: vec![test_submesh(0, "mat", 0)],
        model_min: [0.0, 0.0, 0.0],
        model_max: [1.0, 1.0, 0.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [1.0, 1.0, 0.0],
    };

    let bytes = mesh_to_blend("uv_test", &mesh, &None, None, None);

    assert!(bytes.windows(b"UVMap\0".len()).any(|window| window == b"UVMap\0"));
    assert!(bytes
        .windows(b"UVMap.001\0".len())
        .any(|window| window == b"UVMap.001\0"));
    assert!(bytes
        .windows(0.875f32.to_le_bytes().len())
        .any(|window| window == 0.875f32.to_le_bytes()));

    let blocks = parse_blend_blocks(&bytes);
    let mesh_block = mesh_block_by_name(&blocks, "uv_test");
    let active_uv_ptr = u64::from_le_bytes(mesh_block.data[1584..1592].try_into().unwrap());
    let default_uv_ptr = u64::from_le_bytes(mesh_block.data[1592..1600].try_into().unwrap());
    assert_ne!(active_uv_ptr, 0, "mesh should persist its active UV map name");
    assert_ne!(default_uv_ptr, 0, "mesh should persist its render-active UV map name");
    assert!(blocks.iter().any(|block| block.old_ptr == active_uv_ptr && block.data == b"UVMap\0"));
    assert!(blocks.iter().any(|block| block.old_ptr == default_uv_ptr && block.data == b"UVMap\0"));
}

#[test]
fn scene_blend_reuses_gobo_image_datablocks_by_path() {
    let mut first = test_light("LightA");
    first.lamp_type = 2;
    first.gobo_path = Some("Data/Textures/lights/shared_gobo_TEX0.png".to_string());
    let mut second = test_light("LightB");
    second.lamp_type = 2;
    second.gobo_path = Some("Data/Textures/lights/shared_gobo_TEX0.png".to_string());

    let bytes = create_scene_blend_package_with_instances(
        "DRAK Test_LOD0_TEX0",
        "DRAK Test",
        &[],
        &[first, second],
        &HashMap::new(),
    )
    .unwrap();
    let blocks = parse_blend_blocks(&bytes);
    let image_blocks = blocks
        .iter()
        .filter(|block| block.code == b"IM\0\0")
        .collect::<Vec<_>>();
    assert_eq!(image_blocks.len(), 1);
    assert!(id_block_name(image_blocks[0]).contains("shared_gobo_TEX0.png"));

    let image_ptr = image_blocks[0].old_ptr;
    let image_node_refs = blocks
        .iter()
        .filter(|block| block.sdna == starbreaker_blend::SDNA_IDX_BNODE)
        .filter(|block| block.data.len() > 0xe0 + 8)
        .filter(|block| u64::from_le_bytes(block.data[0xd8..0xe0].try_into().unwrap()) == image_ptr)
        .count();
    assert_eq!(image_node_refs, 2);
}

#[test]
fn mesh_to_blend_exports_decal_offset_displace_modifier_for_decal_vertex_group() {
    let mesh = test_mesh_with_submeshes(vec![test_submesh(0, "decal", 0)]);
    let vertex_groups = vec![VertexGroup {
        name: DECAL_OFFSET_GROUP_NAME.to_string(),
        vertex_indices: vec![0, 1, 2],
    }];

    let bytes = mesh_to_blend("decal_mesh", &mesh, &None, None, Some(&vertex_groups));
    let blocks = parse_blend_blocks(&bytes);
    let object = object_block_by_name(&blocks, "decal_mesh");
    let last_modifier_ptr = u64::from_le_bytes(object.data[664..672].try_into().unwrap());
    let displace = blocks
        .iter()
        .find(|block| block.sdna == SDNA_IDX_DISPLACE_MODIFIER)
        .expect("decal mesh should include a Displace modifier");

    assert_eq!(displace.old_ptr, last_modifier_ptr);
    assert_eq!(cstr_at(displace.data, 40, 64), DECAL_OFFSET_MODIFIER_NAME);
    assert_eq!(cstr_at(displace.data, 288, 64), DECAL_OFFSET_GROUP_NAME);
    assert!((f32::from_le_bytes(displace.data[280..284].try_into().unwrap()) - 0.005).abs() < 0.000001);
}

#[test]
fn flat_mesh_assets_bake_root_rotation_without_wrappers() {
    let mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: vec![test_submesh(0, "mat", 0)],
        model_min: [0.0, 0.0, 0.0],
        model_max: [1.0, 1.0, 0.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [1.0, 1.0, 0.0],
    };

    let blend_bytes = mesh_to_blend("flat_asset", &mesh, &None, None, None);
    let blocks = parse_blend_blocks(&blend_bytes);
    let object = object_block_by_name(&blocks, "flat_asset");

    assert!(
        blocks.iter().all(|block| {
            if block.code != b"OB\0\0" {
                return true;
            }
            let raw = &block.data[42..300];
            let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
            !matches!(&raw[..end], b"StarBreaker_Y_up" | b"CryEngine_Z_up")
        }),
        "flat mesh export should not emit root wrapper empties",
    );

    let quat = [
        f32::from_le_bytes(object.data[820..824].try_into().unwrap()),
        f32::from_le_bytes(object.data[824..828].try_into().unwrap()),
        f32::from_le_bytes(object.data[828..832].try_into().unwrap()),
        f32::from_le_bytes(object.data[832..836].try_into().unwrap()),
    ];
    assert_eq!(
        quat,
        [1.0, 0.0, 0.0, 0.0,]
    );

    let scale = [
        f32::from_le_bytes(object.data[760..764].try_into().unwrap()),
        f32::from_le_bytes(object.data[764..768].try_into().unwrap()),
        f32::from_le_bytes(object.data[768..772].try_into().unwrap()),
    ];
    assert_eq!(scale, [1.0, 1.0, 1.0]);
}

#[test]
fn scene_manifest_instances_inherit_interior_palette_for_placements() {
    let scene = br#"{
        "interiors": [
            {
                "name": "cabin",
                "palette_id": "palette/interior",
                "placements": [
                    {
                        "mesh_asset": "Data/Objects/interior_panel_LOD0.blend",
                        "cgf_path": "Data/Objects/interior_panel.cgf",
                        "material_sidecar": "Data/Materials/interior.materials.json",
                        "palette_id": null,
                        "transform": [[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]
                    }
                ]
            }
        ]
    }"#;
    let files = vec![ExportedFile {
        relative_path: "Packages/Test/scene.json".to_string(),
        bytes: scene.to_vec(),
        kind: ExportedFileKind::PackageManifest,
    }];

    let instances = scene_manifest_instances(&files);

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].palette_id.as_deref(), Some("palette/interior"));
}

#[test]
fn scene_manifest_instances_use_material_sidecar_default_palette() {
    let scene = br#"{
        "children": [
            {
                "entity_name": "seat",
                "mesh_asset": "Data/Objects/seat_LOD0.blend",
                "material_sidecar": "Data/Materials/interior_TEX0.materials.json",
                "palette_id": null
            }
        ]
    }"#;
    let sidecar = br#"{
        "normalized_export_relative_path": "Data/Materials/interior_TEX0.materials.json",
        "authored_material_set": {
            "attributes": [
                {
                    "name": "DefaultPalette",
                    "value": "Libs/Foundry/Records/TintPalettes/Brand/RSI/rsi_interior/rsi_interior_default"
                }
            ]
        }
    }"#;
    let files = vec![
        ExportedFile {
            relative_path: "Packages/Test/scene.json".to_string(),
            bytes: scene.to_vec(),
            kind: ExportedFileKind::PackageManifest,
        },
        ExportedFile {
            relative_path: "Data/Materials/interior_TEX0.materials.json".to_string(),
            bytes: sidecar.to_vec(),
            kind: ExportedFileKind::MaterialSidecar,
        },
    ];

    let instances = scene_manifest_instances(&files);

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].palette_id.as_deref(), Some("palette/rsi_interior_default"));
}

#[test]
fn interior_placement_mesh_geometry_converts_to_scene_axes() {
    let mut mesh = Mesh {
        positions: vec![[1.0, 2.0, 3.0]],
        indices: Vec::new(),
        uvs: None,
        secondary_uvs: None,
        normals: Some(vec![[0.0, 1.0, 0.0]]),
        tangents: Some(vec![[0.0, 0.0, 1.0, 1.0]]),
        colors: None,
        submeshes: Vec::new(),
        model_min: [1.0, 2.0, 3.0],
        model_max: [1.0, 2.0, 3.0],
        scaling_min: [1.0, 2.0, 3.0],
        scaling_max: [1.0, 2.0, 3.0],
    };

    convert_mesh_geometry_to_scene_axes(&mut mesh);

    assert_eq!(mesh.positions[0], [1.0, -3.0, 2.0]);
    assert_eq!(mesh.normals.as_ref().unwrap()[0], [0.0, -0.0, 1.0]);
    assert_eq!(&mesh.tangents.as_ref().unwrap()[0][..3], &[0.0, -1.0, 0.0]);
    assert_eq!(mesh.model_min, [1.0, -3.0, 2.0]);
    assert_eq!(mesh.model_max, [1.0, -3.0, 2.0]);
}

#[test]
fn interior_placement_assets_flatten_source_hierarchy() {
    let mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: vec![test_submesh(0, "mat", 0)],
        model_min: [0.0, 0.0, 0.0],
        model_max: [1.0, 1.0, 0.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [1.0, 1.0, 0.0],
    };
    let nmc = NodeMeshCombo {
        nodes: vec![crate::nmc::NmcNode {
            name: "source_offset_node".to_string(),
            parent_index: None,
            world_to_bone: [
                [1.0, 0.0, 0.0, -5.0],
                [0.0, 1.0, 0.0, -6.0],
                [0.0, 0.0, 1.0, -7.0],
            ],
            bone_to_world: [
                [1.0, 0.0, 0.0, 5.0],
                [0.0, 1.0, 0.0, 6.0],
                [0.0, 0.0, 1.0, 7.0],
            ],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            properties: HashMap::new(),
        }],
        material_indices: vec![0],
    };
    let mut mesh_data_map = HashMap::new();
    mesh_data_map.insert("data/interior_asset.blend".to_string(), MeshDataEntry {
        mesh,
        materials: None,
        nmc: Some(nmc),
        interior_placement_space: true,
    });
    let job = BlendAssetJob {
        blend_path: "Data/interior_asset.blend".to_string(),
        mesh_name: "interior_asset".to_string(),
        blend_key: "data/interior_asset.blend".to_string(),
    };

    let built = build_native_blend_asset(&job, &mesh_data_map, &HashMap::new(), None).unwrap();

    assert!(
        built.source_nodes.iter().all(|node| node.name != "source_offset_node"),
        "interior placement assets should not preserve source NMC nodes that would be applied again in scene.blend"
    );
    assert_eq!(built.linked_mesh_refs.len(), 1);
    assert_eq!(built.linked_mesh_refs[0].object_name, "source_offset_node");
    assert_eq!(built.linked_mesh_refs[0].object_loc, [0.0, 0.0, 0.0]);
    assert_eq!(
        built.linked_mesh_refs[0].object_quat,
        [1.0, 0.0, 0.0, 0.0,]
    );
    assert_eq!(built.linked_mesh_refs[0].source_parent_name.as_deref(), None);
}

#[test]
fn existing_native_blend_asset_is_reused_for_scene_link_metadata() {
    let mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: vec![test_submesh(0, "mat", 0)],
        model_min: [0.0, 0.0, 0.0],
        model_max: [1.0, 1.0, 0.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [1.0, 1.0, 0.0],
    };
    let existing_bytes = starbreaker_blend::compress_blend_bytes(&mesh_to_blend(
        "existing_asset",
        &mesh,
        &None,
        None,
        None,
    ));
    let job = BlendAssetJob {
        blend_path: "Data/Objects/existing_asset_LOD0.blend".to_string(),
        mesh_name: "existing_asset_LOD0".to_string(),
        blend_key: "data/objects/existing_asset_lod0.blend".to_string(),
    };
    let loader = |relative_path: &str| {
        (relative_path == "Data/Objects/existing_asset_LOD0.blend").then(|| existing_bytes.clone())
    };

    let built = build_native_blend_asset(&job, &HashMap::new(), &HashMap::new(), Some(&loader)).unwrap();

    assert!(built.file.is_none(), "reused assets should not be queued for rewriting");
    assert_eq!(built.relative_path, "Data/Objects/existing_asset_LOD0.blend");
    assert_eq!(built.linked_mesh_refs.len(), 1);
    assert_eq!(built.linked_mesh_refs[0].object_name, "existing_asset");
    assert_eq!(built.linked_mesh_refs[0].mesh_name, "existing_asset");
}

#[test]
fn test_blend_material_slots_use_glb_style_names_and_deduplicate_ids() {
    let mesh = test_mesh_with_submeshes(vec![
        test_submesh(3, "Decal_POM", 0),
        test_submesh(3, "Decal_POM", 3),
        test_submesh(7, "Painted_Metal", 6),
    ]);

    let (names, submesh_slots) = blend_material_slots("fallback", &mesh, &None);

    assert_eq!(
        names,
        vec![
            "fallback_mtl_Decal_POM_03".to_string(),
            "fallback_mtl_Painted_Metal_07".to_string(),
        ]
    );
    assert_eq!(submesh_slots, vec![0, 0, 1]);
}

#[test]
fn test_mesh_blend_with_nmc_writes_hierarchy_objects() {
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
        uvs: Some(vec![[0.0, 0.0]; 6]),
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: Some(vec![[255, 255, 255, 255]; 6]),
        submeshes: vec![
            SubMesh {
                material_name: Some("Hull".to_string()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            },
            SubMesh {
                material_name: Some("Decal".to_string()),
                material_id: 1,
                source_material_id: None,
                first_index: 3,
                num_indices: 3,
                first_vertex: 3,
                num_vertices: 3,
                node_parent_index: 2,
            },
        ],
        model_min: [0.0, 0.0, 0.0],
        model_max: [3.0, 1.0, 0.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [3.0, 1.0, 0.0],
    };
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let nmc = NodeMeshCombo {
        nodes: vec![
            crate::nmc::NmcNode {
                name: "asset_root".to_string(),
                parent_index: None,
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 3,
                properties: HashMap::new(),
            },
            crate::nmc::NmcNode {
                name: "geo_left".to_string(),
                parent_index: Some(0),
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 0,
                properties: HashMap::new(),
            },
            crate::nmc::NmcNode {
                name: "geo_right".to_string(),
                parent_index: Some(0),
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 0,
                properties: HashMap::new(),
            },
        ],
        material_indices: vec![],
    };

    let blend_bytes = mesh_to_blend("asset_LOD0", &mesh, &None, Some(&nmc), None);
    let blocks = parse_blend_blocks(&blend_bytes);
    let object_names = blocks
        .iter()
        .filter(|block| block.code == b"OB\0\0")
        .map(|block| {
            let raw = &block.data[42..300];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).to_string()
        })
        .collect::<Vec<_>>();

    assert!(!object_names.contains(&"CryEngine_Z_up".to_string()));
    assert!(!object_names.contains(&"StarBreaker_Y_up".to_string()));
    assert!(object_names.contains(&"asset".to_string()));
    assert!(object_names.contains(&"asset_root".to_string()));
    assert!(object_names.contains(&"geo_left".to_string()));
    assert!(object_names.contains(&"geo_right".to_string()));
    assert!(!object_names.contains(&"asset_LOD0".to_string()));
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.code == b"ME\0\0")
            .count(),
        2,
        "each geometry-bearing NMC node should get its own mesh"
    );
    let mesh_material_counts = blocks
        .iter()
        .filter(|block| block.code == b"ME\0\0")
        .map(|block| i16::from_le_bytes(block.data[1618..1620].try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        mesh_material_counts,
        vec![1, 1],
        "split NMC mesh objects should only reference the material slots used by that object"
    );
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.code == b"MA\0\0")
            .count(),
        2,
        "the file should still contain the full material table for all split objects"
    );
    assert!(
        object_names.len() > 1,
        "NMC export must not be a single flat mesh object"
    );
    assert!(!object_names.contains(&"CryEngine_Z_up".to_string()));
    assert!(!object_names.contains(&"StarBreaker_Y_up".to_string()));
    assert!(object_names.contains(&"asset_root".to_string()));
    assert!(object_names.contains(&"asset".to_string()));
    assert!(object_names.contains(&"geo_left".to_string()));
    assert!(object_names.contains(&"geo_right".to_string()));
}

#[test]
fn empty_only_nmc_asset_writes_linkable_anchor_mesh() {
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let nmc = NodeMeshCombo {
        nodes: vec![
            crate::nmc::NmcNode {
                name: "EmptyHelper".to_string(),
                parent_index: None,
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 3,
                properties: HashMap::new(),
            },
            crate::nmc::NmcNode {
                name: "IP_child".to_string(),
                parent_index: Some(0),
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 3,
                properties: HashMap::new(),
            },
        ],
        material_indices: Vec::new(),
    };

    let blend_bytes = mesh_to_blend("EmptyHelper_LOD0", &empty_anchor_mesh(), &None, Some(&nmc), None);
    let blocks = parse_blend_blocks(&blend_bytes);

    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.code == b"ME\0\0")
            .count(),
        1,
        "empty-only helper assets still need a real mesh datablock for scene library links"
    );
    let (linked_refs, source_nodes) = blend_link_data_from_bytes(&blend_bytes);
    assert_eq!(linked_refs.len(), 1);
    assert_eq!(linked_refs[0].mesh_name, "EmptyHelper_LOD0");
    assert!(
        source_nodes.iter().any(|node| node.name == "IP_child"),
        "the helper source-empty tree must still be available for child parenting"
    );
}

#[test]
fn collapsed_wrapper_node_preserves_non_identity_root_transform() {
    let mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: vec![SubMesh {
            material_name: Some("Hull".to_string()),
            material_id: 0,
            source_material_id: None,
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 1,
        }],
        model_min: [0.0, 0.0, 0.0],
        model_max: [1.0, 1.0, 0.0],
        scaling_min: [0.0, 0.0, 0.0],
        scaling_max: [1.0, 1.0, 0.0],
    };
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let root_rot_180_z = [
        [-1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let nmc = NodeMeshCombo {
        nodes: vec![
            crate::nmc::NmcNode {
                name: "asset".to_string(),
                parent_index: None,
                world_to_bone: root_rot_180_z,
                bone_to_world: root_rot_180_z,
                scale: [1.0; 3],
                geometry_type: 3,
                properties: HashMap::new(),
            },
            crate::nmc::NmcNode {
                name: "geo".to_string(),
                parent_index: Some(0),
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 0,
                properties: HashMap::new(),
            },
        ],
        material_indices: Vec::new(),
    };

    let blend_bytes = mesh_to_blend("asset_LOD0", &mesh, &None, Some(&nmc), None);
    let blocks = parse_blend_blocks(&blend_bytes);
    let wrapper = object_block_by_name(&blocks, "asset");
    let quat = [
        f32::from_le_bytes(wrapper.data[820..824].try_into().unwrap()),
        f32::from_le_bytes(wrapper.data[824..828].try_into().unwrap()),
        f32::from_le_bytes(wrapper.data[828..832].try_into().unwrap()),
        f32::from_le_bytes(wrapper.data[832..836].try_into().unwrap()),
    ];

    assert!(quat[0].abs() < 1e-5, "expected 180-degree root rotation (w ~= 0), got {quat:?}");
    assert!(quat[1].abs() < 1e-5, "expected 180-degree root rotation around Z, got {quat:?}");
    assert!(quat[2].abs() < 1e-5, "expected 180-degree root rotation around Z, got {quat:?}");
    assert!(
        (quat[3].abs() - 1.0).abs() < 1e-5,
        "expected 180-degree root rotation around Z (|z| ~= 1), got {quat:?}"
    );
}

#[test]
fn linked_scene_object_names_use_geometry_nodes_for_nmc_assets() {
    let mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: vec![
            SubMesh {
                material_name: Some("Left".to_string()),
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            },
            SubMesh {
                material_name: Some("Right".to_string()),
                material_id: 1,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 2,
            },
        ],
        model_min: [0.0; 3],
        model_max: [1.0; 3],
        scaling_min: [0.0; 3],
        scaling_max: [1.0; 3],
    };
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let nmc = NodeMeshCombo {
        nodes: vec![
            crate::nmc::NmcNode {
                name: "asset".to_string(),
                parent_index: None,
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 0,
                properties: HashMap::new(),
            },
            crate::nmc::NmcNode {
                name: "geo_left".to_string(),
                parent_index: Some(0),
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 0,
                properties: HashMap::new(),
            },
            crate::nmc::NmcNode {
                name: "geo_right".to_string(),
                parent_index: Some(0),
                world_to_bone: identity,
                bone_to_world: identity,
                scale: [1.0; 3],
                geometry_type: 0,
                properties: HashMap::new(),
            },
        ],
        material_indices: vec![],
    };

    assert_eq!(
        linked_scene_object_names("asset_LOD0", &mesh, Some(&nmc)),
        vec!["geo_left".to_string(), "geo_right".to_string()]
    );
    assert_eq!(
        linked_scene_object_names("asset_LOD0", &mesh, None),
        vec!["asset_LOD0".to_string()]
    );
}

#[test]
fn test_create_scene_blend_links_object_ids_instead_of_empty_mesh_stubs() {
    let instance = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "rsi_aurora_mk2_airlock_door_LOD0".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "rsi_aurora_mk2_airlock_door_LOD0".to_string(),
        name: "rsi_aurora_mk2_airlock_door_LOD0".to_string(),
        mesh_name: "rsi_aurora_mk2_airlock_door_LOD0_mesh".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/rsi_aurora_mk2_airlock_door_LOD0.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/rsi_aurora_mk2_airlock_door_LOD0.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let blend_bytes = create_scene_blend_with_instances("SceneLinkTest", &[instance], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let linked_mesh_stub = blocks.iter().find(|block| {
        block.code == b"ID\0\0"
            && block.sdna == SDNA_IDX_ID
            && block.data[40..]
                .starts_with(b"MErsi_aurora_mk2_airlock_door_LOD0_mesh")
    });
    let linked_mesh_stub = linked_mesh_stub.expect("scene.blend should link mesh data IDs from mesh .blend files");

    let local_object = object_block_by_name(&blocks, "rsi_aurora_mk2_airlock_door_LOD0");
    assert_eq!(
        u64::from_le_bytes(local_object.data[552..560].try_into().unwrap()),
        linked_mesh_stub.old_ptr,
        "local scene Object.data should point at the linked Mesh ID stub"
    );

    let local_empty_mesh_stub = blocks.iter().find(|block| {
        block.code == b"ME\0\0"
            && block.data[40..]
                .starts_with(b"MErsi_aurora_mk2_airlock_door_LOD0")
    });
    assert!(local_empty_mesh_stub.is_none(), "scene.blend must not replace linked objects with empty local mesh stubs");
}

#[test]
fn test_create_scene_blend_writes_addon_style_scene_anchors() {
    let instance = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "anchor_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "anchor_mesh".to_string(),
        name: "anchor_mesh".to_string(),
        mesh_name: "anchor_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: vec![
            LinkedSourceAncestor {
                name: "StarBreaker_Y_up".to_string(),
                loc: [0.0, 0.0, 0.0],
                quat: [std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
            LinkedSourceAncestor {
                name: "CryEngine_Z_up".to_string(),
                loc: [0.0, 1.0, 0.0],
                quat: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
        ],
        source_loc: [4.0, 5.0, 6.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/anchor_mesh.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/anchor_mesh.blend".to_string(),
        position: [1.0, 2.0, 3.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let blend_bytes = create_scene_blend_with_instances("AnchorEntity", &[instance], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let package_root = object_block_by_name(&blocks, "StarBreaker AnchorEntity");
    let entity_root = object_block_by_name(&blocks, "AnchorEntity");
    let anchor = object_block_by_name(&blocks, "anchor_mesh_anchor");
    let y_up = object_block_by_name(&blocks, "anchor_mesh_0_StarBreaker_Y_up");
    let cryengine_root = object_block_by_name(&blocks, "anchor_mesh_1_CryEngine_Z_up");
    let local_mesh = object_block_by_name(&blocks, "anchor_mesh");

    assert_eq!(u64::from_le_bytes(package_root.data[496..504].try_into().unwrap()), 0);
    assert_eq!(
        u64::from_le_bytes(entity_root.data[496..504].try_into().unwrap()),
        package_root.old_ptr,
        "entity wrapper should be parented to package root"
    );
    assert_eq!(
        [
            f32::from_le_bytes(entity_root.data[820..824].try_into().unwrap()),
            f32::from_le_bytes(entity_root.data[824..828].try_into().unwrap()),
            f32::from_le_bytes(entity_root.data[828..832].try_into().unwrap()),
            f32::from_le_bytes(entity_root.data[832..836].try_into().unwrap()),
        ],
        [1.0, 0.0, 0.0, 0.0,],
        "entity wrapper should carry the baked root rotation",
    );
    assert_eq!(
        [
            f32::from_le_bytes(entity_root.data[760..764].try_into().unwrap()),
            f32::from_le_bytes(entity_root.data[764..768].try_into().unwrap()),
            f32::from_le_bytes(entity_root.data[768..772].try_into().unwrap()),
        ],
        [1.0, 1.0, 1.0],
    );
    assert_eq!(
        u64::from_le_bytes(anchor.data[496..504].try_into().unwrap()),
        entity_root.old_ptr,
        "scene instance anchor should be parented to entity wrapper"
    );
    assert_eq!(
        u64::from_le_bytes(y_up.data[496..504].try_into().unwrap()),
        anchor.old_ptr,
        "source hierarchy root should be parented to its scene instance anchor"
    );
    assert_eq!(
        u64::from_le_bytes(cryengine_root.data[496..504].try_into().unwrap()),
        y_up.old_ptr,
        "source child empty should preserve its source parent chain"
    );
    assert_eq!(
        u64::from_le_bytes(local_mesh.data[496..504].try_into().unwrap()),
        cryengine_root.old_ptr,
        "local mesh object should be parented to the cloned source hierarchy"
    );
    assert_eq!(f32::from_le_bytes(local_mesh.data[736..740].try_into().unwrap()), 4.0);
    assert_eq!(f32::from_le_bytes(local_mesh.data[740..744].try_into().unwrap()), 5.0);
    assert_eq!(f32::from_le_bytes(local_mesh.data[744..748].try_into().unwrap()), 6.0);
    assert_ne!(u64::from_le_bytes(package_root.data[344..352].try_into().unwrap()), 0);
    assert_ne!(u64::from_le_bytes(entity_root.data[344..352].try_into().unwrap()), 0);
    assert_ne!(u64::from_le_bytes(anchor.data[344..352].try_into().unwrap()), 0);
    assert_ne!(u64::from_le_bytes(local_mesh.data[344..352].try_into().unwrap()), 0);
    assert!(blend_bytes.windows(b"starbreaker_package_root".len()).any(|w| w == b"starbreaker_package_root"));
    assert!(blend_bytes.windows(b"starbreaker_mesh_asset".len()).any(|w| w == b"starbreaker_mesh_asset"));
    assert!(blend_bytes.windows(b"Packages/AnchorEntity/scene.json".len()).any(|w| w == b"Packages/AnchorEntity/scene.json"));
    assert!(blend_bytes.windows(b"Data/Objects/Ships/anchor_mesh.blend".len()).any(|w| w == b"Data/Objects/Ships/anchor_mesh.blend"));
}

#[test]
fn test_interior_meshes_do_not_parent_to_global_coordinate_nodes() {
    // Exterior mesh has a StarBreaker_Y_up source node (legacy or explicit in input data).
    // Interior mesh has NO such node — only regular source helpers.
    // The interior mesh must NOT accidentally parent to the exterior's StarBreaker_Y_up node.
    let exterior = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "geo_body".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "geo_body".to_string(),
        name: "geo_body".to_string(),
        mesh_name: "geo_body_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "StarBreaker_Y_up".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("StarBreaker_Y_up".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/geo_body.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/geo_body.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let interior = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "interior_panel".to_string(),
        parent_entity_name: None,
        parent_empty_name: Some("interior_cabin".to_string()),
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 5.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: true,
        source_object_name: "interior_panel".to_string(),
        name: "interior_panel".to_string(),
        mesh_name: "interior_panel_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "interior_helper".to_string(),
            parent_name: None,
            loc: [1.0, 2.0, 3.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("interior_helper".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/interior_panel.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/interior_panel.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let blend_bytes = create_scene_blend_with_instances("InteriorParentEntity", &[exterior, interior], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let exterior_y_up = object_block_by_name(&blocks, "geo_body_0_StarBreaker_Y_up");
    let interior_anchor = object_block_by_name(&blocks, "interior_panel_anchor");
    let interior_helper = object_block_by_name(&blocks, "interior_panel_0_interior_helper");
    let interior_mesh = object_block_by_name(&blocks, "interior_panel");

    assert_eq!(
        u64::from_le_bytes(interior_helper.data[496..504].try_into().unwrap()),
        interior_anchor.old_ptr,
        "interior source node with no parent should fall back to the scene anchor"
    );
    assert_eq!(
        u64::from_le_bytes(interior_mesh.data[496..504].try_into().unwrap()),
        interior_helper.old_ptr,
        "interior mesh should parent to its interior_helper source node"
    );
    assert_ne!(
        u64::from_le_bytes(interior_mesh.data[496..504].try_into().unwrap()),
        exterior_y_up.old_ptr,
        "interior mesh must not accidentally parent to the exterior StarBreaker_Y_up"
    );
}

#[test]
fn test_create_scene_blend_uses_full_source_empty_tree_for_parent_nodes() {
    let root = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "root_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "root_mesh".to_string(),
        name: "root_mesh".to_string(),
        mesh_name: "root_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![
            LinkedSourceNode {
                name: "hardpoint_child".to_string(),
                parent_name: Some("Root".to_string()),
                loc: [1.0, 2.0, 3.0],
                quat: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
            LinkedSourceNode {
                name: "Root".to_string(),
                parent_name: None,
                loc: [0.0, 0.0, 0.0],
                quat: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
        ],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("hardpoint_child".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/root.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/root.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let child = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "child_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "child_mesh".to_string(),
        name: "child_mesh".to_string(),
        mesh_name: "child_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: Some("hardpoint_child".to_string()),
        blend_path: "//../../Data/Objects/Ships/child.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/child.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let blend_bytes = create_scene_blend_with_instances("ParentNodeEntity", &[root, child], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let hardpoint = object_block_by_name(&blocks, "root_mesh_0_hardpoint_child");
    let root_empty = object_block_by_name(&blocks, "root_mesh_1_Root");
    let root_anchor = object_block_by_name(&blocks, "root_mesh_anchor");
    let child_anchor = object_block_by_name(&blocks, "child_mesh_anchor");
    let root_mesh = object_block_by_name(&blocks, "root_mesh");

    assert_eq!(
        u64::from_le_bytes(hardpoint.data[496..504].try_into().unwrap()),
        root_empty.old_ptr,
        "source-empty parent resolution must be independent of source node block order"
    );
    assert_eq!(
        u64::from_le_bytes(root_empty.data[496..504].try_into().unwrap()),
        root_anchor.old_ptr,
        "source-empty hierarchy roots should attach to the scene instance anchor"
    );
    assert_eq!(
        u64::from_le_bytes(child_anchor.data[496..504].try_into().unwrap()),
        hardpoint.old_ptr,
        "scene-record anchors should snap to source empties that do not contain mesh geometry"
    );
    assert_eq!(
        u64::from_le_bytes(root_mesh.data[496..504].try_into().unwrap()),
        hardpoint.old_ptr,
        "mesh objects should reuse the full source empty tree instead of duplicating ancestor empties"
    );
}

#[test]
fn test_create_scene_blend_parents_source_empty_to_local_mesh_object() {
    let instance = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "parent_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "parent_mesh".to_string(),
        name: "parent_mesh".to_string(),
        mesh_name: "parent_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "child_empty".to_string(),
            parent_name: Some("parent_mesh".to_string()),
            loc: [1.0, 2.0, 3.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/parent.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/parent.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let blend_bytes = create_scene_blend_with_instances("MeshParentEntity", &[instance], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let parent_mesh = object_block_by_name(&blocks, "parent_mesh");
    let child_empty = object_block_by_name(&blocks, "parent_mesh_0_child_empty");

    assert_eq!(
        u64::from_le_bytes(child_empty.data[496..504].try_into().unwrap()),
        parent_mesh.old_ptr,
        "source empties whose source parent is a mesh object should not fall back to the entity root"
    );
}

#[test]
fn test_create_scene_blend_uses_instance_local_source_parent_before_global_name() {
    let first = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "first_instance_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "first_instance_mesh".to_string(),
        name: "first_instance_mesh".to_string(),
        mesh_name: "first_instance_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "shared_source_parent".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("shared_source_parent".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/shared.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/shared.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let second = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "second_instance_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "second_instance_mesh".to_string(),
        name: "second_instance_mesh".to_string(),
        mesh_name: "second_instance_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "shared_source_parent".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("shared_source_parent".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/shared.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/shared.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let blend_bytes = create_scene_blend_with_instances("LocalSourceParentEntity", &[first, second], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let first_parent = object_block_by_name(&blocks, "first_instance_mesh_0_shared_source_parent");
    let second_parent = object_block_by_name(&blocks, "second_instance_mesh_0_shared_source_parent");
    let first_mesh = object_block_by_name(&blocks, "first_instance_mesh");
    let second_mesh = object_block_by_name(&blocks, "second_instance_mesh");

    assert!(blend_bytes
        .windows(b"shared_source_parent".len())
        .any(|window| window == b"shared_source_parent"));
    assert_eq!(
        u64::from_le_bytes(first_mesh.data[496..504].try_into().unwrap()),
        first_parent.old_ptr,
        "first instance should use its local source parent clone"
    );
    assert_eq!(
        u64::from_le_bytes(second_mesh.data[496..504].try_into().unwrap()),
        second_parent.old_ptr,
        "second instance should not attach to the first instance's source parent clone"
    );
}

#[test]
fn test_create_scene_blend_reuses_source_tree_for_same_scene_instance_mesh_refs() {
    let global_decoy = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "decoy_mesh".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "decoy_mesh".to_string(),
        name: "decoy_mesh".to_string(),
        mesh_name: "decoy_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "mav_main".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("mav_main".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/decoy.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/decoy.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let instance_source_tree_holder = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "mav_housing".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "housing_mesh".to_string(),
        name: "housing_mesh".to_string(),
        mesh_name: "housing_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "mav_main".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("mav_main".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/mav.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/mav.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let second_mesh_ref_same_scene_instance = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "geo_mav_main".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "geo_mav_main".to_string(),
        name: "geo_mav_main".to_string(),
        mesh_name: "geo_mav_main_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: Some("mav_main".to_string()),
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/mav.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/mav.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };

    let blend_bytes = create_scene_blend_with_instances(
        "MavEntity",
        &[global_decoy, instance_source_tree_holder, second_mesh_ref_same_scene_instance],
        &[],
    )
    .unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let decoy_parent = object_block_by_name(&blocks, "decoy_mesh_0_mav_main");
    let instance_parent = object_block_by_name(&blocks, "housing_mesh_0_mav_main");
    let mav_mesh = object_block_by_name(&blocks, "geo_mav_main");

    assert_ne!(decoy_parent.old_ptr, instance_parent.old_ptr);
    assert_eq!(
        u64::from_le_bytes(mav_mesh.data[496..504].try_into().unwrap()),
        instance_parent.old_ptr,
        "a second linked mesh ref from the same asset instance must use that instance's source tree, not the first global same-named source node"
    );
}

#[test]
fn test_create_scene_blend_resolves_parent_node_against_matching_parent_entity_instance() {
    let first_parent = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "Mount_Gimbal_S2".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "first_gimbal_mesh".to_string(),
        name: "first_gimbal_mesh".to_string(),
        mesh_name: "first_gimbal_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "hardpoint_class_2".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/gimbal.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/gimbal.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let first_weapon = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "KLWE_LaserRepeater_S2".to_string(),
        parent_entity_name: Some("Mount_Gimbal_S2".to_string()),
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "first_weapon_mesh".to_string(),
        name: "first_weapon_mesh".to_string(),
        mesh_name: "first_weapon_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: Some("hardpoint_class_2".to_string()),
        blend_path: "//../../Data/Objects/Ships/weapon.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/weapon.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let second_parent = LinkedMeshInstance {
        scene_instance_id: 2,
        entity_name: "Mount_Gimbal_S2".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "second_gimbal_mesh".to_string(),
        name: "second_gimbal_mesh".to_string(),
        mesh_name: "second_gimbal_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "hardpoint_class_2".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/gimbal.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/gimbal.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let second_weapon = LinkedMeshInstance {
        scene_instance_id: 3,
        entity_name: "KLWE_LaserRepeater_S2".to_string(),
        parent_entity_name: Some("Mount_Gimbal_S2".to_string()),
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "second_weapon_mesh".to_string(),
        name: "second_weapon_mesh".to_string(),
        mesh_name: "second_weapon_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: Some("hardpoint_class_2".to_string()),
        blend_path: "//../../Data/Objects/Ships/weapon.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/weapon.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };

    let blend_bytes = create_scene_blend_with_instances(
        "RepeatedParentEntity",
        &[first_parent, first_weapon, second_parent, second_weapon],
        &[],
    )
    .unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let first_hardpoint = object_block_by_name(&blocks, "first_gimbal_mesh_0_hardpoint_class_2");
    let second_hardpoint = object_block_by_name(&blocks, "second_gimbal_mesh_0_hardpoint_class_2");
    let first_anchor = object_block_by_name(&blocks, "first_weapon_mesh_anchor");
    let second_anchor = object_block_by_name(&blocks, "second_weapon_mesh_anchor");

    assert_eq!(
        u64::from_le_bytes(first_anchor.data[496..504].try_into().unwrap()),
        first_hardpoint.old_ptr
    );
    assert_eq!(
        u64::from_le_bytes(second_anchor.data[496..504].try_into().unwrap()),
        second_hardpoint.old_ptr
    );
}

#[test]
fn test_create_scene_blend_resolves_parent_node_case_insensitively() {
    let parent = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "RetroMount".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "retro_mount_mesh".to_string(),
        name: "retro_mount_mesh".to_string(),
        mesh_name: "retro_mount_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: vec![LinkedSourceNode {
            name: "hardpoint_Right_Main_Retro".to_string(),
            parent_name: None,
            loc: [0.0, 0.0, 0.0],
            quat: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }],
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/Ships/retro_mount.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/retro_mount.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let child = LinkedMeshInstance {
        scene_instance_id: 1,
        entity_name: "RetroThruster".to_string(),
        parent_entity_name: Some("RetroMount".to_string()),
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "retro_thruster_mesh".to_string(),
        name: "retro_thruster_mesh".to_string(),
        mesh_name: "retro_thruster_mesh_data".to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_nodes: Vec::new(),
        source_ancestors: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: Some("hardpoint_right_main_retro".to_string()),
        blend_path: "//../../Data/Objects/Ships/retro_thruster.blend".to_string(),
        mesh_asset: "Data/Objects/Ships/retro_thruster.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };

    let blend_bytes =
        create_scene_blend_with_instances("CaseInsensitiveParent", &[parent, child], &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let hardpoint = object_block_by_name(&blocks, "retro_mount_mesh_0_hardpoint_Right_Main_Retro");
    let child_anchor = object_block_by_name(&blocks, "retro_thruster_mesh_anchor");

    assert_eq!(
        u64::from_le_bytes(child_anchor.data[496..504].try_into().unwrap()),
        hardpoint.old_ptr,
    );
}

#[test]
fn test_create_scene_blend_parents_lights_to_entity_wrapper_with_properties() {
    let light = test_light("SceneLight");
    let blend_bytes = create_scene_blend("LightEntity", 0, "Data/Objects", &[light]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let entity_root = object_block_by_name(&blocks, "LightEntity");
    let light_object = object_block_by_name(&blocks, "SceneLight");

    assert_eq!(
        u64::from_le_bytes(light_object.data[496..504].try_into().unwrap()),
        entity_root.old_ptr
    );
    assert_ne!(u64::from_le_bytes(light_object.data[344..352].try_into().unwrap()), 0);
    assert!(blend_bytes.windows(b"starbreaker_source_node_name".len()).any(|w| w == b"starbreaker_source_node_name"));
}

#[test]
fn scene_blend_makes_duplicate_light_id_names_unique() {
    let blend_bytes = create_scene_blend(
        "LightEntity",
        0,
        "Data/Objects",
        &[test_light("RepeatedLight"), test_light("RepeatedLight")],
    )
    .unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let object_names = id_block_names(&blocks, b"OB\0\0");
    let lamp_names = id_block_names(&blocks, b"LA\0\0");

    assert_eq!(object_names.iter().filter(|name| name.as_str() == "RepeatedLight").count(), 1);
    assert!(object_names.iter().any(|name| name == "RepeatedLight_001"));
    assert_eq!(lamp_names.iter().filter(|name| name.as_str() == "RepeatedLight").count(), 1);
    assert!(lamp_names.iter().any(|name| name == "RepeatedLight_001"));
}

/// Helper to create a minimal DecomposedInput for testing
fn create_test_input(
    entity_name: &str,
    num_children: usize,
) -> DecomposedInput {
    let root_mesh = Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
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
    };

    let mut children = Vec::new();
    for i in 0..num_children {
        children.push(crate::types::EntityPayload {
            mesh: root_mesh.clone(),
            materials: None,
            textures: None,
            nmc: None,
            palette: None,
            geometry_path: format!("path/to/mesh_{}", i),
            material_path: format!("path/to/mat_{}", i),
            bones: vec![],
            skeleton_source_path: None,
            entity_name: format!("child_{}", i),
            entity_category: None,
            parent_node_name: "Root".to_string(),
            parent_entity_name: entity_name.to_string(),
            no_rotation: false,
            offset_position: [0.0, 0.0, 0.0],
            offset_rotation: [0.0, 0.0, 0.0],
            detach_direction: [0.0, 0.0, 0.0],
            port_flags: String::new(),
        });
    }

    DecomposedInput {
        entity_name: entity_name.to_string(),
        geometry_path: "path/to/geometry".to_string(),
        material_path: "path/to/materials".to_string(),
        root_mesh,
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children,
        interiors: LoadedInteriors {
            unique_cgfs: vec![],
            containers: vec![],
        },
        paint_variants: vec![],
    }
}

#[test]
fn test_manifest_scene_transform_uses_gltf_y_up_local_transform() {
    let record = serde_json::json!({
        "source_transform_basis": "gltf_y_up",
        "offset_position": [9.0, 9.0, 9.0],
        "local_transform_sc": [
            [0.0, -1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [1.25, 2.5, 3.75, 1.0]
        ]
    });

    let (loc, quat, scale) = manifest_scene_transform(&record);

    assert_eq!(loc, [1.25, 2.5, 3.75]);
    assert_eq!(scale, [1.0, 1.0, 1.0]);
    assert!(
        (quat[0].abs() - std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-5,
        "expected 90 degree Z rotation quaternion, got {quat:?}"
    );
    assert!(
        (quat[3].abs() - std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-5,
        "expected 90 degree Z rotation quaternion, got {quat:?}"
    );
}

#[test]
fn test_scene_manifest_instances_mark_invisible_port_hidden() {
    let scene = serde_json::json!({
        "children": [{
            "entity_name": "HiddenComponent",
            "mesh_asset": "Data/Objects/hidden_component.blend",
            "port_flags": "invisible uneditable"
        }]
    });
    let manifest = vec![ExportedFile {
        relative_path: "Packages/Test/scene.json".to_string(),
        bytes: serde_json::to_vec(&scene).unwrap(),
        kind: ExportedFileKind::PackageManifest,
    }];

    let instances = scene_manifest_instances(&manifest);

    assert_eq!(instances.len(), 1);
    assert!(instances[0].hidden, "invisible port flags should hide, not skip, the scene instance");
}

#[test]
fn test_create_scene_blend_hides_invisible_linked_instance_objects() {
    let instance = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "HiddenComponent".to_string(),
        parent_entity_name: None,
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        is_interior: false,
        source_object_name: "HiddenMesh".to_string(),
        name: "HiddenMesh".to_string(),
        mesh_name: "HiddenMesh".to_string(),
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
        blend_path: "//../../Data/Objects/hidden_component.blend".to_string(),
        mesh_asset: "Data/Objects/hidden_component.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: true,
    };

    let blend_bytes = create_scene_blend_package_with_instances("Test", "Test", &[instance], &[], &HashMap::new()).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let anchor = object_block_by_name(&blocks, "HiddenMesh_anchor");
    let mesh = object_block_by_name(&blocks, "HiddenMesh");

    assert_eq!(i16::from_le_bytes(anchor.data[1082..1084].try_into().unwrap()), 0x0005);
    assert_eq!(i16::from_le_bytes(mesh.data[1082..1084].try_into().unwrap()), 0x0005);
}

#[test]
fn test_create_scene_blend_writes_decal_offset_modifier_for_decal_mesh_asset() {
    let instance = LinkedMeshInstance {
        scene_instance_id: 0,
        entity_name: "HullComponent".to_string(),
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
        material_sidecar: Some("Data/Objects/Spaceships/Ships/DRAK/Clipper/drak_clipper_ext_TEX0.materials.json".to_string()),
        palette_id: None,
        source_ancestors: Vec::new(),
        source_nodes: Vec::new(),
        source_loc: [0.0, 0.0, 0.0],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0, 1.0, 1.0],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: "//../../Data/Objects/hull_component.blend".to_string(),
        mesh_asset: "Data/Objects/hull_component.blend".to_string(),
        position: [0.0, 0.0, 0.0],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        hidden: false,
    };
    let decal_mesh_refs = HashSet::from([(
        "data/objects/hull_component.blend".to_string(),
        "HullMesh".to_string(),
    )]);

    let blend_bytes = create_scene_blend_package_with_instances_and_decal_offsets(
        "Test",
        "Test",
        &[instance],
        &[],
        &HashMap::new(),
        &decal_mesh_refs,
    )
    .unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let object = object_block_by_name(&blocks, "HullMesh");
    let last_modifier_ptr = u64::from_le_bytes(object.data[664..672].try_into().unwrap());
    let displace = blocks
        .iter()
        .find(|block| block.sdna == SDNA_IDX_DISPLACE_MODIFIER)
        .expect("scene mesh instance should include a Displace modifier");

    assert_eq!(displace.old_ptr, last_modifier_ptr);
    assert_eq!(cstr_at(displace.data, 40, 64), DECAL_OFFSET_MODIFIER_NAME);
    assert_eq!(cstr_at(displace.data, 288, 64), DECAL_OFFSET_GROUP_NAME);
    assert!((f32::from_le_bytes(displace.data[280..284].try_into().unwrap()) - 0.005).abs() < 0.000001);
}

#[test]
fn test_create_scene_blend_basic() {
    let result = create_scene_blend("TestEntity", 1, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed with basic input");
    
    let blend_bytes = result.unwrap();
    assert!(!blend_bytes.is_empty(), "Output should not be empty");
    assert!(blend_bytes.len() > 100, "Output should be substantial");
    
    // Verify BLENDER17 magic header
    assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501", "Should have valid Blender header");
}

#[test]
fn test_create_scene_blend_multiple_meshes() {
    let result = create_scene_blend("MultiMesh", 5, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed with multiple children");
    
    let blend_bytes = result.unwrap();
    assert!(!blend_bytes.is_empty(), "Output should not be empty");
    
    // Verify BLENDER17 magic header
    assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501", "Should have valid Blender header");
    
    // With 5 children, the file should be larger than with 1
    let single = create_scene_blend("Single", 1, "Data/Objects", &[])
        .unwrap();
    assert!(blend_bytes.len() > single.len(), "Multiple meshes should produce larger file");
}

#[test]
fn test_create_scene_blend_collections_structure() {
    let result = create_scene_blend("CollTest", 2, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed");
    
    let blend_bytes = result.unwrap();

    // The output should contain collection markers (GRP\0 blocks).
    // We can't easily verify collection structure without parsing the binary format,
    // but we can verify the file format is valid.
    assert!(blend_bytes.len() > 200, "Valid scene file should be substantial");
}

#[test]
fn test_create_scene_blend_file_format() {
    let result = create_scene_blend("FormatTest", 1, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed");
    
    let blend_bytes = result.unwrap();
    
    // Verify file structure markers
    // BLENDER17 header
    assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501");
    
    // Find GLOB block (should appear early)
    let glob_marker = b"GLOB";
    assert!(blend_bytes.windows(4).any(|w| w == glob_marker), "Should contain GLOB block");
    
    // Find ENDB marker (should be at end)
    let endb_marker = b"ENDB";
    assert!(blend_bytes.windows(4).any(|w| w == endb_marker), "Should contain ENDB block");
    
    // Find DNA1 (DNA structure)
    let dna1_marker = b"DNA1";
    assert!(blend_bytes.windows(4).any(|w| w == dna1_marker), "Should contain DNA1 block");
}

#[test]
fn test_create_scene_blend_uses_startup_ui_prefix() {
    let blend_bytes = create_scene_blend("WithUi", 1, "Data/Objects", &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let glob = blocks.iter().find(|block| block.code == b"GLOB").unwrap();

    assert_eq!(
        u64::from_le_bytes(glob.data[16..24].try_into().unwrap()),
        STARTUP_UI_SCREEN_PTR
    );
    assert!(blocks.iter().any(|block| block.code == b"SN\0\0"));
    assert!(!blocks.iter().any(|block| block.code == b"SR\0\0"));
    assert!(blocks.iter().any(|block| block.code == b"WM\0\0"));
    assert!(blocks.iter().any(|block| block.code == b"WS\0\0"));
}

#[test]
fn test_create_scene_blend_scene_data_is_consecutive() {
    let blend_bytes = create_scene_blend("SceneData", 2, "Data/Objects", &[]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);
    let scene_idx = blocks.iter().position(|block| block.code == b"SC\0\0").unwrap();
    let scene_block = &blocks[scene_idx];
    let mut data_sdnas = Vec::new();

    for block in blocks.iter().skip(scene_idx + 1) {
        if block.code != b"DATA" {
            break;
        }
        data_sdnas.push(block.sdna);
    }

    let tool_settings_ptr = u64::from_le_bytes(scene_block.data[568..576].try_into().unwrap());
    assert_ne!(tool_settings_ptr, 0);
    assert!(blocks.iter().any(|block|
        block.code == b"DATA"
            && block.sdna == SDNA_IDX_TOOL_SETTINGS
            && block.old_ptr == tool_settings_ptr
    ));
    assert!(data_sdnas.contains(&SDNA_IDX_TOOL_SETTINGS));
    assert!(data_sdnas.contains(&SDNA_IDX_VIEW_LAYER));
    assert!(data_sdnas.contains(&SDNA_IDX_BASE));
    assert!(data_sdnas.contains(&SDNA_IDX_LAYER_COLLECTION));
    assert!(data_sdnas.contains(&SDNA_IDX_COLLECTION));
    assert!(data_sdnas.contains(&SDNA_IDX_COLLECTION_CHILD));
}

#[test]
fn test_create_scene_blend_objects_do_not_parent_to_collections() {
    let light = ExtractedLight {
        name: "ParentCheckLight".to_string(),
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        position_blend: [0.0, 0.0, 0.0],
        rotation_blend: [1.0, 0.0, 0.0, 0.0],
        color: [1.0, 1.0, 1.0],
        lamp_type: 0,
        energy_watts: 100.0,
        radius: 10.0,
        cutoff_distance: 10.0,
        radius_source: 10.0,
        spot_size: 0.0,
        spot_blend: 0.0,
        intensity_candela: 5.0,
        temperature_k: 3000.0,
        use_temperature: false,
        gobo_path: None,
        active_state: "default".to_string(),
            states_json: None,
            semantic_light_kind: "point".to_string(),
    };
    let blend_bytes = create_scene_blend("ParentCheck", 2, "Data/Objects", &[light]).unwrap();
    let blocks = parse_blend_blocks(&blend_bytes);

    let object_ptrs = blocks
        .iter()
        .filter(|block| block.code == b"OB\0\0")
        .map(|block| block.old_ptr)
        .collect::<HashSet<_>>();
    let collection_ptrs = blocks
        .iter()
        .filter(|block| block.code == b"GR\0\0")
        .map(|block| block.old_ptr)
        .collect::<HashSet<_>>();

    for block in blocks.iter().filter(|block| block.code == b"OB\0\0") {
        let parent_ptr = u64::from_le_bytes(block.data[496..504].try_into().unwrap());
        assert!(
            parent_ptr == 0 || object_ptrs.contains(&parent_ptr),
            "object block 0x{:x} must parent only to another Object or null",
            block.old_ptr
        );
        assert!(
            !collection_ptrs.contains(&parent_ptr),
            "object block 0x{:x} must not use a Collection pointer as Object.parent",
            block.old_ptr
        );
    }
}

#[test]
fn test_create_scene_blend_relative_paths() {
    let result = create_scene_blend("RelPath", 2, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed");
    
    let blend_bytes = result.unwrap();
    
    // Verify that library paths are embedded
    // The mesh_output_dir "Data/Objects" should appear in library blocks
    let blend_str = String::from_utf8_lossy(&blend_bytes);
    assert!(blend_str.contains("Data/Objects") || blend_bytes.windows(12).any(|w| w == b"Data/Objects"),
        "Should contain relative path for mesh files");
}

#[test]
fn test_create_scene_blend_empty_children() {
    let result = create_scene_blend("NoChildren", 0, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed even with no children");
    
    let blend_bytes = result.unwrap();
    assert!(!blend_bytes.is_empty(), "Output should not be empty");
    assert_eq!(&blend_bytes[0..17], b"BLENDER17-01v0501", "Should have valid header");
}

#[test]
fn test_create_scene_blend_output_not_compressed() {
    let result = create_scene_blend("NoCompress", 1, "Data/Objects", &[]);
    
    assert!(result.is_ok(), "Function should succeed");
    
    let blend_bytes = result.unwrap();
    
    // Verify it's NOT gzip compressed
    // gzip header is 0x1f 0x8b
    assert!(blend_bytes.len() < 2 || blend_bytes[0] != 0x1f,
        "Output should NOT be gzip compressed (Phase 2 handles compression)");
}
