//! Loaded-once head rig assets (`HeadAssets`) for repeated per-character
//! builds.
//!
//! `HeadAssets::load` performs the p4k reads + parsing (CDF, head .skin/.skinm,
//! master .dna, RigLogic, co-located sub-meshes, and the head and sub-meshes
//! as the entity pipeline loads them — [`PartSource`]) exactly once;
//! `HeadAssets::rig_state` runs the per-CHF DNA matrix + neutral identity into
//! a rest-pose [`shared::CharacterRig`]; `expression_rig_state` is the same rig
//! playing a face clip. `HeadIdentityContext` wraps `HeadAssets` as a
//! gender-keyed cache for repeated GLB builds (the app's character builder).

use starbreaker_chf::Dna;
use starbreaker_common::NameHash;
use starbreaker_dna::DnaFile;
use starbreaker_p4k::MappedP4k;

use super::{
    apply, cdf, controls, dna_matrix, face_anim, head_skin, identity, load_head_submeshes, HeadSubMesh,
    clips::CharacterAnimation, items::HeadItemMesh, morphing, outfit::Outfit, normals, p4k_read, part_source::PartSource, read_bshapes_gpu_from_skinm, shared,
    submesh, CharacterError,
};
use crate::riglogic::RigLogic;
use std::sync::Arc;

/// Head rig assets loaded once and reused across many per-character renders.
pub struct HeadAssets {
    head: head_skin::HeadSkin,
    rig: RigLogic,
    /// Raw master `.dna` bytes. `DnaFile<'a>` borrows its backing buffer, so we
    /// keep the bytes and re-`DnaFile::read` them per rig
    /// (cheap; parsing is just a header + pointer-table scan) rather than store
    /// a self-referential `DnaFile`.
    dna_bytes: Vec<u8>,
    /// Raw `.skinm` buffer retained for `read_bshapes_gpu_from_skinm` per rig.
    skinm_bytes: Vec<u8>,
    submeshes: Vec<HeadSubMesh>,
    head_skin_path: String,
    /// The head as the entity pipeline loads it (with the `.skin`'s own
    /// material); every rig reshapes it.
    head_source: Arc<PartSource>,
}

impl HeadAssets {
    /// Load the head rig assets for `(gender_hash, variant_hash)` from the p4k.
    ///
    /// Performs every p4k read + parse step (CDF, head `.skin` / `.skinm`,
    /// master `.dna`, RigLogic, co-located sub-meshes) exactly once. The
    /// returned value can be reused across many [`HeadAssets::rig_state`] calls
    /// without touching `Data.p4k` again.
    pub fn load(
        p4k: &MappedP4k,
        gender_hash: NameHash,
        variant_hash: NameHash,
    ) -> Result<Self, CharacterError> {
        let paths = identity::resolve_head_rig(gender_hash, variant_hash)?;
        let cdf = cdf::parse_cdf(&p4k_read(p4k, paths.cdf)?)?;
        let skin_bytes = p4k_read(p4k, paths.head_skin)?;
        let skinm_bytes = p4k_read(p4k, &format!("{}m", paths.head_skin))?;
        let head = head_skin::parse_head_skin(&skin_bytes, &skinm_bytes)?;
        let dna_bytes = p4k_read(p4k, paths.dna)?;
        // Validate the `.dna` parses up front (fail-fast); each rig re-reads from `dna_bytes` because `DnaFile<'a>`
        // borrows its buffer.
        DnaFile::read(&dna_bytes)?;
        let rig = RigLogic::read(&head.riglogic_chunk[8..])?;
        let submeshes = load_head_submeshes(p4k, &cdf, paths.head_skin);
        let head_source = Arc::new(PartSource::load(p4k, paths.head_skin, "")?);
        Ok(Self {
            head,
            rig,
            dna_bytes,
            skinm_bytes,
            submeshes,
            head_skin_path: paths.head_skin.to_owned(),
            head_source,
        })
    }

