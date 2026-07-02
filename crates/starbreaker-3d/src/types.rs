use crate::dequant;
use crate::ivo::material::MaterialName;
use crate::ivo::skin::{NormalData, PositionData, SkinMesh, TangentData};

#[derive(Debug, Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub secondary_uvs: Option<Vec<[f32; 2]>>,
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
    pub source_material_id: Option<u32>,
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
        match (&mut self.uvs, other.uvs) {
            (Some(a), Some(b)) => a.extend(b),
            (Some(a), None) => a.resize(self.positions.len(), [0.0, 0.0]),
            (None, Some(b)) => {
                let mut padded = vec![[0.0, 0.0]; vertex_offset as usize];
                padded.extend(b);
                self.uvs = Some(padded);
            }
            (None, None) => {}
        }
        match (&mut self.secondary_uvs, other.secondary_uvs) {
            (Some(a), Some(b)) => a.extend(b),
            (Some(a), None) => a.resize(self.positions.len(), [0.0, 0.0]),
            (None, Some(b)) => {
                let mut padded = vec![[0.0, 0.0]; vertex_offset as usize];
                padded.extend(b);
                self.secondary_uvs = Some(padded);
            }
            (None, None) => {}
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

fn split_rigid_weighted_submeshes(
    submeshes: &[SubMesh],
    indices: &[u32],
    bone_maps: Option<&[crate::ivo::skin::BoneMap12]>,
) -> (Vec<SubMesh>, Vec<u32>) {
    let Some(bone_maps) = bone_maps else {
        return (submeshes.to_vec(), indices.to_vec());
    };

    let mut rebuilt_submeshes = Vec::with_capacity(submeshes.len());
    let mut rebuilt_indices = Vec::with_capacity(indices.len());

    for submesh in submeshes {
        let start = submesh.first_index as usize;
        let end = (start + submesh.num_indices as usize).min(indices.len());
        let source_indices = &indices[start..end];
        let triangle_count = source_indices.len() / 3;

        if triangle_count == 0 {
            let mut original = submesh.clone();
            original.first_index = rebuilt_indices.len() as u32;
            original.num_indices = source_indices.len() as u32;
            rebuilt_indices.extend_from_slice(source_indices);
            rebuilt_submeshes.push(original);
            continue;
        }

        let mut grouped_indices = std::collections::BTreeMap::<u16, Vec<u32>>::new();
        let mut rigid_triangles = 0usize;
        let mut valid = true;

        for triangle in source_indices.chunks(3) {
            if triangle.len() < 3 {
                continue;
            }

            let mut triangle_joints = [0u16; 3];
            for (joint_slot, vertex_index) in triangle.iter().enumerate() {
                let Some(joint) = bone_maps
                    .get(*vertex_index as usize)
                    .and_then(|entry| entry.dominant_joint())
                else {
                    valid = false;
                    break;
                };
                triangle_joints[joint_slot] = joint;
            }

            if !valid {
                break;
            }

            let triangle_joint = if triangle_joints[0] == triangle_joints[1]
                || triangle_joints[0] == triangle_joints[2]
            {
                triangle_joints[0]
            } else if triangle_joints[1] == triangle_joints[2] {
                triangle_joints[1]
            } else {
                triangle_joints[0]
            };

            if triangle_joints[0] == triangle_joints[1] && triangle_joints[1] == triangle_joints[2]
            {
                rigid_triangles += 1;
            }

            grouped_indices
                .entry(triangle_joint)
                .or_default()
                .extend_from_slice(triangle);
        }

        let rigid_ratio = rigid_triangles as f32 / triangle_count as f32;
        // Phase 47.4: previously bailed when `grouped_indices.len() <= 1`,
        // but a submesh whose triangles all belong to a single dominant
        // joint (e.g. an un-skinned residual housing geometry that the
        // .skin authored as `node_parent_index = 0` (root) but whose
        // BoneMap12 entries all point to a single articulated bone) must
        // still be reassigned to that joint, otherwise it stays static
        // at the CHR root while its owning bone animates. The split loop
        // below handles len == 1 correctly (it just emits one rebuilt
        // submesh with `node_parent_index = joint`); only `!valid` and
        // `rigid_ratio < 0.9` (truly soft-skinned) need to bail.
        if !valid || grouped_indices.is_empty() || rigid_ratio < 0.9 {
            let mut original = submesh.clone();
            original.first_index = rebuilt_indices.len() as u32;
            original.num_indices = source_indices.len() as u32;
            rebuilt_indices.extend_from_slice(source_indices);
            rebuilt_submeshes.push(original);
            continue;
        }

        for (joint, joint_indices) in grouped_indices {
            let mut split = submesh.clone();
            split.first_index = rebuilt_indices.len() as u32;
            split.num_indices = joint_indices.len() as u32;
            split.node_parent_index = joint;
            rebuilt_indices.extend_from_slice(&joint_indices);
            rebuilt_submeshes.push(split);
        }
    }

    (rebuilt_submeshes, rebuilt_indices)
}

pub fn build_mesh(skin: &SkinMesh, materials: &[MaterialName]) -> Mesh {
    build_mesh_with_bbox(skin, materials, false)
}

/// Build mesh, optionally dequantizing with model bbox instead of scaling bbox.
/// Interior CGFs use model bbox because IncludedObjects placements are authored
/// for model-bbox space. The scaling bbox is expanded for GPU skinning across
/// NMC nodes and gives wrong vertex positions for placement.
pub fn build_mesh_with_bbox(
    skin: &SkinMesh,
    materials: &[MaterialName],
    use_model_bbox: bool,
) -> Mesh {
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
    let secondary_uvs: Option<Vec<[f32; 2]>> = skin
        .streams
        .secondary_uvs
        .as_ref()
        .map(|uvs| uvs.iter().map(|uv| dequant::decode_half2(*uv)).collect());

    let submeshes = skin
        .submeshes
        .iter()
        .map(|s| SubMesh {
            material_name: materials.get(s.mat_id as usize).map(|m| m.name.clone()),
            material_id: s.mat_id as u32,
            // Preserve the authored submesh material index so downstream
            // remapping can resolve against sparse source MTL indices.
            source_material_id: Some(s.mat_id as u32),
            first_index: s.first_index,
            num_indices: s.num_indices,
            first_vertex: s.first_vertex,
            num_vertices: s.page_base,
            node_parent_index: s.node_parent_index,
        })
        .collect::<Vec<_>>();

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

    let (submeshes, indices) =
        split_rigid_weighted_submeshes(&submeshes, &indices, skin.streams.bone_maps.as_deref());

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
            data.iter()
                .map(|&raw| dequant::decode_packed_unit_vector(raw))
                .collect(),
        ),
        None => None,
    };

    let tangent_decode: Option<Vec<dequant::NormalTangent>> = match &skin.streams.tangents {
        Some(TangentData::QTangents(data)) => {
            // IVOQTANGENTS: compressed tangent vectors (2× u32, 15-15-1-1 bit packing).
            // Despite the name, these are NOT quaternions — the game converts them at load time.
            Some(
                data.iter()
                    .map(|raw| dequant::decode_compressed_tangent(*raw))
                    .collect(),
            )
        }
        Some(TangentData::Tangents(data)) => {
            // IVOTANGENTS: i16 SNorm quaternion (CryEngine SPipQTangents format)
            Some(
                data.iter()
                    .map(|raw| dequant::decode_qtangent_snorm(*raw))
                    .collect(),
            )
        }
        None => None,
    };

    let normals: Option<Vec<[f32; 3]>> = direct_normals
        .or_else(|| {
            tangent_decode
                .as_ref()
                .map(|td| td.iter().map(|nt| nt.normal).collect())
        })
        .or_else(|| Some(compute_smooth_normals(&positions, &indices)));

    let tangents_out: Option<Vec<[f32; 4]>> =
        tangent_decode.map(|td| td.iter().map(|nt| nt.tangent).collect());

    Mesh {
        positions,
        indices,
        uvs,
        secondary_uvs,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureTransformInfo {
    pub scale: [f32; 2],
    pub tex_coord: u32,
}

/// Loaded texture data for glTF embedding, indexed by submaterial.
#[derive(Debug, Clone)]
pub struct MaterialTextures {
    /// Per-submaterial: Some(png_bytes) for diffuse texture, None if missing.
    pub diffuse: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: Some(png_bytes) for normal map, None if missing.
    pub normal: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: Some(png_bytes) for metallic-roughness texture, None if missing.
    /// Extracted from the alpha channel of `_ddna` normal maps (per-pixel smoothness).
    /// Stored as glTF metallicRoughness: G=perceptual roughness, B=neutral metallic (1), R=0.
    pub roughness: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: Some(png_bytes) for emissive output, None if missing.
    pub emissive: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: Some(png_bytes) for occlusion output, None if missing.
    pub occlusion: Vec<Option<Vec<u8>>>,
    /// Per-submaterial: simple texture transforms that can survive directly in glTF.
    pub diffuse_transform: Vec<Option<TextureTransformInfo>>,
    pub normal_transform: Vec<Option<TextureTransformInfo>>,
    pub roughness_transform: Vec<Option<TextureTransformInfo>>,
    pub emissive_transform: Vec<Option<TextureTransformInfo>>,
    pub occlusion_transform: Vec<Option<TextureTransformInfo>>,
    /// Per-submaterial tags describing deliberate bundled-mode fallbacks.
    pub bundled_fallbacks: Vec<Vec<String>>,
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
    /// Item port detach direction from the parent port definition, in port-local source axes.
    pub detach_direction: [f32; 3],
    /// Raw item port flags from SItemPortDef (e.g. "invisible uneditable").
    pub port_flags: String,
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
    /// Whether this node's DataCore record should be queried for child
    /// object containers during interior discovery.
    ///
    /// Vehicle XML synthetic wheel/tread nodes reuse the root record to supply
    /// geometry, but they are not standalone entity records and must not trigger
    /// child object-container discovery.
    pub allows_child_object_containers: bool,
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
            detach_direction: self.detach_direction,
            port_flags: self.port_flags.clone(),
            nmc: None, // NMC not needed for reparented nodes
            bones: Vec::new(),
            has_geometry: self.has_geometry,
            record: self.record,
            geometry_path: self.geometry_path.clone(),
            material_path: self.material_path.clone(),
            allows_child_object_containers: self.allows_child_object_containers,
            children: Vec::new(), // Children handled separately
        }
    }

    /// Clone only the payload-relevant fields for deferred mesh loading.
    pub fn clone_payload_source(&self) -> ResolvedNode {
        ResolvedNode {
            entity_name: self.entity_name.clone(),
            attachment_name: self.attachment_name.clone(),
            no_rotation: self.no_rotation,
            offset_position: self.offset_position,
            offset_rotation: self.offset_rotation,
            detach_direction: self.detach_direction,
            port_flags: self.port_flags.clone(),
            nmc: self.nmc.clone(),
            bones: self.bones.clone(),
            has_geometry: self.has_geometry,
            record: self.record,
            geometry_path: self.geometry_path.clone(),
            material_path: self.material_path.clone(),
            allows_child_object_containers: self.allows_child_object_containers,
            children: Vec::new(),
        }
    }
}

