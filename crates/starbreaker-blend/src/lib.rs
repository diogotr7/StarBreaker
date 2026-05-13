//! `starbreaker-blend` — write Blender 5.x `.blend` library files.
//!
//! Encodes mesh geometry, UV layers, vertex colors, custom normals, vertex groups,
//! empties, lights, parent hierarchies, and IDProperties (custom object properties)
//! into the binary `.blend` format understood by Blender 5.1.x.
//!
//! All layout constants (SDNA indices, struct sizes, field offsets) were extracted
//! from a live Blender 5.1.1 instance and cross-checked against the bundled DNA1
//! block (`dna1_blender501.bin`).

use std::io::Write;

use rustc_hash::FxHashMap;
use zstd::Encoder;

mod idprop;
mod ui_prefix;

pub use idprop::{allocate_idprop_blocks, write_idprop_blocks, IdPropBlocks};
pub use ui_prefix::{startup_ui_prefix_bytes, STARTUP_UI_SCREEN_PTR};

pub const DNA1_BYTES: &[u8] = include_bytes!("dna1_blender501.bin");

/// Blender 5.1.x file magic (17 bytes).
/// Format: BLENDER (7) + 17 (2) + - (1) + 01 (2) + v (1) + 0501 (4) = 17 bytes
pub const BLEND_MAGIC: &[u8] = b"BLENDER17-01v0501";

/// Standard zstd compression level for generated `.blend` files.
///
/// Blender accepts standard zstd frames regardless of level.  Using zstd's
/// default level keeps export time practical for large native `.blend` packages;
/// maximum compression is much slower and only marginally improves package size.
const BLEND_ZSTD_LEVEL: i32 = 3;

// ── SDNA indices (verified against Blender 5.1.1 DNA1 block) ─────────────────
pub const SDNA_IDX_ATTRIBUTE: u32 = 75;
pub const SDNA_IDX_ATTRIBUTE_ARRAY: u32 = 73;
pub const SDNA_IDX_COLLECTION_OBJECT: u32 = 104;
pub const SDNA_IDX_COLLECTION_CHILD: u32 = 105;
pub const SDNA_IDX_COLLECTION: u32 = 107;
pub const SDNA_IDX_FILE_GLOBAL: u32 = 171;
pub const SDNA_IDX_LAYER_COLLECTION: u32 = 247;
pub const SDNA_IDX_BASE: u32 = 246;
pub const SDNA_IDX_VIEW_LAYER: u32 = 252;
pub const SDNA_IDX_MATERIAL: u32 = 321;
pub const SDNA_IDX_MESH: u32 = 322;
pub const SDNA_IDX_BNODE_TREE: u32 = 473;
pub const SDNA_IDX_OBJECT: u32 = 692;
pub const SDNA_IDX_SCENE: u32 = 757;
pub const SDNA_IDX_TOOL_SETTINGS: u32 = 747;
pub const SDNA_IDX_LAMP: u32 = 253;
pub const SDNA_IDX_BDEFORMGROUP: u32 = 686;
pub const SDNA_IDX_MDEFORMVERT: u32 = 331;
pub const SDNA_IDX_MDEFORMWEIGHT: u32 = 330;
pub const SDNA_IDX_CUSTOM_DATA_LAYER: u32 = 160;
pub const SDNA_IDX_IDPROPERTY: u32 = 9;
pub const SDNA_IDX_LIBRARY: u32 = 15;
pub const SDNA_IDX_ID: u32 = 14;
pub const SDNA_IDX_DNA1: u32 = 0;
pub const SDNA_IDX_SCREEN: u32 = 758;  // bScreen struct
pub const SDNA_IDX_WINDOWMANAGER: u32 = 960;  // wmWindowManager struct
pub const SDNA_IDX_WORLD: u32 = 974;
pub const SDNA_IDX_IMAGE: u32 = 242;
pub const SDNA_IDX_IMAGE_TILE: u32 = 241;
pub const SDNA_IDX_BNODE: u32 = 468;
pub const SDNA_IDX_BNODE_SOCKET: u32 = 466;
pub const SDNA_IDX_BNODE_LINK: u32 = 470;
pub const SDNA_IDX_NODE_TEX_IMAGE: u32 = 531;
pub const SDNA_IDX_NODE_TEX_SKY: u32 = 530;
pub const SDNA_IDX_NODE_TEX_ENVIRONMENT: u32 = 534;
pub const SDNA_IDX_BNSV_RGBA: u32 = 479;
pub const SDNA_IDX_BNSV_FLOAT: u32 = 475;
pub const SDNA_IDX_BNSV_VECTOR: u32 = 477;
pub const SDNA_IDX_DISPLACE_MODIFIER: u32 = 362;
pub const SDNA_IDX_WELD_MODIFIER: u32 = 406;
pub const SDNA_IDX_WEIGHTED_NORMAL_MODIFIER: u32 = 413;

// ── Struct sizes (bytes) ──────────────────────────────────────────────────────
pub const FILE_GLOBAL_SIZE: usize = 1216;
pub const SCENE_SIZE: usize = 6920;
pub const TOOL_SETTINGS_SIZE: usize = 1256;
pub const VIEW_LAYER_SIZE: usize = 328;
pub const SCREEN_SIZE: usize = 544;  // bScreen struct (idx=758, verified from dna1_blender501.bin)
pub const WINDOWMANAGER_SIZE: usize = 1480;  // wmWindowManager struct
pub const WORLD_SIZE: usize = 560;
pub const BASE_SIZE: usize = 48;
pub const COLLECTION_SIZE: usize = 520;
pub const COLLECTION_OBJECT_SIZE: usize = 32;
pub const LAYER_COLLECTION_SIZE: usize = 64;
pub const OBJECT_SIZE: usize = 1288;
pub const MESH_SIZE: usize = 1960;
pub const LAMP_SIZE: usize = 568;
pub const IMAGE_SIZE: usize = 1696;
pub const IMAGE_TILE_SIZE: usize = 136;
pub const MATERIAL_SIZE: usize = 584;
pub const BNODE_TREE_SIZE: usize = 736;
pub const BNODE_SIZE: usize = 384;
pub const BNODE_SOCKET_SIZE: usize = 464;
pub const BNODE_LINK_SIZE: usize = 56;
pub const NODE_TEX_IMAGE_SIZE: usize = 1024;
pub const NODE_TEX_SKY_SIZE: usize = 1024;
pub const NODE_TEX_ENVIRONMENT_SIZE: usize = 1016;
pub const BNSV_RGBA_SIZE: usize = 16;
pub const BNSV_FLOAT_SIZE: usize = 16;
pub const BNSV_VECTOR_SIZE: usize = 32;
pub const ATTRIBUTE_SIZE: usize = 24;
pub const ATTRIBUTE_ARRAY_SIZE: usize = 32;
pub const CURVE_MAP_POINT_SIZE: usize = 12;
pub const BDEFORMGROUP_SIZE: usize = 88;
pub const MDEFORMVERT_SIZE: usize = 16;
pub const MDEFORMWEIGHT_SIZE: usize = 8;
pub const CUSTOM_DATA_LAYER_SIZE: usize = 112;
pub const IDPROPERTY_SIZE: usize = 144;
pub const LIBRARY_SIZE: usize = 1472;  // ID (408) + filepath[1024] + flag (2) + undo_runtime_tag (2) + _pad (4) + archive_parent_library (8) + packedfile (8) + runtime (8) + _pad2 (8)
pub const ID_STUB_SIZE: usize = 408;
pub const DISPLACE_MODIFIER_SIZE: usize = 368;
pub const WELD_MODIFIER_SIZE: usize = 192;
pub const WEIGHTED_NORMAL_MODIFIER_SIZE: usize = 192;
/// `eModifierType_Displace` = 14, `eModifierType_WeightedNormal` = 54, `eModifierType_Weld` = 55.
pub const MODIFIER_TYPE_DISPLACE: i32 = 14;
pub const MODIFIER_TYPE_WELD: i32 = 55;
pub const MODIFIER_TYPE_WEIGHTED_NORMAL: i32 = 54;
/// `mode` field bitmask: `eModifierMode_Realtime` (1) | `eModifierMode_Render` (2).
pub const MODIFIER_MODE_DEFAULT: i32 = 3;
/// `DisplaceModifierData.direction`: displace along vertex normals.
pub const DISPLACE_DIRECTION_NORMAL: i32 = 3;
/// `DisplaceModifierData.space`: local coordinates.
pub const DISPLACE_SPACE_LOCAL: i32 = 0;

const NODE_TEX_SKY_REFERENCE_HEX: &str = concat!(
    "0000000000000000000000000000000000000000000000000000803f0000803f0000803f0000000001020300000000000000803f000000000000000000000000",
    "000000000000803f000000000000000000000000000000000000803f000000000000000000000000000000000000803f0000000000000000000000000000803f",
    "0000803f0000803f000000000000000002000000000000000000000000000000000000000000803f00000000000000000000803f0000803f0000803f0000803f",
    "0000803f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f",
    "0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f00000000",
    "0000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f",
    "0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f",
    "0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f00000000",
    "0000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f",
    "0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f",
    "0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f00000000",
    "0000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f",
    "0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f",
    "0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f000000000000003f0000003f0000003f0000803f0000003f00000000",
    "0000003f0000003f0000003f0000803f0000003f000000000000803f0000803f0000803f00000000cdcc4c3fcdcc4c3fcdcc4c3f000000000000000000000000",
    "0300000000000000000000000000803fcdcc0c409a99993e68d81b3c0000803f920a863edb0fc93f0000c8420000803f0000803f0000803f0100000000000000",
);

// ── Attribute enums ───────────────────────────────────────────────────────────
pub const ATTR_DOMAIN_POINT: u8 = 0;
pub const ATTR_DOMAIN_EDGE: u8 = 1;
pub const ATTR_DOMAIN_FACE: u8 = 2;
pub const ATTR_DOMAIN_CORNER: u8 = 3;
pub const ATTR_TYPE_INT16_2D: i16 = 2;
pub const ATTR_TYPE_INT: i16 = 3;
pub const ATTR_TYPE_INT32_2D: i16 = 4;
pub const ATTR_TYPE_FLOAT2: i16 = 6;
pub const ATTR_TYPE_FLOAT3: i16 = 7;
pub const ATTR_TYPE_BYTE_COLOR: i16 = 9;
pub const ATTR_STORAGE_ARRAY: u8 = 0;

// ── IDProperty type codes ─────────────────────────────────────────────────────
pub const IDP_STRING: u8 = 0;
pub const IDP_INT: u8 = 1;
pub const IDP_GROUP: u8 = 6;
pub const IDP_DOUBLE: u8 = 8;

// ── Pointer allocator ─────────────────────────────────────────────────────────

/// Sequential fake-pointer allocator (file-scope; not related to real addresses).
/// Each call returns the next address and advances by 0x10.
#[derive(Clone, Copy, Debug)]
pub struct PtrAlloc {
    next: u64,
}

impl PtrAlloc {
    pub fn new(start: u64) -> Self {
        Self { next: start }
    }

    pub fn alloc(&mut self) -> u64 {
        let p = self.next;
        self.next += 0x10;
        p
    }
}

// ── Low-level serialisation helpers ──────────────────────────────────────────