    /// The per-character NEUTRAL skeleton as a re-bound rest pose: slot-15
    /// per-variant parent-local joint translations folded by the DNA matrix
    /// (`morphing::compute_neutral_joint_offsets`), applied via
    /// [`apply::rebind_skeleton`] so the joints move but the rest mesh does not
    /// deform (its shape comes from slot 18). Expression joints then pivot
    /// about the character's own joints, and rig-skinned sub-meshes
    /// (eyes/teeth) — skinned against their own original bind — follow.
    fn neutral_rebound_skeleton(
        &self,
        dna_file: &DnaFile,
        matrix: &dna_matrix::DnaMatrix2D,
    ) -> Result<head_skin::BindPoseSkeleton, CharacterError> {
        let num_joints = self.rig.num_joints as usize;
        let joint_weights = dna_file.joint_weights(num_joints)?;
        let slot15 = dna_file.variant_neutral_joints(num_joints)?;
        let neutral =
            morphing::compute_neutral_joint_offsets(matrix, &joint_weights, &slot15, num_joints)?;
        apply::rebind_skeleton(&self.rig, &neutral, &self.head.skeleton)
    }

    /// One CHF's rig at rest: the per-CHF DNA-matrix pipeline, the neutral
    /// identity (slot-15 joints, slot-18 vertex shape) and the BShapesGpu
    /// decode. Parts (sub-meshes, then `items`) are at the character's rest
    /// pose; a pose reaches them through their skins.
    pub(crate) fn rig_state(
        &self,
        chf_dna: &Dna,
        include_submeshes: bool,
        include_morph_targets: bool,
        items: &[HeadItemMesh],
    ) -> Result<shared::CharacterRig, CharacterError> {
        let mut head = self.head.clone();
        let dna_file = DnaFile::read(&self.dna_bytes)?;

        // 4. DNA matrix pipeline.
        let mesh_bpp = dna_file.header.mesh_blends_per_pool as usize;
        let num_variants = dna_file.variant_records.len();
        let weights_f64 =
            dna_matrix::chf_weights::compute_chf_weights(chf_dna, num_variants, mesh_bpp)?;
        let weights_f32 = dna_matrix::top_k::prune_top_k(&weights_f64, num_variants, mesh_bpp)?;
        let matrix =
            dna_matrix::accumulator::build_dna_matrix(&weights_f32, num_variants, mesh_bpp)?;

        // 5. Rest-pose controls (CTL_PSD baseline), then Section C correctives.
        let appearance_weights = controls::ControlInputs::rest_pose(&self.rig).into_appearance_weights();
        let activations = crate::riglogic::compose_correctives(&self.rig, &appearance_weights)?;
        let num_blendshapes = activations.len();

        // 6. Morph: joint-weights from slot 21, deltas from slot 31 variants.
        // (slot-18 neutral vertex deltas, captured in 6c, are reused in §9 to make
        //  the co-located sub-meshes follow the per-character head shape.)
        let joint_weights = dna_file.joint_weights(self.rig.num_joints as usize)?;
        let deltas = morphing::compute_joint_deltas(
            &matrix,
            &joint_weights,
            &dna_file,
            &activations,
            self.rig.num_joints as usize,
            num_blendshapes,
            9, // packed_low_bits for the human face rig (9-bit bs_idx / 23-bit coef)
        )?;

        // 6b. NEUTRAL per-character identity: blend slot-15 per-variant neutral
        //     joint offsets by the DNA matrix and ADD to the (expression) deltas.
        //     Non-zero at rest, so it differentiates characters in the neutral
        //     pose (slot-31 deltas are ≈0 at rest). See §6.16.6 and
        //     `morphing::compute_neutral_joint_offsets`.
        //
        //     This is the ONLY CPU-reproducible per-character neutral signal:
        //     slot 19/31 + the appearance/corrective blendshapes are all
        //     activation-gated to ~0 at rest (Pass 19k, §6.16.19), and the bulk
        //     per-character shape is GPU-resident. slot 15 is joint-level (a
        //     strong archetype moves face joints ~1.5 cm; typical blended CHFs
        //     average toward the mean), applied faithfully with NO joint
        //     exclusion — it renders clean with proper smooth normals.
        let head_neutral_deltas = {
            // 6b'. slot-15 neutral JOINTS as a RE-BIND (new rest pose, recomputed
            //      inverse binds): the joints move to the character's positions
            //      WITHOUT deforming the mesh. Skinning slot 18's shape by a
            //      displaced-but-not-rebound skeleton double-deforms it; re-binding
            //      avoids that while giving expressions the right pivots.
            head.skeleton = self.neutral_rebound_skeleton(&dna_file, &matrix)?;

            // 6c. NEUTRAL per-character VERTEX identity — the BULK face SHAPE.
            //     slot-18 per-vertex deltas (Pass 19N, §6.16.24), folded by the DNA
            //     matrix and added to the head mesh. This is the dominant identity
            //     (clean, distinct faces verified across the full archetype pools);
            //     slot-15 above handles the skeleton (sub-mesh skinning). The head
            //     `.dna`'s `num_vertices` field is unreliable on female rigs, so the
            //     vertex count comes from the mesh.
            let neutral_morphs = dna_file.neutral_morphs(head.mesh.positions.len())?;
            // The archetype codes carry a small authored left/right asymmetry
            // (~7% residual). Apply them AS AUTHORED — the engine does not
            // mirror-average them (user-confirmed against in-game, 2026-06-12;
            // retracts the Pass-19O "engine symmetrizes" claim), and mirroring
            // also smears one-sided DNA edits onto both halves.
            let vertex_deltas = morphing::compute_neutral_vertex_deltas(&neutral_morphs, &matrix);
            apply_vertex_identity(&mut head.mesh, &vertex_deltas);
            // Keep the deltas so co-located sub-meshes (eyes/teeth/eye-details) can
            // follow the same per-vertex head shape (§9) — otherwise the wrap-layer
            // eye-details float in front of the deformed eye surface.
            vertex_deltas
        };

        // 7. Apply to rest pose.
        let modified = apply::apply_deltas(&self.rig, &deltas, &head.skeleton)?;

        // 8. Decode BShapesGpu from the .skinm chunk and emit morph targets.
        //    Pass 15: modulate by .dna slot 17 (per-vertex × per-bs byte mask).
        //    The full rig has ~400 expression blendshapes (tens of MB of morph
        //    targets). They are useless for a STATIC neutral preview and dominate
        //    both GLB size and three.js parse time, so `include_morph_targets =
        //    false` (the GUI preview path) skips the decode entirely.
        let bshapes = if include_morph_targets {
            let slot17_bytes = dna_file
                .segment_for_slot(17)
                .map(|seg| dna_file.segment_bytes(seg));
            Some(read_bshapes_gpu_from_skinm(&self.skinm_bytes, slot17_bytes)?)
        } else {
            None
        };

        // 9. Co-located CDF sub-meshes (eyes/teeth/eye-details) fill the eye
        //    sockets and mouth, plus the CHF items — shaped to the character at
        //    its rest pose. `include_submeshes = false` yields a clean head-only
        //    rig (for isolating the head shape).
        let parts = if include_submeshes {
            let identity = Some(SubMeshIdentity {
                rest: &head.skeleton,
                head_base: &self.head.mesh.positions,
                head_deltas: &head_neutral_deltas,
            });
            let rest = apply::ModifiedSkeleton { bones: head.skeleton.bones.clone() };
            let meshes = self.pose_submeshes(&matrix, identity, &rest, items)?;
            self.parts(items, meshes)
        } else {
            Vec::new()
        };
        Ok(shared::CharacterRig {
            name: shared::stem(&self.head_skin_path),
            head: head.mesh,
            head_source: self.head_source.clone(),
            rest: head.skeleton,
            posed: modified,
            blendshapes: bshapes,
            parts,
            animations: Vec::new(),
            actor: Vec::new(),
            head_hidden: false,
        })
    }

