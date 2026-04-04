use starbreaker_common::ParseError;
use starbreaker_common::reader::SpanReader;
use zerocopy::{FromBytes, Immutable, KnownLayout};

// ── Raw on-disk layouts ─────────────────────────────────────────────────────

/// Raw MeshInfo as stored on disk (76 bytes).
#[derive(Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
struct RawMeshInfo {
    flags2: u32,
    num_vertices: u32,
    num_indices: u32,
    num_submeshes: u32,
    _unknown: u32,
    model_min: [f32; 3],
    model_max: [f32; 3],
    min_bound: [f32; 3],
    max_bound: [f32; 3],
    _vertex_format: u32,
    extra_count: u32,
}

/// Raw SubMeshDescriptor as stored on disk (48 bytes).
#[derive(Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
struct RawSubMeshDescriptor {
    mat_id: u16,
    node_parent_index: u16,
    first_index: u32,
    num_indices: u32,
    first_vertex: u32,
    page_base: u32,
    num_vertices: u32,
    radius: f32,
    center: [f32; 3],
    _unknown0: u32,
    _unknown1: u32,
}

/// Quantized vertex: SNorm i16×3 + pad + RGBA + UV half×2 (16 bytes).
#[derive(Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
struct RawQuantizedVertex {
    pos: [u16; 3],
    _pad: u16,
    color: [u8; 4],
    uv: [u16; 2],
}

/// Float vertex: f32×3 + RGBA + UV half×2 (20 bytes).
#[derive(Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
struct RawFloatVertex {
    pos: [f32; 3],
    color: [u8; 4],
    uv: [u16; 2],
}

const _: () = {
    assert!(size_of::<RawMeshInfo>() == 76);
    assert!(size_of::<RawSubMeshDescriptor>() == 48);
    assert!(size_of::<RawQuantizedVertex>() == 16);
    assert!(size_of::<RawFloatVertex>() == 20);
};

/// Datastream type tag constants.
///
/// Names are from cryengine-converter/tmlaw templates. Some are misleading:
/// - IVOQTANGENTS is NOT quaternions — it's compressed tangent vectors (15-15-1-1 bits)
/// - IVONORMALS2 is dual-purpose: elem_size=12 → f32×3 normals, elem_size=4 → second UV set
///   (elem_size=4 maps to D3D SECOND_UV semantic, not normals)
/// - IVOTANGENTS IS quaternions — i16 SNorm XYZW (CryEngine SPipQTangents format)
pub mod stream_type {
    pub const IVOVERTSUVS: u32 = 0x91329AE9;
    pub const IVOVERTSUVS2: u32 = 0xB3A70D5E;
    pub const IVOINDICES: u32 = 0xEECDC168;
    pub const IVONORMALS: u32 = 0x9CF3F615;
    /// Dual-purpose stream: elem_size=12 → f32×3 normals, elem_size=4 → SECOND_UV (half×2).
    /// Name inherited from cryengine-converter; elem_size=4 variant is actually SECOND_UV
    /// per D3D semantic mapping (SECOND_UV → slot 0xe, flag 0x40).
    pub const IVONORMALS2: u32 = 0x38A581FE;
    pub const IVOTANGENTS: u32 = 0xB95E9A1B;
    pub const IVOQTANGENTS: u32 = 0xEE057252;
    pub const IVOBONEMAP: u32 = 0x677C7B23;
    pub const IVOBONEMAP32: u32 = 0x6ECA3708;
    /// Simple bone mapping: single u16 bone index per vertex, weight implied 1.0.
    /// Used for rigid attachment meshes where each vertex is influenced by one bone.
    pub const IVOSIMPLEBONEMAP: u32 = 0x9D51C5EE;
    pub const IVOCOLORS2: u32 = 0xD9EED421;
}

#[derive(Debug, Clone)]
pub struct MeshInfo {
    pub flags2: u32,
    pub num_vertices: u32,
    pub num_indices: u32,
    pub num_submeshes: u32,
    /// First bounding box — original model-space extent.
    /// NMC scene graph transforms are in this coordinate system.
    pub model_min: [f32; 3],
    pub model_max: [f32; 3],
    /// Second bounding box ("scaling bbox") — the correct extent for SNorm dequantization.
    pub min_bound: [f32; 3],
    pub max_bound: [f32; 3],
    pub extra_count: u32,
}

