//! SOC (Static Object Container) parser suite.
//!
//! `.soc` files are CrCh-format binary blobs that hold the static-object
//! data for one logical "zone" — its brush instances (chunk 0x0010), its
//! entity placements (chunk 0x0004), and its visibility-area / portal
//! graph (chunk 0x000E). A whole level (e.g. an Executive Hangar) is
//! composed by nesting many SOC files together: a top-level "assembly"
//! socpak references one or more child socpaks with per-child
//! position / rotation, and each of those may in turn reference its own
//! children. This module exposes the four pieces required to walk that
//! graph:
//!
//! - [`brushes`] — chunk 0x0010 brush-instance scan + LOD gate +
//!   parent-QuatTS composition.
//! - [`entities`] — chunk 0x0004 CryXmlB walk for lights / doors / loot
//!   / mesh-bearing entities.
//! - [`visarea`] — chunk 0x000E vis-area / portal records.
//! - [`scene`] — multi-zone composition (`CZoneSystem::GetTransform`-
//!   style transform chaining).
//!
//! The brush parser is the only part with hard-format-version checking;
//! the others soft-fail when a chunk is missing or the version doesn't
//! match v15 (other versions appear in older builds and player-hangar
//! variants, which are out of scope for this iteration).

mod common;

pub mod brushes;
pub mod catalog;
pub mod entities;
pub mod render;
pub mod scene;
pub mod visarea;

// Re-export the brush parser API at the module root to preserve the
// public surface that callers (and existing tests) already rely on.
pub use brushes::{
    BrushInstance, ParentTransform, SocBrushes, SocError, parse, parse_with_identity_parent,
};

// Catalog enumeration (data-driven Maps tab list).
pub use catalog::{
    MIN_SOCPAK_SIZE_BYTES, SceneCatalogEntry, SceneSourceKind, SocpakDirEntry,
    SocpakDirEntryKind, enumerate_scene_roots, list_all_socpaks, list_socpak_dir,
};

// Entity / visarea / scene types are namespaced under their submodules
// so the names stay legible at the call site.
pub use entities::{EntityKind, EntityPlacement, SocEntities};
pub use render::{
    DEFAULT_MAX_EMITTED_LIGHTS, EmitOptions, GlbEmitSummary, LightDescriptor, LightInstance,
    LightKind, MeshPlacement, RenderableScene, ResolvedMesh, emit_glb, emit_glb_with_options,
    resolve_scene, resolve_scene_with_progress,
};
pub use scene::{
    ChildSocpakRef, ComposedScene, SceneError, SceneZone, compose_from_flat_list,
    compose_from_root, read_child_refs_from_socpak,
};
pub use visarea::{SocVisAreas, VisAreaRecord, VisAreaRole};
