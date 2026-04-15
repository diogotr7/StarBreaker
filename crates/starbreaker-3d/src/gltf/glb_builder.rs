//! GlbBuilder: centralized GLB construction state.
//!
//! GlbBuilder is used by `write_glb` for all mesh packing, texture embedding,
//! material creation, and NMC node hierarchy building. There is a single unified
//! code path for all export scenarios — single entities, assembled scenes, and interiors.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

use gltf_json as json;
use json::validation::Checked;

use crate::error::Error;
use crate::nmc::NodeMeshCombo;
use crate::types::{MaterialTextures, Mesh, SubMesh, TextureTransformInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PackedTextureKey {
    byte_len: usize,
    content_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextureRegion {
    offset: usize,
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PackedInteriorMeshKey {
    cgf_index: usize,
    palette_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MaterialIdentity {
    material_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SceneMaterialModeKey {
    None,
    Colors,
    Textures,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SceneMeshReuseKey {
    geometry_hash: u64,
    materials_hash: u64,
    textures_hash: u64,
    palette_hash: u64,
    material_mode: SceneMaterialModeKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SceneGeometryReuseKey {
    geometry_hash: u64,
    include_tangents: bool,
}

struct BuiltMaterial {
    material: json::Material,
    identity: MaterialIdentity,
}

#[derive(Clone)]
struct PackedGeometryInfo {
    pos_accessor_idx: u32,
    uv_accessor_idx: Option<u32>,
    secondary_uv_accessor_idx: Option<u32>,
    normal_accessor_idx: Option<u32>,
    color_accessor_idx: Option<u32>,
    tangent_accessor_idx: Option<u32>,
    submesh_idx_accessors: Vec<u32>,
}

/// Holds all mutable state for building a GLB file.
pub(crate) struct GlbBuilder {
    pub bin: Vec<u8>,
    pub buffer_views: Vec<json::buffer::View>,
    pub accessors: Vec<json::Accessor>,
    pub materials_json: Vec<json::Material>,
    pub images_json: Vec<json::Image>,
    pub textures_json: Vec<json::Texture>,
    pub samplers_json: Vec<json::texture::Sampler>,
    pub meshes_json: Vec<json::Mesh>,
    pub nodes_json: Vec<json::Node>,
    tex_cache: TexCache,
    tex_json_dedup: HashMap<TextureRegion, u32>,
    mat_dedup: HashMap<MaterialIdentity, u32>,
    scene_mesh_cache: HashMap<SceneMeshReuseKey, PackedMeshInfo>,
    geometry_cache: HashMap<SceneGeometryReuseKey, PackedGeometryInfo>,
    /// Maps lowercased node/entity name → glTF node index.
    pub node_name_to_idx: HashMap<String, u32>,
}

/// Texture dedup cache: maps a packed texture identity to its binary-buffer region.
type TexCache = HashMap<PackedTextureKey, TextureRegion>;

/// Result of packing a single entity's mesh into the builder.
#[derive(Clone)]
pub(crate) struct PackedMeshInfo {
    pub mesh_idx: u32,
    pub pos_accessor_idx: u32,
    pub uv_accessor_idx: Option<u32>,
    pub secondary_uv_accessor_idx: Option<u32>,
    pub normal_accessor_idx: Option<u32>,
    pub color_accessor_idx: Option<u32>,
    #[allow(dead_code)]
    pub tangent_accessor_idx: Option<u32>,
    /// Per-submesh: the deduplicated glTF material index.
    pub submesh_mat_indices: Vec<u32>,
    /// Per-submesh: the index accessor for this submesh, or None if skipped (e.g. NoDraw).
    pub submesh_idx_accessors: Vec<Option<u32>>,
}


/// Convert a 4×4 column-major matrix to a flat [f32; 16] for glTF.
pub(crate) fn flatten_4x4(m: &[[f32; 4]; 4]) -> [f32; 16] {
    [
        m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3], m[2][0], m[2][1],
        m[2][2], m[2][3], m[3][0], m[3][1], m[3][2], m[3][3],
    ]
}

/// Build a glTF 4x4 column-major matrix from an item port offset (position + Euler degrees).
pub(crate) fn offset_to_gltf_matrix(position: [f32; 3], rotation_deg: [f32; 3]) -> Option<[f32; 16]> {
    let has_pos = position[0] != 0.0 || position[1] != 0.0 || position[2] != 0.0;
    let has_rot = rotation_deg[0] != 0.0 || rotation_deg[1] != 0.0 || rotation_deg[2] != 0.0;
    if !has_pos && !has_rot {
        return None;
    }

    let rx = rotation_deg[0].to_radians();
    let ry = rotation_deg[1].to_radians();
    let rz = rotation_deg[2].to_radians();

    let rot = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
    let trans = glam::Vec3::from(position);
    let m = glam::Mat4::from_rotation_translation(rot, trans);
    Some(m.to_cols_array())
}

impl GlbBuilder {
    pub fn new() -> Self {
        Self {
            bin: Vec::new(),
            buffer_views: Vec::new(),
            accessors: Vec::new(),
            materials_json: Vec::new(),
            images_json: Vec::new(),
            textures_json: Vec::new(),
            samplers_json: Vec::new(),
            meshes_json: Vec::new(),
            nodes_json: Vec::new(),
            tex_cache: HashMap::new(),
            tex_json_dedup: HashMap::new(),
            mat_dedup: HashMap::new(),
            scene_mesh_cache: HashMap::new(),
            geometry_cache: HashMap::new(),
            node_name_to_idx: HashMap::new(),
        }
    }

    fn pack_geometry(&mut self, mesh: &Mesh, include_tangents: bool) -> PackedGeometryInfo {
        // Ensure 4-byte alignment
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }

        // Pack positions
        let pos_offset = self.bin.len();
        for p in &mesh.positions {
            self.bin.extend_from_slice(&p[0].to_le_bytes());
            self.bin.extend_from_slice(&p[1].to_le_bytes());
            self.bin.extend_from_slice(&p[2].to_le_bytes());
        }
        let pos_len = self.bin.len() - pos_offset;

        // Pack UVs
        let uv_offset = self.bin.len();
        let mut uv_len = 0;
        if let Some(uvs) = &mesh.uvs {
            for uv in uvs {
                self.bin.extend_from_slice(&uv[0].to_le_bytes());
                self.bin.extend_from_slice(&uv[1].to_le_bytes());
            }
            uv_len = self.bin.len() - uv_offset;
        }

        // Pack second UV set
        let secondary_uv_offset = self.bin.len();
        let mut secondary_uv_len = 0;
        if let Some(uvs) = &mesh.secondary_uvs {
            for uv in uvs {
                self.bin.extend_from_slice(&uv[0].to_le_bytes());
                self.bin.extend_from_slice(&uv[1].to_le_bytes());
            }
            secondary_uv_len = self.bin.len() - secondary_uv_offset;
        }

        // Pack normals
        let normal_offset = self.bin.len();
        let mut normal_len = 0;
        if let Some(normals) = &mesh.normals {
            for n in normals {
                self.bin.extend_from_slice(&n[0].to_le_bytes());
                self.bin.extend_from_slice(&n[1].to_le_bytes());
                self.bin.extend_from_slice(&n[2].to_le_bytes());
            }
            normal_len = self.bin.len() - normal_offset;
        }

        // Pack tangents (optional)
        let tangent_offset = self.bin.len();
        let mut tangent_len = 0;
        if include_tangents {
            if let Some(tangents) = &mesh.tangents {
                for t in tangents {
                    self.bin.extend_from_slice(&t[0].to_le_bytes());
                    self.bin.extend_from_slice(&t[1].to_le_bytes());
                    self.bin.extend_from_slice(&t[2].to_le_bytes());
                    self.bin.extend_from_slice(&t[3].to_le_bytes());
                }
                tangent_len = self.bin.len() - tangent_offset;
            }
        }

        // Pack vertex colors (optional, normalized U8 RGBA)
        let color_offset = self.bin.len();
        let mut color_len = 0;
        if let Some(colors) = &mesh.colors {
            for color in colors {
                self.bin.extend_from_slice(color);
            }
            color_len = self.bin.len() - color_offset;
        }

        // Pad to 4-byte boundary before indices
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }

        // Pack indices
        let idx_offset = self.bin.len();
        let max_index = mesh.indices.iter().copied().max().unwrap_or(0);
        let use_u16 = max_index <= u16::MAX as u32;
        if use_u16 {
            for &i in &mesh.indices {
                self.bin.extend_from_slice(&(i as u16).to_le_bytes());
            }
            while !self.bin.len().is_multiple_of(4) {
                self.bin.push(0);
            }
        } else {
            for &i in &mesh.indices {
                self.bin.extend_from_slice(&i.to_le_bytes());
            }
        }
        let idx_len = if use_u16 { mesh.indices.len() * 2 } else { mesh.indices.len() * 4 };
        let idx_component_type = if use_u16 {
            json::accessor::ComponentType::U16
        } else {
            json::accessor::ComponentType::U32
        };

        // Position bounds
        let mut pos_min = [f32::MAX; 3];
        let mut pos_max = [f32::MIN; 3];
        for p in &mesh.positions {
            for i in 0..3 {
                pos_min[i] = pos_min[i].min(p[i]);
                pos_max[i] = pos_max[i].max(p[i]);
            }
        }

        // Vertex accessors
        let pos_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            pos_offset,
            pos_len,
            mesh.positions.len(),
            json::accessor::Type::Vec3,
            Some((&pos_min, &pos_max)),
        )
        .unwrap_or(0);

        let uv_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            uv_offset,
            uv_len,
            mesh.uvs.as_ref().map_or(0, |v| v.len()),
            json::accessor::Type::Vec2,
            None,
        );
        let secondary_uv_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            secondary_uv_offset,
            secondary_uv_len,
            mesh.secondary_uvs.as_ref().map_or(0, |v| v.len()),
            json::accessor::Type::Vec2,
            None,
        );
        let normal_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            normal_offset,
            normal_len,
            mesh.normals.as_ref().map_or(0, |v| v.len()),
            json::accessor::Type::Vec3,
            None,
        );
        let color_accessor_idx = if color_len == 0 {
            None
        } else {
            let color_bv_idx = self.buffer_views.len() as u32;
            self.buffer_views.push(json::buffer::View {
                buffer: json::Index::new(0),
                byte_offset: Some(json::validation::USize64(color_offset as u64)),
                byte_length: json::validation::USize64(color_len as u64),
                byte_stride: None,
                target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
                name: None,
                extensions: None,
                extras: Default::default(),
            });
            let color_acc_idx = self.accessors.len() as u32;
            self.accessors.push(json::Accessor {
                buffer_view: Some(json::Index::new(color_bv_idx)),
                byte_offset: Some(json::validation::USize64(0)),
                count: json::validation::USize64(mesh.colors.as_ref().map_or(0, |v| v.len()) as u64),
                component_type: Checked::Valid(json::accessor::GenericComponentType(
                    json::accessor::ComponentType::U8,
                )),
                type_: Checked::Valid(json::accessor::Type::Vec4),
                min: None,
                max: None,
                name: None,
                normalized: true,
                sparse: None,
                extensions: None,
                extras: Default::default(),
            });
            Some(color_acc_idx)
        };
        let tangent_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            tangent_offset,
            tangent_len,
            mesh.tangents.as_ref().map_or(0, |v| v.len()),
            json::accessor::Type::Vec4,
            None,
        );

        // Index buffer view
        let idx_bv_idx = self.buffer_views.len() as u32;
        self.buffer_views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_offset: Some(json::validation::USize64(idx_offset as u64)),
            byte_length: json::validation::USize64(idx_len as u64),
            byte_stride: None,
            target: Some(Checked::Valid(json::buffer::Target::ElementArrayBuffer)),
            name: None,
            extensions: None,
            extras: Default::default(),
        });

        let mut submesh_idx_accessors = Vec::with_capacity(mesh.submeshes.len());
        for sub in &mesh.submeshes {
            let idx_byte_offset = if use_u16 {
                sub.first_index as u64 * 2
            } else {
                sub.first_index as u64 * 4
            };
            let acc_idx = self.accessors.len() as u32;
            self.accessors.push(json::Accessor {
                buffer_view: Some(json::Index::new(idx_bv_idx)),
                byte_offset: Some(json::validation::USize64(idx_byte_offset)),
                count: json::validation::USize64(sub.num_indices as u64),
                component_type: Checked::Valid(json::accessor::GenericComponentType(idx_component_type)),
                type_: Checked::Valid(json::accessor::Type::Scalar),
                min: None,
                max: None,
                name: None,
                normalized: false,
                sparse: None,
                extensions: None,
                extras: Default::default(),
            });
            submesh_idx_accessors.push(acc_idx);
        }

        PackedGeometryInfo {
            pos_accessor_idx,
            uv_accessor_idx,
            secondary_uv_accessor_idx,
            normal_accessor_idx,
            color_accessor_idx,
            tangent_accessor_idx,
            submesh_idx_accessors,
        }
    }

    /// Compute a node's world-space translation by walking up the parent chain.
    /// Uses the glTF column-major matrices stored in nodes.
    pub fn compute_node_world_translation(&self, node_idx: usize) -> [f32; 3] {
        let nodes = &self.nodes_json;
        // Build parent map from children arrays
        let mut parent_of: Vec<Option<usize>> = vec![None; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            if let Some(ref children) = n.children {
                for c in children {
                    let ci = c.value() as usize;
                    if ci < parent_of.len() {
                        parent_of[ci] = Some(i);
                    }
                }
            }
        }

        // Accumulate world transform by multiplying matrices up the chain
        let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];

        fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
            let mut r = [0.0f32; 16];
            for i in 0..4 {
                for j in 0..4 {
                    for k in 0..4 {
                        r[j * 4 + i] += a[k * 4 + i] * b[j * 4 + k];
                    }
                }
            }
            r
        }

        // Collect chain from node to root (cap at node count to break cycles)
        let mut chain = vec![node_idx];
        let mut cur = node_idx;
        while let Some(p) = parent_of[cur] {
            if chain.len() >= nodes.len() {
                log::warn!("cycle in node parent chain at node {cur}, breaking");
                break;
            }
            chain.push(p);
            cur = p;
        }

        // Multiply from root down
        let mut world = identity;
        for &idx in chain.iter().rev() {
            let local = nodes[idx].matrix.as_ref().map(|m| *m).unwrap_or(identity);
            world = mat4_mul(&world, &local);
        }

        [world[12], world[13], world[14]]
    }

    /// Append `child_idx` as a child of `parent_idx` in the node list.
    pub fn append_child_to_node(&mut self, parent_idx: u32, child_idx: u32) {
        if parent_idx == child_idx {
            log::warn!("skipping self-referencing node attachment at index {parent_idx}");
            return;
        }
        if let Some(node) = self.nodes_json.get_mut(parent_idx as usize) {
            if let Some(ref mut children_arr) = node.children {
                children_arr.push(json::Index::new(child_idx));
            } else {
                node.children = Some(vec![json::Index::new(child_idx)]);
            }
        }
    }

    /// Attach skeleton bone nodes to the first scene root, registering them in the name map.
    pub fn attach_skeleton_bones(
        &mut self,
        bones: &[crate::skeleton::Bone],
        scene_nodes: &[json::Index<json::Node>],
    ) {
        for bone in bones {
            let bone_node_idx = self.nodes_json.len() as u32;

            // Convert quaternion [w,x,y,z] + translation to 3x4 matrix.
            let [qw, qx, qy, qz] = bone.world_rotation;
            let rot = glam::Quat::from_xyzw(qx, qy, qz, qw);
            let trans = glam::Vec3::from(bone.world_position);
            let m = glam::Mat4::from_rotation_translation(rot, trans);
            let cols = m.to_cols_array();

            // Convert to our 3x4 row-major format for mat3x4_to_gltf
            let bone_matrix: [[f32; 4]; 3] = [
                [cols[0], cols[4], cols[8],  cols[12]],
                [cols[1], cols[5], cols[9],  cols[13]],
                [cols[2], cols[6], cols[10], cols[14]],
            ];

            let matrix = if super::is_identity_or_zero(&bone_matrix) {
                None
            } else {
                Some(super::mat3x4_to_gltf(&bone_matrix))
            };

            self.nodes_json.push(json::Node {
                name: Some(bone.name.clone()),
                matrix,
                ..Default::default()
            });

            // Add to lookup map (lowercase for case-insensitive matching).
            // Only insert if not already present — NMC nodes take priority.
            self.node_name_to_idx
                .entry(bone.name.to_lowercase())
                .or_insert(bone_node_idx);

            // Attach bone node to the first scene root node
            if let Some(root_node) = scene_nodes.first() {
                let root_idx = root_node.value() as u32;
                self.append_child_to_node(root_idx, bone_node_idx);
            }
        }
    }

    /// Attach a single child entity (with optional mesh + NMC) into the scene graph.
    pub fn attach_child_entity(
        &mut self,
        mut child: crate::types::EntityPayload,
        scene_nodes: &[json::Index<json::Node>],
        material_mode: crate::pipeline::MaterialMode,
        fallback_palette: Option<&crate::mtl::TintPalette>,
        load_textures: &mut dyn FnMut(
            Option<&crate::mtl::MtlFile>,
            Option<&crate::mtl::TintPalette>,
        ) -> Option<crate::types::MaterialTextures>,
    ) {
        if child.offset_position != [0.0; 3] || child.offset_rotation != [0.0; 3] {
            log::info!(
                "  glb '{}': parent_node='{}' offset pos=[{:.2},{:.2},{:.2}] rot=[{:.1},{:.1},{:.1}]",
                child.entity_name,
                child.parent_node_name,
                child.offset_position[0], child.offset_position[1], child.offset_position[2],
                child.offset_rotation[0], child.offset_rotation[1], child.offset_rotation[2],
            );
        }
        let has_mesh = !child.mesh.positions.is_empty();

        // Pack mesh data (skip for NMC-only entities with no geometry).
        let child_packed = if has_mesh {
            let resolved_palette = child.palette.as_ref().or(fallback_palette);
            let loaded_textures = if child.textures.is_some() {
                None
            } else {
                load_textures(child.materials.as_ref(), resolved_palette)
            };
            let packed = self.pack_mesh(
                &child.mesh,
                child.materials.as_ref(),
                child.textures.as_ref().or(loaded_textures.as_ref()),
                resolved_palette,
                Some(&child.entity_name),
                material_mode,
            );
            child.textures = None;
            child.mesh.positions = Vec::new();
            child.mesh.uvs = None;
            child.mesh.secondary_uvs = None;
            child.mesh.normals = None;
            child.mesh.tangents = None;
            child.mesh.colors = None;
            child.mesh.indices = Vec::new();
            Some(packed)
        } else {
            None
        };

        // Build child node(s): if the child has NMC, create its full sub-hierarchy.
        // Otherwise, a single flat node with the mesh.
        let child_root_idx = if let Some(ref child_nmc) = child.nmc {
            if !child_nmc.nodes.is_empty() {
                let child_node_base = self.nodes_json.len() as u32;
                let child_root_nodes = self.build_nmc_hierarchy(
                    child_packed.as_ref().unwrap_or(&PackedMeshInfo {
                        mesh_idx: 0,
                        pos_accessor_idx: 0,
                        uv_accessor_idx: None,
                        secondary_uv_accessor_idx: None,
                        normal_accessor_idx: None,
                        color_accessor_idx: None,
                        tangent_accessor_idx: None,
                        submesh_mat_indices: Vec::new(),
                        submesh_idx_accessors: Vec::new(),
                    }),
                    child_nmc,
                    &child.mesh.submeshes,
                    has_mesh,
                );

                // Log NMC node debug info
                for (_i, nmc_node) in child_nmc.nodes.iter().enumerate() {
                    log::debug!(
                        "  NMC node '{}' (entity '{}') type={} b2w=[{:.3},{:.3},{:.3},{:.3}][{:.3},{:.3},{:.3},{:.3}][{:.3},{:.3},{:.3},{:.3}]",
                        nmc_node.name, child.entity_name, nmc_node.geometry_type,
                        nmc_node.bone_to_world[0][0], nmc_node.bone_to_world[0][1], nmc_node.bone_to_world[0][2], nmc_node.bone_to_world[0][3],
                        nmc_node.bone_to_world[1][0], nmc_node.bone_to_world[1][1], nmc_node.bone_to_world[1][2], nmc_node.bone_to_world[1][3],
                        nmc_node.bone_to_world[2][0], nmc_node.bone_to_world[2][1], nmc_node.bone_to_world[2][2], nmc_node.bone_to_world[2][3],
                    );
                }

                // Create bone attachment nodes for this entity's skeleton.
                let mut bone_node_indices: Vec<json::Index<json::Node>> = Vec::new();
                for bone in &child.bones {
                    if bone.name.is_empty() {
                        continue;
                    }
                    let lower = bone.name.to_lowercase();
                    if self.node_name_to_idx.contains_key(&lower) {
                        continue; // NMC node already registered this name
                    }
                    let bone_idx = self.nodes_json.len() as u32;
                    let [qw, qx, qy, qz] = bone.world_rotation;
                    let rot = glam::Quat::from_xyzw(qx, qy, qz, qw);
                    let trans = glam::Vec3::from(bone.world_position);
                    let m = glam::Mat4::from_rotation_translation(rot, trans);
                    self.nodes_json.push(json::Node {
                        name: Some(bone.name.clone()),
                        matrix: Some(m.to_cols_array()),
                        ..Default::default()
                    });
                    self.node_name_to_idx.insert(lower, bone_idx);
                    bone_node_indices.push(json::Index::new(bone_idx));
                }

                // Create a wrapper node for the child entity containing its NMC root nodes + bone nodes
                let wrapper_idx = self.nodes_json.len() as u32;
                // Only register if the name isn't already taken (e.g., by a parent NMC node
                // with the same name — vehicle XML wheel parts use bone names as entity names).
                let lower_name = child.entity_name.to_lowercase();
                if !self.node_name_to_idx.contains_key(&lower_name) {
                    self.node_name_to_idx.insert(lower_name, wrapper_idx);
                }
                let mut wrapper_children: Vec<json::Index<json::Node>> = child_root_nodes
                    .iter()
                    .map(|&i| json::Index::new(child_node_base + i))
                    .collect();
                wrapper_children.extend(bone_node_indices);
                let offset_matrix = offset_to_gltf_matrix(child.offset_position, child.offset_rotation);
                self.nodes_json.push(json::Node {
                    name: Some(child.entity_name.clone()),
                    children: if wrapper_children.is_empty() {
                        None
                    } else {
                        Some(wrapper_children)
                    },
                    ..Default::default()
                });
                if let Some(m) = offset_matrix {
                    self.nodes_json[wrapper_idx as usize].matrix = Some(m);
                }
                wrapper_idx
            } else {
                // Empty NMC — use flat mesh
                let idx = self.nodes_json.len() as u32;
                let lower_name = child.entity_name.to_lowercase();
                if !self.node_name_to_idx.contains_key(&lower_name) {
                    self.node_name_to_idx.insert(lower_name, idx);
                }
                let offset_matrix = offset_to_gltf_matrix(child.offset_position, child.offset_rotation);
                self.nodes_json.push(json::Node {
                    name: Some(child.entity_name.clone()),
                    mesh: child_packed.as_ref().map(|cp| json::Index::new(cp.mesh_idx)),
                    matrix: offset_matrix,
                    ..Default::default()
                });
                idx
            }
        } else {
            // No NMC — single flat node
            let idx = self.nodes_json.len() as u32;
            let lower_name = child.entity_name.to_lowercase();
            if !self.node_name_to_idx.contains_key(&lower_name) {
                self.node_name_to_idx.insert(lower_name, idx);
            }
            let offset_matrix = offset_to_gltf_matrix(child.offset_position, child.offset_rotation);
            self.nodes_json.push(json::Node {
                name: Some(child.entity_name.clone()),
                mesh: child_packed.as_ref().map(|cp| json::Index::new(cp.mesh_idx)),
                matrix: offset_matrix,
                ..Default::default()
            });
            idx
        };

        let child_node_idx = child_root_idx;

        // Find parent NMC node and attach. Fall back to parent entity node, then root.
        let parent_idx = self.node_name_to_idx
            .get(&child.parent_node_name.to_lowercase())
            .copied()
            .or_else(|| {
                self.node_name_to_idx
                    .get(&child.parent_entity_name.to_lowercase())
                    .copied()
            });

        if parent_idx.is_none() {
            log::warn!(
                "no node for '{}' (parent '{}'), attaching to root",
                child.parent_node_name,
                child.parent_entity_name
            );
        }

        let target_idx = parent_idx
            .or_else(|| scene_nodes.first().map(|n| n.value() as u32))
            .unwrap_or(0);

        if child.no_rotation {
            // noRotation: attach to scene root with translation-only parent position,
            // then apply the item port offset on top (which may include its own rotation).
            let world_translation = self.compute_node_world_translation(target_idx as usize);
            let parent_trans = glam::Mat4::from_translation(glam::Vec3::new(
                world_translation[0], world_translation[1], world_translation[2],
            ));
            let final_matrix = if let Some(offset_m) = offset_to_gltf_matrix(child.offset_position, child.offset_rotation) {
                let offset_mat = glam::Mat4::from_cols_array(&offset_m);
                parent_trans * offset_mat
            } else {
                parent_trans
            };
            let child_node = &mut self.nodes_json[child_node_idx as usize];
            child_node.matrix = Some(final_matrix.to_cols_array());
            // Attach to the first scene root node instead of the parent.
            let root_idx = scene_nodes.first().map(|n| n.value() as u32).unwrap_or(0);
            self.append_child_to_node(root_idx, child_node_idx);
        } else {
            self.append_child_to_node(target_idx, child_node_idx);
        }
    }

    /// Pack and attach interior CGF meshes as instanced nodes.
    /// Returns the scene nodes created for each interior container, plus all lights.
    pub fn attach_interiors(
        &mut self,
        interiors: &crate::pipeline::LoadedInteriors,
        material_mode: crate::pipeline::MaterialMode,
        fallback_palette: Option<&crate::mtl::TintPalette>,
        load_textures: &mut dyn FnMut(
            Option<&crate::mtl::MtlFile>,
            Option<&crate::mtl::TintPalette>,
        ) -> Option<crate::types::MaterialTextures>,
        load_interior_mesh: &mut dyn FnMut(
            &crate::pipeline::InteriorCgfEntry,
        ) -> Option<(crate::types::Mesh, Option<crate::mtl::MtlFile>, Option<NodeMeshCombo>)>,
    ) -> (Vec<json::Index<json::Node>>, Vec<crate::types::LightInfo>) {
        // Cache loaded meshes by CGF index (mesh data + materials from P4k).
        // Mesh loading is expensive; packing with different palettes is cheap.
        let mut mesh_cache: Vec<Option<(crate::types::Mesh, Option<crate::mtl::MtlFile>)>> =
            vec![None; interiors.unique_cgfs.len()];

        // Cache packed glTF mesh indices by (cgf_index, palette_key).
        // Same CGF + same palette = reuse. Different palette = separate mesh.
        let mut packed_cache: HashMap<PackedInteriorMeshKey, u32> = HashMap::new();

        let mut all_lights: Vec<crate::types::LightInfo> = Vec::new();
        let mut container_scene_nodes: Vec<json::Index<json::Node>> = Vec::new();

        for container in &interiors.containers {
            let palette = container.palette.as_ref().or(fallback_palette);
            let palette_key = palette.map(|p| {
                // Hash palette colors for cache key
                use std::hash::{Hash, Hasher};
                let mut h = std::hash::DefaultHasher::new();
                for c in &[p.primary, p.secondary, p.tertiary, p.glass] {
                    c[0].to_bits().hash(&mut h);
                    c[1].to_bits().hash(&mut h);
                    c[2].to_bits().hash(&mut h);
                }
                h.finish()
            }).unwrap_or(0);

            let container_node_idx = self.nodes_json.len() as u32;
            self.nodes_json.push(json::Node {
                name: Some(format!("interior_{}", container.name)),
                matrix: Some(flatten_4x4(&container.container_transform)),
                ..Default::default()
            });

            let mut container_children = Vec::new();

            for &(mesh_array_idx, ref transform) in &container.placements {
                // Get or pack the mesh for this (cgf, palette) pair.
                let cache_key = PackedInteriorMeshKey {
                    cgf_index: mesh_array_idx,
                    palette_hash: palette_key,
                };
                let gltf_mesh_idx = if let Some(&idx) = packed_cache.get(&cache_key) {
                    Some(idx)
                } else {
                    // Load mesh data (cached by CGF index).
                    if mesh_cache[mesh_array_idx].is_none() {
                        mesh_cache[mesh_array_idx] = load_interior_mesh(&interiors.unique_cgfs[mesh_array_idx])
                            .map(|(mesh, mtl, _nmc)| (mesh, mtl));
                    }
                    let Some((ref mesh, ref mtl)) = mesh_cache[mesh_array_idx] else {
                        packed_cache.insert(cache_key, u32::MAX);
                        continue;
                    };
                    let textures = load_textures(mtl.as_ref(), palette);
                    let packed = self.pack_mesh(
                        mesh,
                        mtl.as_ref(),
                        textures.as_ref(),
                        palette,
                        Some(&interiors.unique_cgfs[mesh_array_idx].name),
                        material_mode,
                    );
                    packed_cache.insert(cache_key, packed.mesh_idx);
                    Some(packed.mesh_idx)
                };

                let Some(gltf_mesh_idx) = gltf_mesh_idx.filter(|&i| i != u32::MAX) else {
                    continue;
                };
                let node_idx = self.nodes_json.len() as u32;
                self.nodes_json.push(json::Node {
                    mesh: Some(json::Index::new(gltf_mesh_idx)),
                    matrix: Some(flatten_4x4(transform)),
                    ..Default::default()
                });
                container_children.push(json::Index::new(node_idx));
            }

            // Add light nodes
            for light in &container.lights {
                let light_idx = all_lights.len();
                all_lights.push(light.clone());

                let node_idx = self.nodes_json.len() as u32;
                let pos = light.position;
                let rot = light.rotation;
                self.nodes_json.push(json::Node {
                    name: Some(light.name.clone()),
                    translation: Some([pos[0] as f32, pos[1] as f32, pos[2] as f32]),
                    rotation: Some(json::scene::UnitQuaternion(
                        [rot[1] as f32, rot[2] as f32, rot[3] as f32, rot[0] as f32],
                    )),
                    extensions: Some(json::extensions::scene::Node {
                        khr_lights_punctual: Some(
                            json::extensions::scene::khr_lights_punctual::KhrLightsPunctual {
                                light: json::Index::new(light_idx as u32),
                            },
                        ),
                    }),
                    ..Default::default()
                });
                container_children.push(json::Index::new(node_idx));
            }

            if !container_children.is_empty() {
                self.nodes_json[container_node_idx as usize].children = Some(container_children);
            }
            container_scene_nodes.push(json::Index::new(container_node_idx));
        }

        (container_scene_nodes, all_lights)
    }

    /// Pack a mesh's vertex data, textures, and materials into the builder.
    /// Returns a PackedMeshInfo with accessor/material indices for building primitives.
    pub fn pack_mesh(
        &mut self,
        mesh: &Mesh,
        materials: Option<&crate::mtl::MtlFile>,
        textures: Option<&MaterialTextures>,
        palette: Option<&crate::mtl::TintPalette>,
        mesh_name: Option<&str>,
        material_mode: crate::pipeline::MaterialMode,
    ) -> PackedMeshInfo {
        let include_tangents = material_mode.include_tangents();
        let experimental_textures = material_mode.experimental();
        let reuse_key = build_scene_mesh_reuse_key(mesh, materials, textures, palette, material_mode);
        if let Some(packed) = self.scene_mesh_cache.get(&reuse_key) {
            return packed.clone();
        }
        let geometry_key = SceneGeometryReuseKey {
            geometry_hash: hash_mesh_for_reuse(mesh, include_tangents),
            include_tangents,
        };
        let geometry = if let Some(cached) = self.geometry_cache.get(&geometry_key) {
            cached.clone()
        } else {
            let packed = self.pack_geometry(mesh, include_tangents);
            self.geometry_cache.insert(geometry_key, packed.clone());
            packed
        };

        // Embed textures with content-based dedup.
        let mut texture_regions: Vec<Option<TextureRegion>> = Vec::new();
        let mut normal_regions: Vec<Option<TextureRegion>> = Vec::new();
        let mut roughness_regions: Vec<Option<TextureRegion>> = Vec::new();
        let mut emissive_regions: Vec<Option<TextureRegion>> = Vec::new();
        let mut occlusion_regions: Vec<Option<TextureRegion>> = Vec::new();
        if let Some(tex) = textures {
            for png_opt in &tex.diffuse {
                texture_regions.push(pack_texture_deduped(png_opt.as_deref(), &mut self.bin, &mut self.tex_cache));
            }
            for png_opt in &tex.normal {
                normal_regions.push(pack_texture_deduped(png_opt.as_deref(), &mut self.bin, &mut self.tex_cache));
            }
            for png_opt in &tex.roughness {
                roughness_regions.push(pack_texture_deduped(png_opt.as_deref(), &mut self.bin, &mut self.tex_cache));
            }
            for png_opt in &tex.emissive {
                emissive_regions.push(pack_texture_deduped(png_opt.as_deref(), &mut self.bin, &mut self.tex_cache));
            }
            for png_opt in &tex.occlusion {
                occlusion_regions.push(pack_texture_deduped(png_opt.as_deref(), &mut self.bin, &mut self.tex_cache));
            }
        }

        // Texture → glTF mappings
        let has_any_texture = texture_regions.iter()
            .chain(normal_regions.iter())
            .chain(roughness_regions.iter())
            .chain(emissive_regions.iter())
            .chain(occlusion_regions.iter())
            .any(|r| r.is_some());
        if has_any_texture && self.samplers_json.is_empty() {
            self.samplers_json.push(json::texture::Sampler {
                mag_filter: Some(Checked::Valid(json::texture::MagFilter::Linear)),
                min_filter: Some(Checked::Valid(json::texture::MinFilter::Linear)),
                wrap_s: Checked::Valid(json::texture::WrappingMode::Repeat),
                wrap_t: Checked::Valid(json::texture::WrappingMode::Repeat),
                name: None,
                extensions: None,
                extras: Default::default(),
            });
        }

        let submaterial_texture_idx = regions_to_gltf_textures_deduped(&texture_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut self.tex_json_dedup);
        let submaterial_normal_idx = regions_to_gltf_textures_deduped(&normal_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut self.tex_json_dedup);
        let submaterial_roughness_idx = regions_to_gltf_textures_deduped(&roughness_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut self.tex_json_dedup);
        let submaterial_emissive_idx = regions_to_gltf_textures_deduped(&emissive_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut self.tex_json_dedup);
        let submaterial_occlusion_idx = regions_to_gltf_textures_deduped(&occlusion_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut self.tex_json_dedup);

        // Build materials with dedup
        let submesh_mat_indices = build_materials(
            &mesh.submeshes,
            materials,
            palette,
            textures,
            &submaterial_texture_idx,
            &submaterial_normal_idx,
            &submaterial_roughness_idx,
            &submaterial_emissive_idx,
            &submaterial_occlusion_idx,
            &mut self.materials_json,
            &mut self.mat_dedup,
            experimental_textures,
        );

        // Build mesh primitives (skip NoDraw and textureless decal submeshes)
        let mut primitives = Vec::new();
        let mut submesh_idx_accessors: Vec<Option<u32>> = Vec::with_capacity(mesh.submeshes.len());
        for (i, sub) in mesh.submeshes.iter().enumerate() {
            let mtl_sub = materials.and_then(|m| m.materials.get(sub.material_id as usize));
            if mtl_sub.is_some_and(|m| m.should_hide()) {
                submesh_idx_accessors.push(None);
                continue;
            }
            // Decals without a base color texture are just solid rectangles covering the hull.
            // Their alpha cutout comes entirely from the texture — skip when textures are off.
            if mtl_sub.is_some_and(|m| m.is_decal()) {
                let has_tex = submaterial_texture_idx
                    .get(sub.material_id as usize)
                    .is_some_and(|t| t.is_some());
                if !has_tex {
                    submesh_idx_accessors.push(None);
                    continue;
                }
            }
            let acc_idx = geometry.submesh_idx_accessors[i];
            submesh_idx_accessors.push(Some(acc_idx));

            let mut attributes = BTreeMap::new();
            attributes.insert(
                Checked::Valid(json::mesh::Semantic::Positions),
                json::Index::new(geometry.pos_accessor_idx),
            );
            if let Some(uv_idx) = geometry.uv_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::TexCoords(0)),
                    json::Index::new(uv_idx),
                );
            }
            if let Some(secondary_uv_idx) = geometry.secondary_uv_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::TexCoords(1)),
                    json::Index::new(secondary_uv_idx),
                );
            }
            if let Some(normal_idx) = geometry.normal_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::Normals),
                    json::Index::new(normal_idx),
                );
            }
            if let Some(color_idx) = geometry.color_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::Colors(0)),
                    json::Index::new(color_idx),
                );
            }
            if let Some(tangent_idx) = geometry.tangent_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::Tangents),
                    json::Index::new(tangent_idx),
                );
            }

            primitives.push(json::mesh::Primitive {
                attributes,
                indices: Some(json::Index::new(acc_idx)),
                material: Some(json::Index::new(submesh_mat_indices[i])),
                mode: Checked::Valid(json::mesh::Mode::Triangles),
                targets: None,
                extensions: None,
                extras: Default::default(),
            });
        }

        let mesh_idx = self.meshes_json.len() as u32;
        self.meshes_json.push(json::Mesh {
            primitives,
            weights: None,
            name: mesh_name.map(String::from),
            extensions: None,
            extras: Default::default(),
        });

        let packed = PackedMeshInfo {
            mesh_idx,
            pos_accessor_idx: geometry.pos_accessor_idx,
            uv_accessor_idx: geometry.uv_accessor_idx,
            secondary_uv_accessor_idx: geometry.secondary_uv_accessor_idx,
            normal_accessor_idx: geometry.normal_accessor_idx,
            color_accessor_idx: geometry.color_accessor_idx,
            tangent_accessor_idx: geometry.tangent_accessor_idx,
            submesh_mat_indices,
            submesh_idx_accessors,
        };

        self.scene_mesh_cache.insert(reuse_key, packed.clone());
        packed
    }

    /// Build an NMC node hierarchy from a packed mesh.
    /// Pops the flat mesh, creates per-NMC-node meshes and glTF nodes.
    /// Returns the root node indices for this entity.
    /// Registers node names in `node_name_to_idx`.
    pub fn build_nmc_hierarchy(
        &mut self,
        packed: &PackedMeshInfo,
        nmc: &NodeMeshCombo,
        submeshes: &[crate::types::SubMesh],
        has_mesh: bool,
    ) -> Vec<u32> {
        use std::collections::BTreeMap;

        // Remove the flat mesh — we'll replace with per-NMC-node meshes.
        if has_mesh {
            self.meshes_json.pop();
        }

        // Group submeshes by NMC node.
        let mut node_submeshes: Vec<Vec<usize>> = vec![vec![]; nmc.nodes.len()];
        for (i, sub) in submeshes.iter().enumerate() {
            let nidx = sub.node_parent_index as usize;
            if nidx < nmc.nodes.len() {
                node_submeshes[nidx].push(i);
            }
        }
        log::debug!(
            "  NMC hierarchy: {} nodes, {} submeshes, has_mesh={}, root='{}'",
            nmc.nodes.len(), submeshes.len(), has_mesh,
            nmc.nodes.first().map(|n| n.name.as_str()).unwrap_or("?"),
        );

        // Create per-NMC-node meshes.
        let mut node_mesh_idx: Vec<Option<u32>> = vec![None; nmc.nodes.len()];
        if has_mesh {
            for (nmc_idx, submesh_indices) in node_submeshes.iter().enumerate() {
                if submesh_indices.is_empty() { continue; }
                let mut primitives = Vec::new();
                for &si in submesh_indices {
                    // Skip submeshes that were excluded during packing (e.g. NoDraw).
                    let Some(idx_acc) = packed.submesh_idx_accessors[si] else { continue };
                    let mut attributes = BTreeMap::new();
                    attributes.insert(
                        Checked::Valid(json::mesh::Semantic::Positions),
                        json::Index::new(packed.pos_accessor_idx),
                    );
                    if let Some(uv) = packed.uv_accessor_idx {
                        attributes.insert(
                            Checked::Valid(json::mesh::Semantic::TexCoords(0)),
                            json::Index::new(uv),
                        );
                    }
                    if let Some(secondary_uv) = packed.secondary_uv_accessor_idx {
                        attributes.insert(
                            Checked::Valid(json::mesh::Semantic::TexCoords(1)),
                            json::Index::new(secondary_uv),
                        );
                    }
                    if let Some(n) = packed.normal_accessor_idx {
                        attributes.insert(
                            Checked::Valid(json::mesh::Semantic::Normals),
                            json::Index::new(n),
                        );
                    }
                    if let Some(color) = packed.color_accessor_idx {
                        attributes.insert(
                            Checked::Valid(json::mesh::Semantic::Colors(0)),
                            json::Index::new(color),
                        );
                    }
                    primitives.push(json::mesh::Primitive {
                        attributes,
                        indices: Some(json::Index::new(idx_acc)),
                        material: Some(json::Index::new(packed.submesh_mat_indices[si])),
                        mode: Checked::Valid(json::mesh::Mode::Triangles),
                        targets: None,
                        extensions: None,
                        extras: Default::default(),
                    });
                }
                node_mesh_idx[nmc_idx] = Some(self.meshes_json.len() as u32);
                self.meshes_json.push(json::Mesh {
                    primitives,
                    weights: None,
                    name: Some(nmc.nodes[nmc_idx].name.clone()),
                    extensions: None,
                    extras: Default::default(),
                });
            }
        }

        // Compute parent-child relationships.
        let mut children_map: Vec<Vec<u32>> = vec![vec![]; nmc.nodes.len()];
        let mut root_nodes: Vec<u32> = vec![];
        for (i, node) in nmc.nodes.iter().enumerate() {
            match node.parent_index {
                Some(parent) if (parent as usize) < nmc.nodes.len() => {
                    children_map[parent as usize].push(i as u32);
                }
                _ => root_nodes.push(i as u32),
            }
        }

        // Create glTF nodes.
        let node_base = self.nodes_json.len() as u32;
        for (i, nmc_node) in nmc.nodes.iter().enumerate() {
            let child_indices: Vec<json::Index<json::Node>> = children_map[i]
                .iter()
                .map(|&c| json::Index::new(node_base + c))
                .collect();
            let matrix = if super::is_identity_or_zero(&nmc_node.bone_to_world) {
                None
            } else {
                Some(super::mat3x4_to_gltf(&nmc_node.bone_to_world))
            };
            let node_idx = self.nodes_json.len() as u32;
            if !nmc_node.name.is_empty() {
                self.node_name_to_idx.insert(nmc_node.name.to_lowercase(), node_idx);
            }
            self.nodes_json.push(json::Node {
                name: Some(nmc_node.name.clone()),
                mesh: node_mesh_idx[i].map(json::Index::new),
                matrix,
                children: if child_indices.is_empty() { None } else { Some(child_indices) },
                ..Default::default()
            });
        }

        root_nodes
    }

    /// Finalize the GLB: add coordinate root, serialize JSON + binary to GLB bytes.
    /// `lights` is the list of KHR_lights_punctual lights (empty for no lights).
    pub fn finalize(
        mut self,
        scene_nodes: Vec<json::Index<json::Node>>,
        lights: Vec<crate::types::LightInfo>,
        metadata: &super::GlbMetadata,
    ) -> Result<Vec<u8>, Error> {
        // Wrap all content under CryEngine Z-up → glTF Y-up rotation root.
        let coord_root_idx = self.nodes_json.len() as u32;
        self.nodes_json.push(json::Node {
            name: Some("CryEngine_Z_up".into()),
            children: Some(scene_nodes),
            matrix: Some([
                1.0, 0.0, 0.0, 0.0,
                0.0, 0.0, -1.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ]),
            ..Default::default()
        });

        let scenes_json = vec![json::Scene {
            nodes: vec![json::Index::new(coord_root_idx)],
            name: None,
            extensions: None,
            extras: Default::default(),
        }];

        let mut extensions_used = Vec::new();
        if self
            .materials_json
            .iter()
            .any(|material| material.extensions.as_ref().and_then(|extensions| extensions.transmission.as_ref()).is_some())
        {
            extensions_used.push("KHR_materials_transmission".to_string());
        }
        if self
            .materials_json
            .iter()
            .any(|material| material.extensions.as_ref().and_then(|extensions| extensions.ior.as_ref()).is_some())
        {
            extensions_used.push("KHR_materials_ior".to_string());
        }
        if self
            .materials_json
            .iter()
            .any(|material| material.extensions.as_ref().and_then(|extensions| extensions.volume.as_ref()).is_some())
        {
            extensions_used.push("KHR_materials_volume".to_string());
        }
        if self
            .materials_json
            .iter()
            .any(|material| material.extensions.as_ref().and_then(|extensions| extensions.emissive_strength.as_ref()).is_some())
        {
            extensions_used.push("KHR_materials_emissive_strength".to_string());
        }
        if self.materials_json.iter().any(material_uses_texture_transform) {
            extensions_used.push("KHR_texture_transform".to_string());
        }

        // KHR_lights_punctual
        use json::extensions::scene::khr_lights_punctual as klp;
        let root_extensions = if !lights.is_empty() {
            extensions_used.push("KHR_lights_punctual".to_string());
            let gltf_lights: Vec<klp::Light> = lights.iter().map(|l| {
                let (type_, spot) = if let (Some(inner), Some(outer)) = (l.inner_angle, l.outer_angle) {
                    (Checked::Valid(klp::Type::Spot), Some(klp::Spot {
                        inner_cone_angle: inner.to_radians(),
                        outer_cone_angle: outer.to_radians(),
                    }))
                } else {
                    (Checked::Valid(klp::Type::Point), None)
                };
                klp::Light {
                    color: l.color,
                    intensity: l.intensity,
                    name: Some(l.name.clone()),
                    range: Some(l.radius),
                    type_,
                    spot,
                    extensions: None,
                    extras: Default::default(),
                }
            }).collect();
            Some(json::extensions::root::Root {
                khr_lights_punctual: Some(json::extensions::root::KhrLightsPunctual { lights: gltf_lights }),
            })
        } else {
            None
        };

        let asset_extras = {
            let mut map = serde_json::Map::new();
            map.insert("generator_version".into(), serde_json::json!(env!("CARGO_PKG_VERSION")));
            if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                map.insert("export_timestamp_unix".into(), serde_json::json!(d.as_secs()));
            }
            let eo = &metadata.export_options;
            map.insert("export_options".into(), serde_json::json!({
                "kind": eo.kind,
                "material_mode": eo.material_mode,
                "format": eo.format,
                "lod_level": eo.lod_level,
                "texture_mip": eo.texture_mip,
                "include_attachments": eo.include_attachments,
                "include_interior": eo.include_interior,
            }));
            Some(serde_json::value::RawValue::from_string(
                serde_json::to_string(&serde_json::Value::Object(map))?
            )?.into())
        };

        let root = json::Root {
            asset: json::Asset {
                generator: Some("starbreaker-gltf".into()),
                version: "2.0".into(),
                extras: asset_extras,
                ..Default::default()
            },
            buffers: vec![json::Buffer {
                byte_length: json::validation::USize64(self.bin.len() as u64),
                uri: None,
                name: None,
                extensions: None,
                extras: Default::default(),
            }],
            buffer_views: self.buffer_views,
            accessors: self.accessors,
            meshes: self.meshes_json,
            materials: self.materials_json,
            images: self.images_json,
            textures: self.textures_json,
            samplers: self.samplers_json,
            nodes: self.nodes_json,
            scenes: scenes_json,
            scene: Some(json::Index::new(0)),
            extensions_used,
            extensions: root_extensions,
            ..Default::default()
        };

        serialize_glb(&root, &self.bin)
    }
}

