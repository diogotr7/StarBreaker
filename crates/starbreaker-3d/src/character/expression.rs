//! Per-character expression blendshapes from slot 19.
//!
//! Slot 19 cell `(v, b)` is archetype `v`'s own blendshape `b` (variant 0 =
//! the base head, equal to its `BShapesGpu`). A character's blendshape is the
//! same region-masked convex archetype blend that slot 18 applies to the
//! neutral shape:
//!
//! ```text
//! blend_b[vtx] = Σ_v fold[vtx][v] · T_v[b][vtx]
//! fold[vtx][v] = (Σ_fp regionW[vtx][fp]·matrix[v][fp]) / norm[vtx]
//! norm[vtx]    =  Σ_fp regionW[vtx][fp]·(Σ_active v matrix[v][fp])
//! ```
//!
//! `regionW` (slot 22) is a per-vertex partition of unity and `matrix` is
//! per-region normalised, so `norm` is 1 whenever every region carries CHF
//! blends; dividing by it keeps the fold convex when a CHF leaves a region
//! empty.
//!
//! [`fold_blendshape_slot19`] returns `blend_b − T_0[b]`, the per-character
//! correction to add to the base head's `BShapesGpu` target `b`.

use starbreaker_dna::ExpressionMorphs;

use super::dna_matrix::DnaMatrix2D;

/// Per-vertex convex archetype fold weights for one character.
struct VariantFold<'a> {
    region_weights: &'a [f32],
    regions: usize,
    matrix: &'a DnaMatrix2D,
    active: Vec<usize>,
    norm: Vec<f32>,
}

impl<'a> VariantFold<'a> {
    fn new(region_weights: &'a [f32], regions: usize, matrix: &'a DnaMatrix2D, num_vertices: usize) -> Self {
        let active: Vec<usize> = (0..matrix.num_variants())
            .filter(|&v| (0..regions).any(|fp| matrix.cell(v, fp) != 0.0))
            .collect();
        let colsum: Vec<f32> = (0..regions)
            .map(|fp| active.iter().map(|&v| matrix.cell(v, fp)).sum())
            .collect();
        let norm = (0..num_vertices)
            .map(|vi| {
                (0..regions)
                    .map(|fp| region_weights.get(vi * regions + fp).copied().unwrap_or(0.0) * colsum[fp])
                    .sum::<f32>()
            })
            .collect();
        Self { region_weights, regions, matrix, active, norm }
    }

    /// `fold[vtx][v]`, or `None` when the vertex has no region coverage.
    fn weight(&self, vtx: usize, v: usize) -> Option<f32> {
        let n = *self.norm.get(vtx)?;
        if n <= 0.0 {
            return None;
        }
        let raw: f32 = (0..self.regions)
            .map(|fp| self.region_weights[vtx * self.regions + fp] * self.matrix.cell(v, fp))
            .sum();
        Some(raw / n)
    }

    /// Accumulate `scale · Σ_v fold_v · T_v[b]` into `out`.
    fn accumulate(&self, morphs: &ExpressionMorphs, b: usize, scale: f32, out: &mut [[f32; 3]]) {
        for &v in &self.active {
            let Some(rec) = morphs.record(v, b) else { continue };
            for (vtx, d) in rec.entries() {
                let vi = vtx as usize;
                if vi >= out.len() {
                    continue;
                }
                let Some(w) = self.weight(vi, v) else { continue };
                let s = w * scale;
                for k in 0..3 {
                    out[vi][k] += s * d[k];
                }
            }
        }
    }
}