/// Write a 32-byte block header for Blender 5.x (17-byte magic files).
///
/// Layout: `code[4] + sdna_idx[u32] + old_ptr[u64] + data_len[u32] + zero[u32] + count[u32] + zero[u32]`
pub fn write_block_header(
    out: &mut Vec<u8>,
    code: &[u8; 4],
    sdna_idx: u32,
    old_ptr: u64,
    data_len: u32,
    count: u32,
) {
    out.extend_from_slice(code);
    out.extend_from_slice(&sdna_idx.to_le_bytes());
    out.extend_from_slice(&old_ptr.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

/// Write a complete block (header + data).
pub fn write_block(
    out: &mut Vec<u8>,
    code: &[u8; 4],
    sdna_idx: u32,
    old_ptr: u64,
    count: u32,
    data: &[u8],
) {
    write_block_header(out, code, sdna_idx, old_ptr, data.len() as u32, count);
    out.extend_from_slice(data);
}

/// Compress raw `.blend` bytes using Blender 5.x Zstandard (Zstd) format.
///
/// Blender 5.x requires Zstd compression (not gzip). If compression fails,
/// returns the original input bytes so callers still get a valid uncompressed
/// `.blend` payload.
///
/// Magic bytes: 0x28 0xB5 0x2F 0xFD (Zstd frame header)
pub fn compress_blend_bytes(raw_blend: &[u8]) -> Vec<u8> {
    match Encoder::new(Vec::new(), BLEND_ZSTD_LEVEL) {
        Ok(mut encoder) => {
            if encoder.write_all(raw_blend).is_err() {
                return raw_blend.to_vec();
            }
            match encoder.finish() {
                Ok(compressed) => compressed,
                Err(_) => raw_blend.to_vec(),
            }
        }
        Err(_) => raw_blend.to_vec(),
    }
}

/// Return uncompressed `.blend` bytes.
///
/// Generated StarBreaker `.blend` files are stored as standard Blender 5.x zstd
/// frames, but tests and some tools may pass raw `.blend` bytes directly.
pub fn decompress_blend_bytes_if_needed(bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    if bytes.starts_with(BLEND_MAGIC) {
        return Ok(bytes.to_vec());
    }
    if bytes.starts_with(&ZSTD_MAGIC) {
        return zstd::stream::decode_all(bytes);
    }
    Ok(bytes.to_vec())
}

// ── Byte-level field writers ──────────────────────────────────────────────────

pub fn write_ptr(buf: &mut [u8], off: usize, ptr: u64) {
    buf[off..off + 8].copy_from_slice(&ptr.to_le_bytes());
}

/// Write a 4×4 identity matrix (row-major, 16 × f32 = 64 bytes).
pub fn write_identity_matrix4x4(buf: &mut [u8], off: usize) {
    let id: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    for (i, &v) in id.iter().enumerate() {
        buf[off + i * 4..off + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

pub fn write_i32(buf: &mut [u8], off: usize, v: i32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn write_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn write_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn write_i16(buf: &mut [u8], off: usize, v: i16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn write_f32(buf: &mut [u8], off: usize, v: f32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn write_cstr_fixed(buf: &mut [u8], off: usize, max_len: usize, text: &str) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(max_len.saturating_sub(1));
    buf[off..off + n].copy_from_slice(&bytes[..n]);
    buf[off + n] = 0;
}

/// Write an ID name with a two-character prefix (e.g. `"OB"`, `"ME"`, `"LA"`).
/// The result is null-terminated and fits in `ID.name[258]` at offset 40.
pub fn write_id_name(buf: &mut [u8], id_prefix: &str, name: &str) {
    let full = format!("{}{}", id_prefix, name);
    write_cstr_fixed(buf, 40, 258, &full);
}

// ── Data-array helpers ────────────────────────────────────────────────────────

pub fn floats3_data(values: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 12);
    for v in values {
        out.extend_from_slice(&v[0].to_le_bytes());
        out.extend_from_slice(&v[1].to_le_bytes());
        out.extend_from_slice(&v[2].to_le_bytes());
    }
    out
}

pub fn floats2_data(values: &[[f32; 2]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for v in values {
        out.extend_from_slice(&v[0].to_le_bytes());
        out.extend_from_slice(&v[1].to_le_bytes());
    }
    out
}

pub fn ints_data(values: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn ints2_data(values: &[[i32; 2]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for v in values {
        out.extend_from_slice(&v[0].to_le_bytes());
        out.extend_from_slice(&v[1].to_le_bytes());
    }
    out
}

/// Build Blender 5.x generic edge topology from triangle corner vertices.
///
/// Blender's generic mesh topology requires `.edge_verts` on the EDGE domain
/// and `.corner_edge` alongside `.corner_vert` on the CORNER domain.
pub fn triangle_edge_topology(indices: &[u32]) -> (Vec<[i32; 2]>, Vec<i32>) {
    let mut edge_map: FxHashMap<(u32, u32), i32> = FxHashMap::with_capacity_and_hasher(
        indices.len() / 2,
        Default::default(),
    );
    let mut edge_verts = Vec::with_capacity(indices.len() / 2);
    let mut corner_edges = Vec::with_capacity(indices.len());

    for tri in indices.chunks_exact(3) {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let key = if a <= b { (a, b) } else { (b, a) };
            let edge_index = match edge_map.get(&key) {
                Some(&index) => index,
                None => {
                    let index = edge_verts.len() as i32;
                    edge_verts.push([key.0 as i32, key.1 as i32]);
                    edge_map.insert(key, index);
                    index
                }
            };
            corner_edges.push(edge_index);
        }
    }

    (edge_verts, corner_edges)
}

pub fn bytes4_data(values: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(v);
    }
    out
}

// ── Datablock builders ────────────────────────────────────────────────────────

pub fn build_file_global(screen_ptr: u64, scene_ptr: u64, view_layer_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; FILE_GLOBAL_SIZE];
    data[0..4].copy_from_slice(b"5.01");
    write_u16(&mut data, 4, 1);
    write_u16(&mut data, 6, 500);
    write_ptr(&mut data, 16, screen_ptr);  // curscreen @ offset 16
    write_ptr(&mut data, 24, scene_ptr);   // curscene @ offset 24
    write_ptr(&mut data, 32, view_layer_ptr);  // cur_view_layer @ offset 32
    data
}

pub fn build_screen(screen_name: &str) -> Vec<u8> {
    let mut data = vec![0u8; SCREEN_SIZE];
    // bScreen starts with ID struct (408 bytes)
    write_id_name(&mut data, "SR", screen_name);
    // ListBases for vertbase, edgebase, areabase, regionbase are all NULL (0x0000...)
    // Scene *scene pointer (offset 440) = NULL
    // All flags and state vars default to 0
    data
}

pub fn build_windowmanager(wm_name: &str) -> Vec<u8> {
    let mut data = vec![0u8; WINDOWMANAGER_SIZE];
    // wmWindowManager starts with ID struct (408 bytes)
    write_id_name(&mut data, "WM", wm_name);
    // All other fields default to NULL/0 (windows, layouts, operators, notifier queues, etc.)
    data
}

pub fn build_scene(
    scene_name: &str,
    view_layer_ptr: u64,
    master_collection_ptr: u64,
    tool_settings_ptr: u64,
) -> Vec<u8> {
    build_scene_with_motion_blur_curve(scene_name, view_layer_ptr, master_collection_ptr, tool_settings_ptr, 0)
}

pub fn build_scene_with_motion_blur_curve(
    scene_name: &str,
    view_layer_ptr: u64,
    master_collection_ptr: u64,
    tool_settings_ptr: u64,
    motion_blur_curve_points_ptr: u64,
) -> Vec<u8> {
    build_scene_with_motion_blur_curve_and_properties(
        scene_name,
        view_layer_ptr,
        master_collection_ptr,
        tool_settings_ptr,
        motion_blur_curve_points_ptr,
        0,
        0,
        "BLENDER_WORKBENCH",
    )
}

pub fn build_scene_with_motion_blur_curve_and_properties(
    scene_name: &str,
    view_layer_ptr: u64,
    master_collection_ptr: u64,
    tool_settings_ptr: u64,
    motion_blur_curve_points_ptr: u64,
    system_properties_ptr: u64,
    world_ptr: u64,
    render_engine: &str,
) -> Vec<u8> {
    let mut data = vec![0u8; SCENE_SIZE];
    write_id_name(&mut data, "SC", scene_name);
    // Minimal Blender 5.1 Scene defaults mirrored from DNA_scene_types.h / scene_init_data().
    // Leaving RenderData fully zeroed makes Blender's post-load code hit invalid time/render
    // state before Python can inspect the file.
    data[322] = 32; // scene preview image planes = R_IMF_PLANES_RGBA
    data[609] = 17; // r.im_format.imtype = R_IMF_IMTYPE_PNG
    data[610] = 2; // r.im_format.depth = R_IMF_CHAN_DEPTH_8
    data[611] = 32; // r.im_format.planes = R_IMF_PLANES_RGBA
    data[613] = 90; // r.im_format.quality
    data[614] = 15; // r.im_format.compress
    write_i32(&mut data, 1032, 1); // r.cfra
    write_i32(&mut data, 1036, 1); // r.sfra
    write_i32(&mut data, 1040, 250); // r.efra
    write_i32(&mut data, 1056, 100); // r.images
    write_i32(&mut data, 1060, 100); // r.framapto
    write_u16(&mut data, 1066, 1); // r.threads
    write_f32(&mut data, 1068, 1.0); // r.framelen
    write_i32(&mut data, 1072, 1); // r.frame_step
    write_u16(&mut data, 1078, 100); // r.size
    write_i32(&mut data, 1080, 1920); // r.xsch
    write_i32(&mut data, 1084, 1080); // r.ysch
    write_i32(&mut data, 1108, 0x0001 | 0x0004 | 0x0010); // r.scemode = R_DOCOMP | R_DOSEQ | R_EXTENSION
    write_u16(&mut data, 1116, 24); // r.frs_sec
    write_f32(&mut data, 1120, 0.0); // r.border.xmin
    write_f32(&mut data, 1124, 1.0); // r.border.xmax
    write_f32(&mut data, 1128, 0.0); // r.border.ymin
    write_f32(&mut data, 1132, 1.0); // r.border.ymax
    write_f32(&mut data, 1156, 1.0); // r.xasp
    write_f32(&mut data, 1160, 1.0); // r.yasp
    write_f32(&mut data, 1164, 72.0); // r.ppm_factor
    write_f32(&mut data, 1168, 0.0254); // r.ppm_base
    write_f32(&mut data, 1172, 1.0); // r.frs_sec_base
    write_f32(&mut data, 1176, 1.5); // r.gauss
    write_i32(&mut data, 1180, 1); // r.color_mgt_flag = R_COLOR_MANAGEMENT
    write_f32(&mut data, 1184, 1.0); // r.dither_intensity
    write_cstr_fixed(&mut data, 1196, 1024, "//"); // r.pic
    write_i32(&mut data, 2224, 0x00ff); // r.stamp defaults
    write_u16(&mut data, 2228, 12); // r.stamp_font_id
    for i in 0..4 {
        write_f32(&mut data, 3000 + i * 4, [0.8, 0.8, 0.8, 1.0][i]); // r.fg_stamp
        write_f32(&mut data, 3016 + i * 4, [0.0, 0.0, 0.0, 0.25][i]); // r.bg_stamp
    }
    write_f32(&mut data, 3040, 1.0); // r.simplify_particles
    write_f32(&mut data, 3048, 1.0); // r.simplify_volumes
    data[3028] = 3; // r.seq_prev_type = OB_SOLID
    write_i32(&mut data, 3052, 1); // r.line_thickness_mode = R_LINE_THICKNESS_ABSOLUTE
    write_f32(&mut data, 3056, 1.0); // r.unit_line_thickness
    write_cstr_fixed(&mut data, 3060, 32, render_engine); // r.engine
    write_f32(&mut data, 4544, 0.5); // r.motion_blur_shutter
    // Blender initializes RenderData.mblur_shutter_curve for every new scene.
    // Cycles reads this curve unconditionally when building the camera shutter
    // table, so generated scenes must carry the inline CurveMapping defaults
    // and a valid cm[0].curve pointer.
    write_i32(&mut data, 4552, 17); // r.mblur_shutter_curve.flag
    write_f32(&mut data, 4568, 0.0); // curr.xmin
    write_f32(&mut data, 4572, 1.0); // curr.xmax
    write_f32(&mut data, 4576, 0.0); // curr.ymin
    write_f32(&mut data, 4580, 1.0); // curr.ymax
    write_f32(&mut data, 4584, 0.0); // clipr.xmin
    write_f32(&mut data, 4588, 1.0); // clipr.xmax
    write_f32(&mut data, 4592, 0.0); // clipr.ymin
    write_f32(&mut data, 4596, 1.0); // clipr.ymax
    write_i16(&mut data, 4600, 3); // cm[0].totpoint
    write_i16(&mut data, 4602, 1); // cm[0].flag
    write_f32(&mut data, 4604, 256.0); // cm[0].range
    write_f32(&mut data, 4608, 0.0); // cm[0].mintable
    write_f32(&mut data, 4612, 1.0); // cm[0].maxtable
    for offset in [4616, 4620, 4624, 4628] {
        write_f32(&mut data, offset, -std::f32::consts::FRAC_1_SQRT_2);
    }
    write_ptr(&mut data, 4632, motion_blur_curve_points_ptr); // cm[0].curve
    write_f32(&mut data, 4996, 1.0); // r.time_jump_delta
    write_i32(&mut data, 5000, 1); // r.time_jump_unit
    write_i32(&mut data, 5008, 48000); // audio.mixrate
    write_f32(&mut data, 5012, 1.0); // audio.main
    write_f32(&mut data, 5016, 343.3); // audio.speed_of_sound
    write_f32(&mut data, 5020, 1.0); // audio.doppler_factor
    write_i32(&mut data, 5024, 2); // audio.distance_model
    write_u16(&mut data, 5028, 1); // audio.flag = AUDIO_SYNC
    write_f32(&mut data, 5032, 1.0); // audio.volume
    write_i32(&mut data, 5072, 1); // orientation_slots[0].type
    write_i32(&mut data, 5076, -1); // orientation_slots[0].index_custom
    for off in [5092, 5108, 5124] {
        write_i32(&mut data, off, -1); // orientation_slots[1..].index_custom
    }
    write_f32(&mut data, 5176, 1.0); // unit.scale_length
    data[5180] = 1; // unit.system = USER_UNIT_METRIC
    data[5184] = 3; // unit.length_unit = METERS
    data[5185] = 1; // unit.mass_unit
    data[5186] = 1; // unit.time_unit
    data[5187] = 1; // unit.temperature_unit
    write_i32(&mut data, 5304, 1); // physics_settings.flag = PHYS_GLOBAL_GRAVITY
    write_cstr_fixed(&mut data, 5320, 64, "None"); // view_settings.look
    write_cstr_fixed(&mut data, 5384, 64, "AgX"); // view_settings.view_transform
    write_f32(&mut data, 5448, 0.0); // view_settings.exposure
    write_f32(&mut data, 5452, 1.0); // view_settings.gamma
    write_cstr_fixed(&mut data, 5480, 64, "sRGB"); // display_settings.display_device
    write_cstr_fixed(&mut data, 5552, 64, "sRGB"); // sequencer_colorspace_settings.name
    write_f32(&mut data, 5672, 0.57735026); // display.light_direction[0]
    write_f32(&mut data, 5676, 0.57735026); // display.light_direction[1]
    write_f32(&mut data, 5680, 0.57735026); // display.light_direction[2]
    write_f32(&mut data, 5684, 0.1); // display.shadow_shift
    write_f32(&mut data, 5692, 0.2); // display.matcap_ssao_distance
    write_f32(&mut data, 5696, 1.0); // display.matcap_ssao_attenuation
    write_i32(&mut data, 5700, 16); // display.matcap_ssao_samples
    data[5704] = 1; // display.viewport_aa = FXAA
    data[5705] = 8; // display.render_aa = 8 samples
    data[5712] = 3; // display.shading.type = OB_SOLID
    data[5713] = 3; // display.shading.prev_type = OB_SOLID
    write_u16(&mut data, 5716, (1 << 1) | (1 << 2) | (1 << 12) | (1 << 13)); // View3DShading.flag defaults
    data[5718] = 1; // display.shading.light = V3D_LIGHTING_STUDIO
    data[5720] = 1; // display.shading.cavity_type = V3D_SHADING_CAVITY_CURVATURE
    data[5721] = 2; // display.shading.wire_color_type = V3D_SHADING_SINGLE_COLOR
    write_f32(&mut data, 5970, 0.5); // display.shading.shadow_intensity
    for i in 0..3 {
        write_f32(&mut data, 5974 + i * 4, 0.8); // display.shading.single_color
        write_f32(&mut data, 5986 + i * 4, 0.0); // display.shading.object_outline_color
        write_f32(&mut data, 6014 + i * 4, 0.05); // display.shading.background_color
    }
    write_f32(&mut data, 5998, 0.5); // display.shading.xray_alpha
    write_f32(&mut data, 6002, 0.5); // display.shading.xray_alpha_wire
    write_f32(&mut data, 6006, 1.0); // display.shading.cavity_valley_factor
    write_f32(&mut data, 6010, 1.0); // display.shading.cavity_ridge_factor
    write_f32(&mut data, 6026, 1.0); // display.shading.curvature_ridge_factor
    write_f32(&mut data, 6030, 1.0); // display.shading.curvature_valley_factor
    write_i32(&mut data, 6034, 1); // display.shading.render_pass = SCE_PASS_COMBINED
    write_i32(&mut data, 6656, (1 << 11) | (1 << 24)); // eevee.flag
    write_i32(&mut data, 6660, 3); // eevee.gi_diffuse_bounces
    write_i32(&mut data, 6664, 512); // eevee.gi_cubemap_resolution
    write_i32(&mut data, 6668, 32); // eevee.gi_visibility_resolution
    write_i32(&mut data, 6676, 16); // eevee.gi_irradiance_pool_size
    write_i32(&mut data, 6684, 16); // eevee.taa_samples
    write_i32(&mut data, 6688, 64); // eevee.taa_render_samples
    write_f32(&mut data, 6692, 0.1); // eevee.volumetric_start
    write_f32(&mut data, 6696, 100.0); // eevee.volumetric_end
    write_i32(&mut data, 6700, 8); // eevee.volumetric_tile_size
    write_i32(&mut data, 6704, 64); // eevee.volumetric_samples
    write_f32(&mut data, 6708, 0.8); // eevee.volumetric_sample_distribution
    write_i32(&mut data, 6716, 16); // eevee.volumetric_shadow_samples
    write_i32(&mut data, 6720, 16); // eevee.volumetric_ray_depth
    write_f32(&mut data, 6732, 0.05); // eevee.fast_gi_bias
    write_i32(&mut data, 6736, 2); // eevee.fast_gi_resolution
    write_i32(&mut data, 6740, 8); // eevee.fast_gi_step_count
    write_i32(&mut data, 6744, 2); // eevee.fast_gi_ray_count
    write_f32(&mut data, 6748, 0.25); // eevee.fast_gi_quality
    write_f32(&mut data, 6756, 0.25); // eevee.fast_gi_thickness_near
    write_f32(&mut data, 6760, std::f32::consts::FRAC_PI_4); // eevee.fast_gi_thickness_far
    write_f32(&mut data, 6768, 5.0); // eevee.bokeh_overblur
    write_f32(&mut data, 6772, 100.0); // eevee.bokeh_max_size
    write_f32(&mut data, 6776, 1.0); // eevee.bokeh_threshold
    write_f32(&mut data, 6780, 10.0); // eevee.bokeh_neighbor_max
    write_i32(&mut data, 6788, 32); // eevee.motion_blur_max
    write_i32(&mut data, 6792, 1); // eevee.motion_blur_steps
    write_f32(&mut data, 6804, 100.0); // eevee.motion_blur_depth_scale
    write_i32(&mut data, 6812, 512); // eevee.shadow_pool_size
    write_i32(&mut data, 6816, 1); // eevee.shadow_ray_count
    write_i32(&mut data, 6820, 6); // eevee.shadow_step_count
    write_f32(&mut data, 6824, 1.0); // eevee.shadow_resolution_scale
    write_f32(&mut data, 6836, 10.0); // eevee.clamp_surface_indirect
    write_f32(&mut data, 6844, 1.0); // eevee.direct_light_intensity
    write_f32(&mut data, 6848, 1.0); // eevee.indirect_light_intensity
    write_i32(&mut data, 6852, 1); // eevee.ray_tracing_method
    write_f32(&mut data, 6856, 0.25); // eevee.ray_tracing_options.screen_trace_quality
    write_f32(&mut data, 6860, 0.2); // eevee.ray_tracing_options.screen_trace_thickness
    write_f32(&mut data, 6864, 0.5); // eevee.ray_tracing_options.trace_max_roughness
    write_i32(&mut data, 6868, 2); // eevee.ray_tracing_options.resolution_scale
    write_i32(&mut data, 6872, 1); // eevee.ray_tracing_options.flag
    write_i32(&mut data, 6876, 1 | 2 | 4); // eevee.ray_tracing_options.denoise_stages
    write_f32(&mut data, 6880, 3.0); // eevee.overscan
    write_f32(&mut data, 6884, 0.01); // eevee.light_threshold
    write_ptr(&mut data, 5632, view_layer_ptr);
    write_ptr(&mut data, 5640, view_layer_ptr);
    write_ptr(&mut data, 5648, master_collection_ptr);
    write_ptr(&mut data, 568, tool_settings_ptr);
    write_ptr(&mut data, 352, system_properties_ptr);
    write_ptr(&mut data, 424, world_ptr);
    write_i32(&mut data, 5664, 1);
    write_i32(&mut data, 5668, 250);
    data
}

pub fn build_world(world_name: &str) -> Vec<u8> {
    build_world_with_node_tree(world_name, 0)
}

pub fn build_world_with_node_tree(world_name: &str, node_tree_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; WORLD_SIZE];
    write_id_name(&mut data, "WO", world_name);
    write_f32(&mut data, 424, 0.050876088); // horr
    write_f32(&mut data, 428, 0.050876088); // horg
    write_f32(&mut data, 432, 0.050876088); // horb
    write_i16(&mut data, 450, 1 << 4); // flag = WO_USE_SUN_SHADOW
    write_f32(&mut data, 456, 5.0); // miststa
    write_f32(&mut data, 460, 25.0); // mistdist
    write_f32(&mut data, 468, 10.0); // aodist
    write_f32(&mut data, 472, 1.0); // aoenergy
    write_i32(&mut data, 476, 10); // probe_resolution = LIGHT_PROBE_RESOLUTION_1024
    write_f32(&mut data, 480, 10.0); // sun_threshold
    write_f32(&mut data, 484, 0.009180225); // sun_angle = radians(0.526)
    write_f32(&mut data, 488, 0.001); // sun_shadow_maximum_resolution
    write_f32(&mut data, 492, 10.0); // sun_shadow_jitter_overblur
    write_f32(&mut data, 496, 1.0); // sun_shadow_filter_radius
    write_ptr(&mut data, 512, node_tree_ptr); // nodetree
    data
}

fn reference_sky_texture_storage() -> Vec<u8> {
    let mut data = Vec::with_capacity(NODE_TEX_SKY_SIZE);
    for pair in NODE_TEX_SKY_REFERENCE_HEX.as_bytes().chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16).expect("valid sky storage hex");
        let lo = (pair[1] as char).to_digit(16).expect("valid sky storage hex");
        data.push(((hi << 4) | lo) as u8);
    }
    debug_assert_eq!(data.len(), NODE_TEX_SKY_SIZE);
    data
}

pub fn build_motion_blur_shutter_curve_points() -> Vec<u8> {
    let mut data = vec![0u8; CURVE_MAP_POINT_SIZE * 3];
    for (idx, (x, y)) in [(0.0, 1.0), (0.5, 1.0), (1.0, 1.0)].into_iter().enumerate() {
        let off = idx * CURVE_MAP_POINT_SIZE;
        write_f32(&mut data, off, x);
        write_f32(&mut data, off + 4, y);
    }
    data
}

pub fn build_cycles_render_settings_system_properties(
    _root_ptr: u64,
    cycles_group_ptr: u64,
    child_ptrs: &[u64; 6],
) -> (Vec<u8>, Vec<u8>, Vec<(u64, Vec<u8>)>) {
    let root = build_idproperty(
        IDP_GROUP, "", 0, 0, 0, cycles_group_ptr, cycles_group_ptr, 0, 0.0, 1, 1,
    );
    let cycles_group = build_idproperty(
        IDP_GROUP,
        "cycles",
        0,
        0,
        0,
        child_ptrs[0],
        child_ptrs[5],
        0,
        0.0,
        child_ptrs.len() as i32,
        child_ptrs.len() as i32,
    );
    let values = [
        ("sampling_pattern", 1),
        ("device", 1),
        ("preview_samples", 64),
        ("use_preview_denoising", 1),
        ("samples", 512),
        ("use_denoising", 1),
    ];
    let children = values
        .iter()
        .enumerate()
        .map(|(idx, (name, value))| {
            let next_ptr = if idx + 1 < child_ptrs.len() { child_ptrs[idx + 1] } else { 0 };
            let prev_ptr = if idx > 0 { child_ptrs[idx - 1] } else { 0 };
            let mut block = build_idproperty(
                IDP_INT, name, next_ptr, prev_ptr, 0, 0, 0, *value, 0.0, 0, 0,
            );
            block[17] = 0;
            (child_ptrs[idx], block)
        })
        .collect();
    (root, cycles_group, children)
}

pub fn build_tool_settings() -> Vec<u8> {
    vec![0u8; TOOL_SETTINGS_SIZE]
}

pub fn build_view_layer(
    view_layer_name: &str,
    base_ptr: u64,
    layer_collection_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; VIEW_LAYER_SIZE];
    // ViewLayer struct starts with: next (8) + prev (8) + name[64] (64)
    // NO ID header - ViewLayer is not an ID type!
    let name_bytes = view_layer_name.as_bytes();
    let copy_len = name_bytes.len().min(63);
    data[16..16 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
    data[16 + copy_len] = 0; // null terminator

    write_u16(&mut data, 80, 5);
    write_ptr(&mut data, 88, base_ptr);
    write_ptr(&mut data, 96, base_ptr);
    write_ptr(&mut data, 120, layer_collection_ptr);
    write_ptr(&mut data, 128, layer_collection_ptr);
    write_ptr(&mut data, 136, layer_collection_ptr);
    // Match Blender's renderable ViewLayer defaults: combined/sky/solid/volume
    // passes must be enabled or Cycles/EEVEE render an all-black Combined pass.
    write_i32(&mut data, 144, 0x7fff);
    write_i32(&mut data, 148, 1);
    write_f32(&mut data, 152, 0.5);
    write_i16(&mut data, 156, 8);
    write_i16(&mut data, 158, 6);
    data
}

pub fn build_base(object_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; BASE_SIZE];
    write_ptr(&mut data, 16, object_ptr);
    write_u16(&mut data, 36, 0x00d6);
    write_u16(&mut data, 38, 0x00d6);
    data
}

pub fn build_collection(
    collection_name: &str,
    collection_object_head_ptr: u64,
    collection_object_tail_ptr: u64,
    children_head_ptr: u64,
    children_tail_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_SIZE];
    write_id_name(&mut data, "GR", collection_name);
    // id.flag = 0, owner_id at offset 408 = 0 (overwritten at runtime by Blender)
    
    // Offset 416-423: Collection.gobject ListBase.first (CollectionObject* head)
    write_ptr(&mut data, 416, collection_object_head_ptr);
    
    // Offset 424-431: Collection.gobject ListBase.last (CollectionObject* tail)
    write_ptr(&mut data, 424, collection_object_tail_ptr);
    
    // Offset 432-439: Collection.children ListBase.first (CollectionChild* head)
    write_ptr(&mut data, 432, children_head_ptr);
    
    // Offset 440-447: Collection.children ListBase.last (CollectionChild* tail)
    write_ptr(&mut data, 440, children_tail_ptr);
    
    data
}

/// Build an embedded master Collection (Scene Collection) DATA block.
///
/// The master_collection is NOT a top-level GR block — it is an embedded DATA block
/// immediately following the SC block. Blender reads it via `BLO_read_struct` (datamap),
/// not `newlibadr` (libmap). Required flags:
/// - `ID_FLAG_EMBEDDED_DATA` (0x0400) at id.flag (offset 298)
/// - `COLLECTION_IS_MASTER` (0x20) at collection.flag (offset 496)
pub fn build_master_collection(
    gobject_head_ptr: u64,
    gobject_tail_ptr: u64,
    children_head_ptr: u64,
    children_tail_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_SIZE];
    write_id_name(&mut data, "GR", "Scene Collection");
    // id.flag |= ID_FLAG_EMBEDDED_DATA (0x0400) at offset 298 — Blender asserts this
    write_i16(&mut data, 298, 0x0400);
    // collection.flag |= COLLECTION_IS_MASTER (0x20) at offset 496 — Blender asserts this
    data[496] = 0x20;
    // owner_id at offset 408 = 0 (overwritten at runtime: BKE_collection_blend_read_data sets it)

    // gobject: direct objects in the master collection
    write_ptr(&mut data, 416, gobject_head_ptr);
    write_ptr(&mut data, 424, gobject_tail_ptr);

    // children: CollectionChild linked list pointing to sub-collections
    write_ptr(&mut data, 432, children_head_ptr);
    write_ptr(&mut data, 440, children_tail_ptr);

    data
}

