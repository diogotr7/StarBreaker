//! Normal transfer for morphed meshes (per-character head identity).
//!
//! [`transfer_normals`] carries the authored normals of a mesh onto a morphed
//! copy of it (same topology, displaced positions): each authored normal is
//! rotated by the shortest-arc rotation between the vertex's WELDED geometric
//! normal before and after the morph. Welding (vertices sharing a position,
//! e.g. split along UV seams) keeps seams smooth; rotating the authored normal
//! (instead of replacing it) keeps the artist's normal detail.

use std::collections::HashMap;

/// Position quantum for welding coincident vertices (metres).
const WELD_QUANTUM: f32 = 1e-5;

/// Rotate `authored` normals by the change in welded geometric normal between
/// `base` and `morphed` positions (same topology, triangle `indices`).
///
/// Vertices with a degenerate geometric normal keep their authored normal.
/// Output normals are unit length.
pub fn transfer_normals(
    base: &[[f32; 3]],
    morphed: &[[f32; 3]],
    indices: &[u32],
    authored: &[[f32; 3]],
) -> Vec<[f32; 3]> {
    debug_assert_eq!(base.len(), morphed.len());
    debug_assert_eq!(base.len(), authored.len());
    let weld = weld_groups(base);
    let n0 = welded_geometric_normals(base, indices, &weld);
    let n1 = welded_geometric_normals(morphed, indices, &weld);
    authored
        .iter()
        .enumerate()
        .map(|(v, &a)| match (n0[v], n1[v]) {
            (Some(from), Some(to)) => normalize(rotate_shortest_arc(from, to, a)).unwrap_or(a),
            _ => a,
        })
        .collect()
}

/// Map each vertex to a weld-group id (vertices at the same quantised position).
fn weld_groups(positions: &[[f32; 3]]) -> Vec<usize> {
    let mut ids: HashMap<[i64; 3], usize> = HashMap::new();
    positions
        .iter()
        .map(|p| {
            let key = p.map(|c| (c / WELD_QUANTUM).round() as i64);
            let next = ids.len();
            *ids.entry(key).or_insert(next)
        })
        .collect()
}

/// Area-weighted vertex normals, summed over each weld group.
fn welded_geometric_normals(
    positions: &[[f32; 3]],
    indices: &[u32],
    weld: &[usize],
) -> Vec<Option<[f32; 3]>> {
    let groups = weld.iter().copied().max().map_or(0, |m| m + 1);
    let mut acc = vec![[0.0f32; 3]; groups];
    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i as usize);
        if a >= positions.len() || b >= positions.len() || c >= positions.len() {
            continue;
        }
        let n = cross(sub(positions[b], positions[a]), sub(positions[c], positions[a]));
        for v in [a, b, c] {
            let g = &mut acc[weld[v]];
            g[0] += n[0];
            g[1] += n[1];
            g[2] += n[2];
        }
    }
    weld.iter().map(|&g| normalize(acc[g])).collect()
}

/// Rotate `v` by the shortest-arc rotation taking unit `from` onto unit `to`
/// (Rodrigues). Antiparallel inputs (undefined axis) leave `v` unchanged.
fn rotate_shortest_arc(from: [f32; 3], to: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    let axis = cross(from, to);
    let cos = dot(from, to);
    if cos <= -1.0 + 1e-6 {
        return v;
    }
    // v' = v·c + (k×v) + k(k·v)/(1+c), with k = from×to (|k| = sin).
    let kxv = cross(axis, v);
    let kdv = dot(axis, v) / (1.0 + cos);
    [0, 1, 2].map(|i| v[i] * cos + kxv[i] + axis[i] * kdv)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot(v, v).sqrt();
    (len > 1e-12).then(|| v.map(|c| c / len))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit quad in the XY plane (+Z facing), two triangles.
    fn quad() -> (Vec<[f32; 3]>, Vec<u32>) {
        (
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
    }

    #[test]
    fn unchanged_geometry_keeps_authored_normals() {
        let (p, idx) = quad();
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let authored = vec![[s, 0.0, s]; 4];
        let out = transfer_normals(&p, &p, &idx, &authored);
        assert!(out.iter().all(|n| close(*n, [s, 0.0, s])));
    }

    #[test]
    fn rigid_tilt_rotates_authored_normal() {
        let (p, idx) = quad();
        // Rotate the quad 90° about X: +Z facing becomes −Y facing.
        let tilted: Vec<[f32; 3]> = p.iter().map(|v| [v[0], -v[2], v[1]]).collect();
        let authored = vec![[0.0, 0.0, 1.0]; 4];
        let out = transfer_normals(&p, &tilted, &idx, &authored);
        assert!(out.iter().all(|n| close(*n, [0.0, -1.0, 0.0])), "{out:?}");
    }

    #[test]
    fn rigid_tilt_preserves_authored_detail_offset() {
        let (p, idx) = quad();
        let tilted: Vec<[f32; 3]> = p.iter().map(|v| [v[0], -v[2], v[1]]).collect();
        // Authored normal leans toward +X; after the X-axis rotation it still does.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let out = transfer_normals(&p, &tilted, &idx, &vec![[s, 0.0, s]; 4]);
        assert!(out.iter().all(|n| close(*n, [s, -s, 0.0])), "{out:?}");
    }

    #[test]
    fn seam_split_vertices_share_the_welded_normal() {
        // Two quads hinged along x = 1, the hinge vertices DUPLICATED (seam).
        let base = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0],
        ];
        let idx = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        // Fold the right quad up about the hinge.
        let mut morphed = base.clone();
        for v in [5, 6] {
            morphed[v] = [1.0, morphed[v][1], 1.0];
        }
        let authored = vec![[0.0, 0.0, 1.0]; 8];
        let out = transfer_normals(&base, &morphed, &idx, &authored);
        assert!(close(out[1], out[4]) && close(out[2], out[7]), "seam split: {out:?}");
        // Area-weighted: the 1/4 hinge corner touches one +Z triangle (area ½)
        // and both −X triangles (area ½ each) → normalize(−1, 0, ½).
        let e = normalize([-1.0, 0.0, 0.5]).unwrap();
        assert!(close(out[1], e), "{:?}", out[1]);
    }

    #[test]
    fn degenerate_triangles_keep_authored_normal() {
        let base = vec![[0.0; 3]; 3];
        let out = transfer_normals(&base, &base, &[0, 1, 2], &vec![[0.0, 1.0, 0.0]; 3]);
        assert!(out.iter().all(|n| close(*n, [0.0, 1.0, 0.0])));
    }
}
