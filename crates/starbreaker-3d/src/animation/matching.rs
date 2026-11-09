//! Shared CAF matching and scoring for full animation extraction and sparse catalogue inspection.

use std::collections::{HashMap, HashSet};

use super::serialise::{
    annotate_animation_json_source, database_to_animations_json,
};
use super::{AnimationClip, AnimationDatabase};
use crate::error::Error;
use super::pose::bone_name_hash;

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

fn tokenize_for_match(input: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "animations",
        "animation",
        "spaceships",
        "ships",
        "objects",
        "object",
        "play",
        "ssmp",
        "component",
        "audio",
        "trigger",
        "event",
        "caf",
        // Directional tokens are too generic and cause false matches
        // (e.g. cooler_left_* selecting wing clips just because many wing
        // bones contain "left").
        "left",
        "right",
        "top",
        "bottom",
        "front",
        "rear",
        "main",
        // Common action verbs are non-discriminative across many clips.
        "open",
        "close",
        "deploy",
        "retract",
    ];

    input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.len() >= 3)
        .filter(|t| !STOPWORDS.iter().any(|w| w == t))
        .collect()
}

fn clip_semantic_score(
    clip: &AnimationClip,
    event_tokens: &[String],
    skeleton_bone_name_by_hash: &HashMap<u32, String>,
) -> i32 {
    if event_tokens.is_empty() {
        return 0;
    }

    let mut score = 0i32;

    // DBA metadata names can be misaligned with block contents, so semantic
    // scoring is intentionally based on resolved channel bone names only.
    for ch in &clip.channels {
        let Some(name) = skeleton_bone_name_by_hash.get(&ch.bone_hash) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let bone_tokens = tokenize_for_match(&lower);

        for token in event_tokens {
            if bone_tokens.iter().any(|bt| bt == token) {
                score += 4;
            } else if lower.contains(token) {
                score += 2;
            }
        }
    }
    score
}

fn clip_motion_score_milli(clip: &AnimationClip) -> i64 {
    let mut score = 0.0f64;

    for ch in &clip.channels {
        if ch.rotations.len() >= 2 {
            let q0 = ch.rotations.first().map(|k| k.value).unwrap_or([0.0; 4]);
            let q1 = ch.rotations.last().map(|k| k.value).unwrap_or([0.0; 4]);
            let dot = (q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3])
                .abs()
                .clamp(0.0, 1.0) as f64;
            // Quaternion angular distance in radians.
            let angle = 2.0f64 * dot.acos();
            score += angle;
        }

        if ch.positions.len() >= 2 {
            let p0 = ch.positions.first().map(|k| k.value).unwrap_or([0.0; 3]);
            let p1 = ch.positions.last().map(|k| k.value).unwrap_or([0.0; 3]);
            let dx = (p1[0] - p0[0]) as f64;
            let dy = (p1[1] - p0[1]) as f64;
            let dz = (p1[2] - p0[2]) as f64;
            score += (dx * dx + dy * dy + dz * dz).sqrt();
        }
    }

    (score * 1000.0).round() as i64
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
                    let block_valid = !can_validate || db.clips[path_idx]
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
                    let composite =
                        clip_semantic_score(&db.clips[i], &event_tokens, skeleton_bone_name_by_hash);
                    let motion = clip_motion_score_milli(&db.clips[i]);
                    (
                        i,
                        composite,
                        motion,
                    )
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
                let no_hint_with_overlap =
                    path_idx_hint.is_none() && semantic_score > 0;
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
            log::debug!(
                "[anim] no DBA block found for event '{event_name}' ({resolved_caf})"
            );
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

/// Diagnostic record for a single chrparams event, describing how the
/// matching pipeline (path → semantic → bone-subset) selected a DBA block.
///
/// Returned by [`caf_anchored_remap_decisions`]. Used by the `dba match`
/// CLI subcommand to debug clip-mismatches such as the wings_deploy
/// X-shape issue.
#[derive(Debug, Clone)]
pub struct ClipMatchDecision {
    pub event_name: String,
    pub caf_path: String,
    /// Final block index chosen, or `None` if no block matched.
    pub chosen_block: Option<usize>,
    /// Which step picked the block: "path", "semantic-override",
    /// "semantic-no-hint", "bone-subset", or "unmatched".
    pub method: &'static str,
    /// The block matched purely by path lookup (Step 1), if any.
    pub path_block: Option<usize>,
    /// The block winning the semantic-overlap scoring (Step 1.5), if any.
    pub semantic_block: Option<usize>,
    /// Semantic score of the path-matched block (or i32::MIN if none).
    pub path_score: i32,
    /// Semantic score of the semantic-best block (or 0 if none).
    pub semantic_score: i32,
}