/// All the data needed to add one child entity's geometry to a glTF scene.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiBinding {
    pub binding_kind: String,
    pub source_entity_name: String,
    pub helper_name: Option<String>,
    pub default_view: Option<String>,
    /// `true` when the originally-resolved `defaultView` had no concrete content
    /// (null/zero canvas, or a shell canvas whose content is selected at
    /// runtime). The exporter still renders an "on" image by falling through to
    /// the first non-shell view, but consumers (e.g. Blender) can use this flag
    /// to know the entity's *true* default state is off and gate visibility.
    pub default_state_is_off: bool,
    pub default_state_name: Option<String>,
    pub default_light_color: Option<[u8; 4]>,
    pub default_light_intensity_milli: Option<u16>,
    pub canvas_guid: Option<String>,
    pub canvas_record_name: Option<String>,
    pub canvas_record_path: Option<String>,
    pub canvas_widget_canvas_path: Option<String>,
    pub canvas_widget_url_postfix: Option<String>,
    pub canvas_widget_url_optional: Option<String>,
    pub canvas_variable_binding: Option<String>,
    /// For MFD containers and physical screens whose container canvas has no inline
    /// content: the GUID of the per-helper content canvas resolved from the parent
    /// vehicle's `SCItemUIView_DashboardCanvasDef` record.
    pub content_canvas_guid: Option<String>,
    /// DataCore name of the record pointed to by `content_canvas_guid`.
    pub content_canvas_record_name: Option<String>,
    /// Localization key for the MFD screen name from the resolved `SMFDView.name`
    /// (e.g. `"@ui_MFD_View_TargetStatus"`). Injected into the frame canvas's
    /// `text_ScreenName` node to label the footer. `None` for non-MFD bindings.
    pub screen_name_loc_key: Option<String>,
    /// Localization key of the transit location this screen serves
    /// (`InteriorMesh::ui_location_loc_key` — the nearest `SCTransitDestination`
    /// within its radius). Pins the render's `transitdisplay.panelLocation`
    /// engine variable so a lift-call console's floor-name heading resolves
    /// (e.g. `@ui_interactor_carrack_garage` → "SUB DECK"). Part of the render
    /// identity: two consoles on different floors render different images.
    pub transit_location_loc_key: Option<String>,
    /// View index in the parent dashboard's `SCItemUIView_DashboardCanvasDef.View[]`
    /// array that provided `content_canvas_guid`, if resolved.
    pub dashboard_view_index: Option<u32>,
    /// Screen slot within `View[dashboard_view_index].screens[]` that provided
    /// `content_canvas_guid`, if resolved.
    pub dashboard_screen_slot: Option<u32>,
    pub owner_source_file: Option<String>,
    pub runtime_image_source: Option<String>,
    pub generated_image_path: Option<String>,
    pub generated_context_manifest_path: Option<String>,
    pub generated_resolved_source_path: Option<String>,
    pub generated_backend: Option<String>,
    pub generated_provenance: Option<String>,
    pub generated_confidence: Option<u8>,
    /// Physical display aspect (width / height, ≥ 1.0 for landscape screens) of
    /// the cockpit screen mesh this UI renders onto, derived from the
    /// render-target (`RTT_Screen`) faces of the `helper_name` node (see
    /// `ui_pipeline::screen_aspect`). Drives the render-target proportions for
    /// `physical`/`radar` screens so square gauges render square and the
    /// annunciator as its wide strip, instead of inheriting the shared
    /// `M_Physical_Screen` 16:9 canvas. `None` when the screen mesh is
    /// unavailable or not measurable; sizing then falls back to canvas/SWF.
    pub ui_screen_aspect_w_over_h: Option<f32>,
}

