//! Rig parity tests for the `.blend` writers: a skinned mesh with a skeleton
//! gets an armature object and an Armature modifier bound to it; morph
//! targets become shape keys.

use super::tests_5a_scene_blend::{BlendBlock, cstr_at, mesh_block_by_name, object_block_by_name, parse_blend_blocks};
use super::*;
use crate::mesh_rig::{MorphTarget, Skin};
use crate::skeleton::Bone;
use crate::types::{Mesh, SubMesh};
use starbreaker_blend::armature::{OB_ARMATURE, SDNA_IDX_ARMATURE_MODIFIER, SDNA_IDX_BONE};
use starbreaker_blend::shape_keys::{MESH_KEY_OFFSET, SDNA_IDX_KEY_BLOCK};

fn ptr(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
}

fn bone(name: &str, parent: Option<u16>, at: [f32; 3]) -> Bone {
    Bone {
        name: name.into(),
        parent_index: parent,
        object_node_index: None,
        local_position: at,
        local_rotation: [1.0, 0.0, 0.0, 0.0],
        world_position: at,
        world_rotation: [1.0, 0.0, 0.0, 0.0],
        bind_world_position: at,
        bind_world_rotation: [1.0, 0.0, 0.0, 0.0],
    }
}

fn bones() -> Vec<Bone> {
    vec![bone("root", None, [0.0; 3]), bone("jaw", Some(0), [0.0, 0.0, 1.0])]
}

fn triangle(skinned: bool, morphs: bool) -> Mesh {
    let mut mesh = Mesh::empty();
    mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    mesh.indices = vec![0, 1, 2];
    mesh.submeshes = vec![SubMesh {
        material_name: Some("mat".into()),
        material_id: 0,
        source_material_id: None,
        first_index: 0,
        num_indices: 3,
        first_vertex: 0,
        num_vertices: 3,
        node_parent_index: 0,
    }];
    if skinned {
        mesh.skin = Some(Skin {
            joint_names: vec!["jaw".into(), "root".into()],
            joints: vec![[0; 8]; 3],
            weights: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 3],
        });
    }
    if morphs {
        mesh.morph_targets = vec![MorphTarget { name: "bs_000".into(), position_deltas: vec![[0.0, 0.0, 0.5]; 3], normal_deltas: None }];
    }
    mesh
}

fn armature_objects<'a>(blocks: &'a [BlendBlock<'a>]) -> Vec<&'a BlendBlock<'a>> {
    blocks
        .iter()
        .filter(|b| b.code == b"OB\0\0" && i16::from_le_bytes(b.data[416..418].try_into().unwrap()) == OB_ARMATURE)
        .collect()
}

/// The skinned mesh's first modifier is an Armature modifier bound to the
/// file's armature object, whose armature carries one bone per skeleton bone.
#[test]
fn skinned_flat_mesh_binds_an_armature_modifier() {
    let bytes = mesh_to_blend("head", &triangle(true, false), &None, None, None, &bones(), &[]).unwrap();
    let blocks = parse_blend_blocks(&bytes);
    let rigs = armature_objects(&blocks);
    assert_eq!(rigs.len(), 1, "one armature object");
    let rig = rigs[0];
    let mesh_object = object_block_by_name(&blocks, "head");
    let first_modifier = ptr(mesh_object.data, 656);
    let modifier = blocks.iter().find(|b| b.old_ptr == first_modifier).expect("first modifier block");
    assert_eq!(modifier.sdna, SDNA_IDX_ARMATURE_MODIFIER, "the Armature modifier comes first");
    assert_eq!(ptr(modifier.data, 128), rig.old_ptr, "bound to the armature object");
    assert_ne!(ptr(modifier.data, 0), 0, "the existing modifiers follow");
    let bone_names: Vec<&str> = blocks.iter().filter(|b| b.sdna == SDNA_IDX_BONE).map(|b| cstr_at(b.data, 64, 64)).collect();
    assert_eq!(bone_names, vec!["root", "jaw"]);
}

