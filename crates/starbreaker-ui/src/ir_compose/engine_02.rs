#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use image::RgbaImage;
#[allow(unused_imports)]
use image::imageops;
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet, VecDeque};
#[allow(unused_imports)]
use std::sync::OnceLock;
#[allow(unused_imports)]
use tiny_skia::{BlendMode, Color, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as TskRect, Stroke, Transform};
#[allow(unused_imports)]
use crate::bb_atlas::AtlasLibrary;
#[allow(unused_imports)]
use crate::bb_assets::UiAssetResolver;
#[allow(unused_imports)]
use crate::bb_layout::Rect;
#[allow(unused_imports)]
use crate::compose::ComposeContext;
#[allow(unused_imports)]
use crate::error::UiError;
#[allow(unused_imports)]
use crate::text::{FontKind, TextAlign, TextRenderer, VerticalAlign};
#[allow(unused_imports)]
use crate::swf_assets::{FontGlyphSet, SwfAssetLibrary};
#[allow(unused_imports)]
use crate::ui_ir::{
    validate_ui_ir_document, UiIrAssetLayout, UiIrBorder, UiIrColourBlendMode, UiIrDocument,
    UiIrNode, UiIrPolygon, UiIrRect, UiIrTextPayload, UiIrTextStyle, UiIrValue,
};

// Consolidated engine chunk 02 (formerly: clip.part, polygon_draw.part, part_08.part, part_09.part, part_10.part, part_11.part, part_12.part).
//   clip.part: Ancestor-overflow clipping (`UiIrNode::clip_rect`).
//   polygon_draw.part: WidgetPolygon rendering (the power pip selector arrow).
//   part_12.part: Rounded border chrome: the generic border renderer strokes a rounded-rect

// Ancestor-overflow clipping (`UiIrNode::clip_rect`).
//
// The IR carries the pixel-space intersection of a node's clipping ancestors
// (`overflow` Clip / ClipFade — see `ui_ir::clip_rect_for_node`). Because the
// renderer draws a flat node list, clipping is applied per node:
// `with_node_clip` classifies the node's rect against the clip region and
// either draws normally (fully inside), skips entirely (fully outside), or
// renders through a scratch surface and composites only the clip region back
// (boundary-crossing nodes — the power list's partially visible 4th column).

/// Collect the authored radar spokes (`Circle_Line_*`) from the compiled radar
/// plane: every Primitive node binding the `line_a` material. Geometry +
/// per-spoke colour are read straight from the IR node — `anchor`, authored
/// `sizing` width/height (Percent), `orientation.z`, and the resolved
/// `background` fill (cardinals `Accent1`, diagonals `Base`; alpha 0.1/0.2).
/// Returns the spoke list plus the brand-resolved spoke material path (the
/// fetcher reads its `Gradient`/`InnerAlpha`/`OuterAlpha`/`Glow` for the soft-glow
/// look). Nothing is invented — an empty list means no spoke nodes were loaded.
pub(crate) fn collect_radar_spokes<'a>(
    nodes: &'a [UiIrNode],
    ctx: &ComposeContext<'_>,
) -> (Vec<crate::pipeline::RadarSpokeInput>, Option<&'a str>) {
    let is_line_a = |n: &UiIrNode| {
        n.primitive_material
            .as_deref()
            .is_some_and(|m| m.to_ascii_lowercase().contains("line_a"))
    };
    let spokes = nodes
        .iter()
        .filter(|n| is_line_a(n))
        .filter_map(|n| {
            let fill = n.background_fill_colour.or_else(|| {
                n.background_fill_colour_token
                    .as_deref()
                    .and_then(|t| resolve_surface_colour_token(ctx, t))
                    .map(|mut c| {
                        c[3] *= n.background_fill_alpha.unwrap_or(1.0);
                        c
                    })
            })?;
            Some(crate::pipeline::RadarSpokeInput {
                anchor: n.anchor,
                length_frac: ir_value_to_px(&n.authored_size[1]),
                width_frac: ir_value_to_px(&n.authored_size[0]),
                rotation_deg: n.rotation_deg.unwrap_or(0.0),
                colour: [fill[0], fill[1], fill[2]],
                alpha: fill[3],
            })
        })
        .collect();
    let material = nodes
        .iter()
        .find_map(|n| n.primitive_material.as_deref().filter(|m| m.to_ascii_lowercase().contains("line_a")));
    (spokes, material)
}

/// The outer heading-tape ring (`HeadingTape`): the `headingtape` Primitive whose
/// atlas window is TILED around the ring (`UVSize.x > 1`) — the structural
/// discriminator vs the single-cell `NorthPoint`/readout glyphs that share the
/// material. Returns its brand-resolved material + authored UV window + fill alpha
/// (the brand `Base` ~0.3), all data. `None` when no tape is loaded.
pub(crate) fn collect_radar_heading_tape(nodes: &[UiIrNode]) -> Option<crate::pipeline::RadarHeadingTape<'_>> {
    nodes.iter().find_map(|n| {
        let material = n
            .primitive_material
            .as_deref()
            .filter(|m| m.to_ascii_lowercase().contains("headingtape"))?;
        let uv_start = n.primitive_uv_start?;
        let uv_size = n.primitive_uv_size?;
        let fill_alpha = n
            .background_fill_colour
            .map(|c| c[3])
            .or(n.background_fill_alpha)
            .unwrap_or(1.0);
        (uv_size[0].abs() > 1.0).then_some(crate::pipeline::RadarHeadingTape {
            material_path: material,
            uv_start,
            uv_size,
            fill_alpha,
        })
    })
}

enum ClipRelation {
    Inside,
    Outside,
    Partial,
}

fn classify_clip(clip: &UiIrRect, rect: Rect) -> ClipRelation {
    if clip.w < 0.5
        || clip.h < 0.5
        || rect.x >= clip.x + clip.w
        || rect.y >= clip.y + clip.h
        || rect.x + rect.w <= clip.x
        || rect.y + rect.h <= clip.y
    {
        return ClipRelation::Outside;
    }
    let inside = rect.x >= clip.x
        && rect.y >= clip.y
        && rect.x + rect.w <= clip.x + clip.w
        && rect.y + rect.h <= clip.y + clip.h;
    if inside {
        ClipRelation::Inside
    } else {
        ClipRelation::Partial
    }
}

/// Run `draw` against `pixmap`, restricted to the node's clip region.
pub(crate) fn with_node_clip(
    pixmap: &mut Pixmap,
    clip: Option<&UiIrRect>,
    node_rect: Rect,
    draw: impl FnOnce(&mut Pixmap),
) {
    let Some(clip) = clip else {
        draw(pixmap);
        return;
    };
    match classify_clip(clip, node_rect) {
        ClipRelation::Inside => draw(pixmap),
        ClipRelation::Outside => {}
        ClipRelation::Partial => {
            let Some(mut scratch) = Pixmap::new(pixmap.width(), pixmap.height()) else {
                return;
            };
            draw(&mut scratch);
            composite_clip_region_pixmap(pixmap, &scratch, clip);
        }
    }
}

