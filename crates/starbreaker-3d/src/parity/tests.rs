//! Parity summaries, diff and allowlist; the synthetic fixture (both
//! writers, no p4k) checked against `tests/parity_allowlist.txt`.

use super::*;
use crate::clip::{BoneTrack, Clip, Keys, MaterialParamTrack, ScalarTrack};
use crate::mesh_rig::{MorphTarget, Skin, MAX_INFLUENCES};
use crate::types::{Mesh, SubMesh};

pub(crate) const ALLOWLIST: &str = include_str!("../../tests/parity_allowlist.txt");

#[test]
fn names_drop_lod_and_blender_suffixes() {
    assert_eq!(normalize_name("head_LOD0"), "head");
    assert_eq!(normalize_name("head_LOD0.001"), "head");
    assert_eq!(normalize_name("m_LODGE"), "m_LODGE");
    assert_eq!(normalize_name("teeth"), "teeth");
}

#[test]
fn position_hash_ignores_order_but_not_values() {
    let a = [[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]];
    let b = [[1.0, 2.0, 3.0], [0.0, 0.0, 0.0]];
    assert_eq!(position_hash(a.into_iter()), position_hash(b.into_iter()));
    assert_ne!(position_hash(a.into_iter()), position_hash([[0.0, 0.0, 0.0], [1.0, 2.0, 3.001]].into_iter()));
}

fn summary(vertices: usize) -> Summary {
    let mut s = Summary::default();
    s.meshes.insert("head".into(), MeshSummary { vertices, ..Default::default() });
    s
}

#[test]
fn diff_keys_name_the_fixture_area_and_field() {
    assert!(diff("f", &summary(3), &summary(3)).is_empty());
    assert_eq!(diff("f", &summary(3), &summary(4)), vec!["f/mesh/head/vertices"]);
    let mut lone = Summary::default();
    lone.meshes.insert("other".into(), MeshSummary { vertices: 3, ..Default::default() });
    assert!(diff("f", &summary(3), &lone).is_empty(), "single unpaired meshes pair up");
}

#[test]
fn allowlist_allows_known_gaps_and_flags_stale_ones() {
    let list = Allowlist::parse("# gaps\n*/mesh/*/normals   # custom normals\nf/bone/x/bind\n");
    let diffs = vec!["f/mesh/head/normals".to_string()];
    assert!(list.check(&["f"], &diffs, false).is_err(), "f/bone/x/bind matched nothing: stale");
    let diffs = vec!["f/mesh/head/normals".to_string(), "f/bone/x/bind".to_string()];
    assert!(list.check(&["f"], &diffs, true).is_ok());
    assert!(list.check(&["f"], &["f/mesh/head/vertices".to_string()], false).is_err(), "unexpected");
    assert!(list.check(&["g"], &[], false).is_ok(), "other fixtures' patterns are not checked");
}

/// A skinned, morphing, coloured triangle pair over a two-bone skeleton,
/// with a clip moving the jaw, a morph and a material parameter.
fn synthetic() -> (Mesh, Vec<crate::skeleton::Bone>, Clip) {
    let mut mesh = Mesh::empty();
    mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    mesh.normals = Some(vec![[0.0, 0.0, 1.0]; 4]);
    mesh.uvs = Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
    mesh.colors = Some(vec![[255, 0, 0, 255]; 4]);
    mesh.submeshes = vec![SubMesh {
        material_name: Some("skin".into()),
        material_id: 0,
        source_material_id: None,
        first_index: 0,
        num_indices: 6,
        first_vertex: 0,
        num_vertices: 4,
        node_parent_index: 0,
    }];
    let w = |pairs: &[(u16, f32)]| {
        let (mut j, mut w) = ([0u16; MAX_INFLUENCES], [0f32; MAX_INFLUENCES]);
        for (k, &(joint, weight)) in pairs.iter().enumerate() {
            j[k] = joint;
            w[k] = weight;
        }
        (j, w)
    };
    let (joints, weights) = [w(&[(0, 1.0)]), w(&[(0, 0.5), (1, 0.5)]), w(&[(1, 1.0)]), w(&[(1, 1.0)])].into_iter().unzip();
    mesh.skin = Some(Skin { joint_names: vec!["root".into(), "jaw".into()], joints, weights });
    mesh.morph_targets = vec![MorphTarget { name: "bs_000".into(), position_deltas: vec![[0.0, 0.0, 0.1]; 4], normal_deltas: None }];
    let bone = |name: &str, parent, at: [f32; 3]| crate::skeleton::Bone {
        name: name.into(),
        parent_index: parent,
        object_node_index: None,
        local_position: at,
        local_rotation: [1.0, 0.0, 0.0, 0.0],
        world_position: at,
        world_rotation: [1.0, 0.0, 0.0, 0.0],
        bind_world_position: at,
        bind_world_rotation: [1.0, 0.0, 0.0, 0.0],
    };
    let bones = vec![bone("root", None, [0.0; 3]), bone("jaw", Some(0), [0.0, 0.5, 0.5])];
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let clip = Clip {
        name: "open".into(),
        bones: vec![BoneTrack {
            bone: "jaw".into(),
            rotation: Some(Keys { times: vec![0.0, 1.0], values: vec![[1.0, 0.0, 0.0, 0.0], [h, h, 0.0, 0.0]] }),
            translation: Some(Keys { times: vec![0.0, 1.0], values: vec![[0.0, 0.5, 0.5], [0.0, 0.5, 0.6]] }),
        }],
        morphs: vec![ScalarTrack { name: "bs_000".into(), keys: Keys { times: vec![0.0, 0.5, 1.0], values: vec![0.0, 1.0, 0.0] } }],
        material_params: vec![MaterialParamTrack {
            shader: "HumanSkin_V2".into(),
            param: "PI_WrinklesMask".into(),
            info: serde_json::Value::Null,
            keys: Keys { times: vec![0.0, 1.0], values: vec![vec![0.0], vec![1.0]] },
        }],
    };
    (mesh, bones, clip)
}

fn synthetic_glb(mesh: Mesh, bones: Vec<crate::skeleton::Bone>, clip: Clip) -> Vec<u8> {
    use crate::gltf::{ExportOptionsMetadata, GlbInput, GlbLoaders, GlbMetadata, GlbOptions, write_glb};
    write_glb(
        GlbInput {
            root_mesh: Some(mesh),
            root_materials: None,
            root_textures: None,
            root_nmc: None,
            root_palette: None,
            skeleton_bones: bones,
            children: Vec::new(),
            interiors: Default::default(),
            animations: vec![clip],
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
    .expect("glb")
}

/// Both writers on the same synthetic input agree, up to the allowlisted gaps.
#[test]
fn synthetic_fixture_is_in_parity() {
    let (mesh, bones, clip) = synthetic();
    let glb = synthetic_glb(mesh.clone(), bones.clone(), clip.clone());
    let blend = crate::pipeline::mesh_to_blend("head", &mesh, &None, None, None, &bones, std::slice::from_ref(&clip)).unwrap();
    let g = from_glb(&glb).unwrap();
    let b = from_blend(&blend, "head.blend", &|_| None).unwrap();
    assert_eq!(g.meshes["head"].joints.len(), 2, "{g:#?}");
    assert!(b.clips.contains_key("open"), "{b:#?}");
    let diffs = diff("synthetic", &g, &b);
    eprintln!("synthetic diffs: {diffs:#?}");
    Allowlist::parse(ALLOWLIST).check(&["synthetic"], &diffs, false).unwrap();
}
