use crate::bb_loc::LocFetcher;
use super::modifiers_number::apply_number_field;
use crate::bb_scene::{BbNode, BbNodeType, BbValue};
use super::colors::{
    PaletteSources,
    color_style_role_for_field,
    color_style_token,
    ensure_border,
    parse_color_value,
    write_color_to_raw,
    write_color_token_to_raw,
};
pub(super) fn apply_inline_color_overlay(node: &mut BbNode, palette_source: &serde_json::Value) {
    if node.raw.get("FillColor").is_some() {
        return;
    }
    let overlay_enabled = node
        .raw
        .get("enableColorOverlay")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || node
            .raw
            .get("svgFill")
            .and_then(|v| v.get("enableColorOverlay"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if !overlay_enabled {
        return;
    }

    // Resolve the inline overlay colour with the role appropriate to the node
    // type (custom-shape fill = surface; icon/image overlay = foreground), so a
    // shape's `Accent1` resolves to the darker surface slot rather than the light
    // foreground slot.
    let role = color_style_role_for_field("FillColor", node);
    let color_value = node
        .raw
        .get("color")
        .or_else(|| node.raw.get("svgFill").and_then(|v| v.get("color")));
    if let Some(color) = color_value.and_then(|value| parse_color_value(value, palette_source, role)) {
        let token = color_value.and_then(color_style_token).map(str::to_owned);
        apply_color_field("FillColor", color, token.as_deref(), node);
        return;
    }
    // An overlay-enabled icon with no authored colour (`color: null`) tints to
    // the brand's primary foreground role `Base` — e.g. the MFD footer's nav
    // carats render brand-orange, not the SVG's own (dark) fill. Scoped to
    // `WidgetIcon` (monochrome glyphs): `enableColorOverlay: true` is the
    // universal editor default, so a `WidgetImage` keeps its own colour (a
    // Base default blue-cast the medical card photos and brown-washed the
    // power screen), and custom-shape fills without a colour keep their own
    // paint (an unconditional shape default regressed the target chevrons).
    if node.ty == BbNodeType::WidgetIcon {
        let base = serde_json::json!({"color": "Base", "alpha": 1.0});
        match parse_color_value(&base, palette_source, role) {
            Some(color) => apply_color_field("FillColor", color, Some("Base"), node),
            // The brand palette may live in an external HUD-style record not loaded
            // at scene-resolution time (MFD headers); still record the `Base` token
            // so the render-time colour resolver (which has the effective palette)
            // tints the glyph brand-orange instead of leaving the SVG's dark fill.
            None => write_color_token_to_raw("FillColor", Some("Base"), node),
        }
    }
}


/// Apply a single modifier to a node.
///
/// Parses the `field._Type_` discriminator and `field.field` name, then updates
/// the appropriate typed field on `node` (or writes to `node.raw` as a fallback).
pub(super) fn apply_modifier(
    modifier: &serde_json::Value,
    node: &mut BbNode,
    palettes: &PaletteSources<'_>,
    loc_fetcher: Option<&dyn LocFetcher>,
) {
    let Some((type_str, field_name, value)) = modifier_parts(modifier) else {
        return;
    };

    // Skip canvas-reference modifiers (already handled by bb_resolve).
    if type_str.ends_with("CanvasReferenceRecord") {
        return;
    }

    match type_str {
        "BuildingBlocks_FieldModifierString" => {
            if let Some(value) = value.and_then(|v| v.as_str()) {
                apply_string_field(field_name, value, node, loc_fetcher);
            }
        }
        "BuildingBlocks_FieldModifierNumber" => {
            if let Some(value) = value.and_then(|v| v.as_f64()) {
                apply_number_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierColor" => {
            if let Some(value) = value {
                let token = color_style_token(value).map(str::to_owned);
                let role = color_style_role_for_field(field_name, node);
                if let Some(color) = parse_color_value(value, palettes.for_field(field_name), role) {
                    apply_color_field(field_name, color, token.as_deref(), node);
                    // A styled BackgroundColor replaces the node's authored
                    // at-rest `background.color` (e.g. the MFD footer's
                    // authored Base@0.3 → styled Disabled@0.1), so downstream
                    // fill token/alpha readers see the styled value.
                    // An unconfigured `color: null` block is left untouched —
                    // its null-ness distinguishes an editor-default background
                    // from a configured-but-disabled one.
                    if field_name == "BackgroundColor"
                        && let Some(background) = node
                            .raw
                            .get_mut("background")
                            .and_then(|bg| bg.as_object_mut())
                        && background.get("color").is_some_and(|colour| !colour.is_null())
                    {
                        background.insert("color".to_string(), value.clone());
                    }
                } else {
                    // An entries-only container (no `colorStyles`, e.g. the power
                    // screen's defaultStyles/brandStyles) cannot resolve the RGBA
                    // here — still record the TOKEN so the render-time colour
                    // resolver (which has the effective palette) applies it
                    // ('System Icon Color' FillColor Accent2 on the `icon` tag).
                    // Clear any stale RGBA an earlier (lower-priority) pass wrote
                    // for this field — the same overwrite semantics as the
                    // resolved path; a leftover RGBA would shadow the token at
                    // draw (the medical close-button X kept its Base light-blue
                    // under the bioc ghost entry's Bright token). An UNCHANGED
                    // token keeps its already-resolved RGBA: re-applying the same
                    // entry (the deferred child-subtree pass) must be a no-op.
                    let existing_token = node
                        .raw
                        .get(format!("{field_name}Token"))
                        .and_then(|v| v.as_str());
                    if token.as_deref() != existing_token
                        && token.is_some()
                    {
                        node.raw
                            .as_object_mut()
                            .and_then(|obj| obj.remove(field_name));
                    }
                    write_color_token_to_raw(field_name, token.as_deref(), node);
                }
            }
        }
        "BuildingBlocks_FieldModifierBoolean" => {
            if let Some(value) = value.and_then(|v| v.as_bool()) {
                apply_boolean_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierEnumerated"
        | "BuildingBlocks_FieldModifierEnumeratedTypeImageScalingBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeHeightBehavior"
        | "BuildingBlocks_FieldModifierEnumeratedTypeFlexDirection"
        | "BuildingBlocks_FieldModifierEnumeratedTypeFlexAxisJustification"
        | "BuildingBlocks_FieldModifierEnumeratedTypeFlexCrossAxisJustification" => {
            if let Some(value) = value.and_then(|v| v.as_str()) {
                apply_enum_field(field_name, value, node);
            }
        }
        "BuildingBlocks_FieldModifierRecordRef"
        | "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord" => {
            if let Some(value) = value.and_then(|v| v.as_str()) {
                apply_record_ref_field(field_name, value, node);
            }
        }
        _ => {
            log::debug!(
                "bb_brand_apply: unrecognised modifier type '{}' for field '{}'",
                type_str,
                field_name
            );
            if let Some(value) = value {
                node.raw
                    .as_object_mut()
                    .and_then(|obj| obj.insert(field_name.to_string(), value.clone()));
            }
        }
    }
}

fn modifier_parts(modifier: &serde_json::Value) -> Option<(&str, &str, Option<&serde_json::Value>)> {
    let modifier_type = modifier.get("_Type_").and_then(|v| v.as_str());

    match modifier.get("field")? {
        serde_json::Value::String(field_name) => {
            let type_str = modifier_type?;
            let value = if type_str == "BuildingBlocks_FieldModifierColor" {
                modifier.get("color").or_else(|| modifier.get("value"))
            } else {
                modifier.get("value")
            };
            Some((type_str, field_name.as_str(), value))
        }
        serde_json::Value::Object(field) => {
            let type_str = field
                .get("_Type_")
                .and_then(|v| v.as_str())
                .or(modifier_type)?;
            let field_name = field
                .get("field")
                .and_then(|v| v.as_str())
                .or_else(|| match type_str {
                    "BuildingBlocks_FieldModifierRecordRefTypeFontStyleRecord" => Some("FontStyleRecord"),
                    "BuildingBlocks_FieldModifierEnumeratedTypeWidthBehavior" => Some("WidthBehavior"),
                    "BuildingBlocks_FieldModifierEnumeratedTypeHeightBehavior" => Some("HeightBehavior"),
                    "BuildingBlocks_FieldModifierEnumeratedTypeImageScalingBehavior" => Some("ImageScalingBehavior"),
                    "BuildingBlocks_FieldModifierEnumeratedTypeFlexDirection" => Some("FlexDirection"),
                    "BuildingBlocks_FieldModifierEnumeratedTypeFlexAxisJustification" => Some("FlexAxisJustification"),
                    "BuildingBlocks_FieldModifierEnumeratedTypeFlexCrossAxisJustification" => Some("FlexCrossAxisJustification"),
                    _ => None,
                })
                .unwrap_or("");
            let value = field
                .get("value")
                .or_else(|| field.get("color"))
                .or_else(|| modifier.get("value"))
                .or_else(|| modifier.get("color"));
            Some((type_str, field_name, value))
        }
        _ => None,
    }
}

/// Apply a string-typed modifier field.
///
/// When `loc_fetcher` is provided and `value` starts with `@`, it is resolved
/// through the localization fetcher.  Asset-path fields (`SvgPath`, `ImagePath`)
/// are intentionally NOT resolved — they reference files, not localized strings.
fn apply_string_field(field_name: &str, value: &str, node: &mut BbNode, loc_fetcher: Option<&dyn LocFetcher>) {
    match field_name {
        "SvgPath" | "ImagePath" => {
            // Write to node.raw for the renderer to pick up.
            node.raw
                .as_object_mut()
                .and_then(|obj| obj.insert(field_name.to_string(), serde_json::Value::String(value.to_string())));
        }
        _ => {
            // Resolve @KEY localization references if a fetcher is available.
            let resolved = if value.starts_with('@') {
                if let Some(fetcher) = loc_fetcher {
                    let param_input_values = node
                        .raw
                        .get("paramInputValues")
                        .and_then(|value| value.as_array())
                        .map(|values| values.as_slice())
                        .unwrap_or(&[]);
                    crate::bb_loc::resolve_loc_string(value, param_input_values, fetcher)
                } else {
                    value.to_string()
                }
            } else {
                value.to_string()
            };
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised string field '{}' = '{}'",
                field_name,
                resolved
            );
            node.raw
                .as_object_mut()
                .and_then(|obj| obj.insert(field_name.to_string(), serde_json::Value::String(resolved)));
        }
    }
}

/// Apply a color-typed modifier field.
fn apply_color_field(field_name: &str, color: [f32; 4], token: Option<&str>, node: &mut BbNode) {
    match field_name {
        "FillColor" | "StrokeColor" | "BackgroundColor" => {
            // Update node.background if it exists.
            if let Some(bg) = &mut node.background {
                bg.fill_colour = Some(color);
            }
            // Also write to raw for non-typed cases.
            write_color_to_raw(field_name, color, node);
            if token.is_some() {
                write_color_token_to_raw(field_name, token, node);
            } else {
                // A literal (`ColorSolid`) modifier carries no palette token;
                // drop a stale lower-tier token or it shadows the literal at
                // draw time (e.g. the button standard's solid-black icon
                // FillColor losing to the inline-overlay `Base` default).
                node.raw
                    .as_object_mut()
                    .and_then(|obj| obj.remove(&format!("{field_name}Token")));
            }
        }
        "BorderColor" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.colour = Some(color);
                border.right.colour = Some(color);
                border.bottom.colour = Some(color);
                border.left.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorTop" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.top.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorRight" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.right.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorBottom" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.bottom.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        "BorderColorLeft" => {
            ensure_border(node);
            if let Some(border) = &mut node.border {
                border.left.colour = Some(color);
            }
            write_color_token_to_raw(field_name, token, node);
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised color field '{}' = {:?}",
                field_name,
                color
            );
            write_color_to_raw(field_name, color, node);
            write_color_token_to_raw(field_name, token, node);
        }
    }
}

/// Apply a boolean-typed modifier field.
fn apply_boolean_field(field_name: &str, value: bool, node: &mut BbNode) {
    match field_name {
        "IsActive" => node.is_active = value,
        "SvgFlipHorizontal" | "SvgFlipVertical" => {
            // Write into the authored svgFill structure the IR's asset-layout
            // reader consumes (the MFD header's "Button Icon Flip" mirrors the
            // left nav arrow on instances under an h-align-left icon host).
            let key = if field_name == "SvgFlipHorizontal" {
                "flipHorizontal"
            } else {
                "flipVertical"
            };
            if node.raw.is_null() {
                node.raw = serde_json::Value::Object(serde_json::Map::new());
            }
            if let Some(raw_obj) = node.raw.as_object_mut() {
                let svg_fill = raw_obj
                    .entry("svgFill".to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(svg_obj) = svg_fill.as_object_mut() {
                    svg_obj.insert(key.to_string(), serde_json::Value::Bool(value));
                }
            }
        }
        "EnableBackground" | "EnableColorOverlay" | "EnableNineSliceRect" => {
            // Write to raw for renderer.
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Bool(value),
                )
            });
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised boolean field '{}' = {}",
                field_name,
                value
            );
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::Bool(value),
                )
            });
        }
    }
}

