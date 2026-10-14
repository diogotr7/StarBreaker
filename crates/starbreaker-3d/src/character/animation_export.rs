//! Builds a looping facial-expression animation ([`ExpressionAnim`]: per-bone
//! local TRS + morph-weight + wrinkle tracks) by sampling the existing pose
//! pipeline, and the pruned slot-19-folded morph targets that go with it.
//! [`ExpressionAnim::to_clip`] turns it into the shared model's
//! [`crate::clip::Clip`]; the writers live elsewhere.
//!
//! Key functions:
//! - [`assemble_anim`] — pure prune + pack: drops dead blendshapes (zero weight
//!   every frame) and still bones (TRS equal to frame 0 every frame), then packs
//!   into an [`ExpressionAnim`] + pruned [`BlendShapes`].
//! - [`build_expression_anim`] — the p4k-fed producer: evaluates the pose
//!   pipeline per frame (CAF sample → correctives → joint deltas → skeleton),
//!   folds slot-19 into each morph target, and feeds [`assemble_anim`].

use crate::character::blendshapes::{BlendShapes, MorphTarget};

/// One animated bone: local (parent-relative) TRS keyframes in CryEngine
/// space, one per sample frame; rotations `[w, x, y, z]`. `node` is the bone's
/// index in the head skeleton.
pub struct ExpressionBoneTrack {
    pub node: usize,
    pub translations: Vec<[f32; 3]>,
    pub rotations: Vec<[f32; 4]>,
}

/// A single looping animation: shared sample times (seconds), per-bone TRS
/// tracks, and an optional flat morph-weights track (`frames × num_targets`,
/// row-major by frame; target `k` is the `k`-th pruned blendshape).
pub struct ExpressionAnim {
    pub times: Vec<f32>,
    pub bone_tracks: Vec<ExpressionBoneTrack>,
    pub weights: Option<Vec<f32>>,
    /// Per-frame HumanSkin_V2 `PI_WrinklesMask` weights (one 24-vector per
    /// entry of `times`; see [`super::wrinkles`]), when the rig drives them.
    pub wrinkles: Option<Vec<[f32; super::wrinkles::NUM_WRINKLE_SLOTS]>>,
}

impl ExpressionAnim {
    /// The shared-model clip `name`: bone tracks by `bone_names` (the head
    /// skeleton, in order), morph tracks named like
    /// `shared::morph_targets` names the pruned blendshapes (`bs_NNN` by
    /// position), and the wrinkle weights as a `PI_WrinklesMask` track.
    pub(crate) fn to_clip(&self, name: &str, bone_names: &[String]) -> crate::clip::Clip {
        use crate::clip::{BoneTrack, Keys, MaterialParamTrack, ScalarTrack};
        fn keys<T>(times: &[f32], values: Vec<T>) -> Keys<T> {
            Keys { times: times.to_vec(), values }
        }
        let bones = self
            .bone_tracks
            .iter()
            .filter_map(|t| {
                Some(BoneTrack {
                    bone: bone_names.get(t.node)?.clone(),
                    rotation: Some(keys(&self.times, t.rotations.clone())),
                    translation: Some(keys(&self.times, t.translations.clone())),
                })
            })
            .collect();
        let frames = self.times.len().max(1);
        let morphs = self.weights.as_ref().map_or_else(Vec::new, |w| {
            let targets = w.len() / frames;
            (0..targets)
                .map(|k| ScalarTrack {
                    name: format!("bs_{k:03}"),
                    keys: keys(&self.times, (0..self.times.len()).map(|f| w[f * targets + k]).collect()),
                })
                .collect()
        });
        let material_params = self
            .wrinkles
            .as_ref()
            .map(|frames| {
                use super::wrinkles::SLOT_LAYOUT;
                let slots: Vec<serde_json::Value> = SLOT_LAYOUT
                    .iter()
                    .map(|s| serde_json::json!({"panel": s.panel, "channel": s.channel, "band": s.band, "tile": s.tile}))
                    .collect();
                MaterialParamTrack {
                    shader: "HumanSkin_V2 %WRINKLE_BLENDING_2".into(),
                    param: "PI_WrinklesMask".into(),
                    info: serde_json::json!({ "slots": slots }),
                    keys: keys(&self.times, frames.iter().map(|f| f.to_vec()).collect()),
                }
            })
            .into_iter()
            .collect();
        crate::clip::Clip { name: name.into(), bones, morphs, material_params }
    }
}