/// SourceOver-composite `src`'s clip region onto `dst` (premultiplied RGBA).
fn composite_clip_region_pixmap(dst: &mut Pixmap, src: &Pixmap, clip: &UiIrRect) {
    let w = dst.width() as i32;
    let h = dst.height() as i32;
    let x0 = (clip.x.floor() as i32).clamp(0, w);
    let y0 = (clip.y.floor() as i32).clamp(0, h);
    let x1 = ((clip.x + clip.w).ceil() as i32).clamp(0, w);
    let y1 = ((clip.y + clip.h).ceil() as i32).clamp(0, h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for y in y0..y1 {
        let row = (y * w) as usize;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            let sa = src_data[i + 3] as u32;
            if sa == 0 {
                continue;
            }
            // Premultiplied source-over: d' = s + d × (1 − sa).
            let inv = 255 - sa;
            for c in 0..4 {
                let s = src_data[i + c] as u32;
                let d = dst_data[i + c] as u32;
                dst_data[i + c] = (s + (d * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}

/// Run `draw` against a straight-alpha image, restricted to the clip region
/// (the text pass draws after the pixmap is converted to an `RgbaImage`).
pub(crate) fn with_node_clip_image(
    img: &mut RgbaImage,
    clip: Option<&UiIrRect>,
    node_rect: Rect,
    draw: impl FnOnce(&mut RgbaImage),
) {
    let Some(clip) = clip else {
        draw(img);
        return;
    };
    match classify_clip(clip, node_rect) {
        ClipRelation::Inside => draw(img),
        ClipRelation::Outside => {}
        ClipRelation::Partial => {
            let mut scratch = RgbaImage::new(img.width(), img.height());
            draw(&mut scratch);
            composite_clip_region_image(img, &scratch, clip);
        }
    }
}

/// SourceOver-composite `src`'s clip region onto `dst` (straight RGBA).
fn composite_clip_region_image(dst: &mut RgbaImage, src: &RgbaImage, clip: &UiIrRect) {
    let w = dst.width() as i32;
    let h = dst.height() as i32;
    let x0 = (clip.x.floor() as i32).clamp(0, w);
    let y0 = (clip.y.floor() as i32).clamp(0, h);
    let x1 = ((clip.x + clip.w).ceil() as i32).clamp(0, w);
    let y1 = ((clip.y + clip.h).ceil() as i32).clamp(0, h);
    for y in y0..y1 {
        for x in x0..x1 {
            let sp = src.get_pixel(x as u32, y as u32);
            let sa = sp[3] as f32 / 255.0;
            if sa <= 0.0 {
                continue;
            }
            let dp = dst.get_pixel_mut(x as u32, y as u32);
            let da = dp[3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let s = sp[c] as f32;
                let d = dp[c] as f32;
                dp[c] = (((s * sa) + (d * da * (1.0 - sa))) / out_a).round().min(255.0) as u8;
            }
            dp[3] = (out_a * 255.0).round().min(255.0) as u8;
        }
    }
}

// WidgetPolygon rendering (the power pip selector arrow).

/// Draw a `WidgetPolygon` as a filled regular n-gon inscribed in the node
/// rect (the power pip selector arrow: 3 sides, startAngle 270 with a 90°
/// orientation offset = a right-pointing triangle in the brand Bright role).
pub(crate) fn draw_ir_polygon(
    node: &UiIrNode,
    polygon: &UiIrPolygon,
    rect: Rect,
    ctx: &ComposeContext<'_>,
    pixmap: &mut Pixmap,
) {
    if !polygon.do_fill || polygon.sides < 3 {
        return;
    }
    let Some(mut fill) = polygon
        .fill_colour_token
        .as_deref()
        .and_then(|token| resolve_colour_token(ctx, token))
    else {
        return;
    };
    fill[3] *= polygon.fill_alpha;
    if fill[3] <= 0.005 {
        return;
    }
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let radius = rect.w.min(rect.h) * 0.5;
    if radius < 0.5 {
        return;
    }
    let base = (polygon.start_angle_deg + polygon.rotation_deg).to_radians();
    let mut pb = PathBuilder::new();
    for i in 0..polygon.sides {
        let angle = base + (i as f32) * std::f32::consts::TAU / polygon.sides as f32;
        let px = cx + radius * angle.cos();
        let py = cy + radius * angle.sin();
        if i == 0 {
            pb.move_to(px, py);
        } else {
            pb.line_to(px, py);
        }
    }
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(fill, node.alpha));
    paint.anti_alias = true;
    pixmap
        .as_mut()
        .fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports, dead_code)]

    use super::*;

    use std::collections::HashMap;

    use image::Rgba;

    use crate::bb_atlas::AssetFetcher;
    use crate::canvas::RgbaColor;
    use crate::style::{CrtParams, ManufacturerStyle};
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiRendererHint, UiIrAssetLayout, UiIrCustomShape, UiIrStyleTag, UiIrTextStyle};


    fn text_style_with_font_record(record: serde_json::Value) -> UiIrTextStyle {
        UiIrTextStyle {
            font_record: Some("file://./fontstyles/blenderpro-medium.json".into()),
            resolved_font_record: Some(record),
            font_size: UiIrValue::Fixed { value: 18.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".into(),
            vertical_alignment: "Center".into(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: None,
        }
    }


    fn assert_not_uniform(img: &RgbaImage, label: &str) {
        let (w, h) = img.dimensions();
        let mut first: Option<[u8; 4]> = None;
        let mut differing = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                let px = img.get_pixel(x, y).0;
                match first {
                    None => first = Some(px),
                    Some(f) if f != px => differing += 1,
                    _ => {}
                }
            }
        }
        assert!(
            differing > 0,
            "[{label}] image is entirely one colour ({:?})",
            first.unwrap_or([0, 0, 0, 0])
        );
    }


    fn assert_non_background_fraction(
        img: &RgbaImage,
        bg: [u8; 4],
        min_frac: f32,
        label: &str,
    ) {
        let (w, h) = img.dimensions();
        let mut total = 0usize;
        let mut non_bg = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                total += 1;
                let p = img.get_pixel(x, y).0;
                let differs = p
                    .iter()
                    .zip(bg.iter())
                    .any(|(a, b)| (*a as i32 - *b as i32).abs() > 16);
                if differs {
                    non_bg += 1;
                }
            }
        }
        let frac = non_bg as f32 / total.max(1) as f32;
        assert!(
            frac >= min_frac,
            "[{label}] only {:.1}% pixels differ from bg; expected >= {:.1}%",
            frac * 100.0,
            min_frac * 100.0,
        );
    }


    struct StubFetcher {
        images: HashMap<String, Vec<u8>>,
    }


    impl AssetFetcher for StubFetcher {
        fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
            self.images.get(&p4k_path.to_ascii_lowercase()).cloned()
        }
    }


    fn stub_style() -> ManufacturerStyle {
        // Real s_drak_hud palette via the provenance fixture (no hard-coded
        // colour values in test source — see test_palettes).
        crate::test_palettes::brand_style("s_drak_hud")
    }


    fn minimal_swf_assets() -> crate::swf_assets::SwfAssetLibrary {
        crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse")
    }


    fn blank_node(node_type: &str) -> UiIrNode {
        UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: node_type.to_string(),
            name: "node".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 10.0 },
                UiIrValue::Fixed { value: 10.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            clip_rect: None,
            computed_rect: UiIrRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            background_fill_colour: None,
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None,
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        }
    }


    #[test]
    fn font_symbol_reads_buildingblocks_font_style_record() {
        let style = text_style_with_font_record(serde_json::json!({
            "_RecordName_": "BuildingBlocks_FontStyle.BlenderPro-Medium",
            "_RecordValue_": {
                "_Type_": "BuildingBlocks_FontStyle",
                "font": "$Text1Med",
                "scaleModifier": 1.0
            }
        }));

        assert_eq!(font_symbol_from_text_style(Some(&style)), Some("$Text1Med"));
    }


    #[test]
    fn font_symbol_reads_unwrapped_font_style_record() {
        let style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Thin",
            "scaleModifier": 0.92
        }));

        assert_eq!(font_symbol_from_text_style(Some(&style)), Some("$Text1Thin"));
        assert_eq!(font_style_scale_modifier(Some(&style)), 0.92);
    }


    #[test]
    fn font_style_modifiers_scale_with_font_size() {
        let mut style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Thin",
            "scaleModifier": 1.0,
            "leadingModifier": 0.25,
            "topMarginModifier": -0.5
        }));
        style.font_size = UiIrValue::Fixed { value: 20.0 };

        assert_eq!(font_style_leading_modifier_px(Some(&style)), 5.0);
        assert_eq!(font_style_top_margin_offset_px(Some(&style)), -10.0);
    }



    #[test]
    fn apply_font_style_vertical_offset_shifts_rect_y() {
        let mut style = text_style_with_font_record(serde_json::json!({
            "_Type_": "BuildingBlocks_FontStyle",
            "font": "$Text1Thin",
            "scaleModifier": 1.0,
            "topMarginModifier": -0.25
        }));
        style.font_size = UiIrValue::Fixed { value: 16.0 };

        let rect = Rect { x: 10.0, y: 20.0, w: 30.0, h: 40.0 };
        assert_eq!(
            apply_font_style_vertical_offset(rect, Some(&style)),
            Rect { x: 10.0, y: 16.0, w: 30.0, h: 40.0 }
        );
    }


    #[test]
    fn apply_asset_layout_flip_mirrors_horizontally() {
        let mut node = blank_node("display_widget");
        node.asset_layout = Some(UiIrAssetLayout {
            scaling_behavior: None,
            contain_position_x: None,
            contain_position_y: None,
            flip_horizontal: Some(true),
            flip_vertical: None,
        });

        let mut img = RgbaImage::new(2, 1);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([0, 255, 0, 255]));

        let flipped = apply_asset_layout_flip(&node, img);
        assert_eq!(flipped.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(flipped.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }


    #[test]
    fn large_wrapped_title3_heading_adds_line_gap() {
        let mut node = blank_node("widget_text_field");
        node.computed_rect = UiIrRect { x: 0.0, y: 0.0, w: 1344.0, h: 270.0 };
        let style = UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: UiIrValue::Fixed { value: 103.5 },
            line_spacing: Some(-20.7),
            letter_spacing: None,
            alignment: "Center".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: Some("Bright".to_string()),
            label_style: Some("Title3".to_string()),
        };

        assert_eq!(
            draw_line_spacing_for_node(&node, "DIGITAL MEDICAL ASSISTANT", Some(&style)),
            Some(-16.7)
        );
    }


    #[test]
    fn widget_separator_uses_centered_svg_stroke_extent() {
        let rect = TskRect::from_xywh(10.0, 20.0, 80.0, 8.0).expect("test rect");
        let draw_rect = widget_separator_draw_rect(rect, Some(1.0), None);
        assert_eq!(draw_rect.x(), 10.0);
        assert_eq!(draw_rect.y(), 23.0);
        assert_eq!(draw_rect.width(), 80.0);
        assert_eq!(draw_rect.height(), 2.0);

        let fallback = widget_separator_draw_rect(rect, None, None);
        assert_eq!(fallback, rect);
    }


    /// The widget-standard's Min/MaxSize clamp bounds the VISIBLE strip inside
    /// the authored slot box, placed by the entry's Anchor/Pivot (0.5/0.5 =
    /// centred). The strip wins over the svgFill stroke-extent fallback.
    #[test]
    fn widget_separator_strip_clamps_and_places_within_slot() {
        let rect = TskRect::from_xywh(10.0, 20.0, 80.0, 16.0).expect("test rect");
        let strip = crate::ui_ir::UiIrSeparatorStrip {
            min_h: Some(5.0),
            max_h: Some(5.0),
            anchor_y: Some(0.5),
            pivot_y: Some(0.5),
            ..Default::default()
        };
        let draw_rect = widget_separator_draw_rect(rect, Some(1.0), Some(&strip));
        assert_eq!(draw_rect.x(), 10.0);
        assert_eq!(draw_rect.y(), 25.5, "strip centred in the 16px slot");
        assert_eq!(draw_rect.width(), 80.0);
        assert_eq!(draw_rect.height(), 5.0);

        // Width-axis clamp (vertical separators author Min/MaxSizeX).
        let strip_x = crate::ui_ir::UiIrSeparatorStrip {
            min_w: Some(4.0),
            max_w: Some(4.0),
            anchor_x: Some(0.5),
            pivot_x: Some(0.5),
            ..Default::default()
        };
        let draw_rect = widget_separator_draw_rect(rect, None, Some(&strip_x));
        assert_eq!(draw_rect.x(), 48.0, "strip centred across the 80px slot");
        assert_eq!(draw_rect.width(), 4.0);
        assert_eq!(draw_rect.y(), 20.0);
        assert_eq!(draw_rect.height(), 16.0);
    }


    /// No standard strip and no stroke extent → the slot box itself fills
    /// (the pre-existing fallback for separators no widget-standard entry
    /// matches).
    #[test]
    fn widget_separator_without_style_or_stroke_fills_rect() {
        let rect = TskRect::from_xywh(10.0, 20.0, 80.0, 16.0).expect("test rect");
        let draw_rect = widget_separator_draw_rect(rect, None, None);

        assert_eq!(draw_rect, rect);
    }


    #[test]
    fn custom_shape_fill_prefers_fill_tint_over_stroke() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_custom_shape");
        node.asset_ref = Some("shape.svg".to_string());
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.background_fill_colour = Some([0.1, 0.2, 0.3, 1.0]);
        node.stroke_colour = Some([0.8, 0.1, 0.1, 1.0]);

        assert_eq!(custom_shape_fill_override(&node, &ctx), Some([0.1, 0.2, 0.3, 1.0]));
    }


    #[test]
    fn custom_shape_fill_prefers_svg_tint_token_over_background_colour() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_custom_shape");
        node.asset_ref = Some("shape.svg".to_string());
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.background_fill_colour = Some([0.1, 0.9, 0.1, 1.0]);
        node.icon_tint_colour_token = Some("Accent1".to_string());

        // The icon-tint token wins over the background colour, and `Accent1` on a
        // shape fill resolves with surface semantics (slot 4), not slot 0.
        assert_eq!(custom_shape_fill_override(&node, &ctx), style_colour_slot_rgba(&ctx, 4));
    }


    #[test]
    fn custom_shape_fill_resolves_accent1_token_as_surface_slot() {
        // A custom-shape SVG fill is a *surface* fill, so the `Accent1` token must
        // resolve to the surface slot (4), not the foreground slot (0). Regression
        // guard for the medical "fingerprint" rendering light blue instead of the
        // authored darker blue.
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_custom_shape");
        node.asset_ref = Some("UI/Textures/Vector/General/FingerPrint.svg".to_string());
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.icon_tint_colour_token = Some("Accent1".to_string());

        assert_eq!(
            custom_shape_fill_override(&node, &ctx),
            style_colour_slot_rgba(&ctx, 4),
            "Accent1 on a shape fill must resolve to the surface slot (4 = darker blue), not slot 0"
        );
        // slot 4 must be distinct from the foreground/primary slot 0 for this to be meaningful.
        assert_ne!(style_colour_slot_rgba(&ctx, 4), Some(style_primary_rgba(&ctx)));
    }


    #[test]
    fn svg_fill_override_disables_second_blit_tint() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_custom_shape");
        node.icon_tint_colour = Some([0.2, 0.6, 0.9, 1.0]);

        assert_eq!(
            image_tint_for_blit(&node, "UI/Textures/Vector/General/FingerPrint.svg", Some([0.2, 0.6, 0.9, 1.0]), None, &ctx),
            [1.0, 1.0, 1.0, 1.0]
        );
        assert_eq!(
            image_tint_for_blit(&node, "UI/Textures/Icons/FingerPrint.dds", Some([0.2, 0.6, 0.9, 1.0]), None, &ctx),
            [0.2, 0.6, 0.9, 1.0]
        );
    }


    #[test]
    fn manufacturer_logo_tint_prefers_source_icon_tint_over_derived_accent() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_manufacturer_logo");
        node.icon_tint_colour = Some([0.2, 0.6, 0.9, 1.0]);

        assert_eq!(manufacturer_logo_tint(&node, &ctx), [0.2, 0.6, 0.9, 1.0]);
    }


    #[test]
    fn manufacturer_logo_tint_falls_back_to_neutral_when_ir_has_no_style() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let node = blank_node("widget_manufacturer_logo");

        assert_eq!(manufacturer_logo_tint(&node, &ctx), [1.0, 1.0, 1.0, 1.0]);
    }


    fn uniform_test_border(width: f32, colour: [f32; 4]) -> UiIrBorder {
        let side = crate::ui_ir::UiIrBorderSide {
            width,
            colour: Some(colour),
            colour_token: None,
        };
        UiIrBorder {
            top: side.clone(),
            right: side.clone(),
            bottom: side.clone(),
            left: side,
        }
    }

    /// A uniform border with an authored corner radius renders as a rounded
    /// stroke (the modular-kit ghost button chrome): the square corner pixel
    /// stays empty while the edge midpoints carry the border colour.
    #[test]
    fn uniform_border_with_corner_radius_rounds_the_corners() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let border = uniform_test_border(3.0, [1.0, 0.0, 0.0, 1.0]);
        let rect = Rect { x: 0.0, y: 0.0, w: 64.0, h: 64.0 };

        let mut rounded = Pixmap::new(64, 64).unwrap();
        draw_ir_border(&mut rounded, rect, &border, 1.0, &ctx, Some(12.0));
        let corner = rounded.pixel(1, 1).unwrap();
        let top_mid = rounded.pixel(32, 1).unwrap();
        assert_eq!(corner.alpha(), 0, "rounded corner pixel must stay empty");
        assert!(top_mid.alpha() > 0, "top edge midpoint must carry the border");

        let mut square = Pixmap::new(64, 64).unwrap();
        draw_ir_border(&mut square, rect, &border, 1.0, &ctx, None);
        let square_corner = square.pixel(1, 1).unwrap();
        assert!(square_corner.alpha() > 0, "square border keeps the corner pixel");
    }


    fn style_with_seven_slots() -> ManufacturerStyle {
        // Real s_bioc slots 0..7 from the provenance fixture: slot 0 (Base,
        // light blue) and slot 6 (Bright, muted grey) are distinct, so a
        // resolver that lumps `Bright` onto `Base` is caught.
        let mut style = stub_style();
        style.colour_slots = crate::test_palettes::brand_colour_slots("s_bioc")[..7].to_vec();
        style
    }

    #[test]
    fn resolve_colour_token_bright_is_muted_slot_six_not_base() {
        // `Bright` (BB_ColorStyle index 6) is the muted light-grey role, distinct
        // from `Base` (index 0). This backs dim secondary text such as a caption's
        // value (`Heading6` "200/200") under a `Heading3` label.
        let style = style_with_seven_slots();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let bright = resolve_colour_token(&ctx, "Bright").expect("bright resolves");
        assert_eq!(bright, style_colour_slot_rgba(&ctx, 6).expect("slot 6 exists"));
        assert_ne!(
            bright,
            style_colour_slot_rgba(&ctx, 0).expect("slot 0 exists"),
            "Bright must not collapse onto Base"
        );
    }

    #[test]
    fn resolve_colour_token_disabled_is_near_black_slot_eight() {
        // `Disabled` is BB_ColorStyle index 8 (the drak palette's near-black
        // 20,13,5) — the power screen's unpowered pip hitzones paint it. The
        // old mapping lumped it onto the LIGHT slot 6, rendering grey pips.
        let mut style = style_with_seven_slots();
        // Real s_drak_hud slots 7 (Selected) and 8 (Disabled, near-black).
        style.colour_slots.push(crate::test_palettes::brand_slot("s_drak_hud", 7));
        style.colour_slots.push(crate::test_palettes::brand_slot("s_drak_hud", 8));
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let disabled = resolve_colour_token(&ctx, "Disabled").expect("disabled resolves");
        assert_eq!(
            disabled,
            style_colour_slot_rgba(&ctx, 8).expect("slot 8 exists"),
            "Disabled is the near-black surface slot"
        );
    }

    #[test]
    fn custom_shape_svg_uses_source_over_blend() {
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });

        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/Vector/General/FingerPrint.svg"),
            BlendMode::SourceOver
        );
        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/I_InteractiveScreens/Med/fingerprint_glow.tif"),
            BlendMode::Plus
        );
    }

    /// A custom-shape SVG whose authored `scalingBehavior` is `Contain` must
    /// rasterize aspect-preserved (the universal authored default; the MFD
    /// footer's 7.3×12.8 pixel arrow renders 31×48 in-game, not stretched to
    /// its square icon box). Nine-slice stays for non-Contain shapes.
    #[test]
    fn contain_custom_shape_svg_preserves_aspect() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 20"><rect width="10" height="20" fill="#ffffff"/></svg>"##;
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: Some(true),
            nine_slice_rect: Some([0.25, 0.25, 0.75, 0.75]),
            nine_slice_scale: Some(1.0),
        });
        node.asset_layout = Some(UiIrAssetLayout {
            scaling_behavior: Some("Contain".to_string()),
            contain_position_x: Some(0.5),
            contain_position_y: Some(0.5),
            flip_horizontal: None,
            flip_vertical: None,
        });
        node.resolved_style_tags = vec![crate::ui_ir::UiIrStyleTag {
            uuid: "f530a994-2947-4fba-bd86-c4046c471ba2".to_string(),
            tag_name: Some("icon-element-instance".to_string()),
        }];

        let img = rasterize_svg_for_node(&node, svg, 80, 40, None).expect("raster");
        assert_eq!((img.width(), img.height()), (80, 40));
        // Contained 10:20 into 80×40 → drawn width = 20px, centred (x 30..50);
        // the left margin must be fully transparent, the centre opaque.
        let left_margin_opaque = (0..40).any(|y| img.get_pixel(5, y)[3] > 0);
        let centre_opaque = (0..40).any(|y| img.get_pixel(40, y)[3] > 0);
        assert!(!left_margin_opaque, "left margin must be transparent under Contain");
        assert!(centre_opaque, "glyph must draw in the centre");
    }

    /// A plain custom shape (no icon-element-instance tag) keeps the
    /// nine-slice/stretch pipeline even though `Contain` is its authored
    /// default — the medical menu cards' thin edge-line svgs stretch to full
    /// card height in-game.
    #[test]
    fn plain_custom_shape_svg_keeps_stretch_despite_authored_contain() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 20"><rect width="10" height="20" fill="#ffffff"/></svg>"##;
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: None,
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.asset_layout = Some(UiIrAssetLayout {
            scaling_behavior: Some("Contain".to_string()),
            contain_position_x: Some(0.5),
            contain_position_y: Some(0.5),
            flip_horizontal: None,
            flip_vertical: None,
        });

        let img = rasterize_svg_for_node(&node, svg, 80, 40, None).expect("raster");
        // Stretched: the left edge stays opaque (the rect fills the width).
        let left_opaque = (0..40).any(|y| img.get_pixel(2, y)[3] > 0);
        assert!(left_opaque, "plain custom shape must stretch to the rect");
    }


}