/// Serialize a [`serde_json::Map`] into a [`RawValue`] for glTF extras.
///
/// Returns `None` if serialization fails (should never happen for in-memory JSON
/// values without non-finite floats).
fn map_to_raw_value(
    map: serde_json::Map<String, serde_json::Value>,
) -> Option<Box<serde_json::value::RawValue>> {
    let s = serde_json::to_string(&serde_json::Value::Object(map))
        .map_err(|e| log::warn!("failed to serialize extras map: {e}"))
        .ok()?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| log::warn!("failed to create RawValue from extras JSON: {e}"))
        .ok()
}

fn string_value_to_json(value: &str) -> serde_json::Value {
    if value.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(integer) = value.parse::<i64>() {
        return serde_json::json!(integer);
    }
    if let Ok(float) = value.parse::<f64>() {
        return serde_json::json!(float);
    }
    serde_json::json!(value)
}

fn hash_vec2(hasher: &mut std::hash::DefaultHasher, values: &[f32; 2]) {
    values[0].to_bits().hash(hasher);
    values[1].to_bits().hash(hasher);
}

fn hash_vec3(hasher: &mut std::hash::DefaultHasher, values: &[f32; 3]) {
    values[0].to_bits().hash(hasher);
    values[1].to_bits().hash(hasher);
    values[2].to_bits().hash(hasher);
}

