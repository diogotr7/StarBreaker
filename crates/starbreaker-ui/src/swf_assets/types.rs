use swf::{CharacterId, ColorTransform, Depth, Matrix};

/// An extracted SWF shape character with style and edge records.
#[derive(Clone, Debug)]
pub struct ShapeRecord {
    pub id: CharacterId,
    pub shape_bounds: swf::Rectangle<swf::Twips>,
    pub fill_styles: Vec<swf::FillStyle>,
    pub line_styles: Vec<swf::LineStyle>,
    pub records: Vec<swf::ShapeRecord>,
}

/// A single glyph within a [`FontGlyphSet`].
#[derive(Clone, Debug)]
pub struct FontGlyph {
    pub code: Option<u16>,
    pub advance: Option<i16>,
    pub shape_records: Vec<swf::ShapeRecord>,
}

/// All glyphs for one extracted SWF font character.
#[derive(Clone, Debug)]
pub struct FontGlyphSet {
    pub id: CharacterId,
    pub name: String,
    pub is_bold: bool,
    pub is_italic: bool,
    pub ascent: Option<u16>,
    pub descent: Option<u16>,
    pub leading: Option<i16>,
    pub glyphs: Vec<FontGlyph>,
    /// `glyph.code` → index into `glyphs`, first occurrence wins (matching the
    /// linear `find` this replaces). Built once at extraction so glyph lookup
    /// during text layout is O(1) instead of a scan per character.
    pub glyph_index: std::collections::HashMap<u16, usize>,
}

/// Build [`FontGlyphSet::glyph_index`] for an extracted glyph list.
pub fn build_glyph_index(glyphs: &[FontGlyph]) -> std::collections::HashMap<u16, usize> {
    let mut index = std::collections::HashMap::with_capacity(glyphs.len());
    for (idx, glyph) in glyphs.iter().enumerate() {
        if let Some(code) = glyph.code {
            index.entry(code).or_insert(idx);
        }
    }
    index
}

/// An extracted `DefineEditText` character from a SWF.
///
/// Stores the bounds, font reference, HTML flag, and initial text verbatim
/// (with `@…` loc keys unresolved).  Rendering is handled by the
/// `swf_render::edit_text` module, which resolves loc keys and calls
/// `TextRenderer::draw_swf_font`.
#[derive(Clone, Debug)]
pub struct EditTextRecord {
    pub id: CharacterId,
    pub bounds: swf::Rectangle<swf::Twips>,
    pub is_html: bool,
    /// Raw `initial_text` from the SWF (may start with `@` loc key).
    pub initial_text: Option<String>,
    pub font_id: Option<CharacterId>,
    pub font_height_px: Option<f32>,
}

/// Source `DefineEditText` metrics for an imported/exported font symbol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwfEditTextMetrics {
    pub height_px: f32,
    pub bounds_y_min_px: f32,
    pub bounds_y_max_px: f32,
}

impl SwfEditTextMetrics {
    /// Scale source text-field overscan to the renderer's current font size.
    pub fn top_overscan_px(self, size_px: f32) -> f32 {
        if self.height_px <= 0.0 || !self.height_px.is_finite() || !size_px.is_finite() {
            return 0.0;
        }
        let above = (-self.bounds_y_min_px).max(0.0);
        let below = (self.bounds_y_max_px - self.height_px).max(0.0);
        ((above + below) * (size_px / self.height_px)).max(0.0)
    }
}

/// A single placed child from a sprite/stage frame display list.
#[derive(Clone, Debug)]
pub struct PlaceRecord {
    pub depth: Depth,
    pub character_id: CharacterId,
    pub matrix: Matrix,
    pub color_transform: Option<ColorTransform>,
    pub name: Option<String>,
    /// When set, this character acts as a clipping mask for depths
    /// `depth + 1` through `clip_depth` (inclusive).
    pub clip_depth: Option<Depth>,
}
