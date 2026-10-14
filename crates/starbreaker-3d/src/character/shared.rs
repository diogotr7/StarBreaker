//! CHF character rig → the shared export model (`types::Mesh`,
//! `skeleton::Bone`, `mesh_rig`), so characters go through the same glTF and
//! `.blend` writers as every other entity.
//!
//! - [`skeleton_bones`]: the character skeleton. Locals are the (possibly
//!   posed) [`ModifiedSkeleton`] binds, `world_*` chains them, and
//!   `bind_world_*` is the re-bound rest skeleton's world (the skinning
//!   reference), so a control pose deforms the skinned meshes.
//! - [`skin_from_head`]: a [`HeadMesh`]'s skin (up to 8 influences across
//!   `joints` + `joints_hi`), joints named by its skeleton.
//! - [`morph_targets`]: `BShapesGpu` blendshapes as morph targets. The data
//!   has no names; targets are `bs_NNN` by index (the index animation tracks
//!   address).
//! - [`CharacterRig`]: one character at one pose — the head, its rest and
//!   posed skeleton, optional blendshapes and the parts ([`CharacterPart`]:
//!   CDF sub-meshes, CHF items, the body and garments, at REST, skinned by
//!   joint name; zones covered by other worn items are not written), built
//!   on the actor (body) skeleton when it has one ([`CharacterRig::dress`],
//!   [`overlay_bones`]). The
//!   writer applies the pose through the skins, so parts are never posed on
//!   the CPU. Every part (and the head) carries its pipeline-loaded
//!   [`PartSource`]: the exported meshes are those sources reshaped to the
//!   rig ([`CharacterRig::head_mesh`], [`CharacterRig::part_payloads`]), so
//!   the GLB and the decomposed package carry the same vertex streams,
//!   submeshes and `.mtl` materials. [`CharacterRig::to_glb`] writes it
//!   with the shared glTF writer (`MaterialMode::Colors`: the `.mtl`
//!   materials without textures). It may carry animation clips
//!   (`crate::clip::Clip`) over the same skeleton and morph targets.
//!   [`CharacterRig::hide_head`]: a worn item hides the head's port (a full
//!   helmet) — the head is written without geometry (the skeleton's root
//!   stays), and its CDF sub-meshes and blendshapes go.
//! - [`part_payload`]: a part as a child [`EntityPayload`] of the head;
//!   [`stem`] names parts after their source file.

use super::apply::ModifiedSkeleton;
use super::blendshapes::BlendShapes;
use super::head_skin::{BindPoseSkeleton, HeadMesh};
use super::part_source::PartSource;
use super::CharacterError;
use std::collections::HashSet;
use std::sync::Arc;
use crate::mesh_rig::{MorphTarget, Skin, MAX_INFLUENCES};
use crate::skeleton::Bone;
use crate::types::{EntityPayload, Mesh};

/// A character part (CDF sub-mesh or CHF item) at the character's rest pose,
/// skinned to the joints of its own `skeleton` (bound by name). Bones the
/// character skeleton lacks (an item's hair braids) join it ([`CharacterRig::bones`]).
pub(crate) struct CharacterPart {
    pub name: String,
    /// The part's shape for this character (positions, normals, skin).
    pub mesh: HeadMesh,
    pub skeleton: BindPoseSkeleton,
    /// The part as the pipeline loads it; `mesh` reshapes it.
    pub source: Arc<PartSource>,
    /// Clothing zones other worn items cover: the source's submeshes in
    /// them are not written ([`CharacterRig::part_payloads`]).
    pub hidden_zones: HashSet<u32>,
}

impl CharacterPart {
    /// The part's joint names (its skeleton, in order).
    pub(crate) fn joint_names(&self) -> Vec<String> {
        joint_names(&self.skeleton)
    }
}

/// One character at one pose (see the module docs).
pub(crate) struct CharacterRig {
    /// Root node / entity name.
    pub name: String,
    /// The head at rest, skinned to `rest`.
    pub head: HeadMesh,
    /// The head as the pipeline loads it (streams, submeshes, `.mtl`);
    /// `head` reshapes it.
    pub head_source: Arc<PartSource>,
    /// The (per-character re-bound) rest skeleton: the skinning bind.
    pub rest: BindPoseSkeleton,
    /// The pose: `rest` with the control-driven joint deltas.
    pub posed: ModifiedSkeleton,
    pub blendshapes: Option<BlendShapes>,
    pub parts: Vec<CharacterPart>,
    /// Animation clips (bones by skeleton name, morphs by `bs_NNN`).
    pub animations: Vec<crate::clip::Clip>,
    /// The actor (body) skeleton at its bind, the rig's base when present
    /// ([`Self::bones`]); empty for a head-only rig.
    pub actor: Vec<Bone>,
    /// The head's port is hidden ([`Self::hide_head`]).
    pub head_hidden: bool,
}

