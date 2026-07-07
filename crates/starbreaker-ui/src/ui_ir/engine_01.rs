#[allow(unused_imports)]
use super::*;
// Consolidated engine chunk 01 (formerly: part_01.part, part_02.part, part_03.part, part_04.part, part_05.part, part_06.part).
//   part_01.part: Canonical UI intermediate representation (IR) schema and compiler.
//   part_05.part: Build the value-side (secondary) text style of a `ComponentLabelCaptionPair`. The
//   part_06.part: Whether `node`'s authored overflow clips its descendants (`Clip` or

// Canonical UI intermediate representation (IR) schema and compiler.
//
// This module defines a versioned, renderer-agnostic IR document that captures
// fidelity-critical UI data from a resolved BuildingBlocks scene.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use crate::bb_bindings::BindingResolver;
use crate::bb_layout;
use crate::bb_layout::{LayoutResult, Rect};
use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene};
use crate::defaults::DefaultValueRegistry;
use crate::pipeline::CanvasFetcher;

/// Current IR schema version.
pub const UI_IR_SCHEMA_VERSION: u32 = 1;

/// Renderer backend hint derived from scene content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRendererHint {
    Bb,
    Swf,
    Hybrid,
}

/// Canonical UI IR document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrDocument {
    pub schema_version: u32,
    pub canvas_guid: String,
    pub canvas_name: Option<String>,
    pub target_width: u32,
    pub target_height: u32,
    pub selected_style_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_swf_source: Option<String>,
    pub renderer_hint: UiRendererHint,
    pub confidence: u8,
    pub warnings: Vec<String>,
    pub unresolved_references: Vec<String>,
    pub resolved_asset_refs: Vec<String>,
    pub missing_asset_refs: Vec<String>,
    pub nodes: Vec<UiIrNode>,
}

/// One scene node in the canonical IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub children: Vec<u32>,
    pub node_type: String,
    pub name: String,
    pub is_active: bool,
    pub layer: i32,
    pub alpha: f32,
    pub anchor: [f32; 2],
    pub pivot: [f32; 2],
    /// In-plane rotation in degrees (`orientation.z + orientationOffset.z`),
    /// applied around the node's `pivot` at draw time. `None`/0 = no rotation.
    /// The velocity/g-force ball caps author `orientation.z = 90` on the
    /// left/right chevrons to point them outward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<f32>,
    pub authored_position: [f32; 2],
    pub authored_size: [UiIrValue; 2],
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow_mode: Option<String>,
    /// Pixel-space intersection of clipping ancestors (`overflow` Clip /
    /// ClipFade): the renderer must not paint this node outside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_rect: Option<UiIrRect>,
    pub computed_rect: UiIrRect,
    pub background_fill_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f32>,
    /// Per-corner background-fill geometry when the styled radii are
    /// NON-uniform or any corner is chamfered: radii in `[TL, TR, BR, BL]`
    /// order. A chamfered corner draws a straight cut of its radius instead of
    /// an arc (the button standards' Filled state — ledger 106). Uniform
    /// un-chamfered rounding stays on `corner_radius`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radii: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_chamfers: Option<[bool; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_fill_alpha: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_fill_colour_token: Option<String>,
    /// `WidgetCircle` solid fill: the `fillColor` ColorStyle token, set ONLY when
    /// `doFill` is true (the g-force ball's solid `circle_Cap*` dots). The
    /// `WidgetCircle` draw path returns early before the generic background-fill
    /// block, so this is its own surface-token fill (resolved at compose, like the
    /// other surface tokens, scaled by node alpha); `doFill: false` circles leave
    /// it `None` and keep stroking their outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circle_fill_colour_token: Option<String>,
    /// Debug-only (`SB_UI_STYLE_PROVENANCE=1`): per colour / visibility field,
    /// the cascade `pass/entry` that WON it
    /// (`{"BackgroundColor": "mfd_g_emissions/New Style"}`). Always None in
    /// normal compiles, so freezes / guards / representative-hashes are
    /// unaffected (ledger item A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_provenance: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmented_fill: Option<UiIrSegmentedFill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<UiIrPolygon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<UiIrBorder>,
    pub stroke_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_extent: Option<f32>,
    /// Widget-standard separator strip: the matching brand entry's
    /// Min/MaxSize clamp bounds the VISIBLE strip inside the authored slot
    /// box, placed by the entry's Anchor/Pivot (0.5/0.5 = centred). Set only
    /// when a separator widget-standard entry authors size clamps (e.g.
    /// uilo_a Horizontal Primary/Secondary/Tertiary = 6/4/2 px); wins over
    /// the svgFill `stroke_extent` fallback at draw time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator_strip: Option<UiIrSeparatorStrip>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour_blend_mode: Option<UiIrColourBlendMode>,
    pub icon_tint_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_tint_colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_preset: Option<String>,
    pub text_payload: Option<UiIrTextPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_text_payload: Option<UiIrTextPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_text_style: Option<UiIrTextStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meter_progress: Option<f32>,
    pub text_style: Option<UiIrTextStyle>,
    pub asset_ref: Option<String>,
    /// `primitiveSettings.primitiveMaterialPath` for `rendererType:"Primitive"`
    /// nodes (e.g. the radar `WindowContainer` `map_window.mtl` / the disc
    /// `Circle_Radial_Grid` `radial_grid.mtl`). In-memory only (`serde(skip)`):
    /// consumed by `ir_compose` to identify the radar RTT window + its disc
    /// texture; never serialized, so IR snapshots / freeze hashes are unaffected.
    #[serde(skip)]
    pub primitive_material: Option<String>,
    /// `primitiveSettings.UVStart` / `UVSize` for `rendererType:"Primitive"` nodes
    /// (e.g. the radar `HeadingTape`'s atlas window: `UVStart (-0.24, 0.44)`,
    /// `UVSize (18, 0.06)` — the tick-marker row tiled around the ring). In-memory
    /// only (`serde(skip)`): consumed by `ir_compose` to project the heading-tape
    /// ring; never serialized, so IR snapshots / freeze hashes are unaffected.
    #[serde(skip)]
    pub primitive_uv_start: Option<[f32; 2]>,
    #[serde(skip)]
    pub primitive_uv_size: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_layout: Option<UiIrAssetLayout>,
    pub custom_shape: Option<UiIrCustomShape>,
    pub style_tag_uuids: Vec<String>,
    pub resolved_style_tags: Vec<UiIrStyleTag>,
    /// `true` when this node's source BB widget has `rendererType == "Flash"`.
    /// When `true`, the hybrid renderer replaces this node's BB subtree with
    /// the resolved SWF stage content.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_flash_renderer: bool,
    /// `true` when this node's source BB text widget has `autoFontSize == true`.
    /// The engine grows/shrinks such text so it fills its container rect; the
    /// renderer's fit-to-rect uses this as the "grow" signal (otherwise it only
    /// shrinks on overflow). Render-only hint — skipped in the serialized IR so it
    /// does not perturb snapshot/freeze hashes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_font_size: bool,
    /// `true` when this image widget enables the colour overlay
    /// (`svgFill.enableColorOverlay`, or a styled `EnableColorOverlay`
    /// modifier). The draw stage uses it to give pure-white alpha-mask
    /// textures the brand `Base` overlay (the annunciator chiclet glow);
    /// coloured textures render untinted. Render-only hint — skipped in the
    /// serialized IR so it does not perturb snapshot/freeze hashes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub colour_overlay_enabled: bool,
}

impl UiIrNode {
    /// True iff this node would paint real BB content (so removing it from the
    /// BB pass and stamping a SWF stage over it would blank something visible).
    ///
    /// Used to distinguish a genuine full-stage SWF-host placeholder (a Flash
    /// node whose whole subtree paints nothing) from an ordinary
    /// `rendererType:"Flash"` primitive node that draws real BB content.
    ///
    // ponytail: this list is the drawable set as of the current UiIrNode —
    // paint-bearing FIELDS plus the one node_type-only paint (WidgetManufacturerLogo
    // draws from node_type alone at ir_compose/engine_01.rs:339; every other
    // node_type draw, e.g. WidgetCircle/WidgetSeparator, gates on a colour field
    // that IS listed). Ceiling: a NEW paint-bearing field OR a new node_type-only
    // paint path must be added here, or a node that paints via it reads as "paints
    // nothing" and could be blanked by the overlay. Safe direction is
    // over-inclusion (more nodes "paint" → fewer false full-stage hosts).
    pub fn paints_bb_content(&self) -> bool {
        self.is_active
            && (self.background_fill_colour.is_some()
                || self.background_fill_colour_token.is_some()
                || self.circle_fill_colour_token.is_some()
                || self.segmented_fill.is_some()
                || self.polygon.is_some()
                || self.border.is_some()
                || self.stroke_colour.is_some()
                || self.stroke_colour_token.is_some()
                || self.separator_strip.is_some()
                || self.icon_preset.is_some()
                || self.text_payload.is_some()
                || self.secondary_text_payload.is_some()
                || self.meter_progress.is_some()
                || self.asset_ref.is_some()
                || self.custom_shape.is_some()
                || self.primitive_material.is_some()
                || self
                    .node_type
                    .eq_ignore_ascii_case("BuildingBlocks_WidgetManufacturerLogo"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrAssetLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scaling_behavior: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contain_position_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contain_position_y: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip_horizontal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip_vertical: Option<bool>,
}

/// Typed representation of authored fixed/relative values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiIrValue {
    Fixed { value: f32 },
    Percent { value: f32 },
    Other { value: f32, behavior: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiIrColourBlendMode {
    SourceOver,
    Additive,
}

/// Pixel-space computed rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiIrRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrBorder {
    pub top: UiIrBorderSide,
    pub right: UiIrBorderSide,
    pub bottom: UiIrBorderSide,
    pub left: UiIrBorderSide,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrBorderSide {
    pub width: f32,
    pub colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour_token: Option<String>,
}

/// A `WidgetPolygon`'s regular-polygon shape (the power pip selector arrow:
/// 3 sides, startAngle 270, orientationOffset.z 90 — a right-pointing
/// triangle filled with the brand Bright role).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrPolygon {
    pub sides: u32,
    pub start_angle_deg: f32,
    pub rotation_deg: f32,
    pub do_fill: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_colour_token: Option<String>,
    pub fill_alpha: f32,
}

/// Data-driven separator strip bounds from the separator widget-standard's
/// matching brand entry: per-axis Min/MaxSize clamps on the VISIBLE strip
/// plus the entry's Anchor/Pivot placing it inside the authored slot box
/// (0.5/0.5 = centred — every observed standard authors centred strips).
/// Horizontal separators author the Y fields (uilo_a Primary MinSizeY =
/// MaxSizeY = 6), vertical ones the X fields; each axis applies
/// independently so mixed-authoring entries (drak V-Secondary: X clamp +
/// Y anchor only) stay faithful.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct UiIrSeparatorStrip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_w: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_w: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_h: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_h: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot_y: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrSegmentedFill {
    pub enabled: bool,
    pub angle: f32,
    pub segment_size: f32,
    pub segment_spacing_size: f32,
    pub segment_x_offset: f32,
    pub segmented_bar_fill: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_colour_token: Option<String>,
}

/// Text payload status carried by the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UiIrTextPayload {
    Resolved { text: String },
    UnresolvedKey { key: String },
    IntentionallyEmpty { key: Option<String> },
    Empty,
}

/// Typography/style attributes for text widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrTextStyle {
    pub font_record: Option<String>,
    pub resolved_font_record: Option<serde_json::Value>,
    pub font_size: UiIrValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<f32>,
    /// Per-glyph tracking in target px (brand LetterSpacing × the host-stage
    /// text scale); the renderer adds it to every character advance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f32>,
    pub alignment: String,
    #[serde(default = "default_vertical_alignment")]
    pub vertical_alignment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_to_parent_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_to_parent_y: Option<f32>,
    pub colour: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_style: Option<String>,
}

fn default_vertical_alignment() -> String {
    "Center".to_string()
}

