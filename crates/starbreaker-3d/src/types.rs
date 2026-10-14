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
    /// Per-vertex skinning (see [`crate::mesh_rig::Skin`]).
    pub skin: Option<crate::mesh_rig::Skin>,
    /// Named per-vertex deltas (see [`crate::mesh_rig::MorphTarget`]).
    pub morph_targets: Vec<crate::mesh_rig::MorphTarget>,
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
    ///
    /// Every per-vertex stream stays as long as `positions`: UVs and colours
    /// are padded (zero UV, opaque white), a side without normals gets
    /// geometry-derived smooth normals (as [`build_mesh`] does for a
    /// stream-less mesh), and tangents are kept only when both sides have
    /// them (consumers derive tangents from normals + UVs). Skins and morph
    /// targets merge by joint / target name (see [`crate::mesh_rig`]).
    pub fn merge_from(&mut self, other: Mesh) {
        // Parity guard: no `..` — a new Mesh field must be handled here.
        let Mesh {
            positions,
            indices,
            uvs,
            secondary_uvs,
            normals,
            tangents,
            colors,
            submeshes,
            model_min,
            model_max,
            scaling_min,
            scaling_max,
            skin,
            morph_targets,
        } = other;
        let vertex_offset = self.positions.len() as u32;
        let index_offset = self.indices.len() as u32;
        let total = self.positions.len() + positions.len();

        self.skin = crate::mesh_rig::merge_skins(self.skin.take(), skin, self.positions.len(), positions.len());
        crate::mesh_rig::merge_morph_targets(&mut self.morph_targets, morph_targets, self.positions.len(), positions.len());

        self.normals = match (self.normals.take(), normals) {
            (None, None) => None,
            (mine, theirs) => {
                let mut merged = mine.unwrap_or_else(|| compute_smooth_normals(&self.positions, &self.indices));
                merged.extend(theirs.unwrap_or_else(|| compute_smooth_normals(&positions, &indices)));
                Some(merged)
            }
        };
        self.tangents = match (self.tangents.take(), tangents) {
            (Some(mut mine), Some(theirs)) => {
                mine.extend(theirs);
                Some(mine)
            }
            _ => None,
        };
        merge_padded(&mut self.uvs, uvs, vertex_offset as usize, total, [0.0, 0.0]);
        merge_padded(&mut self.secondary_uvs, secondary_uvs, vertex_offset as usize, total, [0.0, 0.0]);
        merge_padded(&mut self.colors, colors, vertex_offset as usize, total, [255, 255, 255, 255]);

        self.positions.extend(positions);
        self.indices.extend(indices.iter().map(|i| i + vertex_offset));
        for mut sm in submeshes {
            sm.first_index += index_offset;
            sm.first_vertex += vertex_offset;
            self.submeshes.push(sm);
        }
        for i in 0..3 {
            self.model_min[i] = self.model_min[i].min(model_min[i]);
            self.model_max[i] = self.model_max[i].max(model_max[i]);
            self.scaling_min[i] = self.scaling_min[i].min(scaling_min[i]);
            self.scaling_max[i] = self.scaling_max[i].max(scaling_max[i]);
        }
    }
}

