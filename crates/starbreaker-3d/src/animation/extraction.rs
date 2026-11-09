//! Shared full/sparse extraction from a rig's chrparams, DBA/CAF and event metadata.
use super::serialise::{annotate_animation_json_source, database_to_animations_json};
use super::{AnimationDatabase, matching::caf_anchored_remap, pose::bone_name_hash};
use crate::error::Error;
use std::collections::{HashMap, HashSet};

pub fn extract_animations_for_skeleton_json(
    p4k: &starbreaker_p4k::MappedP4k,
    skeleton_path: &str,
    include_unmatched_dba_blocks: bool,
    allow_bone_subset_fallback: bool,
) -> Result<Option<serde_json::Value>, Error> {
    extract_skeleton_json(
        p4k,
        skeleton_path,
        include_unmatched_dba_blocks,
        allow_bone_subset_fallback,
        false,
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
    let (skeleton_bone_hashes, skeleton_bone_name_by_hash): (HashSet<u32>, HashMap<u32, String>) =
        p4k.entry_case_insensitive(&skeleton_p4k_path)
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
                clip["catalogue_source"] = serde_json::json!(if chrparams
                    .animations
                    .contains_key(clip["name"].as_str().unwrap_or(""))
                {
                    "Chrparams"
                } else {
                    "DbaOnly"
                });
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