fn asset_layout_from_raw(raw: &serde_json::Value) -> Option<UiIrAssetLayout> {
    let svg_fill = raw.get("svgFill")?;
    let scaling_behavior = svg_fill
        .get("scalingBehavior")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let contain_position_x = svg_fill
        .get("containPositionX")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32);
    let contain_position_y = svg_fill
        .get("containPositionY")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32);
    let flip_horizontal = svg_fill
        .get("flipHorizontal")
        .and_then(|value| value.as_bool())
        .filter(|value| *value);
    let flip_vertical = svg_fill
        .get("flipVertical")
        .and_then(|value| value.as_bool())
        .filter(|value| *value);

    if scaling_behavior.is_none()
        && contain_position_x.is_none()
        && contain_position_y.is_none()
        && flip_horizontal.is_none()
        && flip_vertical.is_none()
    {
        return None;
    }

    Some(UiIrAssetLayout {
        scaling_behavior,
        contain_position_x,
        contain_position_y,
        flip_horizontal,
        flip_vertical,
    })
}

fn effective_font_record(
    node: &BbNode,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
) -> Option<String> {
    node.raw
        .get("FontStyleRecord")
        .or_else(|| node.raw.get("fontStyle"))
        .or_else(|| node.raw.get("fontRecord"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| node.text.as_ref().and_then(|text| text.font_record.clone()))
        .or_else(|| {
            label_style_name_from_raw(node).and_then(|style| {
                standard_text_style_keys(&style)
                    .into_iter()
                    .find_map(|key| {
                        standard_text_styles
                            .get(&key)
                            .and_then(|standard| standard.font_record.clone())
                    })
            })
        })
}

/// Shape metadata for custom-shape widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrCustomShape {
    pub shape_type: Option<String>,
    pub shape: Option<String>,
    pub svg_path: Option<String>,
    pub render_shape: Option<bool>,
    pub enable_nine_slice_rect: Option<bool>,
    pub nine_slice_rect: Option<[f32; 4]>,
    pub nine_slice_scale: Option<f32>,
}

/// Resolved style-tag metadata from source `styleTags[]` entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiIrStyleTag {
    pub uuid: String,
    pub tag_name: Option<String>,
}

