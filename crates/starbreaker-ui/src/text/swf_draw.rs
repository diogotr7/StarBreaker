use image::RgbaImage;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::bb_layout::Rect;
use crate::swf_assets::{FontGlyph, FontGlyphSet, SwfEditTextMetrics};

use super::{SWF_TEXT_WIDTH_CALIBRATION, TextAlign, TextRenderer, VerticalAlign};

impl TextRenderer {
    #[allow(clippy::too_many_arguments)]
    pub fn draw_swf_font(
        &self,
        img: &mut RgbaImage,
        text: &str,
        rect: Rect,
        swf_font: &FontGlyphSet,
        edit_text_metrics: Option<&SwfEditTextMetrics>,
        size_px: f32,
        colour: [u8; 4],
        align: TextAlign,
        vertical_align: VerticalAlign,
        line_spacing_px: Option<f32>,
        letter_spacing_px: f32,
    ) -> bool {
        if text.is_empty() || rect.w < 1.0 || rect.h < 1.0 || size_px < 1.0 {
            return false;
        }

        let ascent = swf_font.ascent.map(|v| v as f32).unwrap_or(820.0);
        let descent = swf_font.descent.map(|v| v as f32).unwrap_or(-204.0);
        let line_gap = swf_font.leading.map(|v| v as f32).unwrap_or(0.0);
        let units_per_em = (ascent.abs() + descent.abs()).max(1.0);
        if units_per_em <= 0.0 {
            return false;
        }

        let nominal_line_h = (((units_per_em + line_gap) / units_per_em) * size_px).max(1.0);
        let lines = swf_wrap_lines(swf_font, text, units_per_em, size_px, rect.w, letter_spacing_px);

        struct GlyphRun {
            path: tiny_skia::Path,
            x_px: f32,
        }

        struct LineLayout {
            runs: Vec<GlyphRun>,
            min_x_px: f32,
            max_x_px: f32,
            min_y_units: f32,
            max_y_units: f32,
        }

        let scale = size_px / units_per_em;
        let scale_x = scale * SWF_TEXT_WIDTH_CALIBRATION;
        let mut layouts = Vec::new();
        for line in &lines {
            let mut pen_x = 0.0f32;
            let mut runs = Vec::new();
            let mut min_x_px = f32::INFINITY;
            let mut max_x_px = f32::NEG_INFINITY;
            let mut min_y_units = f32::INFINITY;
            let mut max_y_units = f32::NEG_INFINITY;
            for ch in line.chars() {
                // Word-space advance = the font's own space-glyph advance (data-backed;
                // see swf_space_advance_px). Re-derived against the reference.
                if ch == ' ' {
                    pen_x += swf_space_advance_px(swf_font, units_per_em, size_px) + letter_spacing_px;
                    continue;
                }
                let Some((glyph_idx, glyph)) = swf_lookup_glyph(swf_font, ch) else {
                    pen_x += (size_px * 0.5).max(1.0) + letter_spacing_px;
                    continue;
                };

                let mut pb = PathBuilder::new();
                if swf_glyph_to_path(&glyph.shape_records, &mut pb)
                    && let Some(path) = pb.finish()
                {
                    let bounds = path.bounds();
                    min_x_px = min_x_px.min(pen_x + bounds.left() * scale_x);
                    max_x_px = max_x_px.max(pen_x + bounds.right() * scale_x);
                    min_y_units = min_y_units.min(bounds.top());
                    max_y_units = max_y_units.max(bounds.bottom());
                    runs.push(GlyphRun { path, x_px: pen_x });
                }

                let adv = swf_glyph_advance_px(swf_font, glyph_idx, units_per_em, size_px);
                // GFx letterSpacing: a constant per-character tracking added to
                // every advance (brand LetterSpacing × host-stage scale).
                pen_x += adv.max(1.0) + letter_spacing_px;
            }

            if !min_y_units.is_finite() || !max_y_units.is_finite() || max_y_units <= min_y_units {
                min_y_units = descent;
                max_y_units = ascent;
            }
            if !min_x_px.is_finite() || !max_x_px.is_finite() || max_x_px <= min_x_px {
                min_x_px = 0.0;
                max_x_px = pen_x;
            }

            layouts.push(LineLayout {
                runs,
                min_x_px,
                max_x_px,
                min_y_units,
                max_y_units,
            });
        }

        let measured_line_h = layouts
            .iter()
            .map(|layout| (layout.max_y_units - layout.min_y_units).abs() * scale)
            .fold(0.0f32, f32::max);

        // Font-size audit harness (diagnostic only; no effect unless SB_UI_FONT_DUMP
        // is set). Emits the final rendered text height in px — which folds in every
        // render-side scaling factor (calibration + em choice + glyph metrics) — keyed
        // to the source canvas/node. Compare with `font_size_check.py` against
        // `font_size_baseline.tsv`. Format:
        //   FONTDUMP \t canvas \t node \t font \t size_px \t visible_px \t em \t text
        if std::env::var_os("SB_UI_FONT_DUMP").is_some() {
            let cap_units = layouts
                .iter()
                .map(|layout| (layout.max_y_units - layout.min_y_units).abs())
                .fold(0.0f32, f32::max);
            let visible_px = cap_units * scale;
            // Widest rendered line (px) — the measure the user compares against the
            // reference text-block widths. SB_UI_FONT_DUMP-only diagnostic.
            let width_px = layouts
                .iter()
                .map(|layout| (layout.max_x_px - layout.min_x_px).max(0.0))
                .fold(0.0f32, f32::max);
            let (canvas, node) = super::FONT_DUMP_CTX.with(|c| c.borrow().clone());
            eprintln!(
                "FONTDUMP\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.1}\t{:.2}\t{}",
                canvas,
                node,
                swf_font.name,
                size_px,
                visible_px,
                units_per_em,
                width_px,
                text.chars().take(28).collect::<String>().replace('\t', " "),
            );
        }
        // `line_step` is the full em line box (≈ size) — the engine's single-spacing line
        // advance is exactly that box, with NO default extra leading. The brand
        // `LineSpacing` modifier (often negative) tightens from there. Verified against the
        // Clipper medical card descriptions: line-to-line advance == font size (a 25px H6
        // paragraph steps 25px, not the 30px an added 0.2×size interline produced). See
        // docs/clipper-medical-width-targets.md (RC2).
        let line_step = nominal_line_h.max(measured_line_h.max(1.0));
        let interline_px = line_spacing_px
            .filter(|value| value.is_finite())
            .unwrap_or(0.0);
        let baseline_step = (line_step + interline_px).max(1.0);
        let mut min_y_px = f32::INFINITY;
        let mut max_y_px = f32::NEG_INFINITY;
        for (line_index, layout) in layouts.iter().enumerate() {
            let line_offset = line_index as f32 * baseline_step;
            min_y_px = min_y_px.min(layout.min_y_units * scale + line_offset);
            max_y_px = max_y_px.max(layout.max_y_units * scale + line_offset);
        }
        if !min_y_px.is_finite() || !max_y_px.is_finite() || max_y_px <= min_y_px {
            min_y_px = descent * scale;
            max_y_px = ascent * scale;
        }
        let total_h = (max_y_px - min_y_px).max(1.0);
        let source_top_overscan = edit_text_metrics
            .copied()
            .map(|metrics| metrics.top_overscan_px(size_px))
            .unwrap_or(0.0);
        let block_top = match vertical_align {
            VerticalAlign::Top => rect.y + source_top_overscan,
            VerticalAlign::Centre => rect.y + ((rect.h - total_h) * 0.5).max(0.0),
            VerticalAlign::Bottom => rect.y + (rect.h - total_h).max(0.0),
            // Baseline at rect centre + em/2 (the GFx line box is the full em
            // above the baseline, centred on the anchor). `block_top` is only
            // consumed as `start_baseline = block_top - min_y_px`.
            VerticalAlign::EmBaseline => rect.y + rect.h * 0.5 + size_px * 0.5 + min_y_px,
        };
        let start_baseline = block_top - min_y_px;

        let mut pixmap = match Pixmap::new(img.width(), img.height()) {
            Some(pm) => pm,
            None => return false,
        };

        for (line_index, layout) in layouts.iter().enumerate() {
            let line_drawn_w = (layout.max_x_px - layout.min_x_px).max(0.0);
            let start_x = match align {
                TextAlign::Left => rect.x - layout.min_x_px,
                TextAlign::Centre => {
                    rect.x + ((rect.w - line_drawn_w) * 0.5).max(0.0) - layout.min_x_px
                }
                TextAlign::Right => rect.x + (rect.w - line_drawn_w).max(0.0) - layout.min_x_px,
            };
            let baseline_y = start_baseline + line_index as f32 * baseline_step;

            for run in &layout.runs {
                let transform = Transform::from_row(
                    scale_x,
                    0.0,
                    0.0,
                    scale,
                    start_x + run.x_px,
                    baseline_y,
                );

                let mut paint = Paint::default();
                paint.set_color_rgba8(colour[0], colour[1], colour[2], colour[3]);
                pixmap.fill_path(&run.path, &paint, FillRule::Winding, transform, None);
            }
        }

        blend_pixmap_onto_image(&pixmap, img, rect);
        true
    }

