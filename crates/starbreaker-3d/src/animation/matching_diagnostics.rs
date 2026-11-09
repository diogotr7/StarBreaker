//! Per-event CAF matching decisions for diagnostic tools.
use super::AnimationDatabase;
use super::matching_scores::{clip_motion_score_milli, clip_semantic_score, tokenize_for_match};
use std::collections::{HashMap, HashSet};

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

/// Run the per-event matching loop from [`super::matching::caf_anchored_remap`] and return
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
