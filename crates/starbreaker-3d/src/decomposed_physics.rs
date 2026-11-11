//! Decomposed mesh GLBs with the source's CryPhysics collision geometry (`ExportOptions::include_physics`).
//! [`attach_source_physics`] reads the `.cga` / `.cgf`'s `StatObjPhysics` chunk ([`crate::ivo::physics`]) and
//! [`attach_physics`] adds it to a written GLB. Physics keys index the source's full NMC; the GLB holds the
//! filtered NMC (helper nodes dropped by `filter_nmc_hierarchy`), so source nodes are aligned to glTF nodes by
//! order and name, and a dropped node that carries colliders (e.g. `$proxy_*`) is recreated as a transform-only
//! node, with any dropped ancestors, under its kept parent. Each node's records sit in its own frame (the frame of
//! its render vertices) as `extras.sc_physics`; a trimesh becomes an unreferenced glTF mesh named by index;
//! whole-object physics (`0xFFFF`) belongs to the object's single geometry node when it has exactly one (NMC
//! geometry type 0: 96 of 111 sampled `.cga` with whole-object physics, none also with per-node physics), so it
//! moves with that node (an animated door leaf); otherwise it sits on a `$physics_object` child of the first content
//! root whose matrix undoes that root's, so its frame is the object's. A record whose node name gives a collision
//! class ([`crate::physics_class`]) carries it as `class`. Contract: docs/decomposed-export-contract.md.
use crate::error::Error;
use crate::ivo::physics::{PhysicsEntry, PhysicsGeometry, read_physics};
use crate::nmc::{NmcNode, NodeMeshCombo};
use serde_json::{Value, json};
use starbreaker_p4k::MappedP4k;

/// The GLB with the physics of `geometry_path` attached; unchanged when the source carries none.
pub(crate) fn attach_source_physics(p4k: &MappedP4k, geometry_path: &str, nmc: Option<&NodeMeshCombo>, glb: Vec<u8>) -> Result<Vec<u8>, Error> {
    let p4k_path = crate::pipeline::datacore_path_to_p4k(geometry_path);
    let Some(entry) = p4k.entry_case_insensitive(&p4k_path) else { return Ok(glb) };
    let bytes = p4k.read(entry).map_err(|e| Error::Other(format!("{geometry_path}: {e}")))?;
    match read_physics(&bytes).map_err(|e| Error::Other(format!("{geometry_path}: {e}")))? {
        Some(entries) if !entries.is_empty() => {
            let source = crate::nmc::parse_nmc_full(&bytes).map(|(nodes, _)| nodes);
            attach_physics(glb, &entries, source.as_deref(), nmc).map_err(|e| Error::Other(format!("{geometry_path}: {e}")))
        }
        _ => Ok(glb),
    }
}

/// Add `entries` to a GLB written by `crate::gltf::write_glb` for one mesh asset. `source` is the file's full
/// NMC (physics keys index it), `kept` the filtered NMC the GLB was written with (glTF node `i` = kept node `i`).
pub(crate) fn attach_physics(glb: Vec<u8>, entries: &[PhysicsEntry], source: Option<&[NmcNode]>, kept: Option<&NodeMeshCombo>) -> Result<Vec<u8>, Error> {
    let (mut root, mut bin) = split_glb(&glb)?;
    let nodes_len = root["nodes"].as_array().map_or(0, Vec::len);
    let kept: &[NmcNode] = kept.map_or(&[], |n| &n.nodes);
    for (i, node) in kept.iter().enumerate() {
        if i >= nodes_len || root["nodes"][i]["name"].as_str() != Some(node.name.as_str()) {
            return Err(Error::Other(format!("physics: glTF node {i} is not NMC node {}", node.name)));
        }
    }
    let source: &[NmcNode] = source.unwrap_or(kept);
    // Source -> kept by order and name: the filter keeps order and drops whole nodes.
    let mut aligned: Vec<Option<usize>> = Vec::with_capacity(source.len());
    let mut next = 0;
    for node in source {
        let hit = next < kept.len() && kept[next].name == node.name;
        aligned.push(hit.then_some(next));
        next += hit as usize;
    }
    if next != kept.len() {
        return Err(Error::Other(format!("physics: GLB nodes do not follow the source NMC ({next} of {} aligned)", kept.len())));
    }
    let geometry_nodes: Vec<usize> = (0..source.len()).filter(|&i| source[i].geometry_type == 0).collect();
    let object_node = (geometry_nodes.len() == 1).then(|| geometry_nodes[0]);
    let mut records: Vec<(Option<usize>, Value)> = Vec::new();
    for e in entries {
        let source_node = e.node.map(usize::from).or(object_node);
        let target = match source_node {
            Some(n) if n < source.len() => Some(gltf_node(&mut root, source, &mut aligned, n)?),
            Some(n) => return Err(Error::Other(format!("physics: node {n} outside the NMC ({} nodes)", source.len()))),
            None => None,
        };
        let mut r = match &e.geometry {
            PhysicsGeometry::Box { axes, center, half_size } => json!({"kind": "box", "center": center, "half_size": half_size, "axes": axes}),
            PhysicsGeometry::Sphere { center, radius } => json!({"kind": "sphere", "center": center, "radius": radius}),
            PhysicsGeometry::Cylinder { center, axis, radius, half_height } =>
                json!({"kind": "cylinder", "center": center, "axis": axis, "radius": radius, "half_height": half_height}),
            PhysicsGeometry::Capsule { center, axis, radius, half_height } =>
                json!({"kind": "capsule", "center": center, "axis": axis, "radius": radius, "half_height": half_height}),
            PhysicsGeometry::Trimesh { vertices, triangles, .. } => {
                let mesh = push_trimesh(&mut root, &mut bin, vertices, triangles, e)?;
                json!({"kind": "trimesh", "mesh": mesh, "vertices": vertices.len(), "triangles": triangles.len()})
            }
            PhysicsGeometry::DamageMesh { bytes } => json!({"kind": "damage_mesh", "bytes": bytes}),
            PhysicsGeometry::Sdf { bytes } => json!({"kind": "sdf", "bytes": bytes}),
        };
        r["part"] = json!(e.part);
        if let Some(class) = source_node.and_then(|n| crate::physics_class::collision_class(&source[n].name)) {
            r["class"] = json!(class);
        }
        records.push((target, r));
    }
    let object: Vec<Value> = records.iter().filter(|(t, _)| t.is_none()).map(|(_, r)| r.clone()).collect();
    for (target, r) in records.into_iter().filter(|(t, _)| t.is_some()) {
        push_record(&mut root["nodes"][target.unwrap()], r);
    }
    if !object.is_empty() {
        add_object_node(&mut root, object)?;
    }
    while bin.len() % 4 != 0 { bin.push(0); }
    root["buffers"][0]["byteLength"] = json!(bin.len());
    join_glb(&root, &bin)
}

