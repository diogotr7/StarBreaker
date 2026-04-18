use crate::dequant;
use crate::ivo::material::MaterialName;
use crate::ivo::skin::{NormalData, PositionData, SkinMesh, TangentData};

#[derive(Debug, Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub colors: Option<Vec<[u8; 4]>>,
    pub submeshes: Vec<SubMesh>,
    /// Model-space bounding box (first bbox from MeshInfo).
    /// NMC scene graph transforms are in this coordinate system.
    pub model_min: [f32; 3],
    pub model_max: [f32; 3],
    /// Scaling bounding box (second bbox from MeshInfo, used for dequantization).
    pub scaling_min: [f32; 3],
    pub scaling_max: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct SubMesh {
    pub material_name: Option<String>,
    pub material_id: u32,
    pub first_index: u32,
    pub num_indices: u32,
    pub first_vertex: u32,
    pub num_vertices: u32,
    pub node_parent_index: u16,
}

impl Mesh {
    /// Merge another mesh into this one, appending vertex data and reindexing.
    pub fn merge_from(&mut self, other: Mesh) {
        let vertex_offset = self.positions.len() as u32;
        let index_offset = self.indices.len() as u32;

        self.positions.extend(other.positions);
        if let (Some(a), Some(b)) = (&mut self.uvs, other.uvs) {
            a.extend(b);
        }
        if let (Some(a), Some(b)) = (&mut self.normals, other.normals) {
            a.extend(b);
        }
        if let (Some(a), Some(b)) = (&mut self.tangents, other.tangents) {
            a.extend(b);
        }
        // Merge vertex colors: pad with opaque white if one side is missing.
        match (&mut self.colors, other.colors) {
            (Some(a), Some(b)) => a.extend(b),
            (Some(a), None) => {
                // Other mesh has no colors — pad with opaque white
                a.resize(self.positions.len(), [255, 255, 255, 255]);
            }
            (None, Some(b)) => {
                // Self had no colors — backfill with opaque white, then append other's
                let mut padded = vec![[255u8, 255, 255, 255]; vertex_offset as usize];
                padded.extend(b);
                self.colors = Some(padded);
            }
            (None, None) => {}
        }

        self.indices
            .extend(other.indices.iter().map(|i| i + vertex_offset));

        for mut sm in other.submeshes {
            sm.first_index += index_offset;
            sm.first_vertex += vertex_offset;
            self.submeshes.push(sm);
        }

        for i in 0..3 {
            self.model_min[i] = self.model_min[i].min(other.model_min[i]);
            self.model_max[i] = self.model_max[i].max(other.model_max[i]);
            self.scaling_min[i] = self.scaling_min[i].min(other.scaling_min[i]);
            self.scaling_max[i] = self.scaling_max[i].max(other.scaling_max[i]);
        }
    }

}

/// Compute area-weighted smooth normals from geometry (fallback when stream normals unavailable).
fn compute_smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut norms = vec![[0.0f32; 3]; positions.len()];
    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            break;
        }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let fn_ = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &vi in &[i0, i1, i2] {
            norms[vi][0] += fn_[0];
            norms[vi][1] += fn_[1];
            norms[vi][2] += fn_[2];
        }
    }
    for n in &mut norms {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-8 {
            n[0] /= len;
            n[1] /= len;
            n[2] /= len;
        } else {
            *n = [0.0, 0.0, 1.0];
        }
    }
    norms
}

pub fn build_mesh(skin: &SkinMesh, materials: &[MaterialName]) -> Mesh {
    build_mesh_with_bbox(skin, materials, false)
}