/// Bones without a skinned mesh still become an armature (the glTF writer
/// emits them as nodes too), but nothing gets an Armature modifier.
#[test]
fn unskinned_mesh_gets_no_armature_modifier() {
    let bytes = mesh_to_blend("prop", &triangle(false, false), &None, None, None, &bones(), &[]).unwrap();
    let blocks = parse_blend_blocks(&bytes);
    assert_eq!(armature_objects(&blocks).len(), 1);
    assert!(!blocks.iter().any(|b| b.sdna == SDNA_IDX_ARMATURE_MODIFIER));
    let plain = mesh_to_blend("plain", &triangle(false, false), &None, None, None, &[], &[]).unwrap();
    assert!(armature_objects(&parse_blend_blocks(&plain)).is_empty(), "no skeleton, no armature");
}

/// Morph targets become relative shape keys: Basis + one block per target,
/// referenced from `Mesh.key`.
#[test]
fn morph_targets_become_shape_keys() {
    let bytes = mesh_to_blend("head", &triangle(false, true), &None, None, None, &[], &[]).unwrap();
    let blocks = parse_blend_blocks(&bytes);
    let mesh = mesh_block_by_name(&blocks, "head");
    let key = blocks.iter().find(|b| b.code == b"KE\0\0").expect("Key block");
    assert_eq!(ptr(mesh.data, MESH_KEY_OFFSET), key.old_ptr);
    assert_eq!(ptr(key.data, 480), mesh.old_ptr, "Key.from");
    let names: Vec<&str> = blocks.iter().filter(|b| b.sdna == SDNA_IDX_KEY_BLOCK).map(|b| cstr_at(b.data, 48, 64)).collect();
    assert_eq!(names, vec!["Basis", "bs_000"]);
}

fn scene_instance(id: usize, name: &str, parent: Option<&str>, asset: &str) -> LinkedMeshInstance {
    LinkedMeshInstance {
        scene_instance_id: id,
        ui_bindings: Vec::new(),
        entity_name: name.to_string(),
        parent_entity_name: parent.map(str::to_string),
        parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
        parent_empty_loc: [0.0; 3],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0; 3],
        is_interior: false,
        source_object_name: name.to_string(),
        name: name.to_string(),
        mesh_name: name.to_string(),
        material_names: Vec::new(),
        material_sidecar: None,
        palette_id: None,
        source_ancestors: Vec::new(),
        source_nodes: Vec::new(),
        source_loc: [0.0; 3],
        source_quat: [1.0, 0.0, 0.0, 0.0],
        source_scale: [1.0; 3],
        source_parent_name: None,
        parent_node_name: None,
        blend_path: format!("//../../{asset}"),
        mesh_asset: asset.to_string(),
        position: [0.0; 3],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0; 3],
        hidden: false,
    }
}