    /// The parts from their shaped `meshes`, in [`Self::pose_submeshes`]
    /// order (sub-meshes, then `items`): each named after its source file,
    /// with its own skeleton and pipeline source.
    fn parts(&self, items: &[HeadItemMesh], meshes: Vec<head_skin::HeadMesh>) -> Vec<shared::CharacterPart> {
        let submeshes = self.submeshes.iter().map(|s| (shared::stem(&s.skin_path), &s.skeleton, &s.source));
        let items = items.iter().map(|i| (shared::stem(&i.item.geometry), &i.skeleton, &i.source));
        submeshes
            .chain(items)
            .zip(meshes)
            .map(|((name, skeleton, source), mesh)| shared::CharacterPart {
                name,
                mesh,
                skeleton: skeleton.clone(),
                source: source.clone(),
                hidden_zones: Default::default(),
            })
            .collect()
    }

    /// The rig of [`Self::rig_state`] playing a `.caf` face clip: rest pose,
    /// the expression's pruned slot-19-folded blendshapes, and the clip
    /// ("expression") that animates them and the skeleton, at the clip's
    /// authored speed. `caf_chunk` is the `#caf` chunk payload; `intensity`
    /// is the clip weight.
    pub(crate) fn expression_rig_state(
        &self,
        chf_dna: &Dna,
        caf_chunk: &[u8],
        intensity: f32,
        items: &[HeadItemMesh],
    ) -> Result<shared::CharacterRig, CharacterError> {
        let dna_file = DnaFile::read(&self.dna_bytes)?;
        let mesh_bpp = dna_file.header.mesh_blends_per_pool as usize;
        let num_variants = dna_file.variant_records.len();
        let w64 = dna_matrix::chf_weights::compute_chf_weights(chf_dna, num_variants, mesh_bpp)?;
        let w32 = dna_matrix::top_k::prune_top_k(&w64, num_variants, mesh_bpp)?;
        let matrix = dna_matrix::accumulator::build_dna_matrix(&w32, num_variants, mesh_bpp)?;

        let slot17 = dna_file.segment_for_slot(17).map(|s| dna_file.segment_bytes(s));
        let base_bshapes = read_bshapes_gpu_from_skinm(&self.skinm_bytes, slot17)?;
        let clip = face_anim::CafClip::decode(caf_chunk, &self.rig)
            .map_err(|e| CharacterError::FaceAnim(format!("{e}")))?;

        // Per-character NEUTRAL VERTEX identity (slot-18), same as `render` §6c —
        // the dominant per-character face shape. Applied as authored (no
        // mirror-averaging — see `render` §6c).
        let mut head = self.head.clone();
        let neutral_morphs = dna_file.neutral_morphs(head.mesh.positions.len())?;
        let vertex_deltas = morphing::compute_neutral_vertex_deltas(&neutral_morphs, &matrix);
        // Per-character slot-15 joints as the re-bound REST skeleton: the
        // animation's per-frame local TRS and the inverse binds are both built
        // on it, so expressions pivot about the character's own joints.
        head.skeleton = self.neutral_rebound_skeleton(&dna_file, &matrix)?;

        // Play the loop at its authored speed; sample densely (capped) for smooth
        // playback. Keyframe data is tiny next to the morph-target basis.
        let duration_s = (clip.frame_len.max(1) as f32
            / super::animation_export::FACE_ANIM_FPS)
            .max(0.5);
        let frames =
            (clip.frame_len as usize).clamp(24, super::animation_export::EXPRESSION_ANIM_FRAMES);

        // Build the animation; morph targets are slot-18-shifted to match the
        // (also-shifted) base head mesh so their deltas stay pure expression.
        let (anim, pruned_bs, _report) = super::animation_export::build_expression_anim(
            &self.rig,
            &self.dna_bytes,
            &head,
            &matrix,
            &clip,
            &base_bshapes,
            frames,
            duration_s,
            intensity,
        )?;

        // Apply slot-18 to the GLB base head mesh (morph targets are deltas and
        // ride on top, so the identity belongs only here).
        apply_vertex_identity(&mut head.mesh, &vertex_deltas);

        // Rest skeleton — the animation channels drive it per frame.
        let zero = morphing::JointDeltas {
            translation: vec![[0.0; 3]; self.rig.num_joints as usize],
            rotation_eu: vec![[0.0; 3]; self.rig.num_joints as usize],
        };
        let base_skeleton = apply::apply_deltas(&self.rig, &zero, &head.skeleton)?;

        // Sub-meshes (eyes/teeth/eye-details) and items at the per-character
        // neutral identity, at rest as in `render`; the animation moves them
        // through their skins.
        let meshes = self.pose_submeshes(
            &matrix,
            Some(SubMeshIdentity {
                rest: &head.skeleton,
                head_base: &self.head.mesh.positions,
                head_deltas: &vertex_deltas,
            }),
            &base_skeleton,
            items,
        )?;
        let parts = self.parts(items, meshes);
        let clip = anim.to_clip("expression", &shared::joint_names(&head.skeleton));
        Ok(shared::CharacterRig {
            name: shared::stem(&self.head_skin_path),
            head: head.mesh,
            head_source: self.head_source.clone(),
            rest: head.skeleton,
            posed: base_skeleton,
            blendshapes: Some(pruned_bs),
            parts,
            animations: vec![clip],
            actor: Vec::new(),
            head_hidden: false,
        })
    }
}

