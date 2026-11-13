//! Chrparams event → animation data pairing (`caf_anchored_remap`, `clips_for_caf_path`), plus
//! the small name / attribute helpers the Mannequin annotator shares.
//!
//! An event plays the CAF its resolved path names. That CAF is the tracks-database entry of the
//! same name (entry `i` owns key block `i`), or else the loose `.caf` file at that path. Nothing
//! else decides the pairing: not the bones a block animates, not the words in a name.

use std::collections::HashMap;

pub use super::extraction::extract_animations_for_skeleton_json;
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

/// Where a paired clip's keys came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClipOrigin {
    /// A chrparams event whose CAF is an entry of the tracks database.
    DatabaseEvent,
    /// A chrparams event whose CAF is a loose file.
    LooseCafEvent,
    /// A tracks-database entry no event of this rig names; keeps its database name.
    DatabaseOnly,
}

pub(super) struct PairedClip {
    pub clip: AnimationClip,
    pub origin: ClipOrigin,
}

/// Comparison key of an engine animation path: case-insensitive, either separator.
pub(crate) fn caf_path_key(path: &str) -> String {
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

/// The tracks-database entry a CAF path names.
pub(crate) fn clip_for_caf_path<'a>(
    db: &'a AnimationDatabase,
    caf_path: &str,
) -> Option<&'a AnimationClip> {
    let key = caf_path_key(caf_path);
    db.clips.iter().find(|clip| caf_path_key(&clip.name) == key)
}

/// The animation data a CAF path names: the tracks-database entry of that name, else whatever
/// `load_loose_caf` finds stored at that path. The loose file is not consulted for a path the
/// database holds.
pub(crate) fn clips_for_caf_path(
    db: &AnimationDatabase,
    caf_path: &str,
    load_loose_caf: impl FnOnce(&str) -> Vec<AnimationClip>,
) -> Vec<AnimationClip> {
    match clip_for_caf_path(db, caf_path) {
        Some(clip) => vec![clip.clone()],
        None => load_loose_caf(caf_path),
    }
}

/// Name the clips of one rig after its `.chrparams` events.
///
/// Each event takes the database entry its resolved CAF path names; events naming the same CAF
/// all get it. An event whose CAF is not in the database takes what `load_loose_caf` returns for
/// that path, and an event whose CAF exists in neither gets no clip: the game has no animation
/// to play for it either.
///
/// With `include_unmatched`, database entries no event names follow under their database names.
/// Pass `false` for child rigs that share the root's database, so its entries are emitted once.
///
/// Clip direction is not corrected here. The clip keeps its authored key order; the Mannequin
/// fragment that references it carries the meaning (`frag_tags`, forward or `speed = -1`).
pub(super) fn caf_anchored_remap(
    db: &AnimationDatabase,
    chrparams: &crate::chrparams::ChrParams,
    include_unmatched: bool,
    mut load_loose_caf: impl FnMut(&str) -> Vec<AnimationClip>,
) -> Vec<PairedClip> {
    let mut index_by_path: HashMap<String, usize> = HashMap::new();
    for (index, clip) in db.clips.iter().enumerate() {
        index_by_path.entry(caf_path_key(&clip.name)).or_insert(index);
    }

    let mut named_by_event = vec![false; db.clips.len()];
    let mut paired: Vec<PairedClip> = Vec::new();
    for (event_name, caf_path) in &chrparams.animations {
        if let Some(&index) = index_by_path.get(&caf_path_key(caf_path)) {
            named_by_event[index] = true;
            paired.push(PairedClip {
                clip: AnimationClip { name: event_name.clone(), ..db.clips[index].clone() },
                origin: ClipOrigin::DatabaseEvent,
            });
            continue;
        }
        let loose = load_loose_caf(caf_path);
        if loose.is_empty() {
            log::debug!("[anim] event '{event_name}' names no stored animation ({caf_path})");
        }
        paired.extend(loose.into_iter().map(|clip| PairedClip {
            clip: AnimationClip { name: event_name.clone(), ..clip },
            origin: ClipOrigin::LooseCafEvent,
        }));
    }

    if include_unmatched {
        for (index, clip) in db.clips.iter().enumerate() {
            if !named_by_event[index] {
                paired.push(PairedClip { clip: clip.clone(), origin: ClipOrigin::DatabaseOnly });
            }
        }
    }
    paired
}

#[cfg(test)]
#[path = "matching_tests.rs"]
mod tests;
