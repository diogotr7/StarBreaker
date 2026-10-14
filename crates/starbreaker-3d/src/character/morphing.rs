//! Facial-morph computation: ExecuteHuman port.
//!
//! Mirrors `CFaceRigLogic::ExecuteHuman` (Ghidra `0x1466e0aa0`), anchored
//! to `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.9.4 (Pass 7).
//!
//! Consumes:
//!  - The 2-D `DnaMatrix2D` built by Phase 4b.2b's accumulator
//!  - The per-(joint × face-part) mixing matrix from `.dna` slot 21
//!  - Each variant's slot-31 packed-delta payloads (via `variant_deltas`)
//!  - A per-blendshape activation vector
//!
//! Produces `JointDeltas { translation, rotation_eu }` per joint.
//!
//! ## Mapping (Pass 7 anchored)
//!
//! - `variant.primary_payload` → translation deltas (added to bind translation)
//! - `variant.secondary_payload` → rotation Euler XYZ (converted to quaternion,
//!   multiplied INTO bind quaternion as `bind_q * delta_q`)
//!
//! ## Activations
//!
//! `activations: &[f32]` of length `num_appearance + num_corrective` is the
//! per-blendshape weight vector — output of Section D appearance evaluation.
//! For a static rest-pose DNA-driven export, the caller supplies
//! `activations = [1.0, 0.0, ..., 0.0]` (only `CTL_PSD` baseline fires).

use starbreaker_dna::DnaFile;
use thiserror::Error;

use super::dna_matrix::matrix::DnaMatrix2D;
use crate::riglogic::decode_packed_delta;

/// Per-joint deltas — translation additive, rotation as Euler XYZ radians.
#[derive(Debug, Clone, PartialEq)]
pub struct JointDeltas {
    /// Per-joint translation delta. Length = num_joints. Added to
    /// bind-pose translation.
    pub translation: Vec<[f32; 3]>,

    /// Per-joint Euler-XYZ rotation delta in radians. Length = num_joints.
    /// Applied via `bind_quat * euler_xyz_to_quat(delta)`.
    pub rotation_eu: Vec<[f32; 3]>,
}

/// Errors from [`compute_joint_deltas`].
#[derive(Debug, Error, PartialEq)]
pub enum MorphError {
    #[error(
        "joint_weights length mismatch: expected num_joints*mesh_bpp = {expected}, got {got}"
    )]
    JointWeightsLengthMismatch { expected: usize, got: usize },

    #[error(
        "matrix shape mismatch: matrix.num_variants={matrix_variants} vs dna.num_variants={dna_variants}"
    )]
    MatrixNumVariantsMismatch { matrix_variants: usize, dna_variants: usize },

    #[error(
        "activations length mismatch: expected num_blendshapes = {expected}, got {got}"
    )]
    ActivationsLengthMismatch { expected: usize, got: usize },

    #[error("output_idx {output_idx} out of range (num_joints*3 = {max})")]
    OutputIdxOutOfRange { output_idx: u16, max: usize },

    #[error(
        "blendshape_idx {bs_idx} out of range (num_blendshapes = {num_blendshapes})"
    )]
    BlendshapeIdxOutOfRange { bs_idx: u16, num_blendshapes: usize },

    #[error(transparent)]
    DnaIter(#[from] starbreaker_dna::DnaError),
}

