//! Blender 5.1 custom split normals: per-vertex normals encoded as the
//! `custom_normal` corner attribute (`INT16_2D`, one `short2` per corner).
//!
//! Blender stores custom normals relative to each corner's *fan space* (the
//! smooth fan of face corners around a vertex): `x` scales the angle away from
//! the fan's default normal, `y` the angle around it from the fan's first edge.
//! Unlike a free `FLOAT3` `custom_normal` attribute, this encoding follows the
//! surface when an Armature modifier or shape key deforms the mesh.
//!
//! [`encode_vertex_normals`] mirrors Blender 5.1's
//! `normals_split_custom_set_from_vertices` (`mesh_normals.cc`) for a smooth
//! triangle list with no sharp edges or faces: the same fans (split at
//! boundary, non-manifold and flipped-winding edges), the same fan normals
//! (angle-weighted face normals, `safe_acos_approx`) and the same space
//! definition, so Blender decodes exactly the given normals.
//! [`decode_corner_normals`] is the reverse (Blender's corner normals for given
//! data), used to verify an encoding. [`short2_data`] serializes the result.

/// Name of the custom-normal attribute (corner domain, `INT16_2D`).
pub const CUSTOM_NORMAL_ATTRIBUTE: &str = "custom_normal";

type V3 = [f32; 3];

/// `LNOR_SPACE_TRIGO_THRESHOLD`.
const TRIGO_THRESHOLD: f32 = 1.0 - 1e-4;
const PI2: f32 = std::f32::consts::PI * 2.0;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `math::normalize`: zero for tiny or NaN input.
fn normalize(v: V3) -> V3 {
    let len_sq = dot(v, v);
    if len_sq > 1.0e-35 {
        let len = len_sq.sqrt();
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0; 3]
    }
}

/// `math::safe_acos_approx` (Blender's fast acos, max error 4.5e-5). The
/// encoder must use it too: Blender's decoder scales by angles computed with it.
fn safe_acos_approx(x: f32) -> f32 {
    let f = x.abs();
    let m = if f < 1.0 { 1.0 - (1.0 - f) } else { 1.0 };
    let a = (1.0 - m).sqrt() * (1.570_796_3 + m * (-0.213_300_99 + m * (0.077_980_48 + m * -0.021_640_95)));
    if x < 0.0 { std::f32::consts::PI - a } else { a }
}

fn unit_float_to_short(v: f32) -> i16 {
    (v * i16::MAX as f32 + 0.5).floor() as i16
}

fn unit_short_to_float(v: i16) -> f32 {
    v as f32 / i16::MAX as f32
}

/// Blender's `CornerNormalSpace`; `ref_alpha == ref_beta == 0` marks it invalid.
#[derive(Debug, Clone, Copy, Default)]
struct Space {
    vec_lnor: V3,
    vec_ref: V3,
    vec_ortho: V3,
    ref_alpha: f32,
    ref_beta: f32,
}

/// `corner_fan_space_define`.
fn define_space(lnor: V3, vec_ref: V3, vec_other: V3, edge_vectors: &[V3]) -> Space {
    let mut space = Space::default();
    let dtp_ref = dot(vec_ref, lnor);
    let dtp_other = dot(vec_other, lnor);
    if dtp_ref.abs() >= TRIGO_THRESHOLD || dtp_other.abs() >= TRIGO_THRESHOLD {
        return space;
    }
    space.vec_lnor = lnor;
    space.ref_alpha = if edge_vectors.is_empty() {
        (safe_acos_approx(dot(vec_ref, lnor)) + safe_acos_approx(dot(vec_other, lnor))) / 2.0
    } else {
        let mut alpha = 0.0f32;
        for &v in edge_vectors {
            alpha += safe_acos_approx(dot(v, lnor));
        }
        alpha / edge_vectors.len() as f32
    };
    space.vec_ref = normalize(sub(vec_ref, scale(lnor, dtp_ref)));
    space.vec_ortho = normalize(cross(lnor, space.vec_ref));
    let other = normalize(sub(vec_other, scale(lnor, dtp_other)));
    let dtp = dot(space.vec_ref, other);
    space.ref_beta = if dtp < TRIGO_THRESHOLD {
        let beta = safe_acos_approx(dtp);
        if dot(space.vec_ortho, other) < 0.0 { PI2 - beta } else { beta }
    } else {
        PI2
    };
    space
}

