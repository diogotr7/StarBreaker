//! Parse skeleton bones from `.chr` files (CompiledBones chunks).
//!
//! Skeleton bones provide attachment points (e.g., `magAttach`) that loadout
//! children reference by name when attaching to CDF weapon entities.
//!
//! Two chunk versions exist:
//! - v900 (`0xC2011111`): per-bone entries with inline transforms (68 bytes each)
//! - v901 (`0xC201973C`): compact entries (16 bytes) with separate transform blocks

use starbreaker_chunks::ChunkFile;
use starbreaker_chunks::known_types::ivo;
use starbreaker_common::reader::SpanReader;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// A parsed skeleton bone with name and world transform.
#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    pub parent_index: Option<u16>,
    pub object_node_index: Option<u16>,
    /// Parent-relative position [x, y, z]
    pub local_position: [f32; 3],
    /// Parent-relative rotation quaternion [w, x, y, z]
    pub local_rotation: [f32; 4],
    /// World-space position [x, y, z]
    pub world_position: [f32; 3],
    /// World-space rotation quaternion [w, x, y, z]
    pub world_rotation: [f32; 4],
}

// ── Binary layouts ──────────────────────────────────────────────────────────

/// On-disk quaternion + translation (28 bytes). Quaternion stored as (x, y, z, w).
#[derive(Clone, Copy, FromBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
struct RawQuatTrans {
    qx: f32,
    qy: f32,
    qz: f32,
    qw: f32,
    tx: f32,
    ty: f32,
    tz: f32,
}

/// v900 per-bone entry (68 bytes): IDs + relative transform + world transform.
#[derive(Clone, Copy, FromBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
struct BoneEntryV900 {
    controller_id: u32,
    limb_id: u32,
    parent_index: i32,
    relative: RawQuatTrans,
    world: RawQuatTrans,
}

/// v901 per-bone entry (16 bytes): compact metadata, transforms stored separately.
#[derive(Clone, Copy, FromBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
struct BoneEntryV901 {
    controller_id: u32,
    limb_id: u16,
    num_children: u16,
    parent_index: i16,
    _unknown1: i16,
    _unknown2: i16,
    object_node_index: i16,
}

// ── Size assertions ─────────────────────────────────────────────────────────

const _: () = {
    assert!(size_of::<RawQuatTrans>() == 28);
    assert!(size_of::<BoneEntryV900>() == 68);
    assert!(size_of::<BoneEntryV901>() == 16);
};

// ── Parsing ─────────────────────────────────────────────────────────────────

impl RawQuatTrans {
    fn rotation_wxyz(&self) -> [f32; 4] {
        [self.qw, self.qx, self.qy, self.qz]
    }

    fn position_xyz(&self) -> [f32; 3] {
        [self.tx, self.ty, self.tz]
    }
}

/// Resolve a list of rig-like node names from a CryEngine asset, trying
/// the CHR `CompiledBones` chunk first and falling back to the CGA/CGAM
/// NMC scene-graph node names.
///
/// CHR files (skeletons used by skinned meshes) expose names through
/// `parse_skeleton`. CGA/CGAM files (rigid scene graphs, e.g. Scorpius main
/// body) have no CompiledBones — their bone-equivalent names live in the
/// NMC chunk as scene-graph node names. Both name sources hash to the same
/// 32-bit space used by `.dba` channel hashes, so either can be used to
/// resolve animation channels by name.
///
/// Returns `None` if neither chunk is present or parseable. Order is
/// preserved from the source (bone order for CHR, node order for NMC) so
/// callers that care about stable output get deterministic results.
pub fn parse_rig_node_names(data: &[u8]) -> Option<Vec<String>> {
    if let Some(bones) = parse_skeleton(data) {
        if !bones.is_empty() {
            return Some(bones.into_iter().map(|b| b.name).collect());
        }
    }
    if let Some((nodes, _)) = crate::nmc::parse_nmc_full(data) {
        if !nodes.is_empty() {
            return Some(nodes.into_iter().map(|n| n.name).collect());
        }
    }
    None
}

/// Parse the CompiledBones chunk from a `.chr` skeleton file.
///
/// Returns `None` if the file can't be parsed or has no CompiledBones chunk.
pub fn parse_skeleton(data: &[u8]) -> Option<Vec<Bone>> {
    let chunk_file = ChunkFile::from_bytes(data).ok()?;
    let ivo_file = match &chunk_file {
        ChunkFile::Ivo(ivo_file) => ivo_file,
        ChunkFile::CrCh(_) => return None,
    };

    for entry in ivo_file.chunks() {
        let chunk = ivo_file.chunk_data(entry);
        if entry.chunk_type == ivo::COMPILED_BONES {
            return parse_compiled_bones_v901(chunk);
        }
        if entry.chunk_type == ivo::COMPILED_BONES_IVO320 {
            return parse_compiled_bones_v900(chunk);
        }
    }

    None
}