impl Mesh {
    /// A mesh with no geometry (scene-graph anchors, placeholder children).
    pub fn empty() -> Mesh {
        Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: Vec::new(),
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: Vec::new(),
        }
    }

    /// The vertices referenced by `submesh_indices`, reindexed from 0, with
    /// every per-vertex stream carried and the model bbox recomputed. Returns
    /// the old → new vertex index map (for remapping side data such as
    /// vertex groups).
    pub fn subset(&self, submesh_indices: &[usize]) -> (Mesh, std::collections::HashMap<u32, u32>) {
        // Parity guard: no `..` — a new Mesh field must be handled here.
        let Mesh {
            positions,
            indices,
            uvs,
            secondary_uvs,
            normals,
            tangents,
            colors,
            submeshes,
            model_min: _,  // recomputed from the subset
            model_max: _,  // recomputed from the subset
            scaling_min,
            scaling_max,
            skin,
            morph_targets,
        } = self;
        fn take<T: Copy>(src: &Option<Vec<T>>, order: &[u32]) -> Option<Vec<T>> {
            src.as_ref().map(|s| order.iter().map(|&o| s[o as usize]).collect())
        }

        let mut old_to_new: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut order: Vec<u32> = Vec::new();
        let mut new_indices = Vec::new();
        let mut new_submeshes = Vec::new();
        for &submesh_index in submesh_indices {
            let Some(source) = submeshes.get(submesh_index) else { continue };
            let first_index = new_indices.len() as u32;
            let start = (source.first_index as usize).min(indices.len());
            let end = (start + source.num_indices as usize).min(indices.len());
            for tri in indices[start..end].chunks_exact(3) {
                // A triangle with an out-of-range index is dropped whole so the
                // index stream stays triangle-aligned.
                if tri.iter().any(|&i| (i as usize) >= positions.len()) {
                    continue;
                }
                for &old in tri {
                    let new = *old_to_new.entry(old).or_insert_with(|| {
                        order.push(old);
                        order.len() as u32 - 1
                    });
                    new_indices.push(new);
                }
            }
            let num_indices = new_indices.len() as u32 - first_index;
            if num_indices == 0 {
                continue;
            }
            // Parity guard: no `..` — a new SubMesh field must be handled here.
            let SubMesh {
                material_name,
                material_id,
                source_material_id,
                first_index: _,  // reindexed
                num_indices: _,  // reindexed
                first_vertex: _, // recomputed below
                num_vertices: _, // recomputed below
                node_parent_index,
            } = source;
            let (lo, hi) = new_indices[first_index as usize..]
                .iter()
                .fold((u32::MAX, 0), |(lo, hi), &i| (lo.min(i), hi.max(i)));
            new_submeshes.push(SubMesh {
                material_name: material_name.clone(),
                material_id: *material_id,
                source_material_id: *source_material_id,
                first_index,
                num_indices,
                first_vertex: lo,
                num_vertices: hi - lo + 1,
                node_parent_index: *node_parent_index,
            });
        }

        let new_positions: Vec<[f32; 3]> = order.iter().map(|&o| positions[o as usize]).collect();
        let (model_min, model_max) = bounds(&new_positions);
        let mesh = Mesh {
            positions: new_positions,
            indices: new_indices,
            uvs: take(uvs, &order),
            secondary_uvs: take(secondary_uvs, &order),
            normals: take(normals, &order),
            tangents: take(tangents, &order),
            colors: take(colors, &order),
            submeshes: new_submeshes,
            model_min,
            model_max,
            scaling_min: *scaling_min,
            scaling_max: *scaling_max,
            skin: skin.as_ref().map(|s| s.gather(&order)),
            morph_targets: morph_targets.iter().map(|m| m.gather(&order)).collect(),
        };
        (mesh, old_to_new)
    }

    /// Apply a rigid transform to every spatial stream: positions get the
    /// full affine; normals (inverse-transpose), tangent xyz and morph deltas
    /// only the linear part; tangent handedness (w) is kept. The model bbox
    /// is transformed by its 8 corners; the scaling bbox (dequantization
    /// only) is left as is.
    pub fn transform(&mut self, affine: glam::Affine3A) {
        // Parity guard: no `..` — every spatial Mesh field must be transformed here.
        let Mesh {
            positions,
            normals,
            tangents,
            morph_targets,
            model_min,
            model_max,
            scaling_min: _, // dequantization only
            scaling_max: _,
            indices: _,       // not spatial
            uvs: _,           // not spatial
            secondary_uvs: _, // not spatial
            colors: _,        // not spatial
            submeshes: _,     // not spatial
            skin: _,          // joint space, not mesh space
        } = self;
        let linear = glam::Mat3::from(affine.matrix3);
        let normal_matrix = linear.inverse().transpose();
        let map = |m: glam::Mat3, v: [f32; 3]| -> [f32; 3] { (m * glam::Vec3::from(v)).into() };
        for p in positions.iter_mut() {
            *p = affine.transform_point3(glam::Vec3::from(*p)).into();
        }
        for n in normals.iter_mut().flatten() {
            *n = (normal_matrix * glam::Vec3::from(*n)).normalize_or_zero().into();
        }
        for t in tangents.iter_mut().flatten() {
            let v = map(linear, [t[0], t[1], t[2]]);
            *t = [v[0], v[1], v[2], t[3]];
        }
        for target in morph_targets.iter_mut() {
            for d in &mut target.position_deltas {
                *d = map(linear, *d);
            }
            for d in target.normal_deltas.iter_mut().flatten() {
                *d = map(normal_matrix, *d);
            }
        }
        let (mn, mx) = (glam::Vec3::from(*model_min), glam::Vec3::from(*model_max));
        let mut new_min = glam::Vec3::splat(f32::MAX);
        let mut new_max = glam::Vec3::splat(f32::MIN);
        for x in [mn.x, mx.x] {
            for y in [mn.y, mx.y] {
                for z in [mn.z, mx.z] {
                    let t = affine.transform_point3(glam::Vec3::new(x, y, z));
                    new_min = new_min.min(t);
                    new_max = new_max.max(t);
                }
            }
        }
        *model_min = new_min.into();
        *model_max = new_max.into();
    }
}

