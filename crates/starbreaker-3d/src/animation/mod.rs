//! Parser for `.dba` (Animation Database) and `.caf` (Animation Clip) IVO
//! files.
//!
//! Both formats use IVO container with animation blocks. A `.dba` packs
//! multiple clips, while `.caf` has a single clip.
//!
//! ## Block structure
//!
//! ```text
//! Header (12 bytes): signature("#caf"/"#dba") + bone_count(u16) + magic(u16) + data_size(u32)
//! Bone hashes:    [u32; bone_count]  — CRC32 of bone names
//! Controllers:    [ControllerEntry; bone_count]  — 24 bytes each (rot track + pos track)
//! Keyframe data at offsets referenced by controllers (relative to each controller's own offset)
//! ```
//!
//! Cross-validated against the reference implementation on
//! `diogotr7/StarBreaker` (commit
//! [`d01ae21`](https://github.com/diogotr7/StarBreaker/commit/d01ae217fb74bebf1fede7cd45a82b758f44cbb6)
//! on branch `feature/animation`) and the gate test for the Scorpius rear
//! gear (`docs/StarBreaker/animation-research.md`). The `SmallTree48BitQuat`
//! decoder follows the Ghidra-confirmed bit layout (sign-bit borrow across
//! u16 boundaries).

mod codec;
pub mod pose;
pub mod caf;
pub mod dba;
pub mod to_clip;
pub use to_clip::shared_clip;
pub use pose::{
    apply_pose_to_skeleton, bone_name_hash, clip_final_pose, cry_xyzw_to_blender_wxyz,
    find_block_for_skeleton, read_dba_final_pose, BonePose, BoneTransforms,
};
pub use caf::parse_caf;
pub use dba::parse_dba;
pub mod binding_report;
pub mod serialise;
pub mod mannequin;
pub mod matching;
pub use serialise::{
    annotate_animations_json_with_blend_modes, classify_bone_blend_modes,
    clip_to_json, database_to_animations_json, dump_database_to_json,
    sanitize_clip_filename, split_clip_for_sidecar, BoneBlendMode,
};
pub use mannequin::{
    annotate_animation_fragments_json, dump_mannequin_adb_to_json,
};
pub use matching::{
    caf_anchored_remap_decisions, extract_animations_for_skeleton_json,
    ClipMatchDecision,
};

// ── Public types ────────────────────────────────────────────────────────────

/// A parsed animation database containing one or more animation clips.
#[derive(Debug, Clone)]
pub struct AnimationDatabase {
    pub clips: Vec<AnimationClip>,
}

/// A single animation clip with per-bone channels.
#[derive(Debug, Clone)]
pub struct AnimationClip {
    /// Animation name (from DBA metadata, or filename for CAF).
    pub name: String,
    /// Frames per second (from metadata, default 30).
    pub fps: f32,
    /// Per-bone animation channels.
    pub channels: Vec<BoneChannel>,
    /// DBA-metadata `start_rotation` (CryEngine xyzw quaternion) when this
    /// clip came from a DBA. Documented in §14.6 of the SC animation
    /// formats whitepaper as the authored clip-origin rotation; the
    /// addon uses it as the data-backed anchor for the composition
    /// `result = bind ⋅ (start⁻¹ ⋅ sample)`. `None` for `.caf` files
    /// (no DBA metadata block) and for clips whose metadata-vs-block
    /// match-up failed.
    pub start_rotation: Option<[f32; 4]>,
    /// DBA-metadata `start_position` (CryEngine XYZ). The on-disk DBA
    /// entry only stores XY (Z is implicit zero); we expose the full
    /// 3-component vector here so downstream consumers don't have to
    /// know the file-format quirk. `None` when no DBA metadata is
    /// available (`.caf` files, or unmatched DBA entries).
    pub start_position: Option<[f32; 3]>,
}

/// DataCore-declared Mannequin animation-controller sources for an entity.
#[derive(Debug, Clone)]
pub struct AnimationControllerSource {
    pub animation_database: String,
    pub animation_controller: String,
}

/// Animation data for a single bone.
#[derive(Debug, Clone)]
pub struct BoneChannel {
    /// CRC32 hash of the bone name.
    pub bone_hash: u32,
    /// Rotation keyframes (time in frames, quaternion XYZW).
    pub rotations: Vec<Keyframe<[f32; 4]>>,
    /// Position keyframes (time in frames, XYZ).
    pub positions: Vec<Keyframe<[f32; 3]>>,
    /// Raw 16-bit `rot_format_flags` from the CAF/DBA controller entry.
    /// Currently understood as the rotation-keyframe encoding format
    /// (e.g. quaternion compression). Captured verbatim so debug
    /// tooling can hunt for additive/override bits (Phase 37).
    pub rot_format_flags: u16,
    /// Raw 16-bit `pos_format_flags` from the CAF/DBA controller
    /// entry. Captured verbatim alongside `rot_format_flags`.
    pub pos_format_flags: u16,
}

/// A single keyframe with time and value.
#[derive(Debug, Clone)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
}

// ── Internal types ──────────────────────────────────────────────────────────



/// IVO chunk type IDs for animation data.
pub(super) mod chunk_types {
    pub const DBA_DATA: u32 = 0x194FBC50; // IvoDBAData
    pub const DBA_META: u32 = 0xF7351608; // IvoDBAMetadata
    pub const CAF_DATA: u32 = 0xA9496CB5; // IvoCAFData
    pub const ANIM_INFO: u32 = 0x4733C6ED; // IvoAnimInfo
}



#[cfg(test)]
mod bake_tests;

