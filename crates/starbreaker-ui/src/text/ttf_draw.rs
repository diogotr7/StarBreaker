use image::RgbaImage;
use rusttype::{Point, Scale, point};

use crate::bb_layout::Rect;

use super::{FontKind, FontStore, TextAlign, VerticalAlign};

/// Text renderer for bundled TTF font paths.
pub struct TextRenderer {
    fonts: FontStore,
}

impl TextRenderer {
    /// Construct by loading fonts from embedded byte arrays.
    pub fn new() -> Self {
        Self {
            fonts: FontStore::new(),
        }
    }

    /// Return the pixel width and height of `text` at `size_px` without wrapping.
    pub fn measure(&self, text: &str, kind: FontKind, size_px: f32) -> (f32, f32) {
        let font = self.fonts.font(kind);
        let scale = Scale::uniform(size_px);
        let v_metrics = font.v_metrics(scale);
        let h = (v_metrics.ascent - v_metrics.descent).ceil();
        let w = line_advance_width(font, text, scale);
        (w, h)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        img: &mut RgbaImage,
        text: &str,
        rect: Rect,
        kind: FontKind,
        size_px: f32,
        colour: [u8; 4],
        align: TextAlign,
        vertical_align: VerticalAlign,
        line_spacing_px: Option<f32>,
    ) {
        if text.is_empty() || rect.w < 1.0 || rect.h < 1.0 || size_px < 1.0 {
            return;
        }

        let font = self.fonts.font(kind);
        let scale = Scale::uniform(size_px);
        let v_metrics = font.v_metrics(scale);
        let line_spacing = line_spacing_px.filter(|value| value.is_finite()).unwrap_or(0.0);
        let line_h = (v_metrics.ascent - v_metrics.descent + v_metrics.line_gap + line_spacing)
            .ceil()
            .max(1.0);

        let lines = wrap_lines(font, text, scale, rect.w);
        let total_h = lines.len() as f32 * line_h;
        let block_top = match vertical_align {
            VerticalAlign::Top => rect.y,
            VerticalAlign::Centre => rect.y + ((rect.h - total_h) * 0.5).max(0.0),
            VerticalAlign::Bottom => rect.y + (rect.h - total_h).max(0.0),
            // Baseline at rect centre + em/2 (GFx line box above the baseline,
            // centred on the anchor) — see `VerticalAlign::EmBaseline`.
            VerticalAlign::EmBaseline => {
                rect.y + rect.h * 0.5 + size_px * 0.5 - v_metrics.ascent
            }
        };
        let start_baseline = block_top + v_metrics.ascent;

        let img_w = img.width() as i32;
        let img_h = img.height() as i32;
        let clip_min_x = rect.x.floor().max(0.0) as i32;
        let clip_min_y = rect.y.floor().max(0.0) as i32;
        let clip_max_x = (rect.x + rect.w).ceil().min(img.width() as f32) as i32;
        let clip_max_y = (rect.y + rect.h).ceil().min(img.height() as f32) as i32;

        for (i, line) in lines.iter().enumerate() {
            let baseline_y = start_baseline + i as f32 * line_h;
            let line_w = line_advance_width(font, line, scale);

            let start_x = match align {
                TextAlign::Left => rect.x,
                TextAlign::Centre => rect.x + ((rect.w - line_w) * 0.5).max(0.0),
                TextAlign::Right => rect.x + (rect.w - line_w).max(0.0),
            };

            let origin: Point<f32> = point(start_x, baseline_y);
            for g in font.layout(line, scale, origin) {
                if let Some(bb) = g.pixel_bounding_box() {
                    g.draw(|gx, gy, coverage| {
                        if coverage < 1e-4 {
                            return;
                        }
                        let px = bb.min.x + gx as i32;
                        let py = bb.min.y + gy as i32;
                        if px < clip_min_x
                            || py < clip_min_y
                            || px >= clip_max_x
                            || py >= clip_max_y
                            || px >= img_w
                            || py >= img_h
                        {
                            return;
                        }
                        let pixel = img.get_pixel_mut(px as u32, py as u32);
                        // Straight-alpha source-over in LINEAR light. `src_a` folds
                        // glyph coverage × the colour's own alpha; `colour` is straight
                        // sRGB. Text antialiasing blends in linear, matching the engine.
                        let src_a = coverage * colour[3] as f32 / 255.0;
                        crate::colour::blend_straight_linear(
                            &mut pixel.0,
                            [colour[0], colour[1], colour[2]],
                            src_a,
                        );
                    });
                }
            }
        }
    }