fn hash_vec4(hasher: &mut std::hash::DefaultHasher, values: &[f32; 4]) {
    values[0].to_bits().hash(hasher);
    values[1].to_bits().hash(hasher);
    values[2].to_bits().hash(hasher);
    values[3].to_bits().hash(hasher);
}

fn hash_optional_bytes(
    hasher: &mut std::hash::DefaultHasher,
    bytes: Option<&[u8]>,
) {
    match bytes {
        Some(bytes) => {
            true.hash(hasher);
            bytes.len().hash(hasher);
            bytes.hash(hasher);
        }
        None => false.hash(hasher),
    }
}

fn hash_optional_transform(
    hasher: &mut std::hash::DefaultHasher,
    transform: Option<&TextureTransformInfo>,
) {
    use std::hash::Hash;

    match transform {
        Some(transform) => {
            true.hash(hasher);
            transform.tex_coord.hash(hasher);
            hash_vec2(hasher, &transform.scale);
        }
        None => false.hash(hasher),
    }
}

fn scene_material_mode_key(
    material_mode: crate::pipeline::MaterialMode,
) -> SceneMaterialModeKey {
    match material_mode {
        crate::pipeline::MaterialMode::None => SceneMaterialModeKey::None,
        crate::pipeline::MaterialMode::Colors => SceneMaterialModeKey::Colors,
        crate::pipeline::MaterialMode::Textures => SceneMaterialModeKey::Textures,
        crate::pipeline::MaterialMode::All => SceneMaterialModeKey::All,
    }
}

