//! Unit tests for [`super`]: the character skeleton and skin conversion,
//! and [`super::CharacterRig`] written to GLB (one skeleton for head and
//! parts, morph targets, pose through the skins, part-only bones, and the
//! pipeline sources' streams and materials).

use super::*;
use crate::character::blendshapes::MorphTarget as BlendShape;
use crate::character::head_skin::Bone as HeadBone;

fn head_bone(name: &str, parent: Option<u32>, local: [f32; 3], world: [f32; 3]) -> HeadBone {
    HeadBone {
        name: name.into(),
        name_crc: 0,
        name_crc_lower: 0,
        parent,
        bind_translation: local,
        bind_rotation: [0.0, 0.0, 0.0, 1.0],
        world_translation: world,
        world_rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// Locals carry the pose, world chains them, the bind stays at rest; a
/// rotation's (x, y, z, w) becomes [w, x, y, z].
#[test]
fn skeleton_bones_carry_pose_and_bind() {
    let rest = BindPoseSkeleton {
        bones: vec![head_bone("root", None, [0.0; 3], [0.0; 3]), head_bone("jaw", Some(0), [1.0, 0.0, 0.0], [1.0, 0.0, 0.0])],
    };
    let mut posed = ModifiedSkeleton { bones: rest.bones.clone() };
    let h = std::f32::consts::FRAC_1_SQRT_2;
    posed.bones[0].bind_rotation = [0.0, 0.0, h, h]; // 90 degrees about Z
    let bones = skeleton_bones(&rest, &posed);
    assert_eq!(bones[0].local_rotation, [h, 0.0, 0.0, h]);
    assert_eq!(bones[1].parent_index, Some(0));
    let w = bones[1].world_position;
    assert!((w[0]).abs() < 1e-6 && (w[1] - 1.0).abs() < 1e-6, "jaw follows the posed root: {w:?}");
    assert_eq!(bones[1].bind_world_position, [1.0, 0.0, 0.0], "bind stays at rest");
    assert_eq!(bones[1].bind_world_rotation, [1.0, 0.0, 0.0, 0.0]);
}

fn head_mesh(hi: bool) -> HeadMesh {
    HeadMesh {
        num_vertices: 3,
        num_indices: 3,
        num_submeshes: 1,
        bbox_min: [0.0; 3],
        bbox_max: [1.0; 3],
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uvs: vec![[0.0, 0.0]; 3],
        indices: vec![0, 1, 2],
        joints: vec![[0, 1, 2, 3]; 3],
        weights: vec![[0.4, 0.2, 0.1, 0.1]; 3],
        joints_hi: if hi { vec![[4, 5, 0, 0]; 3] } else { Vec::new() },
        weights_hi: if hi { vec![[0.1, 0.1, 0.0, 0.0]; 3] } else { Vec::new() },
    }
}

/// All eight influences land in the skin, named by the skeleton.
#[test]
fn head_mesh_keeps_eight_named_influences() {
    let names: Vec<String> = (0..6).map(|i| format!("j{i}")).collect();
    let mesh = mesh_from_head(&head_mesh(true), &names);
    let skin = mesh.skin.expect("skin");
    assert_eq!(skin.joint_names, names);
    assert_eq!(skin.joints[0], [0, 1, 2, 3, 4, 5, 0, 0]);
    assert!((skin.weights[0].iter().sum::<f32>() - 1.0).abs() < 1e-6);
    assert_eq!(mesh.submeshes[0].num_indices, 3);
    assert_eq!(mesh.normals.as_ref().map(Vec::len), Some(3));
}

#[test]
fn head_mesh_with_four_influences_and_no_names() {
    let names: Vec<String> = (0..4).map(|i| format!("j{i}")).collect();
    let skin = mesh_from_head(&head_mesh(false), &names).skin.expect("skin");
    assert_eq!(skin.joints[0], [0, 1, 2, 3, 0, 0, 0, 0]);
    assert!(mesh_from_head(&head_mesh(false), &[]).skin.is_none(), "unnamed joints cannot bind");
}

#[test]
fn blendshapes_become_index_named_targets() {
    let bs = BlendShapes {
        num_blendshapes: 2,
        num_vertices: 3,
        targets: vec![BlendShape { positions: vec![[0.0, 1.0, 0.0]; 3] }, BlendShape { positions: vec![[0.0; 3]; 3] }],
    };
    let targets = morph_targets(&bs);
    assert_eq!(targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["bs_000", "bs_001"]);
    assert_eq!(targets[0].position_deltas[0], [0.0, 1.0, 0.0]);
}

#[test]
fn stem_drops_directories_and_extension() {
    assert_eq!(stem(r"Data\Objects\x\protos_eyes.skin"), "protos_eyes");
    assert_eq!(stem("Objects/y/m_hair_31.skin"), "m_hair_31");
}

/// A two-joint rig whose head is skinned to both joints, with one part
/// whose own skeleton lists the joints in the opposite order.
fn rig(blendshapes: Option<BlendShapes>) -> CharacterRig {
    let rest = BindPoseSkeleton {
        bones: vec![head_bone("root", None, [0.0; 3], [0.0; 3]), head_bone("jaw", Some(0), [1.0, 0.0, 0.0], [1.0, 0.0, 0.0])],
    };
    let mut head = head_mesh(false);
    head.joints = vec![[0, 1, 0, 0]; 3];
    head.weights = vec![[0.5, 0.5, 0.0, 0.0]; 3];
    let mut part = head_mesh(false);
    part.positions = vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
    part.joints = vec![[0, 0, 0, 0]; 3]; // its joint 0 = "jaw"
    part.weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];
    CharacterRig {
        name: "test_head".into(),
        head_source: Arc::new(PartSource::from_head_mesh("test_head.skin", &head)),
        head,
        posed: ModifiedSkeleton { bones: rest.bones.clone() },
        rest,
        blendshapes,
        parts: vec![CharacterPart {
            name: "teeth".into(),
            source: Arc::new(PartSource::from_head_mesh("teeth.skin", &part)),
            hidden_zones: Default::default(),
            mesh: part,
            skeleton: BindPoseSkeleton {
                bones: vec![head_bone("jaw", None, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]), head_bone("root", None, [0.0; 3], [0.0; 3])],
            },
        }],
        animations: Vec::new(),
        actor: Vec::new(),
        head_hidden: false,
    }
}

