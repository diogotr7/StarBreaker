//! Bone-attached props: rigid items worn on a bone, not skinned to the
//! character (a backpack on its core armour's `backpack` port).
//!
//! The engine puts such an item on a port whose attachment is a bone
//! (`SItemPortDefAttachmentImplementationBone`): the port's helper names the
//! bone and an offset from it, and the helper on the item
//! (`ItemOffsetHelperName`, a node of the item's `.cga`) is the point of the
//! item placed there. A helper's `SubHelpers` refine it for items that carry
//! all of a sub-helper's tags (`qrt_utility_heavy` backpacks sit lower on a
//! Quirinus core).
//!
//! - [`PortHelper`] / [`PortHelper::select`]: a bone port's helper tree and the
//!   helper an item takes.
//! - [`prop_offset`]: the item's transform in the bone's frame, as the
//!   payload offset the writers take.
//! - [`Prop`]: a worn prop resolved for a gender ([`super::loadout`]);
//!   [`prop_payloads`]: props as package children on the root's bones.

use starbreaker_common::CigGuid;
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;

use crate::pipeline::ExportOptions;
use crate::types::EntityPayload;

/// A bone port's helper (`SItemPortDefHelperNode`): the bone, the offset from
/// it (position; rotation as `Ang3` degrees), the item's helper put there,
/// and the sub-helpers for items with their `tags`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortHelper {
    pub tags: Vec<String>,
    pub bone: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub item_helper: String,
    pub sub_helpers: Vec<PortHelper>,
}

impl PortHelper {
    /// The helper an item with `item_tags` takes: the first sub-helper whose
    /// tags are all the item's (any case), refined the same way, else this
    /// one.
    pub fn select(&self, item_tags: &[String]) -> &PortHelper {
        self.sub_helpers
            .iter()
            .find(|sub| sub.tags.iter().all(|t| item_tags.iter().any(|i| i.eq_ignore_ascii_case(t))))
            .map(|sub| sub.select(item_tags))
            .unwrap_or(self)
    }
}

/// The transform of an item in its bone's frame, as a payload offset
/// (position, XYZ Euler degrees — `offset_to_gltf_matrix`'s convention): the
/// port offset, then the inverse of the item's helper (`helper_to_item`, the
/// helper's NMC `bone_to_world` in the item's frame) so that helper lands on
/// the offset point. No helper: the item's origin goes there.
pub fn prop_offset(helper: &PortHelper, helper_to_item: Option<&[[f32; 4]; 3]>) -> ([f32; 3], [f32; 3]) {
    let r = helper.rotation.map(f32::to_radians);
    let offset = glam::Mat4::from_rotation_translation(
        glam::Quat::from_euler(glam::EulerRot::XYZ, r[0], r[1], r[2]),
        glam::Vec3::from(helper.position),
    );
    let item = match helper_to_item {
        Some(m) => offset * nmc_matrix(m).inverse(),
        None => offset,
    };
    let (_, rotation, translation) = item.to_scale_rotation_translation();
    let (x, y, z) = rotation.to_euler(glam::EulerRot::XYZ);
    (translation.into(), [x.to_degrees(), y.to_degrees(), z.to_degrees()])
}

/// An NMC 3x4 row-major matrix (translation in column 3).
fn nmc_matrix(m: &[[f32; 4]; 3]) -> glam::Mat4 {
    glam::Mat4::from_cols(
        glam::Vec4::new(m[0][0], m[1][0], m[2][0], 0.0),
        glam::Vec4::new(m[0][1], m[1][1], m[2][1], 0.0),
        glam::Vec4::new(m[0][2], m[1][2], m[2][2], 0.0),
        glam::Vec4::new(m[0][3], m[1][3], m[2][3], 1.0),
    )
}

/// A worn prop resolved for a gender: the item, the port it is on, the
/// helper it takes there, and its mesh (non-`.skin` geometry and `.mtl`).
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    /// The item record (`EntityClassDefinition`) GUID.
    pub record: CigGuid,
    /// Entity class name.
    pub item: String,
    /// The item whose port it is on (entity class name).
    pub on: String,
    pub port: String,
    pub helper: PortHelper,
    pub geometry: String,
    pub material: String,
}