    pub fn measure_drawn_bounds(
        &self,
        text: &str,
        rect: Rect,
        kind: FontKind,
        size_px: f32,
        align: TextAlign,
        vertical_align: VerticalAlign,
        line_spacing_px: Option<f32>,
    ) -> Option<Rect> {
        if text.is_empty() || rect.w < 1.0 || rect.h < 1.0 || size_px < 1.0 {
            return None;
        }

        let font = self.fonts.font(kind);
        let scale = Scale::uniform(size_px);
        let v_metrics = font.v_metrics(scale);
        let line_spacing = line_spacing_px.filter(|value| value.is_finite()).unwrap_or(0.0);
        let line_h = (v_metrics.ascent - v_metrics.descent + v_metrics.line_gap + line_spacing)
            .ceil()
            .max(1.0);

        let lines = wrap_lines(font, text, scale, rect.w);
        let total_h = lines.len() as f32 * line_h;
        let block_top = match vertical_align {
            VerticalAlign::Top => rect.y,
            VerticalAlign::Centre => rect.y + ((rect.h - total_h) * 0.5).max(0.0),
            VerticalAlign::Bottom => rect.y + (rect.h - total_h).max(0.0),
            // Baseline at rect centre + em/2 (GFx line box above the baseline,
            // centred on the anchor) — see `VerticalAlign::EmBaseline`.
            VerticalAlign::EmBaseline => {
                rect.y + rect.h * 0.5 + size_px * 0.5 - v_metrics.ascent
            }
        };
        let start_baseline = block_top + v_metrics.ascent;

        let clip_min_x = rect.x.floor() as i32;
        let clip_min_y = rect.y.floor() as i32;
        let clip_max_x = (rect.x + rect.w).ceil() as i32;
        let clip_max_y = (rect.y + rect.h).ceil() as i32;

        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for (i, line) in lines.iter().enumerate() {
            let baseline_y = start_baseline + i as f32 * line_h;
            let line_w = line_advance_width(font, line, scale);

            let start_x = match align {
                TextAlign::Left => rect.x,
                TextAlign::Centre => rect.x + ((rect.w - line_w) * 0.5).max(0.0),
                TextAlign::Right => rect.x + (rect.w - line_w).max(0.0),
            };

            let origin: Point<f32> = point(start_x, baseline_y);
            for glyph in font.layout(line, scale, origin) {
                let Some(bb) = glyph.pixel_bounding_box() else {
                    continue;
                };
                let clipped_min_x = bb.min.x.max(clip_min_x);
                let clipped_min_y = bb.min.y.max(clip_min_y);
                let clipped_max_x = bb.max.x.min(clip_max_x);
                let clipped_max_y = bb.max.y.min(clip_max_y);
                if clipped_max_x <= clipped_min_x || clipped_max_y <= clipped_min_y {
                    continue;
                }
                min_x = min_x.min(clipped_min_x);
                min_y = min_y.min(clipped_min_y);
                max_x = max_x.max(clipped_max_x);
                max_y = max_y.max(clipped_max_y);
            }
        }

        // Saturating spans: extreme layout rects saturate glyph coordinates
        // to the i32 limits, and a plain subtraction overflows in debug.
        (min_x < max_x && min_y < max_y).then_some(Rect {
            x: min_x as f32,
            y: min_y as f32,
            w: (max_x as i64 - min_x as i64) as f32,
            h: (max_y as i64 - min_y as i64) as f32,
        })
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn wrap_lines(font: &rusttype::Font<'_>, text: &str, scale: Scale, max_w: f32) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && line_advance_width(font, &candidate, scale) > max_w {
                result.push(current);
                current = word.to_owned();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    result
}

pub(super) fn line_advance_width(font: &rusttype::Font<'_>, text: &str, scale: Scale) -> f32 {
    font.layout(text, scale, point(0.0, 0.0))
        .map(|g| g.unpositioned().h_metrics().advance_width)
        .sum()
}