/// A part's bone the character skeleton lacks joins it under its parent
/// (by name), bound at its own bind, posed with that parent.
#[test]
fn rig_bones_take_in_part_only_bones() {
    let mut rig = rig(None);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    rig.posed.bones[1].bind_rotation = [0.0, 0.0, h, h]; // jaw turned 90° about Z
    rig.parts[0].skeleton.bones.push(head_bone("braid", Some(0), [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]));
    let bones = rig.bones();
    assert_eq!(bones.len(), 3, "root, jaw + braid");
    let braid = &bones[2];
    assert_eq!(braid.name, "braid");
    assert_eq!(braid.parent_index, Some(1), "under the character's jaw");
    assert_eq!(braid.bind_world_position, [1.0, 1.0, 0.0]);
    let w = braid.world_position;
    assert!((w[0] - 0.0).abs() < 1e-5 && (w[1] - 0.0).abs() < 1e-5, "follows the turned jaw: {w:?}");
}

/// A shared-model bone at its bind (world = bind world).
fn actor_bone(name: &str, parent: Option<u16>, local: [f32; 3], world: [f32; 3]) -> Bone {
    Bone {
        name: name.into(),
        parent_index: parent,
        object_node_index: None,
        local_position: local,
        local_rotation: [1.0, 0.0, 0.0, 0.0],
        world_position: world,
        world_rotation: [1.0, 0.0, 0.0, 0.0],
        bind_world_position: world,
        bind_world_rotation: [1.0, 0.0, 0.0, 0.0],
    }
}

