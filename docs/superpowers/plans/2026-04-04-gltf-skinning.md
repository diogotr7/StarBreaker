# glTF Skinned Mesh Animation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export standalone skinned animated GLBs from `.chr` skeleton + `.skin` mesh + DBA animations (e.g. landing gear).

**Architecture:** Extend the `Bone` struct with `controller_id` and `parent_index`. Add `add_skin()` to GlbBuilder that creates joint nodes, inverse bind matrices, and vertex weight attributes. Update `add_animations` to match by controller_id when a skin is present. Wire into a new `skinned_mesh_to_glb()` public function.

**Tech Stack:** Rust, gltf-json v1, glam 0.29

---

### Task 1: Extend Bone struct with controller_id and parent_index

**Files:**
- Modify: `crates/starbreaker-3d/src/skeleton.rs`

- [ ] **Step 1: Add fields to Bone struct**

Add `controller_id` and `parent_index` to the `Bone` struct:

```rust
pub struct Bone {
    pub name: String,
    /// Animation controller ID — CRC32 hash used by the DBA animation system.
    pub controller_id: u32,
    /// Parent bone index (-1 for root).
    pub parent_index: i32,
    /// World-space position [x, y, z]
    pub world_position: [f32; 3],
    /// World-space rotation quaternion [w, x, y, z]
    pub world_rotation: [f32; 4],
}
```

- [ ] **Step 2: Update RawQuatTrans::to_bone**

Change `to_bone` to accept the extra fields:

```rust
impl RawQuatTrans {
    fn to_bone(&self, name: String, controller_id: u32, parent_index: i32) -> Bone {
        Bone {
            name,
            controller_id,
            parent_index,
            world_rotation: [self.qw, self.qx, self.qy, self.qz],
            world_position: [self.tx, self.ty, self.tz],
        }
    }
}
```

- [ ] **Step 3: Update parse_compiled_bones_v900**

Pass `controller_id` and `parent_index` from each `BoneEntryV900`:

```rust
let bones: Vec<Bone> = entries
    .iter()
    .zip(names)
    .map(|(e, name)| e.world.to_bone(name, e.controller_id, e.parent_index))
    .collect();
```

- [ ] **Step 4: Update parse_compiled_bones_v901**

For v901, the entries and transforms are separate. Zip all three:

```rust
let bones: Vec<Bone> = entries
    .iter()
    .zip(names)
    .zip(world_transforms.iter())
    .map(|((e, name), qt)| qt.to_bone(name, e.controller_id, e.parent_index as i32))
    .collect();
```

Note: `BoneEntryV901.parent_index` is `i16`, cast to `i32`.

- [ ] **Step 5: Fix any compile errors from callers that construct Bone**

Search for `Bone {` in the codebase. If any exist outside skeleton.rs, add the new fields with defaults (`controller_id: 0, parent_index: -1`).

- [ ] **Step 6: Build and test**