pub fn validate_ui_ir_document(document: &UiIrDocument) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let mut seen_ids = HashSet::new();

    if document.schema_version != UI_IR_SCHEMA_VERSION {
        errors.push(format!(
            "schema_version mismatch: expected {}, got {}",
            UI_IR_SCHEMA_VERSION, document.schema_version
        ));
    }

    if document.canvas_guid.trim().is_empty() {
        errors.push("canvas_guid must not be empty".to_string());
    }

    if document.target_width == 0 || document.target_height == 0 {
        errors.push("target dimensions must be non-zero".to_string());
    }

    if document.confidence > 100 {
        errors.push("confidence must be in 0..=100".to_string());
    }

    for node in &document.nodes {
        if !seen_ids.insert(node.id) {
            errors.push(format!("duplicate node id {}", node.id));
        }
        if node.name.trim().is_empty() {
            errors.push(format!("node {} has empty name", node.id));
        }
        if node.node_type.trim().is_empty() {
            errors.push(format!("node {} has empty node_type", node.id));
        }
        if node.computed_rect.w < 0.0 || node.computed_rect.h < 0.0 {
            errors.push(format!(
                "node {} has negative computed size ({}, {})",
                node.id, node.computed_rect.w, node.computed_rect.h
            ));
        }
    }

    for node in &document.nodes {
        if let Some(parent_id) = node.parent_id {
            if !seen_ids.contains(&parent_id) {
                errors.push(format!("node {} references missing parent {}", node.id, parent_id));
            }
        }
        for child_id in &node.children {
            if !seen_ids.contains(child_id) {
                errors.push(format!("node {} references missing child {}", node.id, child_id));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Stable SHA-256 hash of a UI IR document.
///
/// The hash is computed from canonical JSON serialization (`serde_json` output
/// from typed structs and BTreeMap-backed sources) to support deterministic
/// fixture comparisons across reruns.
pub fn stable_hash_ui_ir(document: &UiIrDocument) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(document)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{:x}", digest))
}

/// Compile a canonical UI IR document from a resolved scene.
/// Draw-faithful text measurement supplied by the caller that owns the SWF
/// font assets the renderer will draw with (`pipeline`). The pre-layout
/// annotation pass uses it to write `_DrawTextWidthPx_`/`_DrawTextHeightPx_`
/// so `bb_layout`'s intrinsic text boxes hug the glyphs the draw will paint
/// (the SWF path renders at the IR font size with NO TTF calibration — a TTF
/// estimate overshoots ~1.5×; see crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md #3).
/// Returns `None` when no draw font would be selected for the element (the
/// renderer falls back to the TTF path, whose estimate bb_layout already
/// models).
pub trait DrawTextMeasure {
    fn measure_px(
        &self,
        font_symbol: Option<&str>,
        label_style: Option<&str>,
        text: &str,
        font_px: f32,
        letter_spacing_px: f32,
    ) -> Option<(f32, f32)>;

    /// Per-word draw advances for the SAME glyph machinery `measure_px` uses:
    /// `(word_advances, space_cost, line_box)`. `word_advances` follows the
    /// text's word order with `None` entries marking paragraph (`\n`) breaks;
    /// `space_cost` is the inter-word pen advance (space glyph + letter
    /// spacing); `line_box` is the single-line box height. Advances are
    /// additive (a line's advance = Σ words + spaces), so `bb_layout` can
    /// reproduce the draw's greedy wrap exactly for Auto-height content fits.
    /// Default: unavailable (TTF fallback path keeps its estimate).
    fn measure_word_advances_px(
        &self,
        _font_symbol: Option<&str>,
        _label_style: Option<&str>,
        _text: &str,
        _font_px: f32,
        _letter_spacing_px: f32,
    ) -> Option<(Vec<Option<f32>>, f32, f32)> {
        None
    }
}

pub fn compile_ui_ir_from_scene(
    scene: &BbScene,
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    canvas_guid: &str,
    canvas_name: Option<&str>,
    target_size: (u32, u32),
    defaults: &DefaultValueRegistry,
    selected_style_source: Option<String>,
    selected_swf_source: Option<String>,
    unresolved_references: &[String],
    resolved_asset_refs: Vec<String>,
    missing_asset_refs: Vec<String>,
    confidence: u8,
) -> UiIrDocument {
    compile_ui_ir_from_scene_with_animation_sample(
        scene,
        canvas_fetcher,
        canvas_guid,
        canvas_name,
        target_size,
        defaults,
        selected_style_source,
        selected_swf_source,
        unresolved_references,
        resolved_asset_refs,
        missing_asset_refs,
        None,
        confidence,
        1.0,
        None,
        false,
        false,
    )
}

pub fn compile_ui_ir_from_scene_with_animation_sample(
    scene: &BbScene,
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    canvas_guid: &str,
    canvas_name: Option<&str>,
    target_size: (u32, u32),
    defaults: &DefaultValueRegistry,
    selected_style_source: Option<String>,
    selected_swf_source: Option<String>,
    unresolved_references: &[String],
    resolved_asset_refs: Vec<String>,
    missing_asset_refs: Vec<String>,
    animation_sample_percent: Option<f32>,
    confidence: u8,
    design_text_scale: f32,
    text_measure: Option<&dyn DrawTextMeasure>,
    cover_fit: bool,
    mesh_aspect_fill: bool,
) -> UiIrDocument {
    let mut layout_scene = scene.clone();
    // Annunciator 25px edge frame — measured engine pin (fallback register).
    //
    // Derivation attempt exhausted (plan P5.2, 2026-06-12): the value is in
    // NO authored source — the content canvas authors padding 15 TRBL
    // (h_eng_annunciator.json) with no drak brand override (drak's brand
    // entry is only a background-image swap; mrai/rsi DO author
    // Root_Annunciator_Items Padding* overrides via brand entries — none of
    // them 25 either), M_Physical_Screen frames two zero-padding
    // full-percent nodes, staticVariables are null on all three records,
    // and the full BuildingBlocks_root.swf AVM1 dump has no 25-valued
    // pushes (examples/swf_avm1_dump.rs). Like the MFD 44px content-view
    // inset this is a C++-host-side placement; the capture-measured insets
    // (~33/28/17px under skew) bracket the tuned uniform 25. Replace only
    // if an engine-side data source ever surfaces; the brand Padding*
    // overrides above become relevant when mrai/rsi ships onboard.
    if let Some(root_annunciator_items) = layout_scene
        .nodes
        .values_mut()
        .find(|node| node.name.eq_ignore_ascii_case("root_annunciator_items"))
    {
        let vertical_scale = if scene.canvas_size.1 > 0.0 {
            target_size.1 as f32 / scene.canvas_size.1
        } else {
            1.0
        };
        let desired_top_bottom_padding = if vertical_scale > 0.0 {
            25.0 / vertical_scale
        } else {
            25.0
        };

        root_annunciator_items.padding.top = root_annunciator_items
            .padding
            .top
            .max(desired_top_bottom_padding);
        root_annunciator_items.padding.left = root_annunciator_items.padding.left.max(25.0);
        root_annunciator_items.padding.right = root_annunciator_items.padding.right.max(25.0);
        root_annunciator_items.padding.bottom = root_annunciator_items
            .padding
            .bottom
            .max(desired_top_bottom_padding);
    }

    let binding_resolver = BindingResolver::from_operations(&scene.operations);
    let style_font_sizes = collect_style_font_sizes(scene);
    let standard_text_styles =
        collect_standard_text_styles(canvas_fetcher, selected_style_source.as_deref(), canvas_name);
    // Portrait cockpit-screen font scale (the LR-indicator) — 1.0 for every other
    // screen (see `portrait_font_screen_scale`). It must apply to BOTH the
    // layout-time intrinsic text measurement (so a content-fit text field GROWS
    // to the enlarged glyphs instead of clipping them) and the final IR font size
    // (the post-pass below) so measure == draw.
    let font_screen_scale =
        portrait_font_screen_scale(scene.canvas_size, target_size, cover_fit, mesh_aspect_fill);
    // Layout-time intrinsic text measurement must use the EFFECTIVE (styled)
    // font size, not the authored editor size — annotate before layout.
    annotate_effective_font_px(
        &mut layout_scene,
        scene,
        &binding_resolver,
        defaults,
        &style_font_sizes,
        &standard_text_styles,
        design_text_scale,
        font_screen_scale,
        canvas_fetcher,
        text_measure,
    );
    let layout = bb_layout::layout_with_animation_sample(
        &layout_scene,
        target_size.0,
        target_size.1,
        animation_sample_percent,
        cover_fit,
        mesh_aspect_fill,
    );

    let has_text = scene.nodes.values().any(|n| n.text.is_some());
    let has_custom_shape = scene
        .nodes
        .values()
        .any(|n| n.ty == BbNodeType::WidgetCustomShape);
    let has_selected_swf_source = selected_swf_source.is_some();

    let renderer_hint = match (has_selected_swf_source, has_text, has_custom_shape) {
        (true, true, true) => UiRendererHint::Hybrid,
        (true, false, true) => UiRendererHint::Swf,
        _ => UiRendererHint::Bb,
    };

    let mut warnings = Vec::new();
    if scene.roots.is_empty() {
        warnings.push("scene has no root nodes".to_string());
    }
    if !missing_asset_refs.is_empty() {
        warnings.push(format!(
            "{} asset reference(s) could not be resolved",
            missing_asset_refs.len()
        ));
    }
    if has_custom_shape && !has_selected_swf_source {
        warnings.push(
            "custom-shape content present but no SWF source was resolved; using BB renderer"
                .to_string(),
        );
    }

    let mut ordered_ids = layout.draw_order.clone();
    let mut seen_ids: std::collections::HashSet<BbNodeId> =
        ordered_ids.iter().copied().collect();
    for &id in scene.nodes.keys() {
        if seen_ids.insert(id) {
            ordered_ids.push(id);
        }
    }

    // A cockpit HUD canvas (`HC_HUD_*`/`H_HUD_*`/`H_Eng_*`) styles its generic
    // overlay icons differently from the MFD masters (see
    // `custom_shape_overlay_icon_default`).
    let is_hud_canvas = canvas_name
        .map(|name| name.strip_prefix("BuildingBlocks_Canvas.").unwrap_or(name))
        .map(crate::bb_brand_style::is_cockpit_hud_canvas)
        .unwrap_or(false);
    let mut nodes: Vec<UiIrNode> = build_ui_ir_nodes(
        ordered_ids,
        scene,
        &layout,
        &binding_resolver,
        renderer_hint,
        has_selected_swf_source,
        &style_font_sizes,
        &standard_text_styles,
        canvas_fetcher,
        defaults,
        animation_sample_percent,
        selected_style_source.as_deref(),
        design_text_scale,
        is_hud_canvas,
    );

    let unresolved_count = nodes
        .iter()
        .flat_map(|node| node.text_payload.iter().chain(node.secondary_text_payload.iter()))
        .filter(|payload| matches!(payload, UiIrTextPayload::UnresolvedKey { .. }))
        .count() as u8;
    if unresolved_count > 0 {
        warnings.push(format!(
            "{} unresolved text key(s) present in scene",
            unresolved_count
        ));
    }

    let missing_style_semantics: Vec<String> = nodes
        .iter()
        .flat_map(missing_strict_renderer_style_semantics)
        .collect();
    if !missing_style_semantics.is_empty() {
        warnings.push(format!(
            "{} strict-renderer style semantic gap(s) detected",
            missing_style_semantics.len()
        ));
        warnings.extend(missing_style_semantics.into_iter().take(8));
    }

    let computed_confidence = confidence
        .saturating_sub(unresolved_count.saturating_mul(10))
        .min(100);

    // Apply the portrait cockpit-screen font scale (computed above, 1.0 for every
    // non-portrait screen) to the final IR font sizes — the layout already grew the
    // content-fit fields to match (`annotate_effective_font_px`), so measure == draw.
    if (font_screen_scale - 1.0).abs() > f32::EPSILON {
        scale_text_node_fonts(&mut nodes, font_screen_scale);
    }

    UiIrDocument {
        schema_version: UI_IR_SCHEMA_VERSION,
        canvas_guid: canvas_guid.to_string(),
        canvas_name: canvas_name.map(str::to_string),
        target_width: target_size.0,
        target_height: target_size.1,
        selected_style_source,
        selected_swf_source,
        renderer_hint,
        confidence: computed_confidence,
        warnings,
        unresolved_references: unresolved_references.to_vec(),
        resolved_asset_refs,
        missing_asset_refs,
        nodes,
    }
}

/// The render-time font scale for a cockpit fill-branch screen (called from the
/// IR compile). A `useRaw` cockpit screen that FILLS its mesh-aspect target
/// (`cover_fit && mesh_aspect_fill`) stretches the authored 1920×1080 landscape
/// canvas to the mesh. Geometry fills non-uniformly, but a Fixed font authored
/// for the canvas is drawn at design px — fine on a square/landscape mesh (the
/// ball gauges, countermeasures), but far too SMALL on a PORTRAIT mesh
/// (`target_h > target_w`, e.g. the LR-indicator's 0.64 screen, whose cells fill
/// vertically by ~2.78×). There the font scales by the vertical fill axis
/// (`max(sx, sy)`, which is `sy` for a landscape canvas on a portrait mesh).
/// Returns 1.0 for every non-portrait / non-fill screen, so only the portrait
/// LR-indicator is affected today (frozen baselines untouched).
pub(crate) fn portrait_font_screen_scale(
    canvas_size: (f32, f32),
    target_size: (u32, u32),
    cover_fit: bool,
    mesh_aspect_fill: bool,
) -> f32 {
    let (tw, th) = (target_size.0 as f32, target_size.1 as f32);
    if cover_fit && mesh_aspect_fill && th > tw && canvas_size.0 > 0.0 && canvas_size.1 > 0.0 {
        let sx = tw / canvas_size.0;
        let sy = th / canvas_size.1;
        sx.max(sy)
    } else {
        1.0
    }
}

/// Multiply every text node's Fixed font size and target-px text spacings by
/// `scale` (the portrait font screen scale). Called only when `scale != 1.0`, so
/// it is a strict no-op for every non-portrait screen.
fn scale_text_node_fonts(nodes: &mut [UiIrNode], scale: f32) {
    fn scale_style(style: &mut UiIrTextStyle, scale: f32) {
        if let UiIrValue::Fixed { value } = &mut style.font_size {
            *value *= scale;
        }
        if let Some(ls) = style.line_spacing.as_mut() {
            *ls *= scale;
        }
        if let Some(letter) = style.letter_spacing.as_mut() {
            *letter *= scale;
        }
    }
    for node in nodes.iter_mut() {
        if let Some(style) = node.text_style.as_mut() {
            scale_style(style, scale);
        }
        if let Some(style) = node.secondary_text_style.as_mut() {
            scale_style(style, scale);
        }
    }
}

fn missing_strict_renderer_style_semantics(node: &UiIrNode) -> Vec<String> {
    let mut warnings = Vec::new();
    let node_type = node.node_type.trim();

    if node_type.eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")
        || node_type.eq_ignore_ascii_case("widget_separator")
    {
        let has_separator_colour = node.stroke_colour.is_some()
            || node.stroke_colour_token.is_some()
            || node.background_fill_colour.is_some()
            || node.background_fill_colour_token.is_some();
        if !has_separator_colour {
            warnings.push(format!(
                "node {} ({}) separator missing stroke/background colour semantics",
                node.id, node.name
            ));
        }
    }

    if let Some(border) = node.border.as_ref() {
        if border.top.width > 0.0 && border.top.colour.is_none() && border.top.colour_token.is_none() {
            warnings.push(format!(
                "node {} ({}) border.top width {} missing colour semantics",
                node.id, node.name, border.top.width
            ));
        }
        if border.right.width > 0.0
            && border.right.colour.is_none()
            && border.right.colour_token.is_none()
        {
            warnings.push(format!(
                "node {} ({}) border.right width {} missing colour semantics",
                node.id, node.name, border.right.width
            ));
        }
        if border.bottom.width > 0.0
            && border.bottom.colour.is_none()
            && border.bottom.colour_token.is_none()
        {
            warnings.push(format!(
                "node {} ({}) border.bottom width {} missing colour semantics",
                node.id, node.name, border.bottom.width
            ));
        }
        if border.left.width > 0.0 && border.left.colour.is_none() && border.left.colour_token.is_none() {
            warnings.push(format!(
                "node {} ({}) border.left width {} missing colour semantics",
                node.id, node.name, border.left.width
            ));
        }
    }

    warnings
}


/// Write each measurable text field's EFFECTIVE (styled) font size into the
/// layout scene's raw (`_EffectiveFontPx_`) so `bb_layout`'s intrinsic text
/// measurement matches what the renderer will draw (the brand Heading sizes,
/// not the authored editor size). Auto-fit text is skipped — its size
/// depends on the laid-out rect.
///
/// When a [`DrawTextMeasure`] is supplied, additionally measure the resolved
/// text through the DRAW-side glyph machinery and write
/// `_DrawTextWidthPx_`/`_DrawTextHeightPx_`: the SWF draw path renders at the
/// IR font size (no TTF calibration), so only a measure over the same font
/// the renderer selects keeps intrinsic boxes hugging the painted glyphs
/// (crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md catalog #3). The measured size mirrors
/// the IR build path: a styled (brand-table) size verbatim, a plain size with
/// the font record's imageSizePercent boost.
#[allow(clippy::too_many_arguments)]
fn annotate_effective_font_px(
    layout_scene: &mut BbScene,
    scene: &BbScene,
    binding_resolver: &BindingResolver,
    defaults: &DefaultValueRegistry,
    style_font_sizes: &HashMap<String, f32>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
    design_text_scale: f32,
    font_screen_scale: f32,
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    text_measure: Option<&dyn DrawTextMeasure>,
) {
    let ids: Vec<BbNodeId> = layout_scene
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.is_active
                && node.text.is_some()
                && node.raw.get("_ResolvedText_").is_some()
                && !auto_font_size_enabled(&node.raw)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        let Some(node) = scene.nodes.get(&id) else { continue };
        let Some(text) = node.text.as_ref() else { continue };
        let (font_size, is_styled) = resolve_effective_font_size(
            id,
            node,
            text,
            0.0,
            node.raw.get("_ResolvedText_").and_then(|v| v.as_str()),
            scene,
            binding_resolver,
            defaults,
            style_font_sizes,
            standard_text_styles,
            design_text_scale,
        );
        let UiIrValue::Fixed { value: font_px } = font_size else {
            continue;
        };
        if font_px <= 0.0 {
            continue;
        }
        // Portrait cockpit-screen font scale (LR-indicator): grow the measured text
        // so a content-fit field fits the enlarged glyphs. 1.0 (no-op) elsewhere.
        let font_px = font_px * font_screen_scale;
        let draw_metrics = text_measure.and_then(|measure| {
            let resolved_text = node.raw.get("_ResolvedText_").and_then(|v| v.as_str())?;
            let font_record = effective_font_record(node, standard_text_styles);
            let resolved_font_record = font_record
                .as_deref()
                .and_then(|record_ref| resolve_record(canvas_fetcher, &[record_ref]));
            // The draw consumes the IR text_style font size: styled sizes
            // verbatim, plain sizes with the imageSizePercent boost — mirror
            // that here so measure == draw.
            let draw_font_size = apply_font_image_size_percent(
                UiIrValue::Fixed { value: font_px },
                resolved_font_record.as_ref(),
                is_styled,
                design_text_scale,
            );
            let UiIrValue::Fixed { value: draw_font_px } = draw_font_size else {
                return None;
            };
            let font_symbol = resolved_font_record
                .as_ref()
                .map(|record| record.get("_RecordValue_").unwrap_or(record))
                .and_then(|value| value.get("font"))
                .and_then(|value| value.as_str())
                .filter(|symbol| !symbol.is_empty())
                .map(str::to_owned);
            let label_style = label_style_name_from_raw(node);
            let letter_spacing_px = brand_style_letter_spacing(
                label_style.as_deref(),
                standard_text_styles,
            )
            .map(|spacing| spacing * design_text_scale)
            .unwrap_or(0.0);
            let size = measure.measure_px(
                font_symbol.as_deref(),
                label_style.as_deref(),
                resolved_text,
                draw_font_px,
                letter_spacing_px,
            )?;
            // Per-word advances let bb_layout reproduce the draw's greedy wrap
            // for Auto-height content fits (a single-line intrinsic clipped
            // wrapped labels — the transit button's CALL/ELEVATOR, ledger 106).
            let words = measure.measure_word_advances_px(
                font_symbol.as_deref(),
                label_style.as_deref(),
                resolved_text,
                draw_font_px,
                letter_spacing_px,
            );
            Some((size, words))
        });
        if let Some(node) = layout_scene.nodes.get_mut(&id)
            && let Some(map) = node.raw.as_object_mut()
        {
            map.insert("_EffectiveFontPx_".to_string(), serde_json::json!(font_px));
            if let Some(((draw_w, draw_h), word_metrics)) = draw_metrics {
                if draw_w > 0.0 && draw_h > 0.0 {
                    map.insert("_DrawTextWidthPx_".to_string(), serde_json::json!(draw_w));
                    map.insert("_DrawTextHeightPx_".to_string(), serde_json::json!(draw_h));
                }
                if let Some((words, space_cost, line_box)) = word_metrics
                    && line_box > 0.0
                {
                    map.insert(
                        "_DrawTextWordAdvancesPx_".to_string(),
                        serde_json::json!(words),
                    );
                    map.insert(
                        "_DrawTextSpaceCostPx_".to_string(),
                        serde_json::json!(space_cost),
                    );
                    map.insert(
                        "_DrawTextLineBoxPx_".to_string(),
                        serde_json::json!(line_box),
                    );
                }
            }
        }
    }
}

fn build_ui_ir_nodes(
    ordered_ids: Vec<BbNodeId>,
    scene: &BbScene,
    layout: &LayoutResult,
    binding_resolver: &BindingResolver,
    renderer_hint: UiRendererHint,
    has_selected_swf_source: bool,
    style_font_sizes: &HashMap<String, f32>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
    canvas_fetcher: Option<&dyn CanvasFetcher>,
    defaults: &DefaultValueRegistry,
    animation_sample_percent: Option<f32>,
    selected_style_source: Option<&str>,
    design_text_scale: f32,
    is_hud_canvas: bool,
) -> Vec<UiIrNode> {
    let mut nodes = Vec::with_capacity(ordered_ids.len());
    for id in ordered_ids {
        let Some(node) = scene.nodes.get(&id) else {
            continue;
        };
        let layout_rect = layout.rects.get(&id).copied().unwrap_or_default();
        let has_text_intent = node_has_text_intent(node);
        let resolved_text = has_text_intent
            .then(|| binding_resolver.resolve_text_detailed(id, &node.raw, defaults));
        // A ComponentLabelCaptionPair's authored `show` flags gate each text:
        // the engine hides a `show:false` label/caption outright (the
        // lift-call console's floor heading authors label show=false +
        // caption show=true, so only the floor-name caption renders).
        let is_label_caption_pair = node_type_name(&node.ty)
            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair");
        let pair_label_hidden = is_label_caption_pair
            && node
                .raw
                .get("labelProperties")
                .and_then(|lp| lp.get("show"))
                .and_then(|v| v.as_bool())
                == Some(false);
        let pair_caption_hidden = is_label_caption_pair
            && node
                .raw
                .get("captionProperties")
                .and_then(|cp| cp.get("show"))
                .and_then(|v| v.as_bool())
                == Some(false);
        let text_payload = if pair_label_hidden {
            None
        } else {
            resolved_text.as_ref().map(|resolved| {
                // An empty component-parameter-driven label (inactive alert at rest)
                // stays blank — don't let classify re-derive its placeholder label.
                if resolved.text.trim().is_empty()
                    && binding_resolver.is_component_param_label(id, &node.raw)
                {
                    UiIrTextPayload::Empty
                } else {
                    classify_text_payload(Some(resolved.text.as_str()), &node.raw, defaults)
                }
            })
        };
        let resolved_style_tags = resolved_style_tags_for_node(
            canvas_fetcher,
            scene,
            node,
            id,
            &binding_resolver,
            defaults,
        );
        // The node's OWN tags (no ancestor inheritance) — used for icon/shape
        // tinting so a raster image isn't tinted by an inherited container tag.
        let own_style_tags = own_style_tags_for_node(
            canvas_fetcher,
            node,
            id,
            &binding_resolver,
            defaults,
        );
                    // A brand entry that REPLACES the image (styled raw
                    // `ImagePath` differing from the authored `imagePath`) is
                    // real screen art, not a placeholder: the annunciator's
                    // `image_BG` swaps to `DRAK_Background_anunciators.tif`
                    // — the warm backplate the reference shows — and must
                    // not be suppressed.
                    let has_styled_image_override = node
                        .raw
                        .get("ImagePath")
                        .and_then(|v| v.as_str())
                        .is_some_and(|styled| {
                            node.raw
                                .get("imagePath")
                                .and_then(|v| v.as_str())
                                .is_none_or(|authored| !styled.eq_ignore_ascii_case(authored))
                        });
                    let suppress_placeholder_background_image = has_selected_swf_source
                        && matches!(renderer_hint, UiRendererHint::Bb)
                        && matches!(node.ty, BbNodeType::WidgetImage)
                        && !has_styled_image_override
                        && resolved_style_tags.iter().any(|tag| {
                        tag.tag_name
                            .as_deref()
                            .is_some_and(|name| name.eq_ignore_ascii_case("ScreenNameBackground"))
                        });
        let mut style_tag_uuids = node.style_tag_uuids.clone();
        for tag in &resolved_style_tags {
            if !style_tag_uuids.iter().any(|existing| existing == &tag.uuid) {
                style_tag_uuids.push(tag.uuid.clone());
            }
        }
        let font_record = effective_font_record(node, &standard_text_styles);
        let resolved_font_record = font_record
            .as_deref()
            .and_then(|record_ref| resolve_record(canvas_fetcher, &[record_ref]));
        let label_style = label_style_name_from_raw(node);
        let text_style = if let Some(text) = node.text.as_ref() {
            let (font_size, is_styled) = resolve_effective_font_size(
                id,
                node,
                text,
                layout_rect.h,
                text_payload.as_ref().and_then(resolved_text_from_payload),
                scene,
                &binding_resolver,
                defaults,
                &style_font_sizes,
                &standard_text_styles,
                design_text_scale,
            );
            // imageSizePercent compensation: plain always; styled too on the
            // GFx-host path (see apply_font_image_size_percent).
            let font_size = apply_font_image_size_percent(
                font_size,
                resolved_font_record.as_ref(),
                is_styled,
                design_text_scale,
            );
            // Styled text renders at its full brand-table nominal size. (A prior
            // ~0.98 "all-caps display reduction" was removed: it was a registered
            // fudge whose stated 2% caps-width-overshoot rationale did not hold up
            // — the reference in-game RTT capture is uniformly ~2% small, so the
            // reduction shrank text below its true engine size.)
            Some(UiIrTextStyle {
                font_record: font_record.clone(),
                resolved_font_record: resolved_font_record.clone(),
                line_spacing: resolve_effective_line_spacing(node, &font_size, &standard_text_styles, design_text_scale),
                letter_spacing: brand_style_letter_spacing(label_style.as_deref(), &standard_text_styles)
                    .map(|spacing| spacing * design_text_scale),
                font_size,
                alignment: text.alignment.clone(),
                vertical_alignment: node
                    .raw
                    .get("verticalTextAlignment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Center")
                    .to_string(),
                anchor_to_parent_x: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentX"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                anchor_to_parent_y: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentY"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                colour: text
                    .colour
                    .or_else(|| fill_colour_from_raw_for_text(&node.raw))
                    .or_else(|| {
                        // An authored brand FillColor role outside the
                        // token-stable namespace (e.g. the MFD hud H1's
                        // `Accent2`) carries its enum-resolved RGBA explicitly;
                        // a node-level colour/tag token above still wins.
                        if text_colour_token_from_raw(&node.raw).is_none()
                            && node_colour_directive_token(
                                &resolved_style_tags,
                                label_style.as_deref(),
                            )
                            .is_none()
                        {
                            brand_text_style_fill_rgba(label_style.as_deref(), &standard_text_styles)
                        } else {
                            None
                        }
                    }),
                colour_token: text_colour_token_from_raw(&node.raw).or_else(|| {
                    semantic_text_colour_token_from_style_tags(&resolved_style_tags, label_style.as_deref())
                }).or_else(|| {
                    // A text field inherits its brand text style's authored
                    // FillColor (the game's per-style colour role, e.g. s_bioc
                    // H1/H3 = Base light-blue → "Drake Clipper"/tier "T3";
                    // s_bioc H2 = Bright → the medical menu option headers).
                    // Style tags that resolve no colour (UI_Generic_Flag_03)
                    // do NOT block this fallback — a tag is only an override
                    // when an entry/directive actually maps it to a colour
                    // (resolved above).
                    brand_text_style_colour_token(label_style.as_deref(), &standard_text_styles)
                }).or_else(|| {
                    default_style_text_colour_token_from_raw(&node.raw, &node.ty, false)
                }),
                label_style: label_style.clone(),
            })
        } else if text_payload.is_some() {
            let alignment = node
                .raw
                .get("textAlignment")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    node.raw
                        .get("labelProperties")
                        .and_then(|lp| lp.get("textAlignment"))
                        .and_then(|v| v.as_str())
                })
                .or_else(|| {
                    is_label_caption_pair
                        .then(|| pair_component_alignment(&node.raw))
                        .flatten()
                })
                .unwrap_or("Left")
                .to_string();

            // A caption-pair label (e.g. MedGel "MEDGELS", style Heading3) renders at its
            // named brand style verbatim — the same way the plain WidgetTextField path
            // does (LocationName, also Heading3, matches the reference). Prefer the brand
            // FontSize; only fall back to the rect ladder + imageSizePercent boost when
            // the style has no brand-table entry.
            let font_size_value = if let Some(size) =
                brand_style_font_size(label_style.as_deref(), &standard_text_styles)
            {
                apply_font_image_size_percent(
                    UiIrValue::Fixed { value: size * design_text_scale },
                    resolved_font_record.as_ref(),
                    true,
                    design_text_scale,
                )
            } else {
                // The hand-tuned per-style rect ladder that used to sit here
                // (textfield_fallback_font_size_from_signals) was deleted
                // 2026-06-12: no frozen pin referenced it (remediation plan
                // Phase 2/3 audit) — the brand table covers the styled cases.
                let font_size = node
                    .raw
                    .get("fontSize")
                    .or_else(|| node.raw.get("FontSize"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32)
                    .unwrap_or_else(|| {
                        if node_type_name(&node.ty)
                            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
                        {
                            21.0
                        } else {
                            18.0
                        }
                    });
                scale_design_font_value(
                    adjust_ui_ir_font_value_for_font_record_image_percent(
                        UiIrValue::Fixed { value: font_size },
                        resolved_font_record.as_ref(),
                    ),
                    design_text_scale,
                )
            };
            // A caption-pair label (MEDGELS, PATIENT NAME) renders at its full brand
            // nominal — no all-caps size reduction (see the primary-text note above).
            Some(UiIrTextStyle {
                font_record: font_record.clone(),
                resolved_font_record: resolved_font_record.clone(),
                font_size: font_size_value.clone(),
                line_spacing: resolve_effective_line_spacing(
                    node,
                    &font_size_value,
                    &standard_text_styles,
                    design_text_scale,
                ),
                letter_spacing: brand_style_letter_spacing(label_style.as_deref(), &standard_text_styles)
                    .map(|spacing| spacing * design_text_scale),
                alignment,
                vertical_alignment: node
                    .raw
                    .get("verticalTextAlignment")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Center")
                    .to_string(),
                anchor_to_parent_x: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentX"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                anchor_to_parent_y: node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("anchorToParentY"))
                    .and_then(|v| v.as_f64())
                    .map(|value| value as f32),
                colour: fill_colour_from_raw_for_text(&node.raw),
                colour_token: text_colour_token_from_raw(&node.raw).or_else(|| {
                    semantic_text_colour_token_from_style_tags(&resolved_style_tags, label_style.as_deref())
                }).or_else(|| {
                    default_style_text_colour_token_from_raw(&node.raw, &node.ty, false)
                }),
                label_style: label_style.clone(),
            })
        } else {
            None
        };

        let secondary_text_payload = if is_label_caption_pair && !pair_caption_hidden {
            binding_resolver
                .resolve_field_text(id, "ParamInput1", defaults)
                .map(|text| {
                    // The caption applies ITS OWN authored case modifier
                    // (`captionProperties.caseModifier`, e.g. Upper → the
                    // lift-call "SUB DECK" heading), like every other text
                    // resolution path.
                    let case = node
                        .raw
                        .get("captionProperties")
                        .and_then(|cp| cp.get("caseModifier"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let cased = crate::bb_bindings::apply_case_modifier(&text, case);
                    classify_text_payload(Some(cased.as_str()), &node.raw, defaults)
                })
        } else {
            None
        };

        let secondary_text_style = secondary_text_payload.is_some().then(|| {
            caption_pair_secondary_text_style(
                node,
                resolved_font_record.as_ref(),
                &font_record,
                &standard_text_styles,
                &resolved_style_tags,
                design_text_scale,
            )
        });

        let suppress_placeholder_only_label_caption_pair = node_type_name(&node.ty)
            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
            && secondary_text_payload
                .as_ref()
                .is_none_or(is_placeholder_or_empty_secondary_text_payload)
            && node
                .raw
                .get("captionProperties")
                .and_then(|cp| cp.get("caption"))
                .and_then(|value| value.as_str())
                .is_some_and(|caption| caption.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER"));

        let rect = if suppress_placeholder_only_label_caption_pair {
            layout_rect
        } else {
            maybe_reanchor_active_label_caption_pair_rect(scene, &layout, id, node, layout_rect)
        };
        let meter_progress = if node_type_name(&node.ty)
            .eq_ignore_ascii_case("BuildingBlocks_WidgetLinearProgressMeter")
        {
            binding_resolver
                .resolve_field_number(id, "ParamInput0", defaults)
                .map(|v| v.clamp(0.0, 1.0) as f32)
                .or_else(|| {
                    node.raw
                        .get("progress")
                        .and_then(|value| value.as_f64())
                        .map(|value| value.clamp(0.0, 1.0) as f32)
                })
        } else {
            None
        };

        // A bound `SvgPath`/`svgPath` field overrides the authored placeholder when
        // it resolves to a real asset PATH: the DRAK master-mode icon authors
        // `ui_icon_vehicle_ship.svg` but the host canvas drives `shape_Icon.SvgPath`
        // via the `ParamInput4` weapon-group switch (`seatdashboard/currentmode == 3`
        // → `guns.svg`). The binding must look like a path (`contains('/')`) — the
        // MFD footer's pixel nav arrows bind `SvgPath` to a chrome FillStyle TAG
        // reference (`Tag.5616aeff…`) that `collect_node_asset_refs` resolves to the
        // real arrow SVG, so a raw tag must NOT win over that resolution.
        // `resolve_field_text` returns `None` for the common case of an unbound icon.
        let mut asset_ref = binding_resolver
            .resolve_field_text(id, "SvgPath", defaults)
            .or_else(|| binding_resolver.resolve_field_text(id, "svgPath", defaults))
            .filter(|s| {
                let trimmed = s.trim();
                !trimmed.is_empty() && trimmed.contains('/')
            })
            .or_else(|| collect_node_asset_refs(node).into_iter().next())
            .or_else(|| {
                binding_resolver
                    .resolve_field_text(id, "ImagePath", defaults)
                    .or_else(|| binding_resolver.resolve_field_text(id, "imagePath", defaults))
            })
            .or_else(|| {
                binding_resolver
                    .resolve_string_binding(id)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            });

        let custom_shape = build_custom_shape(node);

        let stroke_extent = separator_stroke_extent_from_raw(node);
        let separator_style = separator_standard_style_from_source(
            node,
            selected_style_source.as_deref(),
            canvas_fetcher,
            design_text_scale > 1.0,
        );
        // A resolved widget-standard separator paints the MFD divider SVG (a
        // column of dots) via the asset_ref rasteriser. Contain-fit + centred (no
        // custom_shape, so `rasterize_svg_for_node` takes the aspect-preserved
        // path) keeps the narrow dots round inside the wide slot instead of
        // stretching them to dashes. The colour/alpha below tint it.
        let mut separator_asset_layout = None;
        if let Some(style) = separator_style.as_ref()
            && let Some(svg) = style.svg_path.as_deref()
        {
            asset_ref = Some(svg.to_string());
            separator_asset_layout = Some(UiIrAssetLayout {
                scaling_behavior: Some("Contain".to_string()),
                contain_position_x: Some(0.5),
                contain_position_y: Some(0.5),
                flip_horizontal: None,
                flip_vertical: None,
            });
        }
        let alpha_base = separator_style
            .as_ref()
            .and_then(|style| style.alpha_override)
            .unwrap_or_else(|| effective_alpha_for_node(id, node, scene, animation_sample_percent));
        let svg_fill_overlay_alpha = svg_fill_overlay_alpha_from_raw(&node.raw);
        let alpha = alpha_base
            * separator_style
                .as_ref()
                .and_then(|style| style.colour_alpha)
                .unwrap_or(1.0)
            * svg_fill_overlay_alpha.unwrap_or(1.0);
        let stroke_colour = stroke_colour_from_raw(&node.raw)
            .or_else(|| separator_style.as_ref().and_then(|style| style.colour));
        let stroke_colour_token = stroke_colour_token_from_raw(&node.raw).or_else(|| {
            separator_style
                .as_ref()
                .and_then(|style| style.colour_token.clone())
        });

        let allow_background_fill = node_background_enabled(node);
        let background_fill_colour_token = background_fill_colour_token_from_raw(&node.raw, allow_background_fill, allow_background_fill);
        let background_fill_alpha = background_fill_alpha_from_raw(&node.raw, allow_background_fill, allow_background_fill);
        let circle_fill_token = circle_fill_token_from_raw(node);
        let mut background_fill_colour = allow_background_fill
            .then(|| {
                node.background
                    .as_ref()
                    .and_then(|bg| bg.fill_colour)
                    .or_else(|| node.raw.get("BackgroundColor").and_then(parse_raw_colour))
            })
            .flatten();
        if background_fill_colour_token.is_some()
            && background_fill_colour.is_some_and(|colour| colour[3] <= 0.005)
        {
            background_fill_colour = None;
        }
        // Colour-overlay identity default: a render-shape LEAF whose colour
        // overlay is enabled but carries NO colour (authored or brand) renders
        // WHITE — the overlay's multiplicative identity — not transparent. This
        // draws the velocity / g-force ball's centre dot (a `base_Diagram`
        // rounded shape: `background.enable` + `svgFill.renderShape` +
        // `enableColorOverlay`, every colour null, no `svgPath`). White here is
        // the overlay identity (like alpha 1.0), not a palette value. Gated to
        // leaves so layout-container backgrounds that share the null-overlay
        // shape stay transparent.
        if background_fill_colour.is_none()
            && background_fill_colour_token.is_none()
            && node.children.is_empty()
            && is_untinted_overlay_render_shape(&node.raw)
        {
            background_fill_colour = Some([1.0, 1.0, 1.0, 1.0]);
        }
        let colour_blend_mode = separator_colour_blend_mode_from_raw(node).or_else(|| {
            background_colour_blend_mode_from_raw(node, background_fill_colour_token.as_deref(), allow_background_fill)
        })
        .or_else(|| custom_shape_colour_blend_mode_from_style_tags(&resolved_style_tags, &node.ty));
        let border = border_from_node(node, design_text_scale);
        let overflow_mode = overflow_mode_from_raw(&node.raw);

        let icon_tint_colour = node
            .icon
            .as_ref()
            .and_then(|i| i.tint_colour)
            .or_else(|| svg_fill_overlay_colour_from_raw(&node.raw))
            .or_else(|| custom_shape.as_ref().and_then(|_| fill_colour_from_raw_for_text(&node.raw)));
        let icon_tint_colour_token = icon_tint_colour_token_from_raw(
            &node.raw,
            true,
        )
        .or_else(|| icon_tint_colour_token_from_style_tags(&own_style_tags, &node.ty))
        .or_else(|| {
            custom_shape_overlay_icon_default(
                node,
                icon_tint_colour.is_some()
                    || background_fill_colour.is_some()
                    || background_fill_colour_token.is_some(),
                asset_ref.as_deref(),
                is_hud_canvas,
            )
        });

    {
        nodes.push(UiIrNode {
            id,
            parent_id: node.parent,
            children: node.children.clone(),
            node_type: node_type_name(&node.ty).to_string(),
            name: node.name.clone(),
            is_active: node.is_active
                && !suppress_placeholder_background_image
                && !suppress_placeholder_only_label_caption_pair,
            layer: node.layer,
            alpha,
            anchor: [node.anchor.x, node.anchor.y],
            pivot: [node.pivot.x, node.pivot.y],
            rotation_deg: node_rotation_deg_from_raw(&node.raw),
            authored_position: [node.position.x, node.position.y],
            authored_size: [
                convert_bb_value(&node.sizing.width),
                convert_bb_value(&node.sizing.height),
            ],
            padding: [
                node.padding.top,
                node.padding.right,
                node.padding.bottom,
                node.padding.left,
            ],
            margin: [
                node.margin.top,
                node.margin.right,
                node.margin.bottom,
                node.margin.left,
            ],
            overflow_mode,
            clip_rect: clip_rect_for_node(scene, layout, node),
            computed_rect: UiIrRect {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
            },
            background_fill_colour,
            corner_radius: node_corner_radius(node),
            corner_radii: node_corner_geometry(node).0,
            corner_chamfers: node_corner_geometry(node).1,
            background_fill_alpha,
            background_fill_colour_token,
            circle_fill_colour_token: circle_fill_token,
            style_provenance: style_provenance_from_raw(node),
            segmented_fill: segmented_fill_from_raw(node),
            polygon: polygon_from_raw(node),
            border,
            stroke_colour,
            stroke_colour_token,
            stroke_extent,
            separator_strip: separator_style.as_ref().and_then(|style| style.strip),
            colour_blend_mode,
            icon_tint_colour,
            icon_tint_colour_token,
            icon_preset: node.icon.as_ref().and_then(|i| i.icon_preset.clone()),
            text_payload,
            secondary_text_payload,
            secondary_text_style,
            meter_progress,
            text_style,
            asset_ref,
            // Prefer the cascade-applied top-level `PrimitiveMaterialPath` (the
            // brand override — the radar disc swaps to `ui_grin_…`/`…_RSI` per
            // manufacturer via a `Radial Grid Element` style modifier;
            // `apply_string_field` writes it top-level) over the authored
            // `primitiveSettings.primitiveMaterialPath` (the generic default a
            // no-override brand like DRAK keeps). Keeps the radar disc texture
            // manufacturer-correct.
            primitive_material: node
                .raw
                .get("PrimitiveMaterialPath")
                .and_then(|path| path.as_str())
                .or_else(|| {
                    node.raw
                        .get("primitiveSettings")
                        .and_then(|settings| settings.get("primitiveMaterialPath"))
                        .and_then(|path| path.as_str())
                })
                .map(str::to_owned)
                .filter(|path| !path.is_empty()),
            primitive_uv_start: primitive_settings_vec2(&node.raw, "UVStart"),
            primitive_uv_size: primitive_settings_vec2(&node.raw, "UVSize"),
            asset_layout: separator_asset_layout.or_else(|| asset_layout_from_raw(&node.raw)),
            custom_shape,
            style_tag_uuids,
            resolved_style_tags,
            is_flash_renderer: node
                .raw
                .get("rendererType")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("Flash")),
            auto_font_size: auto_font_size_enabled(&node.raw),
            colour_overlay_enabled: matches!(node.ty, BbNodeType::WidgetImage)
                && node
                    .raw
                    .get("EnableColorOverlay")
                    .and_then(|v| v.as_bool())
                    .or_else(|| {
                        node.raw
                            .get("svgFill")
                            .and_then(|svg| svg.get("enableColorOverlay"))
                            .and_then(|v| v.as_bool())
                    })
                    .unwrap_or(false),
        });
    }

    }

    apply_placeholder_black_icon_inheritance(&mut nodes, scene, is_hud_canvas);

    nodes
}

/// A cascade-applied literal `FillColor` of opaque pure black (`{r:0,g:0,b:0}`)
/// with NO surviving `FillColorToken` is the editor's uninitialised placeholder
/// default. The modular-kit button sheet `sk_uilo_a` authors its caret icon this
/// way (every sibling `uilo` kit uses `ColorStyle(Background)`); a `ColorSolid`
/// literal resolves to a token-less RGBA, which distinguishes it from a role that
/// merely resolved to a dark colour (that keeps its `FillColorToken`).
fn fill_colour_is_placeholder_black(raw: &serde_json::Value) -> bool {
    if raw
        .get("FillColorToken")
        .and_then(|token| token.as_str())
        .is_some_and(|token| !token.trim().is_empty())
    {
        return false;
    }
    let Some(fill) = raw.get("FillColor").and_then(|fill| fill.as_object()) else {
        return false;
    };
    let channel = |key: &str| fill.get(key).and_then(|value| value.as_f64());
    matches!(
        (channel("r"), channel("g"), channel("b")),
        (Some(r), Some(g), Some(b)) if r == 0.0 && g == 0.0 && b == 0.0
    )
}

/// Button-content icon colour: the modular-kit button sheet styles the text
/// field and the icon element separately, and `sk_uilo_a` is the lone `uilo`
/// button kit that authors the icon `FillColor` as a `ColorSolid` pure black —
/// an editor placeholder — while every sibling authors `ColorStyle(Background)`,
/// the button's content foreground (the same role its text field uses). The
/// placeholder resolves to a token-less black, so without this the caret falls
/// through to the SVG's native (unfilled → black) colour instead of the button
/// content colour. The engine treats the placeholder as unset: a `widget_icon`
/// carrying only placeholder black and no resolved tint inherits its sibling text
/// field's colour token. Scoped to non-HUD canvases — the cockpit HUD button kits
/// (`sk_aegs_hud`/`sk_anvl_hud`/…) author `ColorSolid`-black icon fills the HUD
/// draw path handles on its own terms.
fn apply_placeholder_black_icon_inheritance(
    nodes: &mut [UiIrNode],
    scene: &BbScene,
    is_hud_canvas: bool,
) {
    if is_hud_canvas {
        return;
    }
    let mut text_token_by_parent: HashMap<u32, String> = HashMap::new();
    for node in nodes.iter() {
        if node.node_type != "widget_text_field" {
            continue;
        }
        if let (Some(parent), Some(token)) = (node.parent_id, node.icon_tint_colour_token.as_ref())
        {
            text_token_by_parent
                .entry(parent)
                .or_insert_with(|| token.clone());
        }
    }
    for node in nodes.iter_mut() {
        if node.node_type != "widget_icon"
            || node.icon_tint_colour.is_some()
            || node.icon_tint_colour_token.is_some()
        {
            continue;
        }
        let Some(parent) = node.parent_id else {
            continue;
        };
        let Some(token) = text_token_by_parent.get(&parent) else {
            continue;
        };
        if scene
            .nodes
            .get(&node.id)
            .is_some_and(|scene_node| fill_colour_is_placeholder_black(&scene_node.raw))
        {
            node.icon_tint_colour_token = Some(token.clone());
        }
    }
}

/// The pair component's authored `alignment` enum ("Left"/"Center"/"Right") —
/// the field the labelcaptionpair widget-standard's RootCenter/RootRight
/// entries select on to align the pair's content (the lift-call floor heading
/// authors "Center").
fn pair_component_alignment(raw: &serde_json::Value) -> Option<&str> {
    raw.get("alignment")
        .and_then(|v| v.as_str())
        .map(|s| if s == "End" { "Right" } else { s })
        .filter(|s| !s.is_empty())
}

/// Build the value-side (secondary) text style of a `ComponentLabelCaptionPair`. The
/// value is plain, data-bound text, so it gets the same per-font imageSizePercent boost
/// as other plain text; its colour role prefers the caption style's brand `FillColor`.
fn caption_pair_secondary_text_style(
    node: &BbNode,
    resolved_font_record: Option<&serde_json::Value>,
    font_record: &Option<String>,
    standard_text_styles: &HashMap<String, StandardTextStyle>,
    resolved_style_tags: &[UiIrStyleTag],
    design_text_scale: f32,
) -> UiIrTextStyle {
    let alignment = node
        .raw
        .get("captionProperties")
        .and_then(|cp| cp.get("textAlignment"))
        .and_then(|v| v.as_str())
        .or_else(|| pair_component_alignment(&node.raw))
        .unwrap_or("Left")
        .to_string();
    let caption_style = node
        .raw
        .get("captionProperties")
        .and_then(|cp| cp.get("style"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    // The caption-pair value (e.g. MedGel "200/200", style Heading6) renders at its named
    // brand style verbatim — like any styled textfield. Prefer that; only fall back to
    // the node's authored fontSize + imageSizePercent boost when the value's style has no
    // brand-table entry.
    let value_font_size = if let Some(size) =
        brand_style_font_size(caption_style.as_deref(), standard_text_styles)
    {
        apply_font_image_size_percent(
            UiIrValue::Fixed { value: size * design_text_scale },
            resolved_font_record,
            true,
            design_text_scale,
        )
    } else {
        let font_size = node
            .raw
            .get("fontSize")
            .or_else(|| node.raw.get("FontSize"))
            .and_then(|v| v.as_f64())
            .map(|value| value as f32)
            .unwrap_or(18.0);
        scale_design_font_value(
            adjust_ui_ir_font_value_for_font_record_image_percent(
                UiIrValue::Fixed { value: font_size },
                resolved_font_record,
            ),
            design_text_scale,
        )
    };
    // The caption value renders at its full nominal size (no all-caps reduction).
    UiIrTextStyle {
        font_record: font_record.clone(),
        resolved_font_record: resolved_font_record.cloned(),
        font_size: value_font_size.clone(),
        line_spacing: resolve_effective_line_spacing(node, &value_font_size, standard_text_styles, design_text_scale),
        letter_spacing: brand_style_letter_spacing(caption_style.as_deref(), standard_text_styles)
            .map(|spacing| spacing * design_text_scale),
        alignment,
        vertical_alignment: node
            .raw
            .get("verticalTextAlignment")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                // The pair's template text fields are TOP-anchored and stack
                // from the box top; with the label hidden
                // (`labelProperties.show=false`) the caption is the first
                // stacked field, so it tops the box (the lift-call floor
                // heading sits above its separator like the reference). A
                // VISIBLE label keeps the stacked-band model (secondary rect
                // is placed below the primary band), where Center holds.
                let label_hidden = node
                    .raw
                    .get("labelProperties")
                    .and_then(|lp| lp.get("show"))
                    .and_then(|v| v.as_bool())
                    == Some(false);
                if label_hidden { "Top" } else { "Center" }
            })
            .to_string(),
        anchor_to_parent_x: node
            .raw
            .get("captionProperties")
            .and_then(|cp| cp.get("anchorToParentX"))
            .and_then(|v| v.as_f64())
            .map(|value| value as f32),
        anchor_to_parent_y: node
            .raw
            .get("captionProperties")
            .and_then(|cp| cp.get("anchorToParentY"))
            .and_then(|v| v.as_f64())
            .map(|value| value as f32),
        colour: fill_colour_from_raw_for_text(&node.raw),
        colour_token: text_colour_token_from_raw(&node.raw)
            .or_else(|| brand_text_style_colour_token(caption_style.as_deref(), standard_text_styles))
            .or_else(|| {
                semantic_text_colour_token_from_style_tags(resolved_style_tags, caption_style.as_deref())
            })
            .or_else(|| default_style_text_colour_token_from_raw(&node.raw, &node.ty, true)),
        label_style: caption_style,
    }
}

fn build_custom_shape(node: &BbNode) -> Option<UiIrCustomShape> {
    if node.ty != BbNodeType::WidgetCustomShape {
        return None;
    }
    let shape_type = node
        .raw
        .get("shapeType")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let shape = node
        .raw
        .get("shape")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let svg_path = node
        .raw
        .get("svgPath")
        .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("svgPath")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let render_shape = node
        .raw
        .get("renderShape")
        .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("renderShape")))
        .and_then(|v| v.as_bool());
    let enable_nine_slice_rect = node
        .raw
        .get("enableNineSliceRect")
        .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("enableNineSliceRect")))
        .and_then(|v| v.as_bool());
    let nine_slice_rect = node
        .raw
        .get("nineSliceRect")
        .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("nineSliceRect")))
        .and_then(parse_nine_slice_rect);
    let nine_slice_scale = node
        .raw
        .get("nineSliceScale")
        .or_else(|| node.raw.get("svgFill").and_then(|sf| sf.get("nineSliceScale")))
        .and_then(|v| v.as_f64())
        .map(|value| value as f32);
    Some(UiIrCustomShape {
        shape_type,
        shape,
        svg_path,
        render_shape,
        enable_nine_slice_rect,
        nine_slice_rect,
        nine_slice_scale,
    })
}

