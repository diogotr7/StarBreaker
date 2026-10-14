//! glTF output for the shared model's animation clips (`clip::Clip`): bone
//! tracks drive skeleton nodes by name, morph tracks drive the weights of the
//! meshes that carry those targets, material parameter tracks ride in the
//! animation's extras.

use super::*;
use crate::clip::{BoneTrack, Clip, Keys, MaterialParamTrack, ScalarTrack};
use crate::mesh_rig::{MorphTarget, Skin, MAX_INFLUENCES};
use crate::types::{Mesh, SubMesh};

fn bone(name: &str, parent: Option<u16>, at: [f32; 3]) -> crate::skeleton::Bone {
    crate::skeleton::Bone {
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

/// A triangle skinned to `jaw`, with two morph targets.
fn mesh() -> Mesh {
    let mut mesh = Mesh::empty();
    mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    mesh.indices = vec![0, 1, 2];
    mesh.submeshes = vec![SubMesh {
        material_name: Some("m".into()),
        material_id: 0,
        source_material_id: None,
        first_index: 0,
        num_indices: 3,
        first_vertex: 0,
        num_vertices: 3,
        node_parent_index: 0,
    }];
    let mut w = [0f32; MAX_INFLUENCES];
    w[0] = 1.0;
    mesh.skin = Some(Skin { joint_names: vec!["jaw".into()], joints: vec![[0; MAX_INFLUENCES]; 3], weights: vec![w; 3] });
    mesh.morph_targets = ["bs_000", "bs_001"]
        .iter()
        .map(|n| MorphTarget { name: n.to_string(), position_deltas: vec![[0.0, 0.0, 1.0]; 3], normal_deltas: None })
        .collect();
    mesh
}

fn export(clips: Vec<Clip>) -> Result<Vec<u8>, crate::error::Error> {
    write_glb(
        GlbInput {
            root_mesh: Some(mesh()),
            root_materials: None,
            root_textures: None,
            root_nmc: None,
            root_palette: None,
            skeleton_bones: vec![bone("root", None, [0.0; 3]), bone("jaw", Some(0), [0.0, 0.0, 1.0])],
            children: Vec::new(),
            interiors: crate::pipeline::LoadedInteriors::default(),
            animations: clips,
        },
        &mut GlbLoaders { load_textures: &mut |_, _| None, load_interior_mesh: &mut |_| None },
        &GlbOptions {
            material_mode: crate::pipeline::MaterialMode::None,
            preserve_textureless_decal_primitives: false,
            metadata: GlbMetadata {
                entity_name: Some("head".into()),
                geometry_path: None,
                material_path: None,
                export_options: ExportOptionsMetadata {
                    kind: "Bundled".into(),
                    material_mode: "None".into(),
                    format: "Glb".into(),
                    lod_level: 0,
                    texture_mip: 0,
                    include_attachments: true,
                    include_interior: false,
                },
            },
            fallback_palette: None,
        },
    )
}

fn json_chunk(glb: &[u8]) -> serde_json::Value {
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&glb[20..20 + len]).unwrap()
}

fn jaw_clip() -> Clip {
    let h = std::f32::consts::FRAC_1_SQRT_2;
    Clip {
        name: "open".into(),
        bones: vec![BoneTrack {
            bone: "jaw".into(),
            rotation: Some(Keys { times: vec![0.0, 1.0], values: vec![[1.0, 0.0, 0.0, 0.0], [h, h, 0.0, 0.0]] }),
            translation: Some(Keys { times: vec![0.0, 1.0], values: vec![[0.0, 0.0, 1.0], [0.0, 0.1, 1.0]] }),
        }],
        morphs: vec![ScalarTrack { name: "bs_001".into(), keys: Keys { times: vec![0.0, 0.5, 1.0], values: vec![0.0, 1.0, 0.0] } }],
        material_params: Vec::new(),
    }
}

/// Bone tracks animate the named skeleton node: rotation as glTF `[x, y, z, w]`.
#[test]
fn bone_tracks_drive_bone_nodes() {
    let glb = export(vec![jaw_clip()]).unwrap();
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("valid glTF");
    let anim = doc.animations().next().expect("one animation");
    assert_eq!(anim.name(), Some("open"));
    let rotation = anim
        .channels()
        .find(|c| c.target().property() == gltf::animation::Property::Rotation)
        .expect("rotation channel");
    assert_eq!(rotation.target().node().name(), Some("jaw"));
    let reader = rotation.reader(|b| Some(&buffers[b.index()]));
    assert_eq!(reader.read_inputs().unwrap().collect::<Vec<_>>(), vec![0.0, 1.0]);
    let gltf::animation::util::ReadOutputs::Rotations(r) = reader.read_outputs().unwrap() else { panic!("rotations") };
    let r: Vec<[f32; 4]> = r.into_f32().collect();
    let h = std::f32::consts::FRAC_1_SQRT_2;
    assert_eq!(r[1], [h, 0.0, 0.0, h]);
    assert!(anim.channels().any(|c| c.target().property() == gltf::animation::Property::Translation));
    let json = json_chunk(&glb);
    let node = &json["nodes"][rotation.target().node().index()];
    assert!(node.get("matrix").is_none(), "animated nodes use TRS, not a matrix");
}

/// Morph tracks animate the weights of the mesh carrying those targets; a
/// target without a track holds 0.
#[test]
fn morph_tracks_drive_mesh_weights() {
    let glb = export(vec![jaw_clip()]).unwrap();
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("valid glTF");
    let anim = doc.animations().next().unwrap();
    let weights = anim
        .channels()
        .find(|c| c.target().property() == gltf::animation::Property::MorphTargetWeights)
        .expect("weights channel");
    assert!(weights.target().node().mesh().is_some());
    let reader = weights.reader(|b| Some(&buffers[b.index()]));
    assert_eq!(reader.read_inputs().unwrap().collect::<Vec<_>>(), vec![0.0, 0.5, 1.0]);
    let gltf::animation::util::ReadOutputs::MorphTargetWeights(w) = reader.read_outputs().unwrap() else { panic!("weights") };
    assert_eq!(w.into_f32().collect::<Vec<_>>(), vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
}

/// Material parameter tracks ride in the animation's extras.
#[test]
fn material_params_ride_in_animation_extras() {
    let mut clip = jaw_clip();
    clip.material_params.push(MaterialParamTrack {
        shader: "HumanSkin_V2".into(),
        param: "PI_WrinklesMask".into(),
        info: serde_json::json!({"slots": 2}),
        keys: Keys { times: vec![0.0, 1.0], values: vec![vec![0.0, 0.5], vec![1.0, 0.0]] },
    });
    let json = json_chunk(&export(vec![clip]).unwrap());
    let params = &json["animations"][0]["extras"]["starbreaker_material_params"][0];
    assert_eq!(params["param"], "PI_WrinklesMask");
    assert_eq!(params["times"], serde_json::json!([0.0, 1.0]));
    assert_eq!(params["values"], serde_json::json!([[0.0, 0.5], [1.0, 0.0]]));
    assert_eq!(params["info"]["slots"], 2);
}

/// A clip that moves nothing in the scene is not written (glTF animations
/// need a channel); an invalid clip is an error.
#[test]
fn unresolved_and_invalid_clips() {
    let ghost = Clip {
        name: "ghost".into(),
        bones: vec![BoneTrack {
            bone: "no_such_bone".into(),
            rotation: Some(Keys { times: vec![0.0], values: vec![[1.0, 0.0, 0.0, 0.0]] }),
            translation: None,
        }],
        ..Default::default()
    };
    let json = json_chunk(&export(vec![ghost]).unwrap());
    assert!(json.get("animations").is_none_or(|a| a.as_array().is_some_and(Vec::is_empty)));
    let mut bad = jaw_clip();
    bad.morphs[0].keys.values.pop();
    assert!(export(vec![bad]).is_err());
}
