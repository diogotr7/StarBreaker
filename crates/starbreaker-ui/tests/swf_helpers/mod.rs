//! Programmatic SWF fixture builders and rendering assertion helpers.
//!
//! Fixtures are generated in-memory from Rust code so they stay readable and
//! maintainable across phases. Two canonical fixtures cover the full set of
//! SWF tag types the Flash-rendering pipeline needs to handle:
//!
//! - `make_labeled_frames_swf` — two named `FrameLabel` frames, each placing a
//!   distinct coloured shape; exported as `"shape_a"` / `"shape_b"`.
//! - `make_state_sprites_swf` — two exported `DefineSprite` symbols
//!   (`"StateA"` / `"StateB"`), a `DefineText` static record, a
//!   `DefineEditText` with HTML `initial_text` and a `@loc` key (mirroring
//!   `TargetStatus.swf`'s `id=22`), and an `ImportAssets` font reference.
//!
//! The `assert_swf_symbol_has_non_empty_coverage` helper renders a named
//! exported symbol via the production `draw_swf_symbol` path and asserts that
//! at least one non-transparent pixel was produced. Later phases reuse it to
//! verify that new rendering paths produce actual pixels.
//!
//! Phase 2 fixtures:
//!
//! - `make_doubly_nested_sprite_swf` — outer sprite → inner sprite → shape (2-level
//!   nesting exercises the recursive display-list renderer).
//! - `make_scaled_nested_sprite_swf` — outer sprite places inner at 2× scale; inner
//!   places a shape.  With correct matrix composition the shape spans 0–40 px; without
//!   it only 0–20 px.
//! - `make_self_referential_sprite_swf` — sprite that places itself; exercises cycle
//!   detection (must not panic or hang).
//!
//! Phase 3 fixtures:
//!
//! - `make_mixed_state_swf` — a document sprite (`"DocSprite"`) that places an
//!   orange always-visible shape plus two state sprites (`"StateA_Content"`,
//!   `"StateB_Content"`) at non-overlapping positions.  Exercises
//!   `draw_swf_symbol_excluding` (suppress one state, verify the other is
//!   still rendered and the always-placed shape remains).

// Shared test-fixture module included by six swf_* integration tests
// (`mod swf_helpers;`). Rust's dead_code lint is per-crate: a helper used by
// one test binary still warns in another's separate compilation. AUDITED
// 2026-06-13 — EVERY fn here is used by >=1 includer, directly or
// transitively (e.g. place_with_ct/color_mult/translate_matrix via
// make_*_color_transform_swf, remove_tag via make_labeled_frames_swf), so
// there is no genuinely dead code to delete; this allow suppresses only the
// per-crate-locality false positive. Re-run the audit if helpers are removed.
#![allow(dead_code)]

use swf::{
    Color, ColorTransform, Compression, ExportedAsset, FillStyle, Fixed8, Fixed16,
    Font, FontFlag, FontLayout, FrameLabel, Glyph, GlyphEntry, Header, Language,
    Matrix, PlaceObject, PlaceObjectAction, PointDelta,
    Rectangle, RemoveObject, Shape, ShapeFlag, ShapeRecord, ShapeStyles, Sprite, StyleChangeData,
    SwfStr, Tag, Text, TextRecord, Twips,
};
use swf::Point;

// ── Shape helpers ─────────────────────────────────────────────────────────────

fn filled_rect_shape(id: swf::CharacterId, w_px: f64, h_px: f64, color: Color) -> Shape {
    let x_max = Twips::from_pixels(w_px);
    let y_max = Twips::from_pixels(h_px);
    let neg_x_max = Twips::from_pixels(-w_px);
    let neg_y_max = Twips::from_pixels(-h_px);
    Shape {
        version: 1,
        id,
        shape_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max,
            y_min: Twips::ZERO,
            y_max,
        },
        edge_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max,
            y_min: Twips::ZERO,
            y_max,
        },
        flags: ShapeFlag::empty(),
        styles: ShapeStyles {
            fill_styles: vec![FillStyle::Color(color)],
            line_styles: vec![],
        },
        shape: vec![
            ShapeRecord::StyleChange(Box::new(StyleChangeData {
                move_to: Some(Point::new(Twips::ZERO, Twips::ZERO)),
                fill_style_0: None,
                fill_style_1: Some(1),
                line_style: None,
                new_styles: None,
            })),
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(x_max, Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, y_max),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(neg_x_max, Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, neg_y_max),
            },
        ],
    }
}

