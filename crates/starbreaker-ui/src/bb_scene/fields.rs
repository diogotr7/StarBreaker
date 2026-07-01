use super::types::*;

pub(super) fn f32_field(obj: &serde_json::Value, key: &str) -> f32 {
    obj.get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32
}

pub(super) fn parse_vec3(v: Option<&serde_json::Value>) -> Option<Vec3> {
    let obj = v?;
    Some(Vec3 { x: f32_field(obj, "x"), y: f32_field(obj, "y"), z: f32_field(obj, "z") })
}

/// Extract a Vec2 from a field that may be a Vec3 or Vec2 JSON object.
pub(super) fn parse_vec2_from_vec3(v: Option<&serde_json::Value>) -> Vec2 {
    match v {
        Some(obj) => Vec2 { x: f32_field(obj, "x"), y: f32_field(obj, "y") },
        None => Vec2::default(),
    }
}

pub(super) fn parse_bb_value(v: Option<&serde_json::Value>) -> BbValue {
    let obj = match v {
        Some(o) if o.is_object() => o,
        _ => return BbValue::default(),
    };
    let value = f32_field(obj, "value");
    let behavior = obj.get("behavior").and_then(|b| b.as_str()).unwrap_or("Fixed");
    match behavior {
        "Fixed" => BbValue::Fixed(value),
        "Percent" => BbValue::Percent(value),
        other => BbValue::Other { value, behavior: other.to_owned() },
    }
}

pub(super) fn parse_sizing(v: Option<&serde_json::Value>) -> BbSizing {
    let obj = match v {
        Some(o) => o,
        None => return BbSizing::default(),
    };
    BbSizing {
        width: parse_bb_value(obj.get("width")),
        height: parse_bb_value(obj.get("height")),
    }
}

pub(super) fn parse_trbl(v: Option<&serde_json::Value>) -> BbTrbl {
    let obj = match v {
        Some(o) => o,
        None => return BbTrbl::default(),
    };
    BbTrbl {
        top: f32_field(obj, "top"),
        right: f32_field(obj, "right"),
        bottom: f32_field(obj, "bottom"),
        left: f32_field(obj, "left"),
    }
}

pub(super) fn parse_style_tags(v: Option<&serde_json::Value>) -> Vec<String> {
    let arr = match v.and_then(|a| a.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|tag| tag.get("_RecordId_").and_then(|r| r.as_str()).map(str::to_owned))
        .collect()
}

/// Try to parse a colour value from a JSON value into `[r, g, b, a]` (0–1).
///
/// Handles `{ "r": …, "g": …, "b": …, "a": … }` with either 0–255 or 0–1
/// component ranges (heuristic: if any component > 1.0, assume 0–255).
/// Returns `None` for null, strings, or unrecognised shapes.
pub(super) fn parse_colour(v: Option<&serde_json::Value>) -> Option<[f32; 4]> {
    let obj = v?;
    if obj.is_null() {
        return None;
    }
    if obj
        .get("_Type_")
        .and_then(|ty| ty.as_str())
        .is_some_and(|ty| ty == "BuildingBlocks_ColorSolid")
    {
        return parse_colour(obj.get("color"));
    }
    let r = obj.get("r").and_then(|c| c.as_f64())? as f32;
    let g = obj.get("g").and_then(|c| c.as_f64())? as f32;
    let b = obj.get("b").and_then(|c| c.as_f64())? as f32;
    let a = obj.get("a").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        Some([r / 255.0, g / 255.0, b / 255.0, a / 255.0])
    } else {
        Some([r, g, b, a])
    }
}

pub(super) fn parse_background(node: &serde_json::Value) -> Option<BbBackground> {
    let bg = node.get("background")?;
    if bg.is_null() {
        return None;
    }

    // Honour the BuildingBlocks `enable` flag: when an authored background has
    // `enable: false`, the engine skips its fill_colour entirely and the panel
    // contributes only via children. Some scenes carry a non-null `color`
    // (often a debug yellow RGBA 255,255,0,255) gated behind `enable: false`;
    // rendering that fill produces spurious opaque rectangles. The svgFill
    // and segmentedFill sub-features are independent and remain available.
    let bg_enable = bg
        .get("enable")
        .and_then(|e| e.as_bool())
        .unwrap_or(true);

    let fill_colour = if bg_enable {
        bg.get("color").and_then(|c| parse_colour(Some(c)))
    } else {
        None
    };

    let svg_fill_path = node
        .get("svgFill")
        .and_then(|s| s.get("svgPath"))
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .map(str::to_owned);

    let segmented_fill = node
        .get("segmentedFill")
        .and_then(|sf| {
            let enabled = sf.get("enable").and_then(|e| e.as_bool()).unwrap_or(false);
            if enabled {
                let angle = f32_field(sf, "angle");
                Some(BbSegmentedFill { angle })
            } else {
                None
            }
        });

    Some(BbBackground { fill_colour, svg_fill_path, segmented_fill })
}