/// Run the per-event matching loop from [`caf_anchored_remap`] and return
/// per-event decision details without building the named clips. This is a
/// diagnostic helper used by the CLI `dba match` subcommand.
pub fn caf_anchored_remap_decisions(
    db: &AnimationDatabase,
    chrparams: &crate::chrparams::ChrParams,
    skeleton_bone_hashes: &HashSet<u32>,
    skeleton_bone_name_by_hash: &HashMap<u32, String>,
    animevents_targets_by_caf: &HashMap<String, Vec<String>>,
    allow_bone_subset_fallback: bool,
) -> Vec<ClipMatchDecision> {
    let mut name_map: HashMap<String, usize> = HashMap::new();
    for (i, clip) in db.clips.iter().enumerate() {
        name_map.entry(clip.name.to_ascii_lowercase()).or_insert(i);
    }

    let can_validate = !skeleton_bone_hashes.is_empty();
    let mut matched = vec![false; db.clips.len()];
    let mut decisions: Vec<ClipMatchDecision> = Vec::new();

    for (event_name, caf_path) in &chrparams.animations {
        let resolved_caf = chrparams.resolved_caf_path(caf_path);
        let resolved_lower = resolved_caf.to_ascii_lowercase();
        let caf_file = resolved_caf
            .rsplit_once('/')
            .map(|(_, tail)| tail)
            .unwrap_or(resolved_caf.as_str())
            .trim_end_matches(".caf");

        let mut event_tokens = tokenize_for_match(event_name);
        event_tokens.extend(tokenize_for_match(caf_file));
        if let Some(targets) = animevents_targets_by_caf.get(&resolved_lower) {
            for target in targets {
                event_tokens.extend(tokenize_for_match(target));
            }
        }

        let mut chosen_idx: Option<usize> = None;
        let mut method: &'static str = "unmatched";
        let mut path_block: Option<usize> = None;
        let mut semantic_block: Option<usize> = None;
        let mut path_score: i32 = i32::MIN;
        let mut semantic_score_val: i32 = 0;

        if let Some(&path_idx) = name_map.get(&resolved_lower) {
            if !matched[path_idx] {
                let block_valid = !can_validate
                    || db.clips[path_idx]
                        .channels
                        .iter()
                        .all(|ch| skeleton_bone_hashes.contains(&ch.bone_hash));
                if block_valid {
                    path_block = Some(path_idx);
                    chosen_idx = Some(path_idx);
                    method = "path";
                    path_score = clip_semantic_score(
                        &db.clips[path_idx],
                        &event_tokens,
                        skeleton_bone_name_by_hash,
                    );
                }
            }
        }

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

            if let Some((semantic_idx, semantic_score, _)) = best_semantic {
                semantic_block = Some(semantic_idx);
                semantic_score_val = semantic_score;
                let strictly_better = semantic_score > path_score;
                let no_hint_with_overlap = path_block.is_none() && semantic_score > 0;
                if strictly_better {
                    chosen_idx = Some(semantic_idx);
                    method = "semantic-override";
                } else if no_hint_with_overlap {
                    chosen_idx = Some(semantic_idx);
                    method = "semantic-no-hint";
                }
            }
        }

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
                method = "bone-subset";
            }
        }

        if let Some(idx) = chosen_idx {
            matched[idx] = true;
        }

        decisions.push(ClipMatchDecision {
            event_name: event_name.clone(),
            caf_path: resolved_caf,
            chosen_block: chosen_idx,
            method,
            path_block,
            semantic_block,
            path_score,
            semantic_score: semantic_score_val,
        });
    }

    decisions
}

pub fn extract_animations_for_skeleton_json(
    p4k: &starbreaker_p4k::MappedP4k,
    skeleton_path: &str,
    include_unmatched_dba_blocks: bool,
    allow_bone_subset_fallback: bool,
) -> Result<Option<serde_json::Value>, Error> {
    extract_skeleton_json(
        p4k, skeleton_path, include_unmatched_dba_blocks, allow_bone_subset_fallback, false,
    )
}