#[cfg(test)]
mod tests_c {
    #![allow(unused_imports, dead_code)]
    use super::*;

    use std::collections::HashMap;

    use image::Rgba;

    use crate::bb_atlas::AssetFetcher;
    use crate::canvas::RgbaColor;
    use crate::style::{CrtParams, ManufacturerStyle};
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiRendererHint, UiIrAssetLayout, UiIrCustomShape, UiIrStyleTag, UiIrTextStyle};


    fn text_style_with_font_record(record: serde_json::Value) -> UiIrTextStyle {
        UiIrTextStyle {
            font_record: Some("file://./fontstyles/blenderpro-medium.json".into()),
            resolved_font_record: Some(record),
            font_size: UiIrValue::Fixed { value: 18.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".into(),
            vertical_alignment: "Center".into(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: None,
        }
    }


    fn assert_not_uniform(img: &RgbaImage, label: &str) {
        let (w, h) = img.dimensions();
        let mut first: Option<[u8; 4]> = None;
        let mut differing = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                let px = img.get_pixel(x, y).0;
                match first {
                    None => first = Some(px),
                    Some(f) if f != px => differing += 1,
                    _ => {}
                }
            }
        }
        assert!(
            differing > 0,
            "[{label}] image is entirely one colour ({:?})",
            first.unwrap_or([0, 0, 0, 0])
        );
    }


    fn assert_non_background_fraction(
        img: &RgbaImage,
        bg: [u8; 4],
        min_frac: f32,
        label: &str,
    ) {
        let (w, h) = img.dimensions();
        let mut total = 0usize;
        let mut non_bg = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                total += 1;
                let p = img.get_pixel(x, y).0;
                let differs = p
                    .iter()
                    .zip(bg.iter())
                    .any(|(a, b)| (*a as i32 - *b as i32).abs() > 16);
                if differs {
                    non_bg += 1;
                }
            }
        }
        let frac = non_bg as f32 / total.max(1) as f32;
        assert!(
            frac >= min_frac,
            "[{label}] only {:.1}% pixels differ from bg; expected >= {:.1}%",
            frac * 100.0,
            min_frac * 100.0,
        );
    }


    struct StubFetcher {
        images: HashMap<String, Vec<u8>>,
    }


    impl AssetFetcher for StubFetcher {
        fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
            self.images.get(&p4k_path.to_ascii_lowercase()).cloned()
        }
    }


    fn stub_style() -> ManufacturerStyle {
        // Real s_drak_hud palette via the provenance fixture (no hard-coded
        // colour values in test source — see test_palettes).
        crate::test_palettes::brand_style("s_drak_hud")
    }


    fn minimal_swf_assets() -> crate::swf_assets::SwfAssetLibrary {
        crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse")
    }


    fn blank_node(node_type: &str) -> UiIrNode {
        UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: node_type.to_string(),
            name: "node".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 10.0 },
                UiIrValue::Fixed { value: 10.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            clip_rect: None,
            computed_rect: UiIrRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            background_fill_colour: None,
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None,
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        }
    }


    #[test]
    fn matte_strip_preserves_sparse_line_art_strokes() {
        let mut img = RgbaImage::new(10, 10);
        for y in 0..10 {
            img.put_pixel(5, y, image::Rgba([0, 0, 0, 255]));
        }

        let stripped = strip_custom_shape_uniform_matte(&img);
        assert_eq!(stripped.get_pixel(5, 5).0, [0, 0, 0, 255]);
    }


    #[test]
    fn matte_strip_removes_dominant_opaque_matte() {
        let mut img = RgbaImage::new(10, 10);
        for y in 0..9 {
            for x in 0..10 {
                img.put_pixel(x, y, image::Rgba([1, 2, 3, 255]));
            }
        }

        let stripped = strip_custom_shape_uniform_matte(&img);
        assert_eq!(stripped.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(stripped.get_pixel(9, 9).0, [0, 0, 0, 0]);
    }


    #[test]
    fn image_tint_token_resolves_without_background_fill() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_image");
        node.icon_tint_colour_token = Some("Base".to_string());

        assert_eq!(
            image_tint_for_blit(&node, "UI/Textures/Shared/panel-bar.tif", None, None, &ctx),
            style_primary_rgba(&ctx)
        );
    }


    #[test]
    fn missionobjectives_token_resolves_to_slot_16() {
        // BB_ColorStyle enum index 16 = MissionObjectives — the design
        // system's generic icon colour (the power system/card icons render
        // it in-game; HUD records author `FillColor=MissionObjectives` for
        // their generic Icon Styles).
        let mut style = stub_style();
        style.colour_slots.truncate(16);
        while style.colour_slots.len() < 16 {
            style.colour_slots.push(RgbaColor { r: 0, g: 0, b: 0, a: 255 });
        }
        // Real s_drak_hud slot 16 (MissionObjectives).
        style.colour_slots.push(crate::test_palettes::brand_slot("s_drak_hud", 16));
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        assert_eq!(
            resolve_colour_token(&ctx, "MissionObjectives"),
            Some([243.0 / 255.0, 220.0 / 255.0, 110.0 / 255.0, 1.0])
        );
    }


    #[test]
    fn white_mask_overlay_composites_in_linear_light() {
        // The engine composites in linear light; sRGB-space blending crushes
        // low-alpha bright-over-dark blends (the annunciator glow rendered
        // (15,9,3) where the reference shows (71,48,15) at the chiclet edge).
        // The white-mask overlay blit converts to linear, blends, re-encodes:
        // white texel a=146/255, node alpha 0.1, tint (1.0,0.62,0.22) over
        // opaque black => sRGB-encoded ~(68,38,8).
        let mut pixmap = Pixmap::new(1, 1).expect("pixmap");
        pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
        let mut img = RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([255, 255, 255, 146]));

        blit_white_mask_overlay_linear(
            &mut pixmap,
            &img,
            0,
            0,
            [1.0, 0.6196, 0.2235, 1.0],
            0.1,
        );

        let px = pixmap.data();
        assert!(
            (px[0] as i32 - 68).abs() <= 2 && (px[1] as i32 - 38).abs() <= 2 && px[2] <= 10,
            "linear-light blend of the white mask glow, got ({}, {}, {}, {})",
            px[0],
            px[1],
            px[2],
            px[3]
        );
    }


    #[test]
    fn white_mask_image_with_colour_overlay_defaults_to_base_tint() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_image");
        node.colour_overlay_enabled = true;

        // A pure-white texture whose shape lives in the alpha channel (the
        // annunciator chiclet glow Annunciator_On.tif).
        let mut img = RgbaImage::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let alpha = if x < 4 { 0 } else { 128 };
                img.put_pixel(x, y, image::Rgba([255, 255, 255, alpha]));
            }
        }

        assert_eq!(
            image_tint_for_blit(
                &node,
                "UI/Textures/H_HUDscreens/Ships/General/Annunciator_On.tif",
                None,
                Some(&img),
                &ctx
            ),
            style_primary_rgba(&ctx),
            "an overlay-enabled white alpha-mask texture takes the brand Base overlay \
             (the MRAI brand authors the same as explicit FillColor entries)"
        );
    }


    #[test]
    fn coloured_texture_with_colour_overlay_keeps_own_colours() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_image");
        node.colour_overlay_enabled = true;

        // A coloured texture (card photo / MFD body backplate) carries its own
        // colours: the editor-default overlay flag must not tint it.
        let mut img = RgbaImage::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                img.put_pixel(x, y, image::Rgba([120, 60, 20, 255]));
            }
        }

        assert_eq!(
            image_tint_for_blit(
                &node,
                "UI/Textures/I_InteractiveScreens/MFD/DRAK/DRAK_GroundVehicle_Dashboard_background_2.tif",
                None,
                Some(&img),
                &ctx
            ),
            [1.0, 1.0, 1.0, 1.0]
        );
    }


    #[test]
    fn color_style_tokens_resolve_to_palette_slots() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        assert_eq!(resolve_colour_token(&ctx, "Base"), Some(style_primary_rgba(&ctx)));
        assert_eq!(resolve_colour_token(&ctx, "Accent1"), Some(style_primary_rgba(&ctx)));
        // Surface `Accent1` keeps the enum slot 4 (the medical fingerprint's
        // darker blue); non-enum aliases resolve to None (they occur in no
        // DataCore record — remediation plan Phase 1 audit).
        assert_eq!(
            resolve_surface_colour_token(&ctx, "Accent1"),
            style_colour_slot_rgba(&ctx, 4)
        );
        assert_eq!(resolve_colour_token(&ctx, "Accent5"), None);
        assert_eq!(resolve_colour_token(&ctx, "Positive"), style_colour_slot_rgba(&ctx, 1));
        // `Moderate` is BB_ColorStyle index 2 — the same slot the `Accent3` /
        // `Warning` aliases resolve (the annunciator's "Moderate - SubItem"
        // BackgroundColor lights the WPN chiclet amber).
        assert_eq!(
            resolve_colour_token(&ctx, "Moderate"),
            style_colour_slot_rgba(&ctx, 2)
        );

        // `Accent2` is BB_ColorStyle enum index 5: the target screen's
        // `NO TARGET` (drak `Base/Bright Elements` FillColor=Accent2) renders
        // s_drak_hud slot 5 (222,88,3) on the in-game capture — the brand H1
        // deep orange, not the light slot-0/2 orange.
        let drake_hud_like_style = ManufacturerStyle {
            colour_slots: crate::test_palettes::brand_colour_slots("s_drak_hud"),
            ..style.clone()
        };
        let drake_ctx = ComposeContext {
            style: &drake_hud_like_style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        assert_eq!(
            resolve_colour_token(&drake_ctx, "Accent2"),
            style_colour_slot_rgba(&drake_ctx, 5)
        );

        let mut node = blank_node("widget_text_field");
        assert_eq!(resolved_text_colour(&node, None, &ctx), [255, 255, 255, 255]);

        node.text_style = Some(crate::ui_ir::UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: crate::ui_ir::UiIrValue::Fixed { value: 28.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: Some("Base".to_string()),
            label_style: None,
        });
        assert_eq!(
            resolved_text_colour(&node, node.text_style.as_ref(), &ctx),
            rgba_to_u8(style_primary_rgba(&ctx))
        );
    }


    #[test]
    fn untagged_heading_text_defaults_to_white_without_ir_token() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_text_field");
        node.text_style = Some(crate::ui_ir::UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: crate::ui_ir::UiIrValue::Fixed { value: 28.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: Some("Heading1".to_string()),
        });

        assert_eq!(
            resolved_text_colour(&node, node.text_style.as_ref(), &ctx),
            [255, 255, 255, 255]
        );
    }


    #[test]
    fn custom_shape_icon_tint_token_uses_deep_blue_palette_slot() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.icon_tint_colour_token = Some("Accent1".to_string());

        assert_eq!(custom_shape_fill_override(&node, &ctx), style_colour_slot_rgba(&ctx, 4));
    }


    #[test]
    fn custom_shape_without_explicit_ir_tint_does_not_infer_from_style_tags() {
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.resolved_style_tags = vec![UiIrStyleTag {
            uuid: "tag-modify".to_string(),
            tag_name: Some("Modify".to_string()),
        }];

        assert_eq!(custom_shape_fill_override(&node, &ctx), None);
    }


    #[test]
    fn custom_shape_modify_svg_uses_additive_blend() {
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.colour_blend_mode = Some(UiIrColourBlendMode::Additive);

        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/Vector/General/FingerPrint.svg"),
            BlendMode::Plus
        );
    }


    #[test]
    fn custom_shape_svg_style_tags_do_not_imply_additive_without_ir_blend_mode() {
        let mut node = blank_node("widget_custom_shape");
        node.custom_shape = Some(UiIrCustomShape {
            shape_type: None,
            shape: None,
            svg_path: Some("UI/Textures/Vector/General/FingerPrint.svg".to_string()),
            render_shape: Some(true),
            enable_nine_slice_rect: None,
            nine_slice_rect: None,
            nine_slice_scale: None,
        });
        node.resolved_style_tags = vec![UiIrStyleTag {
            uuid: "tag-modify".to_string(),
            tag_name: Some("Modify".to_string()),
        }];

        assert_eq!(
            image_blend_mode_for_node(&node, "UI/Textures/Vector/General/FingerPrint.svg"),
            BlendMode::SourceOver
        );
    }


    #[test]
    fn rasterize_svg_without_custom_shape_uses_standard_svg_path() {
        let node = blank_node("display_widget");
        let svg = br#"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>
            <rect x='0' y='0' width='8' height='8' fill='#ffffff'/>
        </svg>"#;

        let image = rasterize_custom_shape_svg(&node, svg, 8, 8, Some([1.0, 0.0, 0.0, 1.0]))
            .expect("svg should rasterize without custom-shape metadata");
        let pixel = image.get_pixel(4, 4);
        assert!(pixel[0] > 200, "expected red tint contribution, got {}", pixel[0]);
        assert!(pixel[1] < 20, "expected low green channel, got {}", pixel[1]);
    }


    /// The manufacturer logo SVG renders per its AUTHORED asset layout —
    /// contain-fit at the authored contain position, with no alpha-balance
    /// recentring. Measured on the medical bed reference: the square
    /// 1024-viewBox Bioticorp SVG contain-fits its 120×140 box top-anchored
    /// (authored Contain/0/0), putting the glyph at rows 1016–1055; the old
    /// stretch + recentring drew it ~11px lower and ~15% taller.
    #[test]
    fn manufacturer_logo_svg_renders_as_authored_without_recentring() {
        // Square viewBox, bottom-heavy content, in a 40×80 box: contain-fit
        // (width-bound, top-anchored) puts the glyph at rect-relative rows
        // 24..32; a stretch puts it at 48..64; stretch+recentring at 42..58.
        let svg = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10' width='10' height='10'>
            <rect x='1' y='6' width='8' height='2' fill='#ffffff'/>
        </svg>"#.to_vec();
        let mut images = HashMap::new();
        images.insert(
            "data/ui/textures/signs/brands/drak/drak_logo.svg".to_string(),
            svg,
        );
        let fetcher = StubFetcher { images };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let mut node = blank_node("BuildingBlocks_WidgetManufacturerLogo");
        node.computed_rect = UiIrRect { x: 10.0, y: 10.0, w: 40.0, h: 80.0 };
        node.asset_layout = Some(UiIrAssetLayout {
            scaling_behavior: Some("Contain".to_string()),
            contain_position_x: Some(0.0),
            contain_position_y: Some(0.0),
            flip_horizontal: None,
            flip_vertical: None,
        });

        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "logo-test".to_string(),
            canvas_name: Some("BuildingBlocks_Canvas.LogoPlacement".to_string()),
            target_width: 100,
            target_height: 100,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![node],
        };

        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("render");
        // Contain in the 40×80 box is width-bound: the 10-unit viewBox maps
        // to 40px anchored at containY 0.0, so the y=6..8 glyph occupies
        // rect-relative rows 24..32 → absolute rows 34..42.
        let bg = [ctx.style.background.r, ctx.style.background.g, ctx.style.background.b, ctx.style.background.a];
        let glyph_rows: Vec<u32> = (0..img.height())
            .filter(|&y| {
                (0..img.width()).any(|x| {
                    let p = img.get_pixel(x, y).0;
                    p.iter().zip(bg.iter()).any(|(a, b)| (*a as i32 - *b as i32).abs() > 24)
                })
            })
            .collect();
        let top = *glyph_rows.first().expect("logo glyph visible");
        assert!(
            (top as i32 - 34).abs() <= 1,
            "logo glyph contain-fits at the authored position (top ≈34), got top {top} \
             (a stretch puts it at ~58; stretch + alpha-balance recentring at ~52)"
        );
    }

    #[test]
    fn render_ui_ir_document_renders_text_from_golden_fixture() {
        let document: UiIrDocument = serde_json::from_str(include_str!(
            "../../tests/fixtures/ui_ir/expected_testroot_ir.json"
        ))
        .expect("golden fixture should parse");
        let fetcher = StubFetcher { images: HashMap::new() };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("IR render should succeed");
        assert_eq!(img.dimensions(), (200, 100));
        assert_not_uniform(&img, "ir-text-golden");
        assert_non_background_fraction(&img, [48, 32, 16, 255], 0.005, "ir-text-golden");
    }


    #[test]
    fn render_ui_ir_document_draws_fill_border_and_asset_ref() {
        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "test-guid".to_string(),
            canvas_name: Some("BuildingBlocks_Canvas.TestIrRender".to_string()),
            target_width: 32,
            target_height: 32,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: vec!["test/red.png".to_string()],
            missing_asset_refs: Vec::new(),
            nodes: vec![UiIrNode {
                id: 1,
                parent_id: None,
                children: Vec::new(),
                node_type: "widget_image".to_string(),
                name: "card".to_string(),
                is_active: true,
                layer: 0,
                alpha: 1.0,
                anchor: [0.0, 0.0],
                pivot: [0.0, 0.0],
                rotation_deg: None,
                authored_position: [4.0, 4.0],
                authored_size: [
                    UiIrValue::Fixed { value: 24.0 },
                    UiIrValue::Fixed { value: 24.0 },
                ],
                padding: [0.0, 0.0, 0.0, 0.0],
                margin: [0.0, 0.0, 0.0, 0.0],
                overflow_mode: None,
                clip_rect: None,
                computed_rect: UiIrRect { x: 4.0, y: 4.0, w: 24.0, h: 24.0 },
                background_fill_colour: Some([0.0, 0.0, 1.0, 1.0]),
                corner_radius: None,
                corner_radii: None,
                corner_chamfers: None,
                background_fill_alpha: None,
                background_fill_colour_token: None,
                circle_fill_colour_token: None,
                style_provenance: None,
                segmented_fill: None,
            polygon: None,
                border: Some(UiIrBorder {
                    top: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    right: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    bottom: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    left: crate::ui_ir::UiIrBorderSide { width: 2.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                }),
                stroke_colour: None,
                stroke_colour_token: None,
                stroke_extent: None,
                separator_strip: None,
                colour_blend_mode: None,
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None::<UiIrTextStyle>,
                asset_ref: Some("test/red.png".to_string()),
                primitive_material: None,
                primitive_uv_start: None,
                primitive_uv_size: None,
                asset_layout: None,
                custom_shape: None,
                style_tag_uuids: Vec::new(),
                resolved_style_tags: Vec::new(),
                is_flash_renderer: false,
                auto_font_size: false,
            colour_overlay_enabled: false,
            }],
        };

        let png = image::RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(png)
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .expect("png encoding");

        let fetcher = StubFetcher {
            images: HashMap::from([("test/red.png".to_string(), encoded)]),
        };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("IR render should succeed");
        let center = img.get_pixel(16, 16).0;
        assert!(center[0] > 0, "asset tint should contribute red at the center");
        let border = img.get_pixel(4, 4).0;
        assert!(border[0] > 200 && border[1] > 200, "border pixel should be yellow-ish");
    }

    /// A `WidgetRuntimeImage` (the SELF-STATUS own-vehicle hologram) composites
    /// the hologram fetcher's rendered image into the node rect, in place of the
    /// node's flat authored background fill. Guards the `draw_non_text_node`
    /// runtime-image branch + the `ComposeContext::hologram_fetcher` wiring.
    #[test]
    fn render_ui_ir_document_composites_vehicle_hologram() {
        struct StubHolo;
        impl crate::pipeline::HologramFetcher for StubHolo {
            fn fetch_vehicle_hologram(
                &self,
                width: u32,
                height: u32,
                _tint: [f32; 4],
            ) -> Option<crate::pipeline::HologramImage> {
                // Solid opaque GREEN — distinct from the node's blue authored fill.
                Some(crate::pipeline::HologramImage {
                    width,
                    height,
                    rgba: [0u8, 255, 0, 255].repeat((width * height) as usize),
                })
            }
        }

        let mut document: UiIrDocument = serde_json::from_str(include_str!(
            "../../tests/fixtures/ui_ir/expected_testroot_ir.json"
        ))
        .expect("golden fixture should parse");
        // One own-vehicle runtime-image node with a BLUE authored holo tint.
        document.nodes.truncate(1);
        let node = &mut document.nodes[0];
        node.node_type = "BuildingBlocks_WidgetRuntimeImage".to_string();
        node.background_fill_colour = Some([0.0, 0.0, 1.0, 1.0]);
        node.is_active = true;
        node.custom_shape = None;
        node.asset_ref = None;
        node.alpha = 1.0;
        let rect = node.computed_rect;

        let asset_fetcher = StubFetcher { images: HashMap::new() };
        let atlas = AtlasLibrary::new(&asset_fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let holo = StubHolo;

        // With a fetcher: the rect composites GREEN (the hologram).
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: Some(&holo),
        };
        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("IR render should succeed");
        let px = (rect.x + rect.w * 0.5) as u32;
        let py = (rect.y + rect.h * 0.5) as u32;
        let centre = img.get_pixel(px.min(img.width() - 1), py.min(img.height() - 1)).0;
        assert!(
            centre[1] > 150 && centre[1] > centre[2],
            "hologram (green) should be composited into the runtime-image rect, got {centre:?}"
        );

        // Without a fetcher: falls back to the flat authored BLUE fill (no green).
        let ctx_none = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };
        let img2 = render_ui_ir_document(&document, &ctx_none, &atlas).expect("IR render should succeed");
        let centre2 = img2.get_pixel(px.min(img2.width() - 1), py.min(img2.height() - 1)).0;
        assert!(
            centre2[2] > centre2[1],
            "without a fetcher the authored blue fill should show, got {centre2:?}"
        );
    }

    /// A `WidgetTextField`'s `StrokeColor` is its glyph outline, NOT a box around
    /// the field rect — `node_draws_rect_stroke` must exclude text fields so the
    /// DRAK velocity-num readouts (white ~0.64α text StrokeColor) don't render a
    /// box outline around "0m/s"/"0.0 G". Other stroked widgets still box.
    #[test]
    fn text_field_stroke_does_not_draw_rect_box() {
        let document: UiIrDocument = serde_json::from_str(include_str!(
            "../../tests/fixtures/ui_ir/expected_testroot_ir.json"
        ))
        .expect("golden fixture should parse");
        let mut node = document.nodes[0].clone();
        node.stroke_colour = Some([1.0, 1.0, 1.0, 0.643]);
        node.stroke_extent = Some(2.0);
        node.custom_shape = None;

        node.node_type = "widget_text_field".to_string();
        assert!(
            !node_draws_rect_stroke(&node),
            "a text field's StrokeColor is a glyph outline, not a box border"
        );

        node.node_type = "display_widget".to_string();
        assert!(
            node_draws_rect_stroke(&node),
            "a non-text stroked widget still draws its rect box"
        );
    }

    /// A node whose fill extends past its `clip_rect` (the scroll view's
    /// partially visible column) must paint inside the clip and leave the
    /// outside untouched.
    #[test]
    fn render_ui_ir_document_clips_fill_to_clip_rect() {
        let mut document: UiIrDocument = serde_json::from_str(include_str!(
            "../../tests/fixtures/ui_ir/expected_testroot_ir.json"
        ))
        .expect("golden fixture should parse");
        document.nodes = vec![UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: "display_widget".to_string(),
            name: "clipped_fill".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 100.0 },
                UiIrValue::Fixed { value: 20.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            clip_rect: Some(UiIrRect { x: 0.0, y: 0.0, w: 50.0, h: 100.0 }),
            computed_rect: UiIrRect { x: 10.0, y: 10.0, w: 100.0, h: 20.0 },
            background_fill_colour: Some([0.0, 1.0, 0.0, 1.0]),
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None::<UiIrTextStyle>,
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        }];

        let fetcher = StubFetcher { images: HashMap::new() };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let img = render_ui_ir_document(&document, &ctx, &atlas).expect("IR render should succeed");
        let inside = img.get_pixel(30, 15).0;
        assert!(inside[1] > 200, "fill inside the clip must paint green, got {inside:?}");
        let outside = img.get_pixel(70, 15).0;
        let bg = &style.background;
        assert_eq!(
            outside,
            [bg.r, bg.g, bg.b, bg.a],
            "fill outside the clip must leave the background untouched"
        );
    }


}

