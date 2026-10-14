//! `BShapesGpu` chunk decoder — per-vertex per-blendshape Vec3 deltas.
//!
//! Decodes the `.skinm` chunk type `0x57A38888` ("BShapesGpu") which holds
//! compressed per-vertex per-blendshape morph data. The engine evaluates this
//! GPU-side (the `Deformer_BShapes_DQSkinning` compute shader), so the
//! formula here is the simplest plausible interpretation: each affected
//! `(vertex, blendshape)` pair encodes a signed-byte XYZ index that is
//! dequantized via a per-blendshape `Vec3<f32>` scale.
//!
//! ## Format (anchored empirically from
//! `protos_human_male_face_t1_pu_head.skinm`)
//!
//! ```text
//! Header (32 bytes):
//!     +0x00 u32 num_blendshapes
//!     +0x04 u32 num_vertices
//!     +0x08 u32 opcode_count (= sum over vertices of bits-set in bitfield)
//!     +0x0c u32 csr_offset           (per-vertex u32 offset table)
//!     +0x10 u32 per_bs_scales_offset (24 bytes/bs: 2 Vec3<f32>)
//!     +0x14 u32 bitfield_offset      (vertex × blendshape participation bits)
//!     +0x18 u32 opcodes_offset
//!     +0x1c u32 declared_size
//!
//! CSR table at +csr_offset:
//!     u32[num_vertices]   — csr[V] = first opcode index for vertex V
//!     opcode count for vertex V = csr[V+1] - csr[V]
//!     (where csr[num_vertices] is implicit = opcode_count)
//!
//! Per-blendshape scales at +per_bs_scales_offset:
//!     [(Vec3<f32> a, Vec3<f32> b); num_blendshapes]
//!     a = per-axis dequantization scale (position delta per LSB)
//!     b = currently unused (likely a normal-delta scale or unused offset)
//!
//! Bitfield at +bitfield_offset:
//!     u32[num_vertices × ceil(num_blendshapes / 32)]
//!     bit (V, B) set iff vertex V is affected by blendshape B
//!     Order: u32-major, little-endian bits
//!
//! Opcodes at +opcodes_offset:
//!     i8[3 × opcode_count]
//!     For each vertex V's active blendshapes (in ascending B order),
//!     one 3-byte (i8 x, i8 y, i8 z) triple.
//! ```
//!
//! ## Formula (Pass 14, revision 2)
//!
//! ```text
//! delta_xyz(V, B) = byte_unsigned_xyz * a[B] + b[B]
//! ```
//!
//! Where `byte_unsigned ∈ [0, 255]`. Rationale: the per-axis `b/a` ratio
//! across blendshapes is in the range -81 to -200 (NOT a single constant),
//! and `a` values are tiny (1e-6 to 1e-5 range) while `b` values are small
//! offsets (~1e-3 to 1e-4 range). The natural interpretation is that `a`
//! is the per-LSB step and `b` is the byte=0 baseline, so the byte encodes
//! a position within a per-blendshape per-axis range:
//! - byte = 0:   delta = b
//! - byte = 255: delta = 255*a + b
//! - byte = -b/a: delta = 0 (the "neutral" byte value, varies per blendshape)
//!
//! Earlier Pass 14 rev 1 used `delta = i8_signed * a` which produced overly-
//! large displacements per vertex and led to "uniform shifts" for heavy-
//! hitter blendshapes when their bytes clustered near 128 (= -128 signed).

use thiserror::Error;

/// One blendshape's per-vertex position deltas.
///
/// Length of `positions` equals the mesh's vertex count. Vertices not
/// affected by this blendshape have a zero Vec3.
#[derive(Debug, Clone)]
pub struct MorphTarget {
    /// Per-vertex position delta (added to base vertex when the
    /// blendshape activation weight is 1.0).
    pub positions: Vec<[f32; 3]>,
}

/// Decoded set of all blendshapes for a head mesh.
#[derive(Debug)]
pub struct BlendShapes {
    pub num_blendshapes: u32,
    pub num_vertices: u32,
    /// One morph target per blendshape. Length = `num_blendshapes`.
    pub targets: Vec<MorphTarget>,
}