impl CharacterRig {
    /// The head as a skinned shared mesh carrying the blendshapes: its
    /// pipeline source reshaped to `head`; no geometry when it is hidden.
    pub(crate) fn head_mesh(&self) -> Result<Mesh, CharacterError> {
        if self.head_hidden {
            return Ok(Mesh::empty());
        }
        let mut mesh = self.head_source.reshaped(&self.head, &joint_names(&self.rest))?;
        if let Some(bs) = &self.blendshapes {
            if bs.num_vertices as usize != self.head.positions.len()
                || bs.targets.iter().any(|t| t.positions.len() != self.head.positions.len())
            {
                return Err(CharacterError::GlbEmit(format!(
                    "blendshape vertex count {} does not match mesh vertex count {}",
                    bs.num_vertices,
                    self.head.positions.len()
                )));
            }
            mesh.morph_targets = morph_targets(bs);
        }
        Ok(mesh)
    }

    /// The head's materials (none when it is hidden).
    pub(crate) fn head_materials(&self) -> Option<crate::mtl::MtlFile> {
        if self.head_hidden { None } else { self.head_source.materials.clone() }
    }

    /// Hide the head (its port is hidden): no head geometry, no blendshapes
    /// or morph tracks, and the `head_parts` (its CDF sub-meshes) go.
    pub(crate) fn hide_head(&mut self, head_parts: impl Fn(&CharacterPart) -> bool) {
        self.head_hidden = true;
        self.blendshapes = None;
        for clip in &mut self.animations {
            clip.morphs.clear();
        }
        self.parts.retain(|p| !head_parts(p));
    }

    /// Dress the rig in `outfit`: the body's actor skeleton becomes the base
    /// of [`Self::bones`], and the body and garments join the parts
    /// (skinned by joint name, covered zones hidden).
    pub(crate) fn dress(&mut self, outfit: &super::outfit::Outfit) {
        if let Some(body) = &outfit.body {
            self.actor = body.actor.clone();
        }
        self.parts.extend(outfit.parts());
    }

    /// Play a body `clip` (`.dba` / `.caf`, bone-local samples) over the
    /// rig's skeleton ([`crate::animation::shared_clip`] by [`Self::bones`]
    /// names), named after its file. A clip already on the rig (the face
    /// expression) joins it: one clip, the body's tracks winning where both
    /// animate a bone ([`crate::clip::Clip::merge`]).
    pub(crate) fn add_body_clip(&mut self, clip: &crate::animation::AnimationClip) {
        let names: Vec<String> = self.bones().into_iter().map(|b| b.name).collect();
        let mut body = crate::animation::shared_clip(clip, &names);
        body.name = stem(&clip.name);
        if !self.animations.is_empty() {
            body.merge(self.animations.remove(0));
        }
        self.animations.insert(0, body);
    }

    /// The posed skeleton bound at `rest` — laid over the `actor` skeleton
    /// when there is one ([`overlay_bones`]) — plus the parts' bones it lacks
    /// (an item's own bones, e.g. hair braids): each hangs under its own
    /// parent by name, at its bind pose, and follows that parent's pose.
    pub(crate) fn bones(&self) -> Vec<Bone> {
        let head = skeleton_bones(&self.rest, &self.posed);
        let mut bones = if self.actor.is_empty() { head } else { overlay_bones(self.actor.clone(), &head) };
        for part in &self.parts {
            append_missing_bones(&mut bones, &part.skeleton);
        }
        bones
    }