pub struct EntityPayload {
    pub mesh: Mesh,
    pub materials: Option<crate::mtl::MtlFile>,
    pub textures: Option<MaterialTextures>,
    pub nmc: Option<crate::nmc::NodeMeshCombo>,
    pub palette: Option<crate::mtl::TintPalette>,
    /// Source geometry path used to derive stable decomposed asset paths.
    pub geometry_path: String,
    /// Source material path used to derive stable decomposed metadata paths.
    pub material_path: String,
    /// Skeleton bones from this entity's .chr/.skin file.
    /// Used to create attachment points for children that reference bone names.
    pub bones: Vec<crate::skeleton::Bone>,
    /// Source path used to resolve this payload's animation tracks
    /// (`.chrparams`/`$TracksDatabase`) when exporting sidecar animations.
    pub skeleton_source_path: Option<String>,
    pub entity_name: String,
    /// DataCore EntityClassDefinition.Category for this payload, if present.
    pub entity_category: Option<String>,
    /// DataCore SAttachableComponentParams.AttachDef.Type for this payload, if present.
    pub attach_def_type: Option<String>,
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
    /// Item port detach direction from the parent port definition, in port-local source axes.
    pub detach_direction: [f32; 3],
    /// Raw item port flags from SItemPortDef (e.g. "invisible uneditable").
    pub port_flags: String,
    /// Screen/UI bindings discovered from this entity and any attached UI children.
    pub ui_bindings: Vec<UiBinding>,
}

