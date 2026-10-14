//! Shared-model rig data against real game files: a parsed `.skin` carries its
//! soft skin (named joints, normalised weights) in `Mesh::skin`.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";

fn p4k() -> Option<MappedP4k> {
    let path = std::env::var("SC_DATA_P4K")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_P4K_PATH));
    if !path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", path.display());
        return None;
    }
    Some(MappedP4k::open(&path).expect("open p4k"))
}

/// The head `.skin` is soft-skinned (8 influences) to its own bone table.
#[test]
fn head_skin_parses_with_named_soft_skin() {
    let Some(p4k) = p4k() else { return };
    let base = r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\protos_human_male_face_t1_pu_head";
    // IVO meshes keep their streams in the .skinm and the bone table in the .skin
    let skin_file = p4k.read_file(&format!("{base}.skin")).unwrap();
    let skinm = p4k.read_file(&format!("{base}.skinm")).unwrap();
    let mesh = starbreaker_3d::parse_skin_pair(&skinm, Some(&skin_file)).expect("parse head skin");
    let skin = mesh.skin.as_ref().expect("head mesh is soft-skinned");
    assert!(skin.joint_names.len() > 100, "head bone table: {}", skin.joint_names.len());
    assert!(skin.joint_names.iter().any(|n| n.eq_ignore_ascii_case("head")));
    assert_eq!(skin.weights.len(), mesh.positions.len());
    let influenced = skin.weights.iter().filter(|w| w.iter().sum::<f32>() > 0.0).count();
    assert!(influenced * 10 > mesh.positions.len() * 9, "most head vertices are influenced: {influenced}");
    for w in skin.weights.iter().filter(|w| w.iter().sum::<f32>() > 0.0) {
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }
    let multi = skin.weights.iter().filter(|w| w.iter().filter(|&&x| x > 0.0).count() > 4).count();
    assert!(multi > 0, "more than 4 influences survive");
}

/// A worn armour piece is genuinely soft-skinned (blended weights), and the
/// standalone GLB binds every joint to a node of the .skin's own skeleton.
#[test]
fn armour_skin_exports_a_bound_soft_skin() {
    let Some(p4k) = p4k() else { return };
    let base = r"Data\Objects\Characters\Human\male_v7\armor\kap\m_kap_combat_heavy_arms_01";
    let skin_file = p4k.read_file(&format!("{base}.skin")).unwrap();
    let skinm = p4k.read_file(&format!("{base}.skinm")).unwrap();

    let mesh = starbreaker_3d::parse_skin_pair(&skinm, Some(&skin_file)).expect("parse armour");
    let skin = mesh.skin.as_ref().expect("armour is soft-skinned");
    let blended = skin.weights.iter().filter(|w| w.iter().filter(|&&x| x > 0.0).count() > 1).count();
    assert!(blended * 4 > mesh.positions.len(), "blended vertices survive: {blended}/{}", mesh.positions.len());

    let glb = starbreaker_3d::skin_to_glb(&skinm, Some(&skin_file)).expect("glb");
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("valid glTF");
    let node = doc.nodes().find(|n| n.skin().is_some()).expect("a skinned node");
    let gl_skin = node.skin().unwrap();
    // A joint without a skeleton node gets a placeholder named after it, so a
    // resolved skin has exactly one node per joint name.
    let joint_names: Vec<String> = gl_skin.joints().filter_map(|j| j.name().map(str::to_owned)).collect();
    let duplicated: Vec<&String> = joint_names
        .iter()
        .filter(|name| doc.nodes().filter(|n| n.name() == Some(name.as_str())).count() > 1)
        .collect();
    assert!(duplicated.is_empty(), "joints without a skeleton node: {duplicated:?}");
    let bones = starbreaker_3d::skeleton::parse_skeleton(&skin_file).expect("bone table");
    assert_eq!(joint_names.len(), skin.joint_names.len() + 1, "every joint + the anchor");
    assert!(skin.joint_names.iter().all(|j| bones.iter().any(|b| &b.name == j)));
    for primitive in node.mesh().unwrap().primitives() {
        let reader = primitive.reader(|b| Some(&buffers[b.index()]));
        let w0: Vec<[f32; 4]> = reader.read_weights(0).expect("WEIGHTS_0").into_f32().collect();
        let w1: Option<Vec<[f32; 4]>> = reader.read_weights(1).map(|w| w.into_f32().collect());
        for (v, w) in w0.iter().enumerate() {
            let total: f32 = w.iter().sum::<f32>() + w1.as_ref().map_or(0.0, |w1| w1[v].iter().sum());
            assert!((total - 1.0).abs() < 1e-4, "vertex {v}: {total}");
        }
    }
}