pub fn build_collection_object(object_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_OBJECT_SIZE];
    write_ptr(&mut data, 16, object_ptr);
    data
}

/// Build a CollectionObject with doubly-linked list pointers.
/// 
/// CollectionObject (DNA idx=104, size=32):
/// - Offset  0: next pointer (u64) — Blender's BLO_read_struct_list follows this
/// - Offset  8: prev pointer (u64) — overwritten by Blender during list traversal
/// - Offset 16: object pointer (u64)
pub fn build_collection_object_linked(
    object_ptr: u64,
    prev_ptr: u64,
    next_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; COLLECTION_OBJECT_SIZE];
    write_ptr(&mut data, 0, next_ptr);   // next at offset 0 (DNA: CollectionObject.next)
    write_ptr(&mut data, 8, prev_ptr);   // prev at offset 8 (DNA: CollectionObject.prev)
    write_ptr(&mut data, 16, object_ptr);
    data
}

/// Build a LayerCollection with proper doubly-linked list support.
///
/// LayerCollection struct (DNA idx=247, size=64):
/// - Offset  0: next pointer (LayerCollection*) — Blender's BLO_read_struct_list follows this
/// - Offset  8: prev pointer (LayerCollection*) — overwritten during list traversal
/// - Offset 16: collection pointer (Collection*)
/// - Offset 24: _pad1 (8 bytes)
/// - Offset 32: flag (2), runtime_flag (2), _pad (4)
/// - Offset 40: layer_collections ListBase (16 bytes) — nested child LayerCollections
/// - Offset 56: local_collections_bits (2), _pad2[3] (6)
pub fn build_layer_collection_linked(
    collection_ptr: u64,
    prev_ptr: u64,
    next_ptr: u64,
    child_layer_collections_head: u64,
    child_layer_collections_tail: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; LAYER_COLLECTION_SIZE];
    
    // Offset 0: next pointer — BLO_read_struct_list follows this for traversal
    write_ptr(&mut data, 0, next_ptr);
    
    // Offset 8: prev pointer — overwritten by Blender during traversal, but set for correctness
    write_ptr(&mut data, 8, prev_ptr);
    
    // Offset 16: collection pointer
    write_ptr(&mut data, 16, collection_ptr);
    
    // Offset 40-47: layer_collections ListBase.first (nested children head)
    write_ptr(&mut data, 40, child_layer_collections_head);
    
    // Offset 48-55: layer_collections ListBase.last (nested children tail)
    write_ptr(&mut data, 48, child_layer_collections_tail);
    
    // Offset 32: flag (LAYER_COLLECTION_VISIBLE = 0x0001)
    write_u16(&mut data, 32, 0x0001);
    
    data
}

/// Build a simple LayerCollection (for backward compatibility).
/// This variant creates a standalone LayerCollection with no siblings and no children.
pub fn build_layer_collection(collection_ptr: u64) -> Vec<u8> {
    build_layer_collection_linked(collection_ptr, 0, 0, 0, 0)
}

/// Build a mesh `Object` block (`OB_MESH`, type=1).
///
/// `properties_ptr` points to the root `IDProperty` group block (0 = no custom props).
pub fn build_object(
    object_name: &str,
    mesh_ptr: u64,
    object_mat_ptr: u64,
    matbits_ptr: u64,
    material_slots: i32,
    properties_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 1); // OB_MESH
    write_ptr(&mut data, 344, properties_ptr);
    write_ptr(&mut data, 552, mesh_ptr);
    write_ptr(&mut data, 712, object_mat_ptr);
    write_ptr(&mut data, 720, matbits_ptr);
    write_i32(&mut data, 728, material_slots);
    write_i16(&mut data, 732, if material_slots > 0 { 1 } else { 0 }); // actcol
    // Default transform: zero translation, unit scale, identity quaternion.
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, 0.0); }  // loc
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, 1.0); }  // scale
    for i in 0..3 { write_f32(&mut data, 784 + i * 4, 1.0); }  // dscale
    write_f32(&mut data, 820, 1.0); // quat[0] = w
    write_f32(&mut data, 836, 1.0); // dquat[0] = w
    write_f32(&mut data, 856, 1.0); // rotAxis.y
    write_f32(&mut data, 868, 1.0); // drotAxis.y
    write_identity_matrix4x4(&mut data, 884); // parentinv
    write_identity_matrix4x4(&mut data, 948); // constinv
    write_i16(&mut data, 1040, 1);  // ROT_MODE_EUL
    write_i16(&mut data, 1042, -1); // protectflag = OB_LOCK_ROT4D
    data[1046] = 5; // Object.dt = OB_TEXTURE
    data[1047] = 2; // empty_drawtype = OB_PLAINAXES
    write_f32(&mut data, 1048, 1.0); // empty_drawsize
    write_f32(&mut data, 1052, 1.0); // instance_faces_scale
    for i in 0..4 { write_f32(&mut data, 1064 + i * 4, 1.0); } // Object.color
    data
}

/// Build an `OB_EMPTY` Object (type=0, no mesh data).
pub fn build_empty_object(
    object_name: &str,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    build_empty_object_with_properties(object_name, loc, quat, scale, parent_ptr, 0)
}

/// Build an `OB_EMPTY` Object with optional custom properties.
pub fn build_empty_object_with_properties(
    object_name: &str,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
    properties_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 0); // OB_EMPTY
    write_ptr(&mut data, 344, properties_ptr);
    write_ptr(&mut data, 496, parent_ptr);
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, loc[i]); }
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, scale[i]); }
    for i in 0..3 { write_f32(&mut data, 784 + i * 4, 1.0); }
    for i in 0..4 { write_f32(&mut data, 820 + i * 4, quat[i]); }
    write_f32(&mut data, 836, 1.0);
    write_f32(&mut data, 856, 1.0);
    write_f32(&mut data, 868, 1.0);
    write_identity_matrix4x4(&mut data, 884);
    write_identity_matrix4x4(&mut data, 948);
    write_i16(&mut data, 1040, 0);
    write_i16(&mut data, 1042, -1);
    data[1046] = 5;
    data[1047] = 2;
    write_f32(&mut data, 1048, 1.0);
    write_f32(&mut data, 1052, 1.0);
    for i in 0..4 { write_f32(&mut data, 1064 + i * 4, 1.0); }
    data
}

