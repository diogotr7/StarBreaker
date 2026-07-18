use std::collections::HashMap;

use image::RgbaImage;
use sha2::{Digest, Sha256};
use swf::CharacterId;

use crate::error::UiError;

use super::extract::{
    extract_bitmaps_from_tags, extract_edit_text_records_from_tags,
    extract_exported_symbols_from_tags, extract_font_edit_text_metrics_from_tags,
    extract_fonts_from_tags, extract_main_timeline_labels_from_tags, extract_shapes_from_tags,
    parse_tags,
};
use super::stage::{extract_all_sprite_first_frames_from_tags, extract_all_stage_frames_from_tags};
use super::types::{EditTextRecord, FontGlyphSet, PlaceRecord, ShapeRecord, SwfEditTextMetrics};

/// Content-addressed cache of static visual atoms from one SWF.
pub struct SwfAssetLibrary {
    content_hash: String,
    bitmaps: HashMap<CharacterId, RgbaImage>,
    shapes: HashMap<CharacterId, ShapeRecord>,
    fonts: HashMap<CharacterId, FontGlyphSet>,
    exports: HashMap<String, CharacterId>,
    edit_texts: HashMap<CharacterId, EditTextRecord>,
    font_edit_text_metrics: HashMap<String, SwfEditTextMetrics>,
    frame_labels: HashMap<String, u32>,
    /// First-frame display list of every `DefineSprite`, parsed once at
    /// construction so the recursive renderer does not re-parse the SWF per node.
    sprite_first_frames: HashMap<CharacterId, Vec<PlaceRecord>>,
    /// Stage dimensions in pixels, captured once at construction.
    stage_size: (f32, f32),
    /// Per-frame main-timeline display-list snapshots (`stage_frames[N]` = frame
    /// `N`), captured once so `stage_frame` needs no re-parse.
    stage_frames: Vec<Vec<PlaceRecord>>,
    /// Final display list after all tags, returned for any out-of-range frame.
    stage_frames_tail: Vec<PlaceRecord>,
}

impl SwfAssetLibrary {
    pub fn new(swf_bytes: Vec<u8>) -> Result<Self, UiError> {
        let content_hash = {
            let mut hasher = Sha256::new();
            hasher.update(&swf_bytes);
            format!("{:x}", hasher.finalize())
        };

        let buf = parse_tags(&swf_bytes)?;
        let parsed = swf::parse_swf(&buf)?;
        let tags = &parsed.tags;

        let bitmaps = extract_bitmaps_from_tags(tags)?;
        let shapes = extract_shapes_from_tags(tags)?;
        let fonts = extract_fonts_from_tags(tags)?;
        let exports = extract_exported_symbols_from_tags(tags)?;
        let edit_texts = extract_edit_text_records_from_tags(tags)?;
        let font_edit_text_metrics = extract_font_edit_text_metrics_from_tags(tags)?;
        let frame_labels = extract_main_timeline_labels_from_tags(tags)?;
        let sprite_first_frames = extract_all_sprite_first_frames_from_tags(tags);

        let stage_size = {
            let r = buf.header.stage_size();
            (
                (r.x_max - r.x_min).to_pixels() as f32,
                (r.y_max - r.y_min).to_pixels() as f32,
            )
        };
        let (stage_frames, stage_frames_tail) = extract_all_stage_frames_from_tags(tags);
        // buf + parsed dropped here; `raw` no longer stored.

        Ok(Self {
            content_hash,
            bitmaps,
            shapes,
            fonts,
            exports,
            edit_texts,
            font_edit_text_metrics,
            frame_labels,
            sprite_first_frames,
            stage_size,
            stage_frames,
            stage_frames_tail,
        })
    }

