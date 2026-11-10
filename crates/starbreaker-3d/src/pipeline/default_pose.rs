//! Default-pose bake: `apply_default_animation_pose_for_skel` poses a skeleton at the final
//! frame of the first `default_animation_tags` event its `.chrparams` declares. The event's
//! animation data is resolved by the same rule as the exported clips (`clips_for_caf_path`).

use starbreaker_p4k::MappedP4k;

use super::{ExportOptions, datacore_path_to_p4k};

/// Look up a `.chrparams` for the given skeleton path, find an animation
/// matching one of `opts.default_animation_tags`, and bake its final-frame
/// pose into `bones` in native CryEngine Y-up, preserving unkeyed bind
/// components. Sidecar axis conversion is not part of this bake.
/// Returns the number of bones updated (0 = nothing
/// applied / chrparams missing / animation missing).
pub(super) fn apply_default_animation_pose_for_skel(
    p4k: &MappedP4k,
    skel_path: &str,
    bones: &mut [crate::skeleton::Bone],
    opts: &ExportOptions,
) -> usize {
    // Derive the .chrparams path: replace .chr/.skin extension with .chrparams.
    let chrparams_path = match swap_extension(skel_path, "chrparams") {
        Some(p) => p,
        None => return 0,
    };
    let p4k_path = datacore_path_to_p4k(&chrparams_path);
    let bytes = match p4k
        .entry_case_insensitive(&p4k_path)
        .and_then(|entry| p4k.read(entry).ok())
    {
        Some(b) => b,
        None => return 0,
    };
    let cp = match crate::chrparams::ChrParams::from_bytes(&bytes) {
        Ok(cp) => cp,
        Err(e) => {
            log::warn!("[anim] failed to parse {chrparams_path}: {e}");
            return 0;
        }
    };
    // Pick the first matching animation tag.
    let mut tag_match: Option<(&str, String)> = None;
    for tag in &opts.default_animation_tags {
        if let Some(p) = cp.animations.get(tag) {
            tag_match = Some((tag.as_str(), p.clone()));
            break;
        }
    }
    let (_tag, caf_path) = match tag_match {
        Some(t) => t,
        None => return 0,
    };
    // The event plays the CAF its path names, resolved as for the exported clips: the
    // tracks-database entry of that name, else the loose file.
    let db = match crate::animation::extraction::load_tracks_database(p4k, &cp, false) {
        Ok(db) => db,
        Err(e) => {
            log::warn!("[anim] no default pose for {chrparams_path}: {e}");
            return 0;
        }
    };
    let clips = crate::animation::matching::clips_for_caf_path(&db, &caf_path, |path| {
        crate::animation::extraction::loose_caf_clips(p4k, path, false)
    });
    let mut pose = std::collections::HashMap::new();
    for clip in &clips {
        pose.extend(crate::animation::clip_final_pose(clip));
    }
    crate::animation::apply_pose_to_skeleton(bones, &pose)
}

/// Replace the file extension of `path` with `new_ext` (no leading dot).
/// Returns `None` if `path` has no extension.
fn swap_extension(path: &str, new_ext: &str) -> Option<String> {
    let dot = path.rfind('.')?;
    let slash = path.rfind(|c: char| c == '/' || c == '\\').unwrap_or(0);
    if dot < slash {
        return None;
    }
    let mut out = String::with_capacity(dot + 1 + new_ext.len());
    out.push_str(&path[..dot + 1]);
    out.push_str(new_ext);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bone(name: &str) -> crate::skeleton::Bone {
        crate::skeleton::Bone {
            name: name.into(),
            parent_index: None,
            object_node_index: None,
            local_position: [0.0; 3],
            local_rotation: [1.0, 0.0, 0.0, 0.0],
            world_position: [0.0; 3],
            world_rotation: [1.0, 0.0, 0.0, 0.0],
            bind_world_position: [0.0; 3],
            bind_world_rotation: [1.0, 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn default_pose_comes_from_a_loose_caf_when_the_event_names_one() {
        // `loose.chrparams` declares no tracks database; `latch_cycle` names a loose CAF that
        // moves `synthetic_latch` 0 -> 4 -> 8 along X.
        let archive = MappedP4k::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/clip_catalogue/rigs.zip"),
        )
        .unwrap();
        let opts = ExportOptions {
            default_animation_tags: vec!["latch_cycle".into()],
            ..Default::default()
        };
        let mut bones = vec![bone("synthetic_latch"), bone("synthetic_frame")];
        let updated = apply_default_animation_pose_for_skel(
            &archive,
            "Objects/synthetic/loose.chr",
            &mut bones,
            &opts,
        );
        assert_eq!(updated, 1);
        assert_eq!(bones[0].local_position, [8.0, 0.0, 0.0]);
        assert_eq!(bones[1].local_position, [0.0; 3]);
    }
}