#[derive(Debug, Clone)]
pub struct SubMeshDescriptor {
    /// Material index into the MTL submaterial list.
    pub mat_id: u16,
    /// Node index in the NodeMeshCombo (which node this submesh belongs to).
    pub node_parent_index: u16,
    pub first_index: u32,
    pub num_indices: u32,
    pub first_vertex: u32,
    /// Index page base for u16 index addressing on meshes with >65535 vertices.
    /// Add to each index in this submesh's range to get the absolute vertex index.
    pub page_base: u32,
    pub num_vertices: u32,
    pub radius: f32,
    pub center: [f32; 3],
}

/// Per-vertex bone mapping: 4 bone indices + 4 weights.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BoneMapping {
    pub bone_indices: [u16; 4],
    pub weights: [f32; 4],
}

#[derive(Debug)]
pub struct DataStreams {
    pub positions: PositionData,
    pub uvs: Vec<[u16; 2]>,
    pub indices: Vec<u32>,
    pub colors: Option<Vec<[u8; 4]>>,
    pub tangents: Option<TangentData>,
    pub normals: Option<NormalData>,
    pub bone_mappings: Option<Vec<BoneMapping>>,
}

/// Raw normal data — format determined by which stream was present.
#[derive(Debug)]
pub enum NormalData {
    /// IVONORMALS — single u32 packed unit vector (15-15-1-1 bit layout).
    /// Same encoding as IVOQTANGENTS individual vectors.
    Packed(Vec<u32>),
    /// IVONORMALS2 — 3× f32
    Float(Vec<[f32; 3]>),
}

/// Raw tangent data — format determined by which stream was present.
#[derive(Debug)]
pub enum TangentData {
    /// IVOQTANGENTS — 4× i16 SNorm quaternion (XYZW) OR compressed tangent vectors (2× u32).
    /// Actual format depends on whether IVONORMALS2 is also present:
    ///   - With IVONORMALS2: compressed tangent vectors (decode_compressed_tangent)
    ///   - Without IVONORMALS2: QTangent quaternion (decode_qtangent_snorm)
    QTangents(Vec<[u16; 4]>),
    /// IVOTANGENTS — SNorm i16 quaternion (same decode as QTangent)
    Tangents(Vec<[u16; 4]>),
}

#[derive(Debug)]
pub enum PositionData {
    Quantized(Vec<[u16; 3]>),
    Float(Vec<[f32; 3]>),
}

#[derive(Debug)]
pub struct SkinMesh {
    pub flags: u32,
    pub info: MeshInfo,
    pub submeshes: Vec<SubMeshDescriptor>,
    pub streams: DataStreams,
}

impl MeshInfo {
    /// Parse MeshInfo from a SpanReader (reads 76 bytes).
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let raw = reader.read_type::<RawMeshInfo>()?;
        Ok(Self {
            flags2: raw.flags2,
            num_vertices: raw.num_vertices,
            num_indices: raw.num_indices,
            num_submeshes: raw.num_submeshes,
            model_min: raw.model_min,
            model_max: raw.model_max,
            min_bound: raw.min_bound,
            max_bound: raw.max_bound,
            extra_count: raw.extra_count,
        })
    }
}

impl SubMeshDescriptor {
    fn from_raw(raw: &RawSubMeshDescriptor) -> Self {
        Self {
            mat_id: raw.mat_id,
            node_parent_index: raw.node_parent_index,
            first_index: raw.first_index,
            num_indices: raw.num_indices,
            first_vertex: raw.first_vertex,
            page_base: raw.page_base,
            num_vertices: raw.num_vertices,
            radius: raw.radius,
            center: raw.center,
        }
    }
}

use crate::error::Error;

impl SkinMesh {
    pub fn read(data: &[u8]) -> Result<Self, Error> {
        let mut reader = SpanReader::new(data);

        // Header: 4 bytes flags + 76 bytes MeshInfo + 88 bytes padding = 168
        let flags = reader.read_u32()?;
        let info = MeshInfo::read(&mut reader)?;
        reader.advance(88)?;

        // Submesh descriptors
        let raw_submeshes =
            reader.read_slice::<RawSubMeshDescriptor>(info.num_submeshes as usize)?;
        let submeshes: Vec<SubMeshDescriptor> =
            raw_submeshes.iter().map(SubMeshDescriptor::from_raw).collect();

        // Skip extra data after submeshes (determined by extra_count field in MeshInfo)
        reader.advance(info.extra_count as usize * 4)?;

        // Datastreams
        let streams = Self::read_streams(&mut reader, &info)?;

        Ok(Self {
            flags,
            info,
            submeshes,
            streams,
        })
    }