fn place_tag(char_id: swf::CharacterId, depth: swf::Depth) -> Tag<'static> {
    Tag::PlaceObject(Box::new(PlaceObject {
        version: 2,
        action: PlaceObjectAction::Place(char_id),
        depth,
        matrix: Some(Matrix::IDENTITY),
        color_transform: None,
        ratio: None,
        name: None,
        clip_depth: None,
        class_name: None,
        filters: None,
        background_color: None,
        blend_mode: None,
        clip_actions: None,
        has_image: false,
        is_bitmap_cached: None,
        is_visible: None,
        amf_data: None,
    }))
}

fn remove_tag(depth: swf::Depth) -> Tag<'static> {
    Tag::RemoveObject(RemoveObject {
        depth,
        character_id: None,
    })
}

// ── Fixture 1: labeled frames ─────────────────────────────────────────────────

/// Two-frame SWF with named `FrameLabel`s and exported shapes `"shape_a"` /
/// `"shape_b"`.  Exercises: `FrameLabel`, multi-frame main timeline.
///
/// Frame 0 — label `"state_a"`, places shape `id=1` (red 100×100).
/// Frame 1 — label `"state_b"`, removes depth 1, places shape `id=2` (green 100×100).
pub fn make_labeled_frames_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(200.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(200.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 2,
    };

    let red = Color { r: 220, g: 50, b: 50, a: 255 };
    let green = Color { r: 50, g: 200, b: 80, a: 255 };

    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineShape(Box::new(filled_rect_shape(1, 100.0, 100.0, red))),
        Tag::DefineShape(Box::new(filled_rect_shape(2, 100.0, 100.0, green))),
        Tag::ExportAssets(vec![
            ExportedAsset { id: 1, name: SwfStr::from_utf8_str("shape_a") },
            ExportedAsset { id: 2, name: SwfStr::from_utf8_str("shape_b") },
        ]),
        // Frame 0
        Tag::FrameLabel(FrameLabel {
            label: SwfStr::from_utf8_str("state_a"),
            is_anchor: false,
        }),
        place_tag(1, 1),
        Tag::ShowFrame,
        // Frame 1
        Tag::FrameLabel(FrameLabel {
            label: SwfStr::from_utf8_str("state_b"),
            is_anchor: false,
        }),
        remove_tag(1),
        place_tag(2, 1),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_labeled_frames_swf: write_swf failed");
    buf
}

// ── Fixture 2: state sprites ──────────────────────────────────────────────────

