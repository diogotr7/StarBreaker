//! Canonical IR renderer/compositor — rasterises a `UiIrDocument` (fills,
//! borders, SVG/bitmap assets, text, holograms) into the final screen image.
//!
//! Implementation is split across two sibling modules (converted from the
//! former `include!`-spliced `engine_parts/*.part` chunks — review F1, ledger
//! 104); the public surface is glob re-exported so call sites are unchanged.

mod engine_01;
mod engine_02;

pub use engine_01::*;
// engine_02 currently exports no `pub` items — its cross-module surface is
// `pub(crate)`; widen to `pub use` if it ever gains public API.
pub(crate) use engine_02::*;
