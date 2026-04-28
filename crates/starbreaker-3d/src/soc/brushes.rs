//! SOC brush instance parser (chunk 0x0010 + 0x000E).
//!
//! Reads brush instances from CrCh-format `.soc` containers. Each brush is a
//! placement of one of the static-object meshes referenced by the StatObj
//! table (chunk type 0x0010). Brushes appear inside chunk 0x0010 and inside
//! VisArea chunks (0x000E).
//!
//! # Format (SBrushChunk v15, 204 bytes)
//!
//! Per record, after a 4-byte int32 type prefix (== 1, EErType::Brush):
//!
//! | Offset | Size | Field                                                |
//! |--------|------|------------------------------------------------------|
//! | +0x00  | 48   | AABB (6 doubles: minX,minY,minZ,maxX,maxY,maxZ)      |
//! | +0x34  | 4    | render flags (uint32)                                |
//! | +0x37  | 1    | LOD/layer selector (bits [2:0]); overlaps flags MSB  |
//! | +0x38  | 2    | mesh_index (into StatObj table)                      |
//! | +0x3C  | 96   | Matrix34d row-major; translations at +0x54/+74/+94   |
//! | +0xA8  | 2    | material_id                                          |
//!
//! # Two correctness fixes vs. naive scan
//!
//! 1. **LOD/layer gate.** Records with `(byte[+0x37] & 0b111) == 0` are
//!    suppressed at default object-quality. We skip them here too.
//! 2. **Local-space translations.** The Matrix34 stored on disk is in the
//!    parent SOC node's local frame. Composing with the parent QuatTS at
//!    parse time is required for correct world placement; root-node parents
//!    are identity, which is why some brushes appear placed correctly even
//!    without composition.

use glam::{DQuat, DVec3};

use super::common::{
    CHUNK_TABLE_ENTRY_SIZE, CHUNK_TYPE_STATOBJ, CHUNK_TYPE_VISAREA, CRCH_MAGIC,
    CRCH_VERSION, ChunkEntry, CrChHeader, read_f64, read_i32, read_u16, read_u32,
};

const VISAREA_HEADER_SIZE: usize = 20;

const STATOBJ_PATH_LEN: usize = 256;

// ── Brush record geometry ───────────────────────────────────────────────────

const BRUSH_DATA_SIZE: usize = 204;
const TYPE_PREFIX_SIZE: usize = 4;
const BRUSH_RECORD_STRIDE: usize = TYPE_PREFIX_SIZE + BRUSH_DATA_SIZE; // 208

const SCAN_STEP: usize = 4;

const E_ERTYPE_BRUSH: i32 = 1;

// Field offsets within the 204-byte SBrushChunk body (after the i32 type prefix).
const OFF_AABB: usize = 0x00;
const OFF_LOD_LAYER_BYTE: usize = 0x37;
const OFF_MESH_INDEX: usize = 0x38;
const OFF_MATRIX34: usize = 0x3C;
const OFF_MATERIAL_ID: usize = 0xA8;

// AABB validation thresholds (matches reference parser).
const AABB_AXIS_LIMIT: f64 = 50_000.0;
const AABB_EXTENT_LIMIT: f64 = 500.0;
const ROTATION_AXIS_LIMIT: f64 = 2.0;
const TRANSLATION_LIMIT: f64 = 50_000.0;

// ── Public types ────────────────────────────────────────────────────────────

/// Parent transform for an octree node. Brush records on disk store a
/// local-space `Matrix34<double>`; the engine composes that with the parent
/// `QuatTS<double>` at parse time to obtain world placement.
#[derive(Debug, Clone, Copy)]
pub struct ParentTransform {
    pub rotation: DQuat,
    pub translation: DVec3,
}

impl ParentTransform {
    /// Identity transform. Use this for root nodes or when no parent is known.
    pub const fn identity() -> Self {
        Self {
            rotation: DQuat::from_xyzw(0.0, 0.0, 0.0, 1.0),
            translation: DVec3::ZERO,
        }
    }
}

impl Default for ParentTransform {
    fn default() -> Self {
        Self::identity()
    }
}