/// What the neutral identity contributes to sub-mesh posing.
struct SubMeshIdentity<'a> {
    /// The re-bound per-character rest skeleton.
    rest: &'a head_skin::BindPoseSkeleton,
    /// Head positions before / the slot-18 deltas applied (for the wrap layer).
    head_base: &'a [[f32; 3]],
    head_deltas: &'a [[f32; 3]],
}

impl HeadAssets {
    /// Pose the co-located sub-meshes for one character.
    ///
    /// - `WD_Elastic` (eye-details): surface-wrap to the slot-18 head shape.
    /// - `ProtosMisc` with their own `.dna` (eyes, teeth): add that `.dna`'s
    ///   slot-18 deltas for the character (each archetype's own eye/teeth shape
    ///   and placement) and re-point their bind at the re-bound rest, so the
    ///   skeleton only adds motion on top (expressions), never the identity a
    ///   second time.
    /// - Anything else: skinned against its own bind by `posed`.
    /// - CHF head `items` (hair, brows, lashes, beard, scalp — all `WD_*`):
    ///   surface-wrapped like the eye-details.
    ///
    /// Without `identity` every rigged part is skinned against its own bind
    /// and wrapped parts stay at the base head's surface.
    fn pose_submeshes(
        &self,
        matrix: &dna_matrix::DnaMatrix2D,
        identity: Option<SubMeshIdentity<'_>>,
        posed: &apply::ModifiedSkeleton,
        items: &[HeadItemMesh],
    ) -> Result<Vec<head_skin::HeadMesh>, CharacterError> {
        // Output order = input order (sub-meshes, then items), so callers can
        // pair each posed mesh with its source part.
        let mut rigged = Vec::new();
        let mut wrapped = Vec::new();
        let mut slots: Vec<(bool, usize)> = Vec::new(); // (is_wrapped, index in its list)
        for i in items {
            slots.push((true, wrapped.len()));
            wrapped.push((i.mesh.clone(), i.skeleton.clone()));
        }
        let item_slots = slots.split_off(0);
        for sm in &self.submeshes {
            let (mut mesh, mut skeleton) = (sm.mesh.clone(), sm.skeleton.clone());
            if sm.deformer == cdf::DeformerKind::WdElastic {
                slots.push((true, wrapped.len()));
                wrapped.push((mesh, skeleton));
                continue;
            }
            if let (Some(id), Some(dna_bytes)) = (&identity, &sm.dna) {
                let dna = DnaFile::read(dna_bytes)?;
                let morphs = dna.neutral_morphs(mesh.positions.len())?;
                let deltas = morphing::compute_neutral_vertex_deltas(&morphs, matrix);
                apply_vertex_identity(&mut mesh, &deltas);
                submesh::rebind_submesh_to(&mut skeleton, id.rest);
            }
            slots.push((false, rigged.len()));
            rigged.push((mesh, skeleton));
        }
        let skinned = submesh::skin_submeshes_to_head(&rigged, posed);
        if let Some(id) = identity {
            submesh::wrap_submeshes_to_head_deltas(&mut wrapped, id.head_base, id.head_deltas);
        }
        let pick = |&(is_wrapped, i): &(bool, usize)| {
            if is_wrapped { wrapped[i].0.clone() } else { skinned[i].clone() }
        };
        Ok(slots.iter().chain(&item_slots).map(pick).collect())
    }