/// Build an `OB_LAMP` Object (type=10) pointing to a `Lamp` datablock.
pub fn build_lamp_object(
    object_name: &str,
    lamp_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    build_lamp_object_with_properties(object_name, lamp_ptr, loc, quat, scale, parent_ptr, 0)
}

/// Build an `OB_LAMP` Object with optional custom properties.
pub fn build_lamp_object_with_properties(
    object_name: &str,
    lamp_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
    properties_ptr: u64,
) -> Vec<u8> {
    build_lamp_object_with_properties_and_visibility(
        object_name,
        lamp_ptr,
        loc,
        quat,
        scale,
        parent_ptr,
        properties_ptr,
        false,
    )
}

/// Build an `OB_LAMP` Object with optional custom properties and ray visibility.
pub fn build_lamp_object_with_properties_and_visibility(
    object_name: &str,
    lamp_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
    properties_ptr: u64,
    hide_camera: bool,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 10); // OB_LAMP
    write_ptr(&mut data, 344, properties_ptr);
    write_ptr(&mut data, 496, parent_ptr);
    write_ptr(&mut data, 552, lamp_ptr);
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, loc[i]); }
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, scale[i]); }
    for i in 0..3 { write_f32(&mut data, 784 + i * 4, 1.0); }
    for i in 0..4 { write_f32(&mut data, 820 + i * 4, quat[i]); }
    write_f32(&mut data, 836, 1.0);
    write_f32(&mut data, 856, 1.0);
    write_f32(&mut data, 868, 1.0);
    write_identity_matrix4x4(&mut data, 884);
    write_identity_matrix4x4(&mut data, 948);
    write_i16(&mut data, 1040, 0);
    write_i16(&mut data, 1042, -1);
    data[1046] = 5;
    data[1047] = 2;
    write_f32(&mut data, 1048, 1.0);
    write_f32(&mut data, 1052, 1.0);
    for i in 0..4 { write_f32(&mut data, 1064 + i * 4, 1.0); }
    if hide_camera {
        write_i16(&mut data, 1082, 0x0008); // OB_HIDE_CAMERA
    }
    data
}

/// Build an `OB_EMPTY` Object that links to an external mesh via an ID stub.
///
/// This creates an instance that references a mesh datablock from an external library.
/// The object contains a pointer to an ID stub which in turn points to the library.
pub fn build_linked_instance_object(
    object_name: &str,
    id_stub_ptr: u64,
    loc: [f32; 3],
    quat: [f32; 4],
    scale: [f32; 3],
    parent_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_SIZE];
    write_id_name(&mut data, "OB", object_name);
    write_i16(&mut data, 416, 1); // OB_MESH - linked instances must be MESH type to load external mesh data
    write_ptr(&mut data, 496, parent_ptr);
    write_ptr(&mut data, 552, id_stub_ptr); // Point to the ID stub instead of leaving blank
    write_ptr(&mut data, 712, 0); // No local material array (using linked mesh materials)
    write_ptr(&mut data, 720, 0); // No local material bits
    write_i32(&mut data, 728, 0); // No local material slots
    write_i16(&mut data, 732, 0); // actcol = 0
    for i in 0..3 { write_f32(&mut data, 736 + i * 4, loc[i]); }
    for i in 0..3 { write_f32(&mut data, 760 + i * 4, scale[i]); }
    for i in 0..3 { write_f32(&mut data, 784 + i * 4, 1.0); }
    for i in 0..4 { write_f32(&mut data, 820 + i * 4, quat[i]); }
    write_f32(&mut data, 836, 1.0);
    write_f32(&mut data, 856, 1.0);
    write_f32(&mut data, 868, 1.0);
    write_identity_matrix4x4(&mut data, 884);
    write_identity_matrix4x4(&mut data, 948);
    write_i16(&mut data, 1040, 0); // ROT_MODE_QUAT
    write_i16(&mut data, 1042, -1); // protectflag = OB_LOCK_ROT4D
    data[1046] = 5; // Object.dt = OB_TEXTURE
    data[1047] = 2; // empty_drawtype = OB_PLAINAXES
    write_f32(&mut data, 1048, 1.0); // empty_drawsize
    write_f32(&mut data, 1052, 1.0); // instance_faces_scale
    for i in 0..4 { write_f32(&mut data, 1064 + i * 4, 1.0); } // Object.color
    data
}

/// Build a `Lamp` (Light) datablock.
///
/// `lamp_type`: 0 = POINT, 1 = SUN, 2 = SPOT, 4 = AREA.
pub fn build_lamp(
    lamp_name: &str,
    lamp_type: i16,
    color: [f32; 3],
    energy: f32,
    radius: f32,
    cutoff_distance: f32,
    spot_size: f32,
    spot_blend: f32,
    temperature_k: f32,
    use_temperature: bool,
) -> Vec<u8> {
    build_lamp_with_node_tree(
        lamp_name,
        lamp_type,
        color,
        energy,
        radius,
        cutoff_distance,
        spot_size,
        spot_blend,
        temperature_k,
        use_temperature,
        0,
    )
}

/// Build a `Lamp` (Light) datablock with an optional embedded shader node tree.
///
/// `lamp_type`: 0 = POINT, 1 = SUN, 2 = SPOT, 4 = AREA.
pub fn build_lamp_with_node_tree(
    lamp_name: &str,
    lamp_type: i16,
    color: [f32; 3],
    energy: f32,
    radius: f32,
    cutoff_distance: f32,
    spot_size: f32,
    spot_blend: f32,
    temperature_k: f32,
    use_temperature: bool,
    node_tree_ptr: u64,
) -> Vec<u8> {
    build_lamp_with_node_tree_and_properties(
        lamp_name,
        lamp_type,
        color,
        energy,
        radius,
        cutoff_distance,
        spot_size,
        spot_blend,
        temperature_k,
        use_temperature,
        node_tree_ptr,
        0,
    )
}

/// Build a `Lamp` (Light) datablock with optional shader node tree and custom properties.
pub fn build_lamp_with_node_tree_and_properties(
    lamp_name: &str,
    lamp_type: i16,
    color: [f32; 3],
    energy: f32,
    radius: f32,
    cutoff_distance: f32,
    spot_size: f32,
    spot_blend: f32,
    temperature_k: f32,
    use_temperature: bool,
    node_tree_ptr: u64,
    properties_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; LAMP_SIZE];
    write_id_name(&mut data, "LA", lamp_name);
    write_ptr(&mut data, 344, properties_ptr);
    write_i16(&mut data, 416, lamp_type);
    let mut mode = 0x0020_0001u32; // LA_SHADOW | LA_USE_SOFT_FALLOFF
    if use_temperature {
        mode |= 1 << 24; // LA_USE_TEMPERATURE
    }
    write_u32(&mut data, 420, mode);
    write_f32(&mut data, 424, color[0]);
    write_f32(&mut data, 428, color[1]);
    write_f32(&mut data, 432, color[2]);
    write_f32(&mut data, 436, temperature_k);
    write_f32(&mut data, 440, energy);
    write_f32(&mut data, 448, radius);
    write_f32(&mut data, 452, spot_size);
    write_f32(&mut data, 456, spot_blend);
    write_f32(&mut data, 512, 1.0); // diffuse_factor
    write_f32(&mut data, 516, 1.0); // specular_factor
    write_f32(&mut data, 520, 1.0); // transmission_factor
    write_f32(&mut data, 524, 1.0); // volume_factor
    write_f32(&mut data, 528, cutoff_distance);
    if node_tree_ptr != 0 {
        write_i16(&mut data, 486, 1); // use_nodes
        write_ptr(&mut data, 552, node_tree_ptr); // nodetree
    }
    write_f32(&mut data, 560, energy); // deprecated energy mirror
    data
}

/// Build an Image datablock for a file-backed texture.
pub fn build_image(image_name: &str, filepath: &str) -> Vec<u8> {
    build_image_with_tile(image_name, filepath, 0, "Linear Rec.709")
}

/// Build an Image datablock for a file-backed texture with an optional default tile and colorspace.
///
/// `tile_ptr` must be nonzero for file-backed images: Blender 5.x requires at least one
/// `ImageTile` (UDIM 1001) to load images from disk.  Pass zero only when the image will not be
/// used as a texture (e.g. viewer/compositing images).
///
/// `colorspace` is the name of the Blender colorspace, e.g. `"Linear Rec.709"` for colour textures
/// or `"Non-Color"` for data/mask textures such as gobo projector patterns.
pub fn build_image_with_tile(
    image_name: &str,
    filepath: &str,
    tile_ptr: u64,
    colorspace: &str,
) -> Vec<u8> {
    let mut data = vec![0u8; IMAGE_SIZE];
    write_id_name(&mut data, "IM", image_name);
    write_cstr_fixed(&mut data, 416, 1024, filepath);
    write_i16(&mut data, 1488, 1); // source = IMA_SRC_FILE
    write_i16(&mut data, 1490, 0); // type = IMA_TYPE_IMAGE
    write_i16(&mut data, 1496, 8); // seam_margin
    write_f32(&mut data, 1568, 1.0); // aspx
    write_f32(&mut data, 1572, 1.0); // aspy
    write_cstr_fixed(&mut data, 1576, 64, colorspace);
    if tile_ptr != 0 {
        write_i32(&mut data, 1644, 0); // active_tile_index
        write_ptr(&mut data, 1648, tile_ptr); // tiles.first
        write_ptr(&mut data, 1656, tile_ptr); // tiles.last
    }
    data
}

/// Build Blender's default ImageTile record for a single-tile file image.
pub fn build_image_tile() -> Vec<u8> {
    let mut data = vec![0u8; IMAGE_TILE_SIZE];
    write_i32(&mut data, 40, 1001); // tile_number
    write_i32(&mut data, 44, 1024); // gen_x
    write_i32(&mut data, 48, 1024); // gen_y
    data[52] = 1; // gen_type = IMA_GENTYPE_GRID
    data
}

/// Build a `Mesh` datablock.
pub fn build_mesh(
    mesh_name: &str,
    totvert: usize,
    totedge: usize,
    totpoly: usize,
    totloop: usize,
    poly_offset_indices_ptr: u64,
    attributes_ptr: u64,
    mesh_mat_ptr: u64,
    material_slots: i16,
    vgroup_first_ptr: u64,
    vgroup_last_ptr: u64,
    cdl_ptr: u64,
    num_attributes: u32,
) -> Vec<u8> {
    // Initialize mesh with all zeros. Edge/polygon/loop CustomData domains (edata, pdata, ldata)
    // are intentionally left empty since geometry is represented via the Attribute system
    // (standard in Blender 4.0+). Only vdata.CustomData is used for vertex groups (MDeformVert).
    let mut data = vec![0u8; MESH_SIZE];
    write_id_name(&mut data, "ME", mesh_name);
    write_u32(&mut data, 432, totvert as u32);
    write_u32(&mut data, 436, totedge as u32);
    write_u32(&mut data, 440, totpoly as u32);
    write_u32(&mut data, 444, totloop as u32);
    write_ptr(&mut data, 424, mesh_mat_ptr);
    write_ptr(&mut data, 448, poly_offset_indices_ptr);
    // AttributeStorage (inline): dna_attributes*, dna_attributes_num, pad, runtime*
    write_ptr(&mut data, 456, attributes_ptr);
    write_i32(&mut data, 464, num_attributes as i32);
    write_i16(&mut data, 1618, material_slots);
    // vertex_group_names ListBase (first/last pointers); vertex_group_active_index at
    // +1488 and attributes_active_index at +1492 are left as 0 (Blender defaults).
    write_ptr(&mut data, 1472, vgroup_first_ptr);
    write_ptr(&mut data, 1480, vgroup_last_ptr);
    if cdl_ptr != 0 {
        write_ptr(&mut data, 480, cdl_ptr);
        write_i32(&mut data, 488 + 2 * 4, 0); // typemap[CD_MDEFORMVERT=2] = 0
        write_i32(&mut data, 700, 1); // totlayer
        write_i32(&mut data, 704, 1); // maxlayer
        write_i32(&mut data, 708, 16); // vdata.totsize = MDEFORMVERT_SIZE
    }
    data
}

/// Build an empty mesh stub for linked instances (0 vertices, 0 polygons).
/// This provides a valid Mesh datablock that Objects can point to, satisfying Blender's
/// requirement that Objects have valid data pointers. The actual geometry comes from the
/// linked external .blend file.
pub fn build_mesh_stub(mesh_name: &str) -> Vec<u8> {
    let mut data = vec![0u8; MESH_SIZE];
    write_id_name(&mut data, "ME", mesh_name);
    // All other fields remain zero: totvert=0, totpoly=0, totloop=0, no materials, no data pointers
    data
}

pub fn build_attribute(name_ptr: u64, data_type: i16, domain: u8, data_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; ATTRIBUTE_SIZE];
    write_ptr(&mut data, 0, name_ptr);
    write_i16(&mut data, 8, data_type);
    data[10] = domain;
    data[11] = ATTR_STORAGE_ARRAY;
    write_ptr(&mut data, 16, data_ptr);
    data
}

pub fn build_attribute_array(raw_data_ptr: u64, size: i64) -> Vec<u8> {
    let mut data = vec![0u8; ATTRIBUTE_ARRAY_SIZE];
    write_ptr(&mut data, 0, raw_data_ptr);
    data[16..24].copy_from_slice(&size.to_le_bytes());
    data
}

/// Build a `bDeformGroup` block (vertex group name entry, 88 bytes).
pub fn build_bdeformgroup(name: &str, next_ptr: u64, prev_ptr: u64) -> Vec<u8> {
    let mut data = vec![0u8; BDEFORMGROUP_SIZE];
    write_ptr(&mut data, 0, next_ptr);
    write_ptr(&mut data, 8, prev_ptr);
    write_cstr_fixed(&mut data, 16, 64, name);
    data
}

/// Build a `CustomDataLayer` for `MDeformVert` (CD type = 2, 112 bytes).
pub fn build_custom_data_layer_mdeformvert(data_ptr: u64) -> Vec<u8> {
    let mut buf = vec![0u8; CUSTOM_DATA_LAYER_SIZE];
    write_i32(&mut buf, 0, 2); // type = CD_MDEFORMVERT
    write_i32(&mut buf, 4, 0); // offset = 0 (first/only layer in interleaved data)
    write_ptr(&mut buf, 96, data_ptr);
    buf
}

/// Build an `MDeformVert[]` block (16 bytes per vertex).
pub fn build_mdeformvert_array(dw_ptrs: &[(u64, u32)]) -> Vec<u8> {
    let mut data = vec![0u8; dw_ptrs.len() * MDEFORMVERT_SIZE];
    for (i, (dw_ptr, totweight)) in dw_ptrs.iter().enumerate() {
        write_ptr(&mut data, i * 16, *dw_ptr);
        write_i32(&mut data, i * 16 + 8, *totweight as i32);
    }
    data
}

/// Build an `MDeformWeight[]` block for a single vertex (8 bytes per entry).
pub fn build_mdeformweight_array(weights: &[(u32, f32)]) -> Vec<u8> {
    let mut data = vec![0u8; weights.len() * MDEFORMWEIGHT_SIZE];
    for (i, (def_nr, weight)) in weights.iter().enumerate() {
        write_u32(&mut data, i * 8, *def_nr);
        write_f32(&mut data, i * 8 + 4, *weight);
    }
    data
}

/// Custom property value for IDProperty creation.
#[derive(Clone, Debug, PartialEq)]
pub enum IdPropValue {
    String(String),
    Int(i32),
    Double(f64),
}

