//! GLB → [`Summary`]: mesh nodes (by node name), their skins and morph
//! targets, weighted skin joints' bind heads (from the inverse bind matrices, in the
//! space under the `CryEngine_Z_up` root), and animations (channels by node
//! name; weights split per morph target; material parameters from the
//! animation's extras).

use super::{Channel, ClipSummary, MeshSummary, Summary, insert_unique, normalize_name, position_hash};

fn json_chunk(glb: &[u8]) -> Result<serde_json::Value, String> {
    if glb.len() < 20 || &glb[..4] != b"glTF" {
        return Err("not a GLB".into());
    }
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&glb[20..20 + len]).map_err(|e| e.to_string())
}

/// Summarise a GLB (see the module docs).
pub fn from_glb(glb: &[u8]) -> Result<Summary, String> {
    let (doc, buffers, _) = gltf::import_slice(glb).map_err(|e| e.to_string())?;
    let json = json_chunk(glb)?;
    let buffer = |b: gltf::Buffer| Some(&buffers[b.index()][..]);

    // Node worlds, and the CryEngine-space root (the `CryEngine_Z_up` node).
    let mut world = vec![glam::Mat4::IDENTITY; doc.nodes().len()];
    fn walk(n: gltf::Node, parent: glam::Mat4, out: &mut [glam::Mat4]) {
        let m = parent * glam::Mat4::from_cols_array_2d(&n.transform().matrix());
        out[n.index()] = m;
        for c in n.children() {
            walk(c, m, out);
        }
    }
    for scene in doc.scenes() {
        for n in scene.nodes() {
            walk(n, glam::Mat4::IDENTITY, &mut world);
        }
    }
    let to_cry = doc
        .nodes()
        .find(|n| n.name() == Some("CryEngine_Z_up"))
        .map(|n| world[n.index()].inverse())
        .unwrap_or(glam::Mat4::IDENTITY);

    let mut summary = Summary::default();
    for node in doc.nodes() {
        let Some(mesh) = node.mesh() else { continue };
        let name = node.name().or(mesh.name()).unwrap_or("mesh");
        let mut m = MeshSummary::default();
        let mut seen = std::collections::HashSet::new();
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut materials = std::collections::HashSet::new();
        for prim in mesh.primitives() {
            let reader = prim.reader(buffer);
            m.triangles += reader.read_indices().map_or(0, |i| i.into_u32().count() / 3);
            m.normals |= prim.get(&gltf::Semantic::Normals).is_some();
            m.colors |= prim.get(&gltf::Semantic::Colors(0)).is_some();
            m.uv_sets = m.uv_sets.max((0..4).filter(|&k| prim.get(&gltf::Semantic::TexCoords(k)).is_some()).count());
            if let Some(material) = prim.material().index() {
                materials.insert(material);
            }
            let Some(accessor) = prim.get(&gltf::Semantic::Positions) else { continue };
            if !seen.insert(accessor.index()) {
                continue; // primitives sharing one vertex buffer
            }
            positions.extend(reader.read_positions().into_iter().flatten());
            if let Some(skin) = node.skin() {
                let joint_names: Vec<String> = skin.joints().map(|j| j.name().unwrap_or("").to_string()).collect();
                for set in 0..2 {
                    let (Some(joints), Some(weights)) = (reader.read_joints(set), reader.read_weights(set)) else { continue };
                    for (j, w) in joints.into_u16().zip(weights.into_f32()) {
                        for k in 0..4 {
                            let name = &joint_names[j[k] as usize];
                            if w[k] > 0.0 && !name.ends_with("_skin_anchor") {
                                *m.joints.entry(name.clone()).or_insert(0.0) += w[k];
                            }
                        }
                    }
                }
            }
        }
        m.vertices = positions.len();
        m.positions = position_hash(positions.into_iter());
        m.material_slots = materials.len();
        let mut morphs: Vec<String> = json["meshes"][mesh.index()]["extras"]["targetNames"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        morphs.sort();
        m.morphs = morphs;

        if let Some(skin) = node.skin() {
            let ibms: Vec<glam::Mat4> = skin
                .reader(buffer)
                .read_inverse_bind_matrices()
                .map(|it| it.map(|m| glam::Mat4::from_cols_array_2d(&m)).collect())
                .unwrap_or_default();
            for (joint, ibm) in skin.joints().zip(ibms) {
                let name = joint.name().unwrap_or("");
                if name.is_empty() || name.ends_with("_skin_anchor") {
                    continue;
                }
                let bind = to_cry * world[node.index()] * ibm.inverse();
                summary.bones.insert(name.to_string(), bind.w_axis.truncate().to_array());
            }
        }
        insert_unique(&mut summary.meshes, normalize_name(name), m);
    }
    // Like the .blend side: only joints some vertex is weighted to.
    let used: std::collections::BTreeSet<String> = summary.meshes.values().flat_map(|m| m.joints.keys().cloned()).collect();
    summary.bones.retain(|name, _| used.contains(name));

    for (index, anim) in doc.animations().enumerate() {
        let mut clip = ClipSummary::default();
        for channel in anim.channels() {
            let node = channel.target().node();
            let reader = channel.reader(buffer);
            let times: Vec<f32> = reader.read_inputs().map(|i| i.collect()).unwrap_or_default();
            let bone = node.name().unwrap_or("");
            match reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Rotations(_)) => {
                    clip.channels.insert(format!("bone:{bone}:rotation"), Channel { times, values: Vec::new() });
                }
                Some(gltf::animation::util::ReadOutputs::Translations(_)) => {
                    clip.channels.insert(format!("bone:{bone}:translation"), Channel { times, values: Vec::new() });
                }
                Some(gltf::animation::util::ReadOutputs::MorphTargetWeights(w)) => {
                    let w: Vec<f32> = w.into_f32().collect();
                    let targets: Vec<String> = node
                        .mesh()
                        .and_then(|m| json["meshes"][m.index()]["extras"]["targetNames"].as_array().cloned())
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    let n = targets.len().max(1);
                    for (k, target) in targets.iter().enumerate() {
                        let values = w.iter().skip(k).step_by(n).copied().collect();
                        clip.channels.insert(format!("morph:{target}"), Channel { times: times.clone(), values });
                    }
                }
                _ => {}
            }
        }
        for param in json["animations"][index]["extras"]["starbreaker_material_params"].as_array().into_iter().flatten() {
            let times = param["times"].as_array().into_iter().flatten().filter_map(|t| t.as_f64()).map(|t| t as f32).collect();
            clip.channels.insert(format!("material:{}", param["param"].as_str().unwrap_or("")), Channel { times, values: Vec::new() });
        }
        insert_unique(&mut summary.clips, anim.name().unwrap_or("clip").to_string(), clip);
    }
    Ok(summary)
}