/// Compute per-joint translation and rotation deltas. See module docs.
///
/// `packed_low_bits` is typically 9 for the human face rig (gives the
/// 9-bit blendshape_idx / 23-bit signed coef split anchored in §5).
pub fn compute_joint_deltas(
    matrix: &DnaMatrix2D,
    joint_weights: &[f32],
    dna: &DnaFile<'_>,
    activations: &[f32],
    num_joints: usize,
    num_blendshapes: usize,
    packed_low_bits: u32,
) -> Result<JointDeltas, MorphError> {
    let mesh_bpp = matrix.blends_per_pool();
    let num_variants = matrix.num_variants();

    // 1. Validate shapes.
    if matrix.num_variants() != dna.variant_records.len() {
        return Err(MorphError::MatrixNumVariantsMismatch {
            matrix_variants: matrix.num_variants(),
            dna_variants: dna.variant_records.len(),
        });
    }
    let expected_jw = num_joints * mesh_bpp;
    if joint_weights.len() != expected_jw {
        return Err(MorphError::JointWeightsLengthMismatch {
            expected: expected_jw,
            got: joint_weights.len(),
        });
    }
    if activations.len() != num_blendshapes {
        return Err(MorphError::ActivationsLengthMismatch {
            expected: num_blendshapes,
            got: activations.len(),
        });
    }

    // 2. Find active variants (rows with at least one nonzero cell).
    let active_vars: Vec<usize> = (0..num_variants)
        .filter(|&v| (0..mesh_bpp).any(|fp| matrix.cell(v, fp) != 0.0))
        .collect();

    // Dequantization divisor: packed coefs are signed integers in
    // [-(1<<bit_width), (1<<bit_width)-1] where bit_width = 31 - packed_low_bits.
    // The Section D / variant-payload format normalises by this max_packed
    // value to produce f32 deltas (`CFaceRigLogic::ExecuteGeneric`'s formula;
    // see docs/riglogic/SPEC.md).
    //
    // Pass 8 (PHASE_4B_INVESTIGATION §6.10) anchored that the per-variant scale
    // is the RAW `primary_delta_scale` / `secondary_delta_scale` — NOT
    // normalised by `max_pri_scale` / `max_sec_scale`. Pass 7's pseudocode
    // included a `/max_pri_scale` divisor; Ghidra evidence shows that
    // factor is purely an internal +0x150/+0x158 cache round-trip that
    // cancels exactly.
    let bit_width = 31 - packed_low_bits;
    let max_packed = ((1u64 << bit_width) - 1) as f64;

    let mut translation = vec![[0.0_f32; 3]; num_joints];
    let mut rotation_eu = vec![[0.0_f32; 3]; num_joints];

    // 4. PRIMARY pass — translation.
    {
        for &var_idx in &active_vars {
            let variant = &dna.variant_records[var_idx];
            let pri_scale = variant.primary_delta_scale;

            // Pre-compute fold[joint] = Σ_fp joint_weights[joint*bpp + fp] * matrix[var, fp].
            // Reused across all records for this variant.
            let folds: Vec<f32> = (0..num_joints)
                .map(|j| {
                    (0..mesh_bpp)
                        .map(|fp| joint_weights[j * mesh_bpp + fp] * matrix.cell(var_idx, fp))
                        .sum()
                })
                .collect();

            for rec_result in dna.variant_main_deltas(var_idx)? {
                let rec = rec_result?;
                let axis_slot = rec.output_idx as usize;
                if axis_slot >= num_joints * 3 {
                    return Err(MorphError::OutputIdxOutOfRange {
                        output_idx: rec.output_idx,
                        max: num_joints * 3,
                    });
                }
                let joint = axis_slot / 3;
                let axis = axis_slot % 3;
                let fold = folds[joint];
                if fold == 0.0 {
                    continue;
                }

                let mut delta = 0.0_f64;
                for chunk in rec.packed_bytes.chunks_exact(4) {
                    let packed = u32::from_le_bytes(chunk.try_into().unwrap());
                    // packed_low_bits = number of low bits for the index.
                    // decode_packed_delta takes the coef bit width = 31 - packed_low_bits.
                    let (bs_idx, coef) = decode_packed_delta(packed, 31 - packed_low_bits);
                    if (bs_idx as usize) >= num_blendshapes {
                        return Err(MorphError::BlendshapeIdxOutOfRange {
                            bs_idx,
                            num_blendshapes,
                        });
                    }
                    delta += (coef as f64)
                        * pri_scale
                        * (fold as f64)
                        * (activations[bs_idx as usize] as f64)
                        / max_packed;
                }
                translation[joint][axis] += delta as f32;
            }
        }
    }

    // 5. SECONDARY pass — rotation (identical structure, secondary scale + payload).
    {
        for &var_idx in &active_vars {
            let variant = &dna.variant_records[var_idx];
            let sec_scale = variant.secondary_delta_scale;

            let folds: Vec<f32> = (0..num_joints)
                .map(|j| {
                    (0..mesh_bpp)
                        .map(|fp| joint_weights[j * mesh_bpp + fp] * matrix.cell(var_idx, fp))
                        .sum()
                })
                .collect();

            for rec_result in dna.variant_secondary_deltas(var_idx)? {
                let rec = rec_result?;
                let axis_slot = rec.output_idx as usize;
                if axis_slot >= num_joints * 3 {
                    return Err(MorphError::OutputIdxOutOfRange {
                        output_idx: rec.output_idx,
                        max: num_joints * 3,
                    });
                }
                let joint = axis_slot / 3;
                let axis = axis_slot % 3;
                let fold = folds[joint];
                if fold == 0.0 {
                    continue;
                }

                let mut delta = 0.0_f64;
                for chunk in rec.packed_bytes.chunks_exact(4) {
                    let packed = u32::from_le_bytes(chunk.try_into().unwrap());
                    // packed_low_bits = number of low bits for the index.
                    // decode_packed_delta takes the coef bit width = 31 - packed_low_bits.
                    let (bs_idx, coef) = decode_packed_delta(packed, 31 - packed_low_bits);
                    if (bs_idx as usize) >= num_blendshapes {
                        return Err(MorphError::BlendshapeIdxOutOfRange {
                            bs_idx,
                            num_blendshapes,
                        });
                    }
                    delta += (coef as f64)
                        * sec_scale
                        * (fold as f64)
                        * (activations[bs_idx as usize] as f64)
                        / max_packed;
                }
                rotation_eu[joint][axis] += delta as f32;
            }
        }
    }

    Ok(JointDeltas { translation, rotation_eu })
}

