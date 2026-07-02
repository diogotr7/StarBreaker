//! Secondary (caption) text drawing for the IR compositor — the value side
//! of a `ComponentLabelCaptionPair`, drawn after (or instead of) the primary
//! label (split from engine_01 by responsibility — line-cap guard).

#[allow(unused_imports)]
use super::*;
use crate::bb_layout::Rect;
use crate::compose::ComposeContext;
use crate::text::{FontKind, TextAlign, TextRenderer, VerticalAlign};
use crate::ui_ir::{UiIrNode, UiIrTextPayload};
use image::RgbaImage;

/// Draw a node's SECONDARY (caption) text — factored out of
/// [`draw_text_node`] so a pair whose primary label is hidden
/// (`labelProperties.show=false`, e.g. the lift-call floor heading) still
/// draws its caption.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_pair_secondary_text(
    img: &mut RgbaImage,
    node: &UiIrNode,
    renderer: &TextRenderer,
    ctx: &ComposeContext<'_>,
    secondary_rect: Rect,
    secondary_nominal_font_size: f32,
    secondary_fallback_font_size: f32,
    secondary_font_style_scale: f32,
) {
    let Some(UiIrTextPayload::Resolved { text: secondary }) = node.secondary_text_payload.as_ref()
    else {
        return;
    };
    if secondary.is_empty() {
        return;
    }
    let mut secondary_colour = resolved_text_colour(node, node.secondary_text_style.as_ref(), ctx);
    secondary_colour[3] = ((secondary_colour[3] as f32) * node.alpha.clamp(0.0, 1.0)).round() as u8;
    let secondary_align = node
        .secondary_text_style
        .as_ref()
        .map(|style| TextAlign::from_bb_str(&style.alignment))
        .unwrap_or(TextAlign::Left);
    let secondary_vertical_align = node
        .secondary_text_style
        .as_ref()
        .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
        .unwrap_or(VerticalAlign::Centre);
    let secondary_selected_font = select_imported_ui_font(
        ctx,
        node.secondary_text_style
            .as_ref()
            .or(node.text_style.as_ref()),
    );
    let secondary_line_spacing = node
        .secondary_text_style
        .as_ref()
        .or(node.text_style.as_ref())
        .and_then(|style| style.line_spacing);
    let secondary_ttf_font_scale = secondary_font_style_scale;
    let mut secondary_swf_font_size = secondary_nominal_font_size.max(1.0);
    let secondary_rect = apply_font_style_vertical_offset(
        secondary_rect,
        node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
    );
    if let Some(selection) = secondary_selected_font.as_ref() {
        if node.auto_font_size {
            secondary_swf_font_size = fit_swf_font_size_to_rect(
                renderer,
                secondary,
                selection.font,
                secondary_rect,
                secondary_swf_font_size,
                secondary_align,
                secondary_vertical_align,
                secondary_line_spacing,
            );
        }
    }
    let secondary_used_swf = secondary_selected_font.as_ref().is_some_and(|selection| {
        renderer.draw_swf_font(
            img,
            secondary,
            secondary_rect,
            selection.font,
            ctx.assets.font_edit_text_metrics(&selection.symbol),
            secondary_swf_font_size,
            secondary_colour,
            secondary_align,
            secondary_vertical_align,
            secondary_line_spacing,
            node.secondary_text_style
                .as_ref()
                .or(node.text_style.as_ref())
                .and_then(|style| style.letter_spacing)
                .unwrap_or(0.0),
        )
    });
    if !secondary_used_swf {
        renderer.draw(
            img,
            secondary,
            secondary_rect,
            FontKind::Sans,
            secondary_fallback_font_size,
            secondary_colour,
            secondary_align,
            secondary_vertical_align,
            scale_line_spacing(secondary_line_spacing, secondary_ttf_font_scale),
        );
    }
}