    /// Each part as a child payload of the root `root`: its source reshaped
    /// to the part's shape, without the submeshes in its `hidden_zones`
    /// (a part with none left is not written), with the source's `.mtl` and
    /// NMC; `geometry_path` names the payload's geometry from the source.
    pub(crate) fn part_payloads(
        &self,
        root: &str,
        geometry_path: impl Fn(&PartSource) -> String,
    ) -> Result<Vec<EntityPayload>, CharacterError> {
        let mut payloads = Vec::with_capacity(self.parts.len());
        for p in &self.parts {
            let mut mesh = p.source.reshaped(&p.mesh, &p.joint_names())?;
            if !p.hidden_zones.is_empty() {
                let visible = p.source.visible_submeshes(&p.hidden_zones);
                if visible.is_empty() {
                    continue;
                }
                if visible.len() < mesh.submeshes.len() {
                    mesh = mesh.subset(&visible).0;
                }
            }
            let (materials, nmc) = (p.source.materials.clone(), p.source.nmc.clone());
            payloads.push(part_payload(root, p.name.clone(), geometry_path(&p.source), mesh, materials, nmc));
        }
        Ok(payloads)
    }

    /// Write the rig with the shared glTF writer: the head is the root mesh
    /// with the skeleton under it, each part a child of the head. Meshes
    /// carry their `.mtl` materials (no textures: the rig has no p4k to
    /// read them from). Parts are single skinned nodes, so their NMC node
    /// hierarchies are left out.
    pub(crate) fn to_glb(&self) -> Result<Vec<u8>, CharacterError> {
        use crate::gltf::{self, ExportOptionsMetadata, GlbInput, GlbLoaders, GlbMetadata, GlbOptions};
        if self.rest.bones.is_empty() {
            return Err(CharacterError::GlbEmit("skeleton has zero bones".into()));
        }
        let mut children = self.part_payloads(&self.name, |_| String::new())?;
        for child in &mut children {
            child.nmc = None;
        }
        let input = GlbInput {
            root_mesh: Some(self.head_mesh()?),
            root_materials: self.head_materials(),
            root_textures: None,
            root_nmc: None,
            root_palette: None,
            skeleton_bones: self.bones(),
            children,
            interiors: Default::default(),
            animations: self.animations.clone(),
        };
        let opts = GlbOptions {
            material_mode: crate::pipeline::MaterialMode::Colors,
            preserve_textureless_decal_primitives: false,
            metadata: GlbMetadata {
                entity_name: Some(self.name.clone()),
                geometry_path: None,
                material_path: self.head_materials().and_then(|m| m.source_path),
                export_options: ExportOptionsMetadata {
                    kind: "Bundled".to_string(),
                    material_mode: "Colors".to_string(),
                    format: "Glb".to_string(),
                    lod_level: 0,
                    texture_mip: 0,
                    include_attachments: true,
                    include_interior: false,
                },
            },
            fallback_palette: None,
        };
        let mut loaders = GlbLoaders { load_textures: &mut |_, _| None, load_interior_mesh: &mut |_| None };
        gltf::write_glb(input, &mut loaders, &opts).map_err(|e| CharacterError::GlbEmit(e.to_string()))
    }
}

/// A character part as a child entity of the root `root` (attached under it,
/// no offset).
pub(crate) fn part_payload(
    root: &str,
    name: String,
    geometry_path: String,
    mesh: Mesh,
    materials: Option<crate::mtl::MtlFile>,
    nmc: Option<crate::nmc::NodeMeshCombo>,
) -> EntityPayload {
    EntityPayload {
        material_path: materials.as_ref().and_then(|m| m.source_path.clone()).unwrap_or_default(),
        mesh,
        materials,
        textures: None,
        nmc,
        palette: None,
        geometry_path,
        bones: Vec::new(),
        skeleton_source_path: None,
        entity_name: name,
        entity_category: None,
        attach_def_type: None,
        parent_node_name: String::new(),
        parent_entity_name: root.to_owned(),
        no_rotation: false,
        offset_position: [0.0; 3],
        offset_rotation: [0.0; 3],
        detach_direction: [0.0; 3],
        port_flags: String::new(),
        ui_bindings: Vec::new(),
    }
}

/// `(x, y, z, w)` (head_skin) → `[w, x, y, z]` (skeleton).
fn wxyz(q: [f32; 4]) -> [f32; 4] {
    [q[3], q[0], q[1], q[2]]
}