Run: `cargo test -p starbreaker-3d`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/starbreaker-3d/src/skeleton.rs
git commit -m "feat: preserve controller_id and parent_index on skeleton Bone"
```

---

### Task 2: Add skin support to GlbBuilder

**Files:**
- Modify: `crates/starbreaker-3d/src/gltf/glb_builder.rs`

- [ ] **Step 1: Add skins_json field**

Add to `GlbBuilder` struct:

```rust
pub skins_json: Vec<json::Skin>,
```

Initialize as `Vec::new()` in `GlbBuilder::new()`.

- [ ] **Step 2: Wire skins into finalize**

In the `json::Root { .. }` block in `finalize`, add before `..Default::default()`:

```rust
skins: self.skins_json,
```

- [ ] **Step 3: Implement add_skin method**

Add to `impl GlbBuilder`:

```rust
/// Create a glTF skin from skeleton bones.
///
/// Creates joint nodes in a parent-child hierarchy, computes inverse bind matrices,
/// writes the IBM accessor, and sets the skin on the mesh node.
///
/// Returns a map of controller_id → glTF joint node index for animation matching.
pub fn add_skin(
    &mut self,
    bones: &[crate::skeleton::Bone],
    mesh_node_idx: u32,
) -> std::collections::HashMap<u32, u32> {
    let joint_base = self.nodes_json.len() as u32;
    let num_bones = bones.len();

    // Create joint nodes
    let mut children_map: Vec<Vec<u32>> = vec![vec![]; num_bones];
    let mut root_joints: Vec<u32> = Vec::new();

    for (i, bone) in bones.iter().enumerate() {
        if bone.parent_index < 0 || bone.parent_index as usize >= num_bones {
            root_joints.push(i as u32);
        } else {
            children_map[bone.parent_index as usize].push(i as u32);
        }
    }

    for (i, bone) in bones.iter().enumerate() {
        let child_indices: Vec<json::Index<json::Node>> = children_map[i]
            .iter()
            .map(|&c| json::Index::new(joint_base + c))
            .collect();

        // Joint nodes use world transform decomposed to TRS
        let rot = glam::Quat::from_xyzw(bone.world_rotation[1], bone.world_rotation[2],
                                          bone.world_rotation[3], bone.world_rotation[0]);
        let trans = glam::Vec3::from(bone.world_position);

        // For non-root joints, compute LOCAL transform = inverse(parent_world) * world
        let (local_trans, local_rot) = if bone.parent_index >= 0
            && (bone.parent_index as usize) < num_bones
        {
            let parent = &bones[bone.parent_index as usize];
            let parent_rot = glam::Quat::from_xyzw(
                parent.world_rotation[1], parent.world_rotation[2],
                parent.world_rotation[3], parent.world_rotation[0],
            );
            let parent_trans = glam::Vec3::from(parent.world_position);
            let inv_parent_rot = parent_rot.inverse();
            let local_t = inv_parent_rot * (trans - parent_trans);
            let local_r = inv_parent_rot * rot;
            (local_t, local_r)
        } else {
            (trans, rot)
        };

        self.nodes_json.push(json::Node {
            name: Some(bone.name.clone()),
            translation: Some(local_trans.into()),
            rotation: Some(json::scene::UnitQuaternion([
                local_rot.x, local_rot.y, local_rot.z, local_rot.w,
            ])),
            children: if child_indices.is_empty() { None } else { Some(child_indices) },
            ..Default::default()
        });
    }

    // Compute inverse bind matrices (world_to_bone = inverse(bone_to_world))
    let ibm_offset = self.bin.len();
    for bone in bones {
        let rot = glam::Quat::from_xyzw(
            bone.world_rotation[1], bone.world_rotation[2],
            bone.world_rotation[3], bone.world_rotation[0],
        );
        let trans = glam::Vec3::from(bone.world_position);
        let world_mat = glam::Mat4::from_rotation_translation(rot, trans);
        let ibm = world_mat.inverse();
        for &val in ibm.to_cols_array().iter() {
            self.bin.extend_from_slice(&val.to_le_bytes());
        }
    }
    let ibm_byte_length = num_bones * 64; // 4x4 f32 = 64 bytes
    while self.bin.len() % 4 != 0 { self.bin.push(0); }

    let ibm_acc = super::add_vertex_accessor(
        &mut self.buffer_views, &mut self.accessors,
        ibm_offset, ibm_byte_length, num_bones,
        json::accessor::Type::Mat4, None,
    ).unwrap();

    // Joints array
    let joints: Vec<json::Index<json::Node>> = (0..num_bones as u32)
        .map(|i| json::Index::new(joint_base + i))
        .collect();

    // Create skin
    let skeleton_root = root_joints.first().map(|&r| json::Index::new(joint_base + r));
    self.skins_json.push(json::Skin {
        joints,
        inverse_bind_matrices: Some(json::Index::new(ibm_acc)),
        skeleton: skeleton_root,
        name: Some("Skeleton".into()),
        extensions: None,
        extras: Default::default(),
    });

    // Set skin on mesh node
    let skin_idx = (self.skins_json.len() - 1) as u32;
    self.nodes_json[mesh_node_idx as usize].skin = Some(json::Index::new(skin_idx));

    // Build controller_id → joint node index map
    let mut id_map = std::collections::HashMap::new();
    for (i, bone) in bones.iter().enumerate() {
        id_map.insert(bone.controller_id, joint_base + i as u32);
    }
    id_map
}
```

- [ ] **Step 4: Build**

Run: `cargo check -p starbreaker-3d`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add crates/starbreaker-3d/src/gltf/glb_builder.rs
git commit -m "feat: add GlbBuilder::add_skin for glTF skeletal skinning"
```

---

### Task 3: Write vertex weight attributes (JOINTS_0, WEIGHTS_0)