/// With an actor (body) skeleton the rig is built on it: the actor's bones
/// and hierarchy come first, the head rig's own bones hang under their
/// parents by name, and the head rig poses a shared bone only where both
/// skeletons hang it under the same parent.
#[test]
fn actor_skeleton_is_the_rig_base() {
    let mut rig = rig(None); // head rig: root, jaw(root) at (1, 0, 0)
    let lip = head_bone("lip", Some(1), [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]);
    rig.rest.bones.push(lip.clone());
    rig.posed.bones.push(lip);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    rig.posed.bones[0].bind_translation = [0.0, 0.0, 0.5]; // root raised: same parent (none) in both
    rig.posed.bones[1].bind_rotation = [0.0, 0.0, h, h]; // jaw turned: under hips in the actor, so not applied
    rig.actor = vec![
        actor_bone("root", None, [0.0; 3], [0.0; 3]),
        actor_bone("hips", Some(0), [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
        actor_bone("jaw", Some(1), [1.0, 0.0, -1.0], [1.0, 0.0, 0.0]),
    ];
    let bones = rig.bones();
    let names: Vec<&str> = bones.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["root", "hips", "jaw", "lip"]);
    assert_eq!(bones[2].parent_index, Some(1), "the actor's hierarchy");
    assert_eq!(bones[3].parent_index, Some(2), "a head-only bone under its parent by name");
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5);
    assert!(near(bones[0].world_position, [0.0, 0.0, 0.5]), "root posed: {:?}", bones[0].world_position);
    assert_eq!(bones[2].local_rotation, [1.0, 0.0, 0.0, 0.0], "jaw keeps the actor's local");
    assert!(near(bones[2].world_position, [1.0, 0.0, 0.5]), "jaw follows root: {:?}", bones[2].world_position);
    assert!(near(bones[3].world_position, [1.0, 1.0, 0.5]), "lip follows jaw: {:?}", bones[3].world_position);
    assert_eq!(bones[2].bind_world_position, [1.0, 0.0, 0.0]);
    assert_eq!(bones[3].bind_world_position, [1.0, 1.0, 0.0]);
}

fn body_clip(bones: &[&str]) -> crate::animation::AnimationClip {
    use crate::animation::{BoneChannel, Keyframe, bone_name_hash};
    crate::animation::AnimationClip {
        name: "animations/characters/human/male_v7/idle_01.caf".into(),
        fps: 30.0,
        channels: bones
            .iter()
            .map(|b| BoneChannel {
                bone_hash: bone_name_hash(b),
                rotations: vec![Keyframe { time: 0.0, value: [0.0, 0.0, 0.0, 1.0] }, Keyframe { time: 30.0, value: [0.0, 0.0, 1.0, 0.0] }],
                positions: Vec::new(),
                rot_format_flags: 0,
                pos_format_flags: 0,
            })
            .collect(),
        start_rotation: None,
        start_position: None,
    }
}

/// [`rig`] with a face clip: a jaw track and one morph.
fn rig_with_face() -> CharacterRig {
    use crate::clip::{BoneTrack, Clip, Keys, ScalarTrack};
    let mut rig = rig(None);
    let one = |v| Keys { times: vec![0.0], values: vec![v] };
    rig.animations.push(Clip {
        name: "expression".into(),
        bones: vec![BoneTrack { bone: "jaw".into(), rotation: Some(one([1.0, 0.0, 0.0, 0.0])), translation: None }],
        morphs: vec![ScalarTrack { name: "bs_000".into(), keys: Keys { times: vec![0.0], values: vec![1.0] } }],
        material_params: Vec::new(),
    });
    rig
}