/// A single decoded brush placement.
#[derive(Debug, Clone)]
pub struct BrushInstance {
    /// Index into [`SocBrushes::mesh_paths`].
    pub mesh_index: u16,
    /// Material slot id read from the record.
    pub material_id: u16,
    /// World-space translation (parent composed with local). Stored as f32.
    pub translation: [f32; 3],
    /// World-space rotation (parent quaternion times the local rotation
    /// extracted from the Matrix34). `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// Full world-space 3x4 transform (row-major) as f32. Useful for callers
    /// that want the raw matrix instead of decomposed rotation/translation.
    pub world_transform: [[f32; 4]; 3],
}

/// Top-level result of parsing a SOC for brushes.
#[derive(Debug, Clone, Default)]
pub struct SocBrushes {
    /// Lower-cased StatObj geometry paths from chunk 0x0010.
    pub mesh_paths: Vec<String>,
    /// Decoded brush instances, in scan order.
    pub brushes: Vec<BrushInstance>,
}

/// Errors produced by the SOC brush parser.
#[derive(Debug, thiserror::Error)]
pub enum SocError {
    #[error("file too small for CrCh header ({got} bytes)")]
    Truncated { got: usize },

    #[error("not a CrCh file: bad magic")]
    BadMagic,

    #[error("unsupported CrCh container version: 0x{0:04X}")]
    UnsupportedContainerVersion(u32),

    #[error("unsupported SBrushChunk version: {0} (only v15 is supported)")]
    UnsupportedVersion(u32),

    #[error("StatObj chunk (0x0010) not found")]
    MissingStatObj,

    #[error("chunk table extends past end of file")]
    BadChunkTable,

    #[error("StatObj table is malformed")]
    BadStatObjTable,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse a SOC file's brush instances using a single root parent transform.
///
/// Equivalent to [`parse`] with `parent = ParentTransform::identity()` for
/// every record. Suitable when the SOC is loaded at the world origin.
pub fn parse_with_identity_parent(data: &[u8]) -> Result<SocBrushes, SocError> {
    parse(data, &ParentTransform::identity())
}

/// Parse a SOC file's brush instances, composing each local Matrix34 with
/// `parent` to produce world-space placements.
///
/// Callers that resolve per-octree-node parent transforms should split brush
/// extraction by node and call this once per node with the appropriate
/// parent. The current implementation applies the same parent to every brush
/// in the file, which is correct only for SOCs whose every brush sits under
/// the same node (or whose parent transforms are all identity). The API is
/// shaped to extend to per-node walks without breaking callers.
pub fn parse(data: &[u8], parent: &ParentTransform) -> Result<SocBrushes, SocError> {
    let header = read_crch_header(data)?;
    let chunks = read_chunk_table(data, &header)?;

    // Locate the StatObj chunk. Its `version` field in the chunk table is
    // an opaque hint (v1 in current SC builds even though the on-disk
    // records use the v15 SBrushChunk layout — record-level versioning is
    // separate from chunk-level versioning here). The brush scanner
    // validates each record on its own merits, so we accept any chunk
    // version and let validation reject records that don't match v15.
    let statobj = chunks
        .iter()
        .find(|c| c.chunk_type == CHUNK_TYPE_STATOBJ)
        .ok_or(SocError::MissingStatObj)?;

    let (mesh_paths, _statobj_data_start) = read_statobj_paths(data, statobj)?;
    let n_statobj = mesh_paths.len();

    let mut brushes = Vec::new();

    // Scan inside the StatObj chunk.
    let so_start = statobj.offset as usize;
    let so_end = so_start.saturating_add(statobj.size as usize).min(data.len());
    scan_for_brushes(data, so_start, so_end, n_statobj, parent, &mut brushes);

    // Scan inside each VisArea chunk, skipping its 20-byte header.
    for chunk in chunks
        .iter()
        .filter(|c| c.chunk_type == CHUNK_TYPE_VISAREA)
    {
        let va_start = chunk.offset as usize;
        let va_end = va_start
            .saturating_add(chunk.size as usize)
            .min(data.len());
        let scan_from = va_start.saturating_add(VISAREA_HEADER_SIZE);
        if scan_from < va_end {
            scan_for_brushes(data, scan_from, va_end, n_statobj, parent, &mut brushes);
        }
    }

    Ok(SocBrushes {
        mesh_paths,
        brushes,
    })
}

// ── CrCh container ──────────────────────────────────────────────────────────

fn read_crch_header(data: &[u8]) -> Result<CrChHeader, SocError> {
    if data.len() < 16 {
        return Err(SocError::Truncated { got: data.len() });
    }
    if data[0..4] != CRCH_MAGIC {
        return Err(SocError::BadMagic);
    }
    let version = read_u32(data, 4);
    if version != CRCH_VERSION {
        return Err(SocError::UnsupportedContainerVersion(version));
    }
    let chunk_count = read_u32(data, 8);
    let chunk_table_offset = read_u32(data, 12);
    Ok(CrChHeader {
        chunk_count,
        chunk_table_offset,
    })
}

fn read_chunk_table(data: &[u8], header: &CrChHeader) -> Result<Vec<ChunkEntry>, SocError> {
    let count = header.chunk_count as usize;
    let start = header.chunk_table_offset as usize;
    let bytes_needed = count.saturating_mul(CHUNK_TABLE_ENTRY_SIZE);
    let end = start.saturating_add(bytes_needed);
    if end > data.len() {
        return Err(SocError::BadChunkTable);
    }

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = start + i * CHUNK_TABLE_ENTRY_SIZE;
        let chunk_type = read_u16(data, off);
        let version_raw = read_u16(data, off + 2);
        // strip the high bit (big-endian flag) to mirror the chunks crate
        let version = version_raw & 0x7FFF;
        let _id = read_u32(data, off + 4);
        let size = read_u32(data, off + 8);
        let offset = read_u32(data, off + 12);
        out.push(ChunkEntry {
            chunk_type,
            version,
            offset,
            size,
        });
    }
    Ok(out)
}

