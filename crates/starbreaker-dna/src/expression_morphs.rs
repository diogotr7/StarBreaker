//! Slot-19 per-(variant × blendshape) per-vertex blendshape library reader.
//!
//! Slot 19 is the vertex analogue of slot 31's joint deltas: for each identity
//! variant (archetype head) and each RigLogic blendshape, that ARCHETYPE'S OWN
//! blendshape target as sparse per-vertex position deltas. Cell `(v, b)`
//! reproduces `BShapesGpu[b]` of archetype `v`'s `<name>_head.skinm` (variant 0
//! = the protos base head) to ≤ 0.012 mm — the expression counterpart of slot
//! 18 (neutral shape). It is EXPRESSION data: blendshape 0 / CTL_PSD is empty.
//!
//! On-disk layout (master fixture: 48,471,624 bytes, 61 variants, 5584 verts):
//! - `dword[0]` = byte length of **Region A** (= `0xEBE60`). The data region
//!   (Region B) follows.
//! - **Region A** = `num_variants × num_blendshapes` records of 32 bytes,
//!   **variant-major**: record `r → variant = r / num_blendshapes,
//!   blendshape = r % num_blendshapes` (495 = 205 appearance + 290
//!   corrective on the master). Record fields:
//!   - `+0x00` u32: byte offset (from the slot start) of the entry block;
//!   - `+0x04` u32: `lo16` = entry count, `hi16` = START VERTEX of the walk
//!     (`+0x04 == 0` ⇒ empty cell);
//!   - `+0x08` vec3 f32: per-axis dequantisation SCALE (= range / 255);
//!   - `+0x14` vec3 f32: per-axis BIAS (= the cell's per-axis minimum).
//! - **Region B** entry (4 bytes): `[u8 vertex stride][u8 qx][u8 qy][u8 qz]`.
//!   The vertex index starts at `hi16` and is advanced by each stride (before
//!   use); the delta is `bias + q · scale` per axis.
//!
//! An earlier reading (vertex walk from 0, bytes 1..4 as a u24 weight scaling
//! the `+0x14` vec3) produced misplaced, wrong-shaped deltas.

use crate::error::DnaError;
use crate::parser::DnaFile;

/// Pointer-table slot index for the per-(variant × blendshape) vertex morphs.
pub const EXPRESSION_MORPHS_SLOT: usize = 19;

/// Region-A record stride in bytes.
const RECORD_STRIDE: usize = 32;

/// A decoded view over slot 19's variant-major `(variant × blendshape)` grid.
#[derive(Debug, Clone, Copy)]
pub struct ExpressionMorphs<'a> {
    data: &'a [u8],
    num_variants: usize,
    num_blendshapes: usize,
}

/// One non-empty `(variant, blendshape)` cell: the sparse per-vertex deltas
/// of that archetype's blendshape (via [`Self::entries`]).
#[derive(Debug, Clone, Copy)]
pub struct ExpressionRecord<'a> {
    /// First vertex of the walk (record `+0x04` `hi16`).
    pub start_vertex: u32,
    /// Per-axis dequantisation scale (metres per code step).
    pub scale: [f32; 3],
    /// Per-axis bias (metres) — the delta for code 0.
    pub bias: [f32; 3],
    block: &'a [u8],
    count: usize,
}

/// Iterator over an [`ExpressionRecord`]'s `(vertex_index, delta)` entries
/// (delta in metres, CryEngine space).
#[derive(Debug)]
pub struct ExpressionEntries<'a> {
    block: &'a [u8],
    i: usize,
    count: usize,
    vtx: u32,
    scale: [f32; 3],
    bias: [f32; 3],
}

impl<'a> DnaFile<'a> {
    /// Decode slot 19 into a [`ExpressionMorphs`] grid view.
    ///
    /// # Errors
    /// - [`DnaError::ExpressionMorphsUnset`] if slot 19 is null.
    /// - [`DnaError::ExpressionMorphsMalformed`] if Region A's record count is
    ///   not a multiple of `num_variants`.
    pub fn expression_morphs(&self) -> Result<ExpressionMorphs<'a>, DnaError> {
        let segment = self
            .segment_for_slot(EXPRESSION_MORPHS_SLOT)
            .ok_or(DnaError::ExpressionMorphsUnset)?;
        let data = self.segment_bytes(segment);
        let num_variants = self.header.num_variants as usize;
        if data.len() < 4 || num_variants == 0 {
            return Err(DnaError::ExpressionMorphsMalformed {
                records: 0,
                num_variants,
            });
        }
        let region_a_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let records = region_a_len / RECORD_STRIDE;
        if records == 0 || records % num_variants != 0 {
            return Err(DnaError::ExpressionMorphsMalformed {
                records,
                num_variants,
            });
        }
        Ok(ExpressionMorphs {
            data,
            num_variants,
            num_blendshapes: records / num_variants,
        })
    }
}