/// Single-frame SWF with exported `DefineSprite`s, `DefineText`, HTML
/// `DefineEditText`, and `ImportAssets`.  Exercises all SWF tag types needed
/// by Phases 2–4.
///
/// Characters:
/// - `id=1` shape — red 80×80 (inside "StateA" sprite)
/// - `id=2` shape — blue 80×80 (inside "StateB" sprite)
/// - `id=3` sprite — places id=1 → exported as `"StateA"`
/// - `id=4` sprite — places id=2 → exported as `"StateB"`
/// - `id=5`, `id=6` — imported font symbols from `"fonts_en.gfx"`
/// - `id=7` `DefineText` — static glyph record referencing font `id=5`
/// - `id=8` `DefineEditText` — HTML `initial_text` with `@hud_NoTarget` loc key,
///   mirrors `TargetStatus.swf id=22`
pub fn make_state_sprites_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(200.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(200.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let red = Color { r: 220, g: 50, b: 50, a: 255 };
    let blue = Color { r: 50, g: 80, b: 220, a: 255 };

    // HTML initial_text mirrors the real TargetStatus.swf id=22 initial_text.
    // Raw string uses r##..## so that the inner "#ffffff" doesn't end the literal.
    let html_text = r##"<p align="center"><font face="$Furore" size="6" color="#ffffff" letterSpacing="0.600000">@hud_NoTarget</font></p>"##;

    let tags: Vec<Tag<'_>> = vec![
        // Shapes used by the state sprites
        Tag::DefineShape(Box::new(filled_rect_shape(1, 80.0, 80.0, red))),
        Tag::DefineShape(Box::new(filled_rect_shape(2, 80.0, 80.0, blue))),
        // State sprites
        Tag::DefineSprite(Sprite {
            id: 3,
            num_frames: 1,
            tags: vec![place_tag(1, 1), Tag::ShowFrame],
        }),
        Tag::DefineSprite(Sprite {
            id: 4,
            num_frames: 1,
            tags: vec![place_tag(2, 1), Tag::ShowFrame],
        }),
        // Font imports (mirrors TargetStatus.swf ImportAssets)
        Tag::ImportAssets {
            url: SwfStr::from_utf8_str("fonts_en.gfx"),
            imports: vec![
                ExportedAsset { id: 5, name: SwfStr::from_utf8_str("$Furore") },
                ExportedAsset { id: 6, name: SwfStr::from_utf8_str("$OrbitronLight") },
            ],
        },
        // Static DefineText referencing the imported font
        Tag::DefineText(Box::new(Text {
            id: 7,
            bounds: Rectangle {
                x_min: Twips::ZERO,
                x_max: Twips::from_pixels(200.0),
                y_min: Twips::ZERO,
                y_max: Twips::from_pixels(30.0),
            },
            matrix: Matrix::IDENTITY,
            records: vec![TextRecord {
                font_id: Some(5),
                color: Some(Color { r: 255, g: 255, b: 255, a: 255 }),
                x_offset: Some(Twips::ZERO),
                y_offset: Some(Twips::from_pixels(20.0)),
                height: Some(Twips::from_pixels(12.0)),
                glyphs: vec![GlyphEntry { index: 0, advance: 240 }],
            }],
        })),
        // HTML EditText mirroring TargetStatus.swf id=22
        Tag::DefineEditText(Box::new(
            swf::EditText::new()
                .with_id(8)
                .with_font_id(5, Twips::from_pixels(20.0))
                .with_bounds(Rectangle {
                    x_min: Twips::ZERO,
                    x_max: Twips::from_pixels(400.0),
                    y_min: Twips::ZERO,
                    y_max: Twips::from_pixels(30.0),
                })
                .with_initial_text(Some(SwfStr::from_utf8_str(html_text)))
                .with_is_html(true),
        )),
        // Export the state sprites
        Tag::ExportAssets(vec![
            ExportedAsset { id: 3, name: SwfStr::from_utf8_str("StateA") },
            ExportedAsset { id: 4, name: SwfStr::from_utf8_str("StateB") },
        ]),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_state_sprites_swf: write_swf failed");
    buf
}

// ── Coverage assertion helper (Phase 0.3) ─────────────────────────────────────

/// Render `symbol_name` from `swf_bytes` via the production `draw_swf_symbol`
/// path and assert that at least one non-transparent pixel was produced.
///
/// Panics with a descriptive message on failure.  Reuse this helper in later
/// phases to verify that new rendering paths produce actual pixels.
pub fn assert_swf_symbol_has_non_empty_coverage(
    swf_bytes: &[u8],
    symbol_name: &str,
    dest_w: u32,
    dest_h: u32,
) {
    use starbreaker_ui::swf_assets::SwfAssetLibrary;
    use starbreaker_ui::swf_render::draw_swf_symbol;
    use tiny_skia::{Color, IntSize, Pixmap, Rect as TskRect};

    let assets = SwfAssetLibrary::new(swf_bytes.to_vec())
        .unwrap_or_else(|e| panic!("SwfAssetLibrary::new failed for symbol '{symbol_name}': {e}"));

    let size = IntSize::from_wh(dest_w, dest_h)
        .unwrap_or_else(|| panic!("invalid dest size {dest_w}x{dest_h}"));
    let mut pixmap = Pixmap::new(size.width(), size.height())
        .unwrap_or_else(|| panic!("Pixmap::new({dest_w}, {dest_h}) failed"));

    let dest = TskRect::from_xywh(0.0, 0.0, dest_w as f32, dest_h as f32)
        .unwrap_or_else(|| panic!("TskRect::from_xywh failed for {dest_w}x{dest_h}"));

    let white = Color::from_rgba8(255, 255, 255, 255);
    let drew = draw_swf_symbol(&mut pixmap, &assets, symbol_name, dest, white, 1.0);

    assert!(
        drew,
        "draw_swf_symbol returned false for symbol '{symbol_name}' — export not found or no drawable characters"
    );

    let non_transparent = pixmap
        .data()
        .chunks_exact(4)
        .filter(|px| px[3] > 0)
        .count();

    assert!(
        non_transparent > 0,
        "symbol '{symbol_name}' drew but produced 0 non-transparent pixels in a {dest_w}x{dest_h} pixmap"
    );
}

// ── Phase 2 fixtures: display-list rendering ─────────────────────────────────

fn scale2x_matrix() -> Matrix {
    Matrix {
        a: Fixed16::from_f32(2.0),
        b: Fixed16::from_f32(0.0),
        c: Fixed16::from_f32(0.0),
        d: Fixed16::from_f32(2.0),
        tx: Twips::ZERO,
        ty: Twips::ZERO,
    }
}

/// Two-level nested sprite: outer(id=3) → inner(id=2) → shape(id=1, red 30×30).
///
/// `draw_swf_symbol("DoubleNested")` must recurse into inner sprite to draw
/// the shape.  Stage: 100×100.
pub fn make_doubly_nested_sprite_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let red = Color { r: 220, g: 50, b: 50, a: 255 };
    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineShape(Box::new(filled_rect_shape(1, 30.0, 30.0, red))),
        Tag::DefineSprite(Sprite {
            id: 2,
            num_frames: 1,
            tags: vec![place_tag(1, 1), Tag::ShowFrame],
        }),
        Tag::DefineSprite(Sprite {
            id: 3,
            num_frames: 1,
            tags: vec![place_tag(2, 1), Tag::ShowFrame],
        }),
        Tag::ExportAssets(vec![ExportedAsset { id: 3, name: SwfStr::from_utf8_str("DoubleNested") }]),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_doubly_nested_sprite_swf: write_swf failed");
    buf
}

