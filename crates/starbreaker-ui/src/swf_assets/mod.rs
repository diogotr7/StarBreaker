//! SWF static-atom extractor API.

mod decode;
mod extract;
mod library;
mod stage;
#[cfg(test)]
mod tests;
mod types;

pub use extract::{
    extract_bitmaps, extract_edit_text_records, extract_exported_symbols,
    extract_font_edit_text_metrics, extract_fonts, extract_shapes,
};
pub use library::SwfAssetLibrary;
pub use stage::{extract_all_sprite_first_frames, extract_sprite_first_frame, extract_stage_frame, extract_stage_size};
pub use types::{EditTextRecord, FontGlyph, FontGlyphSet, PlaceRecord, ShapeRecord, SwfEditTextMetrics, build_glyph_index};