    /// P4k `.skin` path of the head.
    pub(crate) fn head_skin_path(&self) -> &str {
        &self.head_skin_path
    }

    /// Whether `part` is one of the head's CDF sub-meshes (eyes, teeth, …).
    pub(crate) fn is_head_submesh(&self, part: &shared::CharacterPart) -> bool {
        self.submeshes.iter().any(|s| Arc::ptr_eq(&s.source, &part.source))
    }
}

/// Add slot-18 per-vertex identity deltas to a mesh and carry the
/// authored normals onto the new shape ([`normals::transfer_normals`]); the
/// morph moves vertices by up to ~17 mm, so keeping the base normals would
/// shade the base face's shape onto the character's.
fn apply_vertex_identity(mesh: &mut head_skin::HeadMesh, deltas: &[[f32; 3]]) {
    let base = mesh.positions.clone();
    for (p, d) in mesh.positions.iter_mut().zip(deltas) {
        p[0] += d[0];
        p[1] += d[1];
        p[2] += d[2];
    }
    if mesh.normals.len() == mesh.positions.len() {
        mesh.normals =
            normals::transfer_normals(&base, &mesh.positions, &mesh.indices, &mesh.normals);
    }
}

/// A `HeadAssets` cached for repeated per-character GLB builds.
pub struct HeadIdentityContext {
    assets: HeadAssets,
}