/// A body clip plays over the rig's skeleton, named after its file; a face
/// clip already on the rig joins it as one clip (its morphs and the bones
/// the body clip leaves alone).
#[test]
fn body_clip_combines_with_the_face_clip() {
    let mut rig = rig(None);
    rig.add_body_clip(&body_clip(&["root", "ghost"]));
    assert_eq!(rig.animations.len(), 1);
    assert_eq!(rig.animations[0].name, "idle_01");
    assert_eq!(rig.animations[0].bones.len(), 1, "bones outside the rig drop");
    assert_eq!(rig.animations[0].bones[0].rotation.as_ref().unwrap().times, vec![0.0, 1.0], "frames at 30 fps -> seconds");

    let mut rig = rig_with_face();
    rig.add_body_clip(&body_clip(&["root"]));
    assert_eq!(rig.animations.len(), 1, "one combined clip");
    let clip = &rig.animations[0];
    assert_eq!(clip.name, "idle_01");
    let names: Vec<&str> = clip.bones.iter().map(|b| b.bone.as_str()).collect();
    assert_eq!(names, ["root", "jaw"]);
    assert_eq!(clip.morphs.len(), 1, "the face clip's morphs");

}

/// [`rig`] whose part has two one-triangle submeshes in clothing zones 7
/// and 8.
fn zoned_rig() -> CharacterRig {
    let mut rig = rig(None);
    let part = &mut rig.parts[0];
    part.mesh.positions.extend([[2.0, 0.0, 1.0], [3.0, 0.0, 1.0], [2.0, 1.0, 1.0]]);
    part.mesh.normals.extend([[0.0, 0.0, 1.0]; 3]);
    part.mesh.uvs.extend([[0.0, 0.0]; 3]);
    part.mesh.indices.extend([3, 4, 5]);
    part.mesh.joints.extend([[0, 0, 0, 0]; 3]);
    part.mesh.weights.extend([[1.0, 0.0, 0.0, 0.0]; 3]);
    let mut source = PartSource::from_head_mesh("teeth.skin", &part.mesh);
    let first = source.mesh.submeshes[0].clone();
    source.mesh.submeshes = vec![
        crate::types::SubMesh { num_indices: 3, num_vertices: 3, ..first.clone() },
        crate::types::SubMesh { first_index: 3, num_indices: 3, first_vertex: 3, num_vertices: 3, ..first },
    ];
    source.zones = vec![Some(7), Some(8)];
    part.source = Arc::new(source);
    rig
}

/// Submeshes in covered zones are not written; the rest keep their
/// vertices (compacted) and skin.
#[test]
fn covered_zones_are_not_written() {
    let mut rig = zoned_rig();
    rig.parts[0].hidden_zones = [7].into();
    let payloads = rig.part_payloads("root", |_| String::new()).unwrap();
    assert_eq!(payloads.len(), 1);
    let mesh = &payloads[0].mesh;
    assert_eq!(mesh.submeshes.len(), 1);
    assert_eq!(mesh.positions.len(), 3, "only the visible triangle's vertices");
    assert_eq!(mesh.positions[0], [2.0, 0.0, 1.0]);
    assert_eq!(mesh.skin.as_ref().map(|s| s.joints.len()), Some(3));
}

/// A part with every submesh covered (a body under an undersuit) is not
/// written at all; its bones stay in the skeleton.
#[test]
fn fully_covered_parts_are_not_written() {
    let mut rig = zoned_rig();
    rig.parts[0].hidden_zones = [7, 8].into();
    assert!(rig.part_payloads("root", |_| String::new()).unwrap().is_empty());
    assert_eq!(rig.bones().len(), 2);
}

fn json_chunk(glb: &[u8]) -> serde_json::Value {
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&glb[20..20 + len]).unwrap()
}