fn parse_border_side(v: Option<&serde_json::Value>) -> BbBorderSide {
    let obj = match v {
        Some(o) => o,
        None => return BbBorderSide::default(),
    };
    BbBorderSide {
        width: f32_field(obj, "width"),
        colour: parse_colour(obj.get("color")),
    }
}

pub(super) fn parse_border(v: Option<&serde_json::Value>) -> Option<BbBorder> {
    let obj = v?;
    if obj.is_null() {
        return None;
    }
    Some(BbBorder {
        top: parse_border_side(obj.get("top")),
        right: parse_border_side(obj.get("right")),
        bottom: parse_border_side(obj.get("bottom")),
        left: parse_border_side(obj.get("left")),
    })
}

pub(super) fn parse_radial(v: Option<&serde_json::Value>) -> Option<BbRadialTransform> {
    let obj = v?;
    if obj.is_null() {
        return None;
    }
    let transform_multiplier = f32_field(obj, "transformMultiplier");
    let curvature_axis = obj
        .get("curvatureAxis")
        .and_then(|a| a.as_str())
        .unwrap_or("Z")
        .to_owned();
    Some(BbRadialTransform { transform_multiplier, curvature_axis })
}

pub(super) fn parse_text(node: &serde_json::Value) -> BbText {
    let alignment = node
        .get("textAlignment")
        .and_then(|v| v.as_str())
        .unwrap_or("Left")
        .to_owned();

    // Static text string — typically bound at runtime; default to empty.
    let string = node
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    // Font record — not present in current fixtures but captured if available.
    let font_record = node
        .get("fontRecord")
        .or_else(|| node.get("FontStyleRecord"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // Font size — use a sensible default when absent.
    let font_size = node
        .get("fontSize")
        .or_else(|| node.get("FontSize"))
        .map(|v| parse_bb_value(Some(v)))
        .unwrap_or(BbValue::Fixed(12.0));

    let colour = node
        .get("textColor")
        .or_else(|| node.get("textColour"))
        .and_then(|c| parse_colour(Some(c)));

    // Horizontal anchor-to-parent from labelProperties.anchorToParentX.
    let anchor_to_parent_x = node
        .get("labelProperties")
        .and_then(|lp| lp.get("anchorToParentX"))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32);

    BbText {
        string,
        font_record,
        font_size,
        alignment,
        colour,
        anchor_to_parent_x,
    }
}

pub(crate) fn parse_icon(node: &serde_json::Value, ty: &BbNodeType) -> BbIcon {
    let image_record = node
        .get("iconProperties")
        .and_then(|ip| ip.get("customIcon"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            node.get("imagePath")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });

    // Tint colour — not present in current fixtures but captured if available.
    let tint_colour = node
        .get("iconProperties")
        .and_then(|ip| ip.get("color"))
        .and_then(|c| parse_colour(Some(c)));

    let icon_preset = node
        .get("iconProperties")
        .and_then(|ip| ip.get("iconPreset"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // A raw `WidgetIcon` with no explicit `customIcon`/`imagePath` selects its
    // glyph by `iconPreset` (a `BB_IconWidgetPreset` enum value); resolve it to the
    // engine's vector-icon asset so the icon renders (e.g. the MFD footer's `<`/`>`
    // carats). Button components (GeneralButton[Secondary]) also carry an
    // `iconProperties` but paint their icon through their own component pipeline
    // (the procedural `GeneralX` close button, or a `ParamInput0`-supplied SVG), so
    // the preset fallback is scoped to `WidgetIcon` to avoid injecting a duplicate.
    let image_record = image_record.or_else(|| {
        (*ty == BbNodeType::WidgetIcon)
            .then(|| icon_preset.as_deref().and_then(crate::icon_preset::svg_path_for_preset))
            .flatten()
    });

    BbIcon {
        image_record,
        icon_preset,
        tint_colour,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