fn hash_mesh_for_reuse(mesh: &Mesh, include_tangents: bool) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::hash::DefaultHasher::new();

    mesh.positions.len().hash(&mut hasher);
    for position in &mesh.positions {
        hash_vec3(&mut hasher, position);
    }

    match &mesh.uvs {
        Some(uvs) => {
            true.hash(&mut hasher);
            uvs.len().hash(&mut hasher);
            for uv in uvs {
                hash_vec2(&mut hasher, uv);
            }
        }
        None => false.hash(&mut hasher),
    }

    match &mesh.secondary_uvs {
        Some(uvs) => {
            true.hash(&mut hasher);
            uvs.len().hash(&mut hasher);
            for uv in uvs {
                hash_vec2(&mut hasher, uv);
            }
        }
        None => false.hash(&mut hasher),
    }

    match &mesh.normals {
        Some(normals) => {
            true.hash(&mut hasher);
            normals.len().hash(&mut hasher);
            for normal in normals {
                hash_vec3(&mut hasher, normal);
            }
        }
        None => false.hash(&mut hasher),
    }

    include_tangents.hash(&mut hasher);
    if include_tangents {
        match &mesh.tangents {
            Some(tangents) => {
                true.hash(&mut hasher);
                tangents.len().hash(&mut hasher);
                for tangent in tangents {
                    hash_vec4(&mut hasher, tangent);
                }
            }
            None => false.hash(&mut hasher),
        }
    }

    match &mesh.colors {
        Some(colors) => {
            true.hash(&mut hasher);
            colors.len().hash(&mut hasher);
            for color in colors {
                color.hash(&mut hasher);
            }
        }
        None => false.hash(&mut hasher),
    }

    mesh.indices.len().hash(&mut hasher);
    mesh.indices.hash(&mut hasher);

    mesh.submeshes.len().hash(&mut hasher);
    for submesh in &mesh.submeshes {
        submesh.material_name.hash(&mut hasher);
        submesh.material_id.hash(&mut hasher);
        submesh.first_index.hash(&mut hasher);
        submesh.num_indices.hash(&mut hasher);
        submesh.first_vertex.hash(&mut hasher);
        submesh.num_vertices.hash(&mut hasher);
        submesh.node_parent_index.hash(&mut hasher);
    }

    hasher.finish()
}

