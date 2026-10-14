use tiny_skia::{Color, IntSize, Pixmap, Rect as TskRect};

use crate::swf_assets::SwfAssetLibrary;

use super::draw_swf_symbol;

/// Build a minimal SWF with a single 100x100 red rectangle exported as `test_shape`.
fn make_exported_rect_swf() -> Vec<u8> {
    use swf::*;

    let header = Header {
        compression: Compression::None,
        version: 6,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };

    let shape = Shape {
        version: 1,
        id: 1,
        shape_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        edge_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(100.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(100.0),
        },
        flags: ShapeFlag::empty(),
        styles: ShapeStyles {
            fill_styles: vec![FillStyle::Color(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            })],
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
                delta: PointDelta::new(Twips::from_pixels(100.0), Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(100.0)),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::from_pixels(-100.0), Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(-100.0)),
            },
        ],
    };

    let export: Vec<ExportedAsset<'_>> = vec![ExportedAsset {
        id: 1,
        name: SwfStr::from_utf8_str("test_shape"),
    }];

    let tags = [
        Tag::DefineShape(Box::new(shape)),
        Tag::ExportAssets(export),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
    buf
}

#[test]
fn red_rect_shape_rasterises_to_red_pixels() {
    let swf_bytes = make_exported_rect_swf();
    let assets = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary::new");

    let size = IntSize::from_wh(100, 100).expect("int size");
    let mut pixmap = Pixmap::new(size.width(), size.height()).expect("pixmap");

    let dest = TskRect::from_xywh(0.0, 0.0, 100.0, 100.0).expect("dest");
    let white = Color::from_rgba8(255, 255, 255, 255);
    let drew = draw_swf_symbol(&mut pixmap, &assets, "test_shape", dest, white, 1.0);

    assert!(drew, "draw_swf_symbol returned false for test_shape");

    let data = pixmap.data();
    let idx = (50 * 100 + 50) * 4;
    let r = data[idx];
    let g = data[idx + 1];
    let b = data[idx + 2];
    let a = data[idx + 3];

    assert!(r > 200, "expected red centre pixel, got r={r} g={g} b={b} a={a}");
    assert!(g < 50, "expected red centre pixel, got r={r} g={g} b={b} a={a}");
    assert!(b < 50, "expected red centre pixel, got r={r} g={g} b={b} a={a}");
    assert!(a > 200, "expected opaque centre pixel, got a={a}");
}

#[test]
fn missing_symbol_returns_false() {
    let swf_bytes = make_exported_rect_swf();
    let assets = SwfAssetLibrary::new(swf_bytes).expect("SwfAssetLibrary::new");
    let mut pixmap = Pixmap::new(64, 64).expect("pixmap");
    let dest = TskRect::from_xywh(0.0, 0.0, 64.0, 64.0).expect("dest");
    let white = Color::from_rgba8(255, 255, 255, 255);
    let drew = draw_swf_symbol(&mut pixmap, &assets, "no_such_symbol", dest, white, 1.0);
    assert!(!drew);
}

#[test]
fn white_fill_is_tinted_when_tint_is_not_white() {
    use swf::*;

    let header = Header {
        compression: Compression::None,
        version: 6,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(10.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(10.0),
        },
        frame_rate: Fixed8::from_f32(24.0),
        num_frames: 1,
    };
    let shape = Shape {
        version: 1,
        id: 2,
        shape_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(10.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(10.0),
        },
        edge_bounds: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(10.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(10.0),
        },
        flags: ShapeFlag::empty(),
        styles: ShapeStyles {
            fill_styles: vec![FillStyle::Color(Color {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            })],
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
                delta: PointDelta::new(Twips::from_pixels(10.0), Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(10.0)),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::from_pixels(-10.0), Twips::ZERO),
            },
            ShapeRecord::StraightEdge {
                delta: PointDelta::new(Twips::ZERO, Twips::from_pixels(-10.0)),
            },
        ],
    };
    let export: Vec<swf::ExportedAsset<'_>> = vec![swf::ExportedAsset {
        id: 2,
        name: swf::SwfStr::from_utf8_str("white_rect"),
    }];
    let tags = [
        Tag::DefineShape(Box::new(shape)),
        Tag::ExportAssets(export),
        Tag::ShowFrame,
    ];
    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("write_swf");
    let assets = SwfAssetLibrary::new(buf).expect("SwfAssetLibrary");
    let mut pixmap = Pixmap::new(10, 10).expect("pixmap");
    let dest = TskRect::from_xywh(0.0, 0.0, 10.0, 10.0).expect("dest");
    let amber = tiny_skia::Color::from_rgba8(240, 168, 104, 255);
    let drew = draw_swf_symbol(&mut pixmap, &assets, "white_rect", dest, amber, 1.0);
    assert!(drew);

    let data = pixmap.data();
    let idx = (5 * 10 + 5) * 4;
    let r = data[idx];
    let g = data[idx + 1];
    let b = data[idx + 2];
    assert!(r > 180, "expected amber-red, got r={r} g={g} b={b}");
    assert!(
        g > 100 && g < 220,
        "expected amber-green, got r={r} g={g} b={b}"
    );
    assert!(b < 150, "expected amber-blue, got r={r} g={g} b={b}");
}