/// `corner_space_custom_normal_to_data`.
fn normal_to_data(space: &Space, custom: V3) -> [i16; 2] {
    let near_default = (0..3).all(|i| (space.vec_lnor[i] - custom[i]).abs() <= 1e-4);
    if custom == [0.0; 3] || near_default {
        return [0, 0];
    }
    let cos_alpha = dot(space.vec_lnor, custom);
    let alpha = safe_acos_approx(cos_alpha);
    let x = if alpha > space.ref_alpha {
        unit_float_to_short(-(PI2 - alpha) / (PI2 - space.ref_alpha))
    } else {
        unit_float_to_short(alpha / space.ref_alpha)
    };
    let vec = normalize(add(scale(space.vec_lnor, -cos_alpha), custom));
    let cos_beta = dot(space.vec_ref, vec);
    let y = if cos_beta < TRIGO_THRESHOLD {
        let mut beta = safe_acos_approx(cos_beta);
        if dot(space.vec_ortho, vec) < 0.0 {
            beta = PI2 - beta;
        }
        if beta > space.ref_beta {
            unit_float_to_short(-(PI2 - beta) / (PI2 - space.ref_beta))
        } else {
            unit_float_to_short(beta / space.ref_beta)
        }
    } else {
        0
    };
    [x, y]
}

/// `corner_space_custom_data_to_normal`.
fn data_to_normal(space: &Space, data: [i16; 2]) -> V3 {
    if data[0] == 0 || space.ref_alpha == 0.0 || space.ref_beta == 0.0 {
        return space.vec_lnor;
    }
    let alphafac = unit_short_to_float(data[0]);
    let alpha = if alphafac > 0.0 { space.ref_alpha } else { PI2 - space.ref_alpha } * alphafac;
    let betafac = unit_short_to_float(data[1]);
    let mut out = scale(space.vec_lnor, alpha.cos());
    if betafac == 0.0 {
        out = add(out, scale(space.vec_ref, alpha.sin()));
    } else {
        let sinalpha = alpha.sin();
        let beta = if betafac > 0.0 { space.ref_beta } else { PI2 - space.ref_beta } * betafac;
        out = add(out, scale(space.vec_ref, sinalpha * beta.cos()));
        out = add(out, scale(space.vec_ortho, sinalpha * beta.sin()));
    }
    out
}

/// One face corner around the current vertex (`VertCornerInfo`); edges are
/// indices into the vertex's local edge list.
#[derive(Debug, Clone, Copy)]
struct CornerInfo {
    face: usize,
    corner: usize,
    edge_prev: usize,
    edge_next: usize,
}

/// How an edge around the current vertex connects its corners (`VertEdgeInfo`).
#[derive(Debug, Clone, Copy)]
enum EdgeInfo {
    Uninitialized,
    One { corner: usize, towards_vert: bool },
    Two(usize, usize),
    Sharp,
}

/// `add_corner_to_edge` with no sharp edges: a second corner with the same
/// winding direction, or a third corner, makes the edge sharp.
fn add_corner_to_edge(info: &mut EdgeInfo, local_corner: usize, towards_vert: bool) {
    *info = match *info {
        EdgeInfo::Uninitialized => EdgeInfo::One { corner: local_corner, towards_vert },
        EdgeInfo::One { corner, towards_vert: first } if first != towards_vert => {
            EdgeInfo::Two(corner, local_corner)
        }
        _ => EdgeInfo::Sharp,
    };
}

/// `traverse_fan_local_corners`, bounded so malformed input cannot loop.
fn traverse_fan(corners: &[CornerInfo], edges: &[EdgeInfo], start: usize, fan: &mut Vec<usize>) {
    fan.clear();
    fan.push(start);
    let limit = corners.len();
    let mut current = start;
    let mut edge = corners[current].edge_next;
    let mut cyclic = false;
    while let EdgeInfo::Two(a, b) = edges[edge] {
        current = if a == current { b } else { a };
        if current == start {
            cyclic = true;
            break;
        }
        if fan.len() >= limit {
            break;
        }
        fan.push(current);
        edge = corners[current].edge_next;
    }
    fan.reverse();
    if cyclic {
        // Blender rotates a cyclic fan so its smallest corner index comes first.
        let first = (0..fan.len()).min_by_key(|&i| corners[fan[i]].corner).unwrap_or(0);
        fan.rotate_left(first);
        return;
    }
    current = start;
    edge = corners[current].edge_prev;
    while let EdgeInfo::Two(a, b) = edges[edge] {
        current = if current == a { b } else { a };
        if fan.len() >= limit {
            break;
        }
        fan.push(current);
        edge = corners[current].edge_prev;
    }
}