fn hash_materials_for_reuse(materials: Option<&crate::mtl::MtlFile>) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::hash::DefaultHasher::new();
    match materials {
        Some(materials) => {
            true.hash(&mut hasher);
            materials.source_path.hash(&mut hasher);
            materials.materials.len().hash(&mut hasher);
            for material in &materials.materials {
                material.name.hash(&mut hasher);
                material.shader.hash(&mut hasher);
                hash_vec3(&mut hasher, &material.diffuse);
                material.opacity.to_bits().hash(&mut hasher);
                material.alpha_test.to_bits().hash(&mut hasher);
                material.string_gen_mask.hash(&mut hasher);
                material.is_nodraw.hash(&mut hasher);
                hash_vec3(&mut hasher, &material.specular);
                material.shininess.to_bits().hash(&mut hasher);
                hash_vec3(&mut hasher, &material.emissive);
                material.glow.to_bits().hash(&mut hasher);
                material.surface_type.hash(&mut hasher);
                material.diffuse_tex.hash(&mut hasher);
                material.normal_tex.hash(&mut hasher);
                material.palette_tint.hash(&mut hasher);

                material.layers.len().hash(&mut hasher);
                for layer in &material.layers {
                    layer.path.hash(&mut hasher);
                    hash_vec3(&mut hasher, &layer.tint_color);
                    layer.palette_tint.hash(&mut hasher);
                    layer.uv_tiling.to_bits().hash(&mut hasher);
                }

                material.texture_slots.len().hash(&mut hasher);
                for slot in &material.texture_slots {
                    slot.slot.hash(&mut hasher);
                    slot.path.hash(&mut hasher);
                    slot.is_virtual.hash(&mut hasher);
                }

                material.public_params.len().hash(&mut hasher);
                for param in &material.public_params {
                    param.name.hash(&mut hasher);
                    param.value.hash(&mut hasher);
                }
            }
        }
        None => false.hash(&mut hasher),
    }
    hasher.finish()
}

