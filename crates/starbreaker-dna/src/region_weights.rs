//! Slot-22 per-vertex DNA-region blend-weight reader.
//!
//! Slot 22 holds, for each mesh vertex, a weight for each of the
//! `mesh_blends_per_pool` DNA regions (the `ECharacterCustomizerDNARegion`
//! set — 13 for the master human head). It is the per-vertex analogue of
//! slot 21's per-joint region weights, and supplies the per-vertex variant
//! fold used to blend per-variant data (slot 19) into a single character.
//!
//! On-disk layout: REGION-major `[region][vertex]` little-endian f32 starting
//! at the slot offset, followed by 8 trailing bytes. Master fixture:
//! `5584 × 13 × 4 + 8 = 290,376` bytes (exact). The per-vertex weights form a
//! partition of unity (each vertex's 13 region weights sum to 1). Verified
//! byte-exact against an independent consumer of the master male `.dna`;
//! an earlier reading (8-byte leading header, vertex-major) scrambled every
//! mask. The reader transposes to the vertex-major view below.
//! Character-invariant geometry (NOT identity data).
//!
//! Per-part `.dna` files (eyes, teeth) cover only some face parts: header
//! byte `0x29` is the mesh's LOCAL region count and slot 12 lists the global
//! face part of each local region ([`DnaFile::mesh_regions`]; eyes = EYE_L/
//! EYE_R, teeth = NOSE_C/MOUTH_C). Slot 22 is then `[local][vertex]`; the
//! reader scatters it into the global `[vertex][mesh_blends_per_pool]` view.
//!
//! See `docs/riglogic/SPEC.md` ("Slot 22") for the byte map and Pass 19k for
//! the decode evidence.

use crate::error::DnaError;
use crate::parser::DnaFile;

/// Pointer-table slot index for the per-vertex region weights.
pub const REGION_WEIGHTS_SLOT: usize = 22;

/// Pointer-table slot listing the global face part of each local region.
pub const MESH_REGIONS_SLOT: usize = 12;

/// Trailing bytes after the weight table in slot 22 (not a leading header —
/// the weights start at the slot offset).
const SLOT22_TRAILER: usize = 8;

impl<'a> DnaFile<'a> {
    /// Read slot 22 as a flat `Vec<f32>` of length
    /// `num_vertices * mesh_blends_per_pool`, indexed
    /// `[vertex * regions + region]`.
    ///
    /// `regions` is `header.mesh_blends_per_pool`; the vertex count is taken
    /// from `header.num_vertices`. The female `.dna` header's `num_vertices`
    /// is unreliable — callers that know the true vertex count from the head
    /// mesh should use [`DnaFile::region_weights_with_num_vertices`].
    ///
    /// # Errors
    /// - [`DnaError::RegionWeightsUnset`] if slot 22 is null.
    /// - [`DnaError::RegionWeightsLengthMismatch`] if the span is too small.
    pub fn region_weights(&self) -> Result<Vec<f32>, DnaError> {
        self.region_weights_with_num_vertices(self.header.num_vertices as usize)
    }

    /// Like [`DnaFile::region_weights`] but takes the vertex count explicitly. The
    /// female `.dna` header's `num_vertices` is unreliable, so callers that know
    /// the true count (from the head mesh) must use this. Returns a flat `Vec<f32>`
    /// of length `num_vertices * mesh_blends_per_pool`, indexed `[vtx*regions+region]`.
    ///
    /// # Errors
    /// - [`DnaError::RegionWeightsUnset`] if slot 22 is null.
    /// - [`DnaError::RegionWeightsLengthMismatch`] if the span is too small.
    pub fn region_weights_with_num_vertices(&self, num_vertices: usize) -> Result<Vec<f32>, DnaError> {
        let regions = self.header.mesh_blends_per_pool as usize;
        let local = self.mesh_regions()?;
        let needed = num_vertices * local.len() * 4 + SLOT22_TRAILER;

        let segment = self
            .segment_for_slot(REGION_WEIGHTS_SLOT)
            .ok_or(DnaError::RegionWeightsUnset)?;
        let span = self.segment_bytes(segment);
        if span.len() < needed {
            return Err(DnaError::RegionWeightsLengthMismatch {
                num_vertices,
                regions: local.len(),
                needed,
                got: span.len(),
            });
        }

        // Disk is local-region-major `[local][vertex]`; scatter into the
        // vertex-major `[vertex][global region]` view callers index.
        let mut out = vec![0.0f32; num_vertices * regions];
        for (l, &r) in local.iter().enumerate() {
            for v in 0..num_vertices {
                let off = (l * num_vertices + v) * 4;
                out[v * regions + r] = f32::from_le_bytes([
                    span[off],
                    span[off + 1],
                    span[off + 2],
                    span[off + 3],
                ]);
            }
        }
        Ok(out)
    }