/// Build a single `IDProperty` block (144 bytes).
///
/// ## IDPropertyData union layout (SDNA-verified, Blender 5.1):
/// - `@88`: `data.pointer` (8 bytes) — string data pointer for `IDP_STRING`
/// - `@96`: `group.first` (8 bytes) — first child, for `IDP_GROUP`
/// - `@104`: `group.last`  (8 bytes) — last child,  for `IDP_GROUP`
/// - `@112`: `children_map*` (8 bytes)
/// - `@120`: `val` (i32) — integer value for `IDP_INT`
/// - `@124`: `val2` (i32) — high word for `IDP_DOUBLE`
/// - `@128`: `len`  (i32) — for `IDP_STRING`: byte count including null; for `IDP_GROUP`: child count
/// - `@132`: `totallen` (i32) — allocated buffer length (same as `len` for `IDP_STRING`)
pub fn build_idproperty(
    itype: u8,
    name: &str,
    next_ptr: u64,
    prev_ptr: u64,
    data_ptr: u64,
    group_first_ptr: u64,
    group_last_ptr: u64,
    val_int: i32,
    val_double: f64,
    str_len: i32,
    str_totallen: i32,
) -> Vec<u8> {
    let mut b = vec![0u8; IDPROPERTY_SIZE];
    write_ptr(&mut b, 0, next_ptr);
    write_ptr(&mut b, 8, prev_ptr);
    b[16] = itype;
    if itype != IDP_STRING {
        b[17] = itype;
    }
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(63);
    b[20..20 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
    write_ptr(&mut b, 88, data_ptr); // IDP_STRING: string data pointer
    if itype == IDP_GROUP {
        write_ptr(&mut b, 96, group_first_ptr);
        write_ptr(&mut b, 104, group_last_ptr);
    }
    match itype {
        IDP_INT => write_i32(&mut b, 120, val_int),
        IDP_DOUBLE => b[120..128].copy_from_slice(&val_double.to_le_bytes()),
        _ => {}
    }
    write_i32(&mut b, 128, str_len);
    write_i32(&mut b, 132, str_totallen);
    b
}

/// Build a complete IDProperty subtree for a list of custom object properties.
///
/// Returns `(root_block, children, string_data)`:
/// - `root_block`: the `IDP_GROUP` "user_properties" root (144 bytes)
/// - `children`: `Vec<(old_ptr, block_bytes)>` — child IDProperty blocks
/// - `string_data`: `Vec<(old_ptr, data_bytes)>` — string DATA blocks (in child order)
///
/// **Block ordering rule**: write blocks in this sequence after the parent `OB` block:
/// 1. `DATA` root group
/// 2. For each child: `DATA` child IDProperty, then immediately `DATA` string data (if IDP_STRING)
/// 3. `DATA` Object.mat** and matbits (if any)
pub fn build_idprop_tree(
    _root_ptr: u64,
    child_ptrs: &[u64],
    str_data_ptrs: &[u64],
    props: &[(String, IdPropValue)],
) -> (Vec<u8>, Vec<(u64, Vec<u8>)>, Vec<(u64, Vec<u8>)>) {
    assert_eq!(child_ptrs.len(), props.len());
    let n = props.len();
    let mut str_idx = 0usize;
    let mut children = Vec::with_capacity(n);
    let mut string_data = Vec::new();

    for (i, (name, val)) in props.iter().enumerate() {
        let next_ptr = if i + 1 < n { child_ptrs[i + 1] } else { 0 };
        let prev_ptr = if i > 0 { child_ptrs[i - 1] } else { 0 };

        let (itype, data_ptr, str_data, slen, stotlen, ival, dval) = match val {
            IdPropValue::String(s) => {
                let mut bytes = s.as_bytes().to_vec();
                bytes.push(0);
                let slen = bytes.len() as i32; // len = totallen = strlen+1 (Blender convention)
                let sdptr = str_data_ptrs[str_idx];
                str_idx += 1;
                (IDP_STRING, sdptr, Some(bytes), slen, slen, 0i32, 0f64)
            }
            IdPropValue::Int(v) => (IDP_INT, 0u64, None, 0i32, 0i32, *v, 0f64),
            IdPropValue::Double(v) => (IDP_DOUBLE, 0u64, None, 0i32, 0i32, 0i32, *v),
        };

        let block = build_idproperty(
            itype, name, next_ptr, prev_ptr, data_ptr, 0, 0, ival, dval, slen, stotlen,
        );
        children.push((child_ptrs[i], block));

        if let Some(str_bytes) = str_data {
            string_data.push((str_data_ptrs[str_idx - 1], str_bytes));
        }
    }

    let group_first = if n > 0 { child_ptrs[0] } else { 0 };
    let group_last = if n > 0 { child_ptrs[n - 1] } else { 0 };
    let root = build_idproperty(
        IDP_GROUP, "user_properties", 0, 0, 0,
        group_first, group_last, 0, 0.0, n as i32, n as i32,
    );

    (root, children, string_data)
}

/// Build an `Object.mat**` / `Mesh.mat**` pointer array block — all null pointers.
pub fn build_mat_ptr_array(n: usize) -> Vec<u8> {
    vec![0u8; n * 8]
}

pub fn build_mat_ptr_array_from_ptrs(ptrs: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ptrs.len() * 8);
    for ptr in ptrs {
        out.extend_from_slice(&ptr.to_le_bytes());
    }
    out
}

/// Build an `Object.matbits` block — one byte per slot, 0 = mesh-linked.
pub fn build_matbits(n: usize) -> Vec<u8> {
    vec![0u8; n]
}

/// Build a minimal named `Material` datablock.
pub fn build_material(material_name: &str) -> Vec<u8> {
    build_material_with_node_tree(material_name, 0)
}

/// Build a minimal named `Material` datablock with an optional embedded node tree pointer.
///
/// Blender 5.1 keeps `Material.use_nodes` only as a deprecated compatibility
/// byte; `Material.nodetree` must point at an embedded `bNodeTree` for the node
/// editor to have an editable shader tree.
pub fn build_material_with_node_tree(material_name: &str, node_tree_ptr: u64) -> Vec<u8> {
    build_material_with_node_tree_and_properties(material_name, node_tree_ptr, 0)
}

/// Build a minimal named `Material` datablock with optional embedded node tree and custom properties.
pub fn build_material_with_node_tree_and_properties(
    material_name: &str,
    node_tree_ptr: u64,
    properties_ptr: u64,
) -> Vec<u8> {
    let mut data = vec![0u8; MATERIAL_SIZE];
    write_id_name(&mut data, "MA", material_name);
    write_ptr(&mut data, 344, properties_ptr);
    write_f32(&mut data, 420, 0.8); // r
    write_f32(&mut data, 424, 0.8); // g
    write_f32(&mut data, 428, 0.8); // b
    write_f32(&mut data, 432, 1.0); // a
    write_f32(&mut data, 436, 1.0); // specr
    write_f32(&mut data, 440, 1.0); // specg
    write_f32(&mut data, 444, 1.0); // specb
    write_f32(&mut data, 456, 0.5); // spec
    write_f32(&mut data, 464, 0.4); // roughness
    data[472] = u8::from(node_tree_ptr != 0); // use_nodes
    data[473] = 1; // pr_type = MA_SPHERE
    write_ptr(&mut data, 480, node_tree_ptr); // nodetree
    write_f32(&mut data, 524, 0.5); // alpha_threshold
    data[533] = 1; // blend_shadow = MA_BS_SOLID
    data[534] = 1 << 6; // blend_flag = MA_BL_TRANSPARENT_SHADOW
    data
}

/// Build an empty embedded shader `bNodeTree` for a material.
///
/// Write this as a `DATA` block immediately after its owning `MA` block. An
/// empty tree is valid in Blender 5.1 and lets Blender/Python/UI add shader
/// nodes later; default Principled/Output nodes are intentionally not faked.
pub fn build_empty_shader_node_tree(owner_material_ptr: u64) -> Vec<u8> {
    build_empty_shader_node_tree_named(owner_material_ptr, "Shader Nodetree")
}

pub fn build_empty_shader_node_tree_named(owner_material_ptr: u64, tree_name: &str) -> Vec<u8> {
    let mut data = vec![0u8; BNODE_TREE_SIZE];
    write_id_name(&mut data, "NT", tree_name);
    write_i16(&mut data, 298, 0x0400); // ID_FLAG_EMBEDDED_DATA
    write_ptr(&mut data, 416, owner_material_ptr); // owner_id
    write_cstr_fixed(&mut data, 432, 64, "ShaderNodeTree"); // idname
    write_i32(&mut data, 552, 0); // type = NTREE_SHADER
    data
}

/// Write all blocks for a World with a Sky Texture → Background → World Output
/// shader node tree, producing a non-black sky in both Cycles and EEVEE.
///
/// Emits directly into `out`:
///   WO block, then 20 DATA blocks: bNodeTree, bNode×3, bNodeSocket×8,
///   default-value structs×5, NodeTexSky, bNodeLink×2.
///
/// `world_ptr` and `world_node_tree_ptr` must already be allocated by the caller
/// (they are referenced by the Scene block). All additional block pointers are
/// allocated here via `ptrs`.
pub fn write_world_with_sky_shader(
    out: &mut Vec<u8>,
    world_name: &str,
    world_ptr: u64,
    world_node_tree_ptr: u64,
    ptrs: &mut PtrAlloc,
) {
    let world_output_ptr    = ptrs.alloc();
    let background_ptr      = ptrs.alloc();
    let sky_tex_ptr         = ptrs.alloc();

    let wo_surface_ptr      = ptrs.alloc();
    let wo_volume_ptr       = ptrs.alloc();

    let bg_color_ptr        = ptrs.alloc();
    let bg_strength_ptr     = ptrs.alloc();
    let bg_weight_ptr       = ptrs.alloc();
    let bg_out_ptr          = ptrs.alloc();

    let bg_color_def_ptr    = ptrs.alloc();
    let bg_strength_def_ptr = ptrs.alloc();
    let bg_weight_def_ptr   = ptrs.alloc();

    let sky_vector_ptr      = ptrs.alloc();
    let sky_color_out_ptr   = ptrs.alloc();

    let sky_vector_def_ptr  = ptrs.alloc();
    let sky_color_def_ptr   = ptrs.alloc();

    let sky_storage_ptr     = ptrs.alloc();

    let link1_ptr = ptrs.alloc(); // Background.out → WorldOutput.Surface
    let link2_ptr = ptrs.alloc(); // SkyTexture.Color → Background.Color

    // ── WO block ─────────────────────────────────────────────────────────────
    write_block(out, b"WO\0\0", SDNA_IDX_WORLD, world_ptr, 1,
        &build_world_with_node_tree(world_name, world_node_tree_ptr));

    // ── bNodeTree (embedded DATA, owner = WO) ────────────────────────────────
    {
        let mut d = vec![0u8; BNODE_TREE_SIZE];
        write_id_name(&mut d, "NT", "World Nodetree");
        write_i16(&mut d, 298, 0x0400);                        // ID_FLAG_EMBEDDED_DATA
        write_ptr(&mut d, 416, world_ptr);                     // owner_id
        write_cstr_fixed(&mut d, 432, 64, "ShaderNodeTree");   // idname
        write_ptr(&mut d, 520, world_output_ptr);              // nodes.first
        write_ptr(&mut d, 528, sky_tex_ptr);                   // nodes.last
        write_ptr(&mut d, 536, link1_ptr);                     // links.first
        write_ptr(&mut d, 544, link2_ptr);                     // links.last
        write_i32(&mut d, 552, 0);                             // type = NTREE_SHADER
        write_block(out, b"DATA", SDNA_IDX_BNODE_TREE, world_node_tree_ptr, 1, &d);
    }

    // ── bNode: WorldOutput ───────────────────────────────────────────────────
    {
        let mut d = vec![0u8; BNODE_SIZE];
        write_ptr(&mut d, 0x000, background_ptr);              // next
        write_ptr(&mut d, 0x010, wo_surface_ptr);              // inputs.first
        write_ptr(&mut d, 0x018, wo_volume_ptr);               // inputs.last
        write_i32(&mut d, 0x074, 0x00010042i32);               // flag
        write_cstr_fixed(&mut d, 0x030, 64, "World Output");
        write_cstr_fixed(&mut d, 0x078, 64, "ShaderNodeOutputWorld");
        write_i16(&mut d, 0x0c0, 125);                         // type = SH_NODE_OUTPUT_WORLD
        write_f32(&mut d, 0x100, 120.0); write_f32(&mut d, 0x104, 100.0);
        write_f32(&mut d, 0x108, 140.0); write_f32(&mut d, 0x10c, 100.0);
        write_block(out, b"DATA", SDNA_IDX_BNODE, world_output_ptr, 1, &d);
    }
    // WorldOutput.Surface (SHADER in, connected via link1)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, wo_surface_ptr, 1,
        &build_bnode_socket(wo_volume_ptr, 0, "Surface", "Surface",
            3, 0x0044, 1, 1, "NodeSocketShader", 0, link1_ptr));
    // WorldOutput.Volume (SHADER in)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, wo_volume_ptr, 1,
        &build_bnode_socket(0, wo_surface_ptr, "Volume", "Volume",
            3, 0x0040, 1, 1, "NodeSocketShader", 0, 0));

    // ── bNode: Background ────────────────────────────────────────────────────
    {
        let mut d = vec![0u8; BNODE_SIZE];
        write_ptr(&mut d, 0x000, sky_tex_ptr);                 // next
        write_ptr(&mut d, 0x008, world_output_ptr);            // prev
        write_ptr(&mut d, 0x010, bg_color_ptr);                // inputs.first
        write_ptr(&mut d, 0x018, bg_weight_ptr);               // inputs.last
        write_ptr(&mut d, 0x020, bg_out_ptr);                  // outputs.first
        write_ptr(&mut d, 0x028, bg_out_ptr);                  // outputs.last
        write_i32(&mut d, 0x074, 0x00010002i32);               // flag
        write_cstr_fixed(&mut d, 0x030, 64, "Background");
        write_cstr_fixed(&mut d, 0x078, 64, "ShaderNodeBackground");
        write_i16(&mut d, 0x0c0, 130);                         // type = SH_NODE_BACKGROUND
        write_i16(&mut d, 0x0c2, 1);                           // ui_order
        write_f32(&mut d, 0x100, -80.0); write_f32(&mut d, 0x104, 100.0);
        write_f32(&mut d, 0x108, 140.0); write_f32(&mut d, 0x10c, 100.0);
        write_block(out, b"DATA", SDNA_IDX_BNODE, background_ptr, 1, &d);
    }
    // Background.Color (RGBA in, connected via link2)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, bg_color_ptr, 1,
        &build_bnode_socket(bg_strength_ptr, 0, "Color", "Color",
            2, 0x0044, 1, 1, "NodeSocketColor", bg_color_def_ptr, link2_ptr));
    {
        let mut d = vec![0u8; BNSV_RGBA_SIZE];
        write_f32(&mut d, 0, 0.7948867); write_f32(&mut d, 4, 0.7948867);
        write_f32(&mut d, 8, 0.7948867); write_f32(&mut d, 12, 1.0);
        write_block(out, b"DATA", SDNA_IDX_BNSV_RGBA, bg_color_def_ptr, 1, &d);
    }
    // Background.Strength (FLOAT in)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, bg_strength_ptr, 1,
        &build_bnode_socket(bg_weight_ptr, bg_color_ptr, "Strength", "Strength",
            0, 0x0040, 1, 1, "NodeSocketFloat", bg_strength_def_ptr, 0));
    {
        let mut d = vec![0u8; BNSV_FLOAT_SIZE];
        // subtype=0, value=0.5, min=0.0, max=1_000_000.0
        write_f32(&mut d, 4, 0.5);
        write_f32(&mut d, 12, 1_000_000.0_f32);
        write_block(out, b"DATA", SDNA_IDX_BNSV_FLOAT, bg_strength_def_ptr, 1, &d);
    }
    // Background.Weight (FLOAT in)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, bg_weight_ptr, 1,
        &build_bnode_socket(0, bg_strength_ptr, "Weight", "Weight",
            0, 0x0048, 1, 1, "NodeSocketFloat", bg_weight_def_ptr, 0));
    {
        let mut d = vec![0u8; BNSV_FLOAT_SIZE];
        // subtype=0, value=0.0, min=-FLT_MAX, max=+FLT_MAX
        write_f32(&mut d, 8, f32::from_bits(0xff7fffff));
        write_f32(&mut d, 12, f32::from_bits(0x7f7fffff));
        write_block(out, b"DATA", SDNA_IDX_BNSV_FLOAT, bg_weight_def_ptr, 1, &d);
    }
    // Background.Background (SHADER out)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, bg_out_ptr, 1,
        &build_bnode_socket(0, 0, "Background", "Background",
            3, 0x0044, 4095, 2, "NodeSocketShader", 0, 0));

    // ── bNode: SkyTexture ────────────────────────────────────────────────────
    {
        let mut d = vec![0u8; BNODE_SIZE];
        write_ptr(&mut d, 0x008, background_ptr);              // prev
        write_ptr(&mut d, 0x010, sky_vector_ptr);              // inputs.first
        write_ptr(&mut d, 0x018, sky_vector_ptr);              // inputs.last
        write_ptr(&mut d, 0x020, sky_color_out_ptr);           // outputs.first
        write_ptr(&mut d, 0x028, sky_color_out_ptr);           // outputs.last
        write_i32(&mut d, 0x074, 0x00014012u32 as i32);        // flag
        write_cstr_fixed(&mut d, 0x030, 64, "Sky Texture");
        write_cstr_fixed(&mut d, 0x078, 64, "ShaderNodeTexSky");
        write_i16(&mut d, 0x0c0, 145);                         // type = SH_NODE_TEX_SKY
        write_i16(&mut d, 0x0c2, 2);                           // ui_order
        write_ptr(&mut d, 0x0e0, sky_storage_ptr);             // storage
        write_f32(&mut d, 0x100, -300.0); write_f32(&mut d, 0x104, 100.0);
        write_f32(&mut d, 0x108, 140.0); write_f32(&mut d, 0x10c, 100.0);
        write_block(out, b"DATA", SDNA_IDX_BNODE, sky_tex_ptr, 1, &d);
    }
    // SkyTexture.Vector (VECTOR in)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, sky_vector_ptr, 1,
        &build_bnode_socket(0, 0, "Vector", "Vector",
            1, 0x00c8u16 as i16, 1, 1, "NodeSocketVector", sky_vector_def_ptr, 0));
    {
        let mut d = vec![0u8; BNSV_VECTOR_SIZE];
        // subtype=0, value={0,0,0}, min=-FLT_MAX, max=+FLT_MAX, dims=3
        write_f32(&mut d, 16, f32::from_bits(0xff7fffff));
        write_f32(&mut d, 20, f32::from_bits(0x7f7fffff));
        write_i32(&mut d, 24, 3);
        write_block(out, b"DATA", SDNA_IDX_BNSV_VECTOR, sky_vector_def_ptr, 1, &d);
    }
    // SkyTexture.Color (RGBA out)
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, sky_color_out_ptr, 1,
        &build_bnode_socket(0, 0, "Color", "Color",
            2, 0x0044, 4095, 2, "NodeSocketColor", sky_color_def_ptr, 0));
    {
        let mut d = vec![0u8; BNSV_RGBA_SIZE];
        write_f32(&mut d, 0, 0.8); write_f32(&mut d, 4, 0.8);
        write_f32(&mut d, 8, 0.8); write_f32(&mut d, 12, 1.0);
        write_block(out, b"DATA", SDNA_IDX_BNSV_RGBA, sky_color_def_ptr, 1, &d);
    }

    // ── NodeTexSky storage ───────────────────────────────────────────────────
    {
        let mut d = reference_sky_texture_storage();
        write_f32(&mut d, 988, 0.1); // sun_intensity
        write_block(out, b"DATA", SDNA_IDX_NODE_TEX_SKY, sky_storage_ptr, 1, &d);
    }

    // ── bNodeLink × 2 ────────────────────────────────────────────────────────
    // link1: Background.Background_out → WorldOutput.Surface
    {
        let mut d = vec![0u8; BNODE_LINK_SIZE];
        write_ptr(&mut d, 0, link2_ptr);           // next
        write_ptr(&mut d, 16, background_ptr);     // fromnode
        write_ptr(&mut d, 24, world_output_ptr);   // tonode
        write_ptr(&mut d, 32, bg_out_ptr);         // fromsock
        write_ptr(&mut d, 40, wo_surface_ptr);     // tosock
        write_i32(&mut d, 48, 2);                  // flag
        write_block(out, b"DATA", SDNA_IDX_BNODE_LINK, link1_ptr, 1, &d);
    }
    // link2: SkyTexture.Color_out → Background.Color
    {
        let mut d = vec![0u8; BNODE_LINK_SIZE];
        write_ptr(&mut d, 8, link1_ptr);           // prev
        write_ptr(&mut d, 16, sky_tex_ptr);        // fromnode
        write_ptr(&mut d, 24, background_ptr);     // tonode
        write_ptr(&mut d, 32, sky_color_out_ptr);  // fromsock
        write_ptr(&mut d, 40, bg_color_ptr);       // tosock
        write_i32(&mut d, 48, 2);                  // flag
        write_block(out, b"DATA", SDNA_IDX_BNODE_LINK, link2_ptr, 1, &d);
    }
}