/// The character skeleton in the shared model (see the module docs).
/// `posed` and `rest` share bone order and parentage.
pub(crate) fn skeleton_bones(rest: &BindPoseSkeleton, posed: &ModifiedSkeleton) -> Vec<Bone> {
    let mut world: Vec<glam::Mat4> = Vec::with_capacity(posed.bones.len());
    posed
        .bones
        .iter()
        .zip(&rest.bones)
        .map(|(pose, bind)| {
            let local_q = glam::Quat::from_array(pose.bind_rotation);
            let local = glam::Mat4::from_rotation_translation(local_q, glam::Vec3::from(pose.bind_translation));
            let w = match pose.parent {
                Some(p) => world[p as usize] * local,
                None => local,
            };
            world.push(w);
            let (_, world_q, world_t) = w.to_scale_rotation_translation();
            Bone {
                name: pose.name.clone(),
                parent_index: pose.parent.map(|p| p as u16),
                object_node_index: None,
                local_position: pose.bind_translation,
                local_rotation: wxyz(pose.bind_rotation),
                world_position: world_t.to_array(),
                world_rotation: wxyz(world_q.to_array()),
                bind_world_position: bind.world_translation,
                bind_world_rotation: wxyz(bind.world_rotation),
            }
        })
        .collect()
}

/// The skin of a [`HeadMesh`] (up to 8 influences over `joints` +
/// `joints_hi`), its joints named by `joint_names`; `None` without joints or
/// names.
pub(crate) fn skin_from_head(mesh: &HeadMesh, joint_names: &[String]) -> Option<Skin> {
    if mesh.joints.is_empty() || joint_names.is_empty() {
        return None;
    }
    let count = mesh.positions.len();
    let mut joints = vec![[0u16; MAX_INFLUENCES]; count];
    let mut weights = vec![[0f32; MAX_INFLUENCES]; count];
    for v in 0..count {
        let sets = [(mesh.joints.get(v), mesh.weights.get(v)), (mesh.joints_hi.get(v), mesh.weights_hi.get(v))];
        for (set, (j, w)) in sets.into_iter().enumerate() {
            if let (Some(j), Some(w)) = (j, w) {
                joints[v][set * 4..set * 4 + 4].copy_from_slice(j);
                weights[v][set * 4..set * 4 + 4].copy_from_slice(w);
            }
        }
    }
    Some(Skin { joint_names: joint_names.to_vec(), joints, weights })
}

/// A [`HeadMesh`] as a skinned shared `Mesh` (one submesh), its joints named
/// by `joint_names` (the skeleton its joint indices refer to). Test rigs use
/// it as a stand-in for a pipeline-loaded [`PartSource`].
#[cfg(test)]
pub(crate) fn mesh_from_head(mesh: &HeadMesh, joint_names: &[String]) -> Mesh {
    let count = mesh.positions.len();
    let mut out = Mesh::empty();
    out.positions = mesh.positions.clone();
    out.indices = mesh.indices.clone();
    out.normals = (mesh.normals.len() == count).then(|| mesh.normals.clone());
    out.uvs = (mesh.uvs.len() == count).then(|| mesh.uvs.clone());
    out.submeshes = vec![crate::types::SubMesh {
        material_name: None,
        material_id: 0,
        source_material_id: None,
        first_index: 0,
        num_indices: mesh.indices.len() as u32,
        first_vertex: 0,
        num_vertices: count as u32,
        node_parent_index: 0,
    }];
    let (min, max) = mesh.positions.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(mut lo, mut hi), p| {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        (lo, hi)
    });
    if count > 0 {
        out.model_min = min;
        out.model_max = max;
        out.scaling_min = min;
        out.scaling_max = max;
    }
    out.skin = skin_from_head(mesh, joint_names);
    out
}

/// File stem of a p4k / DataCore path (`…/protos_eyes.skin` → `protos_eyes`).
pub(crate) fn stem(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.split('.').next().unwrap_or(file).to_owned()
}

/// `base` (the actor skeleton) with the head rig's `overlay` bones merged by
/// name: an overlay-only bone hangs under its parent by name (a root when
/// that is missing) with its local pose and bind; a shared bone takes the
/// overlay's local pose only where both skeletons hang it under the
/// same-named parent (the head rig roots `Spine3` at `World`, the body under
/// `Spine2`), and keeps the base's otherwise. Worlds are chained afresh.
fn overlay_bones(mut base: Vec<Bone>, overlay: &[Bone]) -> Vec<Bone> {
    let parent_name = |bones: &[Bone], b: &Bone| b.parent_index.map(|p| bones[p as usize].name.clone());
    for bone in overlay {
        let overlay_parent = parent_name(overlay, bone);
        match base.iter().position(|b| b.name == bone.name) {
            Some(i) => {
                if parent_name(&base, &base[i]) == overlay_parent {
                    base[i].local_position = bone.local_position;
                    base[i].local_rotation = bone.local_rotation;
                }
            }
            None => {
                let parent = overlay_parent.and_then(|n| base.iter().position(|b| b.name == n));
                let mut merged = bone.clone();
                merged.parent_index = parent.map(|p| p as u16);
                if parent.is_none() {
                    // A root's local is its world.
                    merged.local_position = bone.world_position;
                    merged.local_rotation = bone.world_rotation;
                }
                base.push(merged);
            }
        }
    }
    chain_worlds(&mut base);
    base
}