fn hash_textures_for_reuse(textures: Option<&MaterialTextures>) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::hash::DefaultHasher::new();
    match textures {
        Some(textures) => {
            true.hash(&mut hasher);

            textures.diffuse.len().hash(&mut hasher);
            for diffuse in &textures.diffuse {
                hash_optional_bytes(&mut hasher, diffuse.as_deref());
            }

            textures.normal.len().hash(&mut hasher);
            for normal in &textures.normal {
                hash_optional_bytes(&mut hasher, normal.as_deref());
            }

            textures.roughness.len().hash(&mut hasher);
            for roughness in &textures.roughness {
                hash_optional_bytes(&mut hasher, roughness.as_deref());
            }

            textures.emissive.len().hash(&mut hasher);
            for emissive in &textures.emissive {
                hash_optional_bytes(&mut hasher, emissive.as_deref());
            }

            textures.occlusion.len().hash(&mut hasher);
            for occlusion in &textures.occlusion {
                hash_optional_bytes(&mut hasher, occlusion.as_deref());
            }

            textures.diffuse_transform.len().hash(&mut hasher);
            for transform in &textures.diffuse_transform {
                hash_optional_transform(&mut hasher, transform.as_ref());
            }

            textures.normal_transform.len().hash(&mut hasher);
            for transform in &textures.normal_transform {
                hash_optional_transform(&mut hasher, transform.as_ref());
            }

            textures.roughness_transform.len().hash(&mut hasher);
            for transform in &textures.roughness_transform {
                hash_optional_transform(&mut hasher, transform.as_ref());
            }

            textures.emissive_transform.len().hash(&mut hasher);
            for transform in &textures.emissive_transform {
                hash_optional_transform(&mut hasher, transform.as_ref());
            }

            textures.occlusion_transform.len().hash(&mut hasher);
            for transform in &textures.occlusion_transform {
                hash_optional_transform(&mut hasher, transform.as_ref());
            }

            textures.bundled_fallbacks.len().hash(&mut hasher);
            for fallbacks in &textures.bundled_fallbacks {
                fallbacks.hash(&mut hasher);
            }
        }
        None => false.hash(&mut hasher),
    }
    hasher.finish()
}

fn hash_palette_for_reuse(palette: Option<&crate::mtl::TintPalette>) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::hash::DefaultHasher::new();
    match palette {
        Some(palette) => {
            true.hash(&mut hasher);
            palette.source_name.hash(&mut hasher);
            hash_vec3(&mut hasher, &palette.primary);
            hash_vec3(&mut hasher, &palette.secondary);
            hash_vec3(&mut hasher, &palette.tertiary);
            hash_vec3(&mut hasher, &palette.glass);
        }
        None => false.hash(&mut hasher),
    }
    hasher.finish()
}

fn build_scene_mesh_reuse_key(
    mesh: &Mesh,
    materials: Option<&crate::mtl::MtlFile>,
    textures: Option<&MaterialTextures>,
    palette: Option<&crate::mtl::TintPalette>,
    material_mode: crate::pipeline::MaterialMode,
) -> SceneMeshReuseKey {
    SceneMeshReuseKey {
        geometry_hash: hash_mesh_for_reuse(mesh, material_mode.include_tangents()),
        materials_hash: hash_materials_for_reuse(materials),
        textures_hash: hash_textures_for_reuse(textures),
        palette_hash: hash_palette_for_reuse(palette),
        material_mode: scene_material_mode_key(material_mode),
    }
}