**Files:**
- Modify: `crates/starbreaker-3d/src/gltf/glb_builder.rs` (pack_mesh method)

- [ ] **Step 1: Add JOINTS_0 and WEIGHTS_0 to PackedMeshInfo**

Find `struct PackedMeshInfo` and add:

```rust
pub joints_accessor_idx: Option<u32>,
pub weights_accessor_idx: Option<u32>,
```

Initialize both as `None` where `PackedMeshInfo` is constructed.

- [ ] **Step 2: Write vertex weight data in pack_mesh**

In the `pack_mesh` method, after writing other vertex attributes (normals, tangents, colors), add:

```rust
// Bone weights (JOINTS_0 + WEIGHTS_0)
if let Some(ref mappings) = mesh.bone_mappings {
    // JOINTS_0: 4×u16 per vertex
    let joints_offset = self.bin.len();
    for m in mappings {
        for &idx in &m.bone_indices {
            self.bin.extend_from_slice(&idx.to_le_bytes());
        }
    }
    let joints_len = mappings.len() * 8;
    while self.bin.len() % 4 != 0 { self.bin.push(0); }

    // Need a custom accessor with U16 component type
    let jbv_idx = self.buffer_views.len() as u32;
    self.buffer_views.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_offset: Some(json::validation::USize64(joints_offset as u64)),
        byte_length: json::validation::USize64(joints_len as u64),
        byte_stride: None,
        target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
        name: None, extensions: None, extras: Default::default(),
    });
    let jacc_idx = self.accessors.len() as u32;
    self.accessors.push(json::Accessor {
        buffer_view: Some(json::Index::new(jbv_idx)),
        byte_offset: Some(json::validation::USize64(0)),
        count: json::validation::USize64(mappings.len() as u64),
        component_type: Checked::Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::U16,
        )),
        type_: Checked::Valid(json::accessor::Type::Vec4),
        min: None, max: None, name: None, normalized: false, sparse: None,
        extensions: None, extras: Default::default(),
    });

    // WEIGHTS_0: 4×f32 per vertex
    let weights_offset = self.bin.len();
    for m in mappings {
        for &w in &m.weights {
            self.bin.extend_from_slice(&w.to_le_bytes());
        }
    }
    let weights_len = mappings.len() * 16;
    while self.bin.len() % 4 != 0 { self.bin.push(0); }

    let wacc_idx = super::add_vertex_accessor(
        &mut self.buffer_views, &mut self.accessors,
        weights_offset, weights_len, mappings.len(),
        json::accessor::Type::Vec4, None,
    ).unwrap();

    packed.joints_accessor_idx = Some(jacc_idx);
    packed.weights_accessor_idx = Some(wacc_idx);
}
```

- [ ] **Step 3: Add JOINTS_0 and WEIGHTS_0 to primitive attributes**

Find where primitives are constructed (both flat path and NMC path) and add after the existing attributes:

```rust
if let Some(j) = packed.joints_accessor_idx {
    attributes.insert(
        Checked::Valid(json::mesh::Semantic::Joints(0)),
        json::Index::new(j),
    );
}
if let Some(w) = packed.weights_accessor_idx {
    attributes.insert(
        Checked::Valid(json::mesh::Semantic::Weights(0)),
        json::Index::new(w),
    );
}
```

This needs to be added in **both** the flat mesh primitive construction AND the `build_nmc_hierarchy` per-node primitive construction.

- [ ] **Step 4: Build and test**