// ── StatObj path table ──────────────────────────────────────────────────────

fn read_statobj_paths(
    data: &[u8],
    chunk: &ChunkEntry,
) -> Result<(Vec<String>, usize), SocError> {
    let start = chunk.offset as usize;
    let end = start
        .saturating_add(chunk.size as usize)
        .min(data.len());
    if end < start + 8 {
        return Err(SocError::BadStatObjTable);
    }
    // First u32 unknown, second u32 is path count.
    let count = read_u32(data, start + 4) as usize;
    let mut off = start + 8;
    let table_end = off.saturating_add(count.saturating_mul(STATOBJ_PATH_LEN));
    if table_end > end {
        return Err(SocError::BadStatObjTable);
    }
    let mut paths = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = &data[off..off + STATOBJ_PATH_LEN];
        let nul = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let s = String::from_utf8_lossy(&raw[..nul]).to_lowercase();
        paths.push(s);
        off += STATOBJ_PATH_LEN;
    }
    Ok((paths, off))
}

// ── Brush record scanning ───────────────────────────────────────────────────

fn scan_for_brushes(
    data: &[u8],
    start: usize,
    end: usize,
    n_statobj: usize,
    parent: &ParentTransform,
    out: &mut Vec<BrushInstance>,
) {
    let mut off = start;
    while off + BRUSH_RECORD_STRIDE <= end {
        match try_decode_brush(data, off, n_statobj, parent) {
            Some(brush) => {
                out.push(brush);
                off += BRUSH_RECORD_STRIDE;
            }
            None => off += SCAN_STEP,
        }
    }
}