/// A light extracted from a CryXMLB entity in a .soc file.
#[derive(Debug, Clone)]
pub struct LightInfo {
    pub name: String,
    /// Position in CryEngine coordinates (relative to container).
    pub position: [f64; 3],
    /// Basis label for ``position`` / ``rotation`` / ``direction_sc``.
    pub transform_basis: String,
    /// Rotation quaternion [w, x, y, z].
    pub rotation: [f64; 4],
    /// Normalized forward direction in source SC space.
    pub direction_sc: [f64; 3],
    /// Color [r, g, b] normalized 0..1.
    pub color: [f32; 3],
    /// Source light type (for example: Omni, Projector).
    pub light_type: String,
    /// Normalized reusable light kind for downstream DCCs.
    pub semantic_light_kind: String,
    /// Raw CryEngine intensity on the flattened active record.
    pub intensity_raw: f32,
    /// Unit label for ``intensity_raw``.
    pub intensity_unit: String,
    /// Explicit label for the exporter's candela-style proxy value.
    pub intensity_candela_proxy: f32,
    pub intensity: f32,
    /// Attenuation radius in meters.
    pub radius: f32,
    /// Explicit metric label duplicate for downstream consumers.
    pub radius_m: f32,
    /// For spot lights: inner cone angle in degrees.
    pub inner_angle: Option<f32>,
    /// For spot lights: outer cone angle in degrees.
    pub outer_angle: Option<f32>,
    /// For projector / spot lights: P4k-relative DDS path of the gobo
    /// projector texture (authored under `<projectorParams texture="...">`).
    pub projector_texture: Option<String>,
    /// Name of the authored state whose values currently populate
    /// `color` / `intensity` (first-non-zero of default → auxiliary →
    /// emergency → cinematic). Empty if the component had no usable state.
    pub active_state: String,
    /// All authored CryEngine light states keyed by state name
    /// (`offState`, `defaultState`, `auxiliaryState`, `emergencyState`,
    /// `cinematicState`). Missing states are absent from the map. Lets
    /// downstream tools (Blender addon) switch live between authored
    /// states without re-reading the source `.socpak`.
    pub states: std::collections::BTreeMap<String, LightStateInfo>,
}