    fn read_streams(reader: &mut SpanReader, info: &MeshInfo) -> Result<DataStreams, Error> {
        let mut positions: Option<PositionData> = None;
        let mut uvs: Option<Vec<[u16; 2]>> = None;
        let mut indices: Option<Vec<u32>> = None;
        let mut colors: Option<Vec<[u8; 4]>> = None;
        let mut bone_mappings: Option<Vec<BoneMapping>> = None;
        let mut tangents: Option<TangentData> = None;
        let mut normals: Option<NormalData> = None;

        let num_verts = info.num_vertices as usize;
        let num_idx = info.num_indices as usize;

        while reader.remaining() >= 4 {
            let stream_type = reader.read_u32()?;

            // Zero-tag: inter-stream alignment padding, skip it
            if stream_type == 0 {
                continue;
            }

            // All real streams have an element_size u32 after the tag
            if reader.remaining() < 4 {
                break;
            }
            let element_size = reader.read_u32()?;

            // Record position before reading stream data (for alignment)
            let stream_data_start = reader.position();

            if std::env::var("SB_DEBUG_STREAMS").is_ok() {
                let name = match stream_type {
                    stream_type::IVOVERTSUVS => "IVOVERTSUVS",
                    stream_type::IVOVERTSUVS2 => "IVOVERTSUVS2",
                    stream_type::IVOINDICES => "IVOINDICES",
                    stream_type::IVONORMALS => "IVONORMALS",
                    stream_type::IVONORMALS2 => "IVONORMALS2",
                    stream_type::IVOTANGENTS => "IVOTANGENTS",
                    stream_type::IVOQTANGENTS => "IVOQTANGENTS",
                    stream_type::IVOBONEMAP => "IVOBONEMAP",
                    stream_type::IVOBONEMAP32 => "IVOBONEMAP32",
                    stream_type::IVOSIMPLEBONEMAP => "IVOSIMPLEBONEMAP",
                    stream_type::IVOCOLORS2 => "IVOCOLORS2",
                    _ => "UNKNOWN",
                };
                eprintln!("  stream 0x{stream_type:08X} ({name}): elem_size={element_size}, count={num_verts}");
            }

            match stream_type {
                stream_type::IVOVERTSUVS | stream_type::IVOVERTSUVS2 => {
                    // element_size determines layout:
                    // 16 = SNorm i16×3 positions + pad(2) + RGBA(4) + UV half×2(4)
                    // 20 = float f32×3 positions + RGBA(4) + UV half×2(4)
                    if element_size == 16 {
                        let verts = reader.read_slice::<RawQuantizedVertex>(num_verts)?;
                        positions = Some(PositionData::Quantized(
                            verts.iter().map(|v| v.pos).collect(),
                        ));
                        colors = Some(verts.iter().map(|v| v.color).collect());
                        uvs = Some(verts.iter().map(|v| v.uv).collect());
                    } else if element_size == 20 {
                        let verts = reader.read_slice::<RawFloatVertex>(num_verts)?;
                        positions = Some(PositionData::Float(
                            verts.iter().map(|v| v.pos).collect(),
                        ));
                        colors = Some(verts.iter().map(|v| v.color).collect());
                        uvs = Some(verts.iter().map(|v| v.uv).collect());
                    } else {
                        return Err(Error::UnexpectedElementSize {
                            expected: 16,
                            got: element_size,
                        });
                    }
                }
                stream_type::IVOINDICES => {
                    if element_size == 2 {
                        let raw = reader.read_slice::<u16>(num_idx)?;
                        indices = Some(raw.iter().map(|&i| i as u32).collect());
                    } else {
                        indices = Some(reader.read_slice::<u32>(num_idx)?.to_vec());
                    }
                }
                stream_type::IVONORMALS => {
                    normals = Some(NormalData::Packed(
                        reader.read_slice::<u32>(num_verts)?.to_vec(),
                    ));
                }
                stream_type::IVONORMALS2 => {
                    if element_size == 12 {
                        normals = Some(NormalData::Float(
                            reader.read_slice::<[f32; 3]>(num_verts)?.to_vec(),
                        ));
                    } else {
                        // elem_size=4: second UV set, not normals. Skip.
                        reader.advance(element_size as usize * num_verts)?;
                    }
                }
                stream_type::IVOQTANGENTS => {
                    let raw = reader.read_slice::<[u16; 4]>(num_verts)?;
                    if std::env::var("SB_DUMP_QTANGENTS").is_ok() {
                        for (i, v) in raw.iter().take(20).enumerate() {
                            eprintln!(
                                "QTANG[{i}]: 0x{:04X} 0x{:04X} 0x{:04X} 0x{:04X}",
                                v[0], v[1], v[2], v[3]
                            );
                        }
                    }
                    tangents = Some(TangentData::QTangents(raw.to_vec()));
                }
                stream_type::IVOTANGENTS => {
                    let raw = reader.read_slice::<[u16; 4]>(num_verts)?;
                    if std::env::var("SB_DEBUG_STREAMS").is_ok() {
                        for (i, v) in raw.iter().take(5).enumerate() {
                            eprintln!(
                                "    IVOTANGENTS[{i}]: 0x{:04X} 0x{:04X} 0x{:04X} 0x{:04X}",
                                v[0], v[1], v[2], v[3]
                            );
                        }
                    }
                    tangents = Some(TangentData::Tangents(raw.to_vec()));
                }
                stream_type::IVOBONEMAP => {
                    // elem_size=8: 4×u8 bone indices + 4×u8 weights
                    let mut mappings = Vec::with_capacity(num_verts);
                    for _ in 0..num_verts {
                        let raw = reader.read_slice::<u8>(element_size as usize)?;
                        mappings.push(BoneMapping {
                            bone_indices: [raw[0] as u16, raw[1] as u16, raw[2] as u16, raw[3] as u16],
                            weights: [
                                raw[4] as f32 / 255.0,
                                raw[5] as f32 / 255.0,
                                raw[6] as f32 / 255.0,
                                raw[7] as f32 / 255.0,
                            ],
                        });
                    }
                    bone_mappings = Some(mappings);
                }
                stream_type::IVOSIMPLEBONEMAP => {
                    // elem_size=2: single u16 bone index per vertex, weight = 1.0
                    let indices_raw = reader.read_slice::<u16>(num_verts)?;
                    let mappings = indices_raw.iter().map(|&idx| BoneMapping {
                        bone_indices: [idx, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    }).collect();
                    bone_mappings = Some(mappings);
                }
                stream_type::IVOBONEMAP32 => {
                    // elem_size=12: 4×u16 bone indices + 4×u8 weights
                    let mut mappings = Vec::with_capacity(num_verts);
                    for _ in 0..num_verts {
                        let indices_raw = reader.read_slice::<u16>(4)?;
                        let weights_raw = reader.read_slice::<u8>(4)?;
                        mappings.push(BoneMapping {
                            bone_indices: [indices_raw[0], indices_raw[1], indices_raw[2], indices_raw[3]],
                            weights: [
                                weights_raw[0] as f32 / 255.0,
                                weights_raw[1] as f32 / 255.0,
                                weights_raw[2] as f32 / 255.0,
                                weights_raw[3] as f32 / 255.0,
                            ],
                        });
                    }
                    bone_mappings = Some(mappings);
                }
                _ => {
                    let count = num_verts;
                    let skip_bytes = element_size as usize * count;
                    if reader.remaining() >= skip_bytes {
                        reader.advance(skip_bytes)?;
                    } else {
                        break;
                    }
                }
            }

            // Align to 8-byte boundary after each stream (matching cryengine-converter)
            let bytes_read = reader.position() - stream_data_start;
            let remainder = bytes_read % 8;
            if remainder != 0 {
                let pad = 8 - remainder;
                if reader.remaining() >= pad {
                    reader.advance(pad)?;
                }
            }
        }

        Ok(DataStreams {
            positions: positions.ok_or(Error::MissingChunk {
                chunk_type: stream_type::IVOVERTSUVS,
            })?,
            uvs: uvs.unwrap_or_default(),
            indices: indices.ok_or(Error::MissingChunk {
                chunk_type: stream_type::IVOINDICES,
            })?,
            colors,
            tangents,
            normals,
            bone_mappings,
        })
    }
}
