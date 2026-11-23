//! Where a CDF `CA_BONE` attachment sits relative to its bone.
//!
//! An `<Attachment>` gives its pose one of two ways: `RelRotation` /
//! `RelPosition` in its bone's frame, or `Rotation` / `Position` in the
//! character's own (model) space, which the engine turns into a bone-relative
//! pose against the bone's default pose. Quaternions are written `w,x,y,z`.
//!
//! - [`AttachPose`]: the pose as the CDF gives it.
//! - [`attach_pose`]: reads it from an attachment's attributes.
//! - [`AttachPose::bone_local`]: the pose in the bone's frame, for a bone.

use std::collections::HashMap;

/// An attachment's pose, as its CDF gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AttachPose {
    /// In the bone's frame (`RelRotation` / `RelPosition`).
    BoneRelative(glam::Affine3A),
    /// In the character's model space (`Rotation` / `Position`).
    Model(glam::Affine3A),
}

impl Default for AttachPose {
    fn default() -> Self {
        AttachPose::BoneRelative(glam::Affine3A::IDENTITY)
    }
}

impl AttachPose {
    /// The pose in `bone`'s frame.
    pub(crate) fn bone_local(&self, bone: &crate::skeleton::Bone) -> glam::Affine3A {
        match *self {
            AttachPose::BoneRelative(local) => local,
            AttachPose::Model(model) => {
                glam::Affine3A::from_mat4(super::nmc_bridge::bone_world_transform(bone)).inverse() * model
            }
        }
    }
}

/// The pose an `<Attachment>`'s attributes give: the bone-relative pair when
/// either is present, else the model-space pair, else the bone itself.
pub(crate) fn attach_pose(attrs: &HashMap<&str, &str>) -> AttachPose {
    let pose = |rotation: &str, position: &str| {
        let rotation = attrs.get(rotation).and_then(|v| floats::<4>(v));
        let position = attrs.get(position).and_then(|v| floats::<3>(v));
        (rotation.is_some() || position.is_some()).then(|| {
            let [w, x, y, z] = rotation.unwrap_or([1.0, 0.0, 0.0, 0.0]);
            glam::Affine3A::from_rotation_translation(
                glam::Quat::from_xyzw(x, y, z, w).normalize(),
                glam::Vec3::from(position.unwrap_or([0.0; 3])),
            )
        })
    };
    if let Some(local) = pose("RelRotation", "RelPosition") {
        AttachPose::BoneRelative(local)
    } else if let Some(model) = pose("Rotation", "Position") {
        AttachPose::Model(model)
    } else {
        AttachPose::default()
    }
}

/// `N` comma-separated floats, or `None` when the value is not exactly that.
fn floats<const N: usize>(value: &str) -> Option<[f32; N]> {
    let mut out = [0.0; N];
    let mut parts = value.split(',');
    for slot in &mut out {
        *slot = parts.next()?.trim().parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs<'a>(pairs: &[(&'a str, &'a str)]) -> HashMap<&'a str, &'a str> {
        pairs.iter().copied().collect()
    }

    /// 90 degrees about +Z, at (10, 0, 0); quaternion stored [w, x, y, z].
    fn rotated_bone() -> crate::skeleton::Bone {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        crate::skeleton::Bone {
            name: "slide".to_string(),
            parent_index: None,
            object_node_index: None,
            local_position: [10.0, 0.0, 0.0],
            local_rotation: [h, 0.0, 0.0, h],
            world_position: [10.0, 0.0, 0.0],
            world_rotation: [h, 0.0, 0.0, h],
            bind_world_position: [10.0, 0.0, 0.0],
            bind_world_rotation: [h, 0.0, 0.0, h],
        }
    }

    fn close(a: glam::Vec3, b: glam::Vec3) -> bool {
        (a - b).abs().max_element() < 1e-5
    }

    #[test]
    fn a_bone_relative_pose_reads_w_first_quaternions() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let pose = attach_pose(&attrs(&[("RelRotation", &format!("{h},0,0,{h}")), ("RelPosition", "0,0,0.5")]));
        let AttachPose::BoneRelative(m) = pose else { panic!("{pose:?}") };
        assert!(close(m.transform_point3(glam::Vec3::X), glam::Vec3::new(0.0, 1.0, 0.5)), "{m:?}");
    }

    #[test]
    fn a_model_space_pose_reads_rotation_and_position() {
        let pose = attach_pose(&attrs(&[("Rotation", "0.99999994,0,-1.7763566e-15,0"), ("Position", "1,2,3")]));
        let AttachPose::Model(m) = pose else { panic!("{pose:?}") };
        assert!(close(m.transform_point3(glam::Vec3::ZERO), glam::Vec3::new(1.0, 2.0, 3.0)), "{m:?}");
    }

    #[test]
    fn an_attachment_without_a_pose_sits_on_its_bone() {
        assert_eq!(attach_pose(&attrs(&[("BoneName", "root")])), AttachPose::default());
    }

    /// A part authored in model space keeps its place when hung on a bone:
    /// its bone-local pose undoes the bone's.
    #[test]
    fn a_model_space_pose_undoes_its_bone() {
        let bone = rotated_bone();
        let world = glam::Affine3A::from_mat4(super::super::nmc_bridge::bone_world_transform(&bone));
        let local = AttachPose::Model(glam::Affine3A::from_translation(glam::Vec3::new(0.0, 0.0, 2.0))).bone_local(&bone);
        assert!(close((world * local).transform_point3(glam::Vec3::ZERO), glam::Vec3::new(0.0, 0.0, 2.0)));
        let rel = glam::Affine3A::from_translation(glam::Vec3::Y);
        assert_eq!(AttachPose::BoneRelative(rel).bone_local(&bone), rel);
    }
}