/// One authored CryEngine light state.
///
/// CryEngine `<EntityComponentLight>` carries up to five child state nodes
/// (`offState`, `defaultState`, `auxiliaryState`, `emergencyState`,
/// `cinematicState`). Each carries its own `intensity`, `temperature`, and
/// `<color r g b>` child. We preserve all of them so downstream consumers
/// can switch between them at runtime.
#[derive(Debug, Clone)]
pub struct LightStateInfo {
    /// Raw CryEngine intensity (pre-candela multiplier). Zero means the
    /// light is authored off in this state.
    pub intensity_raw: f32,
    /// Explicit unit label for ``intensity_raw``.
    pub intensity_unit: String,
    /// Same value scaled by the glTF candela multiplier (× 200).
    pub intensity_cd: f32,
    /// Explicit alias for the exporter-scaled candela-style proxy.
    pub intensity_candela_proxy: f32,
    /// Authored colour temperature in Kelvin.
    pub temperature: f32,
    /// Mirrors the `useTemperature` flag on the parent component.
    /// When true, consumers should derive RGB from `temperature` instead
    /// of reading `color` directly.
    pub use_temperature: bool,
    /// Authored linear RGB triple (0..1) from the `<color>` child. When
    /// `use_temperature` is true this is usually `[1,1,1]`.
    pub color: [f32; 3],
    /// CryEngine authored light style id (e.g. strobe patterns).
    pub light_style: i32,
    /// Optional authored preset tag (e.g. ``fast`` / ``slow``).
    pub preset_tag: String,
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
    /// Entity class short name from socpak root item-port XML when no GUID is
    /// authored but the room setup still names a visual entity to spawn.
    pub entity_class_name: Option<String>,
    /// Per-object tint-palette record path, resolved at parse time from the
    /// object's palette index into its own `.soc`
    /// `IncludedObjects.tint_palette_paths`. `None` means the object carries no
    /// palette override (index 0xFFFF).
    pub tint_palette_name: Option<String>,
    /// Per-instance UI canvas GUID authored inline on this entity's
    /// `EntityComponentUIBuildingBlocks` component in the `.soc`
    /// (`layers[].views[].component.canvas`). This OVERRIDES the entity class
    /// record's default canvas — e.g. transit elevator consoles author the
    /// manufacturer-specific `OLD_TransitUIPanelExterior_ANVL` here even though
    /// their class default is the generic `OLD_TransitUIPanelExterior`. `None`
    /// when the entity authors no inline UIBuildingBlocks canvas.
    pub ui_canvas_guid: Option<String>,
    /// Localization key of the transit location this canvas-bearing screen
    /// serves (`@ui_interactor_carrack_garage` → "Sub Deck"), resolved at
    /// socpak parse time as the nearest `SCTransitDestination` metadata entry
    /// within its authored `Radius` (the console entity carries no destination
    /// reference — the engine associates transit peripherals spatially). Feeds
    /// the render's `transitdisplay.panelLocation` variable so each lift-call
    /// console shows ITS floor name. `None` for non-transit screens.
    pub ui_location_loc_key: Option<String>,
}