    /// Global face part (`0..mesh_blends_per_pool`) of each of this mesh's
    /// LOCAL DNA regions, in local order. The head covers all 13; per-part
    /// `.dna` files list theirs in slot 12, with the count in header byte
    /// `0x29`. Absent slot 12 / zero count => the identity over all regions.
    ///
    /// # Errors
    /// [`DnaError::MeshRegionsMalformed`] if the count exceeds
    /// `mesh_blends_per_pool` or an entry is out of range.
    pub fn mesh_regions(&self) -> Result<Vec<usize>, DnaError> {
        let bpp = self.header.mesh_blends_per_pool as usize;
        let local = match self.header.header_u8_29 as usize {
            0 => bpp,
            n => n,
        };
        if local > bpp {
            return Err(DnaError::MeshRegionsMalformed(format!(
                "{local} local regions > mesh_blends_per_pool {bpp}"
            )));
        }
        let regions: Vec<usize> = match self.slot_bytes(MESH_REGIONS_SLOT) {
            Some(b) if b.len() >= local * 4 => (0..local)
                .map(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()) as usize)
                .collect(),
            _ => (0..local).collect(),
        };
        if let Some(bad) = regions.iter().find(|&&r| r >= bpp) {
            return Err(DnaError::MeshRegionsMalformed(format!(
                "region {bad} >= mesh_blends_per_pool {bpp}"
            )));
        }
        Ok(regions)
    }
}

#[cfg(test)]
mod tests {
    use super::{REGION_WEIGHTS_SLOT, SLOT22_TRAILER};
    use crate::parser::{DnaFile, MAGIC, MIN_FILE_SIZE, POINTER_TABLE_OFFSET};

