# glTF Animation Export — Design Spec

**Date:** 2026-04-04
**Scope:** Add animation tracks to GLB export for ship/NMC node animations (landing gear, doors, etc.)
**Out of scope:** Skeleton skinning (vertex weights), character animations, pipeline integration

## Context

The DBA parser (`animation/dba.rs`) produces `AnimationDatabase` with per-bone keyframe data (rotation quaternions + position vectors). The GLB builder already exports NMC node hierarchies with a `node_name_to_idx` name→index map. This spec connects the two: DBA animation clips become standard glTF animation tracks targeting NMC nodes.

This is the same code path the future Blender plugin will consume — Blender's built-in glTF importer reads these animation tracks natively.

## Bone Hash → Node Matching

DBA animations key channels by `bone_hash: u32` — the CRC32 of the lowercase bone name. GlbBuilder's `node_name_to_idx: HashMap<String, u32>` maps lowercase node names to glTF node indices (populated by `build_nmc_hierarchy`).

Resolution:
1. Build a reverse map: iterate `node_name_to_idx`, compute `CRC32(name)` for each entry → `HashMap<u32, u32>` (bone_hash → gltf_node_idx)
2. For each `BoneChannel`, look up `bone_hash` in the reverse map
3. Skip channels with no matching node (expected — not all DBA bones correspond to geometry nodes)

CRC32 uses polynomial `0xEDB88320` on lowercase ASCII, matching the DBA format.

## glTF Animation Structure

Per `AnimationClip`:

```
gltf_json::Animation {
    name: clip.name,
    channels: [
        // one per (matched_bone, property) pair
        Channel { sampler: N, target: { node: X, path: Rotation } },
        Channel { sampler: N+1, target: { node: X, path: Translation } },
        ...
    ],
    samplers: [
        // one per channel
        Sampler { input: time_accessor, output: value_accessor, interpolation: Linear },
        ...
    ],
}
```

Each sampler gets:
- **Input accessor**: `f32` timestamps in seconds (`frame / fps`)
- **Output accessor**: `[f32; 4]` quaternions for rotation, `[f32; 3]` vectors for translation

Accessor data is appended to the GLB binary buffer. Component types: `F32`, accessor types: `VEC4` (rotation) / `VEC3` (translation) / `SCALAR` (time). Min/max are set on time accessors (required by spec).

## Coordinate System

CryEngine is Z-up right-handed. The GLB export wraps all content under a `CryEngine_Z_up` root node with a Z→Y-up rotation matrix. Since NMC node transforms are in CryEngine space and animation keyframes are also in CryEngine space, the root node's transform handles the conversion automatically. **No per-keyframe coordinate conversion is needed.**

## Quaternion Format

- DBA output: `[x, y, z, w]`
- glTF spec: `[x, y, z, w]`
- No swizzle or conversion needed.

## Animated Nodes: Matrix → TRS Decomposition

The glTF spec requires that animation target nodes use `translation`/`rotation`/`scale` properties, not `matrix`. Currently `build_nmc_hierarchy` sets `matrix` on NMC nodes.

When `add_animations` matches a bone hash to a node, it must:
1. Read the node's current `matrix` (the rest pose)
2. Decompose it into `translation` + `rotation` + `scale` (via `glam::Mat4::to_scale_rotation_translation`)
3. Clear `matrix`, set the TRS properties instead
4. The animation keyframes then override these TRS values at playback

Nodes that are NOT animation targets keep their `matrix` unchanged.

## API

### GlbBuilder method

```rust
impl GlbBuilder {
    /// Add animation clips to the GLB output.
    ///
    /// Matches DBA bone hashes to glTF nodes via CRC32 of node names
    /// in `self.node_name_to_idx`. Must be called after `build_nmc_hierarchy`
    /// and before `finalize`.
    ///
    /// Channels that don't match any node are silently skipped.
    pub fn add_animations(&mut self, clips: &[AnimationClip])
}
```

Internally stores `Vec<json::Animation>` and appends accessor/buffer data to the existing buffer.

### finalize changes

`finalize()` already builds the `json::Root`. Add:
```rust
root.animations = self.animations_json;
```

No new fields on `GlbInput` yet — that's pipeline integration (future).

## Test Example

`examples/test_animated_glb.rs`:

1. Load Zeus entity via `export_entity_payload` (reuses existing pipeline)
2. Search P4k for `Animations/Spaceships/Ships/RSI/Zeus.dba`
3. Parse DBA → `AnimationDatabase`
4. After `build_nmc_hierarchy`, call `builder.add_animations(&db.clips)`
5. `finalize()` → write `zeus_animated.glb` to disk
6. Verify: open in VS Code glTF viewer / Blender / https://gltf-viewer.donmccurdy.com

Success criteria: landing gear deploy/retract animation plays correctly with visible bone movement.

## Files Modified

| File | Change |
|------|--------|
| `crates/starbreaker-3d/src/gltf/glb_builder.rs` | Add `add_animations` method, `animations_json` field, accessor/buffer writes |
| `crates/starbreaker-3d/src/gltf/glb_builder.rs` | Update `finalize` to include animations in root |
| `crates/starbreaker-3d/examples/test_animated_glb.rs` | New test example |

## Not Included

- No CLI flag changes
- No `GlbInput` changes
- No pipeline integration (chrparams→DBA auto-discovery)
- No skeleton skinning / vertex weights
- No character animation support
