//! Synthetic coverage of the `StatObjPhysics` decoder: table/blob framing, every CryPhysics geometry type
//! (with the volume identities real data satisfies), the trimesh vertex-map variant, damage kinds, rejection.
use super::*;

fn header(volume: f32, kind: i32) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [2i32, 0] { b.extend(v.to_le_bytes()); }
    for v in [1.0f32, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, volume] { b.extend(v.to_le_bytes()); }
    for v in [1i32, 0, 0, 0, kind] { b.extend(v.to_le_bytes()); }
    assert_eq!(b.len(), 72);
    b
}
fn floats(b: &mut Vec<u8>, v: &[f32]) { for x in v { b.extend(x.to_le_bytes()); } }
fn chunk(blobs: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut c = vec![0u8; 80];
    c[40..44].copy_from_slice(&(blobs.len() as u32).to_le_bytes());
    for (key, blob) in blobs { c.extend((blob.len() as u32).to_le_bytes()); c.extend(key.to_le_bytes()); }
    for (_, blob) in blobs { c.extend(blob); }
    c
}
fn box_blob() -> Vec<u8> {
    let mut b = header(1.834, 0);
    floats(&mut b, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    b.extend(0i32.to_le_bytes());
    floats(&mut b, &[0.0, 0.0, 0.05, 1.834, 2.5, 0.05]);
    b.resize(140, 0); // tree tail
    b
}
fn trimesh_blob(vertex_map: bool) -> Vec<u8> {
    // A tetrahedron: 4 vertices, 4 triangles.
    let mut b = header(1.0 / 6.0, 1);
    let n_a = 5i32;
    for v in [4i32, 4, 3, 0x40, n_a] { b.extend(v.to_le_bytes()); }
    b.push(vertex_map as u8);
    b.push(1);
    for i in 0..n_a { b.extend((i as u16).to_le_bytes()); }
    if vertex_map { for i in 0..4u16 { b.extend(i.to_le_bytes()); } }
    b.extend([0u8, 17, 0, 255]); // per-triangle material ids
    floats(&mut b, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    for i in [0u16, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3] { b.extend(i.to_le_bytes()); }
    b.extend([0u8; 24]); // tree tail
    b
}

#[test]
fn decodes_entries_parts_and_every_geometry_type() {
    let mut sphere = header(4.0 / 3.0 * std::f32::consts::PI * 8.0, 4);
    floats(&mut sphere, &[1.0, 2.0, 3.0, 2.0]);
    sphere.resize(156, 0);
    let mut cylinder = header(2.0 * std::f32::consts::PI * 0.25 * 1.5, 5);
    floats(&mut cylinder, &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 1.5]);
    cylinder.resize(176, 0);
    let mut capsule = header(0.0, 6);
    floats(&mut capsule, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.25, 0.5]);
    capsule.resize(176, 0);
    let mut damage = 0xCAFE_BABEu32.to_le_bytes().to_vec();
    damage.extend([0u8; 12]);
    let mut sdf = 0xDEAD_BEB5u32.to_le_bytes().to_vec();
    sdf.extend([0u8; 8]);
    let c = chunk(&[
        (0x0000_0007, box_blob()),
        (0x0001_0003, trimesh_blob(false)),
        (0x0002_0003, trimesh_blob(true)),
        (0x0001_FFFF, sphere),
        (0x0003_0003, cylinder),
        (0x0004_0003, capsule),
        (0x0001_0005, damage),
        (0x0001_000B, sdf),
    ]);
    let e = parse_stat_obj_physics(&c).unwrap();
    assert_eq!(e.len(), 8);
    assert_eq!((e[0].node, e[0].part), (Some(7), 0));
    match &e[0].geometry {
        PhysicsGeometry::Box { axes, center, half_size } => {
            assert_eq!(*axes, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
            assert_eq!(*center, [0.0, 0.0, 0.05]);
            assert_eq!(*half_size, [1.834, 2.5, 0.05]);
        }
        g => panic!("box expected, got {g:?}"),
    }
    for entry in &e[1..3] {
        match &entry.geometry {
            PhysicsGeometry::Trimesh { vertices, triangles, material_ids } => {
                assert_eq!(vertices.len(), 4);
                assert_eq!(triangles[1], [0, 1, 3]);
                assert_eq!(material_ids, &vec![0, 17, 0, 255]);
            }
            g => panic!("trimesh expected, got {g:?}"),
        }
    }
    assert_eq!((e[3].node, e[3].part), (None, 1), "0xFFFF = the whole object");
    assert!(matches!(e[3].geometry, PhysicsGeometry::Sphere { radius, .. } if radius == 2.0));
    assert!(matches!(e[4].geometry, PhysicsGeometry::Cylinder { half_height, .. } if half_height == 1.5));
    assert!(matches!(e[5].geometry, PhysicsGeometry::Capsule { axis, .. } if axis == [1.0, 0.0, 0.0]));
    assert!(matches!(e[6].geometry, PhysicsGeometry::DamageMesh { bytes: 16 }));
    assert!(matches!(e[7].geometry, PhysicsGeometry::Sdf { bytes: 12 }));
}

#[test]
fn rejects_malformed_physics() {
    let mut short = trimesh_blob(false);
    short.truncate(150);
    assert!(parse_stat_obj_physics(&chunk(&[(0x10000, short)])).is_err(), "truncated trimesh");
    let mut bad_index = trimesh_blob(false);
    let at = 72 + 20 + 2 + 10 + 4 + 48; // first index
    bad_index[at..at + 2].copy_from_slice(&9u16.to_le_bytes());
    assert!(parse_stat_obj_physics(&chunk(&[(0x10000, bad_index)])).is_err(), "index past the vertices");
    let mut unknown = box_blob();
    unknown[68..72].copy_from_slice(&3i32.to_le_bytes());
    assert!(parse_stat_obj_physics(&chunk(&[(0x10000, unknown)])).is_err(), "unknown geometry type");
    let mut table = chunk(&[(0x10000, box_blob())]);
    table[80..84].copy_from_slice(&999u32.to_le_bytes());
    assert!(parse_stat_obj_physics(&table).is_err(), "blob past the chunk");
}

/// Live data (needs the game's Data.p4k): the Cutlass ramp and cargo bay, measured in the WP-P3 survey.
#[test]
#[ignore = "needs Data.p4k"]
fn cutlass_ramp_and_bay_decode_as_surveyed() {
    let Ok(p4k) = starbreaker_p4k::open_p4k() else { return };
    let read = |path: &str| p4k.read_file(path).expect(path);
    let ramp = read_physics(&read("Data\\Objects\\Spaceships\\Ships\\DRAK\\Cutlass\\DRAK_Cutlass_Door_Ramp.cga")).unwrap().unwrap();
    assert_eq!(ramp.len(), 17);
    let player = ramp.iter().find(|e| e.node == Some(1)).unwrap();
    assert_eq!(player.part, 0);
    assert!(matches!(&player.geometry, PhysicsGeometry::Trimesh { vertices, triangles, .. } if vertices.len() == 10 && triangles.len() == 16));
    let tires = ramp.iter().find(|e| e.node == Some(7)).unwrap();
    match tires.geometry {
        PhysicsGeometry::Box { half_size, .. } => {
            for (a, b) in half_size.iter().zip([1.834, 2.5, 0.05]) { assert!((a - b).abs() < 1e-3, "{half_size:?}"); }
        }
        ref g => panic!("box expected, got {g:?}"),
    }
    assert!(ramp.iter().any(|e| e.node == Some(11) && e.part == 1 && matches!(e.geometry, PhysicsGeometry::Sdf { .. })));
    let bay = read_physics(&read("Data\\Objects\\Spaceships\\Ships\\DRAK\\Cutlass\\DRAK_Cutlass_Black_Int_Rear.cgf")).unwrap().unwrap();
    let walk = bay.iter().find(|e| e.node.is_none() && matches!(e.geometry, PhysicsGeometry::Trimesh { .. })).unwrap();
    assert_eq!(walk.part, 1);
    assert!(matches!(&walk.geometry, PhysicsGeometry::Trimesh { vertices, triangles, .. } if vertices.len() == 470 && triangles.len() == 744));
}