Run: `cargo test -p starbreaker-3d`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/starbreaker-3d/src/gltf/glb_builder.rs
git commit -m "feat: write JOINTS_0 and WEIGHTS_0 vertex attributes for skinned meshes"
```

---

### Task 4: Update add_animations for controller_id matching

**Files:**
- Modify: `crates/starbreaker-3d/src/gltf/glb_builder.rs`

- [ ] **Step 1: Add controller_id map parameter**

Change `add_animations` signature to accept an optional controller_id map:

```rust
pub fn add_animations(
    &mut self,
    clips: &[crate::animation::dba::AnimationClip],
    controller_id_map: Option<&std::collections::HashMap<u32, u32>>,
)
```

- [ ] **Step 2: Use controller_id map when provided**

At the top of `add_animations`, replace the hash_to_node construction:

```rust
let hash_to_node: HashMap<u32, u32> = if let Some(cid_map) = controller_id_map {
    cid_map.clone()
} else {
    // Fall back to CRC32 of original-case node names (for NMC animations)
    let mut map = HashMap::new();
    for &idx_val in self.node_name_to_idx.values() {
        if let Some(ref name) = self.nodes_json[idx_val as usize].name {
            let hash = crc32fast::hash(name.as_bytes());
            map.insert(hash, idx_val);
        }
    }
    map
};
```

- [ ] **Step 3: Fix all callers of add_animations**

Search for `.add_animations(` in the codebase. Update each call:

- `crates/starbreaker-3d/src/gltf/mod.rs` in `write_glb`: pass `None`
  ```rust
  builder.add_animations(&input.animations, None);
  ```

- [ ] **Step 4: Build and test**

Run: `cargo test -p starbreaker-3d`
Expected: all tests pass, NMC animations still work

- [ ] **Step 5: Commit**

```bash
git add crates/starbreaker-3d/src/gltf/glb_builder.rs crates/starbreaker-3d/src/gltf/mod.rs
git commit -m "feat: add_animations supports controller_id matching for skinned meshes"
```

---

### Task 5: skinned_mesh_to_glb public function + test example

**Files:**
- Modify: `crates/starbreaker-3d/src/gltf/mod.rs`
- Modify: `crates/starbreaker-3d/src/lib.rs`
- Create: `crates/starbreaker-3d/examples/test_skinned_glb.rs`

- [ ] **Step 1: Add skinned_mesh_to_glb function**

In `crates/starbreaker-3d/src/gltf/mod.rs`:

```rust
/// Build a standalone skinned animated GLB from CDF components.
pub fn skinned_mesh_to_glb(
    skin_data: &[u8],
    chr_data: &[u8],
    dba_data: Option<&[u8]>,
) -> Result<Vec<u8>, crate::error::Error> {
    let mesh = crate::parse_skin(skin_data)?;
    let bones = crate::skeleton::parse_skeleton(chr_data)
        .ok_or_else(|| crate::error::Error::Other("Failed to parse skeleton".into()))?;

    let animations = match dba_data {
        Some(data) => crate::animation::dba::parse_dba(data)?.clips,
        None => Vec::new(),
    };

    let mut builder = GlbBuilder::new();
    let packed = builder.pack_mesh(&mesh, None, None, None, None, crate::pipeline::MaterialMode::None);

    // Create mesh node
    let mesh_node_idx = builder.nodes_json.len() as u32;
    builder.nodes_json.push(gltf_json::Node {
        mesh: Some(gltf_json::Index::new(packed.mesh_idx)),
        name: Some("Mesh".into()),
        ..Default::default()
    });

    // Add skin (joint nodes + inverse bind matrices)
    let controller_id_map = builder.add_skin(&bones, mesh_node_idx);

    // Add animations matched by controller_id
    if !animations.is_empty() {
        builder.add_animations(&animations, Some(&controller_id_map));
    }

    // Scene: mesh node + skeleton root joints
    let mut scene_nodes = vec![gltf_json::Index::new(mesh_node_idx)];
    // Add root joint nodes to scene
    for (i, bone) in bones.iter().enumerate() {
        if bone.parent_index < 0 || bone.parent_index as usize >= bones.len() {
            let joint_node_idx = *controller_id_map.get(&bone.controller_id).unwrap();
            scene_nodes.push(gltf_json::Index::new(joint_node_idx));
        }
    }

    let metadata = GlbMetadata {
        entity_name: None, geometry_path: None, material_path: None,
        export_options: ExportOptionsMetadata {
            material_mode: "None".into(), format: "Glb".into(),
            lod_level: 0, texture_mip: 0, include_attachments: false, include_interior: false,
        },
    };

    builder.finalize(scene_nodes, Vec::new(), &metadata)
}
```

- [ ] **Step 2: Re-export from lib.rs**

Add to the `pub use` block in `lib.rs`:

```rust
pub use gltf::skinned_mesh_to_glb;
```

- [ ] **Step 3: Create test example**

Create `crates/starbreaker-3d/examples/test_skinned_glb.rs`:

```rust
//! Export a skinned animated GLB (e.g. landing gear).
//! Usage: test_skinned_glb [cdf_search] [dba_search] [output.glb]

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let cdf_search = args.get(1).map(|s| s.as_str()).unwrap_or("Hornet/F7A/F7A_LandingGear/LandingGear_Front");
    let dba_search = args.get(2).map(|s| s.as_str()).unwrap_or("Landing_Gear/Hornet_F7A.dba");
    let output = args.get(3).map(|s| s.as_str()).unwrap_or("landing_gear.glb");

    let p4k = starbreaker_p4k::open_p4k().expect("no P4k");

    // Find CDF
    let cdf_lower = cdf_search.to_lowercase().replace('/', "\\");
    let cdf_entry = p4k.entries().iter()
        .find(|e| e.name.to_lowercase().contains(&cdf_lower) && e.name.to_lowercase().ends_with(".cdf"))
        .unwrap_or_else(|| panic!("No .cdf matching '{cdf_search}'"));
    eprintln!("CDF: {}", cdf_entry.name);

    // Parse CDF to get .chr and .skin paths
    let cdf_data = p4k.read(cdf_entry).unwrap();
    let cdf_xml = starbreaker_cryxml::from_bytes(&cdf_data).unwrap();
    let root = cdf_xml.root();

    let mut chr_path = String::new();
    let mut skin_path = String::new();
    for child in cdf_xml.node_children(root) {
        let tag = cdf_xml.node_tag(child);
        if tag == "Model" {
            chr_path = cdf_xml.node_attributes(child)
                .find(|(k, _)| *k == "File").map(|(_, v)| v.to_string()).unwrap_or_default();
        }
        if tag == "AttachmentList" {
            for att in cdf_xml.node_children(child) {
                let attrs: std::collections::HashMap<&str, &str> = cdf_xml.node_attributes(att).collect();
                if attrs.get("Type") == Some(&"CA_SKIN") {
                    skin_path = attrs.get("Binding").unwrap_or(&"").to_string();
                }
            }
        }
    }

    eprintln!("CHR: {chr_path}");
    eprintln!("SKIN: {skin_path}");

    // Load files
    let load = |path: &str| -> Vec<u8> {
        let p4k_path = format!("Data\\{}", path.replace('/', "\\"));
        let entry = p4k.entry_case_insensitive(&p4k_path)
            .unwrap_or_else(|| panic!("Not found: {p4k_path}"));
        p4k.read(entry).unwrap()
    };

    let chr_data = load(&chr_path);
    let skin_data = load(&skin_path);

    // Find DBA
    let dba_lower = dba_search.to_lowercase().replace('/', "\\");
    let dba_data = p4k.entries().iter()
        .find(|e| e.name.to_lowercase().contains(&dba_lower) && e.name.to_lowercase().ends_with(".dba"))
        .map(|e| {
            eprintln!("DBA: {}", e.name);
            p4k.read(e).unwrap()
        });

    let glb = starbreaker_3d::gltf::skinned_mesh_to_glb(
        &skin_data, &chr_data, dba_data.as_deref(),
    ).expect("failed to build GLB");

    std::fs::write(output, &glb).unwrap();
    eprintln!("Wrote {} ({:.1} KB)", output, glb.len() as f64 / 1024.0);
}
```

- [ ] **Step 4: Build and run**

Run: `cargo build --example test_skinned_glb`
Then: `cargo run --example test_skinned_glb`

Expected: outputs `landing_gear.glb` with skeleton joint nodes, vertex weights, and animation clips.

- [ ] **Step 5: Visual verification**

Open in Blender. Check:
- Mesh is present with vertex groups (from JOINTS_0/WEIGHTS_0)
- Armature is present with bone hierarchy
- Animation clips (if matched) play with mesh deformation

- [ ] **Step 6: Commit**

```bash
git add crates/starbreaker-3d/src/gltf/mod.rs crates/starbreaker-3d/src/lib.rs crates/starbreaker-3d/examples/test_skinned_glb.rs
git commit -m "feat: skinned_mesh_to_glb for standalone skinned animated GLBs"
```

---

### Task 6: Fix issues from visual inspection

Buffer task for post-inspection corrections.

- [ ] **Step 1: Check in Blender**

Look for:
- Mesh deforms with skeleton when playing animation
- No bones at wrong positions
- Vertex weights look correct (select vertices, check weight paint)
- If DBA hashes don't match (like Avenger), try Hornet F7A or Gladius landing gear

- [ ] **Step 2: Fix and commit**

```bash
git add -u
git commit -m "fix: skinning corrections from visual inspection"
```
