//! GlbBuilder: centralized GLB construction state.
//!
//! GlbBuilder is used by `write_glb` for all mesh packing, texture embedding,
//! material creation, and NMC node hierarchy building. There is a single unified
//! code path for all export scenarios — single entities, assembled scenes, and interiors.

use std::collections::{BTreeMap, HashMap};

use gltf_json as json;
use json::validation::Checked;

use crate::error::Error;
use crate::nmc::NodeMeshCombo;
use crate::types::{MaterialTextures, Mesh, SubMesh};

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
    pub tex_cache: TexCache,
    pub mat_dedup: HashMap<String, u32>,
    /// Maps lowercased node/entity name → glTF node index.
    pub node_name_to_idx: HashMap<String, u32>,
    /// Completed animation clips ready for glTF serialization.
    pub animations_json: Vec<json::Animation>,
}

/// Texture dedup cache: maps (len, first_8_bytes_hash) → (offset, len) in binary buffer.
pub(crate) type TexCache = HashMap<(usize, u64), (usize, usize)>;

/// Result of packing a single entity's mesh into the builder.
pub(crate) struct PackedMeshInfo {
    pub mesh_idx: u32,
    pub pos_accessor_idx: u32,
    pub uv_accessor_idx: Option<u32>,
    pub normal_accessor_idx: Option<u32>,
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
            mat_dedup: HashMap::new(),
            node_name_to_idx: HashMap::new(),
            animations_json: Vec::new(),
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
        load_textures: &mut dyn FnMut(Option<&crate::mtl::MtlFile>) -> Option<crate::types::MaterialTextures>,
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
            let textures = load_textures(child.materials.as_ref());
            let packed = self.pack_mesh(
                &child.mesh,
                child.materials.as_ref(),
                textures.as_ref(),
                child.palette.as_ref().or(fallback_palette),
                Some(&child.entity_name),
                material_mode,
            );
            drop(textures);
            child.mesh.positions = Vec::new();
            child.mesh.uvs = None;
            child.mesh.normals = None;
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
                        normal_accessor_idx: None,
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
        load_textures: &mut dyn FnMut(Option<&crate::mtl::MtlFile>) -> Option<crate::types::MaterialTextures>,
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
        let mut packed_cache: HashMap<(usize, u64), u32> = HashMap::new();

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
                let cache_key = (mesh_array_idx, palette_key);
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
                    let textures = load_textures(mtl.as_ref());
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