/// Whether `node`'s authored overflow clips its descendants (`Clip` or
/// `ClipFade` — the fade is an edge treatment on the same clipped region).
/// Per-axis hard-clip flags for a node's authored `BuildingBlocks_Overflow`.
/// Returns `(x_clips, y_clips)`; `(false, false)` when the mode is not
/// Clip/ClipFade. An axis whose `fade<Axis>` flag is TRUE is owned by the
/// engine's edge-fade machinery, not the scissor: the power Scrollview
/// authors Clip with fadeXAxis=true / fadeYAxis=false and the in-game capture
/// renders the third column's temp gauge complete past the viewport's right
/// edge, unfaded (widthFadeThreshold 0 disables the fade itself), while the
/// list stays bounded vertically.
fn node_clip_axes(node: &BbNode) -> (bool, bool) {
    let Some(overflow) = node.raw.get("overflow") else { return (false, false) };
    let mode_clips = overflow
        .get("overflow")
        .and_then(|v| v.as_str())
        .or_else(|| overflow.as_str())
        .is_some_and(|mode| {
            mode.eq_ignore_ascii_case("Clip") || mode.eq_ignore_ascii_case("ClipFade")
        });
    if !mode_clips {
        return (false, false);
    }
    let fade_axis = |key: &str| overflow.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    (!fade_axis("fadeXAxis"), !fade_axis("fadeYAxis"))
}

