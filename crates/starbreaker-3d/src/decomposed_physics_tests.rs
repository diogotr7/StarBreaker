//! `attach_physics` on a hand-built mesh-asset GLB: per-node records, unreferenced trimesh meshes, whole-object
//! records under an inverse-root node, node/NMC agreement.
use super::*;
use crate::nmc::NmcNode;

fn nmc(names: &[&str]) -> NodeMeshCombo {
    NodeMeshCombo {
        nodes: names.iter().map(|n| NmcNode {
            name: n.to_string(), parent_index: None, world_to_bone: [[0.0; 4]; 3], bone_to_world: [[0.0; 4]; 3],
            scale: [1.0; 3], geometry_type: 0, properties: Default::default(),
        }).collect(),
        material_indices: Vec::new(),
    }
}
/// Nodes A, B (NMC), then the wrapper; A has a translation matrix (column-major, t = (1, 2, 3)).
fn glb() -> Vec<u8> {
    let root = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [2]}],
        "nodes": [
            {"name": "A", "matrix": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 2.0, 3.0, 1.0], "children": [1]},
            {"name": "B"},
            {"name": "CryEngine_Z_up", "children": [0]}
        ],
        "buffers": [{"byteLength": 4}], "bufferViews": [], "accessors": [], "meshes": []
    });
    join_glb(&root, &[1, 2, 3, 4]).unwrap()
}
fn tri() -> PhysicsGeometry {
    PhysicsGeometry::Trimesh { vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], triangles: vec![[0, 1, 2]], material_ids: vec![0] }
}

#[test]
fn records_land_on_their_nodes_in_their_frames() {
    let entries = vec![
        PhysicsEntry { node: Some(1), part: 0, geometry: tri() },
        PhysicsEntry { node: Some(0), part: 1, geometry: PhysicsGeometry::Sphere { center: [0.0, 0.0, 1.0], radius: 0.5 } },
        PhysicsEntry { node: None, part: 1, geometry: PhysicsGeometry::Box { axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], center: [0.0; 3], half_size: [1.0; 3] } },
        PhysicsEntry { node: Some(0), part: 1, geometry: PhysicsGeometry::Sdf { bytes: 12 } },
    ];
    let out = attach_physics(glb(), &entries, None, Some(&nmc(&["A", "B"]))).unwrap();
    let (root, bin) = split_glb(&out).unwrap();
    assert_eq!(&bin[..4], &[1, 2, 3, 4], "existing binary kept first");
    assert_eq!(root["buffers"][0]["byteLength"].as_u64().unwrap() as usize, bin.len());
    let b = &root["nodes"][1]["extras"]["sc_physics"][0];
    assert_eq!((b["kind"].as_str(), b["part"].as_u64()), (Some("trimesh"), Some(0)));
    let mesh = &root["meshes"][b["mesh"].as_u64().unwrap() as usize];
    let pos = &root["accessors"][mesh["primitives"][0]["attributes"]["POSITION"].as_u64().unwrap() as usize];
    assert_eq!((pos["count"].as_u64(), pos["max"][0].as_f64()), (Some(3), Some(1.0)));
    assert!(root["nodes"].as_array().unwrap().iter().all(|n| n["mesh"].as_u64() != b["mesh"].as_u64()), "unreferenced");
    let a = root["nodes"][0]["extras"]["sc_physics"].as_array().unwrap();
    assert_eq!(a.iter().map(|r| r["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["sphere", "sdf"]);
    // Whole object: a child of the content root A that undoes A's translation; the wrapper keeps one child.
    let object = root["nodes"].as_array().unwrap().iter().position(|n| n["name"] == "$physics_object").unwrap();
    assert!(root["nodes"][0]["children"].as_array().unwrap().contains(&json!(object)));
    assert_eq!(root["nodes"][2]["children"], json!([0]));
    let m = root["nodes"][object]["matrix"].as_array().unwrap();
    assert_eq!((m[12].as_f64(), m[13].as_f64(), m[14].as_f64()), (Some(-1.0), Some(-2.0), Some(-3.0)));
    assert_eq!(root["nodes"][object]["extras"]["sc_physics"][0]["kind"], "box");
}

#[test]
fn rejects_nodes_that_do_not_match_the_nmc() {
    let entries = vec![PhysicsEntry { node: Some(0), part: 1, geometry: tri() }];
    assert!(attach_physics(glb(), &entries, None, Some(&nmc(&["B", "A"]))).is_err(), "glTF node 0 is A, not B");
    let far = vec![PhysicsEntry { node: Some(5), part: 1, geometry: tri() }];
    assert!(attach_physics(glb(), &far, None, Some(&nmc(&["A", "B"]))).is_err(), "node outside the NMC");
    let source = nmc(&["A", "X", "C"]).nodes;
    assert!(attach_physics(glb(), &entries, Some(&source), Some(&nmc(&["A", "B"]))).is_err(), "B is not in the source NMC");
}

#[test]
fn affine_inverse_round_trips() {
    // Rotation 90° about Z with translation (column-major).
    let m = [0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0];
    let inv = invert_affine(&m).unwrap();
    let apply = |m: &[f64], p: [f64; 3]| [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]);
    let p = apply(&inv, apply(&m, [1.0, 2.0, 3.0]));
    for (a, b) in p.iter().zip([1.0, 2.0, 3.0]) { assert!((a - b).abs() < 1e-9); }
}

#[test]
fn dropped_proxy_nodes_are_recreated_in_their_frames() {
    // Source NMC: A, $proxy (child of A, translated by (0, 0, 5)), B (child of A). The GLB kept A and B.
    let mut source = nmc(&["A", "$proxy_Body", "B"]).nodes;
    source[1].parent_index = Some(0);
    source[1].bone_to_world = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 5.0]];
    source[2].parent_index = Some(0);
    let entries = vec![
        PhysicsEntry { node: Some(1), part: 1, geometry: tri() },
        PhysicsEntry { node: Some(2), part: 1, geometry: PhysicsGeometry::Sphere { center: [0.0; 3], radius: 1.0 } },
    ];
    let out = attach_physics(glb(), &entries, Some(&source), Some(&nmc(&["A", "B"]))).unwrap();
    let (root, _) = split_glb(&out).unwrap();
    assert_eq!(root["nodes"][1]["extras"]["sc_physics"][0]["kind"], "sphere", "source node 2 is glTF node 1 (B)");
    let proxy = root["nodes"].as_array().unwrap().iter().position(|n| n["name"] == "$proxy_Body").unwrap();
    assert!(root["nodes"][0]["children"].as_array().unwrap().contains(&json!(proxy)), "under its kept parent A");
    assert_eq!(root["nodes"][proxy]["matrix"][14].as_f64(), Some(5.0));
    assert_eq!(root["nodes"][proxy]["extras"]["sc_physics"][0]["kind"], "trimesh");
}