    pub fn merge_swf_bytes(&mut self, swf_bytes: &[u8]) -> Result<(), UiError> {
        let buf = parse_tags(swf_bytes)?;
        let parsed = swf::parse_swf(&buf)?;
        let tags = &parsed.tags;

        self.bitmaps.extend(extract_bitmaps_from_tags(tags)?);
        self.shapes.extend(extract_shapes_from_tags(tags)?);
        self.fonts.extend(extract_fonts_from_tags(tags)?);

        for (symbol, metrics) in extract_font_edit_text_metrics_from_tags(tags)? {
            self.font_edit_text_metrics.entry(symbol).or_insert(metrics);
        }

        for (name, id) in extract_exported_symbols_from_tags(tags)? {
            self.exports.entry(name).or_insert(id);
        }

        Ok(())
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn get_bitmap(&self, id: CharacterId) -> Option<&RgbaImage> {
        self.bitmaps.get(&id)
    }

    pub fn get_shape(&self, id: CharacterId) -> Option<&ShapeRecord> {
        self.shapes.get(&id)
    }

    pub fn get_edit_text(&self, id: CharacterId) -> Option<&EditTextRecord> {
        self.edit_texts.get(&id)
    }

    pub fn get_font(&self, id: CharacterId) -> Option<&FontGlyphSet> {
        self.fonts.get(&id)
    }

    pub fn find_font_by_name(&self, query: &str) -> Option<&FontGlyphSet> {
        let q = query.to_ascii_lowercase();
        // Iterate fonts by ascending CharacterId (their SWF definition order)
        // rather than `HashMap` iteration order, which varies per process.
        // Multiple embedded font subsets can share a logical name; HashMap order
        // made the choice — and therefore which glyphs were available — non-
        // deterministic across renders, flipping glyph-missing digits between
        // values. Matching rule is unchanged (exact or substring, first wins).
        let mut ids: Vec<CharacterId> = self.fonts.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().map(|id| &self.fonts[&id]).find(|font| {
            font.name.eq_ignore_ascii_case(&q) || font.name.to_ascii_lowercase().contains(&q)
        })
    }

    pub fn lookup_export(&self, name: &str) -> Option<CharacterId> {
        self.exports.get(name).copied()
    }

    pub fn export_entries(&self) -> impl Iterator<Item = (&str, CharacterId)> {
        self.exports.iter().map(|(name, &id)| (name.as_str(), id))
    }

    pub fn font_edit_text_metrics(&self, symbol: &str) -> Option<&SwfEditTextMetrics> {
        self.font_edit_text_metrics.get(symbol)
    }

    /// Return the 0-based frame index for a main-timeline `FrameLabel`, or `None`
    /// if the SWF has no such label.
    pub fn frame_label_index(&self, label: &str) -> Option<u32> {
        self.frame_labels.get(label).copied()
    }

    pub fn get_sprite_first_frame(&self, sprite_id: CharacterId) -> Result<Vec<PlaceRecord>, UiError> {
        self.sprite_first_frames
            .get(&sprite_id)
            .cloned()
            .ok_or_else(|| UiError::UnsupportedTag(format!("DefineSprite id={sprite_id} not found")))
    }

    /// First-frame display list of `character_id` from the construction-time
    /// cache (empty if `character_id` is not a `DefineSprite`).
    pub fn extract_sprite_first_frame(&self, character_id: CharacterId) -> Vec<PlaceRecord> {
        self.sprite_first_frames
            .get(&character_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn export_name_for(&self, character_id: CharacterId) -> Option<String> {
        self.exports
            .iter()
            .find_map(|(name, &id)| (id == character_id).then(|| name.clone()))
    }

    pub fn stage_frame(&self, frame_index: u32) -> Vec<PlaceRecord> {
        self.stage_frames
            .get(frame_index as usize)
            .cloned()
            .unwrap_or_else(|| self.stage_frames_tail.clone())
    }

    pub fn stage_size(&self) -> (f32, f32) {
        self.stage_size
    }

    pub fn stage_visual_bounds(&self, frame_index: u32) -> Option<(f32, f32, f32, f32)> {
        let stage_places = self.stage_frame(frame_index);
        if stage_places.is_empty() {
            return None;
        }

        let mut out: Option<(f32, f32, f32, f32)> = None;
        for place in &stage_places {
            let matrix = Matrix2d::from_swf(&place.matrix);
            self.accumulate_visual_bounds(place.character_id, matrix, 6, &mut out);
        }
        out
    }

    pub fn bitmap_count(&self) -> usize {
        self.bitmaps.len()
    }

    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    pub fn font_count(&self) -> usize {
        self.fonts.len()
    }

    pub fn export_count(&self) -> usize {
        self.exports.len()
    }

    pub fn visual_exports(&self) -> impl Iterator<Item = CharacterId> + '_ {
        self.exports
            .iter()
            .filter(|(name, _)| !name.starts_with("__Packages."))
            .map(|(_, &id)| id)
    }

    fn accumulate_visual_bounds(
        &self,
        character_id: CharacterId,
        matrix: Matrix2d,
        max_depth: u8,
        out: &mut Option<(f32, f32, f32, f32)>,
    ) {
        if let Some(shape) = self.get_shape(character_id) {
            if !shape_contributes_visual_bounds(shape) {
                return;
            }

            let b = &shape.shape_bounds;
            let bx0 = b.x_min.to_pixels() as f32;
            let by0 = b.y_min.to_pixels() as f32;
            let bx1 = b.x_max.to_pixels() as f32;
            let by1 = b.y_max.to_pixels() as f32;
            let corners = [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)];

            let mut min_x = f32::INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut max_y = f32::NEG_INFINITY;

            for (x, y) in corners {
                let (tx, ty) = matrix.apply(x, y);
                min_x = min_x.min(tx);
                min_y = min_y.min(ty);
                max_x = max_x.max(tx);
                max_y = max_y.max(ty);
            }

            if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
                *out = Some(match *out {
                    Some((ox0, oy0, ox1, oy1)) => {
                        (ox0.min(min_x), oy0.min(min_y), ox1.max(max_x), oy1.max(max_y))
                    }
                    None => (min_x, min_y, max_x, max_y),
                });
            }
            return;
        }

        if max_depth == 0 {
            return;
        }

        let sprite_places = self.extract_sprite_first_frame(character_id);
        if sprite_places.is_empty() {
            return;
        }

        for place in &sprite_places {
            let child_matrix = matrix.compose(Matrix2d::from_swf(&place.matrix));
            self.accumulate_visual_bounds(
                place.character_id,
                child_matrix,
                max_depth.saturating_sub(1),
                out,
            );
        }
    }
}

fn shape_contributes_visual_bounds(shape: &ShapeRecord) -> bool {
    let has_opaque_fill = shape
        .fill_styles
        .iter()
        .any(|fill| matches!(fill, swf::FillStyle::Color(c) if c.a > 0));
    if has_opaque_fill {
        return true;
    }

    shape.line_styles.iter().any(|line| {
        line.width().to_pixels() > 0.0
            && matches!(line.fill_style(), swf::FillStyle::Color(c) if c.a > 0)
    })
}

#[derive(Clone, Copy)]
struct Matrix2d {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl Matrix2d {
    fn from_swf(m: &swf::Matrix) -> Self {
        Self {
            a: m.a.to_f32(),
            b: m.b.to_f32(),
            c: m.c.to_f32(),
            d: m.d.to_f32(),
            tx: m.tx.to_pixels() as f32,
            ty: m.ty.to_pixels() as f32,
        }
    }

    fn compose(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
            ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
        }
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }
}
