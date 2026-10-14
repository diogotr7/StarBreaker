//! glTF output for the shared model's rig data and per-vertex attributes:
//! skins (`JOINTS_n` / `WEIGHTS_n`, joints by name, inverse binds, the anchor
//! joint), morph targets, and attribute parity between the flat and NMC paths.

use super::*;
use crate::mesh_rig::{MorphTarget, Skin, MAX_INFLUENCES};
use crate::nmc::{NmcNode, NodeMeshCombo};
use crate::types::{Mesh, SubMesh};

fn submesh(first_index: u32, node: u16) -> SubMesh {
    SubMesh {
        material_name: Some("m".into()),
        material_id: 0,
        source_material_id: None,
        first_index,
        num_indices: 3,
        first_vertex: first_index,
        num_vertices: 3,
        node_parent_index: node,
    }
}

fn quad_mesh() -> Mesh {
    let mut mesh = Mesh::empty();
    mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
    mesh.indices = vec![0, 1, 2, 3, 4, 5];
    mesh.uvs = Some(vec![[0.0, 0.0]; 6]);
    mesh.normals = Some(vec![[0.0, 0.0, 1.0]; 6]);
    mesh.tangents = Some(vec![[1.0, 0.0, 0.0, 1.0]; 6]);
    mesh.colors = Some(vec![[255, 255, 255, 255]; 6]);
    mesh.submeshes = vec![submesh(0, 0)];
    mesh.submeshes[0].num_indices = 6;
    mesh.submeshes[0].num_vertices = 6;
    mesh.model_max = [3.0, 1.0, 0.0];
    mesh.scaling_max = [3.0, 1.0, 0.0];
    mesh
}

