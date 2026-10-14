//! JSON views of a parsed [`RigLogic`] for the `riglogic dump` CLI command
//! and the `riglogic_dump` MCP tool: the whole struct, or one named section
//! ([`SECTIONS`]: `header`, `joints`, `regions`, `appearance`, `corrective`,
//! `protos-info`) via [`section_json`].

use serde::Serialize;
use serde_json::{Value, json};

use super::{ProtosInfo, RigLogic};

/// The section names [`section_json`] accepts.
pub const SECTIONS: &str = "header | joints | regions | appearance | corrective | protos-info";

/// `rig` — with `protos_info` when its source carried one — as JSON: the
/// whole struct, or only `section`. Errors name an unknown section, or a
/// `protos-info` request without a ProtosInfo chunk.
pub fn section_json(rig: &RigLogic, protos_info: Option<&ProtosInfo>, section: Option<&str>) -> Result<Value, String> {
    match section {
        None => {
            let mut v = to_value(rig)?;
            if let Some(pi) = protos_info {
                v["protos_info"] = to_value(pi)?;
            }
            Ok(v)
        }
        Some("header") => Ok(json!({
            "name": rig.name,
            "rig_type": rig.rig_type,
            "file_size": rig.file_size,
            "header_size": rig.header_size,
            "identity_crc_lo": rig.identity_crc_lo,
            "identity_crc_hi_or_x": rig.identity_crc_hi_or_x,
            "num_appearance_blendshapes": rig.num_appearance_blendshapes,
            "num_corrective_blendshapes": rig.num_corrective_blendshapes,
            "num_joints": rig.num_joints,
            "num_main_delta_records": rig.num_main_delta_records,
            "num_secondary_delta_records": rig.num_secondary_delta_records,
            "num_blend_regions": rig.num_blend_regions,
            "packed_value_bit_width": rig.packed_value_bit_width,
            "primary_delta_scale": rig.primary_delta_scale,
            "secondary_delta_scale": rig.secondary_delta_scale,
            "section_starts": rig.section_starts,
        })),
        Some("joints") => Ok(json!({
            "joint_control_name_crcs": rig.joint_control_name_crcs,
            "joint_control_names": rig.joint_control_names,
            "skeleton_bone_name_crcs": rig.skeleton_bone_name_crcs,
        })),
        Some("regions") => Ok(json!({
            "num_blend_regions": rig.num_blend_regions,
            "blend_region_bones": rig.blend_region_bones,
            "region_weights": rig.region_weights,
        })),
        Some("appearance") => Ok(json!({
            "num_main_delta_records": rig.num_main_delta_records,
            "primary_delta_scale": rig.primary_delta_scale,
            "packed_value_bit_width": rig.packed_value_bit_width,
            "appearance": {
                "scale": rig.appearance.scale,
                "record_count": rig.appearance.records.len(),
                "records": rig.appearance.records,
            },
        })),
        Some("corrective") => Ok(json!({
            "num_secondary_delta_records": rig.num_secondary_delta_records,
            "secondary_delta_scale": rig.secondary_delta_scale,
            "packed_value_bit_width": rig.packed_value_bit_width,
            "corrective": {
                "scale": rig.corrective.scale,
                "record_count": rig.corrective.records.len(),
                "records": rig.corrective.records,
            },
            "num_corrective_blendshapes": rig.num_corrective_blendshapes,
            "corrective_composition": rig.corrective_composition,
        })),
        Some("protos-info") => match protos_info {
            Some(pi) => to_value(pi),
            None => Err("section 'protos-info' needs a .skin IVO container: \
                         standalone .RigLogic files carry no ProtosInfo chunk"
                .to_string()),
        },
        Some(unknown) => Err(format!("unknown section '{unknown}'; valid values: {SECTIONS}")),
    }
}

fn to_value(v: &impl Serialize) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("serialize error: {e}"))
}

