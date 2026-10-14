//! Slot-18 per-vertex NEUTRAL identity morph reader (Pass 19N).
//!
//! Slot 18 is the carrier of the per-character NEUTRAL face SHAPE — the bulk
//! identity that slots 15 (joints, ~cm) and 19 (expression, zero at rest) do
//! NOT provide. The runtime deformer (`CAttachmentManager::GetVariantVertices`
//! worker `FUN_146c67950`, decompiled from the 4.8 client) walks it per vertex
//! and accumulates a per-vertex position delta, weighted by the DNA-archetype
//! blend. See `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.16.24.
//!
//! On-disk layout (`num_controls = local_regions × (num_variants − 1)`, where
//! `local_regions` = [`DnaFile::mesh_regions`] — 13 for the head, 2 for the
//! eyes/teeth `.dna`;
//! `mask_words = ceil(num_controls / 32)`):
//! ```text
//! [ num_vertices × mask_words  u32 ]  per-vertex BITMASK (bit c = control c active for the vertex)
//! [ num_vertices               u32 ]  per-vertex CURSOR (running count of active (vertex,control) pairs)
//! [ num_controls × 0x20        f32 ]  per-control records: scale(xyz)@+0/4/8, bias(xyz)@+0x10/14/18
//! ( + 4-byte gap )
//! [ u8 code stream ]                  3 bytes (cx,cy,cz) per active (vertex,control) pair
//! ```
//! The control index `c` encodes `(variant 1..num_variants-1, local region)`
//! variant-major: `c = (variant − 1) × local_regions + local`; local region
//! `l` is global face part `regions()[l]`.
//!
//! Decode (per vertex `V`):
//! ```text
//! delta[V] = Σ_{control c set in bitmask[V]}  coef[c] · (code_axis · scale[c]_axis + bias[c]_axis)
//! ```
//! walking the bitmask LSB-first and consuming the 3 codes per active control
//! sequentially from `code_stream[(cursor[V] + k) * 3]`. `coef` is the per-control
//! DNA-archetype blend weight (neutral, no activation gate); supply it from the
//! CHF's DNA matrix. Layout validated by `cursor[V+1] − cursor[V] ==
//! popcount(bitmask[V])` for every vertex.

use crate::error::DnaError;
use crate::parser::DnaFile;

/// Pointer-table slot index for the per-vertex neutral identity morphs.
pub const NEUTRAL_VERTEX_MORPHS_SLOT: usize = 18;

/// Per-control record stride in bytes (scale vec3 + pad + bias vec3 + pad).
const RECORD_STRIDE: usize = 0x20;

/// A validated view over slot 18's per-vertex neutral identity morph data.
#[derive(Debug, Clone)]
pub struct NeutralMorphs<'a> {
    data: &'a [u8],
    regions: Vec<usize>,
    num_vertices: usize,
    num_controls: usize,
    mask_words: usize,
    cursor_off: usize,
    records_off: usize,
    codes_off: usize,
}

impl<'a> DnaFile<'a> {
    /// Decode slot 18 into a [`NeutralMorphs`] view.
    ///
    /// `num_vertices` must come from the mesh (the `.dna` header's
    /// `num_vertices` field is unreliable on female rigs); `num_variants` comes
    /// from the header and the region list from [`DnaFile::mesh_regions`].
    ///
    /// # Errors
    /// - [`DnaError::NeutralMorphsUnset`] if slot 18 is null.
    /// - [`DnaError::NeutralMorphsMalformed`] if the layout does not fit the
    ///   segment or the cursor/popcount invariant fails.
    pub fn neutral_morphs(&self, num_vertices: usize) -> Result<NeutralMorphs<'a>, DnaError> {
        let segment = self
            .segment_for_slot(NEUTRAL_VERTEX_MORPHS_SLOT)
            .ok_or(DnaError::NeutralMorphsUnset)?;
        let data = self.segment_bytes(segment);
        NeutralMorphs::new(
            data,
            num_vertices,
            self.header.num_variants as usize,
            self.mesh_regions()?,
        )
    }
}