/// Outer sprite places inner at 2× scale; inner places shape(id=1, red 20×20).
///
/// With correct matrix composition the shape covers (0,0)–(40,40) in the
/// 100×100 stage.  Without composition it would cover only (0,0)–(20,20).
/// Stage: 100×100.
pub fn make_scaled_nested_sprite_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let red = Color { r: 220, g: 50, b: 50, a: 255 };

    // Inner sprite places shape at identity; outer places inner at 2×.
    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineShape(Box::new(filled_rect_shape(1, 20.0, 20.0, red))),
        Tag::DefineSprite(Sprite {
            id: 2,
            num_frames: 1,
            tags: vec![place_tag(1, 1), Tag::ShowFrame],
        }),
        Tag::DefineSprite(Sprite {
            id: 3,
            num_frames: 1,
            tags: vec![
                Tag::PlaceObject(Box::new(PlaceObject {
                    version: 2,
                    action: PlaceObjectAction::Place(2),
                    depth: 1,
                    matrix: Some(scale2x_matrix()),
                    color_transform: None,
                    ratio: None,
                    name: None,
                    clip_depth: None,
                    class_name: None,
                    filters: None,
                    background_color: None,
                    blend_mode: None,
                    clip_actions: None,
                    has_image: false,
                    is_bitmap_cached: None,
                    is_visible: None,
                    amf_data: None,
                })),
                Tag::ShowFrame,
            ],
        }),
        Tag::ExportAssets(vec![ExportedAsset { id: 3, name: SwfStr::from_utf8_str("ScaledOuter") }]),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_scaled_nested_sprite_swf: write_swf failed");
    buf
}

/// A `ColorTransform` with the given channel multipliers (all adds = 0).
fn color_mult(r: f32, g: f32, b: f32, a: f32) -> ColorTransform {
    ColorTransform {
        r_multiply: Fixed8::from_f32(r),
        g_multiply: Fixed8::from_f32(g),
        b_multiply: Fixed8::from_f32(b),
        a_multiply: Fixed8::from_f32(a),
        r_add: 0,
        g_add: 0,
        b_add: 0,
        a_add: 0,
    }
}

/// Place `char_id` at `depth` with the given matrix and colour transform.
fn place_with_ct(
    char_id: swf::CharacterId,
    depth: swf::Depth,
    matrix: Matrix,
    ct: ColorTransform,
) -> Tag<'static> {
    Tag::PlaceObject(Box::new(PlaceObject {
        version: 2,
        action: PlaceObjectAction::Place(char_id),
        depth,
        matrix: Some(matrix),
        color_transform: Some(ct),
        ratio: None,
        name: None,
        clip_depth: None,
        class_name: None,
        filters: None,
        background_color: None,
        blend_mode: None,
        clip_actions: None,
        has_image: false,
        is_bitmap_cached: None,
        is_visible: None,
        amf_data: None,
    }))
}