/// Compute per-joint NEUTRAL translation offsets by blending the `.dna`
/// slot-15 per-variant neutral joint offsets with the DNA matrix.
///
/// Unlike [`compute_joint_deltas`] (slot 31, expression-driven, ≈0 at rest),
/// this is the **rest-pose facial identity**: it uses no activation vector
/// and is non-zero at rest. Per character, a different `DnaMatrix2D` yields a
/// different weighted blend of the variant offsets → a different neutral
/// face (via skinning the adjusted skeleton).
///
/// Formula (mirrors the §6.16.6 hypothesis, **centered on variant 0**):
/// ```text
/// base[j]        = slot15[0][j]                         (variant 0 = the protos
///                                                        base = the .skin bind pose)
/// translation[j] = Σ_v fold[j,v] · (slot15[v][j] − base[j])
/// fold[j,v]      = Σ_pool joint_weights[j*bpp + pool] · matrix[v][pool]
/// ```
///
/// The raw per-variant offsets are relative to some authoring base, not the
/// bind pose; adding them raw shoves every face feature forward (Pass 19g).
/// **Variant 0's slot-2 archetype name is `"protos_human_male_face_t1_pu"`
/// — i.e. variant 0 IS the protos base that the head `.skin` bind pose
/// represents.** So the correct reference to subtract is variant 0 (NOT the
/// variant mean — an earlier mistake that left a constant offset visible on
/// the in-game defaults). The selectable heads are variants 1..60 (CHF
/// `head_id` 1..60, direct); variant 0 is the non-selectable neutral base.
///
/// `slot15` is the flat `num_variants * num_joints * 3` f32 buffer from
/// [`starbreaker_dna::DnaFile::variant_neutral_joints`]. Returns
/// [`JointDeltas`] with `rotation_eu` all-zero (slot 15 carries no rotation),
/// suitable for [`super::apply::apply_deltas`].
pub fn compute_neutral_joint_offsets(
    matrix: &DnaMatrix2D,
    joint_weights: &[f32],
    slot15: &[f32],
    num_joints: usize,
) -> Result<JointDeltas, MorphError> {
    let mesh_bpp = matrix.blends_per_pool();
    let num_variants = matrix.num_variants();

    let expected_jw = num_joints * mesh_bpp;
    if joint_weights.len() != expected_jw {
        return Err(MorphError::JointWeightsLengthMismatch {
            expected: expected_jw,
            got: joint_weights.len(),
        });
    }
    let expected_s15 = num_variants * num_joints * 3;
    if slot15.len() < expected_s15 {
        // Reuse the joint-weights mismatch shape isn't ideal; surface a clear
        // error via the same variant for now (length-class failure).
        return Err(MorphError::JointWeightsLengthMismatch {
            expected: expected_s15,
            got: slot15.len(),
        });
    }

    // Reference = variant 0 (the protos base = the .skin bind pose; its
    // slot-2 archetype name is "protos_human_<gender>_face_t1_pu").
    // Subtracting it expresses each character as a deviation from the base
    // the bind pose already represents.
    let base_at = |j: usize, a: usize| slot15[(0 * num_joints + j) * 3 + a];

    let active_vars: Vec<usize> = (0..num_variants)
        .filter(|&v| (0..mesh_bpp).any(|fp| matrix.cell(v, fp) != 0.0))
        .collect();

    let mut translation = vec![[0.0_f32; 3]; num_joints];
    for j in 0..num_joints {
        let b0 = base_at(j, 0);
        let b1 = base_at(j, 1);
        let b2 = base_at(j, 2);
        for &v in &active_vars {
            let fold: f32 = (0..mesh_bpp)
                .map(|fp| joint_weights[j * mesh_bpp + fp] * matrix.cell(v, fp))
                .sum();
            if fold == 0.0 {
                continue;
            }
            let base = (v * num_joints + j) * 3;
            translation[j][0] += fold * (slot15[base] - b0);
            translation[j][1] += fold * (slot15[base + 1] - b1);
            translation[j][2] += fold * (slot15[base + 2] - b2);
        }
    }

    Ok(JointDeltas {
        translation,
        rotation_eu: vec![[0.0_f32; 3]; num_joints],
    })
}