fn validate(positions: &[V3], indices: &[u32], corner_values: usize) -> Result<(), String> {
    if indices.len() % 3 != 0 {
        return Err(format!("custom normals: {} indices is not a triangle list", indices.len()));
    }
    if corner_values != indices.len() {
        return Err(format!("custom normals: {corner_values} corner values for {} corners", indices.len()));
    }
    if let Some(&bad) = indices.iter().find(|&&i| i as usize >= positions.len()) {
        return Err(format!("custom normals: index {bad} out of range for {} vertices", positions.len()));
    }
    Ok(())
}

/// Find `other` in the vertex's local edge list, adding it if new
/// (`index_of_or_add`).
fn local_edge(local_edges: &mut Vec<u32>, other: u32) -> usize {
    match local_edges.iter().position(|&e| e == other) {
        Some(index) => index,
        None => {
            local_edges.push(other);
            local_edges.len() - 1
        }
    }
}

/// Walk every fan of a smooth triangle list (as `normals_calc_corners`),
/// calling `visit(fan_corners, space)` with global corner indices.
fn for_each_fan(positions: &[V3], indices: &[u32], mut visit: impl FnMut(&[usize], &Space)) {
    let face_normals: Vec<V3> = indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]];
            let n = normalize(cross(sub(a, b), sub(b, c)));
            if n == [0.0; 3] { [0.0, 0.0, 1.0] } else { n }
        })
        .collect();
    // vert -> faces in ascending order (Blender's `vert_to_face_map`; a face
    // using a vertex twice is listed twice).
    let mut offsets = vec![0usize; positions.len() + 1];
    for &v in indices {
        offsets[v as usize + 1] += 1;
    }
    for i in 0..positions.len() {
        offsets[i + 1] += offsets[i];
    }
    let mut fill = offsets.clone();
    let mut vert_faces = vec![0usize; indices.len()];
    for (corner, &v) in indices.iter().enumerate() {
        vert_faces[fill[v as usize]] = corner / 3;
        fill[v as usize] += 1;
    }

    let (mut corners, mut local_edges, mut edges, mut edge_dirs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut visited, mut fan, mut fan_corners, mut fan_edge_dirs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for vert in 0..positions.len() {
        let faces = &vert_faces[offsets[vert]..offsets[vert + 1]];
        if faces.is_empty() {
            continue;
        }
        corners.clear();
        local_edges.clear();
        for &face in faces {
            let start = face * 3;
            let k = (0..3).find(|&k| indices[start + k] as usize == vert).unwrap_or(0);
            let (prev, next) = (start + (k + 2) % 3, start + (k + 1) % 3);
            let edge_prev = local_edge(&mut local_edges, indices[prev]);
            let edge_next = local_edge(&mut local_edges, indices[next]);
            corners.push(CornerInfo { face, corner: start + k, edge_prev, edge_next });
        }
        edges.clear();
        edges.resize(local_edges.len(), EdgeInfo::Uninitialized);
        for (local, info) in corners.iter().enumerate() {
            // The "previous" edge winds towards the vertex, the "next" one away.
            add_corner_to_edge(&mut edges[info.edge_prev], local, true);
            add_corner_to_edge(&mut edges[info.edge_next], local, false);
        }
        let origin = positions[vert];
        edge_dirs.clear();
        edge_dirs.extend(local_edges.iter().map(|&other| normalize(sub(positions[other as usize], origin))));

        visited.clear();
        visited.resize(corners.len(), false);
        let mut visited_count = 0;
        let mut start = 0;
        loop {
            traverse_fan(&corners, &edges, start, &mut fan);
            let fan_normal = if fan.len() == 1 {
                face_normals[corners[fan[0]].face]
            } else {
                let mut sum = [0.0f32; 3];
                for &local in &fan {
                    let info = corners[local];
                    let factor = safe_acos_approx(dot(edge_dirs[info.edge_prev], edge_dirs[info.edge_next]));
                    sum = add(sum, scale(face_normals[info.face], factor));
                }
                normalize(sum)
            };
            let edge_first = corners[fan[0]].edge_next;
            let edge_last = corners[fan[fan.len() - 1]].edge_prev;
            fan_edge_dirs.clear();
            if fan.len() > 1 {
                fan_edge_dirs.extend(fan.iter().map(|&local| edge_dirs[corners[local].edge_next]));
                if edge_last != edge_first {
                    fan_edge_dirs.push(edge_dirs[edge_last]);
                }
            }
            let space = define_space(fan_normal, edge_dirs[edge_first], edge_dirs[edge_last], &fan_edge_dirs);
            fan_corners.clear();
            fan_corners.extend(fan.iter().map(|&local| corners[local].corner));
            visit(&fan_corners, &space);

            for &local in &fan {
                if !visited[local] {
                    visited[local] = true;
                    visited_count += 1;
                }
            }
            if visited_count >= corners.len() {
                break;
            }
            while visited[start] {
                start += 1;
            }
        }
    }
}