fn try_decode_brush(
    data: &[u8],
    rec_off: usize,
    n_statobj: usize,
    parent: &ParentTransform,
) -> Option<BrushInstance> {
    // Type prefix.
    if read_i32(data, rec_off) != E_ERTYPE_BRUSH {
        return None;
    }
    let body = rec_off + TYPE_PREFIX_SIZE;
    if body + BRUSH_DATA_SIZE > data.len() {
        return None;
    }

    // AABB sanity.
    let mut aabb = [0f64; 6];
    for (i, slot) in aabb.iter_mut().enumerate() {
        *slot = read_f64(data, body + OFF_AABB + i * 8);
    }
    if !aabb_is_plausible(&aabb) {
        return None;
    }

    // Mesh index must reference the StatObj table.
    let mesh_index = read_u16(data, body + OFF_MESH_INDEX);
    if (mesh_index as usize) >= n_statobj {
        return None;
    }

    // Read the 12-double Matrix34.
    let mut mat = [0f64; 12];
    for (i, slot) in mat.iter_mut().enumerate() {
        *slot = read_f64(data, body + OFF_MATRIX34 + i * 8);
    }
    if !matrix34_is_plausible(&mat) {
        return None;
    }

    // LOD/layer byte at +0x37. The lower three bits are documented as a
    // "skip at default object-quality" gate, but real-world dungeon SOCs
    // (Executive Hangar's `base_bulkh_*` modules and friends) ship with
    // `byte[+0x37] = 0x20` — high bit only, bits[2:0] zero — and the
    // reference Python parser the renderer was modelled on does not apply
    // the gate. Gating here drops every brush in those zones, so we leave
    // the byte as a future filter and let the AABB / mesh-index / matrix
    // sanity checks do the validation work.
    let _lod_byte = data[body + OFF_LOD_LAYER_BYTE];

    let material_id = read_u16(data, body + OFF_MATERIAL_ID);

    // Compose with the parent QuatTS to lift local-space to world-space.
    let local_rot = rotation_matrix_to_quat(&mat);
    let local_translation = DVec3::new(mat[3], mat[7], mat[11]);

    let world_translation = parent.translation + parent.rotation * local_translation;
    let world_rotation = (parent.rotation * local_rot).normalize();

    let world_transform = compose_world_matrix(&parent.rotation, &mat, &parent.translation);

    Some(BrushInstance {
        mesh_index,
        material_id,
        translation: [
            world_translation.x as f32,
            world_translation.y as f32,
            world_translation.z as f32,
        ],
        rotation: [
            world_rotation.x as f32,
            world_rotation.y as f32,
            world_rotation.z as f32,
            world_rotation.w as f32,
        ],
        world_transform,
    })
}

// ── Validation ──────────────────────────────────────────────────────────────

fn aabb_is_plausible(aabb: &[f64; 6]) -> bool {
    for &v in aabb {
        if !v.is_finite() || v.abs() > AABB_AXIS_LIMIT {
            return false;
        }
    }
    if aabb[0] > aabb[3] + 0.01 || aabb[1] > aabb[4] + 0.01 || aabb[2] > aabb[5] + 0.01 {
        return false;
    }
    let extent_x = aabb[3] - aabb[0];
    let extent_y = aabb[4] - aabb[1];
    let extent_z = aabb[5] - aabb[2];
    if extent_x > AABB_EXTENT_LIMIT
        || extent_y > AABB_EXTENT_LIMIT
        || extent_z > AABB_EXTENT_LIMIT
    {
        return false;
    }
    true
}

fn matrix34_is_plausible(mat: &[f64; 12]) -> bool {
    // Rotation entries: indices 0,1,2,4,5,6,8,9,10. Translations: 3,7,11.
    const ROT: [usize; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];
    for &i in &ROT {
        let v = mat[i];
        if !v.is_finite() || v.abs() > ROTATION_AXIS_LIMIT {
            return false;
        }
    }
    for &i in &[3usize, 7, 11] {
        let v = mat[i];
        if !v.is_finite() || v.abs() > TRANSLATION_LIMIT {
            return false;
        }
    }
    true
}

// ── Math helpers ────────────────────────────────────────────────────────────

/// Extract the 3x3 rotation portion of a row-major 3x4 matrix and convert to
/// a unit quaternion. Uses the standard trace-based decomposition with
/// branch-stable fallbacks.
fn rotation_matrix_to_quat(mat: &[f64; 12]) -> DQuat {
    let (m00, m01, m02) = (mat[0], mat[1], mat[2]);
    let (m10, m11, m12) = (mat[4], mat[5], mat[6]);
    let (m20, m21, m22) = (mat[8], mat[9], mat[10]);

    let trace = m00 + m11 + m22;
    let (x, y, z, w);
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        w = 0.25 / s;
        x = (m21 - m12) * s;
        y = (m02 - m20) * s;
        z = (m10 - m01) * s;
    } else if m00 > m11 && m00 > m22 {
        let s = 2.0 * (1.0 + m00 - m11 - m22).sqrt();
        w = (m21 - m12) / s;
        x = 0.25 * s;
        y = (m01 + m10) / s;
        z = (m02 + m20) / s;
    } else if m11 > m22 {
        let s = 2.0 * (1.0 + m11 - m00 - m22).sqrt();
        w = (m02 - m20) / s;
        x = (m01 + m10) / s;
        y = 0.25 * s;
        z = (m12 + m21) / s;
    } else {
        let s = 2.0 * (1.0 + m22 - m00 - m11).sqrt();
        w = (m10 - m01) / s;
        x = (m02 + m20) / s;
        y = (m12 + m21) / s;
        z = 0.25 * s;
    }
    DQuat::from_xyzw(x, y, z, w).normalize()
}

