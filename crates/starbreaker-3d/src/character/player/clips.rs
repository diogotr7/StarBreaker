//! Player-only animation metadata. Existing entity sidecars are unchanged.
//! IvoAnimInfo flags bit 0 is additive and bit 1 cyclic (asset flags, not
//! controller compression flags). Root motion is in native SC metres/wxyz.
use super::xml::{Result, error};
use crate::animation::{AnimationClip, bone_name_hash, clip_to_json};
use crate::skeleton::Bone;
use serde_json::{Value, json};

pub(super) fn asset_flags(flags: u32) -> (bool, bool) {
    (flags & 1 != 0, flags & 2 != 0)
}

pub(super) fn parameter(name: &str, root: &Value, duration: f64, scale: f64) -> Result<f64> {
    let vector = |v: &Value| -> Result<glam::DVec3> {
        let a = v.as_array().ok_or_else(|| error("missing root position"))?;
        Ok(glam::DVec3::new(
            a[0].as_f64().unwrap(),
            a[1].as_f64().unwrap(),
            a[2].as_f64().unwrap(),
        ))
    };
    let start = vector(&root["start"]["position"])?;
    let end = vector(&root["end"]["position"])?;
    let q = &root["start"]["rotation"];
    // Metadata uses wxyz to match the rest of the scene transforms.
    let q = glam::DQuat::from_xyzw(
        q[1].as_f64().unwrap(),
        q[2].as_f64().unwrap(),
        q[3].as_f64().unwrap(),
        q[0].as_f64().unwrap(),
    );
    let delta = q.inverse() * (end - start);
    match name {
        "MoveSpeed" if duration > 0.0 => Ok(delta.truncate().length() / duration * scale),
        "TravelAngle" => Ok(if delta.truncate().length() < 1e-8 {
            0.0
        } else {
            (-delta.x).atan2(delta.y)
        }),
        "TurnSpeed" | "TurnAngle" if duration > 0.0 => {
            let e = &root["end"]["rotation"];
            let e = glam::DQuat::from_xyzw(
                e[1].as_f64().unwrap(),
                e[2].as_f64().unwrap(),
                e[3].as_f64().unwrap(),
                e[0].as_f64().unwrap(),
            );
            Ok((q.inverse() * e).to_euler(glam::EulerRot::XYZ).2
                * if name == "TurnSpeed" { scale / duration } else { 1.0 })
        }
        "TravelSlope" => Ok(delta.z.atan2(delta.truncate().length())),
        _ => Err(error(format!(
            "cannot derive motion parameter {name} (duration {duration})"
        ))),
    }
}

pub(super) fn sidecar(clip: &AnimationClip, flags: u32, skeleton: &str, bones: &[Bone]) -> Result<Value> {
    let root = bones
        .iter()
        .find(|b| b.parent_index.is_none())
        .ok_or_else(|| error("player has no root bone"))?;
    let (additive, cyclic) = asset_flags(flags);
    let mut value = clip_to_json(clip);
    let duration = clip
        .channels
        .iter()
        .flat_map(|c| {
            c.rotations
                .iter()
                .map(|k| k.time)
                .chain(c.positions.iter().map(|k| k.time))
        })
        .fold(0.0_f32, f32::max)
        / clip.fps;
    let channel = clip.channels.iter().find(|c| c.bone_hash == bone_name_hash(&root.name));
    let endpoint = |last: bool| {
        let pos = channel
            .and_then(|c| if last { c.positions.last() } else { c.positions.first() })
            .map(|k| k.value)
            .unwrap_or(if additive { [0.0; 3] } else { root.local_position });
        let q = channel
            .and_then(|c| if last { c.rotations.last() } else { c.rotations.first() })
            .map(|k| [k.value[3], k.value[0], k.value[1], k.value[2]])
            .unwrap_or(if additive {
                [1.0, 0.0, 0.0, 0.0]
            } else {
                root.local_rotation
            });
        json!({"position":pos,"rotation":q})
    };
    value["additive"] = json!(additive);
    value["cyclic"] = json!(cyclic);
    value["asset_flags"] = json!(flags);
    value["duration"] = json!(duration);
    value["root_motion"] = json!({"bone":root.name,"space":"sc_local","start":endpoint(false),"end":endpoint(true)});
    value["source_skeleton_path"] = json!(skeleton);
    if let Some(map) = value["bones"].as_object_mut() {
        for bone in bones {
            let key = format!("0x{:X}", bone_name_hash(&bone.name));
            if let Some(c) = map.get_mut(&key) {
                c["source_node_name"] = json!(bone.name);
                c["source_skeleton_path"] = json!(skeleton);
            }
        }
    }
    Ok(value)
}