/// Per-character NEUTRAL per-vertex deltas (the bulk face SHAPE) from slot 18,
/// weighted by the DNA-archetype blend matrix.
///
/// This is the carrier of the per-character neutral identity that slot 15
/// (joints, ~cm) and slot 19 (expression, zero at rest) do NOT provide. The
/// slot-18 codec (Pass 19N, `docs/riglogic/PHASE_4B_INVESTIGATION.md` §6.16.24)
/// stores, per control `c = (variant − 1) × mesh_blends_per_pool + region`, a
/// per-vertex affine `code · scale + bias`; this folds them by
/// `coef[c] = matrix.cell(variant, region)` into a per-vertex delta to add to
/// the base mesh. Validated visually across the full male+female archetype
/// pools and on real CHFs (clean, distinct faces).
///
/// `morphs` is [`starbreaker_dna::DnaFile::neutral_morphs`]; returns a
/// `num_vertices`-length `Vec<[f32; 3]>` (metres, CryEngine space).
pub fn compute_neutral_vertex_deltas(
    morphs: &starbreaker_dna::NeutralMorphs,
    matrix: &DnaMatrix2D,
) -> Vec<[f32; 3]> {
    // The mesh's LOCAL regions (all 13 for the head; EYE_L/EYE_R for the eyes,
    // NOSE_C/MOUTH_C for the teeth) and their global face parts.
    let regions = morphs.regions();
    let num_variants = matrix.num_variants();
    let num_controls = morphs.num_controls();
    let mut coef = vec![0.0f32; num_controls];
    // Variant 0 is the base (= the .skin bind pose); controls cover variants
    // 1..num_variants, variant-major.
    for var in 1..num_variants {
        for (l, &reg) in regions.iter().enumerate() {
            let c = (var - 1) * regions.len() + l;
            if c < num_controls {
                coef[c] = matrix.cell(var, reg);
            }
        }
    }
    morphs.decode(&coef)
}