/// Build mesh, optionally dequantizing with model bbox instead of scaling bbox.
/// Interior CGFs use model bbox because IncludedObjects placements are authored
/// for model-bbox space. The scaling bbox is expanded for GPU skinning across
/// NMC nodes and gives wrong vertex positions for placement.
pub fn build_mesh_with_bbox(skin: &SkinMesh, materials: &[MaterialName], use_model_bbox: bool) -> Mesh {
    let (dequant_min, dequant_max) = if use_model_bbox {
        (&skin.info.model_min, &skin.info.model_max)
    } else {
        (&skin.info.min_bound, &skin.info.max_bound)
    };
    let positions = match &skin.streams.positions {
        PositionData::Quantized(raw) => raw
            .iter()
            .map(|p| dequant::dequantize_position(*p, dequant_min, dequant_max))
            .collect(),
        PositionData::Float(f) => f.clone(),
    };

    let uvs: Option<Vec<[f32; 2]>> = if skin.streams.uvs.is_empty() {
        None
    } else {
        Some(
            skin.streams
                .uvs
                .iter()
                .map(|uv| dequant::decode_half2(*uv))
                .collect(),
        )
    };

    let submeshes = skin
        .submeshes
        .iter()
        .map(|s| SubMesh {
            material_name: materials.get(s.mat_id as usize).map(|m| m.name.clone()),
            material_id: s.mat_id as u32,
            first_index: s.first_index,
            num_indices: s.num_indices,
            first_vertex: s.first_vertex,
            num_vertices: s.page_base,
            node_parent_index: s.node_parent_index,
        })
        .collect();

    // Indices in IVO format are relative to a vertex page base.
    // For meshes with >65535 vertices, vertices are split into pages addressable by u16 indices.
    // The SubMeshDescriptor.num_vertices field is actually the page base offset (it's the
    // "Unknown" field in cryengine-converter's MeshSubset, positioned between FirstVertex
    // and the real NumVertices). Add the page base to make indices absolute.
    let mut indices = skin.streams.indices.clone();
    for s in &skin.submeshes {
        if s.page_base != 0 {
            let start = s.first_index as usize;
            let end = start + s.num_indices as usize;
            for idx in &mut indices[start..end] {
                *idx += s.page_base;
            }
        }
    }

    // Decode normals and tangents from stream data.
    // Priority for normals:
    //   1. IVONORMALS2 (f32×3) — direct, highest quality
    //   2. IVONORMALS (packed unit vector, 15-15-1-1 bit layout)
    //   3. QTangent/tangent stream → extract from rotation matrix
    //   4. Geometry-computed smooth normals (fallback)
    // Tangent vectors come from IVOQTANGENTS/IVOTANGENTS when available.
    let direct_normals: Option<Vec<[f32; 3]>> = match &skin.streams.normals {
        Some(NormalData::Float(data)) => Some(data.clone()),
        Some(NormalData::Packed(data)) => Some(
            data.iter().map(|&raw| dequant::decode_packed_unit_vector(raw)).collect(),
        ),
        None => None,
    };

    let tangent_decode: Option<Vec<dequant::NormalTangent>> = match &skin.streams.tangents {
        Some(TangentData::QTangents(data)) => {
            // IVOQTANGENTS: compressed tangent vectors (2× u32, 15-15-1-1 bit packing).
            // Despite the name, these are NOT quaternions — the game converts them at load time.
            Some(data.iter().map(|raw| dequant::decode_compressed_tangent(*raw)).collect())
        }
        Some(TangentData::Tangents(data)) => {
            // IVOTANGENTS: i16 SNorm quaternion (CryEngine SPipQTangents format)
            Some(data.iter().map(|raw| dequant::decode_qtangent_snorm(*raw)).collect())
        }
        None => None,
    };

    let normals: Option<Vec<[f32; 3]>> = direct_normals
        .or_else(|| tangent_decode.as_ref().map(|td| td.iter().map(|nt| nt.normal).collect()))
        .or_else(|| Some(compute_smooth_normals(&positions, &indices)));

    let tangents_out: Option<Vec<[f32; 4]>> =
        tangent_decode.map(|td| td.iter().map(|nt| nt.tangent).collect());

    Mesh {
        positions,
        indices,
        uvs,
        normals,
        tangents: tangents_out,
        colors: skin.streams.colors.clone(),
        submeshes,
        model_min: skin.info.model_min,
        model_max: skin.info.model_max,
        scaling_min: skin.info.min_bound,
        scaling_max: skin.info.max_bound,
    }
}