/// The glTF node of source NMC node `i`, recreating a dropped node (and dropped ancestors) as transform-only.
fn gltf_node(root: &mut Value, source: &[NmcNode], aligned: &mut [Option<usize>], i: usize) -> Result<usize, Error> {
    if let Some(index) = aligned[i] { return Ok(index); }
    let node = &source[i];
    let local: Option<Vec<f64>> = (!crate::gltf::is_identity_or_zero(&node.bone_to_world))
        .then(|| crate::gltf::mat3x4_to_gltf(&node.bone_to_world).iter().map(|&v| f64::from(v)).collect());
    let parent = match node.parent_index.map(usize::from).filter(|&p| p < source.len() && p != i) {
        Some(p) => Some(gltf_node(root, source, aligned, p)?),
        None => None,
    };
    let (parent, matrix) = match parent {
        Some(p) => (p, local),
        // A dropped NMC root: under the first content root, with that root's matrix undone (as `$physics_object`).
        None => {
            let content = content_root(root)?;
            let undo = root["nodes"][content]["matrix"].as_array()
                .map(|m| invert_affine(&m.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect::<Vec<_>>()))
                .map(|m| m.ok_or_else(|| Error::Other("physics: singular content root matrix".into())))
                .transpose()?;
            let matrix = match (undo, local) {
                (Some(u), Some(l)) => Some(multiply(&u, &l)),
                (u, l) => u.or(l),
            };
            (content, matrix)
        }
    };
    let mut json_node = json!({"name": node.name});
    if let Some(m) = matrix { json_node["matrix"] = json!(m); }
    let nodes = root["nodes"].as_array_mut().unwrap();
    let index = nodes.len();
    nodes.push(json_node);
    let parent_node = &mut root["nodes"][parent];
    if !parent_node["children"].is_array() { parent_node["children"] = json!([]); }
    parent_node["children"].as_array_mut().unwrap().push(json!(index));
    aligned[i] = Some(index);
    Ok(index)
}

fn content_root(root: &Value) -> Result<usize, Error> {
    let scene = root["scene"].as_u64().unwrap_or(0) as usize;
    let wrapper = root["scenes"][scene]["nodes"][0].as_u64().ok_or_else(|| Error::Other("physics: GLB without a scene root".into()))? as usize;
    Ok(root["nodes"][wrapper]["children"][0].as_u64().map(|c| c as usize).unwrap_or(wrapper))
}

/// Column-major 4x4 product `a · b`.
fn multiply(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..16).map(|i| { let (c, r) = (i / 4, i % 4); (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum() }).collect()
}

fn push_record(node: &mut Value, record: Value) {
    if !node["extras"].is_object() { node["extras"] = json!({}); }
    if !node["extras"]["sc_physics"].is_array() { node["extras"]["sc_physics"] = json!([]); }
    node["extras"]["sc_physics"].as_array_mut().unwrap().push(record);
}