/// `props` as child payloads of the root `root`: each item's mesh loaded
/// like an entity's (its `.cga`, `.mtl` and NMC), on its port helper's bone
/// at [`prop_offset`], with the tint palette it is worn with by `gender`.
/// Props whose mesh fails to load are reported on stderr and left out.
pub(crate) fn prop_payloads(p4k: &MappedP4k, db: &Database, props: &[Prop], root: &str, gender: &str, opts: &ExportOptions) -> Vec<EntityPayload> {
    props
        .iter()
        .filter_map(|prop| {
            let (mesh, materials, textures, nmc, palette, geometry_path, _, bones, skeleton_source_path) =
                match crate::pipeline::export_entity_from_paths(p4k, &prop.geometry, &prop.material, opts) {
                    Ok(loaded) => loaded,
                    Err(e) => {
                        eprintln!("prop {}: skipped ({e})", prop.item);
                        return None;
                    }
                };
            let mut payload = super::shared::part_payload(root, prop.item.clone(), geometry_path, mesh, materials, nmc);
            payload.textures = textures;
            payload.bones = bones;
            payload.skeleton_source_path = skeleton_source_path;
            let helper_to_item = payload
                .nmc
                .as_ref()
                .and_then(|nmc| nmc.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&prop.helper.item_helper)))
                .map(|n| n.bone_to_world);
            if helper_to_item.is_none() && !prop.helper.item_helper.is_empty() {
                eprintln!("prop {}: no helper {} in {}, its origin goes on the bone", prop.item, prop.helper.item_helper, prop.geometry);
            }
            let (position, rotation) = prop_offset(&prop.helper, helper_to_item.as_ref());
            payload.parent_node_name = prop.helper.bone.clone();
            payload.offset_position = position;
            payload.offset_rotation = rotation;
            payload.palette =
                db.record_by_id(&prop.record).and_then(|r| super::loadout::worn_palette(db, r, gender)).or(palette);
            Some(payload)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper(tags: &[&str], bone: &str, sub_helpers: Vec<PortHelper>) -> PortHelper {
        PortHelper { tags: tags.iter().map(|t| t.to_string()).collect(), bone: bone.into(), sub_helpers, ..Default::default() }
    }
    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|t| t.to_string()).collect()
    }

    /// An item carrying a sub-helper's tags takes it; others the port's own.
    #[test]
    fn items_take_the_sub_helper_their_tags_match() {
        let port = helper(&[], "bone", vec![helper(&["qrt_utility_heavy"], "bone_qrt", vec![helper(&["deep"], "bone_deep", vec![])])]);
        assert_eq!(port.select(&tags(&["backpack", "QRT_Utility_Heavy"])).bone, "bone_qrt");
        assert_eq!(port.select(&tags(&["qrt_utility_heavy", "deep"])).bone, "bone_deep");
        assert_eq!(port.select(&tags(&["backpack"])).bone, "bone");
    }

    fn apply(offset: ([f32; 3], [f32; 3]), p: glam::Vec3) -> glam::Vec3 {
        let r = offset.1.map(f32::to_radians);
        let m = glam::Mat4::from_rotation_translation(glam::Quat::from_euler(glam::EulerRot::XYZ, r[0], r[1], r[2]), offset.0.into());
        m.transform_point3(p)
    }

    /// The item's helper lands on the port's offset point, with the helper's
    /// axes on the offset's: the helper's own frame is undone.
    #[test]
    fn the_item_helper_lands_on_the_port_offset() {
        // A helper swapping Y and Z, flipping X, off the origin (synthetic).
        let helper_to_item = [[-1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, -0.25], [0.0, 1.0, 0.0, 0.5]];
        let port = PortHelper { position: [0.0, -0.03, -0.05], ..helper(&[], "bone", vec![]) };
        let offset = prop_offset(&port, Some(&helper_to_item));
        let helper_origin = glam::Vec3::new(0.0, -0.25, 0.5);
        assert!(apply(offset, helper_origin).distance(glam::Vec3::new(0.0, -0.03, -0.05)) < 1e-5, "{offset:?}");
        // The helper's +Z axis (item +Y) points along the bone's +Z.
        let along = apply(offset, helper_origin + glam::Vec3::Y) - apply(offset, helper_origin);
        assert!(along.distance(glam::Vec3::Z) < 1e-5, "{along:?}");
    }

    #[test]
    fn no_item_helper_puts_the_item_origin_on_the_offset() {
        let port = PortHelper { position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 90.0], ..Default::default() };
        let (position, rotation) = prop_offset(&port, None);
        assert_eq!(position, [1.0, 2.0, 3.0]);
        assert!((rotation[2] - 90.0).abs() < 1e-4, "{rotation:?}");
    }
}