#[cfg(test)]
mod tests {
    use super::*;
    use starbreaker_dna::{
        DnaFile, MAGIC, POINTER_TABLE_OFFSET, VARIANT_ARRAY_SLOT, VARIANT_RECORD_STRIDE,
    };

    const MIN_FILE_SIZE: usize = 0x140;

    /// Encode one variant-delta record into bytes.
    fn encode_record(output_idx: u16, packed: &[u32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + packed.len() * 4);
        out.extend_from_slice(&output_idx.to_le_bytes());
        out.extend_from_slice(&(packed.len() as u16).to_le_bytes());
        for &p in packed {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out
    }

    /// Pack (bs_idx, signed_coef) into a u32 with low_bits = 9.
    fn pack(bs_idx: u16, signed_coef: i32) -> u32 {
        let idx = (bs_idx as u32) & 0x1FF; // low 9 bits
        let coef = ((signed_coef as u32) << 9) & 0xFFFFFE00; // high 23 bits
        idx | coef
    }

    /// Build a synthetic .dna with slot 31 (variants) populated.
    /// `variants` entries are `(num_main, num_sec, pri_scale, sec_scale,
    /// main_payload, sec_payload)`.
    /// `mesh_bpp` is written as a single byte to offset 0x28 per the u8 anchor.
    fn build_dna_with_variants(
        variants: &[(u32, u32, f64, f64, Vec<u8>, Vec<u8>)],
        mesh_bpp: u8,
    ) -> Vec<u8> {
        let nv = variants.len() as u16;
        let body_off = MIN_FILE_SIZE;
        let header_bytes = nv as usize * VARIANT_RECORD_STRIDE;
        let total_payload: usize = variants.iter().map(|(_, _, _, _, m, s)| m.len() + s.len()).sum();
        let total = body_off + header_bytes + total_payload;

        let mut buf = vec![0u8; total];
        buf[..16].copy_from_slice(&MAGIC);
        // mesh_blends_per_pool at 0x28 (u8 field per Task 1 correction).
        buf[0x28] = mesh_bpp;
        buf[0x2a..0x2c].copy_from_slice(&nv.to_le_bytes());

        // Slot 31 → body_off.
        let slot31_off = POINTER_TABLE_OFFSET + VARIANT_ARRAY_SLOT * 8;
        buf[slot31_off..slot31_off + 8].copy_from_slice(&(body_off as u64).to_le_bytes());

        let mut payload_cursor = body_off + header_bytes;
        for (i, (nm, ns, pscale, sscale, mpay, spay)) in variants.iter().enumerate() {
            let hdr_off = body_off + i * VARIANT_RECORD_STRIDE;
            buf[hdr_off..hdr_off + 4].copy_from_slice(&nm.to_le_bytes());
            buf[hdr_off + 4..hdr_off + 8].copy_from_slice(&ns.to_le_bytes());
            buf[hdr_off + 8..hdr_off + 16].copy_from_slice(&pscale.to_le_bytes());
            buf[hdr_off + 16..hdr_off + 24].copy_from_slice(&sscale.to_le_bytes());

            let main_off = payload_cursor;
            buf[main_off..main_off + mpay.len()].copy_from_slice(mpay);
            payload_cursor += mpay.len();

            let sec_off = payload_cursor;
            buf[sec_off..sec_off + spay.len()].copy_from_slice(spay);
            payload_cursor += spay.len();

            buf[hdr_off + 24..hdr_off + 32].copy_from_slice(&(main_off as u64).to_le_bytes());
            buf[hdr_off + 32..hdr_off + 40].copy_from_slice(&(sec_off as u64).to_le_bytes());
        }
        buf
    }

    // Helper: cell_mut is pub(crate); we're in the same crate so direct access works.
    fn matrix_cell_mut(m: &mut DnaMatrix2D, var: usize, fp: usize) -> &mut f32 {
        m.cell_mut(var, fp)
    }

    #[test]
    fn empty_matrix_produces_zero_deltas() {
        let matrix = DnaMatrix2D::zeros(1, 3);
        let joint_weights = vec![1.0_f32, 0.0, 0.0]; // 1 joint × 3 face-parts
        let buf = build_dna_with_variants(
            &[(1, 0, 1.0, 1.0, encode_record(0, &[pack(0, 100)]), Vec::new())],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let activations = vec![1.0_f32; 10];

        let deltas = compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9)
            .expect("ok");
        assert_eq!(deltas.translation, vec![[0.0_f32; 3]]);
        assert_eq!(deltas.rotation_eu, vec![[0.0_f32; 3]]);
    }

    // Dequantization divisor used by the formula. With packed_low_bits = 9,
    // bit_width = 22, max_packed = (1 << 22) - 1 = 4_194_303.
    const MAX_PACKED_22: f64 = 4_194_303.0;

    #[test]
    fn neutral_offsets_blend_variants_by_fold() {
        // 2 variants × 1 joint × 3 face-parts. joint_weights[0..3] = [1,0,0].
        // matrix[v0,pool0]=0.25, matrix[v1,pool0]=0.75. fold[j,v0]=0.25, [j,v1]=0.75.
        // slot15: v0 joint0=(1,2,3), v1 joint0=(10,20,30). base = variant 0 = (1,2,3).
        // Variant-0-centered: 0.25*(v0-base) + 0.75*(v1-base)
        //   = 0.25*(0,0,0) + 0.75*(9,18,27) = (6.75, 13.5, 20.25).
        let mut matrix = DnaMatrix2D::zeros(2, 3);
        *matrix.cell_mut(0, 0) = 0.25;
        *matrix.cell_mut(1, 0) = 0.75;
        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let slot15 = vec![1.0_f32, 2.0, 3.0, 10.0, 20.0, 30.0]; // [v][j][xyz]
        let d = compute_neutral_joint_offsets(&matrix, &joint_weights, &slot15, 1).expect("ok");
        assert!((d.translation[0][0] - 6.75).abs() < 1e-5, "{:?}", d.translation[0]);
        assert!((d.translation[0][1] - 13.5).abs() < 1e-5);
        assert!((d.translation[0][2] - 20.25).abs() < 1e-5);
        assert_eq!(d.rotation_eu, vec![[0.0_f32; 3]]);
    }

    #[test]
    fn neutral_offsets_differ_for_different_matrices() {
        // Variant-0-centered (base = v0 = (1,2,3)): full weight on v0 →
        // (v0-base) = 0; on v1 → (v1-base) = (9,18,27). Distinct.
        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let slot15 = vec![1.0_f32, 2.0, 3.0, 10.0, 20.0, 30.0];
        let mut a = DnaMatrix2D::zeros(2, 3);
        *a.cell_mut(0, 0) = 1.0;
        let mut b = DnaMatrix2D::zeros(2, 3);
        *b.cell_mut(1, 0) = 1.0;
        let da = compute_neutral_joint_offsets(&a, &joint_weights, &slot15, 1).unwrap();
        let db = compute_neutral_joint_offsets(&b, &joint_weights, &slot15, 1).unwrap();
        assert_ne!(da.translation, db.translation);
        assert_eq!(da.translation[0], [0.0, 0.0, 0.0]);
        assert_eq!(db.translation[0], [9.0, 18.0, 27.0]);
    }

    #[test]
    fn single_variant_primary_record_writes_translation() {
        // 1 variant, primary_delta_scale=1.0, 1 main record (output_idx=0
        // → joint 0 axis x), packed = [(bs=5, coef=100)]. matrix[0,0]=1.0.
        // joint_weights[0..3]=[1.0, 0, 0]. activations[5]=1.0.
        //
        // Expected (Pass 8 anchored, no /max_pri_scale normalisation):
        //   delta = coef * primary_delta_scale * fold * activation / max_packed
        //         = 100 * 1.0 * 1.0 * 1.0 / 8388607 ≈ 1.192e-5.
        let mut matrix = DnaMatrix2D::zeros(1, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;

        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(
            &[(1, 0, 1.0, 1.0, encode_record(0, &[pack(5, 100)]), Vec::new())],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let mut activations = vec![0.0_f32; 10];
        activations[5] = 1.0;

        let deltas =
            compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9)
                .expect("ok");
        let expected = (100.0_f64 / MAX_PACKED_22) as f32;
        assert!(
            (deltas.translation[0][0] - expected).abs() < 1e-9,
            "translation[0][0] = {}, expected ~{expected}",
            deltas.translation[0][0]
        );
        assert_eq!(deltas.translation[0][1], 0.0);
        assert_eq!(deltas.translation[0][2], 0.0);
        assert_eq!(deltas.rotation_eu, vec![[0.0_f32; 3]]);
    }

    #[test]
    fn single_variant_secondary_record_writes_rotation() {
        // Same as primary test but SECONDARY payload + secondary_delta_scale.
        let mut matrix = DnaMatrix2D::zeros(1, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;

        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(
            &[(0, 1, 1.0, 1.0, Vec::new(), encode_record(0, &[pack(5, 100)]))],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let mut activations = vec![0.0_f32; 10];
        activations[5] = 1.0;

        let deltas =
            compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9)
                .expect("ok");
        let expected = (100.0_f64 / MAX_PACKED_22) as f32;
        assert!(
            (deltas.rotation_eu[0][0] - expected).abs() < 1e-9,
            "rotation_eu[0][0] = {}, expected ~{expected}",
            deltas.rotation_eu[0][0]
        );
        assert_eq!(deltas.translation, vec![[0.0_f32; 3]]);
    }

    #[test]
    fn fold_uses_joint_weights() {
        // joint_weights[0..3] = [0.5, 0.5, 0]; matrix[0,0]=1.0.
        // fold = 0.5 * 1.0 + 0.5 * 0.0 = 0.5.
        // delta = 100 * 1.0 * 0.5 * 1.0 / 8388607 ≈ 5.96e-6.
        let mut matrix = DnaMatrix2D::zeros(1, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;

        let joint_weights = vec![0.5_f32, 0.5, 0.0];
        let buf = build_dna_with_variants(
            &[(1, 0, 1.0, 1.0, encode_record(0, &[pack(5, 100)]), Vec::new())],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let mut activations = vec![0.0_f32; 10];
        activations[5] = 1.0;

        let deltas =
            compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9)
                .expect("ok");
        let expected = (100.0_f64 * 0.5 / MAX_PACKED_22) as f32;
        assert!(
            (deltas.translation[0][0] - expected).abs() < 1e-9,
            "translation[0][0] = {}, expected ~{expected}",
            deltas.translation[0][0]
        );
    }

    #[test]
    fn activations_modulate_contribution() {
        // Same as single-variant test but activations[5] = 0.5.
        // delta = 100 * 1.0 * 1.0 * 0.5 / 8388607 ≈ 5.96e-6.
        let mut matrix = DnaMatrix2D::zeros(1, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;

        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(
            &[(1, 0, 1.0, 1.0, encode_record(0, &[pack(5, 100)]), Vec::new())],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let mut activations = vec![0.0_f32; 10];
        activations[5] = 0.5;

        let deltas =
            compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9)
                .expect("ok");
        let expected = (100.0_f64 * 0.5 / MAX_PACKED_22) as f32;
        assert!(
            (deltas.translation[0][0] - expected).abs() < 1e-9,
            "translation[0][0] = {}, expected ~{expected}",
            deltas.translation[0][0]
        );
    }

    #[test]
    fn raw_scales_sum_across_active_variants() {
        // Pass 8 anchored that variants use RAW primary_delta_scale, NOT
        // normalised by max_pri_scale. So two variants with scales 1.0 and
        // 2.0 contribute proportionally:
        //   variant 0: 100 * 1.0 * 1.0 * 1.0 / 8388607
        //   variant 1: 100 * 2.0 * 1.0 * 1.0 / 8388607
        //   total: 100 * (1.0 + 2.0) / 8388607 = 300/8388607 ≈ 3.576e-5.
        let mut matrix = DnaMatrix2D::zeros(2, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;
        *matrix_cell_mut(&mut matrix, 1, 0) = 1.0;

        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(
            &[
                (1, 0, 1.0, 1.0, encode_record(0, &[pack(5, 100)]), Vec::new()),
                (1, 0, 2.0, 1.0, encode_record(0, &[pack(5, 100)]), Vec::new()),
            ],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let mut activations = vec![0.0_f32; 10];
        activations[5] = 1.0;

        let deltas =
            compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9)
                .expect("ok");
        let expected = (300.0_f64 / MAX_PACKED_22) as f32;
        assert!(
            (deltas.translation[0][0] - expected).abs() < 1e-9,
            "translation[0][0] = {}, expected ~{expected}",
            deltas.translation[0][0]
        );
    }

    #[test]
    fn joint_weights_length_mismatch_errors() {
        let matrix = DnaMatrix2D::zeros(1, 3);
        let joint_weights = vec![1.0_f32, 0.0]; // wrong length — should be 3
        let buf = build_dna_with_variants(&[(0, 0, 1.0, 1.0, Vec::new(), Vec::new())], 3);
        let dna = DnaFile::read(&buf).expect("parses");
        let activations = vec![0.0_f32; 10];

        let result = compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9);
        assert_eq!(
            result,
            Err(MorphError::JointWeightsLengthMismatch {
                expected: 3,
                got: 2
            })
        );
    }

    #[test]
    fn activations_length_mismatch_errors() {
        let matrix = DnaMatrix2D::zeros(1, 3);
        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(&[(0, 0, 1.0, 1.0, Vec::new(), Vec::new())], 3);
        let dna = DnaFile::read(&buf).expect("parses");
        let activations = vec![0.0_f32; 5]; // wrong length — should be 10

        let result = compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9);
        assert_eq!(
            result,
            Err(MorphError::ActivationsLengthMismatch {
                expected: 10,
                got: 5
            })
        );
    }

    #[test]
    fn bs_idx_out_of_range_errors() {
        // Packed entry with bs_idx = 100, but num_blendshapes = 10.
        let mut matrix = DnaMatrix2D::zeros(1, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;
        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(
            &[(1, 0, 1.0, 1.0, encode_record(0, &[pack(100, 1)]), Vec::new())],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let activations = vec![1.0_f32; 10];

        let result = compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9);
        assert!(
            matches!(
                result,
                Err(MorphError::BlendshapeIdxOutOfRange {
                    bs_idx: 100,
                    num_blendshapes: 10
                })
            ),
            "got {result:?}"
        );
    }

    #[test]
    fn output_idx_out_of_range_errors() {
        // Record with output_idx = 999, but num_joints = 1 (so max = 3).
        let mut matrix = DnaMatrix2D::zeros(1, 3);
        *matrix_cell_mut(&mut matrix, 0, 0) = 1.0;
        let joint_weights = vec![1.0_f32, 0.0, 0.0];
        let buf = build_dna_with_variants(
            &[(1, 0, 1.0, 1.0, encode_record(999, &[pack(0, 1)]), Vec::new())],
            3,
        );
        let dna = DnaFile::read(&buf).expect("parses");
        let activations = vec![1.0_f32; 10];

        let result = compute_joint_deltas(&matrix, &joint_weights, &dna, &activations, 1, 10, 9);
        assert!(
            matches!(
                result,
                Err(MorphError::OutputIdxOutOfRange {
                    output_idx: 999,
                    max: 3
                })
            ),
            "got {result:?}"
        );
    }
}
