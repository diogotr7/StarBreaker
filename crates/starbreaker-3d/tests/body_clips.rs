//! Body animation clips (`.dba`) in the shared model: channels resolve to
//! `bhm_skeleton_v7` bones, and the samples are parent-local in the
//! skeleton's frame (the hips keep their bind height, the posed head stands
//! at human height).
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";

fn p4k() -> Option<MappedP4k> {
    let path = std::env::var("SC_DATA_P4K").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(DEFAULT_P4K_PATH));
    if !path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", path.display());
        return None;
    }
    Some(MappedP4k::open(&path).expect("open p4k"))
}

#[test]
fn character_select_clips_are_skeleton_local() {
    let Some(p4k) = p4k() else { return };
    let skeleton = starbreaker_3d::skeleton::parse_skeleton(
        &p4k.read_file(r"Data\Objects\Characters\Human\male_v7\export\bhm_skeleton_v7.chr").unwrap(),
    )
    .unwrap();
    let names: Vec<String> = skeleton.iter().map(|b| b.name.clone()).collect();
    let db = starbreaker_3d::animation::parse_dba(&p4k.read_file(r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba").unwrap())
        .unwrap();
    assert!(!db.clips.is_empty());

    let mut checked = 0;
    for clip in &db.clips {
        let shared = starbreaker_3d::animation::shared_clip(clip, &names);
        assert!(shared.invalid_track().is_none(), "{}", clip.name);
        let resolved = shared.bones.len() as f32 / clip.channels.len().max(1) as f32;
        assert!(resolved > 0.9, "{}: only {:.0}% of channels name skeleton bones", clip.name, resolved * 100.0);

        // Space: the hips' local translation keeps the bind's pelvis height
        // (an axis or space mix-up would not). Helper bones (`*_def`
        // correctives, IK locks) slide by design, so bone lengths are no test.
        let hips = shared.bones.iter().find(|t| t.bone == "Hips").and_then(|t| t.translation.as_ref()).expect("hips translation");
        let bind_hips = skeleton.iter().find(|b| b.name == "Hips").unwrap().local_position;
        assert!((hips.values[0][2] - bind_hips[2]).abs() < 0.1 * bind_hips[2], "{}: hips at {:?}", clip.name, hips.values[0]);

        // First frame, chained: the head stands at human height.
        let mut world: Vec<glam::Mat4> = Vec::with_capacity(skeleton.len());
        for bone in &skeleton {
            let track = shared.bones.iter().find(|t| t.bone == bone.name);
            let [w, x, y, z] = track.and_then(|t| t.rotation.as_ref()).map_or(bone.local_rotation, |k| k.values[0]);
            let t = track.and_then(|t| t.translation.as_ref()).map_or(bone.local_position, |k| k.values[0]);
            let local = glam::Mat4::from_rotation_translation(glam::Quat::from_xyzw(x, y, z, w), t.into());
            world.push(bone.parent_index.map_or(local, |p| world[p as usize] * local));
        }
        let head = skeleton.iter().position(|b| b.name == "Head").unwrap();
        let height = world[head].w_axis.z;
        eprintln!("{}: {} tracks, head at {:.2} m", clip.name, shared.bones.len(), height);
        assert!((1.2..2.1).contains(&height), "{}: head at {height} m", clip.name);
        checked += 1;
    }
    assert!(checked > 0);
}

/// DBA clip names come out clean: the names table is aligned, and its
/// padding is not always NUL bytes.
#[test]
fn dba_clip_names_are_clean_paths() {
    let Some(p4k) = p4k() else { return };
    let db = starbreaker_3d::animation::parse_dba(&p4k.read_file(r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba").unwrap())
        .unwrap();
    for clip in &db.clips {
        assert!(clip.name.starts_with("animations/"), "{:?}", clip.name);
    }
}