#[test]
fn whole_object_physics_rides_the_single_geometry_node() {
    // A (helper, type 3) and B (the only geometry node): whole-object physics belongs to B and moves with it.
    let mut kept = nmc(&["A", "B"]);
    kept.nodes[0].geometry_type = 3;
    let entries = vec![PhysicsEntry { node: None, part: 1, geometry: PhysicsGeometry::Sphere { center: [0.0; 3], radius: 1.0 } }];
    let out = attach_physics(glb(), &entries, None, Some(&kept)).unwrap();
    let (root, _) = split_glb(&out).unwrap();
    assert_eq!(root["nodes"][1]["extras"]["sc_physics"][0]["kind"], "sphere");
    assert!(root["nodes"].as_array().unwrap().iter().all(|n| n["name"] != "$physics_object"));
}

#[test]
fn records_carry_the_collision_class_of_their_node() {
    let entries = vec![
        PhysicsEntry { node: Some(1), part: 0, geometry: tri() },
        PhysicsEntry { node: Some(0), part: 0, geometry: tri() },
    ];
    let root_glb = {
        let (mut root, bin) = split_glb(&glb()).unwrap();
        root["nodes"][1]["name"] = json!("$IP_SeatAccess");
        join_glb(&root, &bin).unwrap()
    };
    let out = attach_physics(root_glb, &entries, None, Some(&nmc(&["A", "$IP_SeatAccess"]))).unwrap();
    let (root, _) = split_glb(&out).unwrap();
    assert_eq!(root["nodes"][1]["extras"]["sc_physics"][0]["class"], "interaction");
    assert!(root["nodes"][0]["extras"]["sc_physics"][0].get("class").is_none(), "ordinary colliders carry no class");
}