/// Serialize a glTF Root + binary buffer into a GLB byte vector.
fn serialize_glb(root: &json::Root, bin: &[u8]) -> Result<Vec<u8>, Error> {
    let mut json_value = serde_json::to_value(root).map_err(|e| Error::Gltf(e.to_string()))?;
    strip_null_texture_transform_texcoords(&mut json_value);
    let json_bytes = serde_json::to_vec(&json_value).map_err(|e| Error::Gltf(e.to_string()))?;

    let json_padding = (4 - json_bytes.len() % 4) % 4;
    let json_padded_len = json_bytes.len() + json_padding;
    let bin_padding = (4 - bin.len() % 4) % 4;
    let bin_padded_len = bin.len() + bin_padding;
    let total_len = 12 + 8 + json_padded_len + 8 + bin_padded_len;

    let mut glb = Vec::with_capacity(total_len);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_len as u32).to_le_bytes());

    glb.extend_from_slice(&(json_padded_len as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    glb.extend(std::iter::repeat_n(b' ', json_padding));

    glb.extend_from_slice(&(bin_padded_len as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes());
    glb.extend_from_slice(bin);
    glb.extend(std::iter::repeat_n(0u8, bin_padding));

    Ok(glb)
}

fn strip_null_texture_transform_texcoords(value: &mut serde_json::Value) {
    fn recurse(value: &mut serde_json::Value, in_texture_transform: bool) {
        match value {
            serde_json::Value::Object(map) => {
                if in_texture_transform && matches!(map.get("texCoord"), Some(serde_json::Value::Null)) {
                    map.remove("texCoord");
                }
                for (key, child) in map.iter_mut() {
                    recurse(child, key == "KHR_texture_transform");
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    recurse(item, false);
                }
            }
            _ => {}
        }
    }

    recurse(value, false);
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Convert packed texture regions to glTF texture entries with region-based dedup.
fn regions_to_gltf_textures_deduped(
    regions: &[Option<TextureRegion>],
    buffer_views: &mut Vec<json::buffer::View>,
    images_json: &mut Vec<json::Image>,
    textures_json: &mut Vec<json::Texture>,
    dedup: &mut HashMap<TextureRegion, u32>,
) -> Vec<Option<u32>> {
    regions.iter().map(|region| {
        let region = *region.as_ref()?;
        if let Some(&idx) = dedup.get(&region) {
            return Some(idx);
        }
        let bv_idx = buffer_views.len() as u32;
        buffer_views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_offset: Some(json::validation::USize64(region.offset as u64)),
            byte_length: json::validation::USize64(region.len as u64),
            byte_stride: None,
            target: None,
            name: None,
            extensions: None,
            extras: Default::default(),
        });
        let image_idx = images_json.len() as u32;
        images_json.push(json::Image {
            buffer_view: Some(json::Index::new(bv_idx)),
            mime_type: Some(json::image::MimeType("image/png".to_string())),
            name: None,
            uri: None,
            extensions: None,
            extras: Default::default(),
        });
        let tex_idx = textures_json.len() as u32;
        textures_json.push(json::Texture {
            name: None,
            sampler: Some(json::Index::new(0)),
            source: json::Index::new(image_idx),
            extensions: None,
            extras: Default::default(),
        });
        dedup.insert(region, tex_idx);
        Some(tex_idx)
    }).collect()
}

fn texture_transform_parts(
    transform: Option<&TextureTransformInfo>,
) -> (u32, Option<json::extensions::texture::Info>) {
    let Some(transform) = transform else {
        return (0, None);
    };

    let needs_transform = (transform.scale[0] - 1.0).abs() > 1e-4
        || (transform.scale[1] - 1.0).abs() > 1e-4;
    let tex_coord = transform.tex_coord;
    if !needs_transform {
        return (tex_coord, None);
    }

    let mut extension = json::extensions::texture::Info::default();
    extension.texture_transform = Some(json::extensions::texture::TextureTransform {
        offset: json::extensions::texture::TextureTransformOffset([0.0, 0.0]),
        rotation: json::extensions::texture::TextureTransformRotation(0.0),
        scale: json::extensions::texture::TextureTransformScale(transform.scale),
        tex_coord: (tex_coord != 0).then_some(tex_coord),
        extras: Default::default(),
    });
    (tex_coord, Some(extension))
}

fn build_texture_info(
    tex_idx: u32,
    transform: Option<&TextureTransformInfo>,
) -> json::texture::Info {
    let (tex_coord, extensions) = texture_transform_parts(transform);
    json::texture::Info {
        index: json::Index::new(tex_idx),
        tex_coord,
        extensions,
        extras: Default::default(),
    }
}

fn build_normal_texture(
    tex_idx: u32,
    transform: Option<&TextureTransformInfo>,
) -> json::material::NormalTexture {
    let tex_coord = transform.map(|transform| transform.tex_coord).unwrap_or(0);
    json::material::NormalTexture {
        index: json::Index::new(tex_idx),
        scale: 1.0,
        tex_coord,
        extensions: None,
        extras: Default::default(),
    }
}

fn build_occlusion_texture(
    tex_idx: u32,
    transform: Option<&TextureTransformInfo>,
) -> json::material::OcclusionTexture {
    let tex_coord = transform.map(|transform| transform.tex_coord).unwrap_or(0);
    json::material::OcclusionTexture {
        index: json::Index::new(tex_idx),
        strength: json::material::StrengthFactor(1.0),
        tex_coord,
        extensions: None,
        extras: Default::default(),
    }
}

fn material_uses_texture_transform(material: &json::Material) -> bool {
    material
        .pbr_metallic_roughness
        .base_color_texture
        .as_ref()
        .and_then(|texture| texture.extensions.as_ref())
        .is_some()
        || material
            .pbr_metallic_roughness
            .metallic_roughness_texture
            .as_ref()
            .and_then(|texture| texture.extensions.as_ref())
            .is_some()
        || material
            .emissive_texture
            .as_ref()
            .and_then(|texture| texture.extensions.as_ref())
            .is_some()
        || material
            .normal_texture
            .as_ref()
            .and_then(|texture| texture.extensions.as_ref())
            .is_some()
        || material
            .occlusion_texture
            .as_ref()
            .and_then(|texture| texture.extensions.as_ref())
            .is_some()
}

/// Build deduplicated glTF materials for a set of submeshes.
fn build_materials(
    submeshes: &[SubMesh],
    materials: Option<&crate::mtl::MtlFile>,
    palette: Option<&crate::mtl::TintPalette>,
    textures: Option<&MaterialTextures>,
    submaterial_texture_idx: &[Option<u32>],
    submaterial_normal_idx: &[Option<u32>],
    submaterial_roughness_idx: &[Option<u32>],
    submaterial_emissive_idx: &[Option<u32>],
    submaterial_occlusion_idx: &[Option<u32>],
    materials_json: &mut Vec<json::Material>,
    mat_dedup: &mut HashMap<MaterialIdentity, u32>,
    experimental_textures: bool,
) -> Vec<u32> {
    submeshes.iter().map(|sub| {
        let BuiltMaterial { material, identity } = build_material(
            sub,
            materials,
            palette,
            textures,
            submaterial_texture_idx,
            submaterial_normal_idx,
            submaterial_roughness_idx,
            submaterial_emissive_idx,
            submaterial_occlusion_idx,
            experimental_textures,
        );
        if let Some(&idx) = mat_dedup.get(&identity) {
            idx
        } else {
            let idx = materials_json.len() as u32;
            materials_json.push(material);
            mat_dedup.insert(identity, idx);
            idx
        }
    }).collect()
}

/// Build a single glTF material from a submesh's material properties.
fn build_material(
    sub: &SubMesh,
    materials: Option<&crate::mtl::MtlFile>,
    palette: Option<&crate::mtl::TintPalette>,
    textures: Option<&MaterialTextures>,
    submaterial_texture_idx: &[Option<u32>],
    submaterial_normal_idx: &[Option<u32>],
    submaterial_roughness_idx: &[Option<u32>],
    submaterial_emissive_idx: &[Option<u32>],
    submaterial_occlusion_idx: &[Option<u32>],
    experimental_textures: bool,
) -> BuiltMaterial {
    let mtl_sub = materials.and_then(|m| m.materials.get(sub.material_id as usize));
    let texture_set = textures;
    let palette_channel_name = |channel: u8| -> Option<&'static str> {
        match channel {
            1 => Some("primary"),
            2 => Some("secondary"),
            3 => Some("tertiary"),
            _ => None,
        }
    };

    let (base_color_factor, alpha_mode, alpha_cutoff_value, double_sided, mat_name) = if let Some(m) = mtl_sub {
        let ac = m.alpha_config();
        let alpha = m.opacity;
        let (gltf_alpha_mode, cutoff) = match ac {
            crate::mtl::AlphaConfig::Opaque => (
                json::material::AlphaMode::Opaque,
                None,
            ),
            crate::mtl::AlphaConfig::Blend => (
                json::material::AlphaMode::Blend,
                None,
            ),
            crate::mtl::AlphaConfig::Mask(v) => (
                json::material::AlphaMode::Mask,
                Some(v),
            ),
        };
        let base_name = if m.name.is_empty() {
            sub.material_name.clone().unwrap_or_default()
        } else {
            m.name.clone()
        };
        // CGF-Converter compatible naming: {mtl_stem}_mtl_{material_name}_0{material_id}
        let name = {
            let mtl_stem = materials.and_then(|mtl| {
                mtl.source_path.as_ref().and_then(|p| {
                    let file = p.rsplit(['\\', '/']).next()?;
                    Some(file.strip_suffix(".mtl").unwrap_or(file).to_string())
                })
            });
            if let Some(stem) = mtl_stem {
                Some(format!("{stem}_mtl_{base_name}_0{}", sub.material_id))
            } else if base_name.is_empty() {
                None
            } else {
                Some(base_name)
            }
        };
        let palette_color = palette.and_then(|p| match m.palette_tint {
            1 => Some(p.primary),
            2 => Some(p.secondary),
            3 => Some(p.tertiary),
            _ if m.is_glass() => Some(p.glass),
            _ => None,
        });
        let layer_color = m.layers.first()
            .map(|l| l.tint_color)
            .filter(|t| *t != [1.0, 1.0, 1.0]);
        let color = palette_color.or(m.metal_base_color()).or(layer_color).unwrap_or(m.diffuse);
        (
            [color[0], color[1], color[2], alpha],
            gltf_alpha_mode,
            cutoff,
            m.is_double_sided(),
            name,
        )
    } else {
        (
            [0.8, 0.8, 0.8, 1.0],
            json::material::AlphaMode::Opaque,
            None,
            false,
            sub.material_name.clone(),
        )
    };

    let alpha_cutoff = alpha_cutoff_value.map(json::material::AlphaCutoff);

    let diffuse_transform = texture_set
        .and_then(|textures| textures.diffuse_transform.get(sub.material_id as usize))
        .and_then(|transform| transform.as_ref());
    let normal_transform = texture_set
        .and_then(|textures| textures.normal_transform.get(sub.material_id as usize))
        .and_then(|transform| transform.as_ref());
    let roughness_transform = texture_set
        .and_then(|textures| textures.roughness_transform.get(sub.material_id as usize))
        .and_then(|transform| transform.as_ref());
    let emissive_transform = texture_set
        .and_then(|textures| textures.emissive_transform.get(sub.material_id as usize))
        .and_then(|transform| transform.as_ref());
    let occlusion_transform = texture_set
        .and_then(|textures| textures.occlusion_transform.get(sub.material_id as usize))
        .and_then(|transform| transform.as_ref());
    let bundled_fallbacks = texture_set
        .and_then(|textures| textures.bundled_fallbacks.get(sub.material_id as usize))
        .cloned()
        .unwrap_or_default();

    let base_color_texture_idx = submaterial_texture_idx
        .get(sub.material_id as usize).copied().flatten();
    let base_color_texture = base_color_texture_idx
        .map(|tex_idx| build_texture_info(tex_idx, diffuse_transform));
    // Only apply per-pixel normal/roughness when the material has a direct TexSlot1 diffuse.
    // When textures come from MatLayer .mtl files (HardSurface/LayerBlend shaders), they are
    // tileable detail patterns for one layer of CryEngine's multi-layer blending system.
    // Applied standalone in glTF, the roughness variation creates extreme specular noise
    // and the normals add unwanted surface perturbation. Use scalar values instead.
    let allow_detail_textures = experimental_textures || mtl_sub.is_some_and(|m| m.diffuse_tex.is_some());
    let normal_texture_idx = if allow_detail_textures {
        submaterial_normal_idx.get(sub.material_id as usize).copied().flatten()
    } else {
        None
    };
    let normal_texture = if allow_detail_textures {
        normal_texture_idx.map(|tex_idx| build_normal_texture(tex_idx, normal_transform))
    } else {
        None
    };

    let (roughness, metallic) = mtl_sub.map(|m| (m.roughness(), m.metallic())).unwrap_or((0.5, 0.0));
    let is_glass = mtl_sub.map(|m| m.is_glass()).unwrap_or(false);

    // Per-pixel roughness creates visible specular noise in glTF PBR, especially on
    // dark surfaces viewed up close. Only enable with --experimental-textures.
    // Scalar roughness from MTL Shininess provides clean uniform glossiness.
    let roughness_texture_idx = if experimental_textures {
        submaterial_roughness_idx.get(sub.material_id as usize).copied().flatten()
    } else {
        None
    };
    let roughness_texture = if experimental_textures {
        roughness_texture_idx.map(|tex_idx| build_texture_info(tex_idx, roughness_transform))
    } else {
        None
    };
    let roughness_factor = if roughness_texture.is_some() { 1.0 } else { roughness };

    let emissive_texture_idx = submaterial_emissive_idx
        .get(sub.material_id as usize)
        .copied()
        .flatten();
    let emissive_texture = emissive_texture_idx
        .map(|tex_idx| build_texture_info(tex_idx, emissive_transform));

    let occlusion_texture_idx = submaterial_occlusion_idx
        .get(sub.material_id as usize)
        .copied()
        .flatten();
    let occlusion_texture = occlusion_texture_idx
        .map(|tex_idx| build_occlusion_texture(tex_idx, occlusion_transform));

    let mut emissive = mtl_sub.map(|m| m.emissive_factor()).unwrap_or([0.0, 0.0, 0.0]);
    if emissive_texture.is_some() && emissive == [0.0, 0.0, 0.0] {
        emissive = [1.0, 1.0, 1.0];
    }
    let emissive_strength = emissive.iter().copied().fold(0.0f32, f32::max);
    let emissive_strength_ext = (emissive_strength > 1.0).then(|| {
        json::extensions::material::EmissiveStrength {
            emissive_strength: json::extensions::material::EmissiveStrengthFactor(emissive_strength),
        }
    });
    if emissive_strength > 1.0 {
        emissive = [
            emissive[0] / emissive_strength,
            emissive[1] / emissive_strength,
            emissive[2] / emissive_strength,
        ];
    }

    let transmission_factor = mtl_sub
        .and_then(|material| material.public_param_f32(&["Transmission", "GlassTransmission"]))
        .unwrap_or(if is_glass { 0.96 } else { 0.0 })
        .clamp(0.0, 1.0);
    let ior = mtl_sub
        .and_then(|material| material.public_param_f32(&["IOR", "GlassIOR", "RefractiveIndex"]))
        .unwrap_or(1.5)
        .clamp(1.0, 2.5);
    let thickness_factor = mtl_sub
        .and_then(|material| material.public_param_f32(&["Thickness", "GlassThickness", "RefractionDepth"]))
        .unwrap_or(0.02)
        .max(0.0);
    let attenuation_distance = mtl_sub
        .and_then(|material| material.public_param_f32(&["AttenuationDistance", "AbsorptionDistance"]))
        .unwrap_or(0.25)
        .max(0.001);
    let attenuation_color = mtl_sub
        .and_then(|material| material.public_param_rgb(&["AbsorptionColor", "AttenuationColor", "TintColor"]))
        .or_else(|| palette.map(|palette| palette.glass))
        .unwrap_or([1.0, 1.0, 1.0]);

    let mut mat_extensions = json::extensions::material::Material::default();
    if is_glass {
        mat_extensions.transmission = Some(json::extensions::material::Transmission {
            transmission_factor: json::extensions::material::TransmissionFactor(transmission_factor),
            transmission_texture: None,
            extras: Default::default(),
        });
        mat_extensions.ior = Some(json::extensions::material::Ior {
            ior: json::extensions::material::IndexOfRefraction(ior),
            extras: Default::default(),
        });
        mat_extensions.volume = Some(json::extensions::material::Volume {
            thickness_factor: json::extensions::material::ThicknessFactor(thickness_factor),
            thickness_texture: None,
            attenuation_distance: json::extensions::material::AttenuationDistance(attenuation_distance),
            attenuation_color: json::extensions::material::AttenuationColor(attenuation_color),
            extras: Default::default(),
        });
    }
    mat_extensions.emissive_strength = emissive_strength_ext;
    let mat_extensions = if mat_extensions.transmission.is_some()
        || mat_extensions.ior.is_some()
        || mat_extensions.volume.is_some()
        || mat_extensions.emissive_strength.is_some()
    {
        Some(mat_extensions)
    } else {
        None
    };

    let mat_extras = {
        let mut map = serde_json::Map::new();
        if let Some(m) = mtl_sub {
            if !m.name.is_empty() {
                map.insert("source_name".into(), serde_json::json!(m.name));
            }
            if !m.shader.is_empty() {
                map.insert("shader".into(), serde_json::json!(m.shader));
            }
            if m.diffuse != [1.0, 1.0, 1.0] {
                map.insert("diffuse".into(), serde_json::json!(m.diffuse));
            }
            if m.specular != [0.04, 0.04, 0.04] {
                map.insert("specular".into(), serde_json::json!(m.specular));
            }
            if (m.shininess - 128.0).abs() > f32::EPSILON {
                map.insert("shininess".into(), serde_json::json!(m.shininess));
            }
            if m.emissive != [0.0, 0.0, 0.0] {
                map.insert("emissive".into(), serde_json::json!(m.emissive));
            }
            if m.glow != 0.0 {
                map.insert("glow".into(), serde_json::json!(m.glow));
            }
            if (m.opacity - 1.0).abs() > f32::EPSILON {
                map.insert("opacity".into(), serde_json::json!(m.opacity));
            }
            if !m.surface_type.is_empty() {
                map.insert("surface_type".into(), serde_json::json!(m.surface_type));
            }
            if !m.string_gen_mask.is_empty() {
                map.insert("string_gen_mask".into(), serde_json::json!(m.string_gen_mask));
            }
            if let Some(ref tex) = m.diffuse_tex {
                map.insert("diffuse_tex".into(), serde_json::json!(tex));
            }
            if let Some(ref tex) = m.normal_tex {
                map.insert("normal_tex".into(), serde_json::json!(tex));
            }
            if m.palette_tint > 0 {
                map.insert("palette_tint".into(), serde_json::json!(m.palette_tint));
            }
            if !m.layers.is_empty() {
                let layers_json: Vec<serde_json::Value> = m.layers.iter().map(|l| {
                    let mut lm = serde_json::Map::new();
                    if !l.path.is_empty() {
                        lm.insert("path".into(), serde_json::json!(l.path));
                    }
                    if l.tint_color != [1.0, 1.0, 1.0] {
                        lm.insert("tint_color".into(), serde_json::json!(l.tint_color));
                    }
                    if l.palette_tint > 0 {
                        lm.insert("palette_tint".into(), serde_json::json!(l.palette_tint));
                    }
                    if (l.uv_tiling - 1.0).abs() > f32::EPSILON {
                        lm.insert("uv_tiling".into(), serde_json::json!(l.uv_tiling));
                    }
                    serde_json::Value::Object(lm)
                }).collect();
                map.insert("layers".into(), serde_json::Value::Array(layers_json));
            }

            let decoded_flags = m.decoded_string_gen_mask();
            let semantic_slots = m.semantic_texture_slots();
            let mut semantic = serde_json::Map::new();
            let (activation_state, activation_reason) = if m.is_nodraw {
                ("inactive", "nodraw")
            } else if m.should_hide() {
                ("inactive", "semantic_hidden")
            } else if m.is_decal() && base_color_texture_idx.is_none() {
                ("inactive", "missing_base_color_texture")
            } else {
                ("active", "visible")
            };
            semantic.insert("shader_family".into(), serde_json::json!(m.shader_family().as_str()));
            semantic.insert("is_hidden".into(), serde_json::json!(m.should_hide()));
            semantic.insert("is_decal".into(), serde_json::json!(m.is_decal()));
            semantic.insert("is_glass".into(), serde_json::json!(m.is_glass()));
            if !bundled_fallbacks.is_empty() {
                semantic.insert(
                    "bundled_fallbacks".into(),
                    serde_json::Value::Array(
                        bundled_fallbacks
                            .iter()
                            .cloned()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            semantic.insert(
                "activation_state".into(),
                serde_json::json!({
                    "state": activation_state,
                    "reason": activation_reason,
                }),
            );
            semantic.insert(
                "decoded_string_gen_mask".into(),
                serde_json::json!({
                    "tokens": decoded_flags.tokens,
                    "has_decal": decoded_flags.has_decal,
                    "has_parallax_occlusion_mapping": decoded_flags.has_parallax_occlusion_mapping,
                    "has_stencil_map": decoded_flags.has_stencil_map,
                    "has_iridescence": decoded_flags.has_iridescence,
                    "has_vertex_colors": decoded_flags.has_vertex_colors,
                }),
            );
            if !semantic_slots.is_empty() {
                let slots_json: Vec<serde_json::Value> = semantic_slots
                    .iter()
                    .map(|binding| {
                        serde_json::json!({
                            "slot": binding.slot,
                            "path": binding.path,
                            "is_virtual": binding.is_virtual,
                            "role": binding.role.as_str(),
                        })
                    })
                    .collect();
                semantic.insert("texture_slots".into(), serde_json::Value::Array(slots_json));
            }
            if !m.public_params.is_empty() {
                let mut params = serde_json::Map::new();
                for param in &m.public_params {
                    params.insert(param.name.clone(), string_value_to_json(&param.value));
                }
                semantic.insert("public_params".into(), serde_json::Value::Object(params));
            }
            let mut palette_semantic = serde_json::Map::new();
            if let Some(source_name) = palette.and_then(|palette| palette.source_name.as_ref()) {
                palette_semantic.insert("source_name".into(), serde_json::json!(source_name));
            }
            match m.palette_tint {
                1..=3 => {
                    if let Some(channel_name) = palette_channel_name(m.palette_tint) {
                        palette_semantic.insert(
                            "material_channel".into(),
                            serde_json::json!({
                                "index": m.palette_tint,
                                "name": channel_name,
                            }),
                        );
                    }
                }
                _ if m.is_glass() => {
                    palette_semantic.insert(
                        "material_channel".into(),
                        serde_json::json!({
                            "index": 0,
                            "name": "glass",
                        }),
                    );
                }
                _ => {}
            }
            let resolved_palette_color = palette.and_then(|palette| match m.palette_tint {
                1 => Some(palette.primary),
                2 => Some(palette.secondary),
                3 => Some(palette.tertiary),
                _ if m.is_glass() => Some(palette.glass),
                _ => None,
            });
            if let Some(color) = resolved_palette_color {
                palette_semantic.insert("resolved_color".into(), serde_json::json!(color));
            }
            let palette_layers: Vec<serde_json::Value> = m
                .layers
                .iter()
                .enumerate()
                .filter_map(|(index, layer)| {
                    let channel_name = palette_channel_name(layer.palette_tint)?;
                    let resolved_color = palette.map(|palette| match layer.palette_tint {
                        1 => palette.primary,
                        2 => palette.secondary,
                        3 => palette.tertiary,
                        _ => unreachable!(),
                    });
                    let mut layer_json = serde_json::Map::new();
                    layer_json.insert("index".into(), serde_json::json!(index));
                    layer_json.insert(
                        "channel".into(),
                        serde_json::json!({
                            "index": layer.palette_tint,
                            "name": channel_name,
                        }),
                    );
                    if let Some(color) = resolved_color {
                        layer_json.insert("resolved_color".into(), serde_json::json!(color));
                    }
                    Some(serde_json::Value::Object(layer_json))
                })
                .collect();
            if !palette_layers.is_empty() {
                palette_semantic.insert("layers".into(), serde_json::Value::Array(palette_layers));
            }
            if !palette_semantic.is_empty() {
                semantic.insert("palette".into(), serde_json::Value::Object(palette_semantic));
            }

            let mut material_set_identity = serde_json::Map::new();
            if let Some(source_path) = materials.and_then(|mtl| mtl.source_path.as_ref()) {
                material_set_identity.insert("source_path".into(), serde_json::json!(source_path));
                if let Some(file_name) = source_path.rsplit(['\\', '/']).next() {
                    material_set_identity.insert(
                        "source_stem".into(),
                        serde_json::json!(file_name.strip_suffix(".mtl").unwrap_or(file_name)),
                    );
                }
            }
            material_set_identity.insert("submaterial_index".into(), serde_json::json!(sub.material_id));
            if !m.name.is_empty() {
                material_set_identity.insert("submaterial_name".into(), serde_json::json!(m.name));
            }
            if let Some(slot_name) = sub.material_name.as_ref() {
                material_set_identity.insert("slot_name".into(), serde_json::json!(slot_name));
            }
            if !material_set_identity.is_empty() {
                semantic.insert(
                    "material_set_identity".into(),
                    serde_json::Value::Object(material_set_identity),
                );
            }
            if !m.layers.is_empty() {
                let manifest: Vec<serde_json::Value> = m
                    .layers
                    .iter()
                    .enumerate()
                    .map(|(index, layer)| {
                        serde_json::json!({
                            "index": index,
                            "path": layer.path,
                            "tint_color": layer.tint_color,
                            "palette_tint": layer.palette_tint,
                            "uv_tiling": layer.uv_tiling,
                        })
                    })
                    .collect();
                semantic.insert("layer_manifest".into(), serde_json::Value::Array(manifest));
            }
            map.insert("semantic".into(), serde_json::Value::Object(semantic));
        }
        if map.is_empty() {
            None
        } else {
            map_to_raw_value(map).map(Into::into)
        }
    };

    let material = json::Material {
        pbr_metallic_roughness: json::material::PbrMetallicRoughness {
            base_color_factor: json::material::PbrBaseColorFactor(base_color_factor),
            metallic_factor: json::material::StrengthFactor(metallic),
            roughness_factor: json::material::StrengthFactor(roughness_factor),
            base_color_texture,
            metallic_roughness_texture: roughness_texture,
            extensions: None,
            extras: Default::default(),
        },
        name: mat_name,
        alpha_cutoff,
        alpha_mode: Checked::Valid(alpha_mode),
        double_sided,
        normal_texture,
        occlusion_texture,
        emissive_texture,
        emissive_factor: json::material::EmissiveFactor(emissive),
        extensions: mat_extensions,
        extras: mat_extras,
    };

    let identity = MaterialIdentity {
        material_json: serde_json::to_string(&material)
            .unwrap_or_else(|_| format!("{:?}", material)),
    };

    BuiltMaterial { material, identity }
}

// ── Assembled-path helpers ──────────────────────────────────────────────────

/// Hash-based texture dedup.
/// Uses DefaultHasher over (len, first_64_bytes, last_64_bytes) for low-collision keys.
fn hash_texture_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn pack_texture_deduped(
    png_bytes: Option<&[u8]>,
    bin: &mut Vec<u8>,
    cache: &mut TexCache,
) -> Option<TextureRegion> {
    let png = png_bytes?;
    let key = PackedTextureKey {
        byte_len: png.len(),
        content_hash: hash_texture_bytes(png),
    };

    if let Some(&cached) = cache.get(&key) {
        return Some(cached);
    }

    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let offset = bin.len();
    bin.extend_from_slice(png);
    let region = TextureRegion {
        offset,
        len: png.len(),
    };
    cache.insert(key, region);
    Some(region)
}