/// Loaded texture data for glTF embedding, indexed by submaterial.
pub struct MaterialTextures {
    /// Per-submaterial: Some(png_bytes) for diffuse texture, None if missing.
    pub diffuse: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: Some(png_bytes) for normal map, None if missing.
    pub normal: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: Some(png_bytes) for metallic-roughness texture, None if missing.
    /// Extracted from the alpha channel of `_ddna` normal maps (per-pixel smoothness).
    /// Stored as glTF metallicRoughness: G=roughness (1-smoothness), B=metallic (0), R=0.
    pub roughness: Vec<Option<Vec<u8>>>,
}

/// A resolved loadout node — lightweight metadata for attachment resolution.
/// Mesh data is NOT stored here; it's loaded on demand by the consumer.
/// NMC is loaded from .cga even when .cgam (mesh) is missing.
pub struct ResolvedNode {
    pub entity_name: String,
    /// How this entity attaches to its parent (NMC node name or bone name).
    pub attachment_name: String,
    /// If true, this entity should not inherit its parent's rotation (translation only).
    pub no_rotation: bool,
    /// Item port helper offset position (CryEngine Z-up).
    pub offset_position: [f32; 3],
    /// Item port helper offset rotation (Euler angles in degrees).
    pub offset_rotation: [f32; 3],
    /// NMC scene graph (loaded from .cga even if mesh is missing).
    pub nmc: Option<crate::nmc::NodeMeshCombo>,
    /// Skeleton bones from .chr (only for CDF entities).
    pub bones: Vec<crate::skeleton::Bone>,
    /// Whether this entity has loadable geometry (.cgam exists).
    pub has_geometry: bool,
    /// The DataCore record for this entity (needed to reload mesh on demand).
    pub record: starbreaker_datacore::types::Record,
    /// Geometry path (for fallback loading via export_entity_from_paths).
    pub geometry_path: Option<String>,
    /// Material path (for fallback loading).
    pub material_path: Option<String>,
    /// Children in the loadout.
    pub children: Vec<ResolvedNode>,
}

impl ResolvedNode {
    /// Shallow clone: copies metadata but borrows children by reference.
    /// Returns a new node with the same children vec (shared, not deep-cloned).
    pub fn clone_shallow(&self) -> ResolvedNode {
        ResolvedNode {
            entity_name: self.entity_name.clone(),
            attachment_name: self.attachment_name.clone(),
            no_rotation: self.no_rotation,
            offset_position: self.offset_position,
            offset_rotation: self.offset_rotation,
            nmc: None, // NMC not needed for reparented nodes
            bones: Vec::new(),
            has_geometry: self.has_geometry,
            record: self.record,
            geometry_path: self.geometry_path.clone(),
            material_path: self.material_path.clone(),
            children: Vec::new(), // Children handled separately
        }
    }
}

/// All the data needed to add one child entity's geometry to a glTF scene.
pub struct EntityPayload {
    pub mesh: Mesh,
    pub materials: Option<crate::mtl::MtlFile>,
    pub textures: Option<MaterialTextures>,
    pub nmc: Option<crate::nmc::NodeMeshCombo>,
    pub palette: Option<crate::mtl::TintPalette>,
    /// Skeleton bones from this entity's .chr/.skin file.
    /// Used to create attachment points for children that reference bone names.
    pub bones: Vec<crate::skeleton::Bone>,
    pub entity_name: String,
    /// Source geometry file path (for mesh dedup across children with the same geometry).
    pub geometry_path: Option<String>,
    /// NMC node name in the parent to attach under.
    pub parent_node_name: String,
    /// Fallback: parent entity name to attach to if parent_node_name isn't found.
    pub parent_entity_name: String,
    /// If true, don't inherit parent node's rotation (translation only).
    pub no_rotation: bool,
    /// Item port helper offset position (CryEngine Z-up).
    pub offset_position: [f32; 3],
    /// Item port helper offset rotation (Euler angles in degrees).
    pub offset_rotation: [f32; 3],
}