/// A bone at `world` (unrotated), its local relative to an unrotated parent at `parent_world`.
fn bone_under(name: &str, parent: Option<u16>, world: [f32; 3], parent_world: [f32; 3]) -> crate::skeleton::Bone {
    let local = [world[0] - parent_world[0], world[1] - parent_world[1], world[2] - parent_world[2]];
    crate::skeleton::Bone {
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

/// A bone at `world` whose parent (if any) sits at the origin.
fn bone(name: &str, parent: Option<u16>, world: [f32; 3]) -> crate::skeleton::Bone {
    bone_under(name, parent, world, [0.0; 3])
}

fn weights(pairs: &[(u16, f32)]) -> ([u16; MAX_INFLUENCES], [f32; MAX_INFLUENCES]) {
    let mut j = [0u16; MAX_INFLUENCES];
    let mut w = [0f32; MAX_INFLUENCES];
    for (slot, &(joint, weight)) in pairs.iter().enumerate() {
        j[slot] = joint;
        w[slot] = weight;
    }
    (j, w)
}

fn skin(names: &[&str], per_vertex: &[&[(u16, f32)]]) -> Skin {
    let (joints, weights): (Vec<_>, Vec<_>) = per_vertex.iter().map(|p| weights(p)).unzip();
    Skin { joint_names: names.iter().map(|s| s.to_string()).collect(), joints, weights }
}

fn opts(material_mode: crate::pipeline::MaterialMode) -> GlbOptions {
    GlbOptions {
        material_mode,
        preserve_textureless_decal_primitives: false,
        metadata: GlbMetadata {
            entity_name: None,
            geometry_path: None,
            material_path: None,
            export_options: ExportOptionsMetadata {
                kind: "Bundled".to_string(),
                material_mode: format!("{material_mode:?}"),
                format: "Glb".to_string(),
                lod_level: 0,
                texture_mip: 0,
                include_attachments: true,
                include_interior: true,
            },
        },
        fallback_palette: None,
    }
}

fn export(mesh: Mesh, nmc: Option<NodeMeshCombo>, bones: Vec<crate::skeleton::Bone>, mode: crate::pipeline::MaterialMode) -> Vec<u8> {
    write_glb(
        GlbInput {
            root_mesh: Some(mesh),
            root_materials: None,
            root_textures: None,
            root_nmc: nmc,
            root_palette: None,
            skeleton_bones: bones,
            children: Vec::new(),
            interiors: crate::pipeline::LoadedInteriors::default(),
            animations: Vec::new(),
        },
        &mut GlbLoaders { load_textures: &mut |_, _| None, load_interior_mesh: &mut |_| None },
        &opts(mode),
    )
    .expect("write_glb")
}

fn nmc_node(name: &str, parent: Option<u16>, tx: f32) -> NmcNode {
    NmcNode {
        name: name.into(),
        parent_index: parent,
        world_to_bone: [[1.0, 0.0, 0.0, -tx], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        bone_to_world: [[1.0, 0.0, 0.0, tx], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        scale: [1.0, 1.0, 1.0],
        geometry_type: 0,
        properties: std::collections::HashMap::new(),
    }
}

fn attribute_names(primitive: &gltf::Primitive) -> Vec<String> {
    let mut names: Vec<String> = primitive.attributes().map(|(s, _)| format!("{s:?}")).collect();
    names.sort();
    names
}

/// Meshes split into an NMC hierarchy carry the same vertex attributes as
/// the flat path (they used to lose TANGENT).
#[test]
fn nmc_hierarchy_primitives_keep_every_vertex_attribute() {
    let mode = crate::pipeline::MaterialMode::Textures;
    let flat = export(quad_mesh(), None, Vec::new(), mode);
    let (flat_doc, _, _) = gltf::import_slice(&flat).expect("flat glb");
    let flat_attrs = attribute_names(&flat_doc.meshes().next().unwrap().primitives().next().unwrap());
    assert!(flat_attrs.contains(&"Tangents".to_string()), "{flat_attrs:?}");

    let mut split = quad_mesh();
    split.submeshes = vec![submesh(0, 0), submesh(3, 1)];
    let nmc = NodeMeshCombo { nodes: vec![nmc_node("Root", None, 0.0), nmc_node("Child", Some(0), 5.0)], material_indices: vec![0, 0] };
    let glb = export(split, Some(nmc), Vec::new(), mode);
    let (doc, _, _) = gltf::import_slice(&glb).expect("nmc glb");
    for mesh in doc.meshes() {
        for primitive in mesh.primitives() {
            assert_eq!(attribute_names(&primitive), flat_attrs, "mesh {:?}", mesh.name());
        }
    }
}

/// A soft skin becomes JOINTS_0 / WEIGHTS_0 and a glTF skin whose joints are
/// the skeleton's bone nodes, matched by name; the inverse bind maps the
/// mesh's bind space into each joint.
#[test]
fn skinned_mesh_emits_a_skin_bound_to_the_named_bones() {
    let mut mesh = quad_mesh();
    let both: &[(u16, f32)] = &[(0, 0.75), (1, 0.25)];
    mesh.skin = Some(skin(&["Root", "Hand"], &[both; 6]));
    let bones = vec![bone("root", None, [0.0, 0.0, 0.0]), bone("hand", Some(0), [5.0, 0.0, 0.0])];
    let glb = export(mesh, None, bones, crate::pipeline::MaterialMode::None);
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("glb");

    let node = doc.nodes().find(|n| n.mesh().is_some()).expect("mesh node");
    let skin = node.skin().expect("mesh node is skinned");
    let joint_names: Vec<_> = skin.joints().map(|j| j.name().unwrap_or("").to_lowercase()).collect();
    assert_eq!(&joint_names[..2], &["root", "hand"]);

    let primitive = node.mesh().unwrap().primitives().next().unwrap();
    let reader = primitive.reader(|b| Some(&buffers[b.index()]));
    let joints: Vec<[u16; 4]> = reader.read_joints(0).expect("JOINTS_0").into_u16().collect();
    let weights: Vec<[f32; 4]> = reader.read_weights(0).expect("WEIGHTS_0").into_f32().collect();
    assert_eq!(joints[0][..2], [0, 1]);
    assert!((weights[0][0] - 0.75).abs() < 1e-6 && (weights[0][1] - 0.25).abs() < 1e-6);
    assert!(reader.read_joints(1).is_none(), "4 influences fit in one set");

    let ibms: Vec<[[f32; 4]; 4]> = skin.reader(|b| Some(&buffers[b.index()])).read_inverse_bind_matrices().unwrap().collect();
    assert_eq!(ibms.len(), skin.joints().count());
    assert!((ibms[1][3][0] + 5.0).abs() < 1e-5, "hand IBM undoes its +5 X: {:?}", ibms[1]);
}

/// More than four influences spill into JOINTS_1 / WEIGHTS_1, strongest first.
#[test]
fn more_than_four_influences_use_a_second_set_strongest_first() {
    let mut mesh = quad_mesh();
    let names = ["a", "b", "c", "d", "e", "f"];
    let six: &[(u16, f32)] = &[(5, 0.05), (4, 0.1), (3, 0.15), (2, 0.2), (1, 0.22), (0, 0.28)];
    mesh.skin = Some(skin(&names, &[six; 6]));
    let bones = names.iter().enumerate().map(|(i, n)| bone(n, None, [i as f32, 0.0, 0.0])).collect();
    let glb = export(mesh, None, bones, crate::pipeline::MaterialMode::None);
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("glb");
    let primitive = doc.meshes().next().unwrap().primitives().next().unwrap();
    let reader = primitive.reader(|b| Some(&buffers[b.index()]));
    let j0: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
    let w0: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
    let j1: Vec<[u16; 4]> = reader.read_joints(1).expect("JOINTS_1").into_u16().collect();
    let w1: Vec<[f32; 4]> = reader.read_weights(1).expect("WEIGHTS_1").into_f32().collect();
    assert_eq!(j0[0], [0, 1, 2, 3], "strongest four first");
    assert_eq!(j1[0][..2], [4, 5]);
    let total: f32 = w0[0].iter().chain(w1[0].iter()).sum();
    assert!((total - 1.0).abs() < 1e-5);
}

/// Vertices without influence bind to an anchor joint, and a joint the
/// skeleton lacks to a placeholder named after it; both are identity children
/// of the mesh node, so the vertices keep their bind pose (glTF skin joints
/// must be unique, so each gets its own node). Every vertex's weights sum to 1.
#[test]
fn unweighted_vertices_and_unknown_joints_bind_to_an_anchor() {
    let mut mesh = quad_mesh();
    let none: &[(u16, f32)] = &[];
    let ghost: &[(u16, f32)] = &[(1, 1.0)];
    let root: &[(u16, f32)] = &[(0, 1.0)];
    mesh.skin = Some(skin(&["root", "ghost"], &[root, root, root, none, none, ghost]));
    let glb = export(mesh, None, vec![bone("root", None, [0.0; 3])], crate::pipeline::MaterialMode::None);
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("glb");
    let node = doc.nodes().find(|n| n.mesh().is_some()).unwrap();
    let skin = node.skin().expect("skin");
    let joints: Vec<_> = skin.joints().collect();
    let primitive = node.mesh().unwrap().primitives().next().unwrap();
    let reader = primitive.reader(|b| Some(&buffers[b.index()]));
    let j: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
    let w: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
    for v in 0..6 {
        assert!((w[v].iter().sum::<f32>() - 1.0).abs() < 1e-6, "vertex {v} weights {:?}", w[v]);
    }
    let anchor = &joints[j[3][0] as usize];
    assert_eq!(anchor.children().count(), 0);
    assert!(node.children().any(|c| c.index() == anchor.index()), "anchor is a child of the mesh node");
    let ghost = &joints[j[5][0] as usize];
    assert_eq!(ghost.name(), Some("ghost"));
    assert!(node.children().any(|c| c.index() == ghost.index()), "placeholder under the mesh node");
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    assert_eq!(ghost.transform().matrix(), identity, "placeholder keeps the bind pose");
    assert_ne!(ghost.index(), anchor.index());
    assert_eq!(joints[j[0][0] as usize].name(), Some("root"));
}

/// Morph targets become glTF targets with zero default weights and their
/// names in `extras.targetNames` (the convention Blender's importer reads).
#[test]
fn morph_targets_emit_named_targets() {
    let mut mesh = quad_mesh();
    mesh.morph_targets = vec![
        MorphTarget { name: "smile".into(), position_deltas: vec![[0.0, 0.1, 0.0]; 6], normal_deltas: None },
        MorphTarget { name: "frown".into(), position_deltas: vec![[0.0, -0.1, 0.0]; 6], normal_deltas: Some(vec![[0.0, 0.0, 0.0]; 6]) },
    ];
    let glb = export(mesh, None, Vec::new(), crate::pipeline::MaterialMode::None);
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("glb");
    let gl_mesh = doc.meshes().next().unwrap();
    assert_eq!(gl_mesh.weights(), Some(&[0.0f32, 0.0][..]));
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let root: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    assert_eq!(root["meshes"][gl_mesh.index()]["extras"]["targetNames"], serde_json::json!(["smile", "frown"]));
    let primitive = gl_mesh.primitives().next().unwrap();
    let reader = primitive.reader(|b| Some(&buffers[b.index()]));
    let targets: Vec<_> = reader.read_morph_targets().collect();
    assert_eq!(targets.len(), 2);
    let smile: Vec<[f32; 3]> = targets[0].0.clone().expect("positions").collect();
    assert_eq!(smile[0], [0.0, 0.1, 0.0]);
    assert!(targets[1].1.is_some(), "frown carries normal deltas");
}

fn node_world(doc: &gltf::Document, target: usize) -> glam::Mat4 {
    let mut parent: Vec<Option<usize>> = vec![None; doc.nodes().count()];
    for node in doc.nodes() {
        for child in node.children() {
            parent[child.index()] = Some(node.index());
        }
    }
    let nodes: Vec<_> = doc.nodes().collect();
    let mut world = glam::Mat4::IDENTITY;
    let mut current = Some(target);
    while let Some(i) = current {
        world = glam::Mat4::from_cols_array_2d(&nodes[i].transform().matrix()) * world;
        current = parent[i];
    }
    world
}

/// Bones become a joint hierarchy (child under parent, local matrices) with
/// the same world transforms as their bind pose.
#[test]
fn skeleton_is_a_joint_hierarchy() {
    let mut mesh = quad_mesh();
    let on_hand: &[(u16, f32)] = &[(2, 1.0)];
    mesh.skin = Some(skin(&["root", "arm", "hand"], &[on_hand; 6]));
    let bones = vec![
        bone_under("root", None, [0.0, 0.0, 0.0], [0.0; 3]),
        bone_under("arm", Some(0), [2.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        bone_under("hand", Some(1), [5.0, 0.0, 1.0], [2.0, 0.0, 0.0]),
    ];
    let glb = export(mesh, None, bones, crate::pipeline::MaterialMode::None);
    let (doc, _, _) = gltf::import_slice(&glb).expect("glb");
    let find = |name: &str| doc.nodes().find(|n| n.name() == Some(name)).expect(name);
    let (root, arm, hand) = (find("root"), find("arm"), find("hand"));
    assert!(root.children().any(|c| c.index() == arm.index()), "arm under root");
    assert!(arm.children().any(|c| c.index() == hand.index()), "hand under arm");
    assert_eq!(hand.transform().decomposed().0, [3.0, 0.0, 1.0], "local translation");
    let root_world = node_world(&doc, root.index());
    let hand_rel = root_world.inverse() * node_world(&doc, hand.index());
    assert!((hand_rel.w_axis.truncate() - glam::Vec3::new(5.0, 0.0, 1.0)).length() < 1e-5);
}

/// A posed skeleton (locals/world moved, bind kept) skins from the bind pose:
/// the inverse bind undoes the bind, so the pose deforms the mesh.
#[test]
fn posed_skeleton_skins_from_the_bind_pose() {
    let mut mesh = quad_mesh();
    let on_hand: &[(u16, f32)] = &[(1, 1.0)];
    mesh.skin = Some(skin(&["root", "hand"], &[on_hand; 6]));
    let mut hand = bone_under("hand", Some(0), [5.0, 0.0, 0.0], [0.0; 3]);
    // posed: hand moved to (5, 2, 0); bind stays at (5, 0, 0)
    hand.local_position = [5.0, 2.0, 0.0];
    hand.world_position = [5.0, 2.0, 0.0];
    let glb = export(mesh, None, vec![bone_under("root", None, [0.0; 3], [0.0; 3]), hand], crate::pipeline::MaterialMode::None);
    let (doc, buffers, _) = gltf::import_slice(&glb).expect("glb");
    let node = doc.nodes().find(|n| n.skin().is_some()).unwrap();
    let skin = node.skin().unwrap();
    let ibms: Vec<[[f32; 4]; 4]> = skin.reader(|b| Some(&buffers[b.index()])).read_inverse_bind_matrices().unwrap().collect();
    let hand_slot = skin.joints().position(|j| j.name() == Some("hand")).unwrap();
    let ibm = glam::Mat4::from_cols_array_2d(&ibms[hand_slot]);
    // skinned position of a vertex at the origin = jointWorld * IBM * v (relative to the mesh node)
    let mesh_world = node_world(&doc, node.index());
    let joint_world = node_world(&doc, skin.joints().nth(hand_slot).unwrap().index());
    let moved = mesh_world.inverse() * joint_world * ibm * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
    assert!((moved.truncate() - glam::Vec3::new(0.0, 2.0, 0.0)).length() < 1e-5, "pose offset applied: {moved:?}");
}