/// What pruning dropped (logged, never silent).
pub struct PruneReport {
    pub active_blendshapes: usize,
    pub total_blendshapes: usize,
    pub active_bones: usize,
    pub total_bones: usize,
}

/// Cap on sample frames in the looping expression animation (the clip is
/// sampled at ~its native rate up to this many frames; keyframe data is tiny
/// next to the morph-target basis, so a high cap is cheap and keeps motion
/// smooth).
pub const EXPRESSION_ANIM_FRAMES: usize = 120;
/// Frame rate of CryEngine face idle clips. The loop's duration is its frame
/// length / this, so it plays at the authored speed.
pub const FACE_ANIM_FPS: f32 = 30.0;
/// Default expression-clip weight (engine `FaceAnimDef` clip weight). The
/// engine accumulates `_Dst[track] += sampled_value · clip_weight` per clip
/// (`CFaceRigLogic::ExecuteHuman`, `FUN_146e19c40`); we synthesize a single
/// idle clip, so this is its blend weight. Now that the CAF controller decode
/// is correct (`value = u16·scale + bias`, real keytimes — see
/// [`crate::character::face_anim`]), the sampled controls are already at their
/// true subtle magnitudes, so the faithful weight is 1.0. The builder exposes
/// this as a slider for artistic exaggeration/damping.
pub const EXPRESSION_DEFAULT_INTENSITY: f32 = 1.0;

/// Prune dead blendshapes (zero weight every frame) and still bones (TRS equal
/// to frame 0 every frame), then pack into an [`ExpressionAnim`] + pruned
/// [`BlendShapes`].
///
/// `base_targets[b]` is blendshape `b`'s (already slot-19-folded) morph target.
/// `weights_per_frame[frame][b]` is blendshape `b`'s activation weight at
/// `frame`; `bone_trs_per_frame[frame][bone]` is the bone's local TRS
/// (CryEngine space, rotation `[w, x, y, z]`). The returned `BlendShapes` keeps only the active targets, and the
/// `ExpressionAnim`'s flat `weights` track is re-packed to match (frame-major).
pub(crate) fn assemble_anim(
    times: Vec<f32>,
    weights_per_frame: &[Vec<f32>],
    bone_trs_per_frame: &[Vec<([f32; 3], [f32; 4])>],
    base_targets: Vec<MorphTarget>,
    num_vertices: usize,
) -> (ExpressionAnim, BlendShapes, PruneReport) {
    let frames = times.len();
    let total_bs = base_targets.len();
    let total_bones = bone_trs_per_frame.first().map(|f| f.len()).unwrap_or(0);

    let active_bs: Vec<usize> = (0..total_bs)
        .filter(|&b| {
            weights_per_frame
                .iter()
                .any(|w| w.get(b).copied().unwrap_or(0.0).abs() > 1e-6)
        })
        .collect();
    let active_bones: Vec<usize> = (0..total_bones)
        .filter(|&bone| {
            let f0 = bone_trs_per_frame[0][bone];
            bone_trs_per_frame.iter().any(|f| {
                let (t, r) = f[bone];
                t.iter().zip(f0.0).any(|(a, b)| (a - b).abs() > 1e-7)
                    || r.iter().zip(f0.1).any(|(a, b)| (a - b).abs() > 1e-7)
            })
        })
        .collect();

    let pruned_targets: Vec<MorphTarget> = active_bs
        .iter()
        .map(|&b| MorphTarget {
            positions: base_targets[b].positions.clone(),
        })
        .collect();
    let mut weights = Vec::with_capacity(frames * active_bs.len());
    for fr in 0..frames {
        for &b in &active_bs {
            weights.push(weights_per_frame[fr].get(b).copied().unwrap_or(0.0));
        }
    }
    let bone_tracks: Vec<ExpressionBoneTrack> = active_bones
        .iter()
        .map(|&bone| ExpressionBoneTrack {
            node: bone,
            translations: bone_trs_per_frame.iter().map(|f| f[bone].0).collect(),
            rotations: bone_trs_per_frame.iter().map(|f| f[bone].1).collect(),
        })
        .collect();

    let report = PruneReport {
        active_blendshapes: active_bs.len(),
        total_blendshapes: total_bs,
        active_bones: active_bones.len(),
        total_bones,
    };
    log::info!(
        "expression anim: {}/{} blendshapes, {}/{} bones kept after prune",
        report.active_blendshapes,
        report.total_blendshapes,
        report.active_bones,
        report.total_bones,
    );
    let pruned = BlendShapes {
        num_blendshapes: pruned_targets.len() as u32,
        num_vertices: num_vertices as u32,
        targets: pruned_targets,
    };
    let weights = if active_bs.is_empty() {
        None
    } else {
        Some(weights)
    };
    (
        ExpressionAnim {
            times,
            bone_tracks,
            weights,
            wrinkles: None,
        },
        pruned,
        report,
    )
}