/// The .blend writer carries the same skin as the glTF writer: one vertex
/// group per skin joint, named after the joints the GLB binds.
#[test]
fn armour_blend_has_the_glb_skin_joints_as_vertex_groups() {
    let Some(p4k) = p4k() else { return };
    let base = r"Data\Objects\Characters\Human\male_v7\armor\kap\m_kap_combat_heavy_arms_01";
    let skin_file = p4k.read_file(&format!("{base}.skin")).unwrap();
    let skinm = p4k.read_file(&format!("{base}.skinm")).unwrap();

    let glb = starbreaker_3d::skin_to_glb(&skinm, Some(&skin_file)).expect("glb");
    let (doc, _, _) = gltf::import_slice(&glb).expect("valid glTF");
    let gl_skin = doc.skins().next().expect("skin");
    let glb_joints: Vec<String> = gl_skin
        .joints()
        .filter_map(|j| j.name().map(str::to_owned))
        .filter(|n| !n.ends_with("_skin_anchor"))
        .collect();

    let blend = starbreaker_3d::skin_to_blend("m_kap_combat_heavy_arms_01", &skinm, Some(&skin_file)).expect("blend");
    assert_eq!(&blend[..7], b"BLENDER");
    for joint in &glb_joints {
        let mut needle = joint.as_bytes().to_vec();
        needle.push(0);
        assert!(blend.windows(needle.len()).any(|w| w == needle.as_slice()), "vertex group '{joint}' missing from .blend");
    }
}

/// `skeleton::Bone` locals chain to its world fields (bind pose), so a joint
/// hierarchy built from locals reproduces the flat world-matrix bones.
#[test]
fn skeleton_locals_chain_to_world_bind() {
    let Some(p4k) = p4k() else { return };
    for path in [
        r"Data\Objects\Characters\Human\male_v7\export\bhm_skeleton_v7.chr",
        r"Data\Objects\Characters\Human\male_v7\armor\kap\m_kap_combat_heavy_arms_01.skin",
        r"Data\Objects\Spaceships\Ships\AEGS\Gladius\AEGS_Gladius.cga",
        r"Data\Objects\Spaceships\Ships\RSI\Aurora\RSI_Aurora_Mk2.cga",
    ] {
        let Ok(bytes) = p4k.read_file(path) else {
            eprintln!("skip {path}");
            continue;
        };
        let Some(bones) = starbreaker_3d::skeleton::parse_skeleton(&bytes) else {
            eprintln!("{path}: no compiled bones");
            continue;
        };
        let quat = |q: [f32; 4]| glam::Quat::from_xyzw(q[1], q[2], q[3], q[0]);
        let mut worlds: Vec<glam::Mat4> = Vec::with_capacity(bones.len());
        let mut worst = 0.0f32;
        for bone in &bones {
            let local = glam::Mat4::from_rotation_translation(quat(bone.local_rotation), bone.local_position.into());
            let world = match bone.parent_index {
                Some(p) => worlds[p as usize] * local,
                None => local,
            };
            let stored = glam::Mat4::from_rotation_translation(quat(bone.world_rotation), bone.world_position.into());
            worst = worst.max((world - stored).to_cols_array().iter().fold(0.0f32, |m, d| m.max(d.abs())));
            worlds.push(world);
        }
        eprintln!("{path}: {} bones, worst |chained - stored| = {worst}", bones.len());
        assert!(worst < 1e-3, "{path}: locals do not chain to world ({worst})");
    }
}
