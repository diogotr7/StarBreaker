# glTF Skinned Mesh Animation — Design Spec

**Date:** 2026-04-04
**Scope:** Export standalone skinned animated GLBs from `.cdf` + `.chr` + `.skin` + DBA files (e.g. landing gear)
**Out of scope:** Integration into full ship assembly, character animations

## Context

Ship landing gear (and other CDF entities) use skeletal skinning: a `.chr` skeleton drives a `.skin` mesh via vertex weights. The DBA animation targets skeleton bones by `controller_id` (a u32 stored per-bone in CompiledBones). This is distinct from the NMC node animation system already implemented.

Vertex weights are already parsed (`IVOBONEMAP`/`IVOBONEMAP32` → `BoneMapping`). Skeleton bones are parsed but currently discard `controller_id` and `parent_index`. The glTF builder has no skin support.

## Skeleton Changes

The `Bone` struct needs additional fields from CompiledBones:

```rust
pub struct Bone {
    pub name: String,
    pub controller_id: u32,       // NEW: CRC32 hash used by animation system
    pub parent_index: i32,        // NEW: -1 for root
    pub world_position: [f32; 3],
    pub world_rotation: [f32; 4], // [w, x, y, z]
}
```

Both `parse_compiled_bones_v900` and `parse_compiled_bones_v901` already read the raw entries containing these fields — just need to preserve them on the output struct.

## glTF Skin Structure

A glTF skin requires:
- **Joint nodes**: one glTF node per skeleton bone, arranged in a parent-child hierarchy
- **Inverse bind matrices**: one 4x4 matrix per joint (the `world_to_bone` transform from the skeleton)
- **Vertex attributes**: `JOINTS_0` (u16x4) and `WEIGHTS_0` (f32x4) on each mesh primitive
- **Mesh node**: references the skin

The inverse bind matrix for each bone is constructed from the bone's world transform:
`inverse_bind = inverse(mat4_from_rotation_translation(world_rotation, world_position))`

## Animation Matching

DBA bone hashes match against skeleton `controller_id`, NOT CRC32 of the bone name. Confirmed via Ghidra: the engine does a binary search of the DBA hash array using controller_ids from the skeleton.

The `add_animations` method needs a second matching path: when a skin is present, build the hash→node map from `controller_id` → joint node index instead of `CRC32(node_name)` → node index.

## API

```rust
/// Build a skinned animated GLB from CDF components.
/// skin_data: .skin/.skinm file bytes (mesh + vertex weights)
/// chr_data: .chr file bytes (skeleton with controller_ids)
/// dba_data: optional DBA file bytes (animation clips)
pub fn skinned_mesh_to_glb(
    skin_data: &[u8],
    skinm_data: &[u8],
    chr_data: &[u8],
    dba_data: Option<&[u8]>,
) -> Result<Vec<u8>, Error>
```

## GlbBuilder Changes

### New method: `add_skin`

```rust
pub fn add_skin(
    &mut self,
    bones: &[Bone],
    mesh_node_idx: u32,
) -> HashMap<u32, u32>  // returns controller_id → joint_node_idx map
```

1. Create one glTF node per bone with parent-child hierarchy (from `parent_index`)
2. Compute inverse bind matrices: `inverse(mat4(world_rot, world_pos))`
3. Write inverse bind matrix accessor to binary buffer
4. Create `gltf_json::Skin` with joints array + IBM accessor
5. Set `skin` on the mesh node
6. Return controller_id → node_idx map for animation matching

### New method: `write_skin_attributes`

Writes `JOINTS_0` and `WEIGHTS_0` vertex attributes to the binary buffer and adds them to mesh primitives. Called during `pack_mesh` when `bone_mappings` is present.

### Updated: `add_animations`

Accept an optional `controller_id_map: Option<&HashMap<u32, u32>>`. When provided, use it for bone hash matching instead of the CRC32-of-name map.

## Test Example

`examples/test_skinned_glb.rs`:
1. Load a landing gear `.cdf` → resolve `.chr` + `.skin` paths
2. Load the landing gear DBA
3. Call `skinned_mesh_to_glb`
4. Write to disk, verify in Blender

Test with Gladius or Hornet front landing gear (which should have matching controller_ids between CHR and DBA).

## Files Modified

| File | Change |
|------|--------|
| `skeleton.rs` | Add `controller_id`, `parent_index` to `Bone`; update both parsers |
| `glb_builder.rs` | Add `add_skin`, `write_skin_attributes`; update `add_animations` |
| `gltf/mod.rs` | Add `skinned_mesh_to_glb` public function |
| `lib.rs` | Re-export `skinned_mesh_to_glb` |
| `examples/test_skinned_glb.rs` | New test example |
