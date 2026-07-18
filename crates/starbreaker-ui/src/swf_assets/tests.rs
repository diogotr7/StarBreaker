use super::*;

/// Build a minimal in-memory SWF using the `swf` crate's writer.
fn make_minimal_swf() -> Vec<u8> {
    use swf::*;

    let header = Header {
        compression: Compression::None,
        version: 6,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(200.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(200.0),
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
                r: 0,
                g: 0,
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

    let tags = [
        Tag::SetBackgroundColor(Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
        Tag::DefineShape(shape),
        Tag::ShowFrame,
    ];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
    buf
}

fn make_swf_with_action() -> Vec<u8> {
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

    let action_bytes: &[u8] = &[0x07, 0x00];
    let tags = [Tag::DoAction(action_bytes), Tag::ShowFrame];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
    buf
}

fn make_swf_with_imported_font_edit_text() -> Vec<u8> {
    use swf::*;

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

    let imported = Tag::ImportAssets {
        url: "fonts.gfx".into(),
        imports: vec![ExportedAsset {
            id: 18,
            name: "$Text1Thin".into(),
        }],
    };
    let edit = Tag::DefineEditText(Box::new(
        EditText::new()
            .with_id(19)
            .with_font_id(18, Twips::from_pixels(20.0))
            .with_bounds(Rectangle {
                x_min: Twips::from_pixels(-2.0),
                x_max: Twips::from_pixels(536.0),
                y_min: Twips::from_pixels(-2.0),
                y_max: Twips::from_pixels(23.4),
            }),
    ));
    let exported_font_sheet_edit = Tag::DefineEditText(Box::new(
        EditText::new()
            .with_id(39)
            .with_font_id(20, Twips::from_pixels(20.0))
            .with_bounds(Rectangle {
                x_min: Twips::from_pixels(-2.0),
                x_max: Twips::from_pixels(536.0),
                y_min: Twips::from_pixels(-2.0),
                y_max: Twips::from_pixels(75.8),
            }),
    ));
    let exported = Tag::ExportAssets(vec![ExportedAsset {
        id: 20,
        name: "$Text1Thin".into(),
    }]);
    let tags = [imported, edit, exported_font_sheet_edit, exported, Tag::ShowFrame];

    let mut buf = Vec::new();
    swf::write_swf(&header, &tags, &mut buf).expect("write_swf failed");
    buf
}

#[test]
fn parse_minimal_swf_without_panicking() {
    let swf_bytes = make_minimal_swf();
    extract_bitmaps(&swf_bytes).expect("extract_bitmaps failed");
    extract_shapes(&swf_bytes).expect("extract_shapes failed");
    extract_fonts(&swf_bytes).expect("extract_fonts failed");
    extract_exported_symbols(&swf_bytes).expect("extract_exported_symbols failed");
}

#[test]
fn shape_extractor_recognises_define_shape() {
    let swf_bytes = make_minimal_swf();
    let shapes = extract_shapes(&swf_bytes).expect("extract_shapes failed");
    assert!(!shapes.is_empty(), "expected at least one shape");
    let shape = shapes.get(&1).expect("shape id=1 not found");
    assert_eq!(shape.id, 1);
    assert!(!shape.fill_styles.is_empty(), "expected fill styles");
}

#[test]
fn action_tags_not_surfaced_by_bitmap_extractor() {
    let swf_bytes = make_swf_with_action();
    let bitmaps = extract_bitmaps(&swf_bytes).expect("extract_bitmaps failed");
    assert!(bitmaps.is_empty(), "DoAction must not produce a bitmap entry");
}

#[test]
fn action_tags_not_surfaced_by_shape_extractor() {
    let swf_bytes = make_swf_with_action();
    let shapes = extract_shapes(&swf_bytes).expect("extract_shapes failed");
    assert!(shapes.is_empty(), "DoAction must not produce a shape entry");
}

#[test]
fn font_edit_text_metrics_use_imported_font_templates() {
    let swf_bytes = make_swf_with_imported_font_edit_text();
    let metrics = extract_font_edit_text_metrics(&swf_bytes).expect("extract metrics failed");
    let text1_thin = metrics.get("$Text1Thin").expect("$Text1Thin metrics");

    assert_eq!(text1_thin.height_px, 20.0);
    assert_eq!(text1_thin.bounds_y_min_px, -2.0);
    assert!((text1_thin.bounds_y_max_px - 23.4).abs() < 0.01);
    assert!((text1_thin.top_overscan_px(20.0) - 5.4).abs() < 0.01);
}

#[test]
fn library_content_hash_is_stable() {
    let bytes = make_minimal_swf();
    let lib1 = SwfAssetLibrary::new(bytes.clone()).expect("library 1 failed");
    let lib2 = SwfAssetLibrary::new(bytes).expect("library 2 failed");
    assert_eq!(lib1.content_hash(), lib2.content_hash());
    assert!(!lib1.content_hash().is_empty());
}

#[test]
fn library_construction_parses_once() {
    use crate::swf_assets::extract::SWF_PARSE_COUNT;
    use std::sync::atomic::Ordering;
    let bytes = make_minimal_swf();
    let before = SWF_PARSE_COUNT.load(Ordering::Relaxed);
    let lib = SwfAssetLibrary::new(bytes.clone()).expect("library");
    let after = SWF_PARSE_COUNT.load(Ordering::Relaxed);
    assert_eq!(
        after - before,
        1,
        "SwfAssetLibrary::new must decompress+parse exactly once"
    );
    // stage size matches the pre-refactor standalone extractor exactly.
    assert_eq!(lib.stage_size(), extract_stage_size(&bytes));
}

#[test]
fn library_shape_count_matches_extract_shapes() {
    let bytes = make_minimal_swf();
    let lib = SwfAssetLibrary::new(bytes.clone()).expect("library failed");
    let direct = extract_shapes(&bytes).expect("extract_shapes failed");
    assert_eq!(lib.shape_count(), direct.len());
    assert!(lib.shape_count() >= 1);
}

#[test]
fn integration_parse_external_swf_if_present() {
    let path = match std::env::var("SB_TEST_SWF") {
        Ok(p) => p,
        Err(_) => return,
    };
    let bytes = std::fs::read(&path).expect("could not read SB_TEST_SWF");
    let lib = SwfAssetLibrary::new(bytes).expect("SwfAssetLibrary::new failed");
    println!(
        "SB_TEST_SWF={path}: bitmaps={} shapes={} fonts={} exports={}",
        lib.bitmap_count(),
        lib.shape_count(),
        lib.font_count(),
        lib.export_count()
    );
    assert!(
        lib.bitmap_count() + lib.shape_count() + lib.font_count() > 0,
        "expected at least one asset in {path}"
    );
}