fn translate_matrix(tx_px: f64, ty_px: f64) -> Matrix {
    Matrix {
        a: Fixed16::from_f32(1.0),
        b: Fixed16::from_f32(0.0),
        c: Fixed16::from_f32(0.0),
        d: Fixed16::from_f32(1.0),
        tx: Twips::from_pixels(tx_px),
        ty: Twips::from_pixels(ty_px),
    }
}

/// Stage places a white 40×40 shape directly at (0,0) with `r_multiply = 0.5`.
///
/// A white shape adopts the (non-white) composed tint, so the rendered red
/// channel reflects how many times the colour transform was applied: a single
/// (correct) application → red ≈ 128; a double application → red ≈ 64.
/// Stage 100×100.
pub fn make_stage_color_transform_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let white = Color { r: 255, g: 255, b: 255, a: 255 };
    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineShape(Box::new(filled_rect_shape(1, 40.0, 40.0, white))),
        place_with_ct(1, 1, Matrix::IDENTITY, color_mult(0.5, 1.0, 1.0, 1.0)),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_stage_color_transform_swf: write_swf failed");
    buf
}

/// Nested colour transforms: stage → spriteA (blue→0) → spriteB (green→0) → white shape.
///
/// The shape (white, 20×20) is placed inside spriteB at (0,0); spriteB is placed
/// inside spriteA at translate (40,40) with a green-zeroing transform; spriteA is
/// placed on the stage with a blue-zeroing transform.  Correct composition folds
/// both transforms (→ red), so the rendered pixel at (50,50) is pure red.  If the
/// intermediate (spriteB) transform is dropped, green survives (→ yellow).
/// Stage 100×100.
pub fn make_nested_color_transform_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let white = Color { r: 255, g: 255, b: 255, a: 255 };
    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineShape(Box::new(filled_rect_shape(1, 20.0, 20.0, white))),
        // spriteB places the shape at its origin (no transform).
        Tag::DefineSprite(Sprite {
            id: 2,
            num_frames: 1,
            tags: vec![place_tag(1, 1), Tag::ShowFrame],
        }),
        // spriteA places spriteB at (40,40) with a green-zeroing transform.
        Tag::DefineSprite(Sprite {
            id: 3,
            num_frames: 1,
            tags: vec![
                place_with_ct(2, 1, translate_matrix(40.0, 40.0), color_mult(1.0, 0.0, 1.0, 1.0)),
                Tag::ShowFrame,
            ],
        }),
        // Stage places spriteA with a blue-zeroing transform.
        place_with_ct(3, 1, Matrix::IDENTITY, color_mult(1.0, 1.0, 0.0, 1.0)),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_nested_color_transform_swf: write_swf failed");
    buf
}

fn place_tag_at(
    char_id: swf::CharacterId,
    depth: swf::Depth,
    tx_px: f64,
    ty_px: f64,
) -> Tag<'static> {
    Tag::PlaceObject(Box::new(PlaceObject {
        version: 2,
        action: PlaceObjectAction::Place(char_id),
        depth,
        matrix: Some(Matrix {
            a: Fixed16::from_f32(1.0),
            b: Fixed16::from_f32(0.0),
            c: Fixed16::from_f32(0.0),
            d: Fixed16::from_f32(1.0),
            tx: Twips::from_pixels(tx_px),
            ty: Twips::from_pixels(ty_px),
        }),
        color_transform: None,
        ratio: None,
        name: None,
        clip_depth: None,
        class_name: None,
        filters: None,
        background_color: None,
        blend_mode: None,
        clip_actions: None,
        has_image: false,
        is_bitmap_cached: None,
        is_visible: None,
        amf_data: None,
    }))
}

/// A sprite that places itself — used to verify cycle detection does not hang.
///
/// Stage: 100×100.  The sprite has no drawable shape, just a self-reference.
pub fn make_self_referential_sprite_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    // Sprite id=1 places itself at depth 1.
    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineSprite(Sprite {
            id: 1,
            num_frames: 1,
            tags: vec![place_tag(1, 1), Tag::ShowFrame],
        }),
        Tag::ExportAssets(vec![ExportedAsset { id: 1, name: SwfStr::from_utf8_str("SelfRef") }]),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_self_referential_sprite_swf: write_swf failed");
    buf
}

// ── Phase 3 fixture: mixed always-placed + state sprites ──────────────────────

