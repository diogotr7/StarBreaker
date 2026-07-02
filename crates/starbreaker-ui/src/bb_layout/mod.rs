//! BuildingBlocks layout engine — resolves authored positions/sizing/flex/
//! anchor/padding into per-node layout rects, including the screen-aspect
//! fit branches (cover/fill/letterbox) for RTT screens.
//!
//! Implementation is split across three sibling modules (converted from the
//! former `include!`-spliced `engine_parts/*.part` chunks — review F1, ledger
//! 104); the public surface is glob re-exported so call sites are unchanged.

mod engine_01;
mod engine_02;
mod engine_03;

pub use engine_01::*;
// engines 02-03 currently export no `pub` items — their cross-module surface
// is `pub(crate)`; widen a glob to `pub use` if one ever gains public API.
// The later globs are unused TODAY (modules depend earlier-only) but define
// the namespace `use super::*` resolves against.
#[allow(unused_imports)]
pub(crate) use engine_02::*;
#[allow(unused_imports)]
pub(crate) use engine_03::*;
