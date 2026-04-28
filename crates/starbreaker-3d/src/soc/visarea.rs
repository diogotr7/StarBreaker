//! Visibility-area / portal data parsed from chunk 0x000E.
//!
//! Each VisArea chunk in a `.soc` file describes the rooms and the portals
//! that connect them. The viewer does not (yet) cull on this data, but having
//! it parsed unblocks future portal-based culling and is also useful for
//! diagnostics ("how many rooms in this scene?").
//!
//! # Layout
//!
//! Each chunk starts with a 20-byte `SVisAreaManChunkHeader`:
//!
//! | Offset | Field                                     |
//! |--------|-------------------------------------------|
//! | +0x00  | `nVersion` (u8) — 5/6/7/8 across builds  |
//! | +0x01  | `nDummy`   (u8)                           |
//! | +0x02  | `nFlags`   (u8)                           |
//! | +0x03  | `nFlags2`  (u8)                           |
//! | +0x04  | `nChunkSize` (u32)                        |
//! | +0x08  | `visAreaCount` (u32)                      |
//! | +0x0C  | `portalCount` (u32)                       |
//! | +0x10  | `occlAreaCount` (u32)                     |
//!
//! After the header are `visAreaCount + portalCount + occlAreaCount`
//! variable-length entries. In the SC v15 layout each entry begins with a
//! `nChunkVersion` u32 (`= 15`) followed by a 32-byte ASCII name and a body
//! whose total size we recover by scanning forward to the next entry marker.
//!
//! For the renderer we surface the per-area name, vertex polygon (when
//! present), portal connection list, and (when recoverable) the AABB. The
//! detailed flags and per-area fields are passed through opaquely; nothing
//! in the viewer consumes them yet.

use super::brushes::SocError;
use super::common::{
    CHUNK_TYPE_VISAREA, ChunkEntry, chunk_slice, parse_chunk_table, parse_crch_header,
    read_f32, read_u32,
};

const VISAREA_HEADER_SIZE: usize = 20;
const ENTRY_NAME_LEN: usize = 32;
const ENTRY_V15_VERSION: u32 = 15;
const MAX_VERTICES: usize = 200; // matches reference parser ceiling

// Field offsets inside one v15 entry, relative to the entry start.
const ENTRY_OFF_NAME: usize = 4;
const ENTRY_OFF_PORTAL_BLENDING: usize = 44;
const ENTRY_OFF_VIEW_DIST_RATIO: usize = 48;
const ENTRY_OFF_HEIGHT_PRIMARY: usize = 80;
const ENTRY_OFF_HEIGHT_FALLBACK: usize = 228;
const ENTRY_OFF_CONNECTIONS: usize = 124;
const CONNECTION_COUNT: usize = 8;
const ENTRY_OFF_VERTEX_FLAG: usize = 264;
const ENTRY_OFF_POINT_COUNT: usize = 268;
const ENTRY_OFF_VERTICES: usize = 272;

// ── Public types ────────────────────────────────────────────────────────────

/// One visibility area, portal, or occluder area.
#[derive(Debug, Clone)]
pub struct VisAreaRecord {
    pub name: String,
    /// Source role: a room, a portal, or an occluder area.
    pub role: VisAreaRole,
    /// View-distance scaling factor.
    pub view_dist_ratio: f32,
    /// Portal blend factor (0 for non-portal records).
    pub portal_blending: f32,
    /// Height extent (vertical extrusion of the floor polygon).
    pub height: f32,
    /// Connection indices into the same `VisAreaSet` (negative entries
    /// stripped). For portals these point at the two rooms they connect.
    pub connections: Vec<i32>,
    /// Floor polygon vertices in entry-local coordinates, when present.
    /// VisAreas without an explicit polygon (the engine derives them from
    /// the AABB) yield an empty list.
    pub vertices: Vec<[f32; 3]>,
}

/// Whether the record came from the visAreas / portals / occluders block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisAreaRole {
    VisArea,
    Portal,
    OcclArea,
}

/// All the records read from a single SOC's chunk 0x000E.
#[derive(Debug, Clone, Default)]
pub struct SocVisAreas {
    pub vis_areas: Vec<VisAreaRecord>,
    pub portals: Vec<VisAreaRecord>,
    pub occl_areas: Vec<VisAreaRecord>,
}