    pub fn measure_swf_advance_width(
        &self,
        text: &str,
        swf_font: &FontGlyphSet,
        size_px: f32,
        letter_spacing_px: f32,
    ) -> Option<f32> {
        if text.is_empty() || size_px < 1.0 {
            return None;
        }
        let ascent = swf_font.ascent.map(|v| v as f32).unwrap_or(820.0);
        let descent = swf_font.descent.map(|v| v as f32).unwrap_or(-204.0);
        let units_per_em = (ascent.abs() + descent.abs()).max(1.0);
        (units_per_em > 0.0)
            .then(|| swf_line_width(text, swf_font, units_per_em, size_px, letter_spacing_px))
    }

}

fn swf_lookup_glyph(swf_font: &FontGlyphSet, ch: char) -> Option<(usize, &FontGlyph)> {
    swf_font
        .glyphs
        .iter()
        .enumerate()
        .find(|(_, g)| g.code == Some(ch as u16))
}

fn swf_glyph_advance_px(
    swf_font: &FontGlyphSet,
    glyph_idx: usize,
    units_per_em: f32,
    size_px: f32,
) -> f32 {
    let units = swf_font
        .glyphs
        .get(glyph_idx)
        .and_then(|g| g.advance)
        .map(|v| v as f32)
        .unwrap_or(320.0);
    (units / units_per_em) * size_px * SWF_TEXT_WIDTH_CALIBRATION
}