pub(crate) fn extract_skeleton_json(
    p4k: &starbreaker_p4k::MappedP4k,
    skeleton_path: &str,
    include_unmatched_dba_blocks: bool,
    allow_bone_subset_fallback: bool,
    endpoints: bool,
) -> Result<Option<serde_json::Value>, Error> {
    let mut candidate_paths = Vec::new();
    if let Some(path) = swap_extension(skeleton_path, ".chrparams") {
        candidate_paths.push(path);
    }
    // SC assets often ship `*_SKIN.skin` + `*_CHR.chr/.chrparams` pairs.
    let skin_to_chr = skeleton_path
        .replace("_SKIN.skin", "_CHR.chrparams")
        .replace("_skin.skin", "_chr.chrparams")
        .replace("_skin.SKIN", "_chr.chrparams");
    if !candidate_paths.iter().any(|path| path.eq_ignore_ascii_case(&skin_to_chr)) {
        candidate_paths.push(skin_to_chr);
    }

    // Try candidate chrparams paths; skip if none found.
    let mut chrparams_data = None;
    let mut chrparams_path = String::new();
    for candidate in &candidate_paths {
        let candidate_p4k = crate::pipeline::datacore_path_to_p4k(candidate);
        if let Some(data) = p4k
            .entry_case_insensitive(&candidate_p4k)
            .and_then(|e| p4k.read(e).ok())
        {
            chrparams_path = candidate.clone();
            chrparams_data = Some(data.to_vec());
            break;
        }
    }
    let Some(chrparams_data) = chrparams_data else {
        return Ok(None); // Skeleton has no discoverable chrparams
    };

    // Parse chrparams to get tracks database path
    let chrparams = match crate::chrparams::ChrParams::from_bytes(&chrparams_data) {
        Ok(value) => value,
        Err(error) => {
            let text = error.to_string();
            // Some non-skeleton assets are probed via heuristic path swaps and
            // resolve to non-CryXml payloads. Treat those as "no animations" to
            // avoid noisy warnings during normal export.
            if text.contains("InvalidMagic") {
                return Ok(None);
            }
            return Err(Error::Other(format!("Failed to parse chrparams: {error}")));
        }
    };

    let animevents_targets_by_caf: HashMap<String, Vec<String>> = chrparams
        .anim_event_database
        .as_deref()
        .and_then(|path| {
            let resolved = chrparams.resolved_caf_path(path);
            let resolved_p4k = crate::pipeline::datacore_path_to_p4k(&resolved);
            p4k.entry_case_insensitive(&resolved_p4k)
                .and_then(|e| p4k.read(e).ok())
                .and_then(|bytes| crate::chrparams::parse_animevents_targets(&bytes).ok())
        })
        .unwrap_or_default();

    // Build the hash→node-name map once and use it for both DBA-backed clips
    // and direct per-CAF chrparams clips.
    //
    // Important: Star Citizen uses the same 32-bit CRC32 name-hash space for
    // CHR bones and rigid CGA/CGAM NMC nodes.  The older code only called
    // parse_skeleton(), which works for CHR CompiledBones but returns None for
    // many CGA object-animation rigs (doors, panels, ladders, etc.).  That made
    // their exported sidecar JSON hash-only, so the Blender add-on had real
    // key data but no source_node_name/binding hints.  parse_rig_node_names()
    // intentionally handles both cases: CHR CompiledBones first, then CGA/CGAM
    // NMC node names.
    let skeleton_p4k_path = crate::pipeline::datacore_path_to_p4k(skeleton_path);
    let (skeleton_bone_hashes, skeleton_bone_name_by_hash): (
        HashSet<u32>,
        HashMap<u32, String>,
    ) = p4k
        .entry_case_insensitive(&skeleton_p4k_path)
        .and_then(|e| p4k.read(e).ok())
        .and_then(|data| crate::skeleton::parse_rig_node_names(&data))
        .map(|names| {
            let hashes = names
                .iter()
                .map(|name| bone_name_hash(name))
                .collect::<HashSet<_>>();
            let name_map = names
                .iter()
                .map(|name| (bone_name_hash(name), name.to_string()))
                .collect::<HashMap<_, _>>();
            (hashes, name_map)
        })
        .unwrap_or_default();
    log::debug!(
        "[anim] rig '{}' has {} resolved CHR/NMC node hashes",
        skeleton_path,
        skeleton_bone_hashes.len()
    );

    // Prefer tracks database (.dba) when present.
    if let Some(tracks_db_path) = chrparams.tracks_database.clone() {
        let resolved_path = chrparams.resolved_caf_path(&tracks_db_path);
        let resolved_p4k = crate::pipeline::datacore_path_to_p4k(&resolved_path);
        let dba_data = p4k
            .entry_case_insensitive(&resolved_p4k)
            .and_then(|e| p4k.read(e).ok())
            .ok_or_else(|| Error::Other(format!("Cannot load tracks database: {resolved_path}")))?
            .to_vec();
        let db = super::dba::parse_dba_mode(&dba_data, endpoints)?;
        let clips = caf_anchored_remap(
            &db,
            &chrparams,
            &skeleton_bone_hashes,
            &skeleton_bone_name_by_hash,
            &animevents_targets_by_caf,
            include_unmatched_dba_blocks,
            allow_bone_subset_fallback,
        );
        // Phase 38 (deferred): a per-bone CAF blend-mode classifier was
        // attempted here using AABB-of-CAF-samples vs CHR-bind containment.
        // Empirically the test inverts the additive/override split (over-
        // marks stationary tracks as override). Phase 37 confirmed neither
        // CAF Controller flags nor Mannequin ADB carry the bit. The
        // `BoneBlendMode` enum, `classify_bone_blend_modes` helper, and
        // `annotate_animations_json_with_blend_modes` helper remain as
        // latent infrastructure for a future data-grounded discriminator;
        // the addon's runtime override path consumes the field when set.
        let mut value = database_to_animations_json(&AnimationDatabase { clips });
        annotate_animation_json_source(&mut value, skeleton_path, &skeleton_bone_name_by_hash);
        if endpoints {
            for clip in value.as_array_mut().into_iter().flatten() {
                clip["catalogue_source"] = serde_json::json!(
                    if chrparams.animations.contains_key(clip["name"].as_str().unwrap_or("")) {
                        "Chrparams"
                    } else {
                        "DbaOnly"
                    }
                );
                clip["chrparams_path"] = serde_json::json!(chrparams_path);
                clip["dba_path"] = serde_json::json!(resolved_path);
            }
        }
        return Ok(Some(value));
    }

    // Fallback for chrparams that reference per-clip CAF files directly.
    if chrparams.animations.is_empty() {
        return Ok(None);
    }
    let mut clips = Vec::new();
    for (event_name, caf_path) in &chrparams.animations {
        let resolved_path = chrparams.resolved_caf_path(caf_path);
        let resolved_p4k = crate::pipeline::datacore_path_to_p4k(&resolved_path);
        let Some(caf_data) = p4k
            .entry_case_insensitive(&resolved_p4k)
            .and_then(|e| p4k.read(e).ok())
        else {
            continue;
        };
        if let Ok(mut db) = super::caf::parse_caf_mode(&caf_data, endpoints) {
            for mut clip in db.clips.drain(..) {
                clip.name = event_name.clone();
                clips.push(clip);
            }
        }
    }
    if clips.is_empty() {
        return Ok(None);
    }
    let mut value = database_to_animations_json(&AnimationDatabase { clips });
    annotate_animation_json_source(&mut value, skeleton_path, &skeleton_bone_name_by_hash);
    if endpoints {
        for clip in value.as_array_mut().into_iter().flatten() {
            clip["catalogue_source"] = serde_json::json!("Caf");
            clip["chrparams_path"] = serde_json::json!(chrparams_path);
        }
    }
    Ok(Some(value))
}