impl<'a> ExpressionMorphs<'a> {
    /// Number of identity variants (Region-A rows).
    pub fn num_variants(&self) -> usize {
        self.num_variants
    }

    /// Number of blendshapes per variant (Region-A columns; 495 on the
    /// master = 205 appearance + 290 corrective).
    pub fn num_blendshapes(&self) -> usize {
        self.num_blendshapes
    }

    /// The `(variant, blendshape)` cell, or `None` if the cell is empty
    /// (`+0x04 == 0`), out of range, or its block overruns the segment.
    pub fn record(&self, variant: usize, blendshape: usize) -> Option<ExpressionRecord<'a>> {
        if variant >= self.num_variants || blendshape >= self.num_blendshapes {
            return None;
        }
        let r = variant * self.num_blendshapes + blendshape;
        let base = r * RECORD_STRIDE;
        if base + RECORD_STRIDE > self.data.len() {
            return None;
        }
        let u32_at = |o: usize| {
            u32::from_le_bytes([
                self.data[o],
                self.data[o + 1],
                self.data[o + 2],
                self.data[o + 3],
            ])
        };
        let f32_at = |o: usize| f32::from_le_bytes(self.data[o..o + 4].try_into().unwrap());
        let pk = u32_at(base + 4);
        if pk == 0 {
            return None; // empty cell
        }
        let count = (pk & 0xFFFF) as usize;
        let off = u32_at(base) as usize;
        if off + count * 4 > self.data.len() {
            return None; // malformed block extent
        }
        Some(ExpressionRecord {
            start_vertex: pk >> 16,
            scale: [f32_at(base + 0x08), f32_at(base + 0x0c), f32_at(base + 0x10)],
            bias: [f32_at(base + 0x14), f32_at(base + 0x18), f32_at(base + 0x1c)],
            block: &self.data[off..off + count * 4],
            count,
        })
    }
}

impl<'a> ExpressionRecord<'a> {
    /// Iterate the cell's `(vertex_index, delta)` entries.
    pub fn entries(&self) -> ExpressionEntries<'a> {
        ExpressionEntries {
            block: self.block,
            i: 0,
            count: self.count,
            vtx: self.start_vertex,
            scale: self.scale,
            bias: self.bias,
        }
    }

    /// Number of `(vertex, delta)` entries in this cell.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the cell has no entries.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Iterator for ExpressionEntries<'_> {
    /// `(vertex_index, delta)` — delta in metres, CryEngine space.
    type Item = (u32, [f32; 3]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.count {
            return None;
        }
        let e = &self.block[self.i * 4..self.i * 4 + 4];
        self.vtx = self.vtx.wrapping_add(e[0] as u32);
        let delta = [0, 1, 2].map(|k| self.bias[k] + e[1 + k] as f32 * self.scale[k]);
        self.i += 1;
        Some((self.vtx, delta))
    }
}

#[cfg(test)]
mod tests {
    use super::{EXPRESSION_MORPHS_SLOT, RECORD_STRIDE};
    use crate::parser::{DnaFile, MAGIC, MIN_FILE_SIZE, POINTER_TABLE_OFFSET};

    /// One synthetic cell: `(variant, blendshape, start_vertex, scale, bias,
    /// entries = [(vertex_stride, [qx, qy, qz])])`.
    type Cell = (usize, usize, u16, [f32; 3], [f32; 3], Vec<(u8, [u8; 3])>);