/// Pixel-space intersection of all clipping ancestors' laid-out rects: the
/// region the renderer may paint this node into (the power screen's
/// `Scrollview` clips its 7-column list to the 3 visible columns).
fn clip_rect_for_node(
    scene: &BbScene,
    layout: &LayoutResult,
    node: &BbNode,
) -> Option<UiIrRect> {
    let mut clip: Option<crate::bb_layout::Rect> = None;
    let mut parent = node.parent;
    while let Some(parent_id) = parent {
        let Some(ancestor) = scene.nodes.get(&parent_id) else { break };
        let (x_clips, y_clips) = node_clip_axes(ancestor);
        if (x_clips || y_clips)
            && let Some(rect) = layout.rects.get(&parent_id)
        {
            // A faded (non-clipping) axis contributes the full canvas extent
            // so only the hard-clipped axis scissors descendants.
            let mut rect = *rect;
            if !x_clips {
                rect.x = layout.canvas.x;
                rect.w = layout.canvas.w;
            }
            if !y_clips {
                rect.y = layout.canvas.y;
                rect.h = layout.canvas.h;
            }
            clip = Some(match clip {
                None => rect,
                Some(existing) => existing.intersect(&rect).unwrap_or(crate::bb_layout::Rect {
                    x: existing.x,
                    y: existing.y,
                    w: 0.0,
                    h: 0.0,
                }),
            });
        }
        parent = ancestor.parent;
    }
    clip.map(|rect| UiIrRect { x: rect.x, y: rect.y, w: rect.w, h: rect.h })
}