        // Embed textures with hash-based dedup
        let mut texture_regions: Vec<Option<(usize, usize)>> = Vec::new();
        let mut normal_regions: Vec<Option<(usize, usize)>> = Vec::new();
        let mut roughness_regions: Vec<Option<(usize, usize)>> = Vec::new();
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
        }

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
            &mut self.buffer_views, &mut self.accessors, pos_offset, pos_len,
            mesh.positions.len(), json::accessor::Type::Vec3, Some((&pos_min, &pos_max)),
        ).unwrap_or(0);

        let uv_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views, &mut self.accessors, uv_offset, uv_len,
            mesh.uvs.as_ref().map_or(0, |v| v.len()), json::accessor::Type::Vec2, None,
        );
        let normal_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views, &mut self.accessors, normal_offset, normal_len,
            mesh.normals.as_ref().map_or(0, |v| v.len()), json::accessor::Type::Vec3, None,
        );
        let tangent_accessor_idx = super::add_vertex_accessor(
            &mut self.buffer_views, &mut self.accessors, tangent_offset, tangent_len,
            mesh.tangents.as_ref().map_or(0, |v| v.len()), json::accessor::Type::Vec4, None,
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

        // Texture → glTF mappings
        let has_any_texture = texture_regions.iter()
            .chain(normal_regions.iter())
            .chain(roughness_regions.iter())
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

        let mut tex_json_dedup: HashMap<(usize, usize), u32> = HashMap::new();
        let submaterial_texture_idx = regions_to_gltf_textures_deduped(&texture_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut tex_json_dedup);
        let submaterial_normal_idx = regions_to_gltf_textures_deduped(&normal_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut tex_json_dedup);
        let submaterial_roughness_idx = regions_to_gltf_textures_deduped(&roughness_regions, &mut self.buffer_views, &mut self.images_json, &mut self.textures_json, &mut tex_json_dedup);

        // Build materials with dedup
        let submesh_mat_indices = build_materials(
            &mesh.submeshes,
            materials,
            palette,
            &submaterial_texture_idx,
            &submaterial_normal_idx,
            &submaterial_roughness_idx,
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
            let idx_byte_offset = if use_u16 {
                sub.first_index as u64 * 2
            } else {
                sub.first_index as u64 * 4
            };

            let acc_idx = self.accessors.len() as u32;
            submesh_idx_accessors.push(Some(acc_idx));
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

            let mut attributes = BTreeMap::new();
            attributes.insert(
                Checked::Valid(json::mesh::Semantic::Positions),
                json::Index::new(pos_accessor_idx),
            );
            if let Some(uv_idx) = uv_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::TexCoords(0)),
                    json::Index::new(uv_idx),
                );
            }
            if let Some(normal_idx) = normal_accessor_idx {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::Normals),
                    json::Index::new(normal_idx),
                );
            }
            if let Some(tangent_idx) = tangent_accessor_idx {
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

        PackedMeshInfo {
            mesh_idx,
            pos_accessor_idx,
            uv_accessor_idx,
            normal_accessor_idx,
            tangent_accessor_idx,
            submesh_mat_indices,
            submesh_idx_accessors,
        }
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
                    if let Some(n) = packed.normal_accessor_idx {
                        attributes.insert(
                            Checked::Valid(json::mesh::Semantic::Normals),
                            json::Index::new(n),
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

    // ─── Animation export helpers ────────────────────────────────────────────

    /// Decompose a node's 4x4 column-major matrix into TRS components.
    ///
    /// glTF requires TRS (not matrix) on nodes targeted by animation channels.
    /// If the node already has rotation/translation set, or has no matrix, this is a no-op.
    /// Decompose an animated node's matrix into TRS (required by glTF spec).
    fn prepare_node_for_animation(&mut self, node_idx: usize) {
        let node = &self.nodes_json[node_idx];
        if node.rotation.is_some() || node.translation.is_some() {
            return;
        }
        let Some(matrix) = node.matrix else { return };

        let m = glam::Mat4::from_cols_array(&matrix);
        let (scale, rot, trans) = m.to_scale_rotation_translation();

        let node = &mut self.nodes_json[node_idx];
        node.matrix = None;
        node.translation = Some(trans.into());
        node.rotation = Some(json::scene::UnitQuaternion([rot.x, rot.y, rot.z, rot.w]));
        if (scale - glam::Vec3::ONE).length() > 1e-5 {
            node.scale = Some(scale.into());
        }
    }

    /// Write an array of f32 scalar values to the binary buffer and create an accessor.
    /// Returns the accessor index. Used for animation time tracks.
    fn write_time_accessor(&mut self, times: &[f32]) -> u32 {
        let byte_offset = self.bin.len();
        for &t in times {
            self.bin.extend_from_slice(&t.to_le_bytes());
        }
        // Pad to 4-byte alignment (f32 is already 4 bytes, but be safe).
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let byte_length = times.len() * 4;
        let min = times.iter().copied().reduce(f32::min).unwrap_or(0.0);
        let max = times.iter().copied().reduce(f32::max).unwrap_or(0.0);
        super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            byte_offset,
            byte_length,
            times.len(),
            json::accessor::Type::Scalar,
            Some((&[min], &[max])),
        )
        .expect("write_time_accessor: times must not be empty")
    }

    /// Write an array of [f32; 4] values and create a VEC4 accessor. Returns accessor index.
    fn write_vec4_accessor(&mut self, values: &[[f32; 4]]) -> u32 {
        let byte_offset = self.bin.len();
        for v in values {
            for &f in v {
                self.bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let byte_length = values.len() * 16;
        super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            byte_offset,
            byte_length,
            values.len(),
            json::accessor::Type::Vec4,
            None,
        )
        .expect("write_vec4_accessor: values must not be empty")
    }

    /// Write an array of [f32; 3] values and create a VEC3 accessor. Returns accessor index.
    fn write_vec3_accessor(&mut self, values: &[[f32; 3]]) -> u32 {
        let byte_offset = self.bin.len();
        for v in values {
            for &f in v {
                self.bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let byte_length = values.len() * 12;
        super::add_vertex_accessor(
            &mut self.buffer_views,
            &mut self.accessors,
            byte_offset,
            byte_length,
            values.len(),
            json::accessor::Type::Vec3,
            None,
        )
        .expect("write_vec3_accessor: values must not be empty")
    }

    /// Add animation clips to the GLB.
    ///
    /// Builds a CRC32 reverse-map from `node_name_to_idx`, then for each clip
    /// writes time/rotation/translation accessors and creates glTF animation
    /// samplers + channels targeting the matched skeleton/NMC nodes.
    pub fn add_animations(&mut self, clips: &[crate::animation::dba::AnimationClip]) {
        if clips.is_empty() {
            return;
        }

        // Build reverse map: bone_hash (CRC32 of ORIGINAL CASE name) → glTF node index.
        // DBA bone hashes use CRC32 of the case-sensitive bone name, NOT lowercase.
        let mut hash_to_node: HashMap<u32, u32> = HashMap::new();
        for (&idx_val) in self.node_name_to_idx.values() {
            if let Some(ref name) = self.nodes_json[idx_val as usize].name {
                let hash = crc32fast::hash(name.as_bytes());
                hash_to_node.insert(hash, idx_val);
            }
        }

        for clip in clips {
            let fps = if clip.fps > 0.0 { clip.fps } else { 30.0 };
            let mut samplers: Vec<json::animation::Sampler> = Vec::new();
            let mut channels: Vec<json::animation::Channel> = Vec::new();
            let mut matched = 0u32;
            let mut skipped = 0u32;

            for bone_channel in &clip.channels {
                let Some(&node_idx) = hash_to_node.get(&bone_channel.bone_hash) else {
                    skipped += 1;
                    continue;
                };
                matched += 1;

                // Decompose node's matrix to TRS (required by glTF for animation targets).
                // Both rotations and positions are ABSOLUTE — the SNORM position decode
                // formula (u16 * scale + offset) already includes the rest position offset.
                self.prepare_node_for_animation(node_idx as usize);

                // Rotation channel — raw absolute values
                if !bone_channel.rotations.is_empty() {
                    let times: Vec<f32> = bone_channel
                        .rotations
                        .iter()
                        .map(|kf| kf.time / fps)
                        .collect();
                    let values: Vec<[f32; 4]> = bone_channel
                        .rotations
                        .iter()
                        .map(|kf| kf.value)
                        .collect();

                    let time_acc = self.write_time_accessor(&times);
                    let rot_acc = self.write_vec4_accessor(&values);

                    let sampler_idx = samplers.len() as u32;
                    samplers.push(json::animation::Sampler {
                        input: json::Index::new(time_acc),
                        interpolation: Checked::Valid(json::animation::Interpolation::Linear),
                        output: json::Index::new(rot_acc),
                        extensions: None,
                        extras: Default::default(),
                    });
                    channels.push(json::animation::Channel {
                        sampler: json::Index::new(sampler_idx),
                        target: json::animation::Target {
                            node: json::Index::new(node_idx),
                            path: Checked::Valid(json::animation::Property::Rotation),
                            extensions: None,
                            extras: Default::default(),
                        },
                        extensions: None,
                        extras: Default::default(),
                    });
                }

                // Translation channel — absolute values (SNORM offset includes rest pos)
                if !bone_channel.positions.is_empty() {
                    let times: Vec<f32> = bone_channel
                        .positions
                        .iter()
                        .map(|kf| kf.time / fps)
                        .collect();
                    let values: Vec<[f32; 3]> = bone_channel
                        .positions
                        .iter()
                        .map(|kf| kf.value)
                        .collect();

                    let time_acc = self.write_time_accessor(&times);
                    let pos_acc = self.write_vec3_accessor(&values);

                    let sampler_idx = samplers.len() as u32;
                    samplers.push(json::animation::Sampler {
                        input: json::Index::new(time_acc),
                        interpolation: Checked::Valid(json::animation::Interpolation::Linear),
                        output: json::Index::new(pos_acc),
                        extensions: None,
                        extras: Default::default(),
                    });
                    channels.push(json::animation::Channel {
                        sampler: json::Index::new(sampler_idx),
                        target: json::animation::Target {
                            node: json::Index::new(node_idx),
                            path: Checked::Valid(json::animation::Property::Translation),
                            extensions: None,
                            extras: Default::default(),
                        },
                        extensions: None,
                        extras: Default::default(),
                    });
                }
            }

            log::info!(
                "animation '{}': {} bones matched, {} skipped, {} samplers",
                clip.name,
                matched,
                skipped,
                samplers.len()
            );

            if !channels.is_empty() {
                self.animations_json.push(json::Animation {
                    name: Some(clip.name.clone()),
                    channels,
                    samplers,
                    extensions: None,
                    extras: Default::default(),
                });
            }
        }
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
        if self.materials_json.iter().any(|m| m.extensions.is_some()) {
            extensions_used.push("KHR_materials_transmission".to_string());
            extensions_used.push("KHR_materials_ior".to_string());
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
            animations: self.animations_json,
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

/// Serialize a glTF Root + binary buffer into a GLB byte vector.
fn serialize_glb(root: &json::Root, bin: &[u8]) -> Result<Vec<u8>, Error> {
    let json_bytes = serde_json::to_vec(root).map_err(|e| Error::Gltf(e.to_string()))?;

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

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Convert texture regions to glTF texture entries with dedup by (offset, len).
fn regions_to_gltf_textures_deduped(
    regions: &[Option<(usize, usize)>],
    buffer_views: &mut Vec<json::buffer::View>,
    images_json: &mut Vec<json::Image>,
    textures_json: &mut Vec<json::Texture>,
    dedup: &mut HashMap<(usize, usize), u32>,
) -> Vec<Option<u32>> {
    regions.iter().map(|region| {
        let &(offset, len) = region.as_ref()?;
        let key = (offset, len);
        if let Some(&idx) = dedup.get(&key) {
            return Some(idx);
        }
        let bv_idx = buffer_views.len() as u32;
        buffer_views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_offset: Some(json::validation::USize64(offset as u64)),
            byte_length: json::validation::USize64(len as u64),
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
        dedup.insert(key, tex_idx);
        Some(tex_idx)
    }).collect()
}

/// Build deduplicated glTF materials for a set of submeshes.
fn build_materials(
    submeshes: &[SubMesh],
    materials: Option<&crate::mtl::MtlFile>,
    palette: Option<&crate::mtl::TintPalette>,
    submaterial_texture_idx: &[Option<u32>],
    submaterial_normal_idx: &[Option<u32>],
    submaterial_roughness_idx: &[Option<u32>],
    materials_json: &mut Vec<json::Material>,
    mat_dedup: &mut HashMap<String, u32>,
    experimental_textures: bool,
) -> Vec<u32> {
    submeshes.iter().map(|sub| {
        let mat = build_material(sub, materials, palette, submaterial_texture_idx, submaterial_normal_idx, submaterial_roughness_idx, experimental_textures);
        let key = format!(
            "{}|{:?}|{:?}|{:?}|{}|{:?}|{:?}|{:?}|{:?}",
            mat.name.as_deref().unwrap_or(""),
            mat.pbr_metallic_roughness.base_color_factor,
            mat.pbr_metallic_roughness.metallic_factor,
            mat.pbr_metallic_roughness.roughness_factor,
            mat.double_sided,
            mat.alpha_mode,
            mat.alpha_cutoff,
            mat.pbr_metallic_roughness.base_color_texture.as_ref().map(|t| t.index),
            mat.normal_texture.as_ref().map(|t| t.index),
        );
        *mat_dedup.entry(key).or_insert_with(|| {
            let idx = materials_json.len() as u32;
            materials_json.push(mat);
            idx
        })
    }).collect()
}

/// Build a single glTF material from a submesh's material properties.
fn build_material(
    sub: &SubMesh,
    materials: Option<&crate::mtl::MtlFile>,
    palette: Option<&crate::mtl::TintPalette>,
    submaterial_texture_idx: &[Option<u32>],
    submaterial_normal_idx: &[Option<u32>],
    submaterial_roughness_idx: &[Option<u32>],
    experimental_textures: bool,
) -> json::Material {
    let mtl_sub = materials.and_then(|m| m.materials.get(sub.material_id as usize));

    let (base_color, alpha_mode, alpha_cutoff, double_sided, mat_name) = if let Some(m) = mtl_sub {
        let ac = m.alpha_config();
        let alpha = m.opacity;
        let (gltf_alpha_mode, cutoff) = match ac {
            crate::mtl::AlphaConfig::Opaque => (json::material::AlphaMode::Opaque, None),
            crate::mtl::AlphaConfig::Blend => (json::material::AlphaMode::Blend, None),
            crate::mtl::AlphaConfig::Mask(v) => (
                json::material::AlphaMode::Mask,
                Some(json::material::AlphaCutoff(v)),
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
            json::material::PbrBaseColorFactor([color[0], color[1], color[2], alpha]),
            gltf_alpha_mode,
            cutoff,
            m.is_double_sided(),
            name,
        )
    } else {
        (
            json::material::PbrBaseColorFactor([0.8, 0.8, 0.8, 1.0]),
            json::material::AlphaMode::Opaque,
            None,
            false,
            sub.material_name.clone(),
        )
    };

    let base_color_texture = submaterial_texture_idx
        .get(sub.material_id as usize).copied().flatten()
        .map(|tex_idx| json::texture::Info {
            index: json::Index::new(tex_idx), tex_coord: 0,
            extensions: None, extras: Default::default(),
        });
    // Only apply per-pixel normal/roughness when the material has a direct TexSlot1 diffuse.
    // When textures come from MatLayer .mtl files (HardSurface/LayerBlend shaders), they are
    // tileable detail patterns for one layer of CryEngine's multi-layer blending system.
    // Applied standalone in glTF, the roughness variation creates extreme specular noise
    // and the normals add unwanted surface perturbation. Use scalar values instead.
    let allow_detail_textures = experimental_textures || mtl_sub.is_some_and(|m| m.diffuse_tex.is_some());
    let normal_texture = if allow_detail_textures {
        submaterial_normal_idx
            .get(sub.material_id as usize).copied().flatten()
            .map(|tex_idx| json::material::NormalTexture {
                index: json::Index::new(tex_idx), scale: 1.0, tex_coord: 0,
                extensions: None, extras: Default::default(),
            })
    } else {
        None
    };

    let (roughness, metallic) = mtl_sub.map(|m| (m.roughness(), m.metallic())).unwrap_or((0.5, 0.0));
    let emissive = mtl_sub.map(|m| m.emissive_factor()).unwrap_or([0.0, 0.0, 0.0]);
    let is_glass = mtl_sub.map(|m| m.is_glass()).unwrap_or(false);

    let mat_extensions = if is_glass {
        Some(json::extensions::material::Material {
            transmission: Some(json::extensions::material::Transmission {
                transmission_factor: json::extensions::material::TransmissionFactor(0.95),
                transmission_texture: None,
                extras: Default::default(),
            }),
            ior: Some(json::extensions::material::Ior {
                ior: json::extensions::material::IndexOfRefraction(1.5),
                extras: Default::default(),
            }),
        })
    } else {
        None
    };

    // Per-pixel roughness creates visible specular noise in glTF PBR, especially on
    // dark surfaces viewed up close. Only enable with --experimental-textures.
    // Scalar roughness from MTL Shininess provides clean uniform glossiness.
    let roughness_texture = if experimental_textures {
        submaterial_roughness_idx
            .get(sub.material_id as usize).copied().flatten()
            .map(|tex_idx| json::texture::Info {
                index: json::Index::new(tex_idx), tex_coord: 0,
                extensions: None, extras: Default::default(),
            })
    } else {
        None
    };
    let roughness_factor = if roughness_texture.is_some() { 1.0 } else { roughness };

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
        }
        if map.is_empty() {
            None
        } else {
            map_to_raw_value(map).map(Into::into)
        }
    };

    json::Material {
        pbr_metallic_roughness: json::material::PbrMetallicRoughness {
            base_color_factor: base_color,
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
        occlusion_texture: None,
        emissive_texture: None,
        emissive_factor: json::material::EmissiveFactor(emissive),
        extensions: mat_extensions,
        extras: mat_extras,
    }
}

// ── Assembled-path helpers ──────────────────────────────────────────────────

/// Hash-based texture dedup.
/// Uses DefaultHasher over (len, first_64_bytes, last_64_bytes) for low-collision keys.
fn pack_texture_deduped(
    png_bytes: Option<&[u8]>,
    bin: &mut Vec<u8>,
    cache: &mut TexCache,
) -> Option<(usize, usize)> {
    let png = png_bytes?;
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        png.len().hash(&mut h);
        png[..png.len().min(64)].hash(&mut h);
        if png.len() > 64 {
            png[png.len() - 64..].hash(&mut h);
        }
        h.finish()
    };
    let key = (png.len(), hash);

    if let Some(&cached) = cache.get(&key) {
        return Some(cached);
    }

    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let offset = bin.len();
    bin.extend_from_slice(png);
    let region = (offset, png.len());
    cache.insert(key, region);
    Some(region)
}

