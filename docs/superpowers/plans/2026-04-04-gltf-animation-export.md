# glTF Animation Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `GlbBuilder::add_animations()` that emits standard glTF animation tracks from DBA `AnimationClip` data, targeting NMC scene graph nodes by CRC32 bone hash matching.

**Architecture:** DBA bone hashes match NMC node names via CRC32. The method writes keyframe accessors (time/rotation/translation) to the GLB binary buffer, creates samplers and channels in the glTF JSON, and decomposes animated nodes from `matrix` to TRS (required by glTF spec). Animations flow through `GlbInput` → `write_glb` → `GlbBuilder::add_animations`. A test example exports an animated Zeus GLB for visual verification.

**Tech Stack:** Rust, gltf-json v1, glam 0.29, crc32fast

---

### Task 1: Add `crc32fast` dependency

**Files:**
- Modify: `crates/starbreaker-3d/Cargo.toml`

- [ ] **Step 1: Add the dependency**

Add to `[dependencies]` in `crates/starbreaker-3d/Cargo.toml`:

```toml
crc32fast = "1"
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p starbreaker-3d`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add crates/starbreaker-3d/Cargo.toml
git commit -m "chore: add crc32fast dependency to starbreaker-3d"
```

---

### Task 2: Implement `add_animations` on GlbBuilder

**Files:**
- Modify: `crates/starbreaker-3d/src/gltf/glb_builder.rs`

This is the core implementation. All code goes in `glb_builder.rs`.

- [ ] **Step 1: Add `animations_json` field to GlbBuilder**

Add after the `node_name_to_idx` field (line 30):

```rust
    pub animations_json: Vec<json::Animation>,
```

Initialize in `GlbBuilder::new()`:

```rust
    animations_json: Vec::new(),
