//! Canvas graph resolver — pass-1/pass-2 canvas-reference resolution, widget
//! standard expansion, modular-kit sheet application, and style-cascade
//! orchestration for BuildingBlocks scenes.
//!
//! Implementation is split across four sibling modules (converted from the
//! former `include!`-spliced `engine_parts/*.part` chunks — review F1, ledger
//! 104); their public surface is glob re-exported here so call sites are
//! unchanged. Later modules may depend on earlier ones; cross-module items are
//! `pub(crate)`.

mod engine_01;
mod engine_02;
mod engine_03;
mod engine_04;

pub use engine_01::*;
// engines 02–04 currently export no `pub` items — their cross-module surface
// is `pub(crate)`; widen a glob to `pub use` if one ever gains public API.
pub(crate) use engine_02::*;
pub(crate) use engine_03::*;
pub(crate) use engine_04::*;