/// A resolved mesh placement from a .soc interior container.
#[derive(Debug, Clone)]
pub struct InteriorPlacement {
    pub mesh_index: usize,
    pub transform: [[f32; 4]; 4],
    pub palette: Option<crate::mtl::TintPalette>,
    pub ui_bindings: Vec<UiBinding>,
}

/// All geometry and lights from a single socpak interior container.
#[derive(Debug)]
pub struct InteriorPayload {
    pub name: String,
    /// Optional scene instance that owns this container when it comes from a
    /// child entity rather than the root export record.
    pub parent_entity_name: Option<String>,
    /// Optional source node/bone inside the parent scene instance that the
    /// container is authored relative to.
    pub parent_node_name: Option<String>,
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
    use crate::ivo::skin::{BoneMap12, DataStreams, MeshInfo, SubMeshDescriptor};

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
                unknown0: 0,
                unknown1: 0,
            }],
            extra_words: Vec::new(),
            streams: DataStreams {
                positions: PositionData::Quantized(vec![
                    [0x8001, 0x8001, 0x8001], // SNorm -1 → bbox min
                    [0, 0, 0],                // SNorm 0 → bbox center
                    [32767, 32767, 32767],    // SNorm +1 → bbox max
                ]),
                uvs: vec![[0x0000, 0x0000], [0x3C00, 0x3C00], [0x3800, 0x3800]],
                secondary_uvs: Some(vec![[0x3800, 0x0000], [0x0000, 0x3800], [0x3C00, 0x0000]]),
                indices: vec![0, 1, 2],
                bone_maps: None,
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

        let secondary_uvs = mesh.secondary_uvs.as_ref().unwrap();
        assert_eq!(secondary_uvs[0], [0.5, 0.0]);
        assert_eq!(secondary_uvs[1], [0.0, 0.5]);
        assert_eq!(secondary_uvs[2], [1.0, 0.0]);
    }

    #[test]
    fn build_mesh_splits_rigid_bone_mapped_submeshes() {
        let skin = SkinMesh {
            flags: 0,
            info: MeshInfo {
                flags2: 5,
                num_vertices: 6,
                num_indices: 6,
                num_submeshes: 1,
                model_min: [0.0, 0.0, 0.0],
                model_max: [1.0, 1.0, 1.0],
                min_bound: [0.0, 0.0, 0.0],
                max_bound: [1.0, 1.0, 1.0],
                extra_count: 0,
            },
            submeshes: vec![SubMeshDescriptor {
                mat_id: 0,
                node_parent_index: 0,
                first_index: 0,
                num_indices: 6,
                first_vertex: 0,
                page_base: 0,
                num_vertices: 6,
                radius: 1.0,
                center: [0.5, 0.5, 0.5],
                unknown0: 0,
                unknown1: 0,
            }],
            extra_words: Vec::new(),
            streams: DataStreams {
                positions: PositionData::Float(vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [2.0, 0.0, 0.0],
                    [3.0, 0.0, 0.0],
                    [2.0, 1.0, 0.0],
                ]),
                uvs: Vec::new(),
                secondary_uvs: None,
                indices: vec![0, 1, 2, 3, 4, 5],
                bone_maps: Some(vec![
                    BoneMap12 {
                        joint_indices: [0, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [0, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [0, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [3, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [3, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [3, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                ]),
                colors: None,
                tangents: None,
                normals: None,
            },
        };

        let materials = vec![MaterialName {
            name: "test_material".into(),
        }];

        let mesh = build_mesh(&skin, &materials);

        assert_eq!(mesh.submeshes.len(), 2);
        assert_eq!(mesh.submeshes[0].node_parent_index, 0);
        assert_eq!(mesh.submeshes[1].node_parent_index, 3);
        assert_eq!(mesh.submeshes[0].num_indices, 3);
        assert_eq!(mesh.submeshes[1].num_indices, 3);
        assert_eq!(mesh.indices, vec![0, 1, 2, 3, 4, 5]);
    }

    /// Phase 47.4: a submesh whose `node_parent_index = 0` (root) but
    /// whose triangle vertices all map to a single non-root bone via
    /// BoneMap12 must be re-assigned to that bone — otherwise the mesh
    /// stays static at the CHR root while its owning bone animates.
    /// Previously the function bailed when `grouped_indices.len() <= 1`,
    /// keeping the original (root-parented) submesh.
    #[test]
    fn build_mesh_reassigns_single_joint_root_submesh_to_owning_bone() {
        let skin = SkinMesh {
            flags: 0,
            info: MeshInfo {
                flags2: 5,
                num_vertices: 3,
                num_indices: 3,
                num_submeshes: 1,
                model_min: [0.0, 0.0, 0.0],
                model_max: [1.0, 1.0, 1.0],
                min_bound: [0.0, 0.0, 0.0],
                max_bound: [1.0, 1.0, 1.0],
                extra_count: 0,
            },
            submeshes: vec![SubMeshDescriptor {
                mat_id: 0,
                node_parent_index: 0, // authored at root
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                page_base: 0,
                num_vertices: 3,
                radius: 1.0,
                center: [0.5, 0.5, 0.5],
                unknown0: 0,
                unknown1: 0,
            }],
            extra_words: Vec::new(),
            streams: DataStreams {
                positions: PositionData::Float(vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                ]),
                uvs: Vec::new(),
                secondary_uvs: None,
                indices: vec![0, 1, 2],
                // All vertices weighted to bone 7 — so the submesh's
                // owning bone is 7, not 0.
                bone_maps: Some(vec![
                    BoneMap12 {
                        joint_indices: [7, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [7, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                    BoneMap12 {
                        joint_indices: [7, 0, 0, 0],
                        weights: [255, 0, 0, 0],
                    },
                ]),
                colors: None,
                tangents: None,
                normals: None,
            },
        };

        let materials = vec![MaterialName { name: "m".into() }];
        let mesh = build_mesh(&skin, &materials);

        assert_eq!(mesh.submeshes.len(), 1);
        assert_eq!(
            mesh.submeshes[0].node_parent_index, 7,
            "single-joint submesh should be reassigned from root (0) to owning bone (7)"
        );
        assert_eq!(mesh.submeshes[0].num_indices, 3);
    }
}