/// Axis-aligned bounds of `positions` (zero box when empty).
fn bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    (min, max)
}

/// Append `theirs` to `mine` for an optional per-vertex stream, padding the
/// missing side with `fill` so the result has `total` entries.
fn merge_padded<T: Copy>(mine: &mut Option<Vec<T>>, theirs: Option<Vec<T>>, mine_len: usize, total: usize, fill: T) {
    match (mine.as_mut(), theirs) {
        (None, None) => {}
        (Some(a), Some(b)) => a.extend(b),
        (Some(a), None) => a.resize(total, fill),
        (None, Some(b)) => {
            let mut padded = vec![fill; mine_len];
            padded.extend(b);
            *mine = Some(padded);
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

/// Split rigidly-skinned triangles onto their bone (`node_parent_index`).
///
/// A triangle is rigid when all three vertices have exactly one influence,
/// on the same joint. A submesh whose triangles are at least 90 % rigid is
/// split: rigid triangles go to one submesh per joint, the blended remainder
/// stays a soft submesh on the original node. Otherwise the submesh stays
/// soft as a whole. Returns the rebuilt submeshes and indices, and per
/// rebuilt submesh whether it is a rigid split.
fn split_rigid_weighted_submeshes(
    submeshes: &[SubMesh],
    indices: &[u32],
    bone_maps: Option<&[crate::ivo::skin::BoneMap12]>,
) -> (Vec<SubMesh>, Vec<u32>, Vec<bool>) {
    let Some(bone_maps) = bone_maps else {
        return (submeshes.to_vec(), indices.to_vec(), vec![false; submeshes.len()]);
    };
    // The joint of a vertex with exactly one influence.
    let single_joint = |vertex: u32| -> Option<u16> {
        let entry = bone_maps.get(vertex as usize)?;
        let mut found = None;
        for (&joint, &weight) in entry.joint_indices.iter().zip(&entry.weights) {
            if weight > 0 {
                if found.is_some() {
                    return None;
                }
                found = Some(joint);
            }
        }
        found
    };

    let mut rebuilt_submeshes = Vec::with_capacity(submeshes.len());
    let mut rebuilt_indices = Vec::with_capacity(indices.len());
    let mut rigid = Vec::with_capacity(submeshes.len());
    let push = |submesh: &SubMesh, tris: &[u32], node: Option<u16>, is_rigid: bool,
                    out_sub: &mut Vec<SubMesh>, out_idx: &mut Vec<u32>, out_rigid: &mut Vec<bool>| {
        let mut part = submesh.clone();
        part.first_index = out_idx.len() as u32;
        part.num_indices = tris.len() as u32;
        if let Some(node) = node {
            part.node_parent_index = node;
        }
        out_idx.extend_from_slice(tris);
        out_sub.push(part);
        out_rigid.push(is_rigid);
    };

    for submesh in submeshes {
        let start = submesh.first_index as usize;
        let end = (start + submesh.num_indices as usize).min(indices.len());
        let source_indices = &indices[start..end];
        let triangles: Vec<&[u32]> = source_indices.chunks_exact(3).collect();
        if triangles.is_empty() {
            push(submesh, source_indices, None, false, &mut rebuilt_submeshes, &mut rebuilt_indices, &mut rigid);
            continue;
        }

        let mut rigid_groups = std::collections::BTreeMap::<u16, Vec<u32>>::new();
        let mut soft_remainder: Vec<u32> = Vec::new();
        for tri in &triangles {
            let joints = [single_joint(tri[0]), single_joint(tri[1]), single_joint(tri[2])];
            match joints {
                [Some(a), Some(b), Some(c)] if a == b && b == c => {
                    rigid_groups.entry(a).or_default().extend_from_slice(tri)
                }
                _ => soft_remainder.extend_from_slice(tri),
            }
        }
        let rigid_triangles = triangles.len() - soft_remainder.len() / 3;
        if (rigid_triangles as f32) < 0.9 * triangles.len() as f32 {
            push(submesh, source_indices, None, false, &mut rebuilt_submeshes, &mut rebuilt_indices, &mut rigid);
            continue;
        }
        // Phase 47.4: a single rigid group is still re-parented to its joint,
        // otherwise it stays static at the CHR root while its bone animates.
        for (joint, joint_indices) in rigid_groups {
            push(submesh, &joint_indices, Some(joint), true, &mut rebuilt_submeshes, &mut rebuilt_indices, &mut rigid);
        }
        if !soft_remainder.is_empty() {
            push(submesh, &soft_remainder, None, false, &mut rebuilt_submeshes, &mut rebuilt_indices, &mut rigid);
        }
    }

    (rebuilt_submeshes, rebuilt_indices, rigid)
}

/// Skin for the vertices of soft-skinned (non-rigid-split) submeshes from the
/// IVO bone map (8 influences preferred over 4), weights normalised. `None`
/// without joint names, without a bone map, or when no vertex is influenced.
fn soft_skin(
    streams: &crate::ivo::skin::DataStreams,
    vertex_count: usize,
    submeshes: &[SubMesh],
    indices: &[u32],
    rigid: &[bool],
    joint_names: Option<&[String]>,
) -> Option<crate::mesh_rig::Skin> {
    use crate::mesh_rig::MAX_INFLUENCES;
    let joint_names = joint_names?;
    let influences: Vec<([u16; MAX_INFLUENCES], [u8; MAX_INFLUENCES])> = if let Some(maps) = &streams.bone_maps32 {
        maps.iter().map(|m| (m.joint_indices, m.weights)).collect()
    } else {
        streams
            .bone_maps
            .as_ref()?
            .iter()
            .map(|m| {
                let mut j = [0u16; MAX_INFLUENCES];
                let mut w = [0u8; MAX_INFLUENCES];
                j[..4].copy_from_slice(&m.joint_indices);
                w[..4].copy_from_slice(&m.weights);
                (j, w)
            })
            .collect()
    };
    let mut soft = vec![false; vertex_count];
    for (submesh, &is_rigid) in submeshes.iter().zip(rigid) {
        if is_rigid {
            continue;
        }
        let start = submesh.first_index as usize;
        let end = (start + submesh.num_indices as usize).min(indices.len());
        for &v in &indices[start..end] {
            if let Some(flag) = soft.get_mut(v as usize) {
                *flag = true;
            }
        }
    }
    let mut joints = vec![[0u16; MAX_INFLUENCES]; vertex_count];
    let mut weights = vec![[0f32; MAX_INFLUENCES]; vertex_count];
    let mut any = false;
    for (v, is_soft) in soft.iter().enumerate() {
        let Some((j, w)) = influences.get(v).filter(|_| *is_soft) else { continue };
        let total: u32 = w.iter().map(|&x| x as u32).sum();
        if total == 0 {
            continue;
        }
        for slot in 0..MAX_INFLUENCES {
            if w[slot] > 0 && (j[slot] as usize) < joint_names.len() {
                joints[v][slot] = j[slot];
                weights[v][slot] = w[slot] as f32 / total as f32;
            }
        }
        any = true;
    }
    any.then(|| crate::mesh_rig::Skin { joint_names: joint_names.to_vec(), joints, weights })
}

pub fn build_mesh(skin: &SkinMesh, materials: &[MaterialName]) -> Mesh {
    build_mesh_with_bbox(skin, materials, false)
}

/// Build mesh, optionally dequantizing with model bbox instead of scaling bbox.
/// Interior CGFs use model bbox because IncludedObjects placements are authored
/// for model-bbox space. The scaling bbox is expanded for GPU skinning across
/// NMC nodes and gives wrong vertex positions for placement.
pub fn build_mesh_with_bbox(skin: &SkinMesh, materials: &[MaterialName], use_model_bbox: bool) -> Mesh {
    build_mesh_with_rig(skin, materials, use_model_bbox, None)
}

/// [`build_mesh_with_bbox`] plus the soft-skinned vertices' [`crate::mesh_rig::Skin`],
/// named by `joint_names` (the `.skin`'s own bone table).
pub fn build_mesh_with_rig(
    skin: &SkinMesh,
    materials: &[MaterialName],
    use_model_bbox: bool,
    joint_names: Option<&[String]>,
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

    let (submeshes, indices, rigid) = split_rigid_weighted_submeshes(
        &submeshes,
        &indices,
        skin.streams.bone_maps.as_deref(),
    );

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
    let rig = soft_skin(&skin.streams, positions.len(), &submeshes, &indices, &rigid, joint_names);

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
        skin: rig,
        morph_targets: Vec::new(),
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

    fn triangle(normals: bool, tangents: bool) -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: None,
            secondary_uvs: None,
            normals: normals.then(|| vec![[0.0, 0.0, 1.0]; 3]),
            tangents: tangents.then(|| vec![[1.0, 0.0, 0.0, 1.0]; 3]),
            colors: None,
            submeshes: vec![SubMesh {
                material_name: None,
                material_id: 0,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            }],
            model_min: [0.0; 3],
            model_max: [1.0, 1.0, 0.0],
            scaling_min: [0.0; 3],
            scaling_max: [1.0, 1.0, 0.0],
            skin: None,
            morph_targets: Vec::new(),
        }
    }

    fn skinned(names: &[&str], joint: u16) -> crate::mesh_rig::Skin {
        crate::mesh_rig::Skin {
            joint_names: names.iter().map(|n| n.to_string()).collect(),
            joints: vec![[joint, 0, 0, 0, 0, 0, 0, 0]; 3],
            weights: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 3],
        }
    }

    fn morph(name: &str, delta: [f32; 3]) -> crate::mesh_rig::MorphTarget {
        crate::mesh_rig::MorphTarget { name: name.into(), position_deltas: vec![delta; 3], normal_deltas: None }
    }

    /// Each .skin numbers joints against its own bone table, so a merge must
    /// union the tables by name and remap the incoming indices.
    #[test]
    fn merge_remaps_skin_joints_by_name() {
        let mut a = triangle(true, false);
        a.skin = Some(skinned(&["root", "spine"], 1));
        let mut b = triangle(true, false);
        b.skin = Some(skinned(&["spine", "head"], 1));
        a.merge_from(b);
        let skin = a.skin.expect("skin");
        assert_eq!(skin.joint_names, vec!["root", "spine", "head"]);
        assert_eq!(skin.joints.len(), 6);
        assert_eq!(skin.joints[0][0], 1, "a keeps spine");
        assert_eq!(skin.joints[3][0], 2, "b's head remapped to the merged table");
    }

    /// Vertices from an unskinned side carry no influence (all-zero weights);
    /// writers decide how to represent that.
    #[test]
    fn merge_with_unskinned_side_leaves_it_without_influence() {
        for skinned_left in [true, false] {
            let mut a = triangle(true, false);
            let mut b = triangle(true, false);
            if skinned_left { a.skin = Some(skinned(&["root"], 0)) } else { b.skin = Some(skinned(&["root"], 0)) }
            a.merge_from(b);
            let skin = a.skin.expect("skin");
            assert_eq!(skin.weights.len(), 6);
            let unskinned = if skinned_left { 3..6 } else { 0..3 };
            assert!(skin.weights[unskinned].iter().all(|w| w.iter().all(|&x| x == 0.0)));
        }
    }

    /// Morph targets merge by name; a side without a target gets zero deltas.
    #[test]
    fn merge_unions_morph_targets_by_name() {
        let mut a = triangle(true, false);
        a.morph_targets = vec![morph("smile", [0.0, 1.0, 0.0])];
        let mut b = triangle(true, false);
        b.morph_targets = vec![morph("frown", [0.0, -1.0, 0.0])];
        a.merge_from(b);
        let names: Vec<_> = a.morph_targets.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["smile", "frown"]);
        assert_eq!(a.morph_targets[0].position_deltas[4], [0.0, 0.0, 0.0]);
        assert_eq!(a.morph_targets[1].position_deltas[1], [0.0, 0.0, 0.0]);
        assert_eq!(a.morph_targets[1].position_deltas[4], [0.0, -1.0, 0.0]);
    }

    /// One transform moves every spatial stream: positions get the full
    /// affine; normals, tangent xyz and morph deltas only the rotation;
    /// tangent handedness (w) is kept.
    #[test]
    fn transform_moves_every_spatial_stream() {
        let mut mesh = triangle(true, true);
        mesh.morph_targets = vec![morph("m", [1.0, 0.0, 0.0])];
        let rot = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        mesh.transform(glam::Affine3A::from_rotation_translation(rot, glam::Vec3::new(10.0, 0.0, 0.0)));
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(mesh.positions[1], [10.0, 1.0, 0.0]), "{:?}", mesh.positions[1]);
        assert!(close(mesh.normals.as_ref().unwrap()[0], [0.0, 0.0, 1.0]));
        let t = mesh.tangents.as_ref().unwrap()[0];
        assert!(close([t[0], t[1], t[2]], [0.0, 1.0, 0.0]) && t[3] == 1.0, "{t:?}");
        assert!(close(mesh.morph_targets[0].position_deltas[0], [0.0, 1.0, 0.0]));
        // the model bbox is transformed by its corners: (1, 1) -> (-1, 1) + (10, 0)
        assert!(close(mesh.model_min, [9.0, 0.0, 0.0]) && close(mesh.model_max, [10.0, 1.0, 0.0]));
    }

    /// `subset` carries skin and morph data for the kept vertices.
    #[test]
    fn subset_carries_skin_and_morphs() {
        let mut mesh = triangle(true, false);
        mesh.skin = Some(skinned(&["root"], 0));
        mesh.morph_targets = vec![morph("m", [0.0, 0.0, 2.0])];
        let (part, _) = mesh.subset(&[0]);
        assert_eq!(part.skin.as_ref().map(|s| s.joints.len()), Some(3));
        assert_eq!(part.morph_targets[0].position_deltas.len(), 3);
    }

    /// `subset` keeps exactly the vertices the chosen submeshes reference,
    /// carries every per-vertex stream, and reindexes from 0.
    #[test]
    fn subset_carries_every_stream_for_the_chosen_submeshes() {
        let mut mesh = triangle(true, true);
        mesh.colors = Some(vec![[1, 2, 3, 4]; 3]);
        mesh.uvs = Some(vec![[0.5, 0.5]; 3]);
        let mut second = triangle(true, true);
        second.positions = vec![[5.0, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 0.0]];
        second.colors = Some(vec![[9, 9, 9, 9]; 3]);
        second.uvs = Some(vec![[0.25, 0.75]; 3]);
        second.submeshes[0].material_id = 7;
        mesh.merge_from(second);

        let (part, old_to_new) = mesh.subset(&[1]);
        assert_eq!(part.positions, vec![[5.0, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 0.0]]);
        assert_eq!(part.indices, vec![0, 1, 2]);
        assert_eq!(part.colors, Some(vec![[9, 9, 9, 9]; 3]));
        assert_eq!(part.uvs, Some(vec![[0.25, 0.75]; 3]));
        assert_eq!(part.normals.as_ref().map(Vec::len), Some(3));
        assert_eq!(part.tangents.as_ref().map(Vec::len), Some(3));
        assert_eq!(part.submeshes.len(), 1);
        assert_eq!((part.submeshes[0].material_id, part.submeshes[0].first_index), (7, 0));
        assert_eq!(part.model_min, [5.0, 0.0, 0.0]);
        assert_eq!(old_to_new.get(&3), Some(&0));
    }

    /// Per-vertex streams must stay as long as `positions` after a merge:
    /// a side without normals gets geometry-derived smooth normals (as the
    /// mesh builder does for a stream-less mesh); tangents that cannot be
    /// complete are dropped.
    #[test]
    fn merge_keeps_per_vertex_streams_aligned() {
        for (left, right) in [(true, false), (false, true)] {
            let mut mesh = triangle(left, left);
            mesh.merge_from(triangle(right, right));
            let normals = mesh.normals.as_ref().expect("normals");
            assert_eq!(normals.len(), mesh.positions.len(), "normals ({left}, {right})");
            for n in normals {
                assert!((n[2].abs() - 1.0).abs() < 1e-5, "unit +/-Z for a flat XY triangle: {n:?}");
            }
            assert!(mesh.tangents.is_none(), "incomplete tangents must be dropped ({left}, {right})");
        }
        let mut both = triangle(true, true);
        both.merge_from(triangle(true, true));
        assert_eq!(both.tangents.as_ref().map(Vec::len), Some(6));
    }

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
                bone_maps32: None,
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
                bone_maps32: None,
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

    fn two_triangle_skin(bone_maps: Vec<BoneMap12>) -> SkinMesh {
        SkinMesh {
            flags: 0,
            info: MeshInfo {
                flags2: 5,
                num_vertices: 6,
                num_indices: 6,
                num_submeshes: 1,
                model_min: [0.0; 3],
                model_max: [3.0, 1.0, 0.0],
                min_bound: [0.0; 3],
                max_bound: [3.0, 1.0, 0.0],
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
                center: [1.5, 0.5, 0.0],
                unknown0: 0,
                unknown1: 0,
            }],
            extra_words: Vec::new(),
            streams: DataStreams {
                positions: PositionData::Float(vec![
                    [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0],
                    [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0],
                ]),
                uvs: Vec::new(),
                secondary_uvs: None,
                indices: vec![0, 1, 2, 3, 4, 5],
                bone_maps: Some(bone_maps),
                bone_maps32: None,
                colors: None,
                tangents: None,
                normals: None,
            },
        }
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    /// A soft-skinned submesh keeps its weights as a named, normalised skin.
    /// (Triangles mix dominant joints, so the rigid splitter leaves it soft;
    /// see the parity plan for the dominant-joint misclassification.)
    #[test]
    fn build_mesh_keeps_soft_skin_weights() {
        let blended = BoneMap12 { joint_indices: [0, 1, 0, 0], weights: [128, 127, 0, 0] };
        let other = BoneMap12 { joint_indices: [0, 1, 0, 0], weights: [27, 228, 0, 0] };
        let skin = two_triangle_skin(vec![blended, other, blended, other, blended, other]);
        let materials = vec![MaterialName { name: "m".into() }];
        let mesh = build_mesh_with_rig(&skin, &materials, false, Some(&names(&["root", "spine"])));
        let rig = mesh.skin.expect("soft skin kept");
        assert_eq!(rig.joint_names, names(&["root", "spine"]));
        assert_eq!(rig.joints.len(), 6);
        assert_eq!(&rig.joints[0][..2], &[0, 1]);
        assert!((rig.weights[0][0] - 128.0 / 255.0).abs() < 1e-6);
        assert!((rig.weights[0].iter().sum::<f32>() - 1.0).abs() < 1e-6, "normalised");
    }

    /// Blended vertices are soft even when every vertex of a triangle has the
    /// same strongest joint: "rigid" means exactly one influence. (The splitter
    /// used to snap such triangles onto the dominant joint's bone.)
    #[test]
    fn build_mesh_keeps_blended_triangles_soft_when_dominant_joints_agree() {
        let blended = BoneMap12 { joint_indices: [0, 1, 0, 0], weights: [200, 55, 0, 0] };
        let skin = two_triangle_skin(vec![blended; 6]);
        let materials = vec![MaterialName { name: "m".into() }];
        let mesh = build_mesh_with_rig(&skin, &materials, false, Some(&names(&["root", "spine"])));
        assert_eq!(mesh.submeshes.len(), 1, "not split onto bones");
        let rig = mesh.skin.expect("blended weights kept");
        assert!((rig.weights[0][1] - 55.0 / 255.0).abs() < 1e-6);
    }

    /// A mostly rigid submesh splits its single-influence triangles onto
    /// bones and keeps the blended remainder as a soft submesh.
    #[test]
    fn build_mesh_splits_rigid_triangles_and_keeps_a_soft_remainder() {
        let a = BoneMap12 { joint_indices: [0, 0, 0, 0], weights: [255, 0, 0, 0] };
        let blended = BoneMap12 { joint_indices: [0, 1, 0, 0], weights: [128, 127, 0, 0] };
        let mut skin = two_triangle_skin(vec![a; 6]);
        // 10 rigid triangles + 1 blended one: ratio 10/11 >= 0.9
        let mut positions = Vec::new();
        let mut maps = Vec::new();
        for t in 0..11 {
            for k in 0..3 {
                positions.push([t as f32, k as f32, 0.0]);
                maps.push(if t == 10 { blended } else { a });
            }
        }
        skin.streams.positions = PositionData::Float(positions);
        skin.streams.indices = (0..33).collect();
        skin.streams.bone_maps = Some(maps);
        skin.info.num_vertices = 33;
        skin.info.num_indices = 33;
        skin.submeshes[0].num_indices = 33;
        skin.submeshes[0].num_vertices = 33;
        let materials = vec![MaterialName { name: "m".into() }];
        let mesh = build_mesh_with_rig(&skin, &materials, false, Some(&names(&["root", "spine"])));
        assert_eq!(mesh.submeshes.len(), 2, "rigid part + soft remainder");
        let soft = mesh.submeshes.iter().find(|s| s.num_indices == 3).expect("soft remainder");
        let v = mesh.indices[soft.first_index as usize] as usize;
        let rig = mesh.skin.expect("soft remainder keeps its skin");
        assert!(rig.weights[v][1] > 0.0);
        assert!(rig.weights[0].iter().all(|&w| w == 0.0), "rigid vertices stay bone-parented");
    }

    /// Rigid-split submeshes are parented to their bone node; weighting them
    /// too would apply the bone twice, so a fully rigid mesh has no skin.
    #[test]
    fn build_mesh_rigid_split_has_no_skin() {
        let a = BoneMap12 { joint_indices: [0, 0, 0, 0], weights: [255, 0, 0, 0] };
        let b = BoneMap12 { joint_indices: [1, 0, 0, 0], weights: [255, 0, 0, 0] };
        let skin = two_triangle_skin(vec![a, a, a, b, b, b]);
        let materials = vec![MaterialName { name: "m".into() }];
        let mesh = build_mesh_with_rig(&skin, &materials, false, Some(&names(&["root", "spine"])));
        assert_eq!(mesh.submeshes.len(), 2, "split onto bones as before");
        assert!(mesh.skin.is_none());
    }

    /// Without the .skin's bone table the joint indices cannot be named.
    #[test]
    fn build_mesh_without_joint_names_has_no_skin() {
        let blended = BoneMap12 { joint_indices: [0, 1, 0, 0], weights: [128, 127, 0, 0] };
        let materials = vec![MaterialName { name: "m".into() }];
        let mesh = build_mesh_with_rig(&two_triangle_skin(vec![blended; 6]), &materials, false, None);
        assert!(mesh.skin.is_none());
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
                bone_maps32: None,
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