/// Whole-object physics: a child of the first content root (under the coordinate wrapper) with that root's
/// matrix inverted, so the records sit in the object's frame without changing the wrapper's children.
fn add_object_node(root: &mut Value, records: Vec<Value>) -> Result<(), Error> {
    let content = content_root(root)?;
    let mut node = json!({"name": "$physics_object", "extras": {"sc_physics": records}});
    if let Some(m) = root["nodes"][content]["matrix"].as_array() {
        let m: Vec<f64> = m.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect();
        node["matrix"] = json!(invert_affine(&m).ok_or_else(|| Error::Other("physics: singular content root matrix".into()))?);
    }
    let nodes = root["nodes"].as_array_mut().unwrap();
    let index = nodes.len();
    nodes.push(node);
    let parent = &mut root["nodes"][content];
    if !parent["children"].is_array() { parent["children"] = json!([]); }
    parent["children"].as_array_mut().unwrap().push(json!(index));
    Ok(())
}

/// Column-major glTF affine matrix inverse.
fn invert_affine(m: &[f64]) -> Option<Vec<f64>> {
    if m.len() != 16 || m.iter().any(|v| !v.is_finite()) { return None; }
    let a = |r: usize, c: usize| m[c * 4 + r];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-12 { return None; }
    let mut inv = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            let (r1, r2, c1, c2) = ((c + 1) % 3, (c + 2) % 3, (r + 1) % 3, (r + 2) % 3);
            inv[r][c] = (a(r1, c1) * a(r2, c2) - a(r1, c2) * a(r2, c1)) / det;
        }
    }
    let t = [a(0, 3), a(1, 3), a(2, 3)];
    let mut out = vec![0.0; 16];
    for c in 0..3 { for r in 0..3 { out[c * 4 + r] = inv[r][c]; } }
    for r in 0..3 { out[12 + r] = -(inv[r][0] * t[0] + inv[r][1] * t[1] + inv[r][2] * t[2]); }
    out[15] = 1.0;
    Some(out)
}

fn push_trimesh(root: &mut Value, bin: &mut Vec<u8>, vertices: &[[f32; 3]], triangles: &[[u16; 3]], e: &PhysicsEntry) -> Result<usize, Error> {
    if vertices.is_empty() || triangles.is_empty() { return Err(Error::Other("physics: empty trimesh".into())); }
    let view = |root: &mut Value, bin: &mut Vec<u8>, data: &[u8], target: u32| {
        while bin.len() % 4 != 0 { bin.push(0); }
        let offset = bin.len();
        bin.extend_from_slice(data);
        let views = root["bufferViews"].as_array_mut().unwrap();
        views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": data.len(), "target": target}));
        views.len() - 1
    };
    if !root["bufferViews"].is_array() { root["bufferViews"] = json!([]); }
    if !root["accessors"].is_array() { root["accessors"] = json!([]); }
    if !root["meshes"].is_array() { root["meshes"] = json!([]); }
    let positions: Vec<u8> = vertices.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let indices: Vec<u8> = triangles.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in vertices { for k in 0..3 { min[k] = min[k].min(v[k]); max[k] = max[k].max(v[k]); } }
    let pv = view(root, bin, &positions, 34962);
    let iv = view(root, bin, &indices, 34963);
    let accessors = root["accessors"].as_array_mut().unwrap();
    accessors.push(json!({"bufferView": pv, "componentType": 5126, "count": vertices.len(), "type": "VEC3", "min": min, "max": max}));
    accessors.push(json!({"bufferView": iv, "componentType": 5123, "count": triangles.len() * 3, "type": "SCALAR"}));
    let (pa, ia) = (accessors.len() - 2, accessors.len() - 1);
    let meshes = root["meshes"].as_array_mut().unwrap();
    let name = format!("physics_{}_part{}", e.node.map_or("object".to_string(), |n| n.to_string()), e.part);
    meshes.push(json!({"name": name, "primitives": [{"attributes": {"POSITION": pa}, "indices": ia, "mode": 4}]}));
    Ok(meshes.len() - 1)
}

fn split_glb(glb: &[u8]) -> Result<(Value, Vec<u8>), Error> {
    let u32_at = |at: usize| glb.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    let bad = || Error::Other("physics: not a GLB".into());
    if glb.get(..4) != Some(b"glTF") { return Err(bad()); }
    let json_len = u32_at(12).ok_or_else(bad)?;
    let json_bytes = glb.get(20..20 + json_len).ok_or_else(bad)?;
    let root: Value = serde_json::from_slice(json_bytes).map_err(|e| Error::Other(format!("physics: GLB JSON: {e}")))?;
    let bin_at = 20 + json_len;
    let bin = match u32_at(bin_at) {
        Some(len) => glb.get(bin_at + 8..bin_at + 8 + len).ok_or_else(bad)?.to_vec(),
        None => Vec::new(),
    };
    Ok((root, bin))
}

fn join_glb(root: &Value, bin: &[u8]) -> Result<Vec<u8>, Error> {
    let mut json = serde_json::to_vec(root).map_err(|e| Error::Other(format!("physics: GLB JSON: {e}")))?;
    while json.len() % 4 != 0 { json.push(b' '); }
    let mut bin = bin.to_vec();
    while bin.len() % 4 != 0 { bin.push(0); }
    let total = 12 + 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "decomposed_physics_tests.rs"]
mod tests;
