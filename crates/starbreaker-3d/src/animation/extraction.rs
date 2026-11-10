//! Shared full/sparse extraction of one rig's clips: its chrparams events paired with the
//! tracks database (.dba) entries or loose CAF files their paths name.
use super::matching::{ClipOrigin, caf_anchored_remap};
use super::serialise::{annotate_animation_json_source, database_to_animations_json};
use super::{AnimationDatabase, pose::bone_name_hash};
use crate::error::Error;
use std::collections::HashMap;

pub fn extract_animations_for_skeleton_json(
    p4k: &starbreaker_p4k::MappedP4k,
    skeleton_path: &str,
    include_unmatched_dba_blocks: bool,
) -> Result<Option<serde_json::Value>, Error> {
    extract_skeleton_json(
        p4k,
        skeleton_path,
        include_unmatched_dba_blocks,
        false,
    )
}

pub(crate) fn extract_skeleton_json(
    p4k: &starbreaker_p4k::MappedP4k,
    skeleton_path: &str,
    include_unmatched_dba_blocks: bool,
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
    if !candidate_paths
        .iter()
        .any(|path| path.eq_ignore_ascii_case(&skin_to_chr))
    {
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
    let skeleton_bone_name_by_hash: HashMap<u32, String> = p4k
        .entry_case_insensitive(&skeleton_p4k_path)
        .and_then(|e| p4k.read(e).ok())
        .and_then(|data| crate::skeleton::parse_rig_node_names(&data))
        .map(|names| {
            names
                .iter()
                .map(|name| (bone_name_hash(name), name.to_string()))
                .collect()
        })
        .unwrap_or_default();
    log::debug!(
        "[anim] rig '{}' has {} resolved CHR/NMC node hashes",
        skeleton_path,
        skeleton_bone_name_by_hash.len()
    );

    // The tracks database (.dba) holds the rig's CAFs; a rig without one stores each loose.
    let dba_path = chrparams.tracks_database.clone();
    let db = match &dba_path {
        Some(resolved_path) => {
            let resolved_p4k = crate::pipeline::datacore_path_to_p4k(resolved_path);
            let dba_data = p4k
                .entry_case_insensitive(&resolved_p4k)
                .and_then(|e| p4k.read(e).ok())
                .ok_or_else(|| {
                    Error::Other(format!("Cannot load tracks database: {resolved_path}"))
                })?
                .to_vec();
            super::dba::parse_dba_mode(&dba_data, endpoints)?
        }
        None => AnimationDatabase { clips: Vec::new() },
    };
    let paired = caf_anchored_remap(&db, &chrparams, include_unmatched_dba_blocks, |caf_path| {
        let caf_p4k = crate::pipeline::datacore_path_to_p4k(caf_path);
        p4k.entry_case_insensitive(&caf_p4k)
            .and_then(|e| p4k.read(e).ok())
            .and_then(|caf_data| super::caf::parse_caf_mode(&caf_data, endpoints).ok())
            .map(|caf| caf.clips)
            .unwrap_or_default()
    });
    if paired.is_empty() && dba_path.is_none() {
        return Ok(None);
    }
    // A per-bone blend-mode classifier (additive vs override) was tried here and deferred: no
    // data-grounded discriminator is known. `BoneBlendMode`, `classify_bone_blend_modes` and
    // `annotate_animations_json_with_blend_modes` remain for a future one.
    let (clips, origins): (Vec<_>, Vec<_>) = paired
        .into_iter()
        .map(|paired| (paired.clip, paired.origin))
        .unzip();
    let mut value = database_to_animations_json(&AnimationDatabase { clips });
    annotate_animation_json_source(&mut value, skeleton_path, &skeleton_bone_name_by_hash);
    if endpoints {
        for (clip, origin) in value.as_array_mut().into_iter().flatten().zip(origins) {
            clip["catalogue_source"] = serde_json::json!(match origin {
                ClipOrigin::DatabaseEvent => "Chrparams",
                ClipOrigin::LooseCafEvent => "Caf",
                ClipOrigin::DatabaseOnly => "DbaOnly",
            });
            clip["chrparams_path"] = serde_json::json!(chrparams_path);
            if let (Some(dba_path), false) = (&dba_path, origin == ClipOrigin::LooseCafEvent) {
                clip["dba_path"] = serde_json::json!(dba_path);
            }
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