use crate::character::dna_matrix::DnaMatrix2D;
use crate::character::face_anim::CafClip;
use crate::character::{apply, expression, head_skin, morphing, CharacterError};
use crate::riglogic::RigLogic;
use starbreaker_dna::DnaFile;

/// Evaluate the pose pipeline at `frames` uniform times over `duration_s` and
/// produce the looping animation + pruned, slot-19-folded morph targets.
///
/// Per frame: sample the CAF clip → compose Section C correctives → fold the
/// DNA matrix + slot-31 variants into per-joint deltas → apply to the rest
/// skeleton → record the bones' local TRS and the activation weights. After the
/// loop, each base morph target is offset by its per-character slot-19
/// correction (the character's blended blendshape minus the base head's) and
/// the whole set is pruned + packed via [`assemble_anim`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_expression_anim(
    rig: &RigLogic,
    dna_bytes: &[u8],
    base_head: &head_skin::HeadSkin,
    matrix: &DnaMatrix2D,
    clip: &CafClip,
    base_bshapes: &BlendShapes,
    frames: usize,
    duration_s: f32,
    intensity: f32,
) -> Result<(ExpressionAnim, BlendShapes, PruneReport), CharacterError> {
    let dna_file = DnaFile::read(dna_bytes)?;
    let num_vertices = base_head.mesh.positions.len();
    let mesh_bpp = dna_file.header.mesh_blends_per_pool as usize;
    let joint_weights = dna_file.joint_weights(rig.num_joints as usize)?;
    let morphs = dna_file.expression_morphs()?;
    let region_weights = dna_file.region_weights_with_num_vertices(num_vertices)?;

    let times: Vec<f32> = (0..frames)
        .map(|i| duration_s * i as f32 / (frames.max(2) - 1) as f32)
        .collect();
    let mut weights_per_frame: Vec<Vec<f32>> = Vec::with_capacity(frames);
    let mut bone_trs_per_frame: Vec<Vec<([f32; 3], [f32; 4])>> = Vec::with_capacity(frames);
    let mut wrinkles_per_frame = Vec::with_capacity(frames);
    for i in 0..frames {
        let t = if frames <= 1 {
            0.0
        } else {
            i as f32 / (frames - 1) as f32
        };
        // Engine `_Dst[track] += sampled · clip_weight`: scale the sampled
        // appearance controls by the clip weight (`intensity`) before composing
        // correctives. CTL_PSD (index 0) is the always-on baseline, never scaled.
        let mut appearance = clip.sample(t);
        for a in appearance.iter_mut().skip(1) {
            *a *= intensity;
        }
        let activations = crate::riglogic::compose_correctives(rig, &appearance)?;
        // Index alignment guard (spec): GPU blendshape count must match activations.
        if base_bshapes.num_blendshapes as usize != activations.len() {
            return Err(CharacterError::GlbEmit(format!(
                "blendshape/activation mismatch: bshapes={}, activations={}",
                base_bshapes.num_blendshapes,
                activations.len()
            )));
        }
        let deltas = morphing::compute_joint_deltas(
            matrix,
            &joint_weights,
            &dna_file,
            &activations,
            rig.num_joints as usize,
            activations.len(),
            9, // packed_low_bits for the human face rig (9-bit bs_idx / 23-bit coef)
        )?;
        let modified = apply::apply_deltas(rig, &deltas, &base_head.skeleton)?;
        let trs: Vec<([f32; 3], [f32; 4])> = modified
            .bones
            .iter()
            .map(|b| {
                let [x, y, z, w] = b.bind_rotation;
                (b.bind_translation, [w, x, y, z])
            })
            .collect();
        bone_trs_per_frame.push(trs);
        wrinkles_per_frame.push(super::wrinkles::wrinkle_slot_weights(rig, &activations));
        weights_per_frame.push(activations);
    }

    let mut folded: Vec<MorphTarget> = Vec::with_capacity(base_bshapes.targets.len());
    for (b, base) in base_bshapes.targets.iter().enumerate() {
        let fold = expression::fold_blendshape_slot19(
            &morphs,
            &region_weights,
            mesh_bpp,
            matrix,
            b,
            num_vertices,
        );
        // Morph targets are POSITION DELTAS (the GLB builder adds them straight
        // onto the base mesh). The slot-18 per-character identity lives ONLY on
        // the base mesh — adding it here would re-apply it once per active morph.
        let positions = base
            .positions
            .iter()
            .zip(&fold)
            .map(|(p, d)| [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
            .collect();
        folded.push(MorphTarget { positions });
    }
    let (mut anim, pruned, report) =
        assemble_anim(times, &weights_per_frame, &bone_trs_per_frame, folded, num_vertices);
    anim.wrinkles = Some(wrinkles_per_frame);
    Ok((anim, pruned, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::blendshapes::MorphTarget;

    #[test]
    fn assemble_prunes_dead_blendshapes_and_still_bones() {
        let weights_per_frame = vec![vec![0.2f32, 0.0], vec![0.6, 0.0]];
        let bone_trs_per_frame = vec![
            vec![
                ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
                ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
            ],
            vec![
                ([0.0, 0.1, 0.0], [1.0, 0.0, 0.0, 0.0]),
                ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
            ],
        ];
        let base_targets = vec![
            MorphTarget {
                positions: vec![[1.0, 0.0, 0.0]],
            },
            MorphTarget {
                positions: vec![[9.0, 9.0, 9.0]],
            },
        ];
        let (anim, pruned, report) = assemble_anim(
            vec![0.0, 1.0],
            &weights_per_frame,
            &bone_trs_per_frame,
            base_targets,
            1,
        );
        assert_eq!(report.active_blendshapes, 1);
        assert_eq!(report.total_blendshapes, 2);
        assert_eq!(report.active_bones, 1);
        assert_eq!(report.total_bones, 2);
        assert_eq!(pruned.targets.len(), 1);
        assert_eq!(anim.bone_tracks.len(), 1);
        assert_eq!(anim.bone_tracks[0].node, 0);
        let w = anim.weights.as_ref().unwrap();
        assert_eq!(w.len(), 2);
        assert!((w[0] - 0.2).abs() < 1e-6);
        assert!((w[1] - 0.6).abs() < 1e-6);
    }
}
