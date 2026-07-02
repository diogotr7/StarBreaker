#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use sha2::{Digest, Sha256};
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use crate::bb_bindings::BindingResolver;
#[allow(unused_imports)]
use crate::bb_layout;
#[allow(unused_imports)]
use crate::bb_layout::{LayoutResult, Rect};
#[allow(unused_imports)]
use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene, BbValue};
#[allow(unused_imports)]
use crate::defaults::DefaultValueRegistry;
#[allow(unused_imports)]
use crate::pipeline::CanvasFetcher;

// Consolidated engine chunk 02 (formerly: part_07.part, part_08.part, part_09.part, part_09b.part, part_09c.part, part_10.part).
//   part_09b.part: Read a node's raw `fontSize`/`FontSize` (authored field, applied style
//   part_09c.part: Typography brand-palette helpers: the authoritative `BB_ColorStyle` enum

pub(crate) fn border_from_node(node: &BbNode, design_text_scale: f32) -> Option<UiIrBorder> {
    let border = node.border.as_ref()?;
    // Border widths are stage-unit properties like font sizes: on the MFD
    // frame path they pick up the host-stage view scale (the footer's 2px
    // Base rule renders ~3.3px on the 1600×1200 RTT); elsewhere the scale is
    // 1.0. Geometry rects reflow through the layout and are unscaled.
    let width = |value: f32| value * design_text_scale;
    Some(UiIrBorder {
        top: UiIrBorderSide {
            width: width(border.top.width),
            colour: border.top.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Top"),
        },
        right: UiIrBorderSide {
            width: width(border.right.width),
            colour: border.right.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Right"),
        },
        bottom: UiIrBorderSide {
            width: width(border.bottom.width),
            colour: border.bottom.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Bottom"),
        },
        left: UiIrBorderSide {
            width: width(border.left.width),
            colour: border.left.colour,
            colour_token: border_colour_token_from_raw(&node.raw, "Left"),
        },
    })
}

fn border_colour_token_from_raw(raw: &serde_json::Value, side: &str) -> Option<String> {
    raw.get(format!("BorderColor{side}Token"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            raw.get("BorderColorToken")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            raw.get("border")
                .and_then(|border| border.get(side.to_ascii_lowercase()))
                .and_then(|value| value.get("color"))
                .and_then(colour_style_token)
        })
}

pub(crate) fn stroke_colour_from_raw(raw: &serde_json::Value) -> Option<[f32; 4]> {
    let obj = raw.get("StrokeColor")?.as_object()?;
    let r = obj.get("r").and_then(|v| v.as_f64())? as f32;
    let g = obj.get("g").and_then(|v| v.as_f64())? as f32;
    let b = obj.get("b").and_then(|v| v.as_f64())? as f32;
    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}

