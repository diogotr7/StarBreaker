//! Intrinsic text measurement for Auto-sized flex children: single-line
//! draw-metric measures (`auto_text_intrinsic_main`, `node_resolved_text_size`)
//! and the draw-faithful wrapped variant (`auto_text_intrinsic_main_wrapped`,
//! `wrapped_text_intrinsic_height`) that reproduces the renderer's greedy word
//! wrap over annotated per-word advances (ledger 106). Split from `engine_02`
//! by responsibility (line-cap guard).

#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use crate::bb_scene::{BbNode, BbNodeId, BbScene, BbValue};

// Intrinsic text measurement for Auto-sized flex children.
//
// The no-grow flex flow treats Auto main-axis children as zero-sized (no
// content measurement), which collapses text-backed value groups like the
// OUTPUT card's "2" + "/ 16" pair (Auto containers in a Center-justified
// row) onto one spot. `auto_text_intrinsic_main` measures the subtree's
// RESOLVED text (bb_resolve writes `raw["_ResolvedText_"]` for active text
// fields) with the shared TTF metrics, so such children flow at their
// natural width/height like the engine lays them out.

/// Best-effort intrinsic main-axis size of an Auto-sized flex child whose
/// subtree carries resolved text. `None` when no measurable text exists
/// (the caller keeps the zero-size rule).
pub(crate) fn auto_text_intrinsic_main(
    node_id: BbNodeId,
    scene: &BbScene,
    canvas_scale: f32,
    is_row: bool,
) -> Option<f32> {
    auto_text_intrinsic_main_wrapped(node_id, scene, canvas_scale, is_row, None)
}

/// Like [`auto_text_intrinsic_main`], but when `wrap_width` is known (a COLUMN
/// child's resolved width), a text node's intrinsic HEIGHT accounts for the
/// draw's word wrap: the single-line measure under-sized wrapped labels and
/// the draw clipped their later lines (the transit button's CALL/ELEVATOR).
pub(crate) fn auto_text_intrinsic_main_wrapped(
    node_id: BbNodeId,
    scene: &BbScene,
    canvas_scale: f32,
    is_row: bool,
    wrap_width: Option<f32>,
) -> Option<f32> {
    let renderer = crate::text::TextRenderer::new();
    let mut best: Option<f32> = None;
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        let Some(node) = scene.nodes.get(&id) else { continue };
        if !node.is_active {
            continue;
        }
        stack.extend(node.children.iter().copied());
        if !is_row
            && let Some(max_w) = wrap_width
            && let Some(wrapped_h) = wrapped_text_intrinsic_height(node, max_w)
        {
            best = Some(best.map_or(wrapped_h, |b: f32| b.max(wrapped_h)));
            continue;
        }
        let Some((w, h)) = node_resolved_text_size(node, canvas_scale, &renderer) else {
            continue;
        };
        let main = if is_row { w } else { h };
        best = Some(best.map_or(main, |b: f32| b.max(main)));
    }
    best
}

/// Draw-faithful wrapped intrinsic height for one text node: reproduce the
/// draw's greedy word wrap (`swf_wrap_lines` semantics — advances are
/// additive) over the annotated per-word draw advances at `wrap_width`, then
/// stack the annotated line box per line. `None` when the node carries no
/// word-advance annotations (single-line measure applies as before).
pub(crate) fn wrapped_text_intrinsic_height(
    node: &crate::bb_scene::BbNode,
    wrap_width: f32,
) -> Option<f32> {
    if wrap_width <= 0.0 {
        return None;
    }
    let words = node.raw.get("_DrawTextWordAdvancesPx_")?.as_array()?;
    let space_cost = node.raw.get("_DrawTextSpaceCostPx_")?.as_f64()? as f32;
    let line_box = node.raw.get("_DrawTextLineBoxPx_")?.as_f64()? as f32;
    if line_box <= 0.0 || words.is_empty() {
        return None;
    }
    let mut lines = 0u32;
    let mut current: Option<f32> = None;
    for word in words {
        match word.as_f64() {
            // Paragraph break: flush the running line (an empty paragraph
            // still occupies one line, matching `swf_wrap_lines`).
            None => {
                lines += 1;
                current = None;
            }
            Some(advance) => {
                let advance = advance as f32;
                current = Some(match current {
                    None => advance,
                    Some(run) if run + space_cost + advance > wrap_width => {
                        lines += 1;
                        advance
                    }
                    Some(run) => run + space_cost + advance,
                });
            }
        }
    }
    if current.is_some() {
        lines += 1;
    }
    (lines > 0).then_some(line_box * lines as f32)
}

/// Measure ONE node's resolved text (`raw["_ResolvedText_"]`) at the size the
/// renderer will draw it with: the EFFECTIVE (styled) size annotated by ui_ir
/// before layout, else the authored font size — both at the draw calibration.
/// `None` when the node carries no measurable text.
pub(crate) fn node_resolved_text_size(
    node: &crate::bb_scene::BbNode,
    canvas_scale: f32,
    renderer: &crate::text::TextRenderer,
) -> Option<(f32, f32)> {
    let text_value = node.raw.get("_ResolvedText_").and_then(|v| v.as_str())?;
    let text = (!text_value.trim().is_empty()).then_some(text_value)?;
    // Draw-metric annotations win: ui_ir's pre-layout pass measures the text
    // through the SAME glyph machinery the renderer will draw with (SWF font
    // advances at the IR font size — no TTF estimate, no calibration), so the
    // intrinsic box hugs the painted glyphs like the engine's does.
    let draw_w = node
        .raw
        .get("_DrawTextWidthPx_")
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0);
    let draw_h = node
        .raw
        .get("_DrawTextHeightPx_")
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0);
    if let (Some(w), Some(h)) = (draw_w, draw_h) {
        return Some((w as f32, h as f32));
    }
    let size_px = if let Some(effective) = node
        .raw
        .get("_EffectiveFontPx_")
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0)
    {
        // Measure == draw: the TTF estimate follows the same data-backed em
        // model as the renderer (IR font size = design-em px; plan P3.2
        // retired the tuned 1.5 calibration pair on both sides together).
        effective as f32
    } else {
        match node.text.as_ref().map(|t| &t.font_size) {
            Some(BbValue::Fixed(size)) if *size > 0.0 => {
                *size * canvas_scale
            }
            _ => return None,
        }
    };
    Some(renderer.measure(text, crate::text::FontKind::Mono, size_px))
}