fn overflow_mode_from_raw(raw: &serde_json::Value) -> Option<String> {
    for key in [
        "OverflowMode",
        "overflowMode",
        "overflow",
        "Overflow",
        "ClipMode",
        "clipMode",
        "clip",
        "Clip",
    ] {
        if let Some(value) = raw.get(key) {
            return Some(if let Some(string_value) = value.as_str() {
                string_value.to_string()
            } else {
                value.to_string()
            });
        }
    }

    None
}

/// The generic icon colour for an entry-less colour-overlay custom shape:
/// `BB_ColorStyle` MissionObjectives (enum 16). The power screen's
/// system/card icons (`shape_SystemIcon`, `shape_OutputIcon`…) author
/// `svgFill.enableColorOverlay` with a null colour and the DRAK brand has NO
/// at-rest colour entry for them (misc/orig author explicit "System Icon
/// Color" entries) — in-game they render the MissionObjectives slot, the same
/// role HUD records author as `FillColor=MissionObjectives` for their generic
/// Icon Styles. Entry-coloured shapes (the target chevrons' embedded `Base`,
/// the medical fingerprint's `Accent1`) resolve from raw first and never
/// reach this default.
///
/// This MFD generic-icon default does NOT apply on a cockpit HUD canvas
/// (`HC_HUD_*`/`H_HUD_*`/`H_Eng_*`): there an entry-less overlay shape keeps its
/// SVG's native colour. The DRAK master-mode weapon icon (`guns.svg`, authored
/// all-white) renders WHITE in the in-game reference, not MissionObjectives.
fn custom_shape_overlay_icon_default(
    node: &BbNode,
    has_resolved_colour: bool,
    asset_ref: Option<&str>,
    is_hud_canvas: bool,
) -> Option<String> {
    if has_resolved_colour
        || is_hud_canvas
        || !matches!(node.ty, BbNodeType::WidgetCustomShape)
    {
        return None;
    }
    let svg = node.raw.get("svgFill")?;
    let overlay_enabled = svg
        .get("enableColorOverlay")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let render_shape = svg
        .get("renderShape")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // The icon's vector source can be authored (`svgFill.svgPath`), styled
    // (an `SvgPath` entry — the card icons), or binding-resolved into the
    // asset ref (the system icons' per-system glyph).
    let has_svg_source = svg
        .get("svgPath")
        .and_then(|v| v.as_str())
        .is_some_and(|path| !path.trim().is_empty())
        || asset_ref.is_some_and(|path| {
            path.trim_end().to_ascii_lowercase().ends_with(".svg")
        });
    let colour_is_null = svg.get("color").is_none_or(|v| v.is_null());
    (overlay_enabled && render_shape && has_svg_source && colour_is_null)
        .then(|| "MissionObjectives".to_string())
}