#[cfg(test)]
mod tests_d {
    #![allow(unused_imports, dead_code)]

    use super::*;

    use std::collections::HashMap;

    use image::Rgba;

    use crate::bb_atlas::AssetFetcher;
    use crate::canvas::RgbaColor;
    use crate::style::{CrtParams, ManufacturerStyle};
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiRendererHint, UiIrAssetLayout, UiIrCustomShape, UiIrStyleTag, UiIrTextStyle};


    fn text_style_with_font_record(record: serde_json::Value) -> UiIrTextStyle {
        UiIrTextStyle {
            font_record: Some("file://./fontstyles/blenderpro-medium.json".into()),
            resolved_font_record: Some(record),
            font_size: UiIrValue::Fixed { value: 18.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".into(),
            vertical_alignment: "Center".into(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: None,
        }
    }


    fn assert_not_uniform(img: &RgbaImage, label: &str) {
        let (w, h) = img.dimensions();
        let mut first: Option<[u8; 4]> = None;
        let mut differing = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                let px = img.get_pixel(x, y).0;
                match first {
                    None => first = Some(px),
                    Some(f) if f != px => differing += 1,
                    _ => {}
                }
            }
        }
        assert!(
            differing > 0,
            "[{label}] image is entirely one colour ({:?})",
            first.unwrap_or([0, 0, 0, 0])
        );
    }


    fn assert_non_background_fraction(
        img: &RgbaImage,
        bg: [u8; 4],
        min_frac: f32,
        label: &str,
    ) {
        let (w, h) = img.dimensions();
        let mut total = 0usize;
        let mut non_bg = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                total += 1;
                let p = img.get_pixel(x, y).0;
                let differs = p
                    .iter()
                    .zip(bg.iter())
                    .any(|(a, b)| (*a as i32 - *b as i32).abs() > 16);
                if differs {
                    non_bg += 1;
                }
            }
        }
        let frac = non_bg as f32 / total.max(1) as f32;
        assert!(
            frac >= min_frac,
            "[{label}] only {:.1}% pixels differ from bg; expected >= {:.1}%",
            frac * 100.0,
            min_frac * 100.0,
        );
    }


    struct StubFetcher {
        images: HashMap<String, Vec<u8>>,
    }


    impl AssetFetcher for StubFetcher {
        fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
            self.images.get(&p4k_path.to_ascii_lowercase()).cloned()
        }
    }


    fn stub_style() -> ManufacturerStyle {
        // Real s_drak_hud palette via the provenance fixture (no hard-coded
        // colour values in test source — see test_palettes).
        crate::test_palettes::brand_style("s_drak_hud")
    }


    fn minimal_swf_assets() -> crate::swf_assets::SwfAssetLibrary {
        crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse")
    }


    fn blank_node(node_type: &str) -> UiIrNode {
        UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: node_type.to_string(),
            name: "node".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 10.0 },
                UiIrValue::Fixed { value: 10.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            clip_rect: None,
            computed_rect: UiIrRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            background_fill_colour: None,
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None,
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        }
    }


    #[test]
    fn render_ui_ir_document_is_deterministic_and_layout_sensitive() {
        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "layout-guid".to_string(),
            canvas_name: Some("BuildingBlocks_Canvas.LayoutDeterministic".to_string()),
            target_width: 40,
            target_height: 24,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![UiIrNode {
                id: 1,
                parent_id: None,
                children: Vec::new(),
                node_type: "widget_canvas".to_string(),
                name: "panel".to_string(),
                is_active: true,
                layer: 0,
                alpha: 1.0,
                anchor: [0.0, 0.0],
                pivot: [0.0, 0.0],
                rotation_deg: None,
                authored_position: [5.0, 6.0],
                authored_size: [
                    UiIrValue::Fixed { value: 18.0 },
                    UiIrValue::Fixed { value: 10.0 },
                ],
                padding: [0.0, 0.0, 0.0, 0.0],
                margin: [0.0, 0.0, 0.0, 0.0],
                overflow_mode: None,
                clip_rect: None,
                computed_rect: UiIrRect { x: 5.0, y: 6.0, w: 18.0, h: 10.0 },
                background_fill_colour: Some([0.0, 0.0, 1.0, 1.0]),
                corner_radius: None,
                corner_radii: None,
                corner_chamfers: None,
                background_fill_alpha: None,
                background_fill_colour_token: None,
                circle_fill_colour_token: None,
                style_provenance: None,
                segmented_fill: None,
            polygon: None,
                border: Some(UiIrBorder {
                    top: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    right: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    bottom: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                    left: crate::ui_ir::UiIrBorderSide { width: 1.0, colour: Some([1.0, 1.0, 0.0, 1.0]), colour_token: None },
                }),
                stroke_colour: None,
                stroke_colour_token: None,
                stroke_extent: None,
                separator_strip: None,
                colour_blend_mode: None,
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None::<UiIrTextStyle>,
                asset_ref: None,
                primitive_material: None,
                primitive_uv_start: None,
                primitive_uv_size: None,
                asset_layout: None,
                custom_shape: None,
                style_tag_uuids: Vec::new(),
                resolved_style_tags: Vec::new(),
                is_flash_renderer: false,
                auto_font_size: false,
            colour_overlay_enabled: false,
            }],
        };

        let fetcher = StubFetcher { images: HashMap::new() };
        let atlas = AtlasLibrary::new(&fetcher, Some("drak"));
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse");
        let ctx = ComposeContext {
            style: &style,
            defaults: &defaults,
            assets: &assets,
            hologram_fetcher: None,
        };

        let img_a = render_ui_ir_document(&document, &ctx, &atlas).expect("first render should succeed");
        let img_b = render_ui_ir_document(&document, &ctx, &atlas).expect("second render should succeed");

        assert_eq!(img_a.as_raw(), img_b.as_raw(), "same IR should render bit-stably");
        let bg = ctx.style.background;
        assert_eq!(img_a.get_pixel(1, 1).0, [bg.r, bg.g, bg.b, bg.a], "background pixel should remain style background");
        assert!(img_a.get_pixel(5, 6).0[0] > 200 && img_a.get_pixel(5, 6).0[1] > 200, "border origin should be yellow");
        assert!(img_a.get_pixel(12, 10).0[2] > 200, "panel interior should render blue fill at the authored position");
    }


    #[test]
    fn compose_source_does_not_reintroduce_forbidden_hardcoded_markers() {
        // Scan the REAL engine sources line-by-line. (The pre-F1 version read
        // `engine.inc` — 2 include! lines — so the guard was silently vacuous.)
        // Comments and `test_palettes::` provenance-fixture lines are exempt:
        // tests reference real brand records THROUGH the sanctioned fixture;
        // the ban is on marker-based gating in PRODUCTION composition code.
        let source: String = [include_str!("engine_01.rs"), include_str!("engine_02.rs")]
            .iter()
            .flat_map(|src| src.lines())
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && !line.contains("test_palettes::")
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Hard rule: do not add heuristic marker names. If this trips, fix the
        // structural root cause so composition remains generic across screens.
        // Renaming around this assertion is not an acceptable workaround.
        let forbidden = [
            ["_", "candidate"].concat(),
            ["is_", "med", "ical1_layout"].concat(),
            ["is_", "medical", "_attract_banner_text"].concat(),
            ["is_", "footer", "_brand_text_context"].concat(),
            ["med", "ical_", "cyan_tint"].concat(),
            ["Top_", "seperator"].concat(),
            ["MedGel", "FillMeter"].concat(),
            ["Function", "Title"].concat(),
            ["node", ".name", ".eq_ignore_ascii_case(\"Med", "Gel\")"].concat(),
            ["node", ".name", ".eq_ignore_ascii_case(\"Location", "Name\")"].concat(),
            ["node", ".name", ".eq_ignore_ascii_case(\"Tier", "Level\")"].concat(),
            ["s_", "bioc"].concat(),
            ["s_", "rsi"].concat(),
            ["s_", "aegs"].concat(),
            ["s_", "drak"].concat(),
            ["mockup", "image"].concat(),
            ["i_med_bioc_", "bottom-bar"].concat(),
            ["BG", "Dots"].concat(),
            ["MainMenu", "Canvas"].concat(),
        ];

        for marker in forbidden {
            assert!(
                !source.contains(marker.as_str()),
                "ir_compose hardcoding marker reintroduced: {marker}. This is a hard rule: do not work around this guard by renaming tokens. Keep composition generic for all screens and manageable in scope by fixing the structural root cause instead of reintroducing marker-based hardcoding."
            );
        }
    }


    #[test]
    fn segmented_count_matches_medgel_source_geometry() {
        assert_eq!(segmented_count_for_width(115.0, 3.0, 5.0), 14);
    }


    #[test]
    fn label_caption_pair_stacks_secondary_immediately_below_primary_text_band() {
        let rect = Rect { x: 100.0, y: 20.0, w: 128.0, h: 152.0 };
        let (primary_rect, secondary_rect) = stacked_label_caption_pair_text_rects(
            rect,
            32.0,
            27.0,
            Some(0.5),
            false,
        );

        assert_eq!(primary_rect.y, 47.0);
        assert_eq!(primary_rect.h, 32.0);
        // Pure line-box stack (plan P3.4): the value's line top is one label
        // em below the label's line top — 47 + 32 = 79. (The retired
        // overlap/-8 pair produced 66 only against the retired 1.5-inflated
        // heights; see stacked_label_caption_pair_text_rects.)
        assert_eq!(secondary_rect.y, 79.0);
        assert_eq!(secondary_rect.h, 27.0);
    }


    #[test]
    fn center_anchored_heading_textfield_uses_parent_anchor_text_band() {
        let mut node = blank_node("widget_text_field");
        node.computed_rect = UiIrRect { x: 20.0, y: 25.0, w: 320.0, h: 78.0 };
        node.authored_size = [
            UiIrValue::Fixed { value: 320.0 },
            UiIrValue::Fixed { value: 78.0 },
        ];
        node.anchor = [0.0, -0.12];
        node.pivot = [0.0, 0.0];
        node.text_payload = Some(UiIrTextPayload::Resolved { text: "T3".to_string() });
        node.text_style = Some(UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: UiIrValue::Fixed { value: 41.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: Some(0.5),
            colour: None,
            colour_token: None,
            label_style: Some("Heading1".to_string()),
        });

        let rects = debug_text_rects(&node).expect("text rects");
        assert_eq!(rects.primary.y, 64.0);
        assert_eq!(rects.primary.h, 39.0);
    }


    #[test]
    fn nested_right_pivot_heading_textfield_uses_inline_parent_text_advance() {
        let mut parent = blank_node("widget_text_field");
        parent.id = 1;
        parent.children = vec![2];
        parent.computed_rect = UiIrRect { x: 90.0, y: 25.0, w: 780.0, h: 78.0 };
        parent.text_payload = Some(UiIrTextPayload::Resolved { text: "T3".to_string() });
        parent.text_style = Some(UiIrTextStyle {
            font_record: None,
            resolved_font_record: None,
            font_size: UiIrValue::Fixed { value: 41.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".to_string(),
            vertical_alignment: "Center".to_string(),
            anchor_to_parent_x: Some(0.5),
            anchor_to_parent_y: Some(0.5),
            colour: None,
            colour_token: None,
            label_style: Some("Heading1".to_string()),
        });

        let mut child = blank_node("widget_text_field");
        child.id = 2;
        child.parent_id = Some(1);
        child.anchor = [1.14, 0.0];
        child.pivot = [1.0, 0.0];
        child.computed_rect = UiIrRect { x: 200.0, y: 25.0, w: 780.0, h: 78.0 };
        child.text_payload = Some(UiIrTextPayload::Resolved { text: "INLINE TITLE".to_string() });
        child.text_style = parent.text_style.clone();

        let document = UiIrDocument {
            schema_version: UI_IR_SCHEMA_VERSION,
            canvas_guid: "test-guid".to_string(),
            canvas_name: None,
            target_width: 400,
            target_height: 160,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 100,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![parent, child.clone()],
        };
        let style = stub_style();
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let assets = minimal_swf_assets();
        let ctx = ComposeContext { style: &style, defaults: &defaults, assets: &assets, hologram_fetcher: None };
        let rect = ir_rect_to_layout_rect(child.computed_rect);
        let inline = inline_nested_textfield_text_rect(
            &child,
            rect,
            &document,
            &TextRenderer::new(),
            &ctx,
        )
        .expect("inline rect");

        assert!(inline.x < rect.x, "expected nested title to move left from inflated Auto field");
        assert!(inline.x > 90.0, "expected nested title to remain after parent text");
        assert_eq!(inline.x + inline.w, rect.x + rect.w);
    }
}

