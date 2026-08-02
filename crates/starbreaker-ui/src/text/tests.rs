use super::*;
use image::{Rgba, RgbaImage};
use swf::{Point, PointDelta, ShapeRecord, StyleChangeData, Twips};

use crate::bb_layout::Rect;
use crate::swf_assets::FontGlyphSet;

fn make_img(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]))
}

fn test_swf_font() -> FontGlyphSet {
    let mut font = FontGlyphSet {
        id: 1,
        name: "Test".to_string(),
        is_bold: false,
        is_italic: false,
        ascent: Some(800),
        descent: Some(200),
        leading: Some(0),
        glyphs: vec![crate::swf_assets::FontGlyph {
            code: Some('A' as u16),
            advance: Some(360),
            shape_records: vec![
                ShapeRecord::StyleChange(Box::new(StyleChangeData {
                    move_to: Some(Point::new(Twips::new(0), Twips::new(-300))),
                    fill_style_0: None,
                    fill_style_1: None,
                    line_style: None,
                    new_styles: None,
                })),
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::new(300), Twips::new(0)),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::new(0), Twips::new(300)),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::new(-300), Twips::new(0)),
                },
                ShapeRecord::StraightEdge {
                    delta: PointDelta::new(Twips::new(0), Twips::new(-300)),
                },
            ],
        }],
        glyph_index: Default::default(),
    };
    font.glyph_index = crate::swf_assets::build_glyph_index(&font.glyphs);
    font
}

// The index replaces a linear `find`, which returned the FIRST glyph with a
// given code — duplicates must still resolve to the earliest entry.
#[test]
fn glyph_index_is_first_wins() {
    let mut font = test_swf_font();
    let mut dup = font.glyphs[0].clone();
    dup.advance = Some(999);
    font.glyphs.push(dup);
    font.glyph_index = crate::swf_assets::build_glyph_index(&font.glyphs);

    assert_eq!(font.glyph_index.get(&('A' as u16)), Some(&0));
    let scanned = font
        .glyphs
        .iter()
        .position(|g| g.code == Some('A' as u16))
        .unwrap();
    assert_eq!(scanned, 0);
}

fn first_nonblack_y(img: &RgbaImage) -> Option<u32> {
    img.enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[0] > 0 || pixel[1] > 0 || pixel[2] > 0)
        .map(|(_, y, _)| y)
        .min()
}

#[test]
fn measure_nonempty() {
    let r = TextRenderer::new();
    let (w, h) = r.measure("HELLO", FontKind::Sans, 14.0);
    assert!(w > 0.0, "expected positive width, got {w}");
    assert!(h > 0.0, "expected positive height, got {h}");
}

#[test]
fn draw_leaves_nonblack_pixels() {
    let r = TextRenderer::new();
    let mut img = make_img(128, 32);
    let rect = Rect {
        x: 2.0,
        y: 2.0,
        w: 120.0,
        h: 28.0,
    };
    r.draw(
        &mut img,
        "HI",
        rect,
        FontKind::Sans,
        14.0,
        [255, 255, 255, 255],
        TextAlign::Left,
        VerticalAlign::Centre,
        None,
    );
    let changed = img.pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
    assert!(changed, "no pixels changed after draw");
}

#[test]
fn draw_empty_string_does_nothing() {
    let r = TextRenderer::new();
    let mut img = make_img(64, 16);
    let before: Vec<_> = img.pixels().copied().collect();
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 64.0,
        h: 16.0,
    };
    r.draw(
        &mut img,
        "",
        rect,
        FontKind::Sans,
        12.0,
        [255, 0, 0, 255],
        TextAlign::Left,
        VerticalAlign::Centre,
        None,
    );
    let after: Vec<_> = img.pixels().copied().collect();
    assert_eq!(before, after, "draw of empty string mutated the image");
}

#[test]
fn swf_top_alignment_uses_source_edit_text_overscan() {
    let r = TextRenderer::new();
    let font = test_swf_font();
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 80.0,
        h: 40.0,
    };

    let mut without_metrics = make_img(96, 48);
    assert!(r.draw_swf_font(
        &mut without_metrics,
        "A",
        rect,
        &font,
        None,
        20.0,
        [255, 255, 255, 255],
        TextAlign::Left,
        VerticalAlign::Top,
        None,
        0.0,
    ));

    let metrics = crate::swf_assets::SwfEditTextMetrics {
        height_px: 20.0,
        bounds_y_min_px: -2.0,
        bounds_y_max_px: 23.4,
    };
    let mut with_metrics = make_img(96, 48);
    assert!(r.draw_swf_font(
        &mut with_metrics,
        "A",
        rect,
        &font,
        Some(&metrics),
        20.0,
        [255, 255, 255, 255],
        TextAlign::Left,
        VerticalAlign::Top,
        None,
        0.0,
    ));

    let uncorrected_top = first_nonblack_y(&without_metrics).expect("uncorrected pixels");
    let corrected_top = first_nonblack_y(&with_metrics).expect("corrected pixels");
    assert!(
        corrected_top >= uncorrected_top + 5,
        "expected source edit-text overscan to lower top-aligned SWF text ({uncorrected_top} -> {corrected_top})"
    );
}

#[test]
fn measure_drawn_bounds_uses_source_line_spacing() {
    let r = TextRenderer::new();
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 200.0,
        h: 200.0,
    };
    let default_bounds = r
        .measure_drawn_bounds(
            "A\nA",
            rect,
            FontKind::Sans,
            24.0,
            TextAlign::Left,
            VerticalAlign::Top,
            None,
        )
        .expect("default bounds");
    let tightened_bounds = r
        .measure_drawn_bounds(
            "A\nA",
            rect,
            FontKind::Sans,
            24.0,
            TextAlign::Left,
            VerticalAlign::Top,
            Some(-8.0),
        )
        .expect("tightened bounds");

    assert!(
        tightened_bounds.h < default_bounds.h,
        "expected negative source line spacing to tighten multiline bounds ({:?} vs {:?})",
        tightened_bounds,
        default_bounds
    );
}

#[test]
fn measure_mono_and_sans_return_positive_widths() {
    let r = TextRenderer::new();
    let (ws, _) = r.measure("TEST", FontKind::Sans, 12.0);
    let (wm, _) = r.measure("TEST", FontKind::Mono, 12.0);
    assert!(ws > 0.0 && wm > 0.0);
}

/// `VerticalAlign::EmBaseline` places the BASELINE at rect centre + em/2 —
/// the GFx line box (the full em above the baseline) centred on the anchor.
/// Verified against the Clipper target screen's centre-anchored NO TARGET
/// heading: in-game the cap band sits ~0.2 em (one descent) below the
/// anchor, not cap-centred on it.
#[test]
fn swf_em_baseline_alignment_drops_baseline_to_centre_plus_half_em() {
    let r = TextRenderer::new();
    let font = test_swf_font();
    let rect = Rect { x: 0.0, y: 0.0, w: 96.0, h: 48.0 };
    let size = 20.0; // em 20px; glyph cap band = 6px (300 of 1000 units)

    let mut img = make_img(96, 48);
    assert!(r.draw_swf_font(
        &mut img,
        "A",
        rect,
        &font,
        None,
        size,
        [255, 255, 255, 255],
        TextAlign::Left,
        VerticalAlign::EmBaseline,
        None,
        0.0,
    ));

    // baseline = 24 + 10 = 34; cap band = 28..34.
    let top = first_nonblack_y(&img).expect("glyph pixels") as f32;
    assert!(
        (top - 28.0).abs() <= 1.5,
        "cap top should be ≈28 (baseline 34 − cap 6), got {top}"
    );
}