pub(crate) fn collect_node_asset_refs(node: &BbNode) -> Vec<String> {
    let mut asset_refs = Vec::new();

    // A styled `ImagePath` (PascalCase = written by a style modifier, e.g. the
    // DRAK annunciator backplate swap) overrides the authored image source; an
    // explicitly empty styled path clears it (the ARGO annunciator background
    // pairs `ImagePath: ""` with a flat `FillColor`).
    let styled_image = node.raw.get("ImagePath").and_then(|value| value.as_str());
    let styled_clears_image = styled_image.is_some_and(|value| value.trim().is_empty());
    push_asset_ref(&mut asset_refs, styled_image);
    if !styled_clears_image {
        push_asset_ref(
            &mut asset_refs,
            node.icon
                .as_ref()
                .and_then(|icon| icon.image_record.as_deref()),
        );
    }
    push_asset_ref(
        &mut asset_refs,
        node.background
            .as_ref()
            .and_then(|background| background.svg_fill_path.as_deref()),
    );
    if !styled_clears_image {
        push_asset_ref(
            &mut asset_refs,
            node.raw
                .get("imagePath")
                .and_then(|value| value.as_str()),
        );
    }
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("SvgPath")
            .and_then(|value| value.as_str()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("svgPath")
            .and_then(|value| value.as_str()),
    );
    push_asset_ref(
        &mut asset_refs,
        node.raw
            .get("svgFill")
            .and_then(|value| value.get("svgPath"))
            .and_then(|value| value.as_str()),
    );

    asset_refs
}

fn push_asset_ref(asset_refs: &mut Vec<String>, candidate: Option<&str>) {
    let Some(candidate) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if asset_refs.iter().all(|existing| existing != candidate) {
        asset_refs.push(candidate.to_string());
    }
}

pub(crate) fn unresolved_text_key_from_raw(raw: &serde_json::Value) -> Option<String> {
    let direct = raw.get("text").and_then(|v| v.as_str()).map(str::trim);
    if let Some(key) = direct.filter(|s| s.starts_with('@') && !s.is_empty()) {
        return Some(key.to_string());
    }

    let loc_string = raw
        .get("locString")
        .and_then(|v| v.as_str())
        .map(str::trim);
    if let Some(key) = loc_string.filter(|s| s.starts_with('@') && !s.is_empty()) {
        return Some(key.to_string());
    }

    let label = raw
        .get("labelProperties")
        .and_then(|lp| lp.get("label"))
        .and_then(|v| v.as_str())
        .map(str::trim);
    label
        .filter(|s| s.starts_with('@') && !s.is_empty())
        .map(ToString::to_string)
}

fn is_placeholder_or_empty_secondary_text_payload(payload: &UiIrTextPayload) -> bool {
    match payload {
        UiIrTextPayload::Empty => true,
        UiIrTextPayload::IntentionallyEmpty { .. } => true,
        UiIrTextPayload::Resolved { text } => text.trim().is_empty(),
        UiIrTextPayload::UnresolvedKey { key } => key.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER"),
    }
}

fn node_has_text_intent(node: &BbNode) -> bool {
    node.text.is_some()
        || node.raw.get("text").is_some()
        || node.raw.get("locString").is_some()
        || node.raw.get("labelProperties").is_some()
}

fn maybe_reanchor_active_label_caption_pair_rect(
    scene: &BbScene,
    layout: &LayoutResult,
    node_id: BbNodeId,
    node: &BbNode,
    rect: Rect,
) -> Rect {
    if !node_type_name(&node.ty).eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        || node.anchor.x > 0.01
        || node.pivot.x > 0.01
    {
        return rect;
    }

    let Some(parent_id) = node.parent else {
        return rect;
    };

    if !is_footer_brand_label_context(scene, parent_id) {
        return rect;
    }

    let Some(parent) = scene.nodes.get(&parent_id) else {
        return rect;
    };

    let mut has_placeholder_sibling = false;
    let mut leftmost_x = rect.x;
    for sibling_id in &parent.children {
        let Some(sibling) = scene.nodes.get(sibling_id) else {
            continue;
        };
        if !node_type_name(&sibling.ty)
            .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        {
            continue;
        }

        if *sibling_id != node_id
            && sibling
                .raw
                .get("captionProperties")
                .and_then(|cp| cp.get("caption"))
                .and_then(|value| value.as_str())
                .is_some_and(|caption| caption.trim().eq_ignore_ascii_case("@LOC_PLACEHOLDER"))
        {
            has_placeholder_sibling = true;
        }

        if let Some(sibling_rect) = layout.rects.get(sibling_id)
            && sibling_rect.x < leftmost_x
        {
            leftmost_x = sibling_rect.x;
        }
    }

    if has_placeholder_sibling && leftmost_x < rect.x {
        Rect {
            x: leftmost_x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        }
    } else {
        rect
    }
}

fn node_corner_radius(node: &BbNode) -> Option<f32> {
    explicit_uniform_corner_radius(node)
}

/// Per-corner styled radii `[TL, TR, BR, BL]` + chamfer flags, populated only
/// when the geometry is NON-uniform or any corner is chamfered (the uniform
/// un-chamfered case stays on [`node_corner_radius`]). A missing raw corner
/// reads 0 (square).
fn node_corner_geometry(node: &BbNode) -> (Option<[f32; 4]>, Option<[bool; 4]>) {
    let border = node.raw.get("border");
    let radius = |corner: &str| {
        border
            .and_then(|b| b.get(corner))
            .and_then(|value| value.get("radius"))
            .and_then(|value| value.get("value"))
            .and_then(|value| value.as_f64())
            .map(|value| value as f32)
            .unwrap_or(0.0)
    };
    let chamfer = |field: &str| {
        node.raw
            .get(field)
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    };
    let radii = [
        radius("topLeftRadius"),
        radius("topRightRadius"),
        radius("bottomRightRadius"),
        radius("bottomLeftRadius"),
    ];
    let chamfers = [
        chamfer("EnableTopLeftBorderChamfer"),
        chamfer("EnableTopRightBorderChamfer"),
        chamfer("EnableBottomRightBorderChamfer"),
        chamfer("EnableBottomLeftBorderChamfer"),
    ];
    let any_radius = radii.iter().any(|r| *r > 0.0);
    let any_chamfer = chamfers.iter().any(|c| *c);
    let uniform = radii.iter().all(|r| (*r - radii[0]).abs() <= f32::EPSILON);
    if !any_radius || (uniform && !any_chamfer) {
        return (None, None);
    }
    (Some(radii), Some(chamfers))
}

fn explicit_uniform_corner_radius(node: &BbNode) -> Option<f32> {
    let border = node.raw.get("border")?;
    let radii = ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"]
        .into_iter()
        .map(|corner| {
            border
                .get(corner)
                .and_then(|value| value.get("radius"))
                .and_then(|value| value.get("value"))
                .and_then(|value| value.as_f64())
                .map(|value| value as f32)
        })
        .collect::<Option<Vec<_>>>()?;

    let first = *radii.first()?;
    if first <= 0.0 || radii.iter().any(|radius| (*radius - first).abs() > f32::EPSILON) {
        return None;
    }

    Some(first)
}

fn is_footer_brand_label_context(scene: &BbScene, mut parent_id: BbNodeId) -> bool {
    loop {
        let Some(parent) = scene.nodes.get(&parent_id) else {
            return false;
        };

        let mut has_logo = false;
        let mut has_bottom_bar = false;
        for child_id in &parent.children {
            let Some(child) = scene.nodes.get(child_id) else {
                continue;
            };
            if node_type_name(&child.ty)
                .eq_ignore_ascii_case("BuildingBlocks_WidgetManufacturerLogo")
            {
                has_logo = true;
            }

            let image_path = child
                .raw
                .get("ImagePath")
                .or_else(|| child.raw.get("imagePath"))
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if image_path.contains("bottom-bar") {
                has_bottom_bar = true;
            }
        }

        if has_logo && has_bottom_bar {
            return true;
        }

        if let Some(next_parent) = parent.parent {
            parent_id = next_parent;
        } else {
            return false;
        }
    }
}

/// Reads the cascade's `__StyleProvenance` annotation (stamped only under
/// `SB_UI_STYLE_PROVENANCE=1` by `bb_brand_apply::apply_entry_modifiers` via
/// `stamp_style_provenance`) into the IR node — None in normal compiles.
/// Ledger item A.
fn style_provenance_from_raw(node: &BbNode) -> Option<std::collections::BTreeMap<String, String>> {
    let map = node.raw.get("__StyleProvenance")?.as_object()?;
    let provenance: std::collections::BTreeMap<String, String> = map
        .iter()
        .filter_map(|(field, source)| source.as_str().map(|s| (field.clone(), s.to_string())))
        .collect();
    (!provenance.is_empty()).then_some(provenance)
}

fn background_fill_colour_token_from_raw(
    raw: &serde_json::Value,
    background_enabled: bool,
    allow_fill_colour: bool,
) -> Option<String> {
    background_enabled
        .then(|| raw.get("BackgroundColorToken"))
        .flatten()
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            if !allow_fill_colour {
                return None;
            }
            raw.get("FillColorToken")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            if background_enabled {
                raw.get("background")
                    .and_then(|background| background.get("color"))
                    .and_then(colour_style_token)
                    .or_else(|| raw.get("BackgroundColor").and_then(colour_style_token))
            } else {
                None
            }
            .or_else(|| {
                if allow_fill_colour {
                    raw.get("FillColor").and_then(colour_style_token)
                } else {
                    None
                }
            })
        })
}

fn icon_tint_colour_token_from_style_tags(
    resolved_style_tags: &[UiIrStyleTag],
    node_type: &BbNodeType,
) -> Option<String> {
    let is_manufacturer_logo = matches!(node_type, BbNodeType::Other(kind) if {
        let lower = kind.trim().to_ascii_lowercase();
        lower == "buildingblocks_widgetmanufacturerlogo" || lower == "widgetmanufacturerlogo"
    });
    let supports_icon_tint_from_style_tags = matches!(
        node_type,
        BbNodeType::WidgetCustomShape | BbNodeType::WidgetImage
    ) || is_manufacturer_logo;
    if !supports_icon_tint_from_style_tags {
        return None;
    }

    resolved_style_tags.iter().find_map(|tag| {
        let name = tag.tag_name.as_deref()?.trim().to_ascii_lowercase();
        match name.as_str() {
            "primary" => Some("Accent1".to_string()),
            "ui_generic_flag_01" => Some("Accent1".to_string()),
            "animate_8" => Some("Accent1".to_string()),
            "statemoderate" => Some("Accent2".to_string()),
            "statecritical" => Some("Accent4".to_string()),
            "modify" => Some("Accent5".to_string()),
            _ => None,
        }
    })
}