#[cfg(test)]
mod tests_e {
    #![allow(unused_imports, dead_code)]

    use super::*;

    use std::collections::HashMap;

    use image::Rgba;

    use crate::bb_atlas::AssetFetcher;
    use crate::canvas::RgbaColor;
    use crate::style::{CrtParams, ManufacturerStyle};
    use crate::ui_ir::{UI_IR_SCHEMA_VERSION, UiRendererHint, UiIrAssetLayout, UiIrCustomShape, UiIrStyleTag, UiIrTextStyle};


    fn text_style_with_font_record(record: serde_json::Value) -> UiIrTextStyle {
        UiIrTextStyle {
            font_record: Some("file://./fontstyles/blenderpro-medium.json".into()),
            resolved_font_record: Some(record),
            font_size: UiIrValue::Fixed { value: 18.0 },
            line_spacing: None,
            letter_spacing: None,
            alignment: "Left".into(),
            vertical_alignment: "Center".into(),
            anchor_to_parent_x: None,
            anchor_to_parent_y: None,
            colour: None,
            colour_token: None,
            label_style: None,
        }
    }


    fn assert_not_uniform(img: &RgbaImage, label: &str) {
        let (w, h) = img.dimensions();
        let mut first: Option<[u8; 4]> = None;
        let mut differing = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                let px = img.get_pixel(x, y).0;
                match first {
                    None => first = Some(px),
                    Some(f) if f != px => differing += 1,
                    _ => {}
                }
            }
        }
        assert!(
            differing > 0,
            "[{label}] image is entirely one colour ({:?})",
            first.unwrap_or([0, 0, 0, 0])
        );
    }


    fn assert_non_background_fraction(
        img: &RgbaImage,
        bg: [u8; 4],
        min_frac: f32,
        label: &str,
    ) {
        let (w, h) = img.dimensions();
        let mut total = 0usize;
        let mut non_bg = 0usize;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                total += 1;
                let p = img.get_pixel(x, y).0;
                let differs = p
                    .iter()
                    .zip(bg.iter())
                    .any(|(a, b)| (*a as i32 - *b as i32).abs() > 16);
                if differs {
                    non_bg += 1;
                }
            }
        }
        let frac = non_bg as f32 / total.max(1) as f32;
        assert!(
            frac >= min_frac,
            "[{label}] only {:.1}% pixels differ from bg; expected >= {:.1}%",
            frac * 100.0,
            min_frac * 100.0,
        );
    }


    struct StubFetcher {
        images: HashMap<String, Vec<u8>>,
    }


    impl AssetFetcher for StubFetcher {
        fn fetch_image_bytes(&self, p4k_path: &str) -> Option<Vec<u8>> {
            self.images.get(&p4k_path.to_ascii_lowercase()).cloned()
        }
    }


    fn stub_style() -> ManufacturerStyle {
        // Real s_drak_hud palette via the provenance fixture (no hard-coded
        // colour values in test source — see test_palettes).
        crate::test_palettes::brand_style("s_drak_hud")
    }


    fn minimal_swf_assets() -> crate::swf_assets::SwfAssetLibrary {
        crate::swf_assets::SwfAssetLibrary::new(vec![
            b'F', b'W', b'S', 6, 21, 0, 0, 0,
            0x00, 0x18, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("minimal swf should parse")
    }


    fn blank_node(node_type: &str) -> UiIrNode {
        UiIrNode {
            id: 1,
            parent_id: None,
            children: Vec::new(),
            node_type: node_type.to_string(),
            name: "node".to_string(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 0.0],
            pivot: [0.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 10.0 },
                UiIrValue::Fixed { value: 10.0 },
            ],
            padding: [0.0, 0.0, 0.0, 0.0],
            margin: [0.0, 0.0, 0.0, 0.0],
            overflow_mode: None,
            clip_rect: None,
            computed_rect: UiIrRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            background_fill_colour: None,
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: None,
            text_style: None,
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: Vec::new(),
            resolved_style_tags: Vec::new(),
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        }
    }


    #[test]
    fn bottom_anchored_progress_meter_uses_label_caption_text_band_bottom() {
        let parent = UiIrNode {
            id: 1,
            parent_id: None,
            children: vec![2],
            node_type: "BuildingBlocks_ComponentLabelCaptionPair".into(),
            name: "pair".into(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [1.0, 0.0],
            pivot: [1.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Other {
                    value: 64.0,
                    behavior: "Auto".into(),
                },
                UiIrValue::Other {
                    value: 64.0,
                    behavior: "Auto".into(),
                },
            ],
            padding: [0.0; 4],
            margin: [0.0; 4],
            overflow_mode: None,
            clip_rect: None,
            computed_rect: UiIrRect { x: 1736.0, y: -5.5, w: 128.0, h: 152.3 },
            background_fill_colour: None,
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: Some(UiIrTextPayload::Resolved {
                text: "MEDGELS".into(),
            }),
            secondary_text_payload: Some(UiIrTextPayload::Resolved {
                text: "200/200".into(),
            }),
            secondary_text_style: Some(UiIrTextStyle {
                font_record: None,
                resolved_font_record: None,
                font_size: UiIrValue::Fixed { value: 28.0 },
                line_spacing: None,
                letter_spacing: None,
                alignment: "Left".into(),
                vertical_alignment: "Center".into(),
                anchor_to_parent_x: None,
                anchor_to_parent_y: None,
                colour: None,
                colour_token: None,
                label_style: None,
            }),
            meter_progress: None,
            text_style: Some(UiIrTextStyle {
                font_record: None,
                resolved_font_record: None,
                font_size: UiIrValue::Fixed { value: 32.0 },
                line_spacing: None,
                letter_spacing: None,
                alignment: "Left".into(),
                vertical_alignment: "Center".into(),
                anchor_to_parent_x: Some(0.0),
                anchor_to_parent_y: Some(0.5),
                colour: None,
                colour_token: None,
                label_style: None,
            }),
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: vec![],
            resolved_style_tags: vec![],
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        };
        let meter = UiIrNode {
            id: 2,
            parent_id: Some(1),
            children: vec![],
            node_type: "BuildingBlocks_WidgetLinearProgressMeter".into(),
            name: "meter".into(),
            is_active: true,
            layer: 0,
            alpha: 1.0,
            anchor: [0.0, 1.0],
            pivot: [0.0, 0.0],
            rotation_deg: None,
            authored_position: [0.0, 0.0],
            authored_size: [
                UiIrValue::Fixed { value: 115.0 },
                UiIrValue::Fixed { value: 15.0 },
            ],
            padding: [0.0; 4],
            margin: [0.0; 4],
            overflow_mode: None,
            clip_rect: None,
            computed_rect: UiIrRect { x: 1736.0, y: 146.8, w: 115.0, h: 15.0 },
            background_fill_colour: None,
            corner_radius: None,
            corner_radii: None,
            corner_chamfers: None,
            background_fill_alpha: None,
            background_fill_colour_token: None,
            circle_fill_colour_token: None,
            style_provenance: None,
            segmented_fill: None,
            polygon: None,
            border: None,
            stroke_colour: None,
            stroke_colour_token: None,
            stroke_extent: None,
            separator_strip: None,
            colour_blend_mode: None,
            icon_tint_colour: None,
            icon_tint_colour_token: None,
            icon_preset: None,
            text_payload: None,
            secondary_text_payload: None,
            secondary_text_style: None,
            meter_progress: Some(1.0),
            text_style: None,
            asset_ref: None,
            primitive_material: None,
            primitive_uv_start: None,
            primitive_uv_size: None,
            asset_layout: None,
            custom_shape: None,
            style_tag_uuids: vec![],
            resolved_style_tags: vec![],
            is_flash_renderer: false,
            auto_font_size: false,
            colour_overlay_enabled: false,
        };
        let document = UiIrDocument {
            schema_version: 1,
            canvas_guid: "test-canvas".into(),
            canvas_name: None,
            target_width: 1920,
            target_height: 1080,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: crate::ui_ir::UiRendererHint::Bb,
            confidence: 100,
            warnings: vec![],
            unresolved_references: vec![],
            resolved_asset_refs: vec![],
            missing_asset_refs: vec![],
            nodes: vec![parent, meter.clone()],
        };

        let rect = debug_linear_progress_meter_rect(&meter, &document).expect("meter rect");
        let parent_text_rects = debug_text_rects(&document.nodes[0]).expect("parent text rects");
        let parent_drawn_bounds_early = debug_text_drawn_bounds(&document.nodes[0]).expect("parent drawn bounds");
        // The meter pins to drawn glyph bottom (not line-box bottom) to avoid
        // pushing the bar down by the empty descent space on non-descender text.
        let expected_y = parent_drawn_bounds_early
            .secondary
            .map(|secondary_drawn| secondary_drawn.y + secondary_drawn.h)
            .or_else(|| parent_text_rects.secondary.map(|r| r.y + r.h))
            .unwrap_or_else(|| parent_text_rects.primary.y + parent_text_rects.primary.h);
        assert!(
            (rect.y - expected_y).abs() < 0.1,
            "expected meter to attach to drawn secondary bottom {}, got {}",
            expected_y,
            rect.y
        );

        let parent_drawn_bounds = debug_text_drawn_bounds(&document.nodes[0]).expect("parent text bounds");
        let drawn_padded_y = match (parent_text_rects.secondary, parent_drawn_bounds.secondary) {
            (Some(secondary_rect), Some(secondary_drawn)) => {
                secondary_rect.y + secondary_rect.h + (secondary_rect.h - secondary_drawn.h)
            }
            _ => parent_text_rects.primary.y + parent_text_rects.primary.h,
        };
        assert!(
            rect.y < drawn_padded_y,
            "expected text-band placement to avoid fallback drawn-bounds padding"
        );
    }

    /// `rounded_rect_path` must render a CIRCLE (not a squircle) when the corner
    /// radius reaches half the size — the g-force / velocity centre dot is exactly
    /// this (a 114.86² node with corner_radius 100, clamped to half-size). The
    /// prior quadratic-Bezier corners bulged toward the corners, rendering a
    /// rounded square ~8% larger in area than a circle (the owner flagged the
    /// g-force centre dot as a rounded square). Cubic-Bezier corners with the
    /// standard 90°-arc constant bring it to a true circle.
    #[test]
    fn rounded_rect_full_radius_renders_a_circle() {
        let n = 120u32;
        let rect = TskRect::from_xywh(0.0, 0.0, n as f32, n as f32).expect("rect");
        let path = rounded_rect_path(rect, n as f32 * 0.5).expect("path");
        let mut pm = tiny_skia::Pixmap::new(n, n).expect("pixmap");
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(tiny_skia::Color::WHITE);
        paint.anti_alias = true;
        pm.as_mut().fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
        let opaque = pm.pixels().iter().filter(|p| p.alpha() > 128).count() as f32;
        let circle = std::f32::consts::PI * (n as f32 * 0.5).powi(2);
        let ratio = opaque / circle;
        assert!(
            ratio < 1.04,
            "full-radius rounded rect must render ~circular: filled {opaque:.0}px vs circle \
             {circle:.0}px (ratio {ratio:.3}); ratio >1.04 means a squircle (a square is 1.27)"
        );
    }
}


/// A border drawable as one rounded stroke: all four sides share one
/// width (> 0) and one resolved colour.
fn uniform_border_style(
    border: &UiIrBorder,
    ctx: &ComposeContext<'_>,
) -> Option<(f32, [f32; 4])> {
    let width = border.top.width;
    if width <= 0.0 {
        return None;
    }
    let sides = [&border.right, &border.bottom, &border.left];
    if sides.iter().any(|side| (side.width - width).abs() > 0.01) {
        return None;
    }
    let colour = border_side_colour(&border.top, ctx)?;
    for side in sides {
        if border_side_colour(side, ctx) != Some(colour) {
            return None;
        }
    }
    Some((width, colour))
}

/// Stroke a uniform border as a rounded rect inset by half the border width
/// (the stroke stays inside the node rect like the per-side fills do).
/// Returns `false` when the border is not expressible as one rounded stroke,
/// so the caller can fall back to per-side drawing.
pub(crate) fn draw_rounded_uniform_border(
    pixmap: &mut Pixmap,
    rect: Rect,
    border: &UiIrBorder,
    radius: f32,
    alpha: f32,
    ctx: &ComposeContext<'_>,
) -> bool {
    let Some((width, colour)) = uniform_border_style(border, ctx) else {
        return false;
    };
    let inset = width * 0.5;
    let Some(tsk_rect) = TskRect::from_xywh(
        rect.x + inset,
        rect.y + inset,
        (rect.w - width).max(1.0),
        (rect.h - width).max(1.0),
    ) else {
        return false;
    };
    let Some(path) = rounded_rect_path(tsk_rect, radius) else {
        return false;
    };

    let mut paint = Paint::default();
    paint.set_color(to_skia_color(colour, alpha));
    paint.anti_alias = true;
    let mut stroke = Stroke::default();
    stroke.width = width;
    pixmap
        .as_mut()
        .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    true
}