#[derive(Debug, Error)]
pub enum BlendShapesError {
    #[error("BShapesGpu chunk too small: need {min} bytes, got {got}")]
    ChunkTooSmall { min: usize, got: usize },
    #[error("invalid offset in header (section {section}): off={off}, chunk_size={chunk_size}")]
    InvalidOffset {
        section: &'static str,
        off: usize,
        chunk_size: usize,
    },
    #[error("opcode out of bounds: needed byte {at}, chunk_size={chunk_size}")]
    OpcodeOob { at: usize, chunk_size: usize },
    #[error(
        "slot 17 buffer shape mismatch: len={len}, num_vertices={num_vertices}, num_blendshapes={num_blendshapes} \
         (expected len % num_vertices == 0 and stride >= num_blendshapes)"
    )]
    Slot17ShapeMismatch {
        len: usize,
        num_vertices: usize,
        num_blendshapes: usize,
    },
}

/// Parse a `BShapesGpu` chunk into dense per-blendshape per-vertex Vec3 deltas.
pub fn parse_bshapes_gpu(data: &[u8]) -> Result<BlendShapes, BlendShapesError> {
    parse_bshapes_gpu_with_slot17(data, None)
}

/// Like [`parse_bshapes_gpu`] but optionally applies a per-(vertex, blendshape)
/// modulator from `.dna` slot 17.
///
/// Pass 15 hypothesis (2026-05-24): the `.dna` slot-17 byte at
/// `slot17[V * 496 + B]` (with stride `num_blendshapes + 1 = 496`, last
/// byte per vertex being a flag) modulates the base BShapesGpu delta:
///
/// ```text
/// final_delta(V, B) = bshapes_delta(V, B) * slot17[V * 496 + B] / 255
/// ```
///
/// If the slice is `None` we fall back to the un-modulated base behavior.
/// `slot17_bytes`, if supplied, must be exactly `num_vertices * (num_blendshapes + 1)`
/// bytes long.
pub fn parse_bshapes_gpu_with_slot17(
    data: &[u8],
    slot17_bytes: Option<&[u8]>,
) -> Result<BlendShapes, BlendShapesError> {
    if data.len() < 32 {
        return Err(BlendShapesError::ChunkTooSmall {
            min: 32,
            got: data.len(),
        });
    }

    let num_blendshapes = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let num_vertices = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let opcode_count = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let csr_off = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let scales_off = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
    let bitfield_off = u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize;
    let opcodes_off = u32::from_le_bytes(data[24..28].try_into().unwrap()) as usize;
    // [28..32] = declared total size; ignored for parsing (we use the actual chunk len).

    let bs_count = num_blendshapes as usize;
    let v_count = num_vertices as usize;
    let words_per_vertex = (bs_count + 31) / 32;

    let csr_end = csr_off + v_count * 4;
    let scales_end = scales_off + bs_count * 24;
    let bitfield_end = bitfield_off + v_count * words_per_vertex * 4;
    let opcodes_end = opcodes_off + (opcode_count as usize) * 3;

    for (section, off) in [
        ("csr", csr_end),
        ("scales", scales_end),
        ("bitfield", bitfield_end),
        ("opcodes", opcodes_end),
    ] {
        if off > data.len() {
            return Err(BlendShapesError::InvalidOffset {
                section,
                off,
                chunk_size: data.len(),
            });
        }
    }

    // ── Parse per-blendshape scales ────────────────────────────────────────
    // 24 bytes/blendshape = (Vec3<f32> a, Vec3<f32> b).
    let mut scale_a = Vec::with_capacity(bs_count);
    let mut scale_b = Vec::with_capacity(bs_count);
    for b in 0..bs_count {
        let base = scales_off + b * 24;
        scale_a.push([
            f32::from_le_bytes(data[base..base + 4].try_into().unwrap()),
            f32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap()),
            f32::from_le_bytes(data[base + 8..base + 12].try_into().unwrap()),
        ]);
        scale_b.push([
            f32::from_le_bytes(data[base + 12..base + 16].try_into().unwrap()),
            f32::from_le_bytes(data[base + 16..base + 20].try_into().unwrap()),
            f32::from_le_bytes(data[base + 20..base + 24].try_into().unwrap()),
        ]);
    }

    // Detect "sentinel" blendshapes: those with b ≈ (9, 9, 9) and a ≈ 4e-8.
    // These are reserved/unused slots; emit zero deltas to avoid 9-unit shifts.
    let is_sentinel: Vec<bool> = scale_b
        .iter()
        .map(|&[bx, by, bz]| (bx - 9.0).abs() < 0.01 && (by - 9.0).abs() < 0.01 && (bz - 9.0).abs() < 0.01)
        .collect();

    // ── Resolve slot 17 stride (per-vertex row length in bytes) ─────────
    //
    // The male master fixture has stride = num_blendshapes + 1 = 496 (one
    // trailing flag byte per vertex, see PHASE_4B_INVESTIGATION §6.14.5
    // open Q1). The female fixture has stride = 392 — fewer mask entries
    // per vertex than the mesh has blendshapes. The structural reason is
    // not yet anchored (likely the female slot 17 only carries a mask for
    // a subset of blendshapes; entries past the stride pass through with
    // no modulation). Compute the stride from the actual buffer length.
    //
    // Effective per-row mask-entry count is `stride - 1` (excluding the
    // trailing flag byte). For blendshape indices `B >= stride - 1`, no
    // modulation is applied (the mask read would land in the trailing
    // byte or past the row end).
    let slot17_stride = if let Some(s17) = slot17_bytes {
        if v_count == 0 {
            0
        } else if s17.len() % v_count != 0 {
            return Err(BlendShapesError::Slot17ShapeMismatch {
                len: s17.len(),
                num_vertices: v_count,
                num_blendshapes: bs_count,
            });
        } else {
            s17.len() / v_count
        }
    } else {
        0
    };

    // ── Allocate dense morph targets ───────────────────────────────────────
    let mut targets: Vec<MorphTarget> = (0..bs_count)
        .map(|_| MorphTarget {
            positions: vec![[0.0_f32; 3]; v_count],
        })
        .collect();

    // ── Walk per-vertex opcode lists ───────────────────────────────────────
    for v in 0..v_count {
        let csr_v = u32::from_le_bytes(
            data[csr_off + v * 4..csr_off + (v + 1) * 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let csr_next = if v + 1 < v_count {
            u32::from_le_bytes(
                data[csr_off + (v + 1) * 4..csr_off + (v + 2) * 4]
                    .try_into()
                    .unwrap(),
            ) as usize
        } else {
            opcode_count as usize
        };
        let n_ops = csr_next - csr_v;

        // Collect this vertex's active blendshape indices (ascending order).
        let bitfield_v_base = bitfield_off + v * words_per_vertex * 4;
        let mut active_bs: Vec<u16> = Vec::with_capacity(n_ops);
        for w in 0..words_per_vertex {
            let word_off = bitfield_v_base + w * 4;
            let word = u32::from_le_bytes(data[word_off..word_off + 4].try_into().unwrap());
            if word == 0 {
                continue;
            }
            for bit in 0..32 {
                if word & (1 << bit) != 0 {
                    let b = (w * 32 + bit) as u16;
                    if (b as usize) < bs_count {
                        active_bs.push(b);
                    }
                }
            }
        }
        if active_bs.len() != n_ops {
            // Bitfield bit-count must equal opcode count per vertex.
            // Skip silently for production; in debug we could log a warn.
        }

        // Apply each opcode to its corresponding blendshape's morph target.
        for (k, &bs) in active_bs.iter().enumerate() {
            let op_off = opcodes_off + (csr_v + k) * 3;
            if op_off + 3 > data.len() {
                return Err(BlendShapesError::OpcodeOob {
                    at: op_off + 3,
                    chunk_size: data.len(),
                });
            }
            if is_sentinel[bs as usize] {
                // Sentinel blendshape — emit zero delta to avoid the 9-unit
                // garbage from `byte * a + b` when b ≈ 9.
                continue;
            }
            let ix = data[op_off] as f32; // unsigned [0, 255]
            let iy = data[op_off + 1] as f32;
            let iz = data[op_off + 2] as f32;
            let sa = scale_a[bs as usize];
            let sb = scale_b[bs as usize];
            // Formula: delta = byte_unsigned * a + b
            let mut dx = ix * sa[0] + sb[0];
            let mut dy = iy * sa[1] + sb[1];
            let mut dz = iz * sa[2] + sb[2];
            // Pass 15 (confirmed empirically 2026-05-24): slot 17 is a
            // per-(vertex, blendshape) suppression mask where byte=0 means
            // "fully active" and byte=255 means "masked out for THIS
            // character". 47.5% of slot 17 bytes are 0 → passthrough;
            // 5.6% are 0xff → gated out; rest = partial attenuation.
            // Per-character mask: each .dna file carries its own slot 17
            // tuned to that character's customization.
            if let Some(s17) = slot17_bytes {
                // Stride derived from actual buffer length above. Only
                // apply the mask if the per-row slot for this blendshape
                // exists; entries past `stride - 1` (the trailing flag
                // byte) pass through with no modulation. The female rig
                // has stride 392 < bs_count 495 so the high blendshapes
                // get no modulation; the male rig has stride 496 so all
                // 495 blendshapes get masked.
                if slot17_stride > 0 && (bs as usize) < slot17_stride.saturating_sub(1) {
                    let mod_byte =
                        1.0 - (s17[v * slot17_stride + bs as usize] as f32 / 255.0);
                    dx *= mod_byte;
                    dy *= mod_byte;
                    dz *= mod_byte;
                }
            }
            targets[bs as usize].positions[v] = [dx, dy, dz];
        }
    }

    Ok(BlendShapes {
        num_blendshapes,
        num_vertices,
        targets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic 2-vertex / 3-blendshape BShapesGpu chunk for round-trip.
    /// Vertex 0 is affected by bs[1]; vertex 1 is affected by bs[0] and bs[2].
    fn synth_chunk() -> Vec<u8> {
        let num_bs = 3u32;
        let num_vtx = 2u32;
        let opcode_count = 3u32; // 1 (v0) + 2 (v1)

        // Layout:
        //   header  32 bytes
        //   csr     8 bytes  (2 × u32)
        //   scales  72 bytes (3 × 24)
        //   bitfield 8 bytes (2 × 1×u32)
        //   opcodes 9 bytes (3 × 3 bytes)
        //   pad?
        let header_size = 32;
        let csr_off = header_size;
        let scales_off = csr_off + 2 * 4;
        let bitfield_off = scales_off + 3 * 24;
        let opcodes_off = bitfield_off + 2 * 1 * 4;
        let total = opcodes_off + 3 * 3;

        let mut buf = vec![0u8; total];
        buf[0..4].copy_from_slice(&num_bs.to_le_bytes());
        buf[4..8].copy_from_slice(&num_vtx.to_le_bytes());
        buf[8..12].copy_from_slice(&opcode_count.to_le_bytes());
        buf[12..16].copy_from_slice(&(csr_off as u32).to_le_bytes());
        buf[16..20].copy_from_slice(&(scales_off as u32).to_le_bytes());
        buf[20..24].copy_from_slice(&(bitfield_off as u32).to_le_bytes());
        buf[24..28].copy_from_slice(&(opcodes_off as u32).to_le_bytes());
        buf[28..32].copy_from_slice(&(total as u32).to_le_bytes());

        // CSR offsets: v0 = 0, v1 = 1.
        buf[csr_off..csr_off + 4].copy_from_slice(&0u32.to_le_bytes());
        buf[csr_off + 4..csr_off + 8].copy_from_slice(&1u32.to_le_bytes());

        // Scales: bs[0] = 0.01, bs[1] = 0.1, bs[2] = -1.0 per axis.
        let scales: [[f32; 3]; 3] = [
            [0.01, 0.01, 0.01],
            [0.1, 0.1, 0.1],
            [-1.0, -1.0, -1.0],
        ];
        for (b, &s) in scales.iter().enumerate() {
            let base = scales_off + b * 24;
            buf[base..base + 4].copy_from_slice(&s[0].to_le_bytes());
            buf[base + 4..base + 8].copy_from_slice(&s[1].to_le_bytes());
            buf[base + 8..base + 12].copy_from_slice(&s[2].to_le_bytes());
            // scale_b left as zeros.
        }

        // Bitfield:
        //   v0 word 0 = bit 1 set       = 0b010 = 2
        //   v1 word 0 = bit 0 + bit 2   = 0b101 = 5
        buf[bitfield_off..bitfield_off + 4].copy_from_slice(&2u32.to_le_bytes());
        buf[bitfield_off + 4..bitfield_off + 8].copy_from_slice(&5u32.to_le_bytes());

        // Opcodes: v0 opcode (bs[1]) = (10, 20, 30); v1 opcodes (bs[0], bs[2]) = (-5, -10, -15), (50, 60, 70).
        buf[opcodes_off] = 10i8 as u8;
        buf[opcodes_off + 1] = 20i8 as u8;
        buf[opcodes_off + 2] = 30i8 as u8;
        buf[opcodes_off + 3] = (-5i8) as u8;
        buf[opcodes_off + 4] = (-10i8) as u8;
        buf[opcodes_off + 5] = (-15i8) as u8;
        buf[opcodes_off + 6] = 50i8 as u8;
        buf[opcodes_off + 7] = 60i8 as u8;
        buf[opcodes_off + 8] = 70i8 as u8;

        buf
    }

    #[test]
    fn synthetic_bshapes_decodes() {
        // Formula tested: delta = byte_unsigned * a + b (b=0 in this synth).
        let chunk = synth_chunk();
        let bs = parse_bshapes_gpu(&chunk).expect("decode");
        assert_eq!(bs.num_blendshapes, 3);
        assert_eq!(bs.num_vertices, 2);
        assert_eq!(bs.targets.len(), 3);

        // Opcodes encode UNSIGNED bytes (i8 negatives become 251-255 unsigned).
        // v1's bs[0] opcode = (-5, -10, -15) signed = (251, 246, 241) unsigned.
        // bs[0]: a=(0.01, 0.01, 0.01), b=(0,0,0). delta_x = 251 * 0.01 = 2.51.
        assert_eq!(bs.targets[0].positions[0], [0.0, 0.0, 0.0]);
        assert!((bs.targets[0].positions[1][0] - 2.51).abs() < 1e-6);
        assert!((bs.targets[0].positions[1][1] - 2.46).abs() < 1e-6);
        assert!((bs.targets[0].positions[1][2] - 2.41).abs() < 1e-6);

        // v0's bs[1] opcode = (10, 20, 30) unsigned. bs[1] a=(0.1, 0.1, 0.1).
        // delta = (1.0, 2.0, 3.0).
        assert!((bs.targets[1].positions[0][0] - 1.0).abs() < 1e-6);
        assert!((bs.targets[1].positions[0][1] - 2.0).abs() < 1e-6);
        assert!((bs.targets[1].positions[0][2] - 3.0).abs() < 1e-6);
        assert_eq!(bs.targets[1].positions[1], [0.0, 0.0, 0.0]);

        // v1's bs[2] opcode = (50, 60, 70) unsigned. bs[2] a=(-1, -1, -1).
        // delta = (-50, -60, -70).
        assert_eq!(bs.targets[2].positions[0], [0.0, 0.0, 0.0]);
        assert!((bs.targets[2].positions[1][0] - (-50.0)).abs() < 1e-6);
        assert!((bs.targets[2].positions[1][1] - (-60.0)).abs() < 1e-6);
        assert!((bs.targets[2].positions[1][2] - (-70.0)).abs() < 1e-6);
    }

    #[test]
    fn sentinel_blendshape_emits_zero_delta() {
        // Build a chunk where bs[0] has b = (9.0, 9.0, 9.0) — the sentinel
        // pattern. Result: bs[0]'s morph target should be all zeros.
        let mut chunk = synth_chunk();
        // Overwrite bs[0]'s b vector to the sentinel marker.
        let scales_off = 32 + 8; // 32 header + 8 csr
        let b_off = scales_off + 12; // skip a (12 bytes), write b
        for axis in 0..3 {
            chunk[b_off + axis * 4..b_off + (axis + 1) * 4]
                .copy_from_slice(&9.0_f32.to_le_bytes());
        }
        let bs = parse_bshapes_gpu(&chunk).expect("decode");
        // bs[0] should be all zero regardless of opcode.
        for v in &bs.targets[0].positions {
            assert_eq!(v, &[0.0_f32; 3], "sentinel bs[0] should emit zero delta");
        }
    }
}