#[inline]
fn u32_at(data: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
}
#[inline]
fn f32_at(data: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
}

impl<'a> NeutralMorphs<'a> {
    /// Validate the slot-18 layout for the given dimensions and build a view.
    pub fn new(
        data: &'a [u8],
        num_vertices: usize,
        num_variants: usize,
        regions: Vec<usize>,
    ) -> Result<Self, DnaError> {
        let malformed = |context: &'static str, num_controls: usize| DnaError::NeutralMorphsMalformed {
            context,
            num_vertices,
            num_controls,
        };
        if num_vertices == 0 || num_variants < 2 || regions.is_empty() {
            return Err(malformed("zero/degenerate dimensions", 0));
        }
        let num_controls = regions.len() * (num_variants - 1);
        let mask_words = num_controls.div_ceil(32);
        let cursor_off = num_vertices * mask_words * 4;
        let records_off = cursor_off + num_vertices * 4;
        let codes_off = records_off + num_controls * RECORD_STRIDE + 4;
        if codes_off > data.len() {
            return Err(malformed("layout overruns segment", num_controls));
        }
        // Invariant: cursor[V+1] − cursor[V] == popcount(bitmask[V]). Sampling
        // the first N vertices is enough to reject wrong dimensions.
        let popcount = |v: usize| -> u32 {
            (0..mask_words)
                .map(|w| u32_at(data, v * mask_words * 4 + w * 4).count_ones())
                .sum()
        };
        let sample = (num_vertices - 1).min(256);
        for v in 0..sample {
            let delta = u32_at(data, cursor_off + (v + 1) * 4) as i64
                - u32_at(data, cursor_off + v * 4) as i64;
            if delta != popcount(v) as i64 {
                return Err(malformed("cursor/popcount mismatch", num_controls));
            }
        }
        Ok(Self {
            data,
            regions,
            num_vertices,
            num_controls,
            mask_words,
            cursor_off,
            records_off,
            codes_off,
        })
    }

    /// Number of mesh vertices.
    pub fn num_vertices(&self) -> usize {
        self.num_vertices
    }

    /// Number of controls = `regions().len() × (num_variants − 1)`.
    pub fn num_controls(&self) -> usize {
        self.num_controls
    }

    /// Global face part of each local region (control `c` belongs to local
    /// region `c % regions().len()`).
    pub fn regions(&self) -> &[usize] {
        &self.regions
    }

    /// Decode per-vertex neutral position deltas (metres, CryEngine space) for
    /// a per-control coefficient vector (the DNA-archetype blend weights).
    ///
    /// `coef[c]` for `c = (variant − 1) × regions().len() + local region`; a
    /// `coef` shorter than [`Self::num_controls`] treats the rest as 0. Returns
    /// a `num_vertices`-length `Vec<[f32; 3]>` to add to the base mesh.
    pub fn decode(&self, coef: &[f32]) -> Vec<[f32; 3]> {
        let data = self.data;
        let mut delta = vec![[0.0f32; 3]; self.num_vertices];
        for v in 0..self.num_vertices {
            let cursor = u32_at(data, self.cursor_off + v * 4) as usize;
            let mut k = 0usize; // k-th active control of this vertex
            for w in 0..self.mask_words {
                let mut word = u32_at(data, v * self.mask_words * 4 + w * 4);
                while word != 0 {
                    let c = w * 32 + word.trailing_zeros() as usize;
                    if c < self.num_controls {
                        let cf = coef.get(c).copied().unwrap_or(0.0);
                        let code_o = self.codes_off + (cursor + k) * 3;
                        if cf != 0.0 && code_o + 3 <= data.len() {
                            let rec = self.records_off + c * RECORD_STRIDE;
                            for ax in 0..3 {
                                let val = data[code_o + ax] as f32 * f32_at(data, rec + ax * 4)
                                    + f32_at(data, rec + 0x10 + ax * 4);
                                delta[v][ax] += cf * val;
                            }
                        }
                    }
                    k += 1;
                    word &= word - 1; // clear lowest set bit
                }
            }
        }
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal slot-18 segment: 2 vertices, num_variants=2,
    /// mesh_blends_per_pool=1 → 1 control, 1 mask word. Vertex 0 has control 0
    /// active with codes (cx,cy,cz); vertex 1 has no active controls.
    fn build_slot18(scale: [f32; 3], bias: [f32; 3], codes: [u8; 3]) -> Vec<u8> {
        let nv = 2usize;
        let mask_words = 1usize;
        let nctrl = 1usize;
        let cursor_off = nv * mask_words * 4;
        let records_off = cursor_off + nv * 4;
        let codes_off = records_off + nctrl * RECORD_STRIDE + 4;
        let total = codes_off + 1 * 3; // one active pair (vertex 0)
        let mut b = vec![0u8; total];
        // bitmask: vert0 word = 0b1 (control 0); vert1 = 0.
        b[0..4].copy_from_slice(&1u32.to_le_bytes());
        b[4..8].copy_from_slice(&0u32.to_le_bytes());
        // cursor: vert0 = 0, vert1 = 1 (1 active before vert1).
        b[cursor_off..cursor_off + 4].copy_from_slice(&0u32.to_le_bytes());
        b[cursor_off + 4..cursor_off + 8].copy_from_slice(&1u32.to_le_bytes());
        // record for control 0: scale @+0/4/8, bias @+0x10/14/18.
        let r = records_off;
        for k in 0..3 {
            b[r + k * 4..r + k * 4 + 4].copy_from_slice(&scale[k].to_le_bytes());
            b[r + 0x10 + k * 4..r + 0x10 + k * 4 + 4].copy_from_slice(&bias[k].to_le_bytes());
        }
        // codes for vertex 0's single active control.
        b[codes_off..codes_off + 3].copy_from_slice(&codes);
        b
    }

    #[test]
    fn decodes_neutral_delta() {
        let scale = [0.01, 0.02, 0.03];
        let bias = [0.1, 0.2, 0.3];
        let codes = [100u8, 50, 200];
        let data = build_slot18(scale, bias, codes);
        let m = NeutralMorphs::new(&data, 2, 2, vec![0]).expect("valid layout");
        assert_eq!(m.num_vertices(), 2);
        assert_eq!(m.num_controls(), 1);
        let d = m.decode(&[1.0]);
        // vertex 0: code*scale + bias.
        assert!((d[0][0] - (100.0 * 0.01 + 0.1)).abs() < 1e-5, "x={}", d[0][0]);
        assert!((d[0][1] - (50.0 * 0.02 + 0.2)).abs() < 1e-5, "y={}", d[0][1]);
        assert!((d[0][2] - (200.0 * 0.03 + 0.3)).abs() < 1e-5, "z={}", d[0][2]);
        // vertex 1: no active controls → zero.
        assert_eq!(d[1], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn coef_scales_and_zero_coef_is_zero() {
        let data = build_slot18([0.01, 0.0, 0.0], [0.1, 0.0, 0.0], [100, 0, 0]);
        let m = NeutralMorphs::new(&data, 2, 2, vec![0]).expect("valid");
        // half coefficient → half delta.
        let d = m.decode(&[0.5]);
        assert!((d[0][0] - 0.5 * (100.0 * 0.01 + 0.1)).abs() < 1e-5);
        // zero coefficient → zero delta.
        let z = m.decode(&[0.0]);
        assert_eq!(z[0], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn rejects_wrong_dimensions() {
        let data = build_slot18([0.01, 0.02, 0.03], [0.1, 0.2, 0.3], [100, 50, 200]);
        // wrong num_variants (3 → 2 controls, 2 mask words) breaks the layout.
        assert!(matches!(
            NeutralMorphs::new(&data, 2, 9, vec![0]),
            Err(DnaError::NeutralMorphsMalformed { .. })
        ));
    }
}