/// Recompute `world_*` from the locals, parents first (a parent listed after
/// its child is treated as a root).
fn chain_worlds(bones: &mut [Bone]) {
    let mut world: Vec<glam::Mat4> = Vec::with_capacity(bones.len());
    for i in 0..bones.len() {
        let b = &bones[i];
        let local = glam::Mat4::from_rotation_translation(
            glam::Quat::from_array(xyzw(b.local_rotation)),
            glam::Vec3::from(b.local_position),
        );
        let w = match b.parent_index.map(|p| p as usize).filter(|&p| p < i) {
            Some(p) => world[p] * local,
            None => local,
        };
        let (_, q, t) = w.to_scale_rotation_translation();
        bones[i].world_position = t.to_array();
        bones[i].world_rotation = wxyz(q.to_array());
        world.push(w);
    }
}

/// Append the bones of `skeleton` that `bones` lacks (by name), parented by
/// name; a bone whose parent is missing too becomes a root. Bind = the part
/// skeleton's world; pose = its local under the parent's (posed) world.
fn append_missing_bones(bones: &mut Vec<Bone>, skeleton: &BindPoseSkeleton) {
    for (i, part_bone) in skeleton.bones.iter().enumerate() {
        if part_bone.name.is_empty() || bones.iter().any(|b| b.name == part_bone.name) {
            continue;
        }
        let parent = part_bone
            .parent
            .filter(|&p| (p as usize) < i)
            .and_then(|p| bones.iter().position(|b| b.name == skeleton.bones[p as usize].name));
        let local = glam::Mat4::from_rotation_translation(
            glam::Quat::from_array(part_bone.bind_rotation),
            glam::Vec3::from(part_bone.bind_translation),
        );
        let (local_position, local_rotation, world) = match parent {
            Some(p) => {
                let parent_world = glam::Mat4::from_rotation_translation(
                    glam::Quat::from_array(xyzw(bones[p].world_rotation)),
                    glam::Vec3::from(bones[p].world_position),
                );
                (part_bone.bind_translation, wxyz(part_bone.bind_rotation), parent_world * local)
            }
            // No parent: the bone's local is its world.
            None => (
                part_bone.world_translation,
                wxyz(part_bone.world_rotation),
                glam::Mat4::from_rotation_translation(
                    glam::Quat::from_array(part_bone.world_rotation),
                    glam::Vec3::from(part_bone.world_translation),
                ),
            ),
        };
        let (_, world_q, world_t) = world.to_scale_rotation_translation();
        bones.push(Bone {
            name: part_bone.name.clone(),
            parent_index: parent.map(|p| p as u16),
            object_node_index: None,
            local_position,
            local_rotation,
            world_position: world_t.to_array(),
            world_rotation: wxyz(world_q.to_array()),
            bind_world_position: part_bone.world_translation,
            bind_world_rotation: wxyz(part_bone.world_rotation),
        });
    }
}

/// `[w, x, y, z]` → `(x, y, z, w)`.
fn xyzw(q: [f32; 4]) -> [f32; 4] {
    [q[1], q[2], q[3], q[0]]
}

/// Joint names of a skeleton, in bone order.
pub(crate) fn joint_names(skeleton: &BindPoseSkeleton) -> Vec<String> {
    skeleton.bones.iter().map(|b| b.name.clone()).collect()
}

/// Blendshapes as morph targets named `bs_NNN` by index.
pub(crate) fn morph_targets(blendshapes: &BlendShapes) -> Vec<MorphTarget> {
    blendshapes
        .targets
        .iter()
        .enumerate()
        .map(|(i, t)| MorphTarget { name: format!("bs_{i:03}"), position_deltas: t.positions.clone(), normal_deltas: None })
        .collect()
}

#[cfg(test)]
mod tests;