fn custom_shape_colour_blend_mode_from_style_tags(
    resolved_style_tags: &[UiIrStyleTag],
    node_type: &BbNodeType,
) -> Option<UiIrColourBlendMode> {
    if !matches!(node_type, BbNodeType::WidgetCustomShape) {
        return None;
    }

    resolved_style_tags.iter().find_map(|tag| {
        let name = tag.tag_name.as_deref()?.trim().to_ascii_lowercase();
        match name.as_str() {
            "modify" => Some(UiIrColourBlendMode::Additive),
            _ => None,
        }
    })
}

/// The centre-dot signature: an enabled `background` with a null colour AND an
/// `svgFill.renderShape` colour overlay that is enabled with a null colour and
/// an empty `svgPath`. Such a shape has no colour from any source, so the
/// engine's colour overlay renders it at its identity — white. (Verified to
/// match nothing in any frozen target; in the ball canvases only the centre dot
/// has it.)
fn is_untinted_overlay_render_shape(raw: &serde_json::Value) -> bool {
    let bg = raw.get("background");
    let bg_enabled = bg
        .and_then(|b| b.get("enable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let bg_colour_null = bg
        .and_then(|b| b.get("color"))
        .map(|c| c.is_null())
        .unwrap_or(true);
    let sf = raw.get("svgFill");
    let render_shape = sf
        .and_then(|s| s.get("renderShape"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let overlay = sf
        .and_then(|s| s.get("enableColorOverlay"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sf_colour_null = sf
        .and_then(|s| s.get("color"))
        .map(|c| c.is_null())
        .unwrap_or(true);
    let svg_path_empty = sf
        .and_then(|s| s.get("svgPath"))
        .and_then(|v| v.as_str())
        .map(|p| p.is_empty())
        .unwrap_or(true);
    bg_enabled && bg_colour_null && render_shape && overlay && sf_colour_null && svg_path_empty
}

/// A `Vec2` (`{x, y}`) from `primitiveSettings.<field>` (e.g. `UVStart`/`UVSize`),
/// for `rendererType:"Primitive"` nodes. `None` when absent.
fn primitive_settings_vec2(raw: &serde_json::Value, field: &str) -> Option<[f32; 2]> {
    let v = raw.get("primitiveSettings")?.get(field)?;
    Some([
        v.get("x").and_then(|n| n.as_f64())? as f32,
        v.get("y").and_then(|n| n.as_f64())? as f32,
    ])
}

/// The node's in-plane rotation in degrees: `orientation.z + orientationOffset.z`.
/// Returns `None` when the total is ~0 (the common no-rotation case) so the IR
/// stays clean. Applied around the node's pivot at draw time — the velocity /
/// g-force ball caps author `orientation.z = 90` on the left/right chevrons.
fn node_rotation_deg_from_raw(raw: &serde_json::Value) -> Option<f32> {
    let axis_z = |key: &str| {
        raw.get(key)
            .and_then(|o| o.get("z"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let total = (axis_z("orientation") + axis_z("orientationOffset")) as f32;
    (total.abs() > f32::EPSILON).then_some(total)
}

fn colour_style_alpha(value: &serde_json::Value) -> Option<f32> {
    value
        .get("_Type_")
        .and_then(|v| v.as_str())
        .filter(|ty| *ty == "BuildingBlocks_ColorStyle")?;

    value
        .get("alpha")
        .and_then(|v| v.as_f64())
        .map(|value| (value as f32).clamp(0.0, 1.0))
}

fn background_fill_alpha_from_raw(
    raw: &serde_json::Value,
    background_enabled: bool,
    allow_fill_colour: bool,
) -> Option<f32> {
    if background_enabled {
        raw.get("background")
            .and_then(|background| background.get("color"))
            .and_then(colour_style_alpha)
            .or_else(|| raw.get("BackgroundColor").and_then(colour_style_alpha))
    } else {
        None
    }
        .or_else(|| {
            if allow_fill_colour {
                raw.get("FillColor").and_then(colour_style_alpha)
            } else {
                None
            }
        })
}

/// Whether `node` draws a background fill.
///
/// GFx text fields draw no background unless explicitly enabled: a style
/// entry's `BackgroundColor` restyles that (off) background rather than
/// enabling it (the MFD footer screen-name text carries `Bright@1.0` from the
/// shared `UnSelectedName` entry yet draws no bar in-game). Container widgets
/// are enabled by a supplied colour (`raw_background_enabled`).
fn node_background_enabled(node: &crate::bb_scene::BbNode) -> bool {
    // The Flash UI background is painted only by Scaleform-rendered nodes
    // (`rendererType: "Flash"`, e.g. the compass centre line / tick shapes, the
    // master-mode bar fill) and by nodes with an ABSENT rendererType (the normal
    // case). Two rendererTypes suppress it:
    //   - "None": a non-rendering proxy / group widget (the compass
    //     `CanvasProxyRoot`, which otherwise painted an opaque white sheet over
    //     the dark vignette);
    //   - "Primitive": a node rendered by a 3D primitive material instead of the
    //     Flash path (the master-mode `card_Icon` / `card_CurrentModeText`, whose
    //     empty primitive material draws nothing — the in-game reference shows NO
    //     grey box behind the bullets / SCM).
    // EXCEPTION: a `WidgetRuntimeImage` (the SELF-STATUS own-vehicle hologram) is
    // `rendererType: "Primitive"` too, but the hologram compositor reuses its
    // `background_fill_colour` as the per-manufacturer holo tint, so its
    // background must stay enabled.
    if let Some(kind) = node.raw.get("rendererType").and_then(|value| value.as_str()) {
        if kind.eq_ignore_ascii_case("None") {
            return false;
        }
        if kind.eq_ignore_ascii_case("Primitive") {
            let is_runtime_image = node
                .raw
                .get("_Type_")
                .and_then(|value| value.as_str())
                .is_some_and(|ty| ty.eq_ignore_ascii_case("BuildingBlocks_WidgetRuntimeImage"));
            // A `Primitive` card rendered by an AR HOLO-VOLUME material is a 3D
            // holographic-volume element (the radar readout's `RadarMagnification`,
            // `AR_HoloVolume_standards/ui_ar_card_in_holo_volume.mtl`): its flat
            // background fill is NOT drawn on the flat RTT/screen UI. The in-game
            // DRAK radar readout shows text on the dark vignette with NO orange
            // backplate; the `Type(Card)∧Tag(Locked)→BackgroundColor:null`
            // suppressor exists only in the Greycat/RSI brand blocks, so a DRAK
            // (no-brand-match) ship leaks the card's authored `ColorStyle "Base"`
            // fill. Discriminator is the holo-volume MATERIAL, not the colour: the
            // real palette-role Primitive fills keep rendering — the power-bar
            // `PipBox_Fill` (empty material) and the master-mode `card_BarFill`
            // (`materials/default_rtt.mtl`). `WidgetRuntimeImage` (the SELF-STATUS
            // hologram, also Primitive) is excluded so its holo tint stays.
            let holo_volume_material = node
                .raw
                .get("primitiveSettings")
                .and_then(|settings| settings.get("primitiveMaterialPath"))
                .and_then(|path| path.as_str())
                .is_some_and(|path| path.to_ascii_lowercase().contains("ar_holovolume"));
            if !is_runtime_image && holo_volume_material {
                return false;
            }
            // A `Primitive` node coloured by a palette ROLE (`ColorStyle`) is a real
            // data fill — the power-bar `PipBox_Fill` (`Base`), the master-mode bar
            // `base_Fill` (`Bright`) — and must render. Only a LITERAL `ColorSolid`
            // (SRGBA8) background is the editor placeholder the engine ignores (the
            // master-mode `card_Icon`/`card_CurrentModeText` white α60, which the
            // in-game reference shows as NO grey box).
            let background_is_literal = node
                .raw
                .get("background")
                .and_then(|background| background.get("color"))
                .and_then(|colour| colour.get("_Type_"))
                .and_then(|ty| ty.as_str())
                .is_some_and(|ty| ty.eq_ignore_ascii_case("BuildingBlocks_ColorSolid"));
            if !is_runtime_image && background_is_literal {
                return false;
            }
        }
    }
    if matches!(node.ty, BbNodeType::WidgetTextField | BbNodeType::WidgetText) {
        let explicit = node
            .raw
            .get("EnableBackground")
            .and_then(|enable| enable.as_bool())
            .or_else(|| {
                node.raw
                    .get("background")
                    .and_then(|background| background.get("enable"))
                    .and_then(|enable| enable.as_bool())
            });
        return explicit == Some(true);
    }
    raw_background_enabled(&node.raw)
}

fn raw_background_enabled(raw: &serde_json::Value) -> bool {
    // An explicit EnableBackground modifier is the authoritative gate in either
    // direction (style entries sometimes set both it and a colour).
    match raw.get("EnableBackground").and_then(|e| e.as_bool()) {
        Some(enabled) => return enabled,
        None => {}
    }

    if raw.get("BackgroundColor").is_some()
        || raw
            .get("BackgroundColorToken")
            .and_then(|token| token.as_str())
            .is_some_and(|token| !token.trim().is_empty())
    {
        return true;
    }

    raw.get("background")
        .and_then(|background| background.get("enable"))
        .and_then(|enable| enable.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
mod node_background_enabled_tests {
    use super::node_background_enabled;
    use serde_json::json;

    /// Build a one-node canvas with a `Primitive` `WidgetCard` carrying the
    /// given primitive material and an enabled palette-role `ColorStyle` "Base"
    /// background, parse it, and return whether the card's background is enabled.
    fn primitive_card_background_enabled(material_path: &str) -> bool {
        let canvas = json!({
            "_RecordValue_": {
                "size": { "x": 100.0, "y": 100.0 },
                "scene": [{
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetCard",
                    "name": "card",
                    "rendererType": "Primitive",
                    "primitiveSettings": {
                        "_Type_": "BuildingBlocks_PrimitiveSettings",
                        "primitiveMaterialPath": material_path
                    },
                    "background": {
                        "_Type_": "BuildingBlocks_Background",
                        "enable": true,
                        "color": { "_Type_": "BuildingBlocks_ColorStyle", "color": "Base", "alpha": 1.0 }
                    },
                    "isActive": true
                }]
            }
        });
        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("parse");
        let node = scene.nodes.values().next().expect("one node");
        node_background_enabled(node)
    }

    /// An AR holo-volume Primitive card (the radar readout's `RadarMagnification`,
    /// material `AR_HoloVolume_standards/ui_ar_card_in_holo_volume.mtl`) is a
    /// 3D-volume element: its flat `ColorStyle "Base"` backplate is NOT drawn on
    /// the flat RTT screen UI (the in-game DRAK radar readout shows text on the
    /// dark vignette, no orange backplate; the `Card→BackgroundColor:null`
    /// suppressor exists only in the Greycat/RSI brand blocks, so DRAK leaks it).
    #[test]
    fn ar_holo_volume_primitive_card_background_is_suppressed() {
        assert!(
            !primitive_card_background_enabled(
                "Materials/UI/AR_HoloVolume_standards/ui_ar_card_in_holo_volume.mtl"
            ),
            "AR holo-volume Primitive card backplate must not render on the flat UI"
        );
    }

    /// Real palette-role Primitive fills must still render: the power-bar
    /// `PipBox_Fill` (empty material) and the master-mode `card_BarFill`
    /// (`materials/default_rtt.mtl`) are not holo-volume cards.
    #[test]
    fn non_holo_volume_primitive_fills_still_render() {
        assert!(
            primitive_card_background_enabled(""),
            "empty-material Primitive fill (power PipBox_Fill) must still render"
        );
        assert!(
            primitive_card_background_enabled("materials/default_rtt.mtl"),
            "default_rtt Primitive fill (master-mode card_BarFill) must still render"
        );
    }
}