/// Convert a raw `WidthBehavior` / `HeightBehavior` JSON value into the correct
/// `BbValue` for the given numeric size `v`.  Falls back to `Fixed` when the
/// behavior field is absent or unrecognised.
pub(super) fn bb_value_with_raw_behavior(
    v: f32,
    raw_behavior: Option<&serde_json::Value>,
    current: &BbValue,
) -> BbValue {
    match raw_behavior.and_then(|b| b.as_str()) {
        Some("Percent") => BbValue::Percent(v),
        Some("Fixed") => BbValue::Fixed(v),
        Some(other) => BbValue::Other { value: v, behavior: other.to_string() },
        // No explicit *Behavior override: the number modifier changes the
        // VALUE and the node's authored behaviour is preserved (the
        // emissions clones are Percent-sized; a `SizeY 1.0` entry keeps them
        // parent fractions — Fixed(1) collapsed the header to 1px).
        None => match current {
            BbValue::Fixed(_) => BbValue::Fixed(v),
            BbValue::Percent(_) => BbValue::Percent(v),
            BbValue::Other { behavior, .. } => BbValue::Other {
                value: v,
                behavior: behavior.clone(),
            },
        },
    }
}

/// Apply an enumerated-typed modifier field.
fn apply_enum_field(field_name: &str, value: &str, node: &mut BbNode) {
    match field_name {
        "WidthBehavior" => {
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(field_name.to_string(), serde_json::Value::String(value.to_string()))
            });
            // Also retro-apply to the typed sizing field so ordering of
            // WidthBehavior vs SizeX modifiers doesn't matter.
            let current = match node.sizing.width {
                BbValue::Fixed(v) | BbValue::Percent(v) => v,
                BbValue::Other { value, .. } => value,
            };
            let sizing = node.sizing.width.clone();
            node.sizing.width = bb_value_with_raw_behavior(current, Some(&serde_json::Value::String(value.to_string())), &sizing);
        }
        "HeightBehavior" => {
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(field_name.to_string(), serde_json::Value::String(value.to_string()))
            });
            let current = match node.sizing.height {
                BbValue::Fixed(v) | BbValue::Percent(v) => v,
                BbValue::Other { value, .. } => value,
            };
            let sizing = node.sizing.height.clone();
            node.sizing.height = bb_value_with_raw_behavior(current, Some(&serde_json::Value::String(value.to_string())), &sizing);
        }
        "ImageScalingBehavior" => {
            // Write to raw for renderer.
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::String(value.to_string()),
                )
            });
        }
        // Flex enum modifiers rewrite the node's AUTHORED layoutPolicy the
        // layout engine reads (the drak emissions "Numbers Container" entry
        // turns the authored Row into a Column). A node without a flex
        // layoutPolicy is left untouched — these restyle an existing flex,
        // they don't create one.
        "FlexDirection" | "FlexAxisJustification" | "FlexCrossAxisJustification" => {
            let key = match field_name {
                "FlexDirection" => "direction",
                "FlexAxisJustification" => "axisJustification",
                _ => "crossAxisJustification",
            };
            if let Some(policy) = node
                .raw
                .get_mut("layoutPolicy")
                .and_then(|p| p.as_object_mut())
            {
                policy.insert(key.to_string(), serde_json::Value::String(value.to_string()));
            }
        }
        _ => {
            // Generic fallback → write to raw.
            log::debug!(
                "bb_brand_apply: unrecognised enum field '{}' = '{}'",
                field_name,
                value
            );
            node.raw.as_object_mut().and_then(|obj| {
                obj.insert(
                    field_name.to_string(),
                    serde_json::Value::String(value.to_string()),
                )
            });
        }
    }
}

/// Apply a record-ref-typed modifier field.
fn apply_record_ref_field(field_name: &str, value: &str, node: &mut BbNode) {
    // Treat as a string and store in node.raw.
    log::debug!(
        "bb_brand_apply: record-ref field '{}' = '{}'",
        field_name,
        value
    );
    node.raw.as_object_mut().and_then(|obj| {
        obj.insert(
            field_name.to_string(),
            serde_json::Value::String(value.to_string()),
        )
    });
}