/// Write a file-backed projector/gobo shader graph for a Lamp.
///
/// Simple 3-node graph matching the reference: `Image Texture -> Emission -> Light Output`.
/// The Image Texture node's `iuser` is initialized with Blender's defaults
/// (`frames=100`, `sfra=1`) so the image is properly associated during GPU evaluation.
pub fn write_light_gobo_node_tree(
    out: &mut Vec<u8>,
    lamp_ptr: u64,
    node_tree_ptr: u64,
    image_ptr: u64,
    image_name: &str,
    image_filepath: &str,
    write_image_block: bool,
    ptrs: &mut PtrAlloc,
) {
    // Node pointers: Image Texture → Emission → Light Output
    let image_tile_ptr = ptrs.alloc();
    let image_tex_ptr = ptrs.alloc();
    let emission_ptr = ptrs.alloc();
    let output_ptr = ptrs.alloc();

    // Image Texture sockets
    let image_vector_in_ptr = ptrs.alloc();
    let image_color_out_ptr = ptrs.alloc();
    let image_alpha_out_ptr = ptrs.alloc();

    // Emission sockets
    let emission_color_in_ptr = ptrs.alloc();
    let emission_strength_in_ptr = ptrs.alloc();
    let emission_out_ptr = ptrs.alloc();

    // Output socket
    let output_surface_in_ptr = ptrs.alloc();

    // Default values
    let emission_color_def_ptr = ptrs.alloc();
    let emission_strength_def_ptr = ptrs.alloc();
    let image_vector_def_ptr = ptrs.alloc();

    // Image Texture storage
    let image_storage_ptr = ptrs.alloc();

    // Links: ImageTex.Color → Emission.Color, Emission → LightOutput
    let link_image_emission_ptr = ptrs.alloc();
    let link_emission_output_ptr = ptrs.alloc();

    {
        let mut d = vec![0u8; BNODE_TREE_SIZE];
        write_id_name(&mut d, "NT", "Shader Nodetree");
        write_i16(&mut d, 298, 0x0400); // ID_FLAG_EMBEDDED_DATA
        write_ptr(&mut d, 416, lamp_ptr); // owner_id
        write_cstr_fixed(&mut d, 432, 64, "ShaderNodeTree");
        write_ptr(&mut d, 520, image_tex_ptr);          // nodes.first
        write_ptr(&mut d, 528, output_ptr);             // nodes.last
        write_ptr(&mut d, 536, link_image_emission_ptr); // links.first
        write_ptr(&mut d, 544, link_emission_output_ptr); // links.last
        write_i32(&mut d, 552, 0); // NTREE_SHADER
        write_block(out, b"DATA", SDNA_IDX_BNODE_TREE, node_tree_ptr, 1, &d);
    }

    // Image Texture node (SH_NODE_TEX_IMAGE = 143): 1 input (Vector), 2 outputs (Color, Alpha)
    write_shader_node(
        out,
        image_tex_ptr,
        0,             // prev (first node)
        emission_ptr,  // next
        image_vector_in_ptr,
        image_vector_in_ptr,
        image_color_out_ptr,
        image_alpha_out_ptr,
        "Image Texture",
        "ShaderNodeTexImage",
        143,
        0x0001_0002,
        image_ptr,
        image_storage_ptr,
        -300.0,
        0.0,
    );
    // Emission node (SH_NODE_EMISSION = 140)
    write_shader_node(
        out,
        emission_ptr,
        image_tex_ptr,
        output_ptr,
        emission_color_in_ptr,
        emission_strength_in_ptr,
        emission_out_ptr,
        emission_out_ptr,
        "Emission",
        "ShaderNodeEmission",
        140,
        0x0001_0002,
        0,
        0,
        -100.0,
        0.0,
    );
    // Light Output node (SH_NODE_OUTPUT_LIGHT = 126, NODE_DO_OUTPUT = 0x40)
    write_shader_node(
        out,
        output_ptr,
        emission_ptr,
        0,
        output_surface_in_ptr,
        output_surface_in_ptr,
        0,
        0,
        "Light Output",
        "ShaderNodeOutputLight",
        126,
        0x0001_0042,
        0,
        0,
        100.0,
        0.0,
    );

    // Image Texture sockets (Vector input is unlinked, matches reference)
    write_vector_socket(out, image_vector_in_ptr, 0, 0, "Vector", 4095, 1, image_vector_def_ptr, 0);
    write_color_socket(out, image_color_out_ptr, image_alpha_out_ptr, 0, "Color", 4095, 2, 0, link_image_emission_ptr);
    write_float_socket(out, image_alpha_out_ptr, 0, image_color_out_ptr, "Alpha", 4095, 2, 0, 0);

    // Emission sockets
    write_color_socket(out, emission_color_in_ptr, emission_strength_in_ptr, 0, "Color", 1, 1, emission_color_def_ptr, link_image_emission_ptr);
    write_float_socket(out, emission_strength_in_ptr, 0, emission_color_in_ptr, "Strength", 1, 1, emission_strength_def_ptr, 0);
    write_shader_socket(out, emission_out_ptr, 0, 0, "Emission", 4095, 2, 0, link_emission_output_ptr);

    // Light Output socket
    write_shader_socket(out, output_surface_in_ptr, 0, 0, "Surface", 1, 1, 0, link_emission_output_ptr);

    // Default values
    write_color_default(out, emission_color_def_ptr, [1.0, 1.0, 1.0, 1.0]);
    write_float_default(out, emission_strength_def_ptr, 1.0);
    write_vector_default(out, image_vector_def_ptr, [0.0, 0.0, 0.0]);

    // Image Texture storage (1024 bytes).
    // NodeTexImage.iuser (ImageUser) starts at offset 960.
    // BKE_imageuser_default sets frames=100 (offset 972) and sfra=1 (offset 980).
    // Without these, iuser.frames=0 prevents Blender from associating the image
    // for the current frame during GPU shader evaluation, causing a black light.
    let mut image_storage = vec![0u8; NODE_TEX_IMAGE_SIZE];
    // NodeTexBase.tex_mapping (TexMapping) at offset 0 — must match BKE_texture_mapping_init.
    // TexMapping SDNA (144 bytes):
    //   loc[3]   +0000  (floats, zero → 0,0,0)
    //   rot[3]   +0012  (floats, zero → 0,0,0)
    //   size[3]  +0024  (floats, default 1,1,1)
    //   flag     +0036  (int,    zero)
    //   projx    +0040  (char,   TEXMAP_PROJ_X=1)
    //   projy    +0041  (char,   TEXMAP_PROJ_Y=2)
    //   projz    +0042  (char,   TEXMAP_PROJ_Z=3)
    //   mapping  +0043  (char,   TEXMAP_CLIP=0)
    //   type     +0044  (int,    0=POINT)
    //   mat[4][4]+0048  (floats, identity 4x4)
    //   min[3]   +0112  (floats, zero)
    //   max[3]   +0124  (floats, 1,1,1)
    //   *ob      +0136  (ptr,    null)
    //
    // size=(1,1,1)
    image_storage[24..28].copy_from_slice(&1.0f32.to_le_bytes());
    image_storage[28..32].copy_from_slice(&1.0f32.to_le_bytes());
    image_storage[32..36].copy_from_slice(&1.0f32.to_le_bytes());
    // projx/projy/projz: must be X/Y/Z so the gobo spot direction maps correctly to UV.
    // With projx=0 (NONE) Blender maps every ray to UV(0,0), sampling only the corner
    // pixel and making the gobo appear as a solid black or single-colour smear.
    image_storage[40] = 1; // TEXMAP_PROJ_X
    image_storage[41] = 2; // TEXMAP_PROJ_Y
    image_storage[42] = 3; // TEXMAP_PROJ_Z
    // mat[4][4] identity matrix (offsets 48..112, 16 floats, row-major)
    for i in 0..4 {
        let off = 48 + i * (4 * 4) + i * 4;
        image_storage[off..off + 4].copy_from_slice(&1.0f32.to_le_bytes());
    }
    // max[3]=(1,1,1) — matching BKE_texture_mapping_init default
    image_storage[124..128].copy_from_slice(&1.0f32.to_le_bytes());
    image_storage[128..132].copy_from_slice(&1.0f32.to_le_bytes());
    image_storage[132..136].copy_from_slice(&1.0f32.to_le_bytes());
    // iuser.frames = 100 at offset 972
    image_storage[972..976].copy_from_slice(&100u32.to_le_bytes());
    // iuser.sfra = 1 at offset 980
    image_storage[980..984].copy_from_slice(&1u32.to_le_bytes());
    write_block(out, b"DATA", SDNA_IDX_NODE_TEX_IMAGE, image_storage_ptr, 1, &image_storage);

    // Node links: ImageTex.Color → Emission.Color, Emission → LightOutput
    write_node_link(out, link_image_emission_ptr, 0, link_emission_output_ptr, image_tex_ptr, emission_ptr, image_color_out_ptr, emission_color_in_ptr);
    write_node_link(out, link_emission_output_ptr, link_image_emission_ptr, 0, emission_ptr, output_ptr, emission_out_ptr, output_surface_in_ptr);

    if write_image_block {
        // Image block — gobo projector textures are data/mask textures (Non-Color),
        // and require an ImageTile (UDIM 1001) so Blender 5.x can load the file from disk.
        write_block(
            out,
            b"IM\0\0",
            SDNA_IDX_IMAGE,
            image_ptr,
            1,
            &build_image_with_tile(image_name, image_filepath, image_tile_ptr, "Linear Rec.709"),
        );
        // ImageTile block — UDIM tile 1001 required for IMA_SRC_FILE images in Blender 5.x.
        // Without this tile, Image.tiles ListBase is null and Blender cannot load the file,
        // resulting in has_data=false and a magenta fallback colour on the gobo light.
        write_block(out, b"DATA", SDNA_IDX_IMAGE_TILE, image_tile_ptr, 1, &build_image_tile());
    }
}