/// A light extracted from a CryXMLB entity in a .soc file.
#[derive(Debug, Clone)]
pub struct LightInfo {
    pub name: String,
    /// Position in CryEngine coordinates (relative to container).
    pub position: [f64; 3],
    /// Rotation quaternion [w, x, y, z].
    pub rotation: [f64; 4],
    /// Color [r, g, b] normalized 0..1.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Attenuation radius in meters.
    pub radius: f32,
    /// For spot lights: inner cone angle in degrees.
    pub inner_angle: Option<f32>,
    /// For spot lights: outer cone angle in degrees.
    pub outer_angle: Option<f32>,
}

/// A geometry placement from a .soc interior container.
#[derive(Debug, Clone)]
pub struct InteriorMesh {
    /// Path to .cgf file in P4k (e.g., "objects/spaceships/ships/rsi/zeus/interior/...cgf").
    pub cgf_path: String,
    /// Material path override (if any, from IncludedObjects material list).
    pub material_path: Option<String>,
    /// 4×4 column-major transform matrix (f32, for glTF).
    pub transform: [[f32; 4]; 4],
    /// EntityClassGUID from CryXMLB — used to resolve geometry via DataCore
    /// when no inline PropertiesDataCore geometry path is available.
    pub entity_class_guid: Option<String>,
}

/// All geometry and lights from a single socpak interior container.
#[derive(Debug)]
pub struct InteriorPayload {
    pub name: String,
    /// Static geometry placements from IncludedObjects + CryXMLB entities.
    pub meshes: Vec<InteriorMesh>,
    /// Lights from CryXMLB entities.
    pub lights: Vec<LightInfo>,
    /// 4×4 column-major transform for this container relative to hull.
    pub container_transform: [[f32; 4]; 4],
    /// Tint palette record names from IncludedObjects (e.g. "rsi_interior_zeus_base").
    pub tint_palette_names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ivo::skin::{DataStreams, MeshInfo, SubMeshDescriptor};

    #[test]
    fn build_mesh_from_synthetic_skin() {
        let skin = SkinMesh {
            flags: 0,
            info: MeshInfo {
                flags2: 5,
                num_vertices: 3,
                num_indices: 3,
                num_submeshes: 1,
                model_min: [0.0, 0.0, 0.0],
                model_max: [10.0, 10.0, 10.0],
                min_bound: [0.0, 0.0, 0.0],
                max_bound: [10.0, 10.0, 10.0],
                extra_count: 0,
            },
            submeshes: vec![SubMeshDescriptor {
                mat_id: 0,
                node_parent_index: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                page_base: 0,
                num_vertices: 3,
                radius: 5.0,
                center: [5.0, 5.0, 5.0],
            }],
            streams: DataStreams {
                positions: PositionData::Quantized(vec![
                    [0x8001, 0x8001, 0x8001], // SNorm -1 → bbox min
                    [0, 0, 0],                // SNorm 0 → bbox center
                    [32767, 32767, 32767],    // SNorm +1 → bbox max
                ]),
                uvs: vec![[0x0000, 0x0000], [0x3C00, 0x3C00], [0x3800, 0x3800]],
                indices: vec![0, 1, 2],
                colors: None,
                tangents: None,
                normals: None,
            },
        };

        let materials = vec![MaterialName {
            name: "test_material".into(),
        }];

        let mesh = build_mesh(&skin, &materials);

        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.submeshes.len(), 1);
        assert_eq!(
            mesh.submeshes[0].material_name.as_deref(),
            Some("test_material")
        );
        assert!(mesh.positions[0][0] < 1.0);
        assert!(mesh.positions[2][0] > 9.0);

        let uvs = mesh.uvs.as_ref().unwrap();
        assert_eq!(uvs[0], [0.0, 0.0]);
        assert_eq!(uvs[1], [1.0, 1.0]);
        assert_eq!(uvs[2], [0.5, 0.5]);
    }
}