impl SocVisAreas {
    pub fn total(&self) -> usize {
        self.vis_areas.len() + self.portals.len() + self.occl_areas.len()
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse VisArea data from a SOC. Containers without a 0x000E chunk produce
/// an empty result rather than an error.
pub fn parse(data: &[u8]) -> Result<SocVisAreas, SocError> {
    let header = parse_crch_header(data).ok_or(SocError::BadMagic)?;
    let chunks = parse_chunk_table(data, &header).ok_or(SocError::BadChunkTable)?;

    let mut out = SocVisAreas::default();
    for chunk in chunks.iter().filter(|c| c.chunk_type == CHUNK_TYPE_VISAREA) {
        parse_one_chunk(data, chunk, &mut out);
    }
    Ok(out)
}

// ── Chunk walker ────────────────────────────────────────────────────────────

fn parse_one_chunk(data: &[u8], chunk: &ChunkEntry, out: &mut SocVisAreas) {
    let body = chunk_slice(data, chunk);
    if body.len() < VISAREA_HEADER_SIZE {
        return;
    }

    let _version = body[0];
    // header[1] = nDummy, header[2] = nFlags, header[3] = nFlags2 (unused)
    let _chunk_size = read_u32(body, 4);
    let vis_count = read_u32(body, 8) as usize;
    let portal_count = read_u32(body, 12) as usize;
    let occl_count = read_u32(body, 16) as usize;
    let total = vis_count + portal_count + occl_count;

    if total == 0 {
        return;
    }

    let entry_offsets = locate_v15_entry_offsets(body);
    if entry_offsets.is_empty() {
        return;
    }

    for (idx, &start) in entry_offsets.iter().enumerate() {
        let end = entry_offsets
            .get(idx + 1)
            .copied()
            .unwrap_or(body.len());
        let role = if idx < vis_count {
            VisAreaRole::VisArea
        } else if idx < vis_count + portal_count {
            VisAreaRole::Portal
        } else {
            VisAreaRole::OcclArea
        };

        if let Some(record) = decode_entry_v15(body, start, end, role) {
            match role {
                VisAreaRole::VisArea => out.vis_areas.push(record),
                VisAreaRole::Portal => out.portals.push(record),
                VisAreaRole::OcclArea => out.occl_areas.push(record),
            }
        }
    }
}

/// Walk the chunk body looking for v15 entry markers (`u32 == 15` followed
/// by an ASCII name). Mirrors the reference parser's heuristic.
fn locate_v15_entry_offsets(body: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    if body.len() < VISAREA_HEADER_SIZE + 36 {
        return offsets;
    }

    let mut pos = VISAREA_HEADER_SIZE;
    while pos + 36 <= body.len() {
        let cv = read_u32(body, pos);
        if cv == ENTRY_V15_VERSION
            && let Some(name) = ascii_name_at(body, pos + ENTRY_OFF_NAME, ENTRY_NAME_LEN)
            && name.len() >= 2
            && name.chars().take(5).any(|c| c.is_ascii_alphabetic())
        {
            offsets.push(pos);
            pos += 1; // proceed byte-by-byte; entries can be tightly packed
            continue;
        }
        pos += 1;
    }

    offsets
}

fn ascii_name_at(body: &[u8], off: usize, len: usize) -> Option<String> {
    if off + len > body.len() {
        return None;
    }
    let raw = &body[off..off + len];
    let nul = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let s = std::str::from_utf8(&raw[..nul]).ok()?;
    if s.chars().any(|c| !c.is_ascii_graphic() && !c.is_ascii_whitespace()) {
        return None;
    }
    Some(s.trim().to_string())
}

fn decode_entry_v15(
    body: &[u8],
    start: usize,
    end: usize,
    _role: VisAreaRole,
) -> Option<VisAreaRecord> {
    if start + ENTRY_OFF_VERTICES > body.len() {
        return None;
    }
    let entry_end = end.min(body.len());

    let name = ascii_name_at(body, start + ENTRY_OFF_NAME, ENTRY_NAME_LEN).unwrap_or_default();
    let portal_blending = read_f32(body, start + ENTRY_OFF_PORTAL_BLENDING);
    let view_dist_ratio = read_f32(body, start + ENTRY_OFF_VIEW_DIST_RATIO);

    let height_primary = read_f32(body, start + ENTRY_OFF_HEIGHT_PRIMARY);
    let height_fallback = read_f32(body, start + ENTRY_OFF_HEIGHT_FALLBACK);
    let height = if height_primary.abs() > 0.01 {
        height_primary
    } else {
        height_fallback
    };

    let mut connections = Vec::with_capacity(CONNECTION_COUNT);
    for j in 0..CONNECTION_COUNT {
        let off = start + ENTRY_OFF_CONNECTIONS + j * 4;
        if off + 4 > body.len() {
            break;
        }
        let raw = read_u32(body, off) as i32;
        if raw >= 0 {
            connections.push(raw);
        }
    }

    let vertex_flag = read_u32(body, start + ENTRY_OFF_VERTEX_FLAG);
    let mut vertices = Vec::new();
    if vertex_flag != 0 && start + ENTRY_OFF_VERTICES <= body.len() {
        let pts_count = read_u32(body, start + ENTRY_OFF_POINT_COUNT) as usize;
        if pts_count > 0 && pts_count <= MAX_VERTICES {
            for v in 0..pts_count {
                let voff = start + ENTRY_OFF_VERTICES + v * 12;
                if voff + 12 > entry_end {
                    break;
                }
                vertices.push([
                    read_f32(body, voff),
                    read_f32(body, voff + 4),
                    read_f32(body, voff + 8),
                ]);
            }
        }
    }

    Some(VisAreaRecord {
        name,
        role: _role,
        view_dist_ratio,
        portal_blending,
        height,
        connections,
        vertices,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chunk_returns_empty_visareas() {
        let data = build_minimal_crch_no_visarea();
        let parsed = parse(&data).expect("parse ok");
        assert_eq!(parsed.total(), 0);
    }

    fn build_minimal_crch_no_visarea() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"CrCh");
        data.extend_from_slice(&0x0746u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // chunk_count
        data.extend_from_slice(&16u32.to_le_bytes()); // chunk_table_offset
        data
    }

    #[test]
    fn rejects_non_crch_input() {
        let data = vec![0u8; 16];
        assert!(matches!(parse(&data), Err(SocError::BadMagic)));
    }
}