impl HeadIdentityContext {
    /// Load and cache assets for one gender (by `gender_hash`). `variant_hash`
    /// is not load-bearing; pass the rig's pinned value.
    pub fn load(p4k: &MappedP4k, gender_hash: NameHash) -> Result<Self, CharacterError> {
        let variant_hash = if gender_hash == identity::FEMALE_T1_PU_GENDER_HASH {
            identity::FEMALE_T1_PU_VARIANT_HASH
        } else {
            identity::MALE_T1_PU_VARIANT_HASH
        };
        Ok(Self {
            assets: HeadAssets::load(p4k, gender_hash, variant_hash)?,
        })
    }

    /// Build a rest-pose, per-character neutral-identity head GLB.
    ///
    /// - `full = true` includes eyes/teeth/eye-details sub-meshes; `false` is
    ///   head-only.
    /// - `morph_targets = true` bakes the ~400 expression blendshapes as glTF
    ///   morph targets (tens of MB — for export to Blender). `false` omits them
    ///   for a small, fast static head (the live GUI preview path).
    pub fn build_glb(
        &self,
        dna: &Dna,
        full: bool,
        morph_targets: bool,
    ) -> Result<Vec<u8>, CharacterError> {
        self.assets.rig_state(dna, full, morph_targets, &[])?.to_glb()
    }

    /// The whole character as a GLB — the rig the package writes
    /// ([`super::clips`]): CHF head `items`, dressed in `outfit` (body on
    /// its actor skeleton, garments), the head's blendshapes as morph
    /// targets, and `animation` (face and/or body clip, as one clip).
    pub fn build_character_glb(
        &self,
        dna: &Dna,
        items: &[HeadItemMesh],
        outfit: &Outfit,
        animation: &CharacterAnimation<'_>,
    ) -> Result<Vec<u8>, CharacterError> {
        self.assets.character_rig(dna, items, outfit, animation)?.to_glb()
    }