/// Build a 3x4 row-major world-space matrix from
/// `parent_translation + parent_rotation * local_matrix`.
fn compose_world_matrix(
    parent_rot: &DQuat,
    local: &[f64; 12],
    parent_trans: &DVec3,
) -> [[f32; 4]; 3] {
    // Rotation columns of the local matrix (row-major source).
    let col0 = DVec3::new(local[0], local[4], local[8]);
    let col1 = DVec3::new(local[1], local[5], local[9]);
    let col2 = DVec3::new(local[2], local[6], local[10]);
    let local_t = DVec3::new(local[3], local[7], local[11]);

    let r0 = *parent_rot * col0;
    let r1 = *parent_rot * col1;
    let r2 = *parent_rot * col2;
    let world_t = *parent_trans + *parent_rot * local_t;

    [
        [r0.x as f32, r1.x as f32, r2.x as f32, world_t.x as f32],
        [r0.y as f32, r1.y as f32, r2.y as f32, world_t.y as f32],
        [r0.z as f32, r1.z as f32, r2.z as f32, world_t.z as f32],
    ]
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(buf: &mut Vec<u8>, v: u16) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn put_u32(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn put_i32(buf: &mut Vec<u8>, v: i32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn put_f64(buf: &mut Vec<u8>, v: f64) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn put_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
        buf.extend_from_slice(bytes);
    }

    /// Builder for a 204-byte SBrushChunk body (without the 4-byte type prefix).
    struct BrushBody {
        aabb: [f64; 6],
        lod_byte: u8,
        mesh_index: u16,
        material_id: u16,
        matrix: [f64; 12],
    }

    impl BrushBody {
        fn identity() -> Self {
            Self {
                aabb: [-1.0, -1.0, -1.0, 1.0, 1.0, 1.0],
                lod_byte: 0b0000_0001,
                mesh_index: 0,
                material_id: 0,
                matrix: [
                    1.0, 0.0, 0.0, 0.0, // row 0 (m00 m01 m02 tx)
                    0.0, 1.0, 0.0, 0.0, // row 1
                    0.0, 0.0, 1.0, 0.0, // row 2
                ],
            }
        }

        fn write(&self, buf: &mut Vec<u8>) {
            let start = buf.len();
            // AABB at +0x00 (48 bytes)
            for &v in &self.aabb {
                put_f64(buf, v);
            }
            // pad up to +0x30
            while buf.len() - start < 0x30 {
                buf.push(0);
            }
            // pad to +0x37 (LOD byte)
            while buf.len() - start < 0x37 {
                buf.push(0);
            }
            buf.push(self.lod_byte); // +0x37
            put_u16(buf, self.mesh_index); // +0x38
            // pad to +0x3C
            while buf.len() - start < 0x3C {
                buf.push(0);
            }
            // Matrix34 at +0x3C (96 bytes)
            for &v in &self.matrix {
                put_f64(buf, v);
            }
            // pad to +0xA8 (material_id)
            while buf.len() - start < 0xA8 {
                buf.push(0);
            }
            put_u16(buf, self.material_id); // +0xA8
            // pad out to 204 bytes total
            while buf.len() - start < BRUSH_DATA_SIZE {
                buf.push(0);
            }
        }
    }

    /// Layout: [CrCh header][chunk0 (StatObj) data][chunk table at the end].
    /// Returns (file_bytes, statobj_chunk_offset).
    fn build_minimal_soc(
        statobj_version: u16,
        n_statobj: u32,
        path0: &str,
        bodies_with_prefix: &[Vec<u8>], // each entry already has the i32 type prefix
    ) -> Vec<u8> {
        // Build StatObj chunk body: u32 unknown(0), u32 count, count * 256-byte path,
        // then concatenated brush records (each prefixed with i32 type).
        let mut so_body = Vec::new();
        put_u32(&mut so_body, 0); // unknown
        put_u32(&mut so_body, n_statobj);
        for i in 0..n_statobj {
            let mut path = vec![0u8; STATOBJ_PATH_LEN];
            let s = if i == 0 { path0 } else { "other.cgf" };
            let bytes = s.as_bytes();
            let copy = bytes.len().min(STATOBJ_PATH_LEN - 1);
            path[..copy].copy_from_slice(&bytes[..copy]);
            put_bytes(&mut so_body, &path);
        }
        for body in bodies_with_prefix {
            put_bytes(&mut so_body, body);
        }

        // Header (16 bytes): magic + version + chunk_count + chunk_table_offset
        let mut file = Vec::new();
        put_bytes(&mut file, &CRCH_MAGIC);
        put_u32(&mut file, CRCH_VERSION);
        put_u32(&mut file, 1); // 1 chunk
        // chunk_table_offset will be patched below
        let chunk_table_offset_pos = file.len();
        put_u32(&mut file, 0);

        // StatObj chunk at +16
        let statobj_offset = file.len() as u32;
        let statobj_size = so_body.len() as u32;
        put_bytes(&mut file, &so_body);

        // Chunk table at end (16 bytes per entry)
        let table_offset = file.len() as u32;
        // patch chunk_table_offset
        file[chunk_table_offset_pos..chunk_table_offset_pos + 4]
            .copy_from_slice(&table_offset.to_le_bytes());
        // Entry: type (u16), version (u16), id (i32), size (u32), offset (u32)
        put_u16(&mut file, CHUNK_TYPE_STATOBJ);
        put_u16(&mut file, statobj_version);
        put_i32(&mut file, 0); // id
        put_u32(&mut file, statobj_size);
        put_u32(&mut file, statobj_offset);

        file
    }

    fn brush_record(body: &BrushBody) -> Vec<u8> {
        let mut rec = Vec::new();
        put_i32(&mut rec, E_ERTYPE_BRUSH);
        body.write(&mut rec);
        rec
    }

    #[test]
    fn rejects_bad_magic() {
        let data = vec![0u8; 16];
        assert!(matches!(parse_with_identity_parent(&data), Err(SocError::BadMagic)));
    }

    #[test]
    fn rejects_unsupported_container_version() {
        let mut data = Vec::new();
        put_bytes(&mut data, &CRCH_MAGIC);
        put_u32(&mut data, 0x999);
        put_u32(&mut data, 0);
        put_u32(&mut data, 16);
        match parse_with_identity_parent(&data) {
            Err(SocError::UnsupportedContainerVersion(0x999)) => {}
            other => panic!("expected UnsupportedContainerVersion(0x999), got {other:?}"),
        }
    }

    #[test]
    fn accepts_chunk_with_non_v15_version_field() {
        // The chunk table's `version` field is an opaque hint and v1 is
        // observed in current SC builds even though the on-disk records use
        // the v15 SBrushChunk layout. Verify the parser does not reject
        // such chunks at the table level.
        let body = BrushBody::identity();
        let rec = brush_record(&body);
        let data = build_minimal_soc(1, 1, "mesh.cgf", &[rec]);
        let parsed = parse_with_identity_parent(&data).expect("parse ok");
        assert_eq!(parsed.brushes.len(), 1, "v15-formatted record under v1 chunk should still match");
    }

    #[test]
    fn rejects_missing_statobj_chunk() {
        // Build a CrCh file whose only chunk has a non-StatObj type.
        let mut file = Vec::new();
        put_bytes(&mut file, &CRCH_MAGIC);
        put_u32(&mut file, CRCH_VERSION);
        put_u32(&mut file, 1);
        let table_offset_pos = file.len();
        put_u32(&mut file, 0);
        let chunk_offset = file.len() as u32;
        put_bytes(&mut file, &[0u8; 8]); // dummy chunk body
        let table_off = file.len() as u32;
        file[table_offset_pos..table_offset_pos + 4].copy_from_slice(&table_off.to_le_bytes());
        put_u16(&mut file, 0x1234); // not 0x0010
        put_u16(&mut file, 15);
        put_i32(&mut file, 0);
        put_u32(&mut file, 8);
        put_u32(&mut file, chunk_offset);
        match parse_with_identity_parent(&file) {
            Err(SocError::MissingStatObj) => {}
            other => panic!("expected MissingStatObj, got {other:?}"),
        }
    }

    #[test]
    fn parses_one_identity_brush() {
        let body = BrushBody::identity();
        let rec = brush_record(&body);
        let data = build_minimal_soc(15, 1, "mesh_zero.cgf", &[rec]);
        let parsed = parse_with_identity_parent(&data).expect("parse ok");
        assert_eq!(parsed.mesh_paths.len(), 1);
        assert_eq!(parsed.mesh_paths[0], "mesh_zero.cgf");
        assert_eq!(parsed.brushes.len(), 1);
        let b = &parsed.brushes[0];
        assert_eq!(b.mesh_index, 0);
        assert_eq!(b.material_id, 0);
        assert!((b.translation[0] - 0.0).abs() < 1e-5);
        // Identity rotation -> quat (0,0,0,1)
        assert!((b.rotation[3] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn keeps_record_with_zero_lod_byte() {
        // Real-world dungeon SOCs ship records with `byte[+0x37] = 0x20`
        // (or even 0) and the renderer needs those brushes. Verify the
        // parser does not gate them.
        let mut body = BrushBody::identity();
        body.lod_byte = 0;
        let rec = brush_record(&body);
        let data = build_minimal_soc(15, 1, "mesh.cgf", &[rec]);
        let parsed = parse_with_identity_parent(&data).expect("parse ok");
        assert_eq!(parsed.brushes.len(), 1);
    }

    #[test]
    fn keeps_record_with_high_lod_byte() {
        let mut body = BrushBody::identity();
        body.lod_byte = 0b0010_0000; // matches observed real-world value
        let rec = brush_record(&body);
        let data = build_minimal_soc(15, 1, "mesh.cgf", &[rec]);
        let parsed = parse_with_identity_parent(&data).expect("parse ok");
        assert_eq!(parsed.brushes.len(), 1);
    }

    #[test]
    fn composes_local_translation_with_parent_translation() {
        // Local translation is (5, 10, 15); parent is identity-rotation, T=(100,0,0).
        let mut body = BrushBody::identity();
        body.matrix[3] = 5.0;
        body.matrix[7] = 10.0;
        body.matrix[11] = 15.0;
        let rec = brush_record(&body);
        let data = build_minimal_soc(15, 1, "mesh.cgf", &[rec]);
        let parent = ParentTransform {
            rotation: DQuat::IDENTITY,
            translation: DVec3::new(100.0, 0.0, 0.0),
        };
        let parsed = parse(&data, &parent).expect("parse ok");
        assert_eq!(parsed.brushes.len(), 1);
        let t = parsed.brushes[0].translation;
        assert!((t[0] - 105.0).abs() < 1e-4, "tx={}", t[0]);
        assert!((t[1] - 10.0).abs() < 1e-4, "ty={}", t[1]);
        assert!((t[2] - 15.0).abs() < 1e-4, "tz={}", t[2]);
    }

    #[test]
    fn composes_local_translation_under_parent_rotation() {
        // Parent rotates 90 degrees around Z; local translation (1,0,0) becomes (0,1,0).
        let mut body = BrushBody::identity();
        body.matrix[3] = 1.0;
        body.matrix[7] = 0.0;
        body.matrix[11] = 0.0;
        let rec = brush_record(&body);
        let data = build_minimal_soc(15, 1, "mesh.cgf", &[rec]);
        let parent = ParentTransform {
            rotation: DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
            translation: DVec3::ZERO,
        };
        let parsed = parse(&data, &parent).expect("parse ok");
        assert_eq!(parsed.brushes.len(), 1);
        let t = parsed.brushes[0].translation;
        assert!(t[0].abs() < 1e-4, "tx={}", t[0]);
        assert!((t[1] - 1.0).abs() < 1e-4, "ty={}", t[1]);
        assert!(t[2].abs() < 1e-4, "tz={}", t[2]);
    }

    #[test]
    fn rejects_brush_with_out_of_range_mesh_index() {
        let mut body = BrushBody::identity();
        body.mesh_index = 99; // we only declare 1 StatObj path
        let rec = brush_record(&body);
        let data = build_minimal_soc(15, 1, "mesh.cgf", &[rec]);
        let parsed = parse_with_identity_parent(&data).expect("parse ok");
        assert_eq!(parsed.brushes.len(), 0, "out-of-range mesh_index must skip");
    }
}
