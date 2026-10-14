//! Head-surface wrap field for `WD_*` wrap-deformed attachments.
//!
//! Hair, brows, lashes, beard, scalp and eye-details carry `WD_Elastic*`
//! deformers: the engine fits them to the per-character head SURFACE rather
//! than to joints. [`HeadDeltaField`] reproduces that for the neutral identity:
//! it spreads the head's per-vertex identity deltas (slot 18) into space by
//! Shepard (inverse-square-distance) interpolation over the head vertices in
//! the 3×3×3 grid cells around a query point, falling back to the nearest
//! head vertex when nothing is in reach. The field is smooth, so hair cards
//! standing off the scalp move coherently instead of snapping between the
//! deltas of whichever single vertex happens to be nearest.

use std::collections::HashMap;

/// Grid cell size (metres): the neighbourhood radius of the interpolation.
const CELL: f32 = 0.03;

/// The head's identity displacement as a smooth field over space.
pub struct HeadDeltaField<'a> {
    base: &'a [[f32; 3]],
    deltas: &'a [[f32; 3]],
    origin: [f32; 3],
    grid: HashMap<[i32; 3], Vec<u32>>,
}

impl<'a> HeadDeltaField<'a> {
    /// `base` = head vertex positions BEFORE the identity, `deltas` = the
    /// identity displacement of each (same length).
    pub fn new(base: &'a [[f32; 3]], deltas: &'a [[f32; 3]]) -> Self {
        debug_assert_eq!(base.len(), deltas.len());
        let origin = base.iter().fold([f32::MAX; 3], |m, p| [0, 1, 2].map(|k| m[k].min(p[k])));
        let mut grid: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        let mut field = Self { base, deltas, origin, grid: HashMap::new() };
        for (i, p) in base.iter().enumerate() {
            grid.entry(field.cell(*p)).or_default().push(i as u32);
        }
        field.grid = grid;
        field
    }

    fn cell(&self, p: [f32; 3]) -> [i32; 3] {
        [0, 1, 2].map(|k| ((p[k] - self.origin[k]) / CELL).floor() as i32)
    }

    /// The identity displacement at point `p` (metres, the head's space).
    pub fn sample(&self, p: [f32; 3]) -> [f32; 3] {
        if self.base.is_empty() {
            return [0.0; 3];
        }
        let c = self.cell(p);
        let (mut acc, mut wsum) = ([0.0f64; 3], 0.0f64);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let Some(ids) = self.grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else {
                        continue;
                    };
                    for &i in ids {
                        let w = 1.0 / (dist2(p, self.base[i as usize]) as f64).max(1e-12);
                        let d = self.deltas[i as usize];
                        for k in 0..3 {
                            acc[k] += w * d[k] as f64;
                        }
                        wsum += w;
                    }
                }
            }
        }
        if wsum > 0.0 {
            return acc.map(|a| (a / wsum) as f32);
        }
        // Nothing within reach: take the nearest head vertex's delta.
        let nearest = (0..self.base.len())
            .min_by(|&a, &b| dist2(p, self.base[a]).total_cmp(&dist2(p, self.base[b])))
            .unwrap();
        self.deltas[nearest]
    }

    /// Displace `positions` in place by the field.
    pub fn apply(&self, positions: &mut [[f32; 3]]) {
        for p in positions {
            let d = self.sample(*p);
            for k in 0..3 {
                p[k] += d[k];
            }
        }
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < eps)
    }

    fn lattice() -> Vec<[f32; 3]> {
        let mut v = Vec::new();
        for x in 0..5 {
            for y in 0..5 {
                v.push([x as f32 * 0.01, y as f32 * 0.01, 0.0]);
            }
        }
        v
    }

    #[test]
    fn constant_field_moves_everything_equally() {
        let base = lattice();
        let deltas = vec![[0.001, -0.002, 0.003]; base.len()];
        let f = HeadDeltaField::new(&base, &deltas);
        for p in [[0.02, 0.02, 0.005], [0.0, 0.04, -0.01], [0.013, 0.007, 0.02]] {
            assert!(close(f.sample(p), [0.001, -0.002, 0.003], 1e-7));
        }
    }

    #[test]
    fn on_a_head_vertex_takes_its_delta() {
        let base = lattice();
        let deltas: Vec<[f32; 3]> = (0..base.len()).map(|i| [i as f32 * 1e-3, 0.0, 0.0]).collect();
        let f = HeadDeltaField::new(&base, &deltas);
        assert!(close(f.sample(base[12]), deltas[12], 1e-6), "{:?}", f.sample(base[12]));
    }

    #[test]
    fn far_point_falls_back_to_nearest_vertex() {
        let base = lattice();
        let deltas: Vec<[f32; 3]> = (0..base.len()).map(|i| [0.0, i as f32 * 1e-3, 0.0]).collect();
        let f = HeadDeltaField::new(&base, &deltas);
        // 0.5 m above the far corner: outside every neighbouring cell.
        let far = [0.04, 0.04, 0.5];
        assert!(close(f.sample(far), deltas[24], 1e-9));
    }

    #[test]
    fn field_is_smooth_between_vertices() {
        let base = lattice();
        // A ramp along x.
        let deltas: Vec<[f32; 3]> = base.iter().map(|p| [p[0] * 0.1, 0.0, 0.0]).collect();
        let f = HeadDeltaField::new(&base, &deltas);
        let a = f.sample([0.0200, 0.02, 0.004]);
        let b = f.sample([0.0201, 0.02, 0.004]);
        assert!((a[0] - b[0]).abs() < 1e-5, "{a:?} vs {b:?}");
        // and stays within the ramp's range
        assert!(a[0] > 0.0 && a[0] < 0.004);
    }

    #[test]
    fn apply_displaces_in_place() {
        let base = lattice();
        let deltas = vec![[0.0, 0.0, 0.01]; base.len()];
        let f = HeadDeltaField::new(&base, &deltas);
        let mut pts = vec![[0.02, 0.02, 0.0]];
        f.apply(&mut pts);
        assert!(close(pts[0], [0.02, 0.02, 0.01], 1e-7));
    }
}