    /// Build a synthetic `.dna` slot-19 segment: `num_variants × num_bs`
    /// records (variant-major); cells not listed are empty.
    fn build_dna_with_slot19(num_variants: u16, num_bs: usize, cells: &[Cell]) -> Vec<u8> {
        let records = num_variants as usize * num_bs;
        let region_a = records * RECORD_STRIDE;
        let mut blocks: Vec<u8> = Vec::new();
        let mut placed: std::collections::HashMap<(usize, usize), (usize, usize)> =
            std::collections::HashMap::new();
        for (v, b, _, _, _, entries) in cells {
            let off = region_a + blocks.len();
            for (stride, q) in entries {
                blocks.extend_from_slice(&[*stride, q[0], q[1], q[2]]);
            }
            placed.insert((*v, *b), (off, entries.len()));
        }
        let body = MIN_FILE_SIZE;
        let mut buf = vec![0u8; body + region_a + blocks.len()];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&num_variants.to_le_bytes());
        let slot_off = POINTER_TABLE_OFFSET + EXPRESSION_MORPHS_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body as u64).to_le_bytes());
        let seg = body;
        buf[seg..seg + 4].copy_from_slice(&(region_a as u32).to_le_bytes());
        buf[seg + region_a..].copy_from_slice(&blocks);
        for (v, b, start, scale, bias, _) in cells {
            let rec = seg + (v * num_bs + b) * RECORD_STRIDE;
            let (off, count) = placed[&(*v, *b)];
            buf[rec..rec + 4].copy_from_slice(&(off as u32).to_le_bytes());
            let packed = ((*start as u32) << 16) | count as u32;
            buf[rec + 4..rec + 8].copy_from_slice(&packed.to_le_bytes());
            for k in 0..3 {
                buf[rec + 0x08 + k * 4..rec + 0x0c + k * 4].copy_from_slice(&scale[k].to_le_bytes());
                buf[rec + 0x14 + k * 4..rec + 0x18 + k * 4].copy_from_slice(&bias[k].to_le_bytes());
            }
        }
        buf
    }

    #[test]
    fn decodes_grid_shape() {
        let buf = build_dna_with_slot19(3, 5, &[]);
        let dna = DnaFile::read(&buf).expect("parse");
        let m = dna.expression_morphs().expect("slot19");
        assert_eq!(m.num_variants(), 3);
        assert_eq!(m.num_blendshapes(), 5);
        // all cells empty
        assert!(m.record(0, 0).is_none());
        assert!(m.record(2, 4).is_none());
        assert!(m.record(3, 0).is_none()); // out of range
    }

    /// Entries are `[u8 vertex stride][u8 qx][u8 qy][u8 qz]`; the vertex walk
    /// starts at the record's `hi16` start vertex, and each axis dequantises as
    /// `bias + q · scale` (bias = per-axis minimum, scale = range / 255).
    /// Verified against the archetype heads' own `BShapesGpu` (≤ 0.012 mm).
    #[test]
    fn decodes_start_vertex_stride_and_per_axis_codes() {
        let (scale, bias) = ([0.001, 0.002, 0.004], [-0.1, 0.0, 0.5]);
        let buf = build_dna_with_slot19(
            1,
            2,
            &[(0, 0, 100, scale, bias, vec![(0, [0, 255, 10]), (3, [1, 2, 3])])],
        );
        let dna = DnaFile::read(&buf).expect("parse");
        let m = dna.expression_morphs().expect("slot19");
        let rec = m.record(0, 0).expect("cell present");
        assert_eq!(rec.len(), 2);
        let got: Vec<(u32, [f32; 3])> = rec.entries().collect();
        assert_eq!(got[0].0, 100); // start vertex + stride 0
        assert_eq!(got[1].0, 103);
        let want0 = [-0.1, 0.51, 0.54];
        let want1 = [-0.099, 0.004, 0.512];
        for k in 0..3 {
            assert!((got[0].1[k] - want0[k]).abs() < 1e-6, "e0 axis {k}: {:?}", got[0].1);
            assert!((got[1].1[k] - want1[k]).abs() < 1e-6, "e1 axis {k}: {:?}", got[1].1);
        }
        assert!(m.record(0, 1).is_none());
    }

    #[test]
    fn unset_slot19_errors() {
        let mut buf = vec![0u8; MIN_FILE_SIZE];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());
        let dna = DnaFile::read(&buf).expect("parse");
        assert!(matches!(
            dna.expression_morphs(),
            Err(crate::error::DnaError::ExpressionMorphsUnset)
        ));
    }
}
