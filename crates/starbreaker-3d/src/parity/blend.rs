//! `.blend` → [`Summary`], read with `starbreaker_blend::reader` at the
//! offsets the writers use: mesh objects (local meshes, or linked mesh stubs
//! followed into their library files through `resolve`), their attributes,
//! vertex groups + `MDeformVert` weights, shape keys; armature bones (the
//! ones some mesh is weighted to); and actions from every file read (clip
//! name = action name without `_shapes`; key frames → seconds at the
//! scene's frame rate).

use std::collections::HashMap;

use starbreaker_blend::reader::{BlendFile, Block, cstr, f32_at, i16_at, i32_at, ptr_at};

use super::{Channel, MeshSummary, Summary, insert_unique, normalize_name, position_hash};

const ATTR_TYPE_FLOAT2: i16 = 6;
const ATTR_TYPE_BYTE_COLOR: i16 = 9;
const ATTR_DOMAIN_CORNER: u8 = 3;

/// `dir/../x` → `x`, `\` → `/`, `//` (blend-relative) prefix dropped.
fn join(base_dir: &str, relative: &str) -> String {
    let relative = relative.trim_start_matches("//").replace('\\', "/");
    let mut parts: Vec<&str> = base_dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn string_at(file: &BlendFile, ptr: u64) -> String {
    file.block(ptr).map(|b| cstr(b.data, 0, b.data.len()).to_string()).unwrap_or_default()
}

fn mesh_summary(file: &BlendFile, me: Block<'_>) -> MeshSummary {
    let d = me.data;
    let mut m = MeshSummary {
        vertices: i32_at(d, 432).max(0) as usize,
        triangles: i32_at(d, 440).max(0) as usize,
        material_slots: i16_at(d, 1618).max(0) as usize,
        ..Default::default()
    };
    if let Some(attrs) = file.block(ptr_at(d, 456)) {
        for a in attrs.data.chunks_exact(24).take(i32_at(d, 464).max(0) as usize) {
            let name = string_at(file, ptr_at(a, 0));
            let (kind, domain) = (i16_at(a, 8), a[10]);
            let raw = file.block(ptr_at(a, 16)).and_then(|arr| file.block(ptr_at(arr.data, 0)));
            match name.as_str() {
                "position" => {
                    if let Some(raw) = raw {
                        m.positions = position_hash(raw.data.chunks_exact(12).map(|c| [f32_at(c, 0), f32_at(c, 4), f32_at(c, 8)]));
                    }
                }
                "custom_normal" => m.normals = true,
                _ if kind == ATTR_TYPE_FLOAT2 && domain == ATTR_DOMAIN_CORNER => m.uv_sets += 1,
                _ if kind == ATTR_TYPE_BYTE_COLOR => m.colors = true,
                _ => {}
            }
        }
    }
    let groups: Vec<String> = file.list(d, 1472).iter().map(|g| cstr(g.data, 16, 64).to_string()).collect();
    if let Some(layer) = file.block(ptr_at(d, 480)).filter(|l| i32_at(l.data, 0) == 2) {
        if let Some(verts) = file.block(ptr_at(layer.data, 96)) {
            for v in verts.data.chunks_exact(16) {
                let Some(weights) = file.block(ptr_at(v, 0)) else { continue };
                for w in weights.data.chunks_exact(8).take(i32_at(v, 8).max(0) as usize) {
                    let Some(name) = groups.get(i32_at(w, 0) as usize) else { continue };
                    if !name.starts_with("starbreaker_") {
                        *m.joints.entry(name.clone()).or_insert(0.0) += f32_at(w, 4);
                    }
                }
            }
        }
    }
    if let Some(key) = file.block(ptr_at(d, 416)) {
        let mut morphs: Vec<String> = file.list(key.data, 464).iter().map(|kb| cstr(kb.data, 48, 64).to_string()).filter(|n| n != "Basis").collect();
        morphs.sort();
        m.morphs = morphs;
    }
    m
}

fn bones(file: &BlendFile, list_owner: &[u8], off: usize, out: &mut HashMap<String, [f32; 3]>) {
    for bone in file.list(list_owner, off) {
        out.insert(cstr(bone.data, 64, 64).to_string(), [f32_at(bone.data, 228), f32_at(bone.data, 232), f32_at(bone.data, 236)]);
        bones(file, bone.data, 48, out);
    }
}

fn actions(file: &BlendFile, fps: f32, summary: &mut Summary) {
    for action in file.ids(b"AC") {
        let name = action.id_name();
        let clip = summary.clips.entry(name.strip_suffix("_shapes").unwrap_or(name).to_string()).or_default();
        for kd in file.ptr_array(ptr_at(action.data, 440)) {
            let Some(kd) = file.block(kd) else { continue };
            for bag in file.ptr_array(ptr_at(kd.data, 0)) {
                let Some(bag) = file.block(bag) else { continue };
                for curve in file.ptr_array(ptr_at(bag.data, 24)) {
                    let Some(curve) = file.block(curve) else { continue };
                    let path = string_at(file, ptr_at(curve.data, 88));
                    let keys: Vec<(f32, f32)> = file
                        .block(ptr_at(curve.data, 48))
                        .map(|b| b.data.chunks_exact(72).map(|k| (f32_at(k, 12), f32_at(k, 16))).collect())
                        .unwrap_or_default();
                    let times = keys.iter().map(|(f, _)| (f - 1.0) / fps).collect();
                    let quoted = path.split('"').nth(1).unwrap_or("");
                    let channel = if path.starts_with("pose.bones[") && i32_at(curve.data, 84) == 0 {
                        if path.ends_with(".rotation_quaternion") {
                            Some((format!("bone:{quoted}:rotation"), Vec::new()))
                        } else if path.ends_with(".location") {
                            Some((format!("bone:{quoted}:translation"), Vec::new()))
                        } else {
                            None
                        }
                    } else if path.starts_with("key_blocks[") && path.ends_with(".value") {
                        Some((format!("morph:{quoted}"), keys.iter().map(|(_, v)| *v).collect()))
                    } else {
                        None
                    };
                    if let Some((channel, values)) = channel {
                        clip.channels.insert(channel, Channel { times, values });
                    }
                }
            }
        }
    }
}

/// Summarise the `.blend` at export-relative `path` (see the module docs);
/// `resolve` returns the bytes of another export-relative file.
pub fn from_blend(bytes: &[u8], path: &str, resolve: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Summary, String> {
    let file = BlendFile::parse(bytes)?;
    let base_dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    let fps = file.ids(b"SC").next().map_or(24.0, |sc| f32::from(u16::from_le_bytes(sc.data[1116..1118].try_into().unwrap())));
    let mut libraries: HashMap<String, BlendFile> = HashMap::new();
    let mut summary = Summary::default();
    let mut armature_bones: HashMap<String, [f32; 3]> = HashMap::new();

    for object in file.ids(b"OB") {
        let data_ptr = ptr_at(object.data, 552);
        match i16_at(object.data, 416) {
            1 => {
                let Some(data) = file.block(data_ptr) else { continue };
                let mesh = if &data.code == b"ME\0\0" {
                    Some(mesh_summary(&file, data))
                } else {
                    // A linked mesh stub: find the mesh in its library.
                    let lib_path = file.block(ptr_at(data.data, 24)).map(|li| join(base_dir, cstr(li.data, 408, 1024)));
                    lib_path.and_then(|lib_path| {
                        if !libraries.contains_key(&lib_path) {
                            let lib = BlendFile::parse(&resolve(&lib_path)?).ok()?;
                            libraries.insert(lib_path.clone(), lib);
                        }
                        let lib = &libraries[&lib_path];
                        lib.ids(b"ME").find(|me| me.id_name() == data.id_name()).map(|me| mesh_summary(lib, me))
                    })
                };
                match mesh {
                    Some(mesh) => insert_unique(&mut summary.meshes, normalize_name(object.id_name()), mesh),
                    None => return Err(format!("object '{}': mesh data not found", object.id_name())),
                }
            }
            25 => {
                if let Some(armature) = file.block(data_ptr) {
                    bones(&file, armature.data, 416, &mut armature_bones);
                }
            }
            _ => {}
        }
    }
    let used: std::collections::BTreeSet<&String> = summary.meshes.values().flat_map(|m| m.joints.keys()).collect();
    summary.bones = armature_bones.into_iter().filter(|(name, _)| used.contains(name)).collect();
    actions(&file, fps, &mut summary);
    for lib in libraries.values() {
        actions(lib, fps, &mut summary);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::join;

    #[test]
    fn library_paths_resolve_against_the_file() {
        assert_eq!(join("Packages/x", "//../../Data/Objects/a.blend"), "Data/Objects/a.blend");
        assert_eq!(join("Packages/x", r"//..\..\Data\a.blend"), "Data/a.blend");
        assert_eq!(join("", "b.blend"), "b.blend");
    }
}