/// Document sprite (`"DocSprite"`) that places three non-overlapping shapes:
///
/// - `id=1` orange 30×30 at (0,0)   — depth 1, always present
/// - `id=4` sprite `"StateA_Content"` at (35,0)  — depth 2, contains red 30×30
/// - `id=5` sprite `"StateB_Content"` at (0,35)  — depth 3, contains blue 30×30
///
/// In a 100×100 dest (stage is also 100×100, sx=sy=1):
/// - Orange → pixels (0–30, 0–30), check (10,10)
/// - StateA red → pixels (35–65, 0–30), check (45,10)
/// - StateB blue → pixels (0–30, 35–65), check (10,45)
///
/// Suppressing `StateB_Content` must zero out (10,45) while leaving (10,10)
/// and (45,10) non-transparent.
pub fn make_mixed_state_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let orange = Color { r: 220, g: 120, b: 0, a: 255 };
    let red = Color { r: 220, g: 50, b: 50, a: 255 };
    let blue = Color { r: 50, g: 80, b: 220, a: 255 };

    let tags: Vec<Tag<'_>> = vec![
        // Leaf shapes
        Tag::DefineShape(Box::new(filled_rect_shape(1, 30.0, 30.0, orange))),
        Tag::DefineShape(Box::new(filled_rect_shape(2, 30.0, 30.0, red))),
        Tag::DefineShape(Box::new(filled_rect_shape(3, 30.0, 30.0, blue))),
        // State sprites — each wraps one leaf shape at identity
        Tag::DefineSprite(Sprite {
            id: 4,
            num_frames: 1,
            tags: vec![place_tag(2, 1), Tag::ShowFrame],
        }),
        Tag::DefineSprite(Sprite {
            id: 5,
            num_frames: 1,
            tags: vec![place_tag(3, 1), Tag::ShowFrame],
        }),
        // Document sprite: orange always at (0,0), StateA at (35,0), StateB at (0,35)
        Tag::DefineSprite(Sprite {
            id: 6,
            num_frames: 1,
            tags: vec![
                place_tag_at(1, 1, 0.0, 0.0),
                place_tag_at(4, 2, 35.0, 0.0),
                place_tag_at(5, 3, 0.0, 35.0),
                Tag::ShowFrame,
            ],
        }),
        Tag::ExportAssets(vec![
            ExportedAsset { id: 4, name: SwfStr::from_utf8_str("StateA_Content") },
            ExportedAsset { id: 5, name: SwfStr::from_utf8_str("StateB_Content") },
            ExportedAsset { id: 6, name: SwfStr::from_utf8_str("DocSprite") },
        ]),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_mixed_state_swf: write_swf failed");
    buf
}

// ── Phase 4 fixture: EditText with inline font ────────────────────────────────

/// SWF with an inline font (id=1, name="TestFont") and a plain-text
/// `DefineEditText` (id=2, text="A", font_id=1) placed on the stage timeline.
///
/// Stage: 100×100.  EditText bounds: (10,30)–(90,70).
/// Font: ascent=800, descent=200 (glyph units), one glyph 'A' as a
/// 300×300-unit filled rectangle.
///
/// Used by Phase 4 test `draw_stage_with_edit_text_produces_pixels` to verify
/// that EditText characters on the stage timeline are rasterised into pixels.
pub fn make_edit_text_with_font_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    // 'A' glyph: rectangle 300×300 in glyph units, top-left at (0, -300).
    // No fill styles needed — swf_glyph_to_path ignores them; colour comes
    // from draw_swf_font's `colour` parameter.
    let a_shape_records = vec![
        ShapeRecord::StyleChange(Box::new(StyleChangeData {
            move_to: Some(Point::new(Twips::new(0), Twips::new(-300))),
            fill_style_0: None,
            fill_style_1: None,
            line_style: None,
            new_styles: None,
        })),
        ShapeRecord::StraightEdge { delta: PointDelta::new(Twips::new(300), Twips::new(0)) },
        ShapeRecord::StraightEdge { delta: PointDelta::new(Twips::new(0), Twips::new(300)) },
        ShapeRecord::StraightEdge { delta: PointDelta::new(Twips::new(-300), Twips::new(0)) },
        ShapeRecord::StraightEdge { delta: PointDelta::new(Twips::new(0), Twips::new(-300)) },
    ];

    let font = Font {
        version: 2,
        id: 1,
        name: SwfStr::from_utf8_str("TestFont"),
        language: Language::Latin,
        layout: Some(FontLayout {
            ascent: 800,
            descent: 200,
            leading: 0,
            kerning: vec![],
        }),
        glyphs: vec![Glyph {
            shape_records: a_shape_records,
            code: 'A' as u16,
            advance: 360,
            // Bounds required by swf::write_swf: x=[0,300], y=[-300,0] in raw Twips.
            bounds: Some(Rectangle {
                x_min: Twips::new(0),
                x_max: Twips::new(300),
                y_min: Twips::new(-300),
                y_max: Twips::new(0),
            }),
        }],
        flags: FontFlag::empty(),
    };

    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineFont2(Box::new(font)),
        Tag::DefineEditText(Box::new(
            swf::EditText::new()
                .with_id(2)
                .with_font_id(1, Twips::from_pixels(20.0))
                .with_bounds(Rectangle {
                    x_min: Twips::from_pixels(10.0),
                    x_max: Twips::from_pixels(90.0),
                    y_min: Twips::from_pixels(30.0),
                    y_max: Twips::from_pixels(70.0),
                })
                .with_initial_text(Some(SwfStr::from_utf8_str("A")))
                .with_is_html(false),
        )),
        // Place the EditText on the stage timeline at depth 1.
        place_tag(2, 1),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf)
        .expect("make_edit_text_with_font_swf: write_swf failed");
    buf
}

