//! Canonical UI IR schema + compiler — builds the deterministic
//! `UiIrDocument` (layout rects, resolved text/colours, asset refs) from a
//! resolved BuildingBlocks scene.
//!
//! Implementation is split across four sibling modules (converted from the
//! former `include!`-spliced `engine_parts/*.part` chunks — review F1, ledger
//! 104); the public surface is glob re-exported so call sites are unchanged.

mod engine_01;
mod engine_02;
mod engine_03;
mod engine_04;

pub use engine_01::*;
// engines 02-04 currently export no `pub` items — their cross-module surface
// is `pub(crate)`; widen a glob to `pub use` if one ever gains public API.
// The later globs are unused TODAY (modules depend earlier-only) but define
// the namespace `use super::*` resolves against.
pub(crate) use engine_02::*;
#[allow(unused_imports)]
pub(crate) use engine_03::*;
#[allow(unused_imports)]
pub(crate) use engine_04::*;