/// Helper: swap file extension. E.g., "file.chr" → "file.chrparams"
fn swap_extension(path: &str, new_ext: &str) -> Option<String> {
    if let Some(dot_pos) = path.rfind('.') {
        let base = &path[..dot_pos];
        Some(format!("{}{}", base, new_ext))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use super::tokenize_for_match;
    use super::caf_anchored_remap;
    use crate::animation::{AnimationClip, AnimationDatabase};
    use crate::chrparams::ChrParams;

    #[test]
    fn tokenize_for_match_keeps_ship_specific_tokens() {
        let tokens = tokenize_for_match("RSI Scorpius wings deploy");
        assert!(tokens.iter().any(|token| token == "rsi"));
        assert!(tokens.iter().any(|token| token == "scorpius"));
    }

    #[test]
    fn tokenize_for_match_still_removes_generic_stopwords() {
        let tokens = tokenize_for_match("Animations ships component wings");
        assert!(!tokens.iter().any(|token| token == "animations"));
        assert!(!tokens.iter().any(|token| token == "ships"));
        assert!(!tokens.iter().any(|token| token == "component"));
        assert!(tokens.iter().any(|token| token == "wings"));
    }

    #[test]
    fn caf_anchored_remap_uses_unmatched_duplicate_name_slots() {
        let db = AnimationDatabase {
            clips: vec![
                AnimationClip {
                    name: "animations/shared_clip.caf".into(),
                    fps: 30.0,
                    channels: vec![],
                    start_rotation: None,
                    start_position: None,
                },
                AnimationClip {
                    name: "animations/shared_clip.caf".into(),
                    fps: 30.0,
                    channels: vec![],
                    start_rotation: None,
                    start_position: None,
                },
            ],
        };

        let chrparams = ChrParams {
            animations: BTreeMap::from([
                ("event_a".to_string(), "animations/shared_clip.caf".to_string()),
                ("event_b".to_string(), "animations/shared_clip.caf".to_string()),
            ]),
            animation_paths: BTreeMap::new(),
            ..Default::default()
        };

        let remapped = caf_anchored_remap(
            &db,
            &chrparams,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            false,
            false,
        );

        assert_eq!(remapped.len(), 2);
        assert!(remapped.iter().any(|clip| clip.name == "event_a"));
        assert!(remapped.iter().any(|clip| clip.name == "event_b"));
    }

    #[test]
    fn caf_anchored_remap_matches_filename_when_metadata_omits_prefix() {
        let db = AnimationDatabase {
            clips: vec![AnimationClip {
                name: "shared_clip.caf".into(),
                fps: 30.0,
                channels: vec![],
                start_rotation: None,
                start_position: None,
            }],
        };

        let chrparams = ChrParams {
            animations: BTreeMap::from([(
                "event_prefixed".to_string(),
                "Animations/shared_clip.caf".to_string(),
            )]),
            animation_paths: BTreeMap::new(),
            ..Default::default()
        };

        let remapped = caf_anchored_remap(
            &db,
            &chrparams,
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            false,
            false,
        );

        assert_eq!(remapped.len(), 1);
        assert_eq!(remapped[0].name, "event_prefixed");
    }

    /// When two DBA blocks have identical semantic scores (no path hint resolves
    /// either), the tiebreaker picks the block with the *higher* motion score
    /// (more actual joint movement).  This test pins that behaviour so a
    /// future change to the tiebreak direction is caught immediately.
    #[test]
    fn caf_anchored_remap_equal_semantic_score_tiebreaks_on_higher_motion() {
        use crate::animation::{BoneChannel, Keyframe};

        let bone_hash = 0xC1571A1A_u32; // BONE_Back_Right_Foot_Main
        let skeleton_hashes: HashSet<u32> = [bone_hash].into_iter().collect();
        let mut skeleton_names: HashMap<u32, String> = HashMap::new();
        skeleton_names.insert(bone_hash, "bone_back_right_foot_main".to_string());

        // Block 0: minimal rotation delta (near-zero motion score).
        let still_clip = AnimationClip {
            name: "animations/gear.caf".into(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash,
                rotations: vec![
                    Keyframe { time: 0.0, value: [1.0, 0.0, 0.0, 0.0] },
                    Keyframe { time: 1.0, value: [1.0, 0.0, 0.0, 0.0] },
                ],
                positions: vec![],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
            start_rotation: None,
            start_position: None,
        };

        // Block 1: 180° rotation delta (high motion score).
        let moving_clip = AnimationClip {
            name: "animations/gear.caf".into(),
            fps: 30.0,
            channels: vec![BoneChannel {
                bone_hash,
                rotations: vec![
                    Keyframe { time: 0.0, value: [1.0, 0.0, 0.0, 0.0] },
                    Keyframe { time: 1.0, value: [0.0, 1.0, 0.0, 0.0] },
                ],
                positions: vec![],
                rot_format_flags: 0,
                pos_format_flags: 0,
            }],
            start_rotation: None,
            start_position: None,
        };

        let db = AnimationDatabase {
            clips: vec![still_clip, moving_clip.clone()],
        };

        // CAF path does NOT match either block by name — path lookup fails,
        // leaving only semantic scoring.  Both blocks have equal token overlap
        // (same bone, caf stem "unknown_anim" has no name tokens in common),
        // so motion score is the sole tiebreaker.
        let chrparams = ChrParams {
            animations: BTreeMap::from([(
                "gear_retract".to_string(),
                "Animations/unknown_anim.caf".to_string(),
            )]),
            animation_paths: BTreeMap::new(),
            ..Default::default()
        };

        let remapped = caf_anchored_remap(
            &db,
            &chrparams,
            &skeleton_hashes,
            &skeleton_names,
            &HashMap::new(),
            false,
            false,
        );

        assert_eq!(remapped.len(), 1, "expected exactly one clip matched");
        // Block 1 (moving_clip) has higher motion — must be chosen.
        assert_eq!(
            remapped[0].channels[0].rotations[1].value,
            [0.0_f32, 1.0, 0.0, 0.0],
            "tiebreaker should select the block with higher motion score"
        );
    }
}