/// A minimal IVO chunk file: `chunks` as (type, data) after the 16-byte header and the 16-byte table entries.
fn ivo(chunks: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [0x6F76_6923u32, 0x900, chunks.len() as u32, 16] { b.extend(v.to_le_bytes()); }
    let mut offset = 16 + 16 * chunks.len() as u64;
    for (kind, data) in chunks {
        b.extend(kind.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(offset.to_le_bytes());
        offset += data.len() as u64;
    }
    for (_, data) in chunks { b.extend(data); }
    b
}
/// A `StatObjPhysics` chunk with one whole-object (`0xFFFF`, part 1) sphere of `radius` at the origin.
fn sphere_physics(radius: f32) -> Vec<u8> {
    let mut blob = Vec::new();
    for v in [2i32, 0] { blob.extend(v.to_le_bytes()); }
    let volume = 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3);
    for v in [1.0f32, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, volume] { blob.extend(v.to_le_bytes()); }
    for v in [1i32, 0, 0, 0, 4] { blob.extend(v.to_le_bytes()); }
    for v in [0.0f32, 0.0, 0.0, radius] { blob.extend(v.to_le_bytes()); }
    blob.resize(156, 0);
    let mut chunk = vec![0u8; 80];
    chunk[40..44].copy_from_slice(&1u32.to_le_bytes());
    chunk.extend((blob.len() as u32).to_le_bytes());
    chunk.extend(0x0001_FFFFu32.to_le_bytes());
    chunk.extend(blob);
    ivo(&[(starbreaker_chunks::known_types::ivo::STATOBJ_PHYSICS, chunk)])
}
/// Content root `body` (translated by (1, 2, 3)) with the bone node `display_01` (translated by (0, 0, 4)) under it.
fn character_glb() -> Vec<u8> {
    let t = |x: f64, y: f64, z: f64| json!([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0]);
    let root = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [2]}],
        "nodes": [
            {"name": "body", "matrix": t(1.0, 2.0, 3.0), "children": [1]},
            {"name": "display_01", "matrix": t(0.0, 0.0, 4.0)},
            {"name": "CryEngine_Z_up", "children": [0]}
        ],
        "buffers": [{"byteLength": 4}], "bufferViews": [], "accessors": [], "meshes": []
    });
    join_glb(&root, &[1, 2, 3, 4]).unwrap()
}
const CHARACTER: &str = r#"<CharacterDefinition><Model File="objects/synthetic/rig.chr"/><AttachmentList>
<Attachment Type="CA_SKIN" AName="body" Binding="objects/synthetic/body.skin"/>
<Attachment Type="CA_BONE" AName="display" Binding="objects/synthetic/display.cgf" BoneName="DISPLAY_01"/>
<Attachment Type="CA_BONE" AName="loose" Binding="objects/synthetic/loose.cgf" BoneName="bone_not_in_rig"/>
</AttachmentList></CharacterDefinition>"#;
fn reader(files: Vec<(&'static str, Vec<u8>)>) -> impl FnMut(&str) -> Result<Option<Vec<u8>>, Error> {
    let files: std::collections::HashMap<&str, Vec<u8>> = files.into_iter().collect();
    move |path: &str| Ok(files.get(path).cloned())
}

#[test]
fn a_character_definition_carries_the_physics_of_its_attachments() {
    let mut read = reader(vec![
        ("objects/synthetic/body.cdf", crate::test_xml::encode(CHARACTER)),
        ("objects/synthetic/body.skin", ivo(&[])),
        ("objects/synthetic/display.cgf", sphere_physics(0.5)),
        ("objects/synthetic/loose.cgf", sphere_physics(2.0)),
    ]);
    let out = attach_physics_from(&mut read, "objects/synthetic/body.cdf", None, character_glb()).unwrap();
    let (root, _) = split_glb(&out).unwrap();
    let nodes = root["nodes"].as_array().unwrap();
    let child_named = |parent: usize, name: &str| {
        root["nodes"][parent]["children"].as_array().into_iter().flatten()
            .map(|c| c.as_u64().unwrap() as usize).find(|&c| nodes[c]["name"] == name)
    };
    // A CA_BONE attachment's physics sits in its bone's frame, the frame its rigid render part is bound in.
    let on_bone = child_named(1, "$physics_object").expect("record node under the bone");
    assert!(nodes[on_bone].get("matrix").is_none(), "the attachment frame is the bone frame");
    let r = &nodes[on_bone]["extras"]["sc_physics"][0];
    assert_eq!((r["kind"].as_str(), r["radius"].as_f64()), (Some("sphere"), Some(0.5)));
    assert_eq!(r["source"], "objects/synthetic/display.cgf");
    // A bone the GLB lacks leaves its part unmoved (as the mesh merge does): the object's frame.
    let on_object = child_named(0, "$physics_object").expect("record node under the content root");
    let m = nodes[on_object]["matrix"].as_array().unwrap();
    assert_eq!((m[12].as_f64(), m[13].as_f64(), m[14].as_f64()), (Some(-1.0), Some(-2.0), Some(-3.0)));
    assert_eq!(nodes[on_object]["extras"]["sc_physics"][0]["radius"].as_f64(), Some(2.0));
    assert_eq!(nodes[on_object]["extras"]["sc_physics"][0]["source"], "objects/synthetic/loose.cgf");
}

#[test]
fn a_character_definition_without_attachment_physics_leaves_the_glb_unchanged() {
    let mut read = reader(vec![
        ("objects/synthetic/body.cdf", crate::test_xml::encode(CHARACTER)),
        ("objects/synthetic/body.skin", ivo(&[])),
        ("objects/synthetic/display.cgf", ivo(&[])),
    ]);
    let glb = character_glb();
    assert_eq!(attach_physics_from(&mut read, "objects/synthetic/body.cdf", None, glb.clone()).unwrap(), glb);
}