/// Word-space advance for SWF text. Data-backed: the font's own space-glyph advance
/// (re-derived against the in-game reference — the Clipper medical header space matches
/// the font advance, ~0.22×em, not the previously-tuned 0.33×em which rendered ~50% too
/// wide). Falls back to 0.33×em only when the font carries no space glyph/advance.
fn swf_space_advance_px(swf_font: &FontGlyphSet, units_per_em: f32, size_px: f32) -> f32 {
    swf_lookup_glyph(swf_font, ' ')
        .map(|(idx, _)| swf_glyph_advance_px(swf_font, idx, units_per_em, size_px))
        .filter(|advance| *advance > 0.0)
        .unwrap_or(size_px * 0.33)
        .max(1.0)
}

fn swf_line_width(
    text: &str,
    swf_font: &FontGlyphSet,
    units_per_em: f32,
    size_px: f32,
    letter_spacing_px: f32,
) -> f32 {
    text.chars().fold(0.0, |acc, ch| {
        let advance = if ch == ' ' {
            swf_space_advance_px(swf_font, units_per_em, size_px)
        } else if let Some((idx, _)) = swf_lookup_glyph(swf_font, ch) {
            swf_glyph_advance_px(swf_font, idx, units_per_em, size_px).max(1.0)
        } else {
            (size_px * 0.5).max(1.0)
        };
        acc + advance + letter_spacing_px
    })
}