fn write_shader_node(
    out: &mut Vec<u8>,
    ptr: u64,
    prev_ptr: u64,
    next_ptr: u64,
    inputs_first: u64,
    inputs_last: u64,
    outputs_first: u64,
    outputs_last: u64,
    name: &str,
    idname: &str,
    node_type: i16,
    flags: u32,
    id_ptr: u64,
    storage_ptr: u64,
    x: f32,
    y: f32,
) {
    let mut d = vec![0u8; BNODE_SIZE];
    write_ptr(&mut d, 0x000, next_ptr);
    write_ptr(&mut d, 0x008, prev_ptr);
    write_ptr(&mut d, 0x010, inputs_first);
    write_ptr(&mut d, 0x018, inputs_last);
    write_ptr(&mut d, 0x020, outputs_first);
    write_ptr(&mut d, 0x028, outputs_last);
    write_cstr_fixed(&mut d, 0x030, 64, name);
    write_i32(&mut d, 0x074, flags as i32);
    write_cstr_fixed(&mut d, 0x078, 64, idname);
    write_i16(&mut d, 0x0c0, node_type);
    write_ptr(&mut d, 0x0d8, id_ptr);
    write_ptr(&mut d, 0x0e0, storage_ptr);
    write_f32(&mut d, 0x100, x);
    write_f32(&mut d, 0x104, y);
    write_f32(&mut d, 0x108, 140.0);
    write_f32(&mut d, 0x10c, 100.0);
    write_block(out, b"DATA", SDNA_IDX_BNODE, ptr, 1, &d);
}

fn write_vector_socket(out: &mut Vec<u8>, ptr: u64, next: u64, prev: u64, name: &str, limit: i16, in_out: i16, default_ptr: u64, link_ptr: u64) {
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, ptr, 1,
        &build_bnode_socket(next, prev, name, name, 1, 0x0044, limit, in_out, "NodeSocketVector", default_ptr, link_ptr));
}

fn write_color_socket(out: &mut Vec<u8>, ptr: u64, next: u64, prev: u64, name: &str, limit: i16, in_out: i16, default_ptr: u64, link_ptr: u64) {
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, ptr, 1,
        &build_bnode_socket(next, prev, name, name, 2, 0x0044, limit, in_out, "NodeSocketColor", default_ptr, link_ptr));
}

fn write_float_socket(out: &mut Vec<u8>, ptr: u64, next: u64, prev: u64, name: &str, limit: i16, in_out: i16, default_ptr: u64, link_ptr: u64) {
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, ptr, 1,
        &build_bnode_socket(next, prev, name, name, 0, 0x0040, limit, in_out, "NodeSocketFloat", default_ptr, link_ptr));
}

fn write_shader_socket(out: &mut Vec<u8>, ptr: u64, next: u64, prev: u64, name: &str, limit: i16, in_out: i16, default_ptr: u64, link_ptr: u64) {
    write_block(out, b"DATA", SDNA_IDX_BNODE_SOCKET, ptr, 1,
        &build_bnode_socket(next, prev, name, name, 3, 0x0044, limit, in_out, "NodeSocketShader", default_ptr, link_ptr));
}

fn write_vector_default(out: &mut Vec<u8>, ptr: u64, value: [f32; 3]) {
    let mut d = vec![0u8; BNSV_VECTOR_SIZE];
    write_f32(&mut d, 4, value[0]);
    write_f32(&mut d, 8, value[1]);
    write_f32(&mut d, 12, value[2]);
    write_f32(&mut d, 16, f32::from_bits(0xff7fffff));
    write_f32(&mut d, 20, f32::from_bits(0x7f7fffff));
    write_i32(&mut d, 24, 3);
    write_block(out, b"DATA", SDNA_IDX_BNSV_VECTOR, ptr, 1, &d);
}

fn write_color_default(out: &mut Vec<u8>, ptr: u64, value: [f32; 4]) {
    let mut d = vec![0u8; BNSV_RGBA_SIZE];
    for (idx, component) in value.iter().enumerate() {
        write_f32(&mut d, idx * 4, *component);
    }
    write_block(out, b"DATA", SDNA_IDX_BNSV_RGBA, ptr, 1, &d);
}

fn write_float_default(out: &mut Vec<u8>, ptr: u64, value: f32) {
    let mut d = vec![0u8; BNSV_FLOAT_SIZE];
    write_f32(&mut d, 4, value);
    write_f32(&mut d, 12, 1_000_000.0);
    write_block(out, b"DATA", SDNA_IDX_BNSV_FLOAT, ptr, 1, &d);
}

fn write_node_link(
    out: &mut Vec<u8>,
    ptr: u64,
    prev_ptr: u64,
    next_ptr: u64,
    from_node: u64,
    to_node: u64,
    from_socket: u64,
    to_socket: u64,
) {
    let mut d = vec![0u8; BNODE_LINK_SIZE];
    write_ptr(&mut d, 0, next_ptr);
    write_ptr(&mut d, 8, prev_ptr);
    write_ptr(&mut d, 16, from_node);
    write_ptr(&mut d, 24, to_node);
    write_ptr(&mut d, 32, from_socket);
    write_ptr(&mut d, 40, to_socket);
    write_i32(&mut d, 48, 2);
    write_block(out, b"DATA", SDNA_IDX_BNODE_LINK, ptr, 1, &d);
}

fn build_bnode_socket(
    next_ptr: u64,
    prev_ptr: u64,
    identifier: &str,
    name: &str,
    sock_type: i16,
    flag: i16,
    limit: i16,
    in_out: i16,
    idname: &str,
    default_value_ptr: u64,
    link_ptr: u64,
) -> Vec<u8> {
    let mut d = vec![0u8; BNODE_SOCKET_SIZE];
    write_ptr(&mut d, 0x000, next_ptr);
    write_ptr(&mut d, 0x008, prev_ptr);
    write_cstr_fixed(&mut d, 0x018, 64, identifier);
    write_cstr_fixed(&mut d, 0x058, 64, name);
    write_i16(&mut d, 0x0a0, sock_type);
    write_i16(&mut d, 0x0a2, flag);
    write_i16(&mut d, 0x0a4, limit);
    write_i16(&mut d, 0x0a6, in_out);
    write_cstr_fixed(&mut d, 0x0b0, 64, idname);
    write_ptr(&mut d, 0x0f0, default_value_ptr);
    write_ptr(&mut d, 0x190, link_ptr);
    d
}


/// Compress `.blend` bytes using Blender 5.x Zstandard format (alias of [`compress_blend_bytes`]).
///
/// Blender 5.1 uses standard single-frame zstd compression (magic `0x28 0xB5 0x2F 0xFD`).
/// The earlier "seekable frames" hypothesis was incorrect — Blender saves with standard zstd.
pub fn compress_blend_bytes_zstd(raw_blend: &[u8]) -> Vec<u8> {
    compress_blend_bytes(raw_blend)
}

/// Build a Library block (LI) for linking to an external .blend file.
///
/// Binary layout (1472 bytes total, SDNA #15, Blender 5.1-verified):
/// - ID struct: 0-407 (408 bytes)
/// - name[1024]: 408-1431 (1024 bytes, UTF-8, null-terminated — stores the filepath)
/// - flag: 1432-1433 (ushort)
/// - undo_runtime_tag: 1434-1435 (ushort)
/// - _pad[4]: 1436-1439 (4 bytes alignment)
/// - *archive_parent_library: 1440-1447 (8 bytes pointer, nullptr)
/// - *packedfile: 1448-1455 (8 bytes pointer, nullptr)
/// - *runtime: 1456-1463 (8 bytes pointer, always nullptr)
/// - *_pad2: 1464-1471 (8 bytes, always nullptr)
///
/// Filepath stored as relative path when possible, UTF-8 encoded.
pub fn build_library_block(lib_name: &str, filepath: &str) -> Vec<u8> {
    let mut buf = vec![0u8; LIBRARY_SIZE];
    
    // Offset 0-408: ID struct (embedded within Library)
    // ID.name is at offset 40 within this struct (char[258])
    write_id_name(&mut buf[0..408], "LI", lib_name);
    
    // Offset 408-1432: filepath[1024] UTF-8, null-terminated, zero-padded
    let filepath_bytes = filepath.as_bytes();
    let filepath_len = filepath_bytes.len().min(1023); // Max 1023 chars + null terminator
    
    if filepath_len > 0 {
        buf[408..(408 + filepath_len)].copy_from_slice(&filepath_bytes[0..filepath_len]);
    }
    // Null-terminate and zero-pad (already initialized to zeros)
    buf[408 + filepath_len] = 0;
    
    // Offset 1432-1434: flag (uint16, 0 = no special flags)
    write_u16(&mut buf, 1432, 0);
    
    // Offset 1434-1436: undo_runtime_tag (uint16, 0)
    write_u16(&mut buf, 1434, 0);
    
    // Offset 1436-1440: _pad (4 bytes, already zero)
    
    // Offset 1440-1448: archive_parent_library (8 bytes pointer, nullptr)
    write_ptr(&mut buf, 1440, 0);
    
    // Offset 1448-1456: packedfile (8 bytes pointer, nullptr)
    write_ptr(&mut buf, 1448, 0);
    
    // Offset 1456-1464: runtime (8 bytes pointer, always nullptr)
    write_ptr(&mut buf, 1456, 0);
    
    buf
}

/// Build an ID stub (ID) for referencing a datablock from an external library.
///
/// The ID stub is embedded in the Object.data pointer and links to:
/// - A Library block (LI) specifying the external .blend file
/// - A specific datablock within that library (e.g., Mesh)
pub fn build_id_stub(datablock_type: &str, name: &str, lib_ptr: u64) -> Vec<u8> {
    let mut buf = vec![0u8; ID_STUB_SIZE];
    
    // ID.name at offset 40 within the ID struct
    write_id_name(&mut buf, datablock_type, name);
    
    // Library pointer at offset 24
    write_ptr(&mut buf, 24, lib_ptr);
    
    buf
}

// ── Modifier builders ─────────────────────────────────────────────────────────

/// Write the 120-byte `ModifierData` base into the start of `data`.
///
/// Layout (SDNA-verified, Blender 5.1):
/// - +0:  `*next` (ptr)
/// - +8:  `*prev` (ptr)
/// - +16: `type`  (int, eModifierType)
/// - +20: `mode`  (int, eModifierMode bitmask)
/// - +40: `name[64]` (char)
fn write_modifier_base(data: &mut Vec<u8>, next_ptr: u64, prev_ptr: u64, mod_type: i32, name: &str) {
    write_ptr(data, 0, next_ptr);
    write_ptr(data, 8, prev_ptr);
    write_i32(data, 16, mod_type);
    write_i32(data, 20, MODIFIER_MODE_DEFAULT);
    write_cstr_fixed(data, 40, 64, name);
}

/// Build a `DisplaceModifierData` block (368 bytes).
///
/// Layout (SDNA #362, Blender 5.1):
/// - +0..+119: `ModifierData` base
/// - +280: `strength` (float)
/// - +284: `direction` (int) — 3 = NORMAL
/// - +288: `defgrp_name[64]` (char)
/// - +352: `midlevel` (float)
/// - +356: `space` (int) — 0 = LOCAL
pub fn build_displace_modifier(
    name: &str,
    next_ptr: u64,
    prev_ptr: u64,
    strength: f32,
    vertex_group_name: &str,
    midlevel: f32,
) -> Vec<u8> {
    let mut data = vec![0u8; DISPLACE_MODIFIER_SIZE];
    write_modifier_base(&mut data, next_ptr, prev_ptr, MODIFIER_TYPE_DISPLACE, name);
    write_f32(&mut data, 280, strength);
    write_i32(&mut data, 284, DISPLACE_DIRECTION_NORMAL);
    write_cstr_fixed(&mut data, 288, 64, vertex_group_name);
    write_f32(&mut data, 352, midlevel);
    write_i32(&mut data, 356, DISPLACE_SPACE_LOCAL);
    data
}

/// Build a `WeldModifierData` block (192 bytes).
///
/// Merges nearby vertices during evaluation.
/// Layout (SDNA #406, Blender 5.1):
/// - +0..+119: `ModifierData` base
/// - +120: `merge_dist` (float)
/// - +124: `defgrp_name[64]` (char) — empty = no vertex group restriction
/// - +188: `mode` (char) — 0 = ALL
pub fn build_weld_modifier(name: &str, next_ptr: u64, prev_ptr: u64, merge_dist: f32) -> Vec<u8> {
    let mut data = vec![0u8; WELD_MODIFIER_SIZE];
    write_modifier_base(&mut data, next_ptr, prev_ptr, MODIFIER_TYPE_WELD, name);
    write_f32(&mut data, 120, merge_dist);
    // defgrp_name[64] at +124: all zeros = no vertex group restriction
    // mode at +188: 0 = ALL (default, already zero)
    data
}

/// Build a `WeightedNormalModifierData` block (192 bytes).
///
/// Applies weighted normals using face area weighting.
/// Layout (SDNA #413, Blender 5.1):
/// - +0..+119: `ModifierData` base
/// - +120: `defgrp_name[64]` (char) — empty = no vertex group restriction
/// - +184: `mode` (char) — 0 = FACE_AREA
/// - +185: `flag` (char)
/// - +186: `weight` (short, i16)
/// - +188: `thresh` (float)
pub fn build_weighted_normal_modifier(
    name: &str,
    next_ptr: u64,
    prev_ptr: u64,
    weight: i16,
    thresh: f32,
) -> Vec<u8> {
    let mut data = vec![0u8; WEIGHTED_NORMAL_MODIFIER_SIZE];
    write_modifier_base(&mut data, next_ptr, prev_ptr, MODIFIER_TYPE_WEIGHTED_NORMAL, name);
    // defgrp_name[64] at +120: all zeros = no vertex group restriction
    // mode at +184: 0 = FACE_AREA (default, already zero)
    write_i16(&mut data, 186, weight);
    write_f32(&mut data, 188, thresh);
    data
}

/// Patch `Object.modifiers` ListBase at offsets 656/664.
///
/// `first_ptr` points to the first modifier in the chain; `last_ptr` to the last.
/// Each modifier's `next`/`prev` pointers form the doubly-linked list.
pub fn set_object_modifiers_listbase(data: &mut Vec<u8>, first_ptr: u64, last_ptr: u64) {
    write_ptr(data, 656, first_ptr);
    write_ptr(data, 664, last_ptr);
}