    /// Build a looping facial-expression animation GLB for `dna`, driven by the
    /// `.caf` idle in `caf_bytes` (the full IVO file), at `intensity` (clip
    /// weight). Extracts the `#caf` chunk and delegates to the loaded assets.
    pub fn build_expression_glb(
        &self,
        dna: &Dna,
        caf_bytes: &[u8],
        intensity: f32,
    ) -> Result<Vec<u8>, CharacterError> {
        let chunk = face_anim::load_caf_chunk_from_ivo(caf_bytes)
            .map_err(|e| CharacterError::FaceAnim(format!("{e}")))?;
        self.assets.expression_rig_state(dna, chunk, intensity, &[])?.to_glb()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An asymmetric DNA edit must stay one-sided: the slot-18 archetype codes
    /// are applied AS AUTHORED, without mirror-averaging (the engine does not
    /// symmetrize them — user-confirmed against in-game, 2026-06-12). Editing
    /// ONLY the left eyebrow must deform one half of the head far more than
    /// the other. Skipped unless Data.p4k is discoverable.
    #[test]
    fn asymmetric_dna_edit_stays_one_sided() {
        use starbreaker_chf::{DnaBlend, FacePart};
        let Some(p4k) = crate::test_p4k::p4k() else { return };
        let chf_bytes = p4k
            .read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf")
            .expect("masculinedefault.chf");
        let default_dna = starbreaker_chf::ChfFile::from_chf(&chf_bytes)
            .unwrap()
            .parse()
            .unwrap()
            .dna;
        let mut edited_dna = default_dna.clone();
        edited_dna.face_parts.insert(
            FacePart::EyebrowLeft,
            [
                DnaBlend { value: 65535, head_id: 18 },
                DnaBlend { value: 0, head_id: 0 },
                DnaBlend { value: 0, head_id: 0 },
                DnaBlend { value: 0, head_id: 0 },
            ],
        );
        edited_dna.max_head_id = edited_dna.max_head_id.max(18);

        let ctx = HeadIdentityContext::load(&p4k, default_dna.gender_hash).expect("ctx");
        let base = ctx.build_glb(&default_dna, false, false).expect("base glb");
        let edit = ctx.build_glb(&edited_dna, false, false).expect("edited glb");

        let positions = |glb: &[u8]| -> Vec<[f32; 3]> {
            let (doc, buffers, _) = gltf::import_slice(glb).expect("parse glb");
            let prim = doc.meshes().next().unwrap().primitives().next().unwrap();
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            reader.read_positions().unwrap().collect()
        };
        let (pb, pe) = (positions(&base), positions(&edit));
        assert_eq!(pb.len(), pe.len());
        // Sum of |delta| per lateral half (the head is modeled about x = 0).
        let (mut neg, mut pos) = (0f64, 0f64);
        for (a, b) in pb.iter().zip(&pe) {
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
                as f64;
            if a[0] < 0.0 {
                neg += d;
            } else {
                pos += d;
            }
        }
        let (lo, hi) = if neg < pos { (neg, pos) } else { (pos, neg) };
        assert!(
            hi > 0.0 && lo / hi < 0.5,
            "left-eyebrow-only edit must be one-sided, got half sums {neg:.4} / {pos:.4}"
        );
    }

    /// `build_expression_glb` must emit a parseable glTF carrying exactly one
    /// animation, for both the male and the female head paths (the female
    /// animated path is the spec's explicit risk). Skipped unless Data.p4k is
    /// discoverable. Variation `02` matches the happy idle used during the
    /// visual gate.
    #[test]
    fn expression_glb_has_one_animation_male_and_female() {
        let Some(p4k) = crate::test_p4k::p4k() else { return };
        let cases = [
            ("masculinedefault.chf", "male"),
            ("femininedefault.chf", "female"),
        ];
        for (chf_name, head_folder) in cases {
            let chf_bytes = p4k
                .read_file(&format!(r"Data\Libs\CharacterCustomizer\{chf_name}"))
                .unwrap_or_else(|e| panic!("read {chf_name}: {e}"));
            let dna = starbreaker_chf::ChfFile::from_chf(&chf_bytes)
                .unwrap()
                .parse()
                .unwrap()
                .dna;
            let caf_path = format!(
                r"Data\Animations\Characters\Human\heads\{head_folder}\ai\idlesets\happy\face\ai_civ_happy_idle_02_face.caf"
            );
            let caf_bytes = p4k
                .read_file(&caf_path)
                .unwrap_or_else(|e| panic!("read {caf_path}: {e}"));

            let ctx = HeadIdentityContext::load(&p4k, dna.gender_hash).expect("ctx");
            let glb = ctx
                .build_expression_glb(&dna, &caf_bytes, super::super::animation_export::EXPRESSION_DEFAULT_INTENSITY)
                .unwrap_or_else(|e| panic!("{head_folder} expression glb: {e}"));

            assert!(!glb.is_empty(), "{head_folder} GLB must be non-empty");
            let gltf = gltf::Gltf::from_slice(&glb)
                .unwrap_or_else(|e| panic!("{head_folder} GLB must parse: {e}"));
            assert_eq!(
                gltf.animations().count(),
                1,
                "{head_folder} GLB must carry exactly one animation"
            );
        }
    }
}