// ── Phase 5 fixture: sample-data state-selection ─────────────────────────────

/// Phase 5 sample-data state-selection fixture.
///
/// Three exported sprites exercising the `compute_sample_data_export_ids` rule:
///
/// - `"StaticState"` (id=4): places shape id=1 + EditText id=2 (`@hud_NoTarget`)
///   → has a loc-key EditText → must NOT be suppressed
/// - `"SampleState"` (id=5): places EditText id=3 ("Ship Name/Label")
///   → all EditText is sample data (no `@`) → must be suppressed
/// - `"AlwaysVisible"` (id=6): places shape id=1 only → no EditText → must NOT be suppressed
pub fn make_sample_data_swf() -> Vec<u8> {
    let header = Header {
        compression: Compression::None,
        version: 8,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let red = Color { r: 220, g: 50, b: 50, a: 255 };
    let loc_html = r##"<p align="center"><font face="$Furore" size="6" color="#ffffff" letterSpacing="0.0">@hud_NoTarget</font></p>"##;

    let tags: Vec<Tag<'_>> = vec![
        Tag::DefineShape(Box::new(filled_rect_shape(1, 50.0, 50.0, red))),
        Tag::DefineEditText(Box::new(
            swf::EditText::new()
                .with_id(2)
                .with_bounds(Rectangle {
                    x_min: Twips::ZERO,
                    x_max: Twips::from_pixels(100.0),
                    y_min: Twips::from_pixels(50.0),
                    y_max: Twips::from_pixels(70.0),
                })
                .with_initial_text(Some(SwfStr::from_utf8_str(loc_html)))
                .with_is_html(true),
        )),
        Tag::DefineEditText(Box::new(
            swf::EditText::new()
                .with_id(3)
                .with_bounds(Rectangle {
                    x_min: Twips::ZERO,
                    x_max: Twips::from_pixels(100.0),
                    y_min: Twips::ZERO,
                    y_max: Twips::from_pixels(30.0),
                })
                .with_initial_text(Some(SwfStr::from_utf8_str("Ship Name/Label")))
                .with_is_html(false),
        )),
        Tag::DefineSprite(Sprite {
            id: 4,
            num_frames: 1,
            tags: vec![place_tag(1, 1), place_tag(2, 2), Tag::ShowFrame],
        }),
        Tag::DefineSprite(Sprite {
            id: 5,
            num_frames: 1,
            tags: vec![place_tag(3, 1), Tag::ShowFrame],
        }),
        Tag::DefineSprite(Sprite {
            id: 6,
            num_frames: 1,
            tags: vec![place_tag(1, 1), Tag::ShowFrame],
        }),
        Tag::ExportAssets(vec![
            ExportedAsset { id: 4, name: SwfStr::from_utf8_str("StaticState") },
            ExportedAsset { id: 5, name: SwfStr::from_utf8_str("SampleState") },
            ExportedAsset { id: 6, name: SwfStr::from_utf8_str("AlwaysVisible") },
        ]),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("make_sample_data_swf: write_swf failed");
    buf
}