/// Parse null-separated bone name strings from a byte slice.
fn parse_bone_names(data: &[u8], expected: usize) -> Option<Vec<String>> {
    let names: Vec<String> = data
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();

    if names.len() < expected {
        log::warn!("CompiledBones: expected {expected} names, got {}", names.len());
        return None;
    }
    Some(names)
}

fn log_bones(bones: &[Bone], version: &str) {
    log::debug!(
        "Parsed {} skeleton bones ({version}): {}",
        bones.len(),
        bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")
    );

    if std::env::var("SB_DEBUG_RIGID_BIND").is_ok() {
        for bone in bones {
            log::debug!(
                "  bone {} parent={:?} object_node_index={:?} local_pos={:?} local_rot={:?}",
                bone.name,
                bone.parent_index,
                bone.object_node_index,
                bone.local_position,
                bone.local_rotation,
            );
        }
    }
}

/// Parse CompiledBones v900 (chunk type `0xC2011111`).
fn parse_compiled_bones_v900(data: &[u8]) -> Option<Vec<Bone>> {
    let mut r = SpanReader::new(data);
    let num_bones = r.read_u32().ok()? as usize;
    if num_bones == 0 || num_bones > 1000 {
        return None;
    }

    let entries = r.read_slice::<BoneEntryV900>(num_bones).ok()?;
    let names = parse_bone_names(r.remaining_bytes(), num_bones)?;

    let bones: Vec<Bone> = entries
        .iter()
        .zip(names)
        .map(|(e, name)| {
            let parent_index = (e.parent_index >= 0).then_some(e.parent_index as u16);
            Bone {
                name,
                parent_index,
                object_node_index: None,
                local_position: e.relative.position_xyz(),
                local_rotation: e.relative.rotation_wxyz(),
                world_position: e.world.position_xyz(),
                world_rotation: e.world.rotation_wxyz(),
            }
        })
        .collect();

    log_bones(&bones, "v900");
    Some(bones)
}

/// Parse CompiledBones v901 (chunk type `0xC201973C`).
fn parse_compiled_bones_v901(data: &[u8]) -> Option<Vec<Bone>> {
    let mut r = SpanReader::new(data);
    let num_bones = r.read_u32().ok()? as usize;
    let string_table_size = r.read_u32().ok()? as usize;
    let _flags1 = r.read_u32().ok()?;
    let _flags2 = r.read_u32().ok()?;
    r.advance(32).ok()?; // padding

    if num_bones == 0 || num_bones > 1000 {
        return None;
    }

    // Skip compact bone entries
    let entries = r.read_slice::<BoneEntryV901>(num_bones).ok()?;

    // Read string table
    let string_bytes = r.read_bytes(string_table_size).ok()?;
    let names = parse_bone_names(string_bytes, num_bones)?;

    let relative_transforms = r.read_slice::<RawQuatTrans>(num_bones).ok()?;
    let world_transforms = r.read_slice::<RawQuatTrans>(num_bones).ok()?;

    let bones: Vec<Bone> = relative_transforms
        .iter()
        .zip(world_transforms.iter())
        .zip(entries.iter().zip(names))
        .map(|((relative, world), (entry, name))| {
            let parent_index = (entry.parent_index >= 0).then_some(entry.parent_index as u16);
            Bone {
                name,
                parent_index,
                object_node_index: (entry.object_node_index >= 0)
                    .then_some(entry.object_node_index as u16),
                local_position: relative.position_xyz(),
                local_rotation: relative.rotation_wxyz(),
                world_position: world.position_xyz(),
                world_rotation: world.rotation_wxyz(),
            }
        })
        .collect();

    log_bones(&bones, "v901");
    Some(bones)
}

impl crate::animation::BoneTransforms for Bone {
    fn name(&self) -> &str {
        &self.name
    }
    fn parent_index(&self) -> Option<usize> {
        self.parent_index.map(|p| p as usize)
    }
    fn local_rotation_wxyz(&self) -> [f32; 4] {
        self.local_rotation
    }
    fn local_position(&self) -> [f32; 3] {
        self.local_position
    }
    fn set_local_rotation_wxyz(&mut self, q: [f32; 4]) {
        self.local_rotation = q;
    }
    fn set_local_position(&mut self, p: [f32; 3]) {
        self.local_position = p;
    }
    fn set_world_rotation_wxyz(&mut self, q: [f32; 4]) {
        self.world_rotation = q;
    }
    fn set_world_position(&mut self, p: [f32; 3]) {
        self.world_position = p;
    }
}