#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_zstd_compression_roundtrip() {
        use std::io::Read;

        let original = b"Hello, Blender 5.1!";
        let compressed = compress_blend_bytes_zstd(original);
        assert!(!compressed.is_empty(), "Compression produced empty result");
        assert_eq!(&compressed[..4], &[0x28, 0xB5, 0x2F, 0xFD]);

        let mut decoder = zstd::Decoder::new(compressed.as_slice())
            .expect("compressed blend bytes should be a valid zstd frame");
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .expect("compressed blend bytes should round-trip");
        assert_eq!(decompressed, original);
    }

    #[test]
    fn test_build_displace_modifier_writes_decal_offset_fields() {
        let bytes = build_displace_modifier(
            "StarBreaker Decal Offset",
            0x2222,
            0x1111,
            0.005,
            "starbreaker_decal_offset",
            0.0,
        );

        assert_eq!(bytes.len(), DISPLACE_MODIFIER_SIZE);
        assert_eq!(u64::from_le_bytes(bytes[0..8].try_into().unwrap()), 0x2222);
        assert_eq!(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), 0x1111);
        assert_eq!(i32::from_le_bytes(bytes[16..20].try_into().unwrap()), MODIFIER_TYPE_DISPLACE);
        assert_eq!(i32::from_le_bytes(bytes[20..24].try_into().unwrap()), MODIFIER_MODE_DEFAULT);
        assert_eq!(
            &bytes[40..40 + "StarBreaker Decal Offset".len()],
            b"StarBreaker Decal Offset"
        );
        assert!((f32::from_le_bytes(bytes[280..284].try_into().unwrap()) - 0.005).abs() < 0.000001);
        assert_eq!(i32::from_le_bytes(bytes[284..288].try_into().unwrap()), DISPLACE_DIRECTION_NORMAL);
        assert_eq!(
            &bytes[288..288 + "starbreaker_decal_offset".len()],
            b"starbreaker_decal_offset"
        );
        assert_eq!(f32::from_le_bytes(bytes[352..356].try_into().unwrap()), 0.0);
        assert_eq!(i32::from_le_bytes(bytes[356..360].try_into().unwrap()), DISPLACE_SPACE_LOCAL);
    }
    
    #[test]
    fn test_build_lamp_accepts_temperature_parameters() {
        // Test 4: build_lamp accepts temperature_k and use_temperature parameters
        let lamp_bytes = build_lamp(
            "TestLight",
            0,  // POINT
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            5.0,
            0.0,
            0.0,
            5000.0,  // temperature_k
            true,    // use_temperature
        );
        
        // Verify output size
        assert_eq!(lamp_bytes.len(), LAMP_SIZE, "Lamp block size mismatch");
    }
    
    #[test]
    fn test_build_lamp_temperature_written_correctly() {
        // Temperature is stored as f32 at offset 436 of the Lamp struct.
        let temp_value = 6500.0;
        let lamp_bytes = build_lamp(
            "TestLight",
            0,
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            5.0,
            0.0,
            0.0,
            temp_value,
            false,
        );

        let read_temp = f32::from_le_bytes(lamp_bytes[436..440].try_into().unwrap());
        assert!((read_temp - temp_value).abs() < 0.01,
            "Temperature not written correctly: expected {}, got {}", temp_value, read_temp);
    }

    #[test]
    fn test_build_lamp_use_temperature_true() {
        // use_temperature is encoded as bit 24 of the `mode` u32 at offset 420 (LA_USE_TEMPERATURE).
        let lamp_bytes = build_lamp(
            "TestLight",
            0,
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            5.0,
            0.0,
            0.0,
            5000.0,
            true,
        );

        let mode = u32::from_le_bytes(lamp_bytes[420..424].try_into().unwrap());
        assert!(mode & (1 << 24) != 0, "LA_USE_TEMPERATURE bit not set when use_temperature=true");
    }

    #[test]
    fn test_build_lamp_use_temperature_false() {
        let lamp_bytes = build_lamp(
            "TestLight",
            0,
            [1.0, 1.0, 1.0],
            100.0,
            5.0,
            5.0,
            0.0,
            0.0,
            5000.0,
            false,
        );

        let mode = u32::from_le_bytes(lamp_bytes[420..424].try_into().unwrap());
        assert!(mode & (1 << 24) == 0, "LA_USE_TEMPERATURE bit set when use_temperature=false");
    }
    
    #[test]
    fn test_build_lamp_temperature_multiple_values() {
        // Multiple temperature values exercise the same offset/bit pair as the
        // dedicated tests above.
        let test_values = vec![
            (2700.0, true),
            (3000.0, false),
            (5000.0, true),
            (9000.0, false),
            (12000.0, true),
        ];

        for (temp, use_temp) in test_values {
            let lamp_bytes = build_lamp(
                &format!("Light_{}", temp as i32),
                0,
                [1.0, 1.0, 1.0],
                100.0,
                5.0,
                5.0,
                0.0,
                0.0,
                temp,
                use_temp,
            );

            let read_temp = f32::from_le_bytes(lamp_bytes[436..440].try_into().unwrap());
            assert!((read_temp - temp).abs() < 0.01,
                "Temperature mismatch for {}: expected {}, got {}", temp, temp, read_temp);

            let mode = u32::from_le_bytes(lamp_bytes[420..424].try_into().unwrap());
            let read_use_temp = (mode & (1 << 24)) != 0;
            assert_eq!(read_use_temp, use_temp,
                "use_temperature bit mismatch for {}: expected {}, got {}", temp, use_temp, read_use_temp);
        }
    }

    // ── Block Header Compliance Tests (Blender 5.x 32-byte format) ───────────

    #[test]
    fn test_block_header_size_is_exactly_32_bytes() {
        let mut buf = Vec::new();
        let code = *b"DNA1";
        write_block_header(&mut buf, &code, 0, 0x0100, 0, 0);
        
        assert_eq!(
            buf.len(),
            32,
            "Block header MUST be exactly 32 bytes (Blender 5.x format), got {}",
            buf.len()
        );
    }

    #[test]
    fn test_block_header_code_is_4_bytes() {
        let mut buf = Vec::new();
        let code = *b"OB00";
        write_block_header(&mut buf, &code, 0, 0, 0, 0);
        
        // Verify bytes 0-3 match code
        assert_eq!(&buf[0..4], b"OB00", "Block code must be exactly 4 ASCII bytes at offset 0-3");
    }

    #[test]
    fn test_block_header_sdna_index_u32_little_endian() {
        let mut buf = Vec::new();
        let code = *b"SC00";
        let sdna_idx = 0x12345678u32;
        write_block_header(&mut buf, &code, sdna_idx, 0, 0, 0);
        
        // Verify bytes 4-7 contain u32 in little-endian
        let read_sdna = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        assert_eq!(
            read_sdna, sdna_idx,
            "SDNA index at bytes 4-7 must be u32 little-endian: expected 0x{:08x}, got 0x{:08x}",
            sdna_idx, read_sdna
        );
    }

    #[test]
    fn test_block_header_old_ptr_u64_little_endian() {
        let mut buf = Vec::new();
        let code = *b"ME00";
        let old_ptr = 0x0102030405060708u64;
        write_block_header(&mut buf, &code, 0, old_ptr, 0, 0);
        
        // Verify bytes 8-15 contain u64 in little-endian
        let mut ptr_bytes = [0u8; 8];
        ptr_bytes.copy_from_slice(&buf[8..16]);
        let read_ptr = u64::from_le_bytes(ptr_bytes);
        assert_eq!(
            read_ptr, old_ptr,
            "Old pointer at bytes 8-15 must be u64 little-endian: expected 0x{:016x}, got 0x{:016x}",
            old_ptr, read_ptr
        );
    }

    #[test]
    fn test_block_header_data_length_u32_little_endian() {
        let mut buf = Vec::new();
        let code = *b"DATA";
        let data_len = 0xdeadbeefu32;
        write_block_header(&mut buf, &code, 0, 0, data_len, 0);
        
        // Verify bytes 16-19 contain u32 in little-endian
        let read_len = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        assert_eq!(
            read_len, data_len,
            "Data length at bytes 16-19 must be u32 little-endian: expected 0x{:08x}, got 0x{:08x}",
            data_len, read_len
        );
    }

    #[test]
    fn test_block_header_padding1_is_zero() {
        let mut buf = Vec::new();
        let code = *b"LA00";
        write_block_header(&mut buf, &code, 0xffffffff, 0xffffffffffffffff, 0xffffffff, 0xffffffff);
        
        // Verify bytes 20-23 are EXACTLY zero (not garbage or uninitialized)
        assert_eq!(&buf[20..24], &[0, 0, 0, 0], 
            "Padding field 1 at bytes 20-23 MUST be exactly zero (not uninitialized)");
    }

    #[test]
    fn test_block_header_count_u32_little_endian() {
        let mut buf = Vec::new();
        let code = *b"GR00";
        let count = 0xabcdef01u32;
        write_block_header(&mut buf, &code, 0, 0, 0, count);
        
        // Verify bytes 24-27 contain u32 in little-endian
        let read_count = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
        assert_eq!(
            read_count, count,
            "Count field at bytes 24-27 must be u32 little-endian: expected 0x{:08x}, got 0x{:08x}",
            count, read_count
        );
    }

    #[test]
    fn test_block_header_padding2_is_zero() {
        let mut buf = Vec::new();
        let code = *b"ENDB";
        write_block_header(&mut buf, &code, 0xffffffff, 0xffffffffffffffff, 0xffffffff, 0xffffffff);
        
        // Verify bytes 28-31 are EXACTLY zero (not garbage or uninitialized)
        assert_eq!(&buf[28..32], &[0, 0, 0, 0], 
            "Padding field 2 at bytes 28-31 MUST be exactly zero (not uninitialized)");
    }

    #[test]
    fn test_block_header_dna1_block_normal() {
        // Normal DNA1 block: code="DNA1", typical indices and counts
        let mut buf = Vec::new();
        let code = *b"DNA1";
        let sdna_idx = SDNA_IDX_DNA1;
        let old_ptr = 0x0100;
        let data_len = 2048; // typical DNA1 size
        let count = 1;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        // Verify full header layout
        assert_eq!(&buf[0..4], b"DNA1");
        assert_eq!(u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]), sdna_idx);
        assert_eq!(u64::from_le_bytes([buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15]]), old_ptr);
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), data_len);
        assert_eq!(&buf[20..24], &[0, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), count);
        assert_eq!(&buf[28..32], &[0, 0, 0, 0]);
    }

    #[test]
    fn test_block_header_object_block_zero_count() {
        // Zero-count block: count=0 (e.g., empty collection or DATA block)
        let mut buf = Vec::new();
        let code = *b"OB00";
        let sdna_idx = SDNA_IDX_OBJECT;
        let old_ptr = 0x0200;
        let data_len = 0;
        let count = 0;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        assert_eq!(&buf[0..4], b"OB00");
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), 0, "Count must be 0");
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), 0, "Data length must be 0");
    }

    #[test]
    fn test_block_header_large_data_length() {
        // Large data length block: max u32 value
        let mut buf = Vec::new();
        let code = *b"DATA";
        let sdna_idx = 160; // CUSTOM_DATA_LAYER
        let old_ptr = 0x0300;
        let data_len = 0xffffffffu32; // max u32
        let count = 1000000;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), data_len, 
            "Data length must support max u32 value");
    }

    #[test]
    fn test_block_header_max_count() {
        // Max count block: count=0xffffffff
        let mut buf = Vec::new();
        let code = *b"SC00";
        let sdna_idx = SDNA_IDX_SCENE;
        let old_ptr = 0x0400;
        let data_len = 65536;
        let count = 0xffffffffu32;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), count, 
            "Count field must support max u32 value");
    }

    #[test]
    fn test_block_header_all_real_block_codes() {
        // Test all documented block codes from spec
        let block_specs = vec![
            (*b"GLOB", SDNA_IDX_FILE_GLOBAL, 0x0100, 1216, 1),
            (*b"SC\0\0", SDNA_IDX_SCENE, 0x0200, 6920, 1),
            (*b"OB\0\0", SDNA_IDX_OBJECT, 0x0300, 800, 1),
            (*b"ME\0\0", SDNA_IDX_MESH, 0x0400, 4096, 1),
            (*b"LA\0\0", SDNA_IDX_LAMP, 0x0500, 1200, 1),
            (*b"GR\0\0", SDNA_IDX_COLLECTION, 0x0600, 520, 1),
            (*b"DATA", 160, 0x0700, 2048, 1),
            (*b"DNA1", SDNA_IDX_DNA1, 0x0800, 2048, 1),
            (*b"ENDB", 0, 0x0900, 0, 0),
        ];
        
        for (code, sdna_idx, old_ptr, data_len, count) in block_specs {
            let mut buf = Vec::new();
            write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
            
            assert_eq!(buf.len(), 32, "Block code {:?} header must be 32 bytes", 
                String::from_utf8_lossy(&code));
            assert_eq!(&buf[0..4], &code, "Block code mismatch");
            assert_eq!(&buf[20..24], &[0, 0, 0, 0], "Padding 1 must be zero for {:?}", 
                String::from_utf8_lossy(&code));
            assert_eq!(&buf[28..32], &[0, 0, 0, 0], "Padding 2 must be zero for {:?}", 
                String::from_utf8_lossy(&code));
        }
    }

    #[test]
    fn test_block_header_byte_layout_complete() {
        // Complete byte-by-byte layout verification
        let mut buf = Vec::new();
        let code = *b"TEST";
        let sdna_idx = 0x11223344u32;
        let old_ptr = 0x1122334455667788u64;
        let data_len = 0x99aabbccu32;
        let count = 0xddeeff00u32;
        
        write_block_header(&mut buf, &code, sdna_idx, old_ptr, data_len, count);
        
        // Verify complete layout:
        // bytes 0-3: code
        assert_eq!(&buf[0..4], b"TEST");
        
        // bytes 4-7: sdna_idx (u32 LE)
        assert_eq!(&buf[4..8], &[0x44, 0x33, 0x22, 0x11]);
        
        // bytes 8-15: old_ptr (u64 LE)
        assert_eq!(&buf[8..16], &[0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]);
        
        // bytes 16-19: data_len (u32 LE)
        assert_eq!(&buf[16..20], &[0xcc, 0xbb, 0xaa, 0x99]);
        
        // bytes 20-23: padding (ZERO)
        assert_eq!(&buf[20..24], &[0x00, 0x00, 0x00, 0x00]);
        
        // bytes 24-27: count (u32 LE)
        assert_eq!(&buf[24..28], &[0x00, 0xff, 0xee, 0xdd]);
        
        // bytes 28-31: padding (ZERO)
        assert_eq!(&buf[28..32], &[0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_write_block_with_data() {
        // Test write_block function that wraps write_block_header
        let mut buf = Vec::new();
        let code = *b"DATA";
        let sdna_idx = 160;
        let old_ptr = 0x1000;
        let count = 2;
        let data = b"Hello, Blender!";
        
        write_block(&mut buf, &code, sdna_idx, old_ptr, count, data);
        
        // Should be header (32 bytes) + data
        assert_eq!(buf.len(), 32 + data.len(), "Block size should be header + data");
        
        // Verify header portion
        assert_eq!(&buf[0..4], b"DATA");
        assert_eq!(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]), data.len() as u32,
            "Data length in header must match actual data size");
        assert_eq!(u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]), count,
            "Count field must match provided count");
        
        // Verify data portion
        assert_eq!(&buf[32..32 + data.len()], data);
    }

    #[test]
    fn test_block_header_boundary_values() {
        // Test boundary values for u32 fields (SDNA index is critical fix)
        let boundary_cases = vec![
            (0u32, "zero"),
            (1u32, "one"),
            (0x7fffffffu32, "max signed i32"),
            (0x80000000u32, "min signed i32 as u32 (critical fix)"),
            (0xffffffffu32, "max u32"),
        ];
        
        for (value, description) in boundary_cases {
            let mut buf = Vec::new();
            let code = *b"TEST";
            
            // Test SDNA index field
            write_block_header(&mut buf, &code, value, 0, 0, 0);
            let read_value = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            assert_eq!(read_value, value, "SDNA index boundary case failed for {}: expected 0x{:08x}, got 0x{:08x}", 
                description, value, read_value);
            
            // Test data length field
            buf.clear();
            write_block_header(&mut buf, &code, 0, 0, value, 0);
            let read_value = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
            assert_eq!(read_value, value, "Data length boundary case failed for {}: expected 0x{:08x}, got 0x{:08x}", 
                description, value, read_value);
            
            // Test count field
            buf.clear();
            write_block_header(&mut buf, &code, 0, 0, 0, value);
            let read_value = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
            assert_eq!(read_value, value, "Count boundary case failed for {}: expected 0x{:08x}, got 0x{:08x}", 
                description, value, read_value);
        }
    }
}
