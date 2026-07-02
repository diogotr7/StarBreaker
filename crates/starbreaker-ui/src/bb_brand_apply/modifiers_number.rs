//! Number-typed style modifier application: `Size*`/`Anchor*`/`Pivot*`,
//! border widths, corner radii (written into the authored raw border
//! structure), padding (typed `node.padding` the layout engine reads), and
//! nine-slice raw passthrough. Split from `modifiers.rs` (line-cap).

use crate::bb_scene::BbNode;
use super::colors::ensure_border;
use super::modifiers::bb_value_with_raw_behavior;

/// Write one corner's radius into the authored raw `border` structure that
/// the IR's corner-geometry readers consume (`border.<corner>.radius.value`).
/// A ZERO radius modifier keeps the previously-styled/authored value — the
/// same `value<=0 → keep authored` semantics as the geometry field bindings:
/// the button standards' state entries author `BorderTopRightRadius 0.0`
/// alongside a real `BorderBottomRightRadius 20` and the reference keeps the
/// canvas's base rounding on the zero corners (ledger 106).
pub(super) fn set_raw_corner_radius(node: &mut BbNode, corner: &str, value: f64) {
    if value <= 0.0 {
        return;
    }
    if node.raw.is_null() {
        node.raw = serde_json::Value::Object(serde_json::Map::new());
    }
    let Some(raw_obj) = node.raw.as_object_mut() else {
        return;
    };
    let border = raw_obj
        .entry("border".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(border_obj) = border.as_object_mut() else {
        return;
    };
    let corner_value = border_obj
        .entry(corner.to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(corner_obj) = corner_value.as_object_mut() else {
        return;
    };
    let radius_value = corner_obj
        .entry("radius".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(radius_obj) = radius_value.as_object_mut() {
        radius_obj.insert(
            "value".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap_or_else(|| 0.into())),
        );
    }
}

/// Apply a number-typed modifier field.
pub(super) fn apply_number_field(field_name: &str, value: f64, node: &mut BbNode) {
    match field_name {
        "SizeX" => {
            let v = value as f32;
            let current = node.sizing.width.clone();
            node.sizing.width = bb_value_with_raw_behavior(v, node.raw.get("WidthBehavior"), &current);
        }
        "SizeY" => {
            let v = value as f32;
            let current = node.sizing.height.clone();
            node.sizing.height = bb_value_with_raw_behavior(v, node.raw.get("HeightBehavior"), &current);
        }
        "AnchorX" => node.anchor.x = value as f32,
        "AnchorY" => node.anchor.y = value as f32,
        "PivotX" => node.pivot.x = value as f32,
        "PivotY" => node.pivot.y = value as f32,
        "Alpha" => node.alpha = (value as f32).clamp(0.0, 1.0),
        "BorderWidth" => {
            let width = value as f32;
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.width = width;
                border.right.width = width;
                border.bottom.width = width;
                border.left.width = width;
            }
        }
        "BorderWidthTop" | "BorderTopWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.width = value as f32;
            }
        }
        "BorderWidthRight" | "BorderRightWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.right.width = value as f32;
            }
        }
        "BorderWidthBottom" | "BorderBottomWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.bottom.width = value as f32;
            }
        }
        "BorderWidthLeft" | "BorderLeftWidth" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.left.width = value as f32;
            }
        }
        "BorderTopLeftRadius" => set_raw_corner_radius(node, "topLeftRadius", value),
        "BorderTopRightRadius" => set_raw_corner_radius(node, "topRightRadius", value),
        "BorderBottomLeftRadius" => set_raw_corner_radius(node, "bottomLeftRadius", value),
        "BorderBottomRightRadius" => set_raw_corner_radius(node, "bottomRightRadius", value),
        "Padding" => {
            let v = value as f32;
            node.padding.top = v;
            node.padding.right = v;
            node.padding.bottom = v;
            node.padding.left = v;
        }
        "PaddingTop" => node.padding.top = value as f32,
        "PaddingRight" => node.padding.right = value as f32,
        "PaddingBottom" => node.padding.bottom = value as f32,
        "PaddingLeft" => node.padding.left = value as f32,
        "NineSliceTop" | "NineSliceBottom" | "NineSliceLeft" | "NineSliceRight" => {
            // Write to raw for renderer.
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap()),
                )
            });
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised number field '{}' = {}",
                field_name,
                value
            );
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap()),
                )
            });
        }
    }
}