    /// Build a synthetic `.dna` whose slot 22 holds `vals` (given in the
    /// vertex-major `[vertex][region]` order the reader returns), written to
    /// disk region-major with the 8-byte trailer. `vals.len()` may be short to
    /// exercise the length check (then written as-is). The header's
    /// `num_vertices` is `header_num_vertices` (garbage on the female `.dna`).
    fn build_slot22(regions: usize, verts: usize, header_num_vertices: u32, vals: &[f32]) -> Vec<u8> {
        let disk: Vec<f32> = if vals.len() == verts * regions {
            (0..regions)
                .flat_map(|r| (0..verts).map(move |v| vals[v * regions + r]))
                .collect()
        } else {
            vals.to_vec()
        };
        let body = MIN_FILE_SIZE;
        let total = body + disk.len() * 4 + SLOT22_TRAILER;
        let mut buf = vec![0u8; total];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x28] = regions as u8; // mesh_blends_per_pool
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes()); // num_variants (non-zero)
        buf[0x30..0x34].copy_from_slice(&header_num_vertices.to_le_bytes());
        let slot_off = POINTER_TABLE_OFFSET + REGION_WEIGHTS_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body as u64).to_le_bytes());
        for (i, &v) in disk.iter().enumerate() {
            let o = body + i * 4;
            buf[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        buf
    }

    fn build_dna_with_slot22(num_vertices: u32, regions: u8, vals: &[f32]) -> Vec<u8> {
        build_slot22(regions as usize, num_vertices as usize, num_vertices, vals)
    }

    #[test]
    fn region_weights_uses_explicit_vertex_count() {
        let regions = 2usize;
        let verts = 3usize;
        let buf = build_slot22(regions, verts, /*header_num_vertices=*/0,
            &[0.1, 0.2,  0.3, 0.4,  0.5, 0.6]);
        let dna = DnaFile::read(&buf).expect("parse");
        let rw = dna.region_weights_with_num_vertices(verts).expect("rw");
        assert_eq!(rw.len(), verts * regions);
        assert!((rw[0] - 0.1).abs() < 1e-6);
        assert!((rw[5] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn reads_slot22_region_weights() {
        // 2 vertices × 3 regions = 6 floats.
        let vals: Vec<f32> = (0..6).map(|i| i as f32 / 255.0).collect();
        let buf = build_dna_with_slot22(2, 3, &vals);
        let dna = DnaFile::read(&buf).expect("parse");
        let got = dna.region_weights().expect("read slot 22");
        assert_eq!(got.len(), 6);
        // vertex 1, region 0 → flat index 3.
        assert!((got[3] - 3.0 / 255.0).abs() < 1e-6, "v1r0 = {}", got[3]);
        assert!((got[5] - 5.0 / 255.0).abs() < 1e-6);
    }

    /// On-disk slot 22 is REGION-major (`[region][vertex]` f32) starting at the
    /// slot offset, followed by 8 trailing bytes — verified byte-exact against
    /// the master male `.dna` (span 290,376 = 13 × 5584 × 4 + 8). The reader
    /// must still return the vertex-major `[vtx * regions + region]` view.
    #[test]
    fn reads_region_major_layout_without_leading_header() {
        let (regions, verts) = (3usize, 2usize);
        // disk[r * verts + v] = 10*r + v
        let disk: Vec<f32> = (0..regions)
            .flat_map(|r| (0..verts).map(move |v| (10 * r + v) as f32))
            .collect();
        let body = MIN_FILE_SIZE;
        let mut buf = vec![0u8; body + disk.len() * 4 + 8];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x28] = regions as u8;
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());
        buf[0x30..0x34].copy_from_slice(&(verts as u32).to_le_bytes());
        let slot_off = POINTER_TABLE_OFFSET + REGION_WEIGHTS_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body as u64).to_le_bytes());
        for (i, v) in disk.iter().enumerate() {
            buf[body + i * 4..body + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let dna = DnaFile::read(&buf).expect("parse");
        let got = dna.region_weights().expect("read slot 22");
        for v in 0..verts {
            for r in 0..regions {
                assert_eq!(got[v * regions + r], (10 * r + v) as f32, "v{v} r{r}");
            }
        }
    }

    /// A per-part `.dna` (2 local regions mapped to global 2 and 3 via slot 12,
    /// count in header byte 0x29) scatters into the global 13-region view.
    #[test]
    fn local_regions_scatter_into_global_view() {
        let (regions, verts) = (13usize, 2usize);
        let disk: [f32; 4] = [0.25, 1.0, 0.75, 0.0]; // [local][vertex]
        let body = MIN_FILE_SIZE;
        let s12 = body;
        let s22 = body + 16;
        let mut buf = vec![0u8; s22 + disk.len() * 4 + SLOT22_TRAILER];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x28] = regions as u8;
        buf[0x29] = 2;
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());
        for (slot, off) in [(super::MESH_REGIONS_SLOT, s12), (REGION_WEIGHTS_SLOT, s22)] {
            let o = POINTER_TABLE_OFFSET + slot * 8;
            buf[o..o + 8].copy_from_slice(&(off as u64).to_le_bytes());
        }
        for (i, r) in [2u32, 3, 0, 0].iter().enumerate() {
            buf[s12 + i * 4..s12 + i * 4 + 4].copy_from_slice(&r.to_le_bytes());
        }
        for (i, v) in disk.iter().enumerate() {
            buf[s22 + i * 4..s22 + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let dna = DnaFile::read(&buf).expect("parse");
        assert_eq!(dna.mesh_regions().unwrap(), vec![2, 3]);
        let rw = dna.region_weights_with_num_vertices(verts).unwrap();
        assert_eq!(rw.len(), verts * regions);
        assert_eq!((rw[2], rw[3]), (0.25, 0.75)); // vertex 0
        assert_eq!((rw[regions + 2], rw[regions + 3]), (1.0, 0.0)); // vertex 1
        assert_eq!(rw.iter().sum::<f32>(), 2.0);
    }

    #[test]
    fn errors_when_slot22_span_too_small() {
        let buf = build_dna_with_slot22(2, 3, &[0.0; 3]); // only 3 floats, need 6
        let dna = DnaFile::read(&buf).expect("parse");
        assert!(dna.region_weights().is_err());
    }

    #[test]
    fn errors_when_slot22_unset() {
        // num_vertices/regions set but slot 22 pointer left null.
        let mut buf = vec![0u8; MIN_FILE_SIZE];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x28] = 3;
        buf[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes());
        buf[0x30..0x34].copy_from_slice(&2u32.to_le_bytes());
        let dna = DnaFile::read(&buf).expect("parse");
        assert!(matches!(
            dna.region_weights(),
            Err(crate::error::DnaError::RegionWeightsUnset)
        ));
    }
}