/// The head and every part are skinned to the one character skeleton:
/// each part's joints resolve by name to the same bone nodes, never to
/// placeholders.
#[test]
fn rig_glb_binds_head_and_parts_to_one_skeleton() {
    let glb = rig(None).to_glb().expect("glb");
    let (doc, _, _) = gltf::import_slice(&glb).expect("valid glTF");
    assert_eq!(doc.skins().count(), 2, "head + part");
    let bone_nodes = |name: &str| doc.nodes().filter(|n| n.name() == Some(name)).count();
    assert_eq!(bone_nodes("jaw"), 1, "one jaw node, no placeholder");
    assert_eq!(bone_nodes("root"), 1);
    let head = doc.nodes().find(|n| n.name() == Some("test_head")).expect("named head node");
    let teeth = doc.nodes().find(|n| n.name() == Some("teeth")).expect("part node");
    assert!(head.children().any(|c| c.index() == teeth.index()), "part hangs under the head");
    let part_skin = teeth.skin().expect("part is skinned");
    assert_eq!(part_skin.joints().next().and_then(|j| j.name().map(str::to_owned)).as_deref(), Some("jaw"));
}

/// Blendshapes become the head's named morph targets.
#[test]
fn rig_glb_carries_named_morph_targets() {
    let bs = BlendShapes { num_blendshapes: 1, num_vertices: 3, targets: vec![BlendShape { positions: vec![[0.0, 0.0, 1.0]; 3] }] };
    let json = json_chunk(&rig(Some(bs)).to_glb().expect("glb"));
    let names: Vec<_> = json["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["extras"]["targetNames"].as_array())
        .collect();
    assert_eq!(names, vec![&vec![serde_json::json!("bs_000")]]);
}

/// Blendshapes that do not match the head's vertex count are an error,
/// not a corrupt file.
#[test]
fn rig_glb_rejects_mismatched_blendshapes() {
    let bs = BlendShapes { num_blendshapes: 1, num_vertices: 4, targets: vec![BlendShape { positions: vec![[0.0; 3]; 4] }] };
    assert!(matches!(rig(Some(bs)).to_glb(), Err(CharacterError::GlbEmit(_))));
}

/// A posed jaw moves the skinned part through the skin, from the rest bind.
#[test]
fn rig_glb_poses_parts_through_the_skin() {
    let mut posed = rig(None);
    posed.posed.bones[1].bind_translation = [1.0, 0.0, -0.5];
    let glb = posed.to_glb().expect("glb");
    let (doc, _, _) = gltf::import_slice(&glb).expect("valid glTF");
    let jaw = doc.nodes().find(|n| n.name() == Some("jaw")).unwrap();
    let [_, _, _, t] = jaw.transform().matrix();
    assert!((t[2] + 0.5).abs() < 1e-6, "jaw node carries the pose: {t:?}");
}

/// A material file with one plain submaterial per name.
fn materials(names: &[&str]) -> crate::mtl::MtlFile {
    let sub = |name: &str| crate::mtl::SubMaterial {
        name: name.into(),
        shader: "Illum".into(),
        diffuse: [1.0, 1.0, 1.0],
        opacity: 1.0,
        alpha_test: 0.0,
        string_gen_mask: String::new(),
        is_nodraw: false,
        specular: [0.04, 0.04, 0.04],
        shininess: 128.0,
        emissive: [0.0, 0.0, 0.0],
        glow: 0.0,
        surface_type: String::new(),
        diffuse_tex: None,
        normal_tex: None,
        layers: Vec::new(),
        palette_tint: 0,
        texture_slots: Vec::new(),
        public_params: Vec::new(),
        authored_attributes: Vec::new(),
        authored_textures: Vec::new(),
        authored_child_blocks: Vec::new(),
    };
    crate::mtl::MtlFile {
        materials: names.iter().map(|n| sub(n)).collect(),
        source_path: Some("Data/Objects/test.mtl".into()),
        paint_override: None,
        material_set: Default::default(),
    }
}

/// The rig of [`rig`] whose head and part sources are pipeline-like:
/// the part has vertex colours, a second UV set and two submeshes over
/// two submaterials; the head one submaterial.
fn pipeline_rig() -> CharacterRig {
    let mut rig = rig(None);
    let mut head = PartSource::from_head_mesh("test_head.skin", &rig.head);
    head.materials = Some(materials(&["skin"]));
    let mut part = PartSource::from_head_mesh("teeth.skin", &rig.parts[0].mesh);
    part.mesh.colors = Some(vec![[255, 128, 0, 255]; 3]);
    part.mesh.secondary_uvs = Some(vec![[0.5, 0.5]; 3]);
    let mut second = part.mesh.submeshes[0].clone();
    second.material_id = 1;
    part.mesh.submeshes.push(second);
    part.materials = Some(materials(&["enamel", "gums"]));
    rig.head_source = Arc::new(head);
    rig.parts[0].source = Arc::new(part);
    rig
}

/// The GLB meshes are the pipeline sources reshaped: a part keeps its
/// colours, second UV set and per-material primitives, named after its
/// `.mtl` submaterials, and takes the rig's shape and skin.
#[test]
fn rig_glb_writes_the_pipeline_streams_and_materials() {
    let rig = pipeline_rig();
    let glb = rig.to_glb().expect("glb");
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("valid glTF");
    let teeth = doc.nodes().find(|n| n.name() == Some("teeth")).expect("part node");
    let mesh = teeth.mesh().expect("part mesh");
    let prims: Vec<_> = mesh.primitives().collect();
    assert_eq!(prims.len(), 2, "one primitive per submesh");
    for prim in &prims {
        assert!(prim.get(&gltf::Semantic::Colors(0)).is_some(), "vertex colours");
        assert!(prim.get(&gltf::Semantic::TexCoords(1)).is_some(), "second UV set");
    }
    let names: Vec<String> = prims.iter().filter_map(|p| p.material().name().map(str::to_owned)).collect();
    assert!(names.iter().any(|n| n.contains("enamel")) && names.iter().any(|n| n.contains("gums")), "{names:?}");
    let positions: Vec<[f32; 3]> =
        prims[0].reader(|b| Some(&buffers[b.index()])).read_positions().expect("positions").collect();
    assert_eq!(positions, rig.parts[0].mesh.positions, "the rig's shape");
    assert!(teeth.skin().is_some(), "the rig's skin");
    let head = doc.nodes().find(|n| n.name() == Some("test_head")).and_then(|n| n.mesh()).expect("head mesh");
    let head_material = head.primitives().next().and_then(|p| p.material().name().map(str::to_owned));
    assert!(head_material.is_some_and(|n| n.contains("skin")), "head .mtl material");
}

/// A source whose vertex stream does not match the rig's shape is an
/// error, not a corrupt file.
#[test]
fn rig_glb_rejects_a_mismatched_part_source() {
    let mut rig = rig(None);
    let mut short = rig.parts[0].mesh.clone();
    short.positions.pop();
    rig.parts[0].source = Arc::new(PartSource::from_head_mesh("teeth.skin", &short));
    assert!(matches!(rig.to_glb(), Err(CharacterError::Customization(_))));
}

/// Package payloads carry the part's `.mtl` and the geometry path the
/// caller derives from the source.
#[test]
fn part_payloads_carry_materials_and_geometry_paths() {
    let rig = pipeline_rig();
    let payloads = rig.part_payloads("character", |s| format!("Characters/x/{}", s.skin_path)).unwrap();
    assert_eq!(payloads.len(), 1);
    let p = &payloads[0];
    assert_eq!(p.entity_name, "teeth");
    assert_eq!(p.parent_entity_name, "character");
    assert_eq!(p.geometry_path, "Characters/x/teeth.skin");
    assert_eq!(p.material_path, "Data/Objects/test.mtl");
    assert_eq!(p.mesh.submeshes.len(), 2);
    assert_eq!(p.mesh.positions, rig.parts[0].mesh.positions);
}
