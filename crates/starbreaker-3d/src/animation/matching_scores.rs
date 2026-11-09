//! Semantic token and endpoint-motion scores used by CAF matching and diagnostics.
use super::AnimationClip;
use std::collections::HashMap;

pub(super) fn tokenize_for_match(input: &str) -> Vec<String> {
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

pub(super) fn clip_semantic_score(
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

pub(super) fn clip_motion_score_milli(clip: &AnimationClip) -> i64 {
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