```

- [ ] **Step 2: Add `add_animations` method**

Add this method to `impl GlbBuilder`, before the `finalize` method:

```rust
    /// Add animation clips to the GLB output.
    ///
    /// Matches DBA bone hashes to glTF nodes via CRC32 of node names
    /// in `self.node_name_to_idx`. Must be called after `build_nmc_hierarchy`
    /// and before `finalize`.
    ///
    /// Channels that don't match any node are silently skipped.
    pub fn add_animations(&mut self, clips: &[crate::animation::dba::AnimationClip]) {
        if clips.is_empty() {
            return;
        }

        // Build reverse map: CRC32(node_name) → glTF node index.
        let mut hash_to_node: HashMap<u32, u32> = HashMap::new();
        for (name, &node_idx) in &self.node_name_to_idx {
            let hash = crc32fast::hash(name.as_bytes());
            hash_to_node.insert(hash, node_idx);
        }

        let mut total_matched = 0u32;
        let mut total_skipped = 0u32;

        for clip in clips {
            let mut channels = Vec::new();
            let mut samplers = Vec::new();

            for bone_ch in &clip.channels {
                let Some(&node_idx) = hash_to_node.get(&bone_ch.bone_hash) else {
                    total_skipped += 1;
                    continue;
                };
                total_matched += 1;

                // Decompose animated node's matrix → TRS (glTF spec requirement).
                self.decompose_node_matrix_to_trs(node_idx as usize);

                let fps = if clip.fps > 0.0 { clip.fps } else { 30.0 };

                // Rotation channel
                if !bone_ch.rotations.is_empty() {
                    let sampler_idx = samplers.len() as u32;
                    let time_acc = self.write_time_accessor(
                        &bone_ch.rotations.iter().map(|k| k.time / fps).collect::<Vec<_>>(),
                    );
                    let rot_acc = self.write_vec4_accessor(
                        &bone_ch.rotations.iter().map(|k| k.value).collect::<Vec<_>>(),
                    );

                    samplers.push(json::animation::Sampler {
                        input: json::Index::new(time_acc),
                        output: json::Index::new(rot_acc),
                        interpolation: Checked::Valid(json::animation::Interpolation::Linear),
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

                // Translation channel
                if !bone_ch.positions.is_empty() {
                    let sampler_idx = samplers.len() as u32;
                    let time_acc = self.write_time_accessor(
                        &bone_ch.positions.iter().map(|k| k.time / fps).collect::<Vec<_>>(),
                    );
                    let pos_acc = self.write_vec3_accessor(
                        &bone_ch.positions.iter().map(|k| k.value).collect::<Vec<_>>(),
                    );

                    samplers.push(json::animation::Sampler {
                        input: json::Index::new(time_acc),
                        output: json::Index::new(pos_acc),
                        interpolation: Checked::Valid(json::animation::Interpolation::Linear),
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

            if !channels.is_empty() {
                self.animations_json.push(json::Animation {
                    name: if clip.name.is_empty() { None } else { Some(clip.name.clone()) },
                    channels,
                    samplers,
                    extensions: None,
                    extras: Default::default(),
                });
            }
        }

        log::info!("Animation export: {} channels matched, {} skipped (no node)", total_matched, total_skipped);
    }
```

- [ ] **Step 3: Add helper methods**

Add these private helpers right after `add_animations`:

```rust
    /// Decompose a node's `matrix` into TRS properties.
    /// Required by glTF spec for animation targets. No-op if already TRS or no matrix.
    fn decompose_node_matrix_to_trs(&mut self, node_idx: usize) {
        let node = &self.nodes_json[node_idx];
        let Some(matrix) = node.matrix else { return };
        if node.rotation.is_some() || node.translation.is_some() {
            return; // already decomposed
        }

        let mat = glam::Mat4::from_cols_array(&matrix);
        let (scale, rotation, translation) = mat.to_scale_rotation_translation();

        let node = &mut self.nodes_json[node_idx];
        node.matrix = None;
        node.translation = Some(translation.into());
        node.rotation = Some(json::scene::UnitQuaternion(rotation.into()));
        if (scale - glam::Vec3::ONE).length() > 1e-6 {
            node.scale = Some(scale.into());
        }
    }

    /// Write f32 scalar accessor for animation timestamps. Returns accessor index.
    fn write_time_accessor(&mut self, times: &[f32]) -> u32 {
        let byte_offset = self.bin.len();
        for &t in times {
            self.bin.extend_from_slice(&t.to_le_bytes());
        }
        let byte_length = self.bin.len() - byte_offset;
        while self.bin.len() % 4 != 0 { self.bin.push(0); }

        let min = times.iter().copied().reduce(f32::min).unwrap_or(0.0);
        let max = times.iter().copied().reduce(f32::max).unwrap_or(0.0);

        super::add_vertex_accessor(
            &mut self.buffer_views, &mut self.accessors,
            byte_offset, byte_length, times.len(),
            json::accessor::Type::Scalar, Some((&[min], &[max])),
        ).unwrap()
    }

    /// Write VEC4 f32 accessor for quaternion rotations. Returns accessor index.
    fn write_vec4_accessor(&mut self, values: &[[f32; 4]]) -> u32 {
        let byte_offset = self.bin.len();
        for v in values {
            for &f in v { self.bin.extend_from_slice(&f.to_le_bytes()); }
        }
        let byte_length = self.bin.len() - byte_offset;
        while self.bin.len() % 4 != 0 { self.bin.push(0); }

        super::add_vertex_accessor(
            &mut self.buffer_views, &mut self.accessors,
            byte_offset, byte_length, values.len(),
            json::accessor::Type::Vec4, None,
        ).unwrap()
    }

    /// Write VEC3 f32 accessor for position translations. Returns accessor index.
    fn write_vec3_accessor(&mut self, values: &[[f32; 3]]) -> u32 {
        let byte_offset = self.bin.len();
        for v in values {
            for &f in v { self.bin.extend_from_slice(&f.to_le_bytes()); }
        }
        let byte_length = self.bin.len() - byte_offset;
        while self.bin.len() % 4 != 0 { self.bin.push(0); }

        super::add_vertex_accessor(
            &mut self.buffer_views, &mut self.accessors,
            byte_offset, byte_length, values.len(),
            json::accessor::Type::Vec3, None,
        ).unwrap()
    }
```

- [ ] **Step 4: Wire animations into `finalize`**

In the `json::Root { ... }` block inside `finalize` (around line 1027), add before the `..Default::default()` line:

```rust
            animations: self.animations_json,
```

- [ ] **Step 5: Build**

Run: `cargo build -p starbreaker-3d`
Expected: compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add crates/starbreaker-3d/src/gltf/glb_builder.rs
git commit -m "feat: add GlbBuilder::add_animations for glTF animation export"
```

---

### Task 3: Thread animations through `GlbInput` and `write_glb`

**Files:**
- Modify: `crates/starbreaker-3d/src/gltf/mod.rs:13-22` (GlbInput struct)
- Modify: `crates/starbreaker-3d/src/gltf/mod.rs:129-223` (write_glb function)

- [ ] **Step 1: Add animations field to GlbInput**

In `mod.rs`, add to `GlbInput` struct after `interiors`:

```rust
    pub animations: Vec<crate::animation::dba::AnimationClip>,
```

- [ ] **Step 2: Call add_animations in write_glb**

In the `write_glb` function, add after the interiors section (after line 187 `scene_nodes.extend(interior_scene_nodes);`) and before the extras section:

```rust
    // ---- Animation tracks ----
    if !input.animations.is_empty() {
        builder.add_animations(&input.animations);
    }
```

- [ ] **Step 3: Fix all existing GlbInput construction sites**

Add `animations: Vec::new()` to every place that constructs `GlbInput`. These are:
- `crates/starbreaker-3d/src/gltf/mod.rs` test helpers `call_write_glb` and `write_glb_simple`
- `crates/starbreaker-3d/src/pipeline.rs` in `assemble_glb_with_loadout`

Search with: `grep -n "GlbInput {" crates/starbreaker-3d/src/`

Add `animations: Vec::new(),` to each.

- [ ] **Step 4: Build and run existing tests**

Run: `cargo test -p starbreaker-3d`
Expected: all existing tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/starbreaker-3d/src/gltf/mod.rs crates/starbreaker-3d/src/pipeline.rs
git commit -m "feat: thread animation clips through GlbInput and write_glb"
```

---

### Task 4: Test example — animated Zeus GLB

**Files:**
- Create: `crates/starbreaker-3d/examples/test_animated_glb.rs`

- [ ] **Step 1: Write the test example**

```rust
//! Export an animated GLB for visual testing.
//! Usage: test_animated_glb [entity_search] [dba_search] [output.glb]
//! Defaults: Zeus CL entity, Zeus ship DBA, zeus_animated.glb

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let entity_search = args.get(1).map(|s| s.as_str()).unwrap_or("Zeus CL");
    let dba_search = args.get(2).map(|s| s.as_str()).unwrap_or("Ships/RSI/Zeus.dba");
    let output_path = args.get(3).map(|s| s.as_str()).unwrap_or("zeus_animated.glb");

    // Open P4k and DataCore
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let dcb_entry = p4k.entries().iter()
        .find(|e| e.name.ends_with(".dcb"))
        .expect("no .dcb in P4k");
    let dcb_data = p4k.read(dcb_entry).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb_data).unwrap();

    // Find entity
    let records = db.records_by_type("EntityClassDefinition");
    let record = records.iter()
        .find(|r| {
            db.record_name(r)
                .map(|n| n.to_lowercase().contains(&entity_search.to_lowercase()))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("No entity matching '{entity_search}'"));

    let entity_name = db.record_name(record).unwrap_or("unknown");
    eprintln!("Entity: {entity_name}");

    // Export geometry via pipeline (no materials for speed)
    let opts = starbreaker_3d::pipeline::ExportOptions {
        material_mode: starbreaker_3d::pipeline::MaterialMode::None,
        lod_level: 0,
        texture_mip: 0,
        include_attachments: false,
        include_interior: false,
        ..Default::default()
    };

    let payload = starbreaker_3d::pipeline::export_entity_payload(&db, &p4k, record, &opts)
        .expect("failed to export entity");

    // Find and parse DBA
    let dba_search_lower = dba_search.to_lowercase();
    let dba_entry = p4k.entries().iter()
        .find(|e| e.name.to_lowercase().contains(&dba_search_lower)
            && e.name.to_lowercase().ends_with(".dba"))
        .unwrap_or_else(|| panic!("No .dba matching '{dba_search}'"));

    eprintln!("DBA: {} ({} bytes)", dba_entry.name, dba_entry.uncompressed_size);
    let dba_data = p4k.read(dba_entry).unwrap();
    let anim_db = starbreaker_3d::animation::dba::parse_dba(&dba_data)
        .expect("failed to parse DBA");
    eprintln!("Parsed {} animation clips", anim_db.clips.len());

    // Build GLB with animations
    use starbreaker_3d::gltf::*;
    let glb = write_glb(
        GlbInput {
            root_mesh: Some(payload.mesh),
            root_materials: payload.materials,
            root_textures: None,
            root_nmc: payload.nmc,
            root_palette: payload.palette,
            skeleton_bones: payload.skeleton_bones,
            children: Vec::new(),
            interiors: starbreaker_3d::pipeline::LoadedInteriors::default(),
            animations: anim_db.clips,
        },
        &mut GlbLoaders {
            load_textures: &mut |_| None,
            load_interior_mesh: &mut |_| None,
        },
        &GlbOptions {
            material_mode: starbreaker_3d::pipeline::MaterialMode::None,
            metadata: GlbMetadata {
                entity_name: Some(entity_name.to_string()),
                geometry_path: payload.geometry_path,
                material_path: payload.material_path,
                export_options: ExportOptionsMetadata {
                    material_mode: "None".into(),
                    format: "Glb".into(),
                    lod_level: 0,
                    texture_mip: 0,
                    include_attachments: false,
                    include_interior: false,
                },
            },
            fallback_palette: None,
        },
    ).expect("failed to build GLB");

    std::fs::write(output_path, &glb).unwrap();
    eprintln!("Wrote {} ({:.1} MB)", output_path, glb.len() as f64 / 1_048_576.0);
}
```

- [ ] **Step 2: Build and run**

Run: `cargo run --example test_animated_glb`
Expected: outputs `zeus_animated.glb`, prints clip count and matched animation count

- [ ] **Step 3: Visual verification**

Open `zeus_animated.glb` in a glTF viewer (VS Code glTF Tools, Blender, or online viewer).

Check for:
- Animation clips appear in the animation list
- Landing gear parts move when playing `*landing_gear*` animations
- Door animations move door parts
- No parts flying off to infinity (coordinate space issue)

- [ ] **Step 4: Commit**

```bash
git add crates/starbreaker-3d/examples/test_animated_glb.rs
git commit -m "feat: animated GLB test example (Zeus landing gear)"
```

---

### Task 5: Fix issues from visual inspection

Buffer task for post-inspection corrections.

- [ ] **Step 1: Identify issues from viewer**

Run the animation in the viewer and document what looks wrong.

- [ ] **Step 2: Apply fixes**

Common issues and fixes:
- **Nothing animates:** CRC32 hash mismatch — add debug logging to print matched/unmatched bone names
- **Parts fly to infinity:** Coordinate space issue — may need per-keyframe axis swap after all
- **Timing too fast/slow:** Check fps conversion (`time / fps` should give seconds)
- **Rotations look wrong:** Check quaternion component order or decompose_node_matrix_to_trs correctness

- [ ] **Step 3: Commit fixes**

```bash
git add -u
git commit -m "fix: animation export corrections from visual inspection"
```
