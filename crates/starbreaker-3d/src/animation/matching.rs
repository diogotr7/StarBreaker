//! Shared CAF matching and scoring for full animation extraction and sparse catalogue inspection.

use std::collections::{HashMap, HashSet};

pub use super::extraction::extract_animations_for_skeleton_json;
pub use super::matching_diagnostics::{ClipMatchDecision, caf_anchored_remap_decisions};
use super::matching_scores::{clip_motion_score_milli, clip_semantic_score, tokenize_for_match};
use super::{AnimationClip, AnimationDatabase};

pub(super) fn clip_name_lookup_keys(name: &str) -> Vec<String> {
    let lower = name.trim().replace('\\', "/").to_ascii_lowercase();
    let stem = lower
        .rsplit_once('/')
        .map(|(_, tail)| tail)
        .unwrap_or(lower.as_str())
        .trim_end_matches(".caf")
        .to_string();
    if stem == lower {
        vec![lower]
    } else {
        vec![lower, stem]
    }
}

pub(super) fn split_tag_list(raw: &str) -> Vec<String> {
    raw.split(|ch: char| ch == '+' || ch == '|' || ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn parse_f32_attr(raw: Option<&str>) -> Option<f32> {
    raw.and_then(|value| value.parse::<f32>().ok())
}

/// Match DBA blocks to `.chrparams` event names using a hybrid approach:
///
/// 1. **Path-based** (primary): resolve the chrparams CAF path to its full
///    engine path and match it case-insensitively against DBA metadata names.
///    This works for any DBA where metadata is correctly ordered.
///
/// 2. **Bone-subset fallback** (secondary): if the path-matched block contains
///    bones that are NOT in this skeleton, the DBA metadata for this section is
///    scrambled (as seen in `Scorpius.dba` for landing gear clips).  In that
///    case, fall back to finding the first unmatched DBA block whose entire bone
///    set is a subset of this skeleton's bones.
///
/// Unmatched DBA blocks retain their original DBA metadata names only when
/// `include_unmatched` is `true`.  Pass `false` for child skeleton sources that
/// share the root's DBA so that already-covered blocks are not duplicated.
pub(super) fn caf_anchored_remap(
    db: &AnimationDatabase,
    chrparams: &crate::chrparams::ChrParams,
    skeleton_bone_hashes: &HashSet<u32>,
    skeleton_bone_name_by_hash: &HashMap<u32, String>,
    animevents_targets_by_caf: &HashMap<String, Vec<String>>,
    include_unmatched: bool,
    allow_bone_subset_fallback: bool,
) -> Vec<AnimationClip> {
    // Build a name→indices map from the DBA metadata names (case-insensitive).
    // Some DBAs can repeat clip names; keep all candidate indices so path-based
    // lookup can choose an unmatched slot instead of hard-wiring to the first.
    let mut name_map: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, clip) in db.clips.iter().enumerate() {
        name_map
            .entry(clip.name.to_ascii_lowercase())
            .or_default()
            .push(i);
    }

    // When skeleton_bone_hashes is empty we skip validation (skeleton not found).
    let can_validate = !skeleton_bone_hashes.is_empty();

    let mut matched = vec![false; db.clips.len()];
    let mut named_clips: Vec<AnimationClip> = Vec::new();

    for (event_name, caf_path) in &chrparams.animations {
        let resolved_caf = chrparams.resolved_caf_path(caf_path);
        let resolved_lower = resolved_caf.to_ascii_lowercase();
        let caf_file = resolved_caf
            .rsplit_once('/')
            .map(|(_, tail)| tail)
            .unwrap_or(resolved_caf.as_str());
        let caf_stem = caf_file.trim_end_matches(".caf");

        let mut event_tokens = tokenize_for_match(event_name);
        event_tokens.extend(tokenize_for_match(caf_stem));
        if let Some(targets) = animevents_targets_by_caf.get(&resolved_lower) {
            for target in targets {
                event_tokens.extend(tokenize_for_match(target));
            }
        }

        let mut chosen_idx: Option<usize> = None;
        let mut path_idx_hint: Option<usize> = None;

        // Step 1: path-based lookup.
        let mut lookup_keys = vec![resolved_lower.clone()];
        let caf_file_lower = caf_file.to_ascii_lowercase();
        if caf_file_lower != resolved_lower {
            lookup_keys.push(caf_file_lower);
        }

        for key in lookup_keys {
            if let Some(path_indices) = name_map.get(&key) {
                if let Some(path_idx) = path_indices.iter().copied().find(|idx| !matched[*idx]) {
                    let block_valid = !can_validate
                        || db.clips[path_idx]
                            .channels
                            .iter()
                            .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash));
                    if block_valid {
                        // Keep as a hint; we may override it if semantic scoring finds
                        // a better candidate among similarly-valid blocks.
                        path_idx_hint = Some(path_idx);
                        chosen_idx = Some(path_idx);
                        break;
                    } else {
                        log::debug!(
                            "[anim] path-matched block {path_idx} for '{event_name}' has bones outside skeleton — using semantic/bone-subset fallback"
                        );
                    }
                }
            }
        }

        // Step 1.5: semantic disambiguation. This is especially important for
        // root body CHRs where many clips share controller counts/start quats and
        // path-index alignment can be wrong when DBA metadata order differs from
        // block order.
        if can_validate && !skeleton_bone_name_by_hash.is_empty() {
            let best_semantic = (0..db.clips.len())
                .filter(|&i| !matched[i])
                .filter(|&i| {
                    !db.clips[i].channels.is_empty()
                        && db.clips[i]
                            .channels
                            .iter()
                            .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
                })
                .map(|i| {
                    let composite = clip_semantic_score(
                        &db.clips[i],
                        &event_tokens,
                        skeleton_bone_name_by_hash,
                    );
                    let motion = clip_motion_score_milli(&db.clips[i]);
                    (i, composite, motion)
                })
                .max_by_key(|(_, score, motion)| (*score, *motion));

            if let Some((semantic_idx, semantic_score, _semantic_motion)) = best_semantic {
                let hinted_score = path_idx_hint
                    .map(|idx| {
                        clip_semantic_score(
                            &db.clips[idx],
                            &event_tokens,
                            skeleton_bone_name_by_hash,
                        )
                    })
                    .unwrap_or(i32::MIN);

                // Prefer semantic winner only when it has a strictly stronger
                // bone-name token match than the path hint, OR when no path
                // hint exists AND the semantic match has positive overlap.
                // Equal-score motion-tiebreak previously caused systematic
                // reassignment of correct path matches to nearby high-motion
                // blocks; require a strict lexical advantage instead.
                let strictly_better = semantic_score > hinted_score;
                let no_hint_with_overlap = path_idx_hint.is_none() && semantic_score > 0;
                if strictly_better || no_hint_with_overlap {
                    chosen_idx = Some(semantic_idx);
                }
            }
        }

        // Step 2: bone-subset fallback if path lookup failed or was invalid.
        // Only used for child CHRs (small bone sets); the root body CHR
        // has a large superset of bones, so this fallback would misfire.
        if chosen_idx.is_none() && can_validate && allow_bone_subset_fallback {
            chosen_idx = (0..db.clips.len()).find(|&i| {
                !matched[i]
                    && !db.clips[i].channels.is_empty()
                    && db.clips[i]
                        .channels
                        .iter()
                        .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash))
            });
            if chosen_idx.is_some() {
                log::debug!(
                    "[anim] bone-subset fallback: assigned block {:?} to '{event_name}'",
                    chosen_idx
                );
            }
        }

        if let Some(idx) = chosen_idx {
            matched[idx] = true;
            named_clips.push(AnimationClip {
                name: event_name.clone(),
                fps: db.clips[idx].fps,
                channels: db.clips[idx].channels.clone(),
                start_rotation: db.clips[idx].start_rotation,
                start_position: db.clips[idx].start_position,
            });
        } else {
            log::debug!("[anim] no DBA block found for event '{event_name}' ({resolved_caf})");
        }
    }

    // Append unmatched DBA blocks with their original metadata names, but only
    // when the caller wants them (root skeleton context).
    if include_unmatched {
        for (i, clip) in db.clips.iter().enumerate() {
            if !matched[i] {
                named_clips.push(clip.clone());
            }
        }
    }

    // NOTE: clip direction is *not* corrected here.
    //
    // The previous Phase 24B implementation called `correct_clip_direction`,
    // which inferred "expected" temporal direction from substrings of the
    // clip name (`deploy`/`open`/`extend` vs. `retract`/`close`/`compress`)
    // and reversed keyframe time when the bind-distance heuristic disagreed.
    // That logic was both name-based (a forbidden hard-coding pattern in
    // this codebase) and based on a wrong assumption — that the bind pose
    // is always the "closed/retracted" state. For Scorpius wings the bind
    // pose is the *deployed* state, so the heuristic reversed `wings_deploy`
    // into a clip that ends in the retracted state, breaking snap-to-state.
    //
    // Direction is now resolved on the addon side using authoritative
    // Mannequin fragment metadata (`speed`, `frag_tags`) and per-channel
    // cyclic detection. The exporter emits the clip with its authored
    // keyframe order; whichever fragment references it provides the
    // semantic mapping (Deploy/Retract/Open/Close, forward or `speed=-1`).
    //
    // See `package_ops._fragment_endpoint_policy` and
    // `_apply_best_channel_transform` for the consuming logic.

    named_clips
}

#[cfg(test)]
#[path = "matching_tests.rs"]
mod tests;
