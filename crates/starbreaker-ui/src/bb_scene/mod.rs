mod clone_expand;
mod fields;
mod parse;
#[cfg(test)]
mod tests;
mod types;

pub use parse::parse_bb_canvas;
pub use types::*;

// Re-parse helper for post-merge forwarding: widget-standard expansion mutates
// a merged instance's raw `iconProperties` and must rebuild the parse-time
// `BbIcon` (which bakes the preset→SVG resolution) so the draw sees the
// forwarded glyph, not the template's authored default.
pub(crate) use fields::parse_icon;