/// Extract a `WidgetPolygon`'s regular-polygon shape.
pub(crate) fn polygon_from_raw(node: &crate::bb_scene::BbNode) -> Option<UiIrPolygon> {
    if !matches!(&node.ty, crate::bb_scene::BbNodeType::Other(kind)
        if kind.eq_ignore_ascii_case("BuildingBlocks_WidgetPolygon"))
    {
        return None;
    }
    let raw = &node.raw;
    let sides = raw.get("sides").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let start_angle_deg = raw
        .get("startAngle")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let rotation_deg = raw
        .get("orientationOffset")
        .and_then(|o| o.get("z"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let do_fill = raw.get("doFill").and_then(|v| v.as_bool()).unwrap_or(true);
    let fill = raw.get("fillColor");
    let fill_colour_token = fill
        .and_then(|c| c.get("color"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let fill_alpha = fill
        .and_then(|c| c.get("alpha"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;
    Some(UiIrPolygon {
        sides,
        start_angle_deg,
        rotation_deg,
        do_fill,
        fill_colour_token,
        fill_alpha,
    })
}

/// `WidgetCircle` solid fill: its `fillColor` ColorStyle token + alpha, captured
/// ONLY when `doFill` is EXPLICITLY true (the g-force ball's `circle_Cap*` solid
/// dots — `fillColor: { color: "Base" }`, `doFill: true`). `doFill` defaults to
/// false here so outline-only circles (the rings, drawn via stroke) are left
/// untouched; the renderer fills the circle with this surface token when present.
pub(crate) fn circle_fill_token_from_raw(node: &crate::bb_scene::BbNode) -> Option<String> {
    if !matches!(&node.ty, crate::bb_scene::BbNodeType::Other(kind)
        if kind.eq_ignore_ascii_case("BuildingBlocks_WidgetCircle"))
    {
        return None;
    }
    let raw = &node.raw;
    if !raw.get("doFill").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    raw.get("fillColor")
        .and_then(|c| c.get("color"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn segmented_fill_from_raw(node: &crate::bb_scene::BbNode) -> Option<UiIrSegmentedFill> {
    let raw = &node.raw;
    let segmented_raw = raw.get("segmentedFill");

    let enabled = raw
        .get("EnableSegmentedFill")
        .and_then(|v| v.as_bool())
        .or_else(|| segmented_raw.and_then(|sf| sf.get("enable")).and_then(|v| v.as_bool()))
        .unwrap_or(false);

    if !enabled {
        return None;
    }

    let angle = raw
        .get("SegmentAngle")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("angle"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .or_else(|| {
            node.background
                .as_ref()
                .and_then(|bg| bg.segmented_fill.as_ref())
                .map(|fill| fill.angle)
        })
        .unwrap_or(0.0);

    let segment_size = raw
        .get("SegmentSize")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("segmentSize"))
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .unwrap_or(64.0);

    let segment_spacing_size = raw
        .get("SegmentSpacingSize")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("spaceSize"))
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .unwrap_or(64.0);

    let segment_x_offset = raw
        .get("SegmentXOffset")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("xOffset"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
        })
        .unwrap_or(0.0);

    let segmented_bar_fill = raw
        .get("EnableSegmentedBarFill")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("barFill"))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);

    let segment_colour = raw
        .get("SegmentColor")
        .and_then(parse_raw_colour)
        .or_else(|| segmented_raw.and_then(|sf| sf.get("segmentColor")).and_then(parse_raw_colour));

    let segment_colour_token = raw
        .get("SegmentColor")
        .and_then(|v| v.get("color"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            segmented_raw
                .and_then(|sf| sf.get("segmentColor"))
                .and_then(|v| v.get("color"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });

    if segment_size <= 0.0 {
        return None;
    }

    Some(UiIrSegmentedFill {
        enabled,
        angle,
        segment_size,
        segment_spacing_size,
        segment_x_offset,
        segmented_bar_fill,
        segment_colour,
        segment_colour_token,
    })
}

pub(crate) fn parse_raw_colour(value: &serde_json::Value) -> Option<[f32; 4]> {
    let r = value.get("r").and_then(|v| v.as_f64())? as f32;
    let g = value.get("g").and_then(|v| v.as_f64())? as f32;
    let b = value.get("b").and_then(|v| v.as_f64())? as f32;
    let a = value.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        Some([r / 255.0, g / 255.0, b / 255.0, a / 255.0])
    } else {
        Some([r, g, b, a])
    }
}

pub(crate) fn parse_nine_slice_rect(value: &serde_json::Value) -> Option<[f32; 4]> {
    let left = value.get("left")?.as_f64()? as f32;
    let top = value.get("top")?.as_f64()? as f32;
    let right = value.get("right")?.as_f64()? as f32;
    let bottom = value.get("bottom")?.as_f64()? as f32;
    Some([left, top, right, bottom])
}

pub(crate) fn separator_stroke_extent_from_raw(node: &crate::bb_scene::BbNode) -> Option<f32> {
    let stroke_extent = node
        .raw
        .get("strokeExtent")
        .or_else(|| node.raw.get("svgFill").and_then(|svg_fill| svg_fill.get("strokeExtent")))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32);

    if is_authored_procedural_separator_strip(node) {
        return None;
    }

    stroke_extent
}

fn is_authored_procedural_separator_strip(node: &crate::bb_scene::BbNode) -> bool {
    if !matches!(node.ty, BbNodeType::Other(ref ty) if ty.eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")) {
        return false;
    }

    let Some(svg_fill) = node.raw.get("svgFill") else {
        return false;
    };
    let svg_path_empty = svg_fill
        .get("svgPath")
        .and_then(|value| value.as_str())
        .is_none_or(|path| path.trim().is_empty());
    if !svg_path_empty {
        return false;
    }
    if !svg_fill
        .get("renderShape")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    if !svg_fill
        .get("enableColorOverlay")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    if !svg_fill
        .get("enableNineSliceRect")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return false;
    }

    let authored_fixed_height = node.raw.get("sizing").and_then(|sizing| sizing.get("height"));
    let is_fixed_sixteen_px = authored_fixed_height
        .and_then(|height| height.get("behavior"))
        .and_then(|value| value.as_str())
        .is_some_and(|behavior| behavior.eq_ignore_ascii_case("Fixed"))
        && authored_fixed_height
            .and_then(|height| height.get("value"))
            .and_then(|value| value.as_f64())
            .is_some_and(|value| (value - 16.0).abs() <= f64::EPSILON);

    is_fixed_sixteen_px
}

pub(crate) fn separator_colour_blend_mode_from_raw(
    node: &crate::bb_scene::BbNode,
) -> Option<UiIrColourBlendMode> {
    if !matches!(node.ty, BbNodeType::Other(ref ty) if ty.eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")) {
        return None;
    }

    let svg_fill = node.raw.get("svgFill")?;
    let svg_path_empty = svg_fill
        .get("svgPath")
        .and_then(|value| value.as_str())
        .is_none_or(|path| path.trim().is_empty());
    let render_shape = svg_fill
        .get("renderShape")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let enable_color_overlay = svg_fill
        .get("enableColorOverlay")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    (svg_path_empty && render_shape && enable_color_overlay).then_some(UiIrColourBlendMode::Additive)
}

pub(crate) fn background_colour_blend_mode_from_raw(
    node: &crate::bb_scene::BbNode,
    colour_token: Option<&str>,
    allow_background_fill: bool,
) -> Option<UiIrColourBlendMode> {
    if !allow_background_fill || !matches!(node.ty, BbNodeType::DisplayWidget) {
        return None;
    }
    let token = colour_token?.trim();
    if !token.to_ascii_lowercase().starts_with("accent") {
        return None;
    }

    let has_authored_background_colour = node
        .raw
        .get("background")
        .and_then(|background| background.get("color"))
        .is_some_and(|value| !value.is_null())
        || node.raw.get("BackgroundColor").is_some()
        || node.raw.get("FillColor").is_some();

    has_authored_background_colour.then_some(UiIrColourBlendMode::Additive)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SeparatorStyleSource {
    pub(crate) colour: Option<[f32; 4]>,
    pub(crate) colour_token: Option<String>,
    pub(crate) colour_alpha: Option<f32>,
    pub(crate) alpha_override: Option<f32>,
    /// The brand's separator SVG (e.g. DRAK_S42_seperator_vertical_2.svg) and
    /// its nine-slice / flip so the dotted glyph rasterises via the asset_ref
    /// path. Only resolved for MFD-frame hosts (see the caller's gate) so the
    /// physical medical screens keep their byte-identical no-separator render.
    pub(crate) svg_path: Option<String>,
    pub(crate) nine_slice_rect: Option<[f32; 4]>,
    pub(crate) enable_color_overlay: Option<bool>,
}

pub(crate) fn separator_standard_style_from_source(
    node: &crate::bb_scene::BbNode,
    selected_style_source: Option<&str>,
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    mfd_frame: bool,
) -> Option<SeparatorStyleSource> {
    if !matches!(node.ty, BbNodeType::Other(ref ty) if ty.eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")) {
        return None;
    }

    let direction = node
        .raw
        .get("direction")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let style = node
        .raw
        .get("style")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    let record_name = match (
        direction.to_ascii_lowercase().as_str(),
        style.to_ascii_lowercase().as_str(),
    ) {
        ("horizontal", "primary") => "BuildingBlocks_Canvas.HorizontalSeparatorPrimaryWidgetStandard",
        ("horizontal", "secondary") => "BuildingBlocks_Canvas.HorizontalSeparatorSecondaryWidgetStandard",
        ("horizontal", "tertiary") => "BuildingBlocks_Canvas.HorizontalSeparatorTertiaryWidgetStandard",
        ("vertical", "primary") => "BuildingBlocks_Canvas.VerticalSeparatorPrimaryWidgetStandard",
        ("vertical", "secondary") => "BuildingBlocks_Canvas.VerticalSeparatorSecondaryWidgetStandard",
        ("vertical", "tertiary") => "BuildingBlocks_Canvas.VerticalSeparatorTertiaryWidgetStandard",
        _ => return None,
    };

    // Colour/alpha match the canvas's DIRECT style slug (the existing
    // behaviour: `canvas:s_bioc` -> `s_bioc`). The dotted SVG additionally
    // resolves the brand by manufacturer family (`manufacturer:drak` ->
    // `s_drak_env`) but ONLY on MFD frames — so physical interior screens
    // (medical, door) match nothing here and keep their byte-identical
    // no-separator render. The frozen medical platinum is never touched.
    let mut match_slugs = match style_source_slug(selected_style_source) {
        Some(slug) => vec![slug],
        None => Vec::new(),
    };
    if mfd_frame {
        match_slugs.extend(separator_brand_candidate_slugs(selected_style_source));
    }
    if match_slugs.is_empty() {
        return None;
    }
    let standard = canvas_fetcher?.fetch_canvas_by_name(record_name).ok()?;
    let mut source = separator_style_from_standard_record(&standard, &match_slugs, mfd_frame)?;
    if mfd_frame {
        // The per-ship modularkit SVG override (e.g. `DRAK_S42_…`) is absent from
        // this build for most ships, so the engine paints the standard's DEFAULT
        // MFD divider — `PU_MFD_Generic_…_V_Divider` (a column of dots that scales
        // to the rect, no nine-slice). Prefer it over the (usually missing) brand
        // override so the dots actually render.
        if let Some(default_svg) = separator_default_svg_path(&standard) {
            source.svg_path = Some(default_svg);
            source.nine_slice_rect = None;
        }
    }
    Some(source)
}

/// The standard's DEFAULT (non-brand) `SvgPath` — the base MFD divider the
/// engine falls back to when a ship has no modularkit separator vector. Scans
/// outside `brandStyles`.
fn separator_default_svg_path(standard: &serde_json::Value) -> Option<String> {
    fn scan(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Object(map) => {
                if map.get("_Type_").and_then(|t| t.as_str()) == Some("BuildingBlocks_BrandStyles") {
                    return None;
                }
                if modifier_field_name(value) == Some("SvgPath")
                    && let Some(path) = value.get("value").and_then(|v| v.as_str())
                    && !path.trim().is_empty()
                {
                    return Some(path.trim().to_string());
                }
                for (key, child) in map {
                    if key == "brandStyles" {
                        continue;
                    }
                    if let Some(found) = scan(child) {
                        return Some(found);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => items.iter().find_map(scan),
            _ => None,
        }
    }
    scan(standard.get("_RecordValue_")?)
}

/// The standard's brand slug for a separator is `s_<mfr>_env` (the env sibling
/// the modularkit authors the dotted glyph under), derived from the canvas's
/// selected style source (`manufacturer:drak` or a `s_drak_hud` style-link).
/// Returns env/hud/bare candidates in priority order.
fn separator_brand_candidate_slugs(selected_style_source: Option<&str>) -> Vec<String> {
    let Some(slug) = style_source_slug(selected_style_source) else {
        return Vec::new();
    };
    let mfr = if let Some(rest) = slug.strip_prefix("manufacturer:") {
        rest.to_string()
    } else if let Some(rest) = slug.strip_prefix("s_") {
        rest.split('_').next().unwrap_or(rest).to_string()
    } else {
        return vec![slug];
    };
    if mfr.is_empty() {
        return Vec::new();
    }
    // The ship brand first, then `orig` — the modularkit default the engine
    // falls back to when a ship has no separator vector of its own (DRAK ships
    // none in this build).
    vec![
        format!("s_{mfr}_env"),
        format!("s_{mfr}_hud"),
        format!("s_{mfr}"),
        "orig".to_string(),
    ]
}

fn separator_style_from_standard_record(
    standard: &serde_json::Value,
    candidate_slugs: &[String],
    extract_svg: bool,
) -> Option<SeparatorStyleSource> {
    let brand_styles = standard.get("_RecordValue_")?.get("brandStyles")?.as_array()?;
    // Honour candidate priority (env before hud before bare): the first
    // candidate with a brand entry that yields a style wins.
    for candidate in candidate_slugs {
    for brand_style in brand_styles {
        let brand_identifier = brand_style.get("brandIdentifier").and_then(|value| value.as_str());
        if style_source_slug(brand_identifier).as_deref() != Some(candidate.as_str()) {
            continue;
        }

        let mut source = SeparatorStyleSource::default();
        for modifier in brand_style
            .get("entries")
            .and_then(|entries| entries.as_array())
            .into_iter()
            .flatten()
            .flat_map(|entry| entry.get("modifiers").and_then(|modifiers| modifiers.as_array()))
            .flatten()
        {
            let field = modifier_field_name(modifier).unwrap_or_default();
            if field.eq_ignore_ascii_case("Alpha") {
                source.alpha_override = modifier
                    .get("value")
                    .and_then(|value| value.as_f64())
                    .map(|value| (value as f32).clamp(0.0, 1.0));
                continue;
            }
            if field.eq_ignore_ascii_case("SvgPath") {
                if extract_svg {
                    source.svg_path = modifier
                        .get("value")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                        .map(str::to_owned);
                }
                continue;
            }
            if field.eq_ignore_ascii_case("EnableColorOverlay") {
                source.enable_color_overlay =
                    Some(modifier.get("value").and_then(|value| value.as_bool()).unwrap_or(false));
                continue;
            }
            // Nine-slice fractions (SVG-only): the standard authors the inner
            // band that tiles the dots, e.g. drak vertical = Top 0.49 / Bottom
            // 0.51. Colour/alpha below stay unconditional.
            if extract_svg
                && let Some(slot) = match field.to_ascii_lowercase().as_str() {
                    "ninesliceleft" => Some(0usize),
                    "nineslicetop" => Some(1),
                    "ninesliceright" => Some(2),
                    "nineslicebottom" => Some(3),
                    _ => None,
                }
            {
                if let Some(value) = modifier.get("value").and_then(|value| value.as_f64()) {
                    let rect = source.nine_slice_rect.get_or_insert([0.0, 0.0, 1.0, 1.0]);
                    rect[slot] = value as f32;
                }
                continue;
            }

            if !field.eq_ignore_ascii_case("FillColor")
                && !field.eq_ignore_ascii_case("BackgroundColor")
            {
                continue;
            }

            let Some(color_value) = modifier.get("color") else {
                continue;
            };
            source.colour = parse_raw_colour(color_value);
            source.colour_token = colour_style_token(color_value);
            source.colour_alpha = color_value
                .get("alpha")
                .and_then(|value| value.as_f64())
                .map(|value| (value as f32).clamp(0.0, 1.0))
                .or_else(|| source.colour.map(|colour| colour[3]));
        }

        if source.colour.is_some()
            || source.colour_token.is_some()
            || source.alpha_override.is_some()
            || source.svg_path.is_some()
        {
            return Some(source);
        }
    }
    }

    None
}

fn style_source_slug(source: Option<&str>) -> Option<String> {
    let source = source?.trim();
    if source.is_empty() {
        return None;
    }

    let source = source.strip_prefix("canvas:").unwrap_or(source);
    let basename = source.rsplit(['/', '\\']).next().unwrap_or(source);
    let slug = basename.strip_suffix(".json").unwrap_or(basename).to_ascii_lowercase();
    (!slug.is_empty()).then_some(slug)
}

fn modifier_field_name(modifier: &serde_json::Value) -> Option<&str> {
    modifier
        .get("field")
        .and_then(|field| field.as_str().or_else(|| field.get("value").and_then(|value| value.as_str())))
}

pub(crate) fn stroke_colour_token_from_raw(raw: &serde_json::Value) -> Option<String> {
    raw.get("StrokeColorToken")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| raw.get("StrokeColor").and_then(colour_style_token))
}

pub(crate) fn icon_tint_colour_token_from_raw(raw: &serde_json::Value, allow_fill_colour: bool) -> Option<String> {
    raw.get("iconProperties")
        .and_then(|properties| properties.get("color"))
        .and_then(colour_style_token)
        .or_else(|| svg_fill_overlay_colour_token_from_raw(raw))
        .or_else(|| {
            allow_fill_colour.then(|| {
                raw.get("FillColorToken")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(str::to_owned)
            }).flatten()
        })
        .or_else(|| {
            allow_fill_colour.then(|| raw.get("FillColor").and_then(colour_style_token)).flatten()
        })
}

pub(crate) fn svg_fill_overlay_colour_from_raw(raw: &serde_json::Value) -> Option<[f32; 4]> {
    let svg_fill = raw.get("svgFill")?;
    let render_shape = svg_fill
        .get("renderShape")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let enable_color_overlay = svg_fill
        .get("enableColorOverlay")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !render_shape || !enable_color_overlay {
        return None;
    }

    parse_raw_colour(svg_fill.get("color")?)
}

fn svg_fill_overlay_colour_token_from_raw(raw: &serde_json::Value) -> Option<String> {
    let svg_fill = raw.get("svgFill")?;
    let render_shape = svg_fill
        .get("renderShape")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let enable_color_overlay = svg_fill
        .get("enableColorOverlay")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !render_shape || !enable_color_overlay {
        return None;
    }

    svg_fill.get("color").and_then(colour_style_token)
}

pub(crate) fn svg_fill_overlay_alpha_from_raw(raw: &serde_json::Value) -> Option<f32> {
    let svg_fill = raw.get("svgFill")?;
    let render_shape = svg_fill
        .get("renderShape")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let enable_color_overlay = svg_fill
        .get("enableColorOverlay")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !render_shape || !enable_color_overlay {
        return None;
    }

    svg_fill
        .get("color")
        .and_then(|value| value.get("alpha"))
        .and_then(|value| value.as_f64())
        .map(|value| (value as f32).clamp(0.0, 1.0))
}

pub(crate) fn text_colour_token_from_raw(raw: &serde_json::Value) -> Option<String> {
    raw.get("FillColorToken")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| raw.get("color").and_then(colour_style_token))
        .or_else(|| raw.get("textColor").and_then(colour_style_token))
        .or_else(|| raw.get("textColour").and_then(colour_style_token))
        .or_else(|| raw.get("FillColor").and_then(colour_style_token))
}

pub(crate) fn auto_font_size_enabled(raw: &serde_json::Value) -> bool {
    raw.get("autoFontSize")
        .or_else(|| raw.get("AutoFontSize"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// True when a text field is a HEIGHT-DRIVEN label: its HEIGHT is a `Percent` of
/// the parent AND its WIDTH is `PercentOfY` (derived FROM that height). Such a
/// field's whole box is defined by its height, so the engine sizes the glyph to
/// fill it rather than falling back to the named-style table default. The compass
/// tick labels are the case (`width: 10 PercentOfY`, `height: 0.8`, no FontSize) —
/// they fill the tick. A field with a Percent height but a normal (Percent /
/// Fixed) width is a width-laid-out label that keeps its named-style size (the
/// medical-bed `ui_target_a` titles: Percent height, NOT PercentOfY width → their
/// frozen 25/30/60 sizes are preserved).
fn text_field_sizes_font_to_relative_height(node: &crate::bb_scene::BbNode) -> bool {
    use crate::bb_scene::BbValue;
    matches!(node.ty, BbNodeType::WidgetTextField | BbNodeType::WidgetText)
        && matches!(node.sizing.height, BbValue::Percent(_))
        && matches!(
            node.sizing.width,
            BbValue::Other { ref behavior, .. } if behavior.eq_ignore_ascii_case("PercentOfY")
        )
}


/// The heading→`Base` semantic guesses that used to live here were DELETED
/// 2026-06-12: with the text-format style route landed, every frozen pin's
/// text colour is authored-entry-driven — disabling them drifts nothing
/// (remediation plan Phase 3 audit). Only the explicit colour-role tag
/// directive below survives.
pub(crate) fn semantic_text_colour_token_from_style_tags(
    tags: &[UiIrStyleTag],
    label_style: Option<&str>,
) -> Option<String> {
    node_colour_directive_token(tags, label_style)
}

/// A node-level colour DIRECTIVE from the node's tags — an authoring-time
/// colour instruction.
pub(crate) fn node_colour_directive_token(
    tags: &[UiIrStyleTag],
    label_style: Option<&str>,
) -> Option<String> {

    let style_name = label_style
        .map(str::trim)
        .unwrap_or_default();
    let is_title_style = !style_name.is_empty() && style_name.starts_with("Title");
    if !is_title_style {
        return None;
    }

    // A colour-ROLE tag (`Bright` — the tag literally names a BB_ColorStyle
    // enum role) authored on a Title-style node is an authoring-time colour
    // instruction (the medical end-of-bed tier label pins it). The former
    // state-tag arms (StateModerate→Base, StateCritical→Background,
    // Primary→Base) were DELETED 2026-06-12: with the text-format style
    // route landed, every frozen pin's colour is entry-driven — disabling
    // them drifts nothing (remediation plan Phase 3 audit).
    if tags.iter().any(|tag| tag.tag_name.as_deref() == Some("Bright")) {
        return Some("Bright".to_string());
    }
    None
}

/// Style tags authored on the node itself (its own `styleTags` plus its own
/// `PrimaryStateTag`), WITHOUT ancestor inheritance.
///
/// Use this where a tag should reflect the node's own intent rather than an
/// inherited container tag — notably icon/shape tinting: a raster image must not
/// be tinted just because an ancestor carries an accent tag like `Primary`.
pub(crate) fn own_style_tags_for_node(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    node: &crate::bb_scene::BbNode,
    node_id: BbNodeId,
    binding_resolver: &BindingResolver,
    defaults: &DefaultValueRegistry,
) -> Vec<UiIrStyleTag> {
    let mut resolved = Vec::new();

    push_style_tags_from_raw(canvas_fetcher, node.raw.get("styleTags"), &mut resolved);

    if let Some(primary_state_tag) = binding_resolver.resolve_field_text(node_id, "PrimaryStateTag", defaults)
        && let Some(tag_uuid) = parse_tag_uuid_from_reference(&primary_state_tag)
        && !resolved.iter().any(|tag| tag.uuid == tag_uuid)
    {
        let tag_ref = serde_json::json!({
            "_RecordId_": tag_uuid,
            "_RecordName_": primary_state_tag,
        });
        let resolved_record = resolve_style_tag_record(
            canvas_fetcher,
            &tag_ref,
            "",
            &primary_state_tag,
            &tag_uuid,
        );
        let tag_name = resolved_record
            .as_ref()
            .and_then(|record| record.get("_RecordValue_"))
            .and_then(|v| v.get("tagName"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        resolved.push(UiIrStyleTag {
            uuid: tag_uuid,
            tag_name,
        });
    }

    resolved
}

pub(crate) fn resolved_style_tags_for_node(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    scene: &crate::bb_scene::BbScene,
    node: &crate::bb_scene::BbNode,
    node_id: BbNodeId,
    binding_resolver: &BindingResolver,
    defaults: &DefaultValueRegistry,
) -> Vec<UiIrStyleTag> {
    let mut resolved = own_style_tags_for_node(canvas_fetcher, node, node_id, binding_resolver, defaults);

    let mut ancestor_id = node.parent;
    while let Some(parent_id) = ancestor_id {
        let parent_node = scene.nodes.get(&parent_id);
        if let Some(parent_node) = parent_node {
            push_style_tags_from_raw(canvas_fetcher, parent_node.raw.get("styleTags"), &mut resolved);
        }
        if let Some(parent_primary_state_tag) =
            binding_resolver.resolve_field_text(parent_id, "PrimaryStateTag", defaults)
            && let Some(tag_uuid) = parse_tag_uuid_from_reference(&parent_primary_state_tag)
                && !resolved.iter().any(|tag| tag.uuid == tag_uuid)
        {
            let tag_ref = serde_json::json!({
                "_RecordId_": tag_uuid,
                "_RecordName_": parent_primary_state_tag,
            });
            let resolved_record = resolve_style_tag_record(
                canvas_fetcher,
                &tag_ref,
                "",
                &parent_primary_state_tag,
                &tag_uuid,
            );
            let tag_name = resolved_record
                .as_ref()
                .and_then(|record| record.get("_RecordValue_"))
                .and_then(|v| v.get("tagName"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            resolved.push(UiIrStyleTag {
                uuid: tag_uuid,
                tag_name,
            });
        }

        ancestor_id = parent_node.and_then(|parent| parent.parent);
    }

    resolved
}

fn push_style_tags_from_raw(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    raw_tags: Option<&serde_json::Value>,
    resolved: &mut Vec<UiIrStyleTag>,
) {
    let Some(tags) = raw_tags.and_then(|v| v.as_array()) else {
        return;
    };

    for tag in tags {
        let Some(uuid) = tag
            .get("_RecordId_")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };

        if resolved.iter().any(|existing| existing.uuid == uuid) {
            continue;
        }

        let resolved_record = resolve_style_tag_record(
            canvas_fetcher,
            tag,
            tag.get("_RecordPath_")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(""),
            tag.get("_RecordName_")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(""),
            &uuid,
        );

        let tag_name = resolved_record
            .as_ref()
            .and_then(|record| record.get("_RecordValue_"))
            .and_then(|v| v.get("tagName"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        resolved.push(UiIrStyleTag { uuid, tag_name });
    }
}

fn parse_tag_uuid_from_reference(reference: &str) -> Option<String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = trimmed
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .trim();
    let is_uuid = candidate.len() == 36
        && candidate
            .chars()
            .enumerate()
            .all(|(i, ch)| match i {
                8 | 13 | 18 | 23 => ch == '-',
                _ => ch.is_ascii_hexdigit(),
            });
    is_uuid.then(|| candidate.to_ascii_lowercase())
}

pub(crate) fn default_style_text_colour_token_from_raw(
    raw: &serde_json::Value,
    node_type: &BbNodeType,
    is_secondary: bool,
) -> Option<String> {
    if is_secondary {
        return None;
    }
    if !node_type_name(node_type).eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair") {
        return None;
    }

    raw.get("__BrandIdentifier")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|identifier| {
            let lower = identifier.to_ascii_lowercase();
            lower.starts_with("s_") || lower.starts_with("gen_")
        })
        .map(|_| "Base".to_string())
}

pub(crate) fn colour_style_token(value: &serde_json::Value) -> Option<String> {
    value
        .get("_Type_")
        .and_then(|v| v.as_str())
        .filter(|ty| *ty == "BuildingBlocks_ColorStyle")?;

    value
        .get("color")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

pub(crate) fn fill_colour_from_raw_for_text(raw: &serde_json::Value) -> Option<[f32; 4]> {
    let obj = raw.get("FillColor")?.as_object()?;
    let r = obj.get("r").and_then(|v| v.as_f64())? as f32;
    let g = obj.get("g").and_then(|v| v.as_f64())? as f32;
    let b = obj.get("b").and_then(|v| v.as_f64())? as f32;
    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}

/// Apply the per-font `imageSizePercent` compensation exactly once.
///
/// PLAIN sizes (raw/bound/fallthrough) always divide — the engine renders
/// plain text larger than its nominal (existing data-backed model). STYLED
/// sizes (brand-table / entry / inline, used verbatim off-host) ALSO divide
/// on the GFx-HOST path (`design_text_scale != 1.0`): the host movie's
/// imported fontlib glyphs are authored at `imageSizePercent` of their em
/// square and the engine compensates at draw. Measured 2026-06-12 on BOTH
/// in-game MFD captures: every text class is ×1/0.75 vs a verbatim render —
/// power frame footer (cap ~80 vs 53), target NO TARGET heading (×1.40) and
/// footer (×1.31), the power entry-sized texts — while the NON-host medical
/// captures match styled sizes verbatim (banner cap 25 ≈ ref 27).
///
/// AVM1 corroboration (plan P2.2c, 2026-06-12): the framework AS2 applies
/// sizes verbatim — `bhvr.utils.TextFieldContainer` does
/// `tf = val.getTextFormat(); tf.size = _fontSize; val.setTextFormat(tf)`
/// with no scaling, and the full `BuildingBlocks_root.swf` dump contains no
/// fontLib/textScale/imageSizePercent handling anywhere
/// (`examples/swf_avm1_dump.rs`). The compensation therefore lives BELOW the
/// SWF layer, in the engine's fontlib rasterisation — consistent with this
/// host-path division and with it never applying to non-host canvases.
pub(crate) fn apply_font_image_size_percent(
    value: UiIrValue,
    resolved_font_record: Option<&serde_json::Value>,
    is_styled: bool,
    design_text_scale: f32,
) -> UiIrValue {
    if is_styled && (design_text_scale - 1.0).abs() <= f32::EPSILON {
        return value;
    }
    adjust_ui_ir_font_value_for_font_record_image_percent(value, resolved_font_record)
}

pub(crate) fn adjust_ui_ir_font_value_for_font_record_image_percent(
    value: UiIrValue,
    resolved_font_record: Option<&serde_json::Value>,
) -> UiIrValue {
    // DATA-BACKED MODEL: applied by the caller to plain/raw text only (the engine
    // renders plain text larger than a named style of the same nominal). Was
    // previously gated to `$Med-Heavy`; now divides by the font's imageSizePercent
    // for any font that carries it.
    let record_value = resolved_font_record
        .and_then(|record| record.get("_RecordValue_").or(Some(record)));
    let image_size_percent = record_value
        .and_then(|record_value| record_value.get("imageSizePercent"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value > 0.0 && (*value - 1.0).abs() > f32::EPSILON);

    let Some(percent) = image_size_percent else {
        return value;
    };

    match value {
        UiIrValue::Fixed { value } => UiIrValue::Fixed {
            value: value / percent,
        },
        other => other,
    }
}

/// Multiply a design-unit font size by the host-stage view scale. Percent /
/// behavioural values resolve against already-scaled rects, so only `Fixed`
/// design sizes scale.
pub(crate) fn scale_design_font_value(value: UiIrValue, design_text_scale: f32) -> UiIrValue {
    if (design_text_scale - 1.0).abs() <= f32::EPSILON {
        return value;
    }
    match value {
        UiIrValue::Fixed { value } => UiIrValue::Fixed {
            value: value * design_text_scale,
        },
        other => other,
    }
}

pub(crate) fn resolve_effective_font_size(
    node_id: BbNodeId,
    node: &crate::bb_scene::BbNode,
    text: &crate::bb_scene::BbText,
    node_rect_h: f32,
    _resolved_text: Option<&str>,
    scene: &BbScene,
    binding_resolver: &BindingResolver,
    defaults: &DefaultValueRegistry,
    style_font_sizes: &HashMap<String, f32>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
    design_text_scale: f32,
) -> (UiIrValue, bool) {
    // DATA-BACKED MODEL. Returns (size, is_styled). `is_styled` = the size came from a
    // named brand style (the engine's em-pixel design size) and is used verbatim;
    // otherwise it's a plain/raw/auto value the caller boosts by the font's
    // imageSizePercent. The SWF renderer maps em→raster via units_per_em
    // (ascent+descent), so styled text renders typographically (no nominal-scale
    // constant). autoFontSize text is fit to its rect render-side.
    //
    // `design_text_scale` is the host Flash stage→target view scale (1.0 outside
    // the MFD frame path): design-unit sizes (brand table, raw authored, bound)
    // multiply by it; rect-derived sizes (auto-fit, fixed-band prompt) are already
    // in target pixels and do not.
    let label_style = label_style_name_from_node_or_ancestors(node_id, node, scene);

    // Data-gated engine auto-fit (`autoFontSize`): the renderer grows/shrinks the
    // glyphs to fill the node's container rect using real glyph metrics, so the size
    // resolved here is just a non-zero starting point (the linear fit is independent
    // of it). Return the node's authored default rather than a heuristic visible-size.
    if auto_font_size_enabled(&node.raw) {
        let raw = font_size_from_raw(node).unwrap_or(UiIrValue::Fixed { value: 16.0 });
        return (raw, false);
    }

    // Numeric binding operations can drive FontSize without mutating node.raw — a
    // runtime override that takes precedence over the static brand style.
    if let Some(bound_font_size) = binding_resolver.resolve_field_number(node_id, "FontSize", defaults) {
        if bound_font_size.is_finite() && bound_font_size > 0.0 {
            return (
                UiIrValue::Fixed { value: bound_font_size as f32 * design_text_scale },
                false,
            );
        }
    }

    // A node-authored inline style FontSize (applied by bb_brand_apply as the
    // FINAL cascade stage, marked `__InlineFontSize`) outranks the brand-table
    // standard — the power card titles author an inline FontSize 30 over the
    // drak heading standard. Inline sizes are style-system em design sizes,
    // used verbatim like the brand table's.
    if node.raw.get("__InlineFontSize").and_then(|v| v.as_bool()) == Some(true)
        && let Some(raw_font_size) = font_size_from_raw(node)
    {
        return (scale_design_font_value(raw_font_size, design_text_scale), true);
    }

    // A cascade-entry-written FontSize (marked `__EntryFontSize` by
    // bb_brand_apply) overrides the named-style table: the engine applies
    // instance style entries over the widget-standard's own styling (the
    // power emissions texts render the M_Eng drak `FontSizeSmall` 40, not the
    // drak Heading1 standard's 60, on the in-game reference). Entry sizes are
    // style-system em design sizes, used VERBATIM: the medical mainmenu
    // banner's brand entry (FontSize 40) renders cap ≈ 25px on the in-game
    // medical reference — the verbatim 40, not a boosted 53. (The power
    // texts' remaining ×4/3 gap vs their reference is the MFD CONTENT
    // canvas's host-stage text scale — an open, separately-evidenced item:
    // the non-entry ºC glyph needs the same ×4/3.)
    if node.raw.get("__EntryFontSize").and_then(|v| v.as_bool()) == Some(true)
        && let Some(raw_font_size) = font_size_from_raw(node)
    {
        return (scale_design_font_value(raw_font_size, design_text_scale), true);
    }

    // A text field that authors a RELATIVE (Percent) HEIGHT with no explicit,
    // bound, inline, or entry FontSize sizes its glyph to fill that authored field
    // height — the engine sizes the text TO the box it was given, rather than the
    // named-style table default. The compass tick labels author `height: 0.8` (of
    // the tick) and no FontSize: they fill the tick (cap ≈ 19% of the strip), not
    // the Heading1 standard's 60 (cap ≈ 6%). The field rect is already in target
    // pixels, so it is NOT multiplied by `design_text_scale`. Scoped to Percent
    // heights so fixed/auto/content-sized fields keep their named-style size.
    //
    // Returned as STYLED (verbatim em), NOT plain: the field height IS the intended
    // glyph em, so it must NOT be divided by the font's imageSizePercent the way a
    // plain authored size (which is a slug-IMAGE height) is. Boosting it ÷0.75
    // over-sized the compass labels to ~22% cap (clipping the bottom into the tick)
    // vs the reference's ~19%; verbatim gives ~19% and clears the tick.
    if text_field_sizes_font_to_relative_height(node) && node_rect_h > 1.0 {
        return (UiIrValue::Fixed { value: node_rect_h }, true);
    }

    // Engine-faithful default: a styled textfield renders at its named style's
    // authored brand-table FontSize. The BuildingBlocks style FontSize modifier is
    // applied *after* the widget's default `fontSize`, so it overrides the node's raw
    // size — hence STYLE precedes the raw/authored branch below.
    if let Some(size) =
        standard_textfield_font_size_from_styles(node, label_style.as_deref(), standard_text_styles)
    {
        let size = size * design_text_scale;
        return (UiIrValue::Fixed { value: size }, true);
    }

    // Explicit per-node authored size (raw FontSize / direct modifier) for nodes
    // whose style has no brand-table FontSize entry.
    if let Some(raw_font_size) = font_size_from_raw(node) {
        return (scale_design_font_value(raw_font_size, design_text_scale), false);
    }

    // Borrow a scene-derived size for the style when a sibling carries an authored
    // FontSize and the brand table has no entry for it.
    if let Some(style_name) = label_style.as_deref() {
        if let Some(size) = style_font_sizes.get(style_name) {
            return (UiIrValue::Fixed { value: *size * design_text_scale }, false);
        }
    }

    // Fall through to whatever parse_text stored.
    (
        scale_design_font_value(convert_bb_value(&text.font_size), design_text_scale),
        false,
    )
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StandardTextStyle {
    pub(crate) line_spacing: Option<f32>,
    /// Brand per-glyph tracking (design units; the GFx renderer adds it to every
    /// character advance, scaled like the font size).
    pub(crate) letter_spacing: Option<f32>,
    pub(crate) font_size: Option<f32>,
    pub(crate) font_record: Option<String>,
    /// Authoritative text colour role for this named style, from the brand text-style
    /// entry's `FillColor` modifier (e.g. `Heading6`/`H6` → `Bright`) — the game's own
    /// per-style colour, preferred over derived/heuristic colour-token guesses.
    pub(crate) fill_colour_token: Option<String>,
    /// The `FillColor` modifier's authored alpha (defaults to 1.0).
    pub(crate) fill_colour_alpha: Option<f32>,
    /// The fill role resolved against the typography brand record's `colorStyles`
    /// palette at the authoritative `BB_ColorStyle` enum index. Populated only for
    /// roles whose compose-token namespace diverges from the enum (see
    /// `bb_colour_style_enum_index`).
    pub(crate) fill_colour_rgba: Option<[f32; 4]>,
}

pub(crate) fn standard_text_field_widget_path() -> &'static str {
    "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/modularkit/standard/widgets/textfieldwidgetstandard.json"
}

pub(crate) fn collect_standard_text_styles(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    selected_style_source: Option<&str>,
    canvas_name: Option<&str>,
) -> HashMap<String, StandardTextStyle> {
    let Some(fetcher) = canvas_fetcher else {
        return HashMap::new();
    };
    let Ok(record) = fetcher.fetch_canvas_by_path(standard_text_field_widget_path()) else {
        return HashMap::new();
    };
    let record_value = record.get("_RecordValue_").unwrap_or(&record);
    let mut styles = HashMap::new();

    for entry in record_value
        .get("defaultStyles")
        .and_then(|styles| styles.get("entries"))
        .and_then(|entries| entries.as_array())
        .into_iter()
        .flatten()
        .chain(
            record_value
                .get("brandStyles")
                .and_then(|styles| styles.as_array())
                .into_iter()
                .flatten()
                .filter_map(|brand| brand.get("entries").and_then(|entries| entries.as_array()))
                .flatten(),
        )
    {
        let Some(name) = entry
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let style = standard_text_style_from_entry(entry);
        styles
            .entry(name.to_string())
            .or_insert(style);
    }

    // The brand override is keyed by the screen's brand style record name. A
    // `canvas:`-sourced style names that record directly; a `manufacturer:<mfr>`
    // source maps to the manufacturer's UI style family: ship MFD canvases
    // (`MC_*`/`M_*`) carry the HUD typography brand `s_<mfr>_hud` (Drake's H1 →
    // `audimatmono-regular`, FillColor `Accent2`, LetterSpacing 2 — verified
    // against the Clipper target/power footer captures), other screens the
    // environment brand `s_<mfr>_env`.
    let selected_style_name = selected_style_source.and_then(|source| {
        source
            .strip_prefix("canvas:")
            .map(str::to_ascii_lowercase)
            .or_else(|| {
                source.strip_prefix("manufacturer:").map(|mfr| {
                    use crate::bb_brand_style::CanvasFamily;
                    let stripped = canvas_name
                        .map(|name| name.strip_prefix("BuildingBlocks_Canvas.").unwrap_or(name));
                    let family = stripped.map(crate::bb_brand_style::classify_canvas_family);
                    // MFD masters (MC_*/M_*) AND cockpit HUD ship-components
                    // (HC_HUD_*/H_Eng_*) are HUD-typography canvases that author
                    // s_<mfr>_hud; classify_canvas_family only catches the former,
                    // so recognise the HUD family explicitly. Otherwise the HUD
                    // labels (compass headings, …) fall to s_<mfr>_env, whose H1 is
                    // audimatmono-Bold/Bright instead of the HUD brand's
                    // audimatmono-regular/Accent2.
                    let is_hud = matches!(family, Some(CanvasFamily::Mfd | CanvasFamily::MfdRoot))
                        || stripped
                            .map(crate::bb_brand_style::is_cockpit_hud_canvas)
                            .unwrap_or(false);
                    let class = if is_hud { "hud" } else { "env" };
                    format!("s_{}_{}", mfr.to_ascii_lowercase(), class)
                })
            })
    });
    let selected_brand = record_value
        .get("brandStyles")
        .and_then(|styles| styles.as_array())
        .into_iter()
        .flatten()
        .find(|brand| {
            let Some(selected_style_name) = selected_style_name.as_deref() else {
                return false;
            };
            brand
                .get("brandIdentifier")
                .and_then(|identifier| identifier.as_str())
                .map(crate::record_name::extract_record_name)
                .is_some_and(|identifier| identifier.eq_ignore_ascii_case(selected_style_name))
        });
    // The typography brand's colour palette (its Style record `colorStyles`),
    // for resolving authored FillColor roles at the enum index.
    let brand_palette = selected_brand
        .and_then(|brand| brand.get("brandIdentifier"))
        .and_then(|identifier| identifier.as_str())
        .and_then(|url| fetcher.fetch_canvas_by_path(url).ok());
    for entry in selected_brand
        .and_then(|brand| brand.get("entries").and_then(|entries| entries.as_array()))
        .into_iter()
        .flatten()
    {
        let Some(name) = entry
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let mut style = standard_text_style_from_entry(entry);
        style.fill_colour_rgba = resolve_enum_divergent_fill_rgba(&style, brand_palette.as_ref());
        if style.line_spacing.is_some() || style.font_size.is_some() {
            styles.insert(name.to_string(), style);
        }
    }

    styles
}

pub(crate) fn standard_text_style_from_entry(entry: &serde_json::Value) -> StandardTextStyle {
    let mut style = StandardTextStyle::default();
    for modifier in entry
        .get("modifiers")
        .and_then(|modifiers| modifiers.as_array())
        .into_iter()
        .flatten()
    {
        let field = modifier.get("field");
        if let Some(field_name) = field.and_then(|field| field.as_str()) {
            let value = modifier
                .get("value")
                .and_then(|value| value.as_f64())
                .map(|value| value as f32);
            if field_name.eq_ignore_ascii_case("LineSpacing") {
                style.line_spacing = value;
            } else if field_name.eq_ignore_ascii_case("LetterSpacing") {
                style.letter_spacing = value;
            } else if field_name.eq_ignore_ascii_case("FontSize") {
                style.font_size = value;
            } else if field_name.eq_ignore_ascii_case("FillColor") {
                style.fill_colour_token = modifier.get("color").and_then(colour_style_token);
                style.fill_colour_alpha = modifier
                    .get("color")
                    .and_then(|colour| colour.get("alpha"))
                    .and_then(|value| value.as_f64())
                    .map(|value| value as f32);
            } else if field_name.eq_ignore_ascii_case("FontStyleRecord") {
                style.font_record = modifier
                    .get("value")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
            }
        } else if let Some(field_obj) = field.and_then(|field| field.as_object()) {
            let field_type = field_obj
                .get("_Type_")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if field_type.ends_with("FontStyleRecord") {
                style.font_record = field_obj
                    .get("value")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
            }
        }
    }
    style
}

pub(crate) fn resolve_effective_line_spacing(
    node: &crate::bb_scene::BbNode,
    effective_font_size: &UiIrValue,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
    design_text_scale: f32,
) -> Option<f32> {
    // Authored line spacing is in design units like the font size; the standard
    // brand-table branch self-scales through the effective/standard size ratio.
    line_spacing_from_raw(node).map(|spacing| spacing * design_text_scale).or_else(|| {
        label_style_name_from_raw(node).and_then(|style| {
            standard_text_style_keys(&style)
                .into_iter()
                .find_map(|key| standard_text_styles.get(&key).and_then(|standard| {
                    standard.line_spacing.map(|line_spacing| {
                        scale_standard_line_spacing(line_spacing, standard.font_size, effective_font_size)
                    })
                }))
        })
    })
}

fn scale_standard_line_spacing(
    line_spacing: f32,
    standard_font_size: Option<f32>,
    effective_font_size: &UiIrValue,
) -> f32 {
    let Some(standard_font_size) = standard_font_size.filter(|value| value.is_finite() && *value > 0.0) else {
        return line_spacing;
    };
    let UiIrValue::Fixed { value: effective_font_size } = effective_font_size else {
        return line_spacing;
    };
    if !effective_font_size.is_finite() || *effective_font_size <= 0.0 {
        return line_spacing;
    }
    line_spacing * (*effective_font_size / standard_font_size)
}

pub(crate) fn standard_text_style_keys(style: &str) -> Vec<String> {
    let trimmed = style.trim();
    let mut keys = Vec::new();
    if let Some(index) = trimmed.strip_prefix("Title") {
        keys.push(format!("T{index}"));
    } else if let Some(index) = trimmed.strip_prefix("Heading") {
        keys.push(format!("H{index}"));
    }
    keys.push(trimmed.to_string());
    keys
}


fn line_spacing_from_raw(node: &crate::bb_scene::BbNode) -> Option<f32> {
    node.raw
        .get("lineSpacing")
        .or_else(|| node.raw.get("LineSpacing"))
        .or_else(|| {
            node.raw
                .get("modifiers")
                .and_then(|mods| mods.get("lineSpacing").or_else(|| mods.get("LineSpacing")))
        })
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite())
}

pub(crate) fn label_style_name_from_raw(node: &crate::bb_scene::BbNode) -> Option<String> {
    node.raw
        .get("labelProperties")
        .and_then(|v| v.get("style"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn resolved_text_from_payload(payload: &UiIrTextPayload) -> Option<&str> {
    match payload {
        UiIrTextPayload::Resolved { text } => Some(text.as_str()),
        _ => None,
    }
}

/// Read a node's raw `fontSize`/`FontSize` (authored field, applied style
/// modifier, or `modifiers` passthrough) as a UI IR value.
fn font_size_from_raw(node: &crate::bb_scene::BbNode) -> Option<UiIrValue> {
    let value = node
        .raw
        .get("fontSize")
        .or_else(|| node.raw.get("FontSize"))
        .or_else(|| {
            node.raw
                .get("modifiers")
                .and_then(|mods| mods.get("fontSize").or_else(|| mods.get("FontSize")))
        })?;

    if let Some(number) = value.as_f64() {
        return Some(UiIrValue::Fixed {
            value: number as f32,
        });
    }

    let obj = value.as_object()?;
    let raw_value = obj.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let behavior = obj
        .get("behavior")
        .and_then(|b| b.as_str())
        .unwrap_or("Fixed");
    Some(match behavior {
        "Fixed" => UiIrValue::Fixed { value: raw_value },
        "Percent" => UiIrValue::Percent { value: raw_value },
        other => UiIrValue::Other {
            value: raw_value,
            behavior: other.to_owned(),
        },
    })
}

fn font_size_fixed_value_from_raw(node: &crate::bb_scene::BbNode) -> Option<f32> {
    match font_size_from_raw(node)? {
        UiIrValue::Fixed { value } => Some(value),
        _ => None,
    }
}

pub(crate) fn classify_text_payload(
    resolved_text: Option<&str>,
    raw: &serde_json::Value,
    defaults: &DefaultValueRegistry,
) -> UiIrTextPayload {
    let Some(text) = resolved_text.map(str::trim) else {
        return UiIrTextPayload::Empty;
    };

    if !text.is_empty() {
        return classify_literal_or_localized_text(text, defaults);
    }

    if let Some(unresolved_key) = unresolved_text_key_from_raw(raw) {
        if let Some(localized) = defaults.lookup_localization(&unresolved_key) {
            if localized.trim().is_empty() {
                UiIrTextPayload::IntentionallyEmpty {
                    key: Some(unresolved_key),
                }
            } else {
                UiIrTextPayload::Resolved {
                    text: localized.trim().to_string(),
                }
            }
        } else {
            UiIrTextPayload::UnresolvedKey {
                key: unresolved_key,
            }
        }
    } else {
        UiIrTextPayload::Empty
    }
}

fn classify_literal_or_localized_text(
    text: &str,
    defaults: &DefaultValueRegistry,
) -> UiIrTextPayload {
    if text.starts_with('@') {
        if let Some(localized) = defaults.lookup_localization(text) {
            if localized.trim().is_empty() {
                UiIrTextPayload::IntentionallyEmpty {
                    key: Some(text.to_string()),
                }
            } else {
                UiIrTextPayload::Resolved {
                    text: localized.trim().to_string(),
                }
            }
        } else {
            UiIrTextPayload::UnresolvedKey {
                key: text.to_string(),
            }
        }
    } else {
        UiIrTextPayload::Resolved {
            text: text.to_string(),
        }
    }
}

// Typography brand-palette helpers: the authoritative `BB_ColorStyle` enum
// index and the resolution of authored brand text-style `FillColor` roles /
// `LetterSpacing` for `StandardTextStyle` consumers.

/// `BB_ColorStyle` enum index (the authoritative DataCore order; see
/// `bb_brand_apply::colors` for the dumped table). Unlike the role-calibrated
/// compose-token resolvers, this is the raw enum used by authored brand
/// typography `FillColor` roles.
fn bb_colour_style_enum_index(name: &str) -> Option<usize> {
    Some(match name {
        "Base" => 0,
        "Positive" => 1,
        "Moderate" => 2,
        "Critical" => 3,
        "Accent1" => 4,
        "Accent2" => 5,
        "Bright" => 6,
        "Selected" => 7,
        "Disabled" => 8,
        "Background" => 9,
        "ContactNeutral" => 10,
        "ContactParty" => 11,
        "ContactPositiveRep" => 12,
        "ContactNegativeRep" => 13,
        "ContactAgressive" => 14,
        "ContactUnknown" => 15,
        "MissionObjectives" => 16,
        _ => return None,
    })
}

/// Resolve a brand text style's authored `FillColor` role to an explicit RGBA
/// against the typography brand record's `colorStyles` — but only for roles
/// whose compose-token namespace diverges from the enum index (`Base` and
/// `Bright` resolve identically through the token path and stay token-only,
/// keeping frozen snapshot semantics stable). Verified for `Accent2` on the
/// Clipper MFD footer (drak hud slot 5, the darker orange).
fn resolve_enum_divergent_fill_rgba(
    style: &StandardTextStyle,
    brand_palette: Option<&serde_json::Value>,
) -> Option<[f32; 4]> {
    let token = style.fill_colour_token.as_deref()?;
    if matches!(token, "Base" | "Bright") {
        return None;
    }
    let slot = bb_colour_style_enum_index(token)?;
    let record_value = brand_palette?
        .get("_RecordValue_")
        .or(brand_palette);
    let colour = record_value?
        .get("colorStyles")
        .and_then(|slots| slots.as_array())
        .and_then(|slots| slots.get(slot))
        .and_then(|entry| entry.get("color"))?;
    let component = |key: &str| -> Option<f32> {
        let value = colour.get(key).and_then(|v| v.as_f64())? as f32;
        Some(if value > 1.0 { value / 255.0 } else { value })
    };
    let alpha = style.fill_colour_alpha.unwrap_or(1.0).clamp(0.0, 1.0);
    Some([component("r")?, component("g")?, component("b")?, alpha])
}

/// The brand text style's pre-resolved fill RGBA for a named style, when the
/// authored role required enum-index resolution (see
/// `resolve_enum_divergent_fill_rgba`).
pub(crate) fn brand_text_style_fill_rgba(
    style: Option<&str>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
) -> Option<[f32; 4]> {
    let style = style.map(str::trim).filter(|name| !name.is_empty())?;
    standard_text_style_keys(style).into_iter().find_map(|key| {
        standard_text_styles
            .get(&key)
            .and_then(|standard| standard.fill_colour_rgba)
    })
}

/// The brand text style's authored per-glyph tracking (design units) for a
/// named style.
pub(crate) fn brand_style_letter_spacing(
    style: Option<&str>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
) -> Option<f32> {
    let style = style.map(str::trim).filter(|name| !name.is_empty())?;
    standard_text_style_keys(style).into_iter().find_map(|key| {
        standard_text_styles
            .get(&key)
            .and_then(|standard| standard.letter_spacing)
    })
}

/// The authoritative colour role for a named text style (e.g. `Heading6` →
/// `Bright`), taken from the active brand's text-style `FillColor` modifier.
/// Returns `None` when the style is unknown or carries no `FillColor`.
pub(crate) fn brand_text_style_colour_token(
    style: Option<&str>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
) -> Option<String> {
    let style = style.map(str::trim).filter(|name| !name.is_empty())?;
    standard_text_style_keys(style).into_iter().find_map(|key| {
        standard_text_styles
            .get(&key)
            .and_then(|standard| standard.fill_colour_token.clone())
    })
}

fn standard_textfield_font_size_from_styles(
    node: &crate::bb_scene::BbNode,
    label_style: Option<&str>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
) -> Option<f32> {
    if !matches!(node.ty, BbNodeType::WidgetTextField) {
        return None;
    }
    brand_style_font_size(label_style, standard_text_styles)
}

/// The authored brand FontSize for a named style (`Heading1`-`6`, `Title1`-`5`, `Body`)
/// — the engine's em-pixel design size, used verbatim (the SWF renderer maps em→raster
/// via units_per_em = ascent + descent, so styled text is typographic, no nominal-scale
/// constant). Node-agnostic so caption-pair label/value styles resolve the same way a
/// plain `WidgetTextField` does.
pub(crate) fn brand_style_font_size(
    label_style: Option<&str>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
) -> Option<f32> {
    let label_style = label_style?;
    standard_text_style_keys(label_style)
        .into_iter()
        .filter_map(|key| standard_text_styles.get(&key))
        .find(|standard| standard.font_size.is_some())
        .and_then(|standard| standard.font_size)
        .filter(|font_size| font_size.is_finite() && *font_size > 0.0)
        .map(|font_size| (font_size * 10.0).round() / 10.0)
}


fn label_style_name_from_node_or_ancestors(
    node_id: BbNodeId,
    node: &crate::bb_scene::BbNode,
    scene: &BbScene,
) -> Option<String> {
    if let Some(style) = label_style_name_from_raw(node) {
        return Some(style);
    }

    let mut current = scene.nodes.get(&node_id).and_then(|n| n.parent);
    while let Some(parent_id) = current {
        let parent = scene.nodes.get(&parent_id)?;
        if let Some(style) = label_style_name_from_raw(parent) {
            return Some(style);
        }
        current = parent.parent;
    }
    None
}

pub(crate) fn collect_style_font_sizes(scene: &BbScene) -> HashMap<String, f32> {
    let mut values_by_style: HashMap<String, Vec<f32>> = HashMap::new();

    for node in scene.nodes.values() {
        let Some(style_name) = label_style_name_from_raw(node) else {
            continue;
        };
        let Some(value) = font_size_fixed_value_from_raw(node) else {
            continue;
        };
        if value > 0.0 {
            values_by_style.entry(style_name).or_default().push(value);
        }
    }

    values_by_style
        .into_iter()
        .map(|(style, mut values)| {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = values[values.len() / 2];
            (style, median)
        })
        .collect()
}

pub(crate) fn resolve_record(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    candidates: &[&str],
) -> Option<serde_json::Value> {
    let fetcher = canvas_fetcher?;
    for candidate in candidates {
        let key = candidate.trim();
        if key.is_empty() {
            continue;
        }
        if let Ok(record) = fetcher.fetch_canvas_by_path(key) {
            return Some(record);
        }
        if let Ok(record) = fetcher.fetch_canvas_by_name(key) {
            return Some(record);
        }
        if let Ok(record) = fetcher.fetch_canvas_json(key) {
            return Some(record);
        }
    }
    None
}

/// Resolve a style-tag record for IR emission.
///
/// Resolution order:
/// 1. Direct record fetch by `_RecordPath_`, `_RecordName_`, and UUID.
/// 2. If those fail and `_RecordPath_` points to `tagdatabase`, fetch the
///    tag-database record and resolve the UUID from its nested `tags[]` tree.
///
/// This keeps tag resolution in the core IR pipeline instead of requiring
/// dump-specific flattening/indexing steps.
pub(crate) fn resolve_style_tag_record(
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    tag_reference: &serde_json::Value,
    record_path: &str,
    record_name: &str,
    tag_uuid: &str,
) -> Option<serde_json::Value> {
    let tag_db_path = record_path.trim();
    let is_tag_database_path = !tag_db_path.is_empty()
        && tag_db_path.to_ascii_lowercase().contains("tagdatabase");

    if !is_tag_database_path {
        if let Some(record) = resolve_record(canvas_fetcher, &[record_path, record_name, tag_uuid]) {
            return Some(record);
        }
    }

    let fetcher = canvas_fetcher?;
    if !is_tag_database_path {
        if let Some(record) = resolve_record(Some(fetcher), &[tag_uuid]) {
            return Some(record);
        }
    }

    if !is_tag_database_path {
        return None;
    }

    // Shared fetch: the `TagDatabase` is large and re-fetched per style-tag per
    // node (thousands of times per heavy binding). `fetch_canvas_by_path_shared`
    // returns the memoising fetcher's cached `Rc` so this is a refcount bump
    // rather than a deep clone of the whole database on every call.
    let tag_db = fetcher.fetch_canvas_by_path_shared(tag_db_path).ok()?;
    let tags = tag_db
        .get("_RecordValue_")
        .and_then(|rv| rv.get("tags"))
        .and_then(|v| v.as_array())?;

    let tag_value = tags.iter().find_map(|tag| find_tag_in_tree(tag, tag_uuid))?;
    let resolved_record_name = tag_reference
        .get("_RecordName_")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Tag.{tag_uuid}"));

    Some(serde_json::json!({
        "_RecordId_": tag_uuid,
        "_RecordName_": resolved_record_name,
        "_RecordPath_": tag_db_path,
        "_Type_": "Tag",
        "_RecordValue_": trim_tag_tree_to_matched_tag(tag_value),
    }))
}

fn find_tag_in_tree<'a>(value: &'a serde_json::Value, tag_uuid: &str) -> Option<&'a serde_json::Value> {
    let object = value.as_object()?;
    let matches = object
        .get("_RecordId_")
        .and_then(|v| v.as_str())
        .is_some_and(|id| id == tag_uuid);
    if matches {
        return Some(value);
    }

    object
        .get("children")
        .and_then(|v| v.as_array())
        .and_then(|children| children.iter().find_map(|child| find_tag_in_tree(child, tag_uuid)))
}

fn trim_tag_tree_to_matched_tag(tag_value: &serde_json::Value) -> serde_json::Value {
    let Some(object) = tag_value.as_object() else {
        return tag_value.clone();
    };

    let mut trimmed = serde_json::Map::with_capacity(object.len());
    for (key, value) in object {
        if key != "children" {
            trimmed.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(trimmed)
}

pub(crate) fn convert_bb_value(value: &BbValue) -> UiIrValue {
    match value {
        BbValue::Fixed(v) => UiIrValue::Fixed { value: *v },
        BbValue::Percent(v) => UiIrValue::Percent { value: *v },
        BbValue::Other { value, behavior } => UiIrValue::Other {
            value: *value,
            behavior: behavior.clone(),
        },
    }
}

fn animation_number_keyframes(raw: &serde_json::Value, field_name: &str) -> Vec<(f64, f32)> {
    let Some(keyframes) = raw.get("animation")
        .and_then(|animation| animation.get("animationTimeline"))
        .and_then(|timeline| timeline.get("keyframes"))
        .and_then(|keyframes| keyframes.as_array())
    else {
        return Vec::new();
    };

    keyframes
        .iter()
        .flat_map(|keyframe| {
            let percent = keyframe
                .get("percent")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            keyframe
                .get("modifiers")
                .and_then(|modifiers| modifiers.as_array())
                .into_iter()
                .flatten()
                .filter_map(move |modifier_data| {
                    let modifier = modifier_data.get("modifier").unwrap_or(modifier_data);
                    let is_number = modifier
                        .get("_Type_")
                        .and_then(|value| value.as_str())
                        .is_some_and(|ty| ty == "BuildingBlocks_FieldModifierNumber");
                    if !is_number {
                        return None;
                    }
                    let matches_field = modifier
                        .get("field")
                        .and_then(|value| value.as_str())
                        .is_some_and(|field| field == field_name);
                    if !matches_field {
                        return None;
                    }
                    modifier
                        .get("value")
                        .and_then(|value| value.as_f64())
                        .map(|value| (percent, value as f32))
                })
        })
        .collect()
}

fn representative_animation_alpha(raw: &serde_json::Value) -> Option<f32> {
    animation_number_keyframes(raw, "Alpha")
        .into_iter()
        .map(|(_, value)| value.clamp(0.0, 1.0))
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn local_alpha_for_node(
    node: &crate::bb_scene::BbNode,
    animation_sample_percent: Option<f32>,
) -> f32 {
    if is_transient_static_pulse_node(node) {
        return 0.0;
    }
    // NOTE: page-in start-state roots (authored alpha=0 that the engine fades in)
    // are settled to 1.0 in `bb_scene::parse` (per-canvas, where the page-in node
    // is still a scene root — canvas merging later re-parents it under the
    // referencing `WidgetCanvas`, which is why settling cannot be done here).
    if let Some(sample_percent) = animation_sample_percent {
        sampled_animation_alpha(&node.raw, sample_percent)
            .unwrap_or_else(|| representative_animation_alpha(&node.raw).unwrap_or(node.alpha))
    } else {
        representative_animation_alpha(&node.raw).unwrap_or(node.alpha)
    }
    .clamp(0.0, 1.0)
}

pub(crate) fn effective_alpha_for_node(
    node_id: BbNodeId,
    node: &crate::bb_scene::BbNode,
    scene: &BbScene,
    animation_sample_percent: Option<f32>,
) -> f32 {
    let mut alpha = local_alpha_for_node(node, animation_sample_percent);
    let mut current_parent = node.parent;
    let mut visited = HashSet::from([node_id]);

    while let Some(parent_id) = current_parent {
        if !visited.insert(parent_id) {
            break;
        }
        let Some(parent) = scene.nodes.get(&parent_id) else {
            break;
        };
        alpha *= local_alpha_for_node(parent, animation_sample_percent);
        current_parent = parent.parent;
    }

    alpha.clamp(0.0, 1.0)
}

fn sampled_animation_alpha(raw: &serde_json::Value, sample_percent: f32) -> Option<f32> {
    sampled_animation_number(raw, "Alpha", sample_percent).map(|value| value.clamp(0.0, 1.0))
}

fn sampled_animation_number(raw: &serde_json::Value, field_name: &str, sample_percent: f32) -> Option<f32> {
    let mut keyframes = animation_number_keyframes(raw, field_name);
    if keyframes.is_empty() {
        return None;
    }
    keyframes.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(std::cmp::Ordering::Equal));

    let timeline_max = keyframes.last().map(|(percent, _)| *percent).unwrap_or(1.0);
    let sample = if timeline_max <= 1.0 {
        sample_percent as f64 / 100.0
    } else {
        sample_percent as f64
    };

    let first = keyframes[0];
    if sample <= first.0 {
        return Some(first.1);
    }

    for window in keyframes.windows(2) {
        let (left_percent, left_value) = window[0];
        let (right_percent, right_value) = window[1];
        if sample <= right_percent {
            let span = right_percent - left_percent;
            if span <= f64::EPSILON {
                return Some(right_value);
            }
            let t = ((sample - left_percent) / span) as f32;
            return Some(left_value + (right_value - left_value) * t);
        }
    }

    keyframes.last().map(|(_, value)| *value)
}

fn is_transient_static_pulse_node(node: &BbNode) -> bool {
    if !node_type_name(&node.ty).eq_ignore_ascii_case("BuildingBlocks_WidgetCircle") {
        return false;
    }

    let looping = node
        .raw
        .get("animation")
        .and_then(|animation| animation.get("loopIndefinitely"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !looping {
        return false;
    }

    let mut alpha_keyframes = animation_number_keyframes(&node.raw, "Alpha");
    if alpha_keyframes.len() < 2 {
        return false;
    }
    alpha_keyframes.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap_or(std::cmp::Ordering::Equal));

    let starts_hidden = alpha_keyframes.first().is_some_and(|(_, value)| *value <= 0.001);
    let ends_hidden = alpha_keyframes.last().is_some_and(|(_, value)| *value <= 0.001);
    let scales_over_time = !animation_number_keyframes(&node.raw, "SizeX").is_empty()
        || !animation_number_keyframes(&node.raw, "SizeY").is_empty();

    starts_hidden && ends_hidden && scales_over_time
}

pub(crate) fn node_type_name(node_type: &BbNodeType) -> &str {
    match node_type {
        BbNodeType::DisplayWidget => "display_widget",
        BbNodeType::WidgetCanvas => "widget_canvas",
        BbNodeType::WidgetIcon => "widget_icon",
        BbNodeType::WidgetCard => "widget_card",
        BbNodeType::WidgetTextField => "widget_text_field",
        BbNodeType::ComponentGeneralButton => "component_general_button",
        BbNodeType::ComponentGeneralButtonSecondary => "component_general_button_secondary",
        BbNodeType::WidgetImage => "widget_image",
        BbNodeType::WidgetText => "widget_text",
        BbNodeType::WidgetCustomShape => "widget_custom_shape",
        BbNodeType::WidgetBodyBackground => "widget_body_background",
        BbNodeType::Other(s) => s,
    }
}