/// Encode unit per-vertex normals of a smooth triangle list (`indices`, three
/// per face) as `custom_normal` corner data. Each fan encodes the average of
/// its corners' normals, as Blender does; normals are normalized first (a zero
/// normal keeps Blender's default). Errors on a non-triangle index count, an
/// out-of-range index, or a normal count that is not one per vertex.
pub fn encode_vertex_normals(
    positions: &[[f32; 3]],
    indices: &[u32],
    vertex_normals: &[[f32; 3]],
) -> Result<Vec<[i16; 2]>, String> {
    if vertex_normals.len() != positions.len() {
        return Err(format!(
            "custom normals: {} normals for {} vertices",
            vertex_normals.len(),
            positions.len()
        ));
    }
    validate(positions, indices, indices.len())?;
    let normals: Vec<V3> = vertex_normals.iter().map(|&n| normalize(n)).collect();
    let mut out = vec![[0i16; 2]; indices.len()];
    for_each_fan(positions, indices, |fan, space| {
        let custom = if fan.len() < 2 {
            normals[indices[fan[0]] as usize]
        } else {
            let sum = fan.iter().fold([0.0f32; 3], |acc, &c| add(acc, normals[indices[c] as usize]));
            scale(sum, 1.0 / fan.len() as f32)
        };
        let data = normal_to_data(space, custom);
        for &corner in fan {
            out[corner] = data;
        }
    });
    Ok(out)
}

/// Decode `custom_normal` corner data to the corner normals Blender 5.1
/// computes for a smooth triangle list (each fan decodes the average of its
/// corners' data). Errors like [`encode_vertex_normals`].
pub fn decode_corner_normals(
    positions: &[[f32; 3]],
    indices: &[u32],
    custom: &[[i16; 2]],
) -> Result<Vec<[f32; 3]>, String> {
    validate(positions, indices, custom.len())?;
    let mut out = vec![[0.0f32; 3]; indices.len()];
    for_each_fan(positions, indices, |fan, space| {
        // int2 average with C++ integer division (truncation towards zero).
        let n = fan.len() as i32;
        let sum = fan
            .iter()
            .fold([0i32; 2], |acc, &c| [acc[0] + custom[c][0] as i32, acc[1] + custom[c][1] as i32]);
        let normal = data_to_normal(space, [(sum[0] / n) as i16, (sum[1] / n) as i16]);
        for &corner in fan {
            out[corner] = normal;
        }
    });
    Ok(out)
}