/// Per-character correction for blendshape `b`: `blend_b − T_0[b]`, to ADD to
/// the base head's `BShapesGpu` target `b` (variant 0 is the base head). No
/// activation factor — the activation is the morph-target weight downstream,
/// so the base shape and its per-character correction play in lockstep.
pub fn fold_blendshape_slot19(
    morphs: &ExpressionMorphs,
    region_weights: &[f32],
    regions: usize,
    matrix: &DnaMatrix2D,
    b: usize,
    num_vertices: usize,
) -> Vec<[f32; 3]> {
    let fold = VariantFold::new(region_weights, regions, matrix, num_vertices);
    let mut out = vec![[0.0f32; 3]; num_vertices];
    fold.accumulate(morphs, b, 1.0, &mut out);
    if let Some(base) = morphs.record(0, b) {
        for (vtx, d) in base.entries() {
            let vi = vtx as usize;
            // Uncovered vertices carry no blend, so they get no correction.
            if vi < num_vertices && fold.norm[vi] > 0.0 {
                for k in 0..3 {
                    out[vi][k] -= d[k];
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use starbreaker_dna::{DnaFile, EXPRESSION_MORPHS_SLOT, MAGIC, POINTER_TABLE_OFFSET};

    const MIN_FILE_SIZE: usize = 0x140;

    /// `(variant, blendshape, [(vertex, delta)])` with deltas exactly
    /// representable as `bias + q · scale` (scale 1 mm, bias −0.1 m).
    type Cell = (usize, usize, Vec<(u32, [f32; 3])>);

    const SCALE: f32 = 0.001;
    const BIAS: f32 = -0.1;

    fn code(x: f32) -> u8 {
        ((x - BIAS) / SCALE).round() as u8
    }

    fn build_slot19(num_variants: u16, num_bs: usize, cells: &[Cell]) -> Vec<u8> {
        let region_a = num_variants as usize * num_bs * 32;
        let mut blocks = Vec::new();
        let mut recs = Vec::new();
        for (v, b, entries) in cells {
            let off = region_a + blocks.len();
            let start = entries[0].0;
            let mut prev = start;
            for (vtx, d) in entries {
                blocks.extend_from_slice(&[(vtx - prev) as u8, code(d[0]), code(d[1]), code(d[2])]);
                prev = *vtx;
            }
            recs.push((v * num_bs + b, off, (start << 16) | entries.len() as u32));
        }
        let body = MIN_FILE_SIZE;
        let mut buf = vec![0u8; body + region_a + blocks.len()];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&num_variants.to_le_bytes());
        let slot_off = POINTER_TABLE_OFFSET + EXPRESSION_MORPHS_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body as u64).to_le_bytes());
        buf[body..body + 4].copy_from_slice(&(region_a as u32).to_le_bytes());
        buf[body + region_a..].copy_from_slice(&blocks);
        for (r, off, packed) in recs {
            let rec = body + r * 32;
            buf[rec..rec + 4].copy_from_slice(&(off as u32).to_le_bytes());
            buf[rec + 4..rec + 8].copy_from_slice(&packed.to_le_bytes());
            for k in 0..3 {
                buf[rec + 8 + k * 4..rec + 12 + k * 4].copy_from_slice(&SCALE.to_le_bytes());
                buf[rec + 0x14 + k * 4..rec + 0x18 + k * 4].copy_from_slice(&BIAS.to_le_bytes());
            }
        }
        buf
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
    }

    /// Variants: 0 = base, 1 and 2 = archetypes. 1 blendshape (index 0).
    /// Vertex 0 lies wholly in region 0, vertex 1 wholly in region 1.
    fn two_region_fixture() -> Vec<u8> {
        build_slot19(
            3,
            1,
            &[
                (0, 0, vec![(0, [0.010, 0.0, 0.0]), (1, [0.010, 0.0, 0.0])]),
                (1, 0, vec![(0, [0.020, 0.0, 0.0]), (1, [0.020, 0.0, 0.0])]),
                (2, 0, vec![(0, [0.0, 0.030, 0.0]), (1, [0.0, 0.030, 0.0])]),
            ],
        )
    }

    const REGION_W: [f32; 4] = [1.0, 0.0, 0.0, 1.0]; // [vtx][region], 2 × 2

    /// The character's blendshape `b`: the convex region-masked blend of the
    /// archetypes' slot-19 blendshape `b`.
    fn blend_blendshape_slot19(
        morphs: &ExpressionMorphs,
        region_weights: &[f32],
        regions: usize,
        matrix: &DnaMatrix2D,
        b: usize,
        num_vertices: usize,
    ) -> Vec<[f32; 3]> {
        let fold = VariantFold::new(region_weights, regions, matrix, num_vertices);
        let mut out = vec![[0.0f32; 3]; num_vertices];
        fold.accumulate(morphs, b, 1.0, &mut out);
        out
    }

    #[test]
    fn region_masks_pick_each_regions_archetype() {
        let buf = two_region_fixture();
        let dna = DnaFile::read(&buf).unwrap();
        let morphs = dna.expression_morphs().unwrap();
        let mut m = DnaMatrix2D::zeros(3, 2);
        *m.cell_mut(1, 0) = 1.0; // region 0 ← archetype 1
        *m.cell_mut(2, 1) = 1.0; // region 1 ← archetype 2
        let blend = blend_blendshape_slot19(&morphs, &REGION_W, 2, &m, 0, 2);
        assert!(close(blend[0], [0.020, 0.0, 0.0]), "{:?}", blend[0]);
        assert!(close(blend[1], [0.0, 0.030, 0.0]), "{:?}", blend[1]);
    }

    #[test]
    fn convex_mix_within_a_region() {
        let buf = two_region_fixture();
        let dna = DnaFile::read(&buf).unwrap();
        let morphs = dna.expression_morphs().unwrap();
        let mut m = DnaMatrix2D::zeros(3, 2);
        for r in 0..2 {
            *m.cell_mut(1, r) = 0.25;
            *m.cell_mut(2, r) = 0.75;
        }
        let blend = blend_blendshape_slot19(&morphs, &REGION_W, 2, &m, 0, 2);
        assert!(close(blend[0], [0.005, 0.0225, 0.0]), "{:?}", blend[0]);
    }

    #[test]
    fn correction_is_blend_minus_base() {
        let buf = two_region_fixture();
        let dna = DnaFile::read(&buf).unwrap();
        let morphs = dna.expression_morphs().unwrap();
        let mut m = DnaMatrix2D::zeros(3, 2);
        *m.cell_mut(1, 0) = 1.0;
        *m.cell_mut(1, 1) = 1.0;
        let corr = fold_blendshape_slot19(&morphs, &REGION_W, 2, &m, 0, 2);
        // T_1 − T_0 = (0.020 − 0.010, 0, 0).
        assert!(close(corr[0], [0.010, 0.0, 0.0]), "{:?}", corr[0]);
        assert!(close(corr[1], [0.010, 0.0, 0.0]), "{:?}", corr[1]);
    }

    #[test]
    fn base_character_has_zero_correction() {
        let buf = two_region_fixture();
        let dna = DnaFile::read(&buf).unwrap();
        let morphs = dna.expression_morphs().unwrap();
        let mut m = DnaMatrix2D::zeros(3, 2);
        *m.cell_mut(0, 0) = 1.0;
        *m.cell_mut(0, 1) = 1.0;
        let corr = fold_blendshape_slot19(&morphs, &REGION_W, 2, &m, 0, 2);
        assert!(corr.iter().all(|c| close(*c, [0.0; 3])), "{corr:?}");
    }

    #[test]
    fn uncovered_vertex_gets_nothing() {
        let buf = two_region_fixture();
        let dna = DnaFile::read(&buf).unwrap();
        let morphs = dna.expression_morphs().unwrap();
        let mut m = DnaMatrix2D::zeros(3, 2);
        *m.cell_mut(1, 0) = 1.0; // region 1 has no CHF blend
        let rw = [1.0, 0.0, 0.0, 1.0];
        let corr = fold_blendshape_slot19(&morphs, &rw, 2, &m, 0, 2);
        assert!(close(corr[0], [0.010, 0.0, 0.0]));
        assert!(close(corr[1], [0.0; 3]), "vertex in an empty region: {:?}", corr[1]);
    }
}
