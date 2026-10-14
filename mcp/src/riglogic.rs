//! RigLogic dump helpers for the `riglogic_dump` MCP tool.
//!
//! Provides the `RigLogicDumpRequest` input schema and the two dump entry points:
//! - `dump_standalone` — parses a raw `.RigLogic` byte slice.
//! - `dump_from_ivo`   — extracts the RigLogic (and optional ProtosInfo) chunk from
//!   a `.skin` IVO container byte slice.
//!
//! The actual tool method lives in the `#[tool_router]` impl block in `tools.rs`
//! so that the rmcp macro can register it.

use rmcp::schemars;
use starbreaker_3d::riglogic::sections::section_json;
use starbreaker_chunks::{ChunkFile, known_types};

/// Size of the 8-byte `data_size` prefix that the IVO chunk wrapper prepends to
/// embedded RigLogic chunk content.
const IVO_CHUNK_PREFIX_BYTES: usize = 8;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RigLogicDumpRequest {
    #[schemars(
        description = "Virtual P4k path to a standalone .RigLogic file (e.g. \
        'Data\\Objects\\Characters\\Human\\head_rig.RigLogic'). Mutually exclusive \
        with skin_path."
    )]
    pub standalone_path: Option<String>,
    #[schemars(
        description = "Virtual P4k path to a head .skin IVO container from which the \
        RigLogic chunk (and optionally ProtosInfo chunk) are auto-extracted. Mutually \
        exclusive with standalone_path."
    )]
    pub skin_path: Option<String>,
    #[schemars(
        description = "Restrict output to a named section: \
        'header' | 'joints' | 'regions' | 'appearance' | 'corrective' | 'protos-info'. \
        Omit to receive the full JSON dump."
    )]
    pub section: Option<String>,
}

/// Dump a standalone `.RigLogic` byte slice, returning pretty-printed JSON or an
/// error message.
pub fn dump_standalone(data: &[u8], section: Option<&str>) -> String {
    let rig = match starbreaker_3d::riglogic::RigLogic::read(data) {
        Ok(r) => r,
        Err(e) => return format!("Failed to parse RigLogic: {e}"),
    };
    match section_json(&rig, None, section) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|e| format!("JSON error: {e}")),
        Err(e) => e,
    }
}

/// Dump the RigLogic chunk (and optionally ProtosInfo) from a `.skin` IVO container
/// byte slice, returning pretty-printed JSON or an error message.
///
/// `label` is used only for error messages (pass the P4k path string).
pub fn dump_from_ivo(data: &[u8], section: Option<&str>, label: &str) -> String {
    let chunk_file = match ChunkFile::from_bytes(data) {
        Ok(cf) => cf,
        Err(e) => return format!("Failed to parse IVO container '{label}': {e}"),
    };
    let ChunkFile::Ivo(ivo) = &chunk_file else {
        return format!("'{label}' is not an IVO chunk file");
    };

    // Find the RigLogic chunk.
    let rl_entry = match ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == known_types::ivo::RIG_LOGIC)
    {
        Some(e) => e,
        None => {
            return format!(
                "No RigLogic chunk (0x{:08X}) in '{label}'",
                known_types::ivo::RIG_LOGIC
            );
        }
    };

    // Strip the 8-byte data_size prefix.
    let raw = ivo.chunk_data(rl_entry);
    if raw.len() < IVO_CHUNK_PREFIX_BYTES {
        return format!(
            "RigLogic chunk in '{label}' is too short ({} bytes)",
            raw.len()
        );
    }
    let rig_bytes = &raw[IVO_CHUNK_PREFIX_BYTES..];
    let rig = match starbreaker_3d::riglogic::RigLogic::read(rig_bytes) {
        Ok(r) => r,
        Err(e) => return format!("Failed to parse RigLogic from '{label}': {e}"),
    };

    // Optionally parse ProtosInfo.
    let protos_info = if section.is_none() || section == Some("protos-info") {
        ivo.chunks()
            .iter()
            .find(|c| c.chunk_type == known_types::ivo::PROTOS_INFO)
            .map(|entry| starbreaker_3d::riglogic::ProtosInfo::read(ivo.chunk_data(entry)))
            .transpose()
            .unwrap_or(None)
    } else {
        None
    };

    match section_json(&rig, protos_info.as_ref(), section) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|e| format!("JSON error: {e}")),
        Err(e) => e,
    }
}