/// Raw little-endian bytes of `short2` corner data (the attribute array's payload).
pub fn short2_data(values: &[[i16; 2]]) -> Vec<u8> {
    values.iter().flat_map(|v| v.iter().flat_map(|s| s.to_le_bytes())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        indices: Vec<u32>,
        custom: Vec<[i16; 2]>,
        corner_normals: Vec<[f32; 3]>,
    }

    /// The Blender 5.1 reference written by `tests/data/custom_normals_blender51.py`
    /// (`normals_split_custom_set_from_vertices` on a mesh with cyclic and open
    /// fans, a split seam, a non-manifold fin, flipped winding and a zero-area face).
    fn fixture() -> Fixture {
        let text = include_str!("../tests/data/custom_normals_blender51.txt");
        let mut f = Fixture { positions: vec![], normals: vec![], indices: vec![], custom: vec![], corner_normals: vec![] };
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut parts = line.split_whitespace();
            let tag = parts.next().unwrap();
            let nums: Vec<f64> = parts.map(|p| p.parse().unwrap()).collect();
            let v3 = |s: &[f64]| [s[0] as f32, s[1] as f32, s[2] as f32];
            match tag {
                "v" => {
                    f.positions.push(v3(&nums[0..3]));
                    f.normals.push(v3(&nums[3..6]));
                }
                "t" => f.indices.extend(nums.iter().map(|&i| i as u32)),
                "c" => {
                    f.custom.push([nums[0] as i16, nums[1] as i16]);
                    f.corner_normals.push(v3(&nums[2..5]));
                }
                other => panic!("unknown fixture tag {other}"),
            }
        }
        f
    }

    fn max_diff(a: [f32; 3], b: [f32; 3]) -> f32 {
        (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn encode_matches_blender_reference() {
        let f = fixture();
        let custom = encode_vertex_normals(&f.positions, &f.indices, &f.normals).unwrap();
        assert_eq!(custom.len(), f.indices.len());
        for (corner, (got, want)) in custom.iter().zip(&f.custom).enumerate() {
            assert_eq!(got, want, "corner {corner}: bit-exact with Blender");
        }
    }

    #[test]
    fn decode_matches_blender_corner_normals() {
        let f = fixture();
        let normals = decode_corner_normals(&f.positions, &f.indices, &f.custom).unwrap();
        for (corner, (got, want)) in normals.iter().zip(&f.corner_normals).enumerate() {
            assert!(max_diff(*got, *want) < 1e-5, "corner {corner}: {got:?} vs Blender {want:?}");
        }
    }

    #[test]
    fn round_trip_reproduces_vertex_normals() {
        let f = fixture();
        let custom = encode_vertex_normals(&f.positions, &f.indices, &f.normals).unwrap();
        let normals = decode_corner_normals(&f.positions, &f.indices, &custom).unwrap();
        // The zero-area triangle (last face) cannot carry a custom normal in
        // Blender either: its corner spaces are degenerate.
        let checked = f.indices.len() - 3;
        for corner in 0..checked {
            let want = f.normals[f.indices[corner] as usize];
            assert!(max_diff(normals[corner], want) < 1e-3, "corner {corner}: {:?} vs {want:?}", normals[corner]);
        }
    }

    #[test]
    fn normal_equal_to_the_default_encodes_as_zero() {
        // A flat quad: the default corner normal is +Z everywhere.
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let indices = [0, 1, 2, 0, 2, 3];
        let custom = encode_vertex_normals(&positions, &indices, &[[0.0, 0.0, 1.0]; 4]).unwrap();
        assert!(custom.iter().all(|c| *c == [0, 0]), "{custom:?}");
    }

    #[test]
    fn non_unit_normals_are_normalized() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let indices = [0, 1, 2, 0, 2, 3];
        let unit = [0.6f32, 0.0, 0.8];
        let scaled = [unit[0] * 3.0, unit[1] * 3.0, unit[2] * 3.0];
        let a = encode_vertex_normals(&positions, &indices, &[unit; 4]).unwrap();
        let b = encode_vertex_normals(&positions, &indices, &[scaled; 4]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn mismatched_lengths_are_errors() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        assert!(encode_vertex_normals(&positions, &[0, 1, 2], &[[0.0, 0.0, 1.0]; 2]).is_err());
        assert!(encode_vertex_normals(&positions, &[0, 1, 3], &[[0.0, 0.0, 1.0]; 3]).is_err());
        assert!(encode_vertex_normals(&positions, &[0, 1], &[[0.0, 0.0, 1.0]; 3]).is_err());
        assert!(decode_corner_normals(&positions, &[0, 1, 2], &[[0, 0]; 2]).is_err());
    }

    #[test]
    fn degenerate_repeated_vertex_does_not_panic() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let indices = [0, 0, 1, 0, 1, 2];
        let custom = encode_vertex_normals(&positions, &indices, &[[0.0, 0.0, 1.0]; 3]).unwrap();
        assert_eq!(custom.len(), 6);
    }

    #[test]
    fn short2_data_is_little_endian_pairs() {
        assert_eq!(short2_data(&[[1, -2], [0x1234, 0]]), vec![1, 0, 0xfe, 0xff, 0x34, 0x12, 0, 0]);
    }
}
