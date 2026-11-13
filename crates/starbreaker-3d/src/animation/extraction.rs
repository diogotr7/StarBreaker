//! Shared full/sparse extraction of one rig's clips: its chrparams events paired with the
//! tracks database (.dba) entries or loose CAF files their paths name. `load_tracks_database`
//! and `loose_caf_clips` are the two sources every consumer of that pairing reads from.
use super::matching::{ClipOrigin, caf_anchored_remap};
use super::serialise::{annotate_animation_json_source, database_to_animations_json};
use super::{AnimationClip, AnimationDatabase, pose::bone_name_hash};
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

    let dba_path = chrparams.tracks_database.clone();
    let db = load_tracks_database(p4k, &chrparams, endpoints)?;
    let paired = caf_anchored_remap(&db, &chrparams, include_unmatched_dba_blocks, |caf_path| {
        loose_caf_clips(p4k, caf_path, endpoints)
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

/// The tracks database (.dba) a rig's chrparams declares. It holds the rig's CAFs; a rig that
/// declares none stores each CAF loose and gets an empty database. A declared database that
/// cannot be read or decoded is an error.
pub(crate) fn load_tracks_database(
    p4k: &starbreaker_p4k::MappedP4k,
    chrparams: &crate::chrparams::ChrParams,
    endpoints: bool,
) -> Result<AnimationDatabase, Error> {
    let Some(dba_path) = chrparams.tracks_database.as_deref() else {
        return Ok(AnimationDatabase { clips: Vec::new() });
    };
    let dba_data = p4k
        .entry_case_insensitive(&crate::pipeline::datacore_path_to_p4k(dba_path))
        .and_then(|e| p4k.read(e).ok())
        .ok_or_else(|| Error::Other(format!("Cannot load tracks database: {dba_path}")))?;
    super::dba::parse_dba_mode(&dba_data, endpoints)
}

/// The clips of the loose `.caf` stored at `caf_path`: none when no file is stored there (the
/// game has nothing to play either), an error when a stored file cannot be read or decoded.
pub(crate) fn read_loose_caf(
    p4k: &starbreaker_p4k::MappedP4k,
    caf_path: &str,
    endpoints: bool,
) -> Result<Vec<AnimationClip>, Error> {
    let Some(entry) = p4k.entry_case_insensitive(&crate::pipeline::datacore_path_to_p4k(caf_path))
    else {
        return Ok(Vec::new());
    };
    let caf_data = p4k
        .read(entry)
        .map_err(|error| Error::Other(format!("cannot read {caf_path}: {error}")))?;
    Ok(super::caf::parse_caf_mode(&caf_data, endpoints)?.clips)
}

/// [`read_loose_caf`] for the pairing: a stored but unusable CAF is reported and yields no clip,
/// so one damaged file costs its own event, not the rig.
pub(crate) fn loose_caf_clips(
    p4k: &starbreaker_p4k::MappedP4k,
    caf_path: &str,
    endpoints: bool,
) -> Vec<AnimationClip> {
    read_loose_caf(p4k, caf_path, endpoints).unwrap_or_else(|error| {
        log::warn!("[anim] loose CAF {caf_path} is stored but unusable: {error}");
        Vec::new()
    })
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
    use super::*;

    fn rigs() -> starbreaker_p4k::MappedP4k {
        starbreaker_p4k::MappedP4k::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/clip_catalogue/rigs.zip"),
        )
        .unwrap()
    }

    #[test]
    fn loose_caf_stored_nowhere_is_no_clip_and_no_error() {
        let clips = read_loose_caf(&rigs(), "animations/synthetic/absent.caf", false).unwrap();
        assert!(clips.is_empty());
    }

    #[test]
    fn loose_caf_that_is_stored_but_not_a_caf_is_an_error() {
        assert!(read_loose_caf(&rigs(), "animations/synthetic/torn.caf", false).is_err());
    }

    #[test]
    fn unusable_loose_caf_costs_its_own_event_only() {
        // `loose.chrparams` names a good CAF, a stored non-CAF and a CAF stored nowhere.
        let value = extract_skeleton_json(&rigs(), "Objects/synthetic/loose.chr", true, false)
            .unwrap()
            .unwrap();
        let names: Vec<&str> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|clip| clip["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["latch_cycle"]);
    }
}