fn swf_wrap_lines(
    swf_font: &FontGlyphSet,
    text: &str,
    units_per_em: f32,
    size_px: f32,
    max_w: f32,
    letter_spacing_px: f32,
) -> Vec<String> {
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
            if !current.is_empty()
                && swf_line_width(&candidate, swf_font, units_per_em, size_px, letter_spacing_px) > max_w
            {
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

#[cfg(test)]
mod blend_tests {
    use super::blend_pixmap_onto_image;
    use crate::bb_layout::Rect;
    use image::RgbaImage;
    use tiny_skia::Pixmap;

    // A premultiplied 50% white source over opaque black must composite in
    // LINEAR light to ~188, NOT the sRGB midpoint 128. `from_rgba8` takes a
    // STRAIGHT alpha, so 50% white is `from_rgba8(255,255,255,128)` which
    // tiny-skia stores as premul (128,128,128,128) — the real premul-50%-white
    // input the blend contract expects (asserted below before blending).
    #[test]
    fn blend_pixmap_onto_image_composites_in_linear() {
        let mut img = RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let mut src = Pixmap::new(2, 2).unwrap();
        src.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 128));
        let stored = src.pixel(0, 0).unwrap();
        assert_eq!(
            (stored.red(), stored.green(), stored.blue(), stored.alpha()),
            (128, 128, 128, 128),
            "from_rgba8(255,255,255,128) must store as premul 50% white"
        );
        let clip = Rect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 };
        blend_pixmap_onto_image(&src, &mut img, clip);
        let px = img.get_pixel(0, 0);
        assert!(
            (px[0] as i32 - 188).abs() <= 3,
            "linear swf glyph blend expected ~188, got {}",
            px[0]
        );
        assert!((px[0] as i32 - 128).abs() > 20, "must not be sRGB 128");
    }
}

fn swf_glyph_to_path(records: &[swf::ShapeRecord], pb: &mut PathBuilder) -> bool {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut started = false;

    for rec in records {
        match rec {
            swf::ShapeRecord::StyleChange(sc) => {
                if let Some(move_to) = sc.move_to {
                    x = move_to.x.get() as f32;
                    y = move_to.y.get() as f32;
                    pb.move_to(x, y);
                    started = true;
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                if !started {
                    pb.move_to(x, y);
                    started = true;
                }
                x += delta.dx.get() as f32;
                y += delta.dy.get() as f32;
                pb.line_to(x, y);
            }
            swf::ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                if !started {
                    pb.move_to(x, y);
                    started = true;
                }
                let cx = x + control_delta.dx.get() as f32;
                let cy = y + control_delta.dy.get() as f32;
                x = cx + anchor_delta.dx.get() as f32;
                y = cy + anchor_delta.dy.get() as f32;
                pb.quad_to(cx, cy, x, y);
            }
        }
    }

    started
}

fn blend_pixmap_onto_image(pixmap: &Pixmap, img: &mut RgbaImage, clip: Rect) {
    let img_w = img.width() as i32;
    let img_h = img.height() as i32;
    let clip_min_x = clip.x.floor().max(0.0) as i32;
    let clip_min_y = clip.y.floor().max(0.0) as i32;
    let clip_max_x = (clip.x + clip.w).ceil().min(img.width() as f32) as i32;
    let clip_max_y = (clip.y + clip.h).ceil().min(img.height() as f32) as i32;

    for py in clip_min_y..clip_max_y {
        for px in clip_min_x..clip_max_x {
            if px < 0 || py < 0 || px >= img_w || py >= img_h {
                continue;
            }
            let Some(src) = pixmap.pixel(px as u32, py as u32) else {
                continue;
            };
            if src.alpha() == 0 {
                continue;
            }
            // `src` is premultiplied (tiny-skia stores premul); its .red()/.green()/
            // .blue() are the premultiplied channels — exactly blend_premul_linear's
            // input contract. Compositing in LINEAR light matches the engine.
            let dst = img.get_pixel_mut(px as u32, py as u32);
            crate::colour::blend_premul_linear(
                &mut dst.0,
                [src.red(), src.green(), src.blue(), src.alpha()],
            );
        }
    }
}