/// scene.blend gets one armature for the package skeleton; every skinned
/// mesh object (root and children) binds to it, unskinned ones do not.
#[test]
fn scene_blend_binds_skinned_objects_to_one_armature() {
    let instances = [
        scene_instance(0, "head", None, "Data/Objects/head.blend"),
        scene_instance(1, "teeth", Some("head"), "Data/Objects/teeth.blend"),
        scene_instance(2, "prop", Some("head"), "Data/Objects/prop.blend"),
    ];
    let skinned = HashSet::from(["data/objects/head.blend".to_string(), "data/objects/teeth.blend".to_string()]);
    let rig = SceneRig { bones: &bones(), skinned_assets: &skinned, clips: &[] };
    let bytes = create_scene_blend_package_with_instances_and_decal_offsets(
        "Test",
        "character",
        &instances,
        &[],
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        Some(&rig),
    )
    .unwrap();
    let blocks = parse_blend_blocks(&bytes);
    let rigs = armature_objects(&blocks);
    assert_eq!(rigs.len(), 1);
    let head = object_block_by_name(&blocks, "head");
    assert_eq!(ptr(rigs[0].data, 496), ptr(head.data, 496), "armature shares the root mesh's parent");
    let first_modifier_sdna = |name: &str| {
        let first = ptr(object_block_by_name(&blocks, name).data, 656);
        let modifier = blocks.iter().find(|b| b.old_ptr == first).unwrap();
        (modifier.sdna, ptr(modifier.data, 128))
    };
    assert_eq!(first_modifier_sdna("head"), (SDNA_IDX_ARMATURE_MODIFIER, rigs[0].old_ptr));
    assert_eq!(first_modifier_sdna("teeth"), (SDNA_IDX_ARMATURE_MODIFIER, rigs[0].old_ptr));
    assert_ne!(first_modifier_sdna("prop").0, SDNA_IDX_ARMATURE_MODIFIER);
    let collected = blocks
        .iter()
        .filter(|b| b.sdna == starbreaker_blend::SDNA_IDX_COLLECTION_OBJECT)
        .any(|b| ptr(b.data, 16) == rigs[0].old_ptr);
    assert!(collected, "the armature is in a collection");
}

fn clip(name: &str) -> crate::clip::Clip {
    use crate::clip::{BoneTrack, Keys, ScalarTrack};
    crate::clip::Clip {
        name: name.into(),
        bones: vec![BoneTrack {
            bone: "jaw".into(),
            rotation: Some(Keys { times: vec![0.0, 2.0], values: vec![[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]] }),
            translation: None,
        }],
        morphs: vec![ScalarTrack { name: "bs_000".into(), keys: Keys { times: vec![0.0, 1.0], values: vec![0.0, 1.0] } }],
        material_params: Vec::new(),
    }
}

/// Clips become actions: bone tracks on the armature object, morph tracks
/// on the mesh's shape-key Key, each assigned through `AnimData`; extra
/// clips are kept with a fake user; the scene plays to the last key.
#[test]
fn clips_become_assigned_actions() {
    use starbreaker_blend::action::{ID_ADT_OFFSET, SDNA_IDX_ANIM_DATA};
    let bytes = mesh_to_blend("head", &triangle(true, true), &None, None, None, &bones(), &[clip("smile"), clip("blink")]).unwrap();
    let blocks = parse_blend_blocks(&bytes);
    let actions: Vec<&BlendBlock> = blocks.iter().filter(|b| b.code == b"AC\0\0").collect();
    let names: Vec<&str> = actions.iter().map(|a| cstr_at(a.data, 42, 64)).collect();
    assert_eq!(names, vec!["smile", "blink", "smile_shapes", "blink_shapes"]);
    let rig = armature_objects(&blocks)[0];
    let adt = blocks.iter().find(|b| b.old_ptr == ptr(rig.data, ID_ADT_OFFSET)).expect("armature AnimData");
    assert_eq!(adt.sdna, SDNA_IDX_ANIM_DATA);
    assert_eq!(ptr(adt.data, 0), actions[0].old_ptr, "first clip assigned");
    let key = blocks.iter().find(|b| b.code == b"KE\0\0").unwrap();
    let key_adt = blocks.iter().find(|b| b.old_ptr == ptr(key.data, ID_ADT_OFFSET)).expect("key AnimData");
    assert_eq!(ptr(key_adt.data, 0), actions[2].old_ptr);
    let fake_user = |a: &BlendBlock| i16::from_le_bytes(a.data[298..300].try_into().unwrap()) & (1 << 9) != 0;
    assert!(!fake_user(actions[0]) && fake_user(actions[1]) && fake_user(actions[3]));
    let scene = blocks.iter().find(|b| b.code == b"SC\0\0").unwrap();
    assert_eq!(i32::from_le_bytes(scene.data[1040..1044].try_into().unwrap()), 49, "2 s at 24 fps from frame 1");
}
