#[allow(unused_imports)]
use super::*;
// Consolidated engine chunk 01 (formerly: part_01.part, part_02.part, part_03.part, part_04.part, part_05.part, part_06.part, part_07.part).
//   part_01.part: Canonical UI IR renderer for generic BuildingBlocks output.
//   part_06.part: Registered fallback (crates/starbreaker-ui/docs/ui-fallback-register.md): the word-space gap inserted between
//   part_07.part: Fit a SWF text size to its container rect using *real glyph metrics*.

// Canonical UI IR renderer for generic BuildingBlocks output.
// GOLDEN RULE: No hard-coding, heuristic workarounds, no procedural fallbacks. Avoid targetted scoping. Find the root cause and fix issues instead. Find the source data even it means doing things the hard way. This is intended to be a pipeline that is completely generic that can work for any UI on any ship and must not have targetted hacks that won't fix the issue in other places. This will keep the code lean and generic. Think how the game-engine would implement it.
//
// This module is the first Phase 2 step toward deterministic renderer
// consumption of [`crate::ui_ir::UiIrDocument`]. It renders the generic BB
// path directly from IR fields that were materialized in Phase 1: layout,
// fill colours, borders, asset references, and resolved text payload/style.
//

use image::RgbaImage;
use image::imageops;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use tiny_skia::{BlendMode, Color, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as TskRect, Stroke, Transform};

use crate::bb_atlas::AtlasLibrary;
use crate::bb_assets::UiAssetResolver;
use crate::bb_layout::Rect;
use crate::compose::ComposeContext;
use crate::error::UiError;
use crate::text::{FontKind, TextAlign, TextRenderer, VerticalAlign};
use crate::swf_assets::{FontGlyphSet, SwfAssetLibrary};
use crate::ui_ir::{
    validate_ui_ir_document, UiIrAssetLayout, UiIrBorder, UiIrColourBlendMode, UiIrDocument,
    UiIrNode, UiIrRect, UiIrTextPayload, UiIrTextStyle, UiIrValue,
};

// The TTF (DejaVu) fallback follows the same data-backed em model as the SWF
// path: the IR font size IS the design-em pixel size and rusttype's Scale
// already normalises to the ascent+|descent| span, so NO calibration constant
// applies (plan P3.2 retired the tuned 1.5 here and its layout twin, both
// calibrated when DejaVu stood in for game fonts on live screens — a case
// that no longer exists: the shared fontlib merges into every binding's
// assets, so the TTF path only draws in no-game-data environments).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugTextRects {
    pub primary: Rect,
    pub secondary: Option<Rect>,
    pub primary_text_origin: (f32, f32),
    pub secondary_text_origin: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugTextDrawnBounds {
    pub primary: Rect,
    pub secondary: Option<Rect>,
}

/// Render a generic BuildingBlocks IR document without consulting raw BB data.
///
/// This renderer intentionally consumes only the canonical IR plus style/assets.
/// SWF-specific and raw-source fallback behavior remains in the legacy renderer
/// until the later Phase 2 split is complete.
pub fn render_ui_ir_document(
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
) -> Result<RgbaImage, UiError> {
    validate_ui_ir_document(document)
        .map_err(|errors| UiError::RenderError(format!("invalid UI IR: {}", errors.join("; "))))?;

    if document.target_width == 0 || document.target_height == 0 {
        return Err(UiError::RenderError(format!(
            "invalid target size {}x{}",
            document.target_width, document.target_height
        )));
    }

    let mut pixmap = Pixmap::new(document.target_width, document.target_height)
        .ok_or_else(|| UiError::RenderError("pixmap allocation failed".into()))?;

    let bg = &ctx.style.background;
    pixmap.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));

    let mut draw_order: Vec<&UiIrNode> = document.nodes.iter().filter(|node| node.is_active).collect();
    // Keep authored IR order within each layer. Synthetic ids are an
    // implementation detail and must not affect visual stacking.
    draw_order.sort_by_key(|node| node.layer);

    let text_renderer = TextRenderer::new();

    for node in &draw_order {
        with_node_clip(
            &mut pixmap,
            node.clip_rect.as_ref(),
            ir_rect_to_layout_rect(node.computed_rect),
            |pixmap| draw_non_text_node(node, document, ctx, atlas, pixmap),
        );
    }

    // Token-driven borders are semantic overlays and should render after base
    // fills/assets so child card fills do not obscure parent outlines.
    for node in &draw_order {
        let Some(border) = &node.border else {
            continue;
        };
        if !border_uses_colour_tokens(border) {
            continue;
        }
        let rect = resolved_linear_progress_meter_rect(node, document)
            .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect));
        if rect.w < 0.5 || rect.h < 0.5 {
            continue;
        }
        with_node_clip(&mut pixmap, node.clip_rect.as_ref(), rect, |pixmap| {
            draw_ir_border(pixmap, rect, border, node.alpha, ctx, node.corner_radius);
        });
    }

    // Keep progress meters on top of base chrome/background fills.
    for node in &draw_order {
        if node.meter_progress.is_none() {
            continue;
        }
        let rect = resolved_linear_progress_meter_rect(node, document)
            .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect));
        let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
            continue;
        };
        with_node_clip(&mut pixmap, node.clip_rect.as_ref(), rect, |pixmap| {
            draw_linear_progress_meter(node, ctx, pixmap, tsk_rect);
        });
    }

    let mut img = pixmap_to_rgba_image(pixmap)?;
    let mut text_draw_order = draw_order.clone();
    text_draw_order.sort_by(|left, right| {
        let left_rect = ir_rect_to_layout_rect(left.computed_rect);
        let right_rect = ir_rect_to_layout_rect(right.computed_rect);
        let left_key = (
            left_rect.x.round() as i32,
            left_rect.y.round() as i32,
            left_rect.w.round() as i32,
            left_rect.h.round() as i32,
        );
        let right_key = (
            right_rect.x.round() as i32,
            right_rect.y.round() as i32,
            right_rect.w.round() as i32,
            right_rect.h.round() as i32,
        );
        let left_len = resolved_text_payload(left).map(|text| text.len()).unwrap_or(usize::MAX);
        let right_len = resolved_text_payload(right)
            .map(|text| text.len())
            .unwrap_or(usize::MAX);

        left_key
            .cmp(&right_key)
            .then(left_len.cmp(&right_len))
            .then(left.id.cmp(&right.id))
    });

    let mut seen_text_rects: HashSet<(i32, i32, i32, i32)> = HashSet::new();
    for node in &text_draw_order {
        with_node_clip_image(
            &mut img,
            node.clip_rect.as_ref(),
            ir_rect_to_layout_rect(node.computed_rect),
            |img| {
                draw_text_node(
                    img,
                    node,
                    document,
                    &text_renderer,
                    ctx,
                    &mut seen_text_rects,
                );
            },
        );
    }

    Ok(img)
}

fn draw_non_text_node(
    node: &UiIrNode,
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
) {
    let rect = resolved_linear_progress_meter_rect(node, document)
        .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect));
    if rect.w < 0.5 || rect.h < 0.5 {
        return;
    }

    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };

    // `WidgetRuntimeImage` is a live 3D primitive in the engine (the SELF-STATUS
    // "Own Vehicle Hologram"). When a hologram fetcher and ship geometry are
    // available, rasterise the loaded vehicle's hull into the node rect, tinted
    // by the node's authored background fill (the per-manufacturer holo colour),
    // and skip the flat authored fill. Falls through to the normal draw when no
    // fetcher / geometry is available (tests, non-vehicle exports).
    //
    // The displayed hologram is the one carrying an authored holo tint
    // (`background_fill_colour`); a sibling runtime-image without a tint (the
    // self diagram's secondary "old man vehicle" placeholder) is not painted.
    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetRuntimeImage")
        && let Some(fetcher) = ctx.hologram_fetcher
        && let Some(tint) = node.background_fill_colour
    {
        // The hologram is a full-screen background 3D render, not 2D UI confined
        // to the inset MFD content sub-rect. Fill the image WIDTH and run from
        // the image top down to the footer — the node rect's own bottom edge
        // (`rect.y + rect.h`), which sits just above the footer chrome — instead
        // of the authored content rect (16px-inset, overflowing the top).
        let holo_x = 0.0_f32;
        let holo_y = 0.0_f32;
        let holo_bottom = (rect.y + rect.h).min(document.target_height as f32);
        let holo_w = (document.target_width as f32).round().max(1.0) as u32;
        let holo_h = (holo_bottom - holo_y).round().max(1.0) as u32;
        if let Some(holo) = fetcher.fetch_vehicle_hologram(holo_w, holo_h, tint)
            && let Some(img) = RgbaImage::from_raw(holo.width, holo.height, holo.rgba)
        {
            blit_atlas_image_tinted_with_mode(
                pixmap,
                &img,
                holo_x.round() as i32,
                holo_y.round() as i32,
                [1.0, 1.0, 1.0, 1.0],
                node.alpha,
                BlendMode::SourceOver,
            );
            return;
        }
    }

    // MFD-radar scope: the radar's `WindowContainer` is a `BuildingBlocks_WidgetWindow`
    // / `Primitive` node (material `…/map_window.mtl`) — a live render-to-texture 3D
    // scene the 2D compositor cannot run, so its 3D children collapse to a point at
    // rest. When a fetcher is available, project the REAL disc texture (resolved from
    // the radar disc node's `…radial_grid.mtl`) into the full window rect, tinted by
    // the brand accent (the disc's `Circle_Ripple_Textured` rings author `Accent1`).
    // Mirrors the SELF-STATUS hologram path above. See `starbreaker_gfx::radar_plane`.
    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetWindow")
        && node
            .primitive_material
            .as_deref()
            .is_some_and(|m| m.to_ascii_lowercase().contains("map_window"))
        && let Some(fetcher) = ctx.hologram_fetcher
        && let Some(disc_material) = document.nodes.iter().find_map(|n| {
            n.primitive_material
                .as_deref()
                .filter(|m| m.to_ascii_lowercase().contains("radial_grid"))
        })
    {
        let tint = resolve_colour_token(ctx, "Accent1")
            .or_else(|| resolve_colour_token(ctx, "Base"))
            .map(|c| [c[0], c[1], c[2]])
            .unwrap_or([1.0, 1.0, 1.0]);
        // The rotating sweep wedge's material (the `Circle_Radial_Idle_Wave` node)
        // — brand-resolved like the disc.
        let sweep_material = document.nodes.iter().find_map(|n| {
            n.primitive_material
                .as_deref()
                .filter(|m| m.to_ascii_lowercase().contains("radial_idle"))
        });
        // The authored radar spokes (`Circle_Line_*`, `line_a` material) read
        // straight from the compiled radar-plane nodes + the brand-resolved spoke
        // material path (see `collect_radar_spokes`). Nothing is invented; an empty
        // list means no spoke nodes were loaded.
        let (spokes, spoke_material) = collect_radar_spokes(&document.nodes, ctx);
        let heading = collect_radar_heading_tape(&document.nodes);
        let win_w = rect.w.round().max(1.0) as u32;
        let win_h = rect.h.round().max(1.0) as u32;
        if let Some(plane) = fetcher.fetch_radar_plane(
            win_w,
            win_h,
            tint,
            disc_material,
            sweep_material,
            spoke_material,
            &spokes,
            heading,
        )
            && let Some(img) = RgbaImage::from_raw(plane.width, plane.height, plane.rgba)
        {
            blit_atlas_image_tinted_with_mode(
                pixmap,
                &img,
                rect.x.round() as i32,
                rect.y.round() as i32,
                [1.0, 1.0, 1.0, 1.0],
                node.alpha,
                BlendMode::SourceOver,
            );
            return;
        }
    }

    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetCircle")
        || node.node_type.eq_ignore_ascii_case("widget_circle")
    {
        draw_widget_circle(ctx, node, pixmap, tsk_rect);
        return;
    }

    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetSeparator")
        || node.node_type.eq_ignore_ascii_case("widget_separator")
    {
        // A widget-standard separator that resolved a brand SVG (the dotted
        // glyph) falls through to the asset_ref rasteriser below (Contain-fit, no
        // custom_shape); the bare / procedural strips keep the solid-fill draw.
        let has_brand_svg = node
            .asset_ref
            .as_deref()
            .is_some_and(|asset| asset.to_ascii_lowercase().ends_with(".svg"));
        if !has_brand_svg {
            draw_widget_separator(ctx, pixmap, node, tsk_rect, node.alpha);
            return;
        }
    }

    if node.meter_progress.is_some() {
        draw_linear_progress_meter(node, ctx, pixmap, tsk_rect);
        return;
    }

    if node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_WidgetManufacturerLogo")
    {
        draw_manufacturer_logo_ir(node, document, ctx, atlas, pixmap);
    }

    // A `WidgetList` is a pure layout container in the engine: it renders its
    // items, never its own authored background (the power screen's
    // `list_PowerAssignmentList` carries an enabled editor-yellow ColorSolid
    // that does not appear in-game).
    let is_list_container = node.node_type.eq_ignore_ascii_case("BuildingBlocks_WidgetList")
        || node.node_type.eq_ignore_ascii_case("widget_list");
    let skip_background_fill =
        (node.custom_shape.is_some() && node.asset_ref.is_some()) || is_list_container;
    if !skip_background_fill {
        let resolved_background_fill = node.background_fill_colour.or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
                .map(|mut fill| {
                    fill[3] *= node.background_fill_alpha.unwrap_or(1.0);
                    fill
                })
        });
        if let Some(fill) = resolved_background_fill
            && fill[3] > 0.005
        {
            let fill_rect = inset_fill_rect_for_token_border(node, rect, tsk_rect);
            if node.name.eq_ignore_ascii_case("Root")
                && node.node_type.eq_ignore_ascii_case("display_widget")
                && rect.w >= document.target_width as f32 * 0.95
                && rect.h <= document.target_height as f32 * 0.16
            {
                // Header root containers are layout scaffolding, not visible chrome.
            } else if let Some(radius) = node.corner_radius.filter(|r| *r > 0.0) {
                fill_rounded_rect_ts_with_mode(
                    pixmap,
                    fill_rect,
                    radius,
                    fill,
                    node.alpha,
                    node_colour_blend_mode(node),
                );
            } else {
                fill_rect_ts_with_mode(pixmap, fill_rect, fill, node.alpha, node_colour_blend_mode(node));
            }
        }
    }

    if let Some(polygon) = node.polygon.as_ref() {
        draw_ir_polygon(node, polygon, rect, ctx, pixmap);
    }

    if let Some(asset_ref) = node.asset_ref.as_deref() {
        let normalised_asset_ref = UiAssetResolver::normalise_path(asset_ref);
        let iw = rect.w.round().max(1.0) as u32;
        let ih = rect.h.round().max(1.0) as u32;
        let is_svg = normalised_asset_ref.ends_with(".svg");
        let is_overlay = UiAssetResolver::is_reference_overlay(asset_ref)
            || UiAssetResolver::is_reference_overlay(&normalised_asset_ref);
        // Fetch the SVG bytes up front so the brand `colorstyle:` recolour can read
        // the asset's authored role before the fill_override is decided.
        let svg_bytes: Option<Vec<u8>> = (is_svg && !is_overlay)
            .then(|| {
                atlas.fetch_raw(asset_ref).or_else(|| {
                    (normalised_asset_ref != asset_ref)
                        .then(|| atlas.fetch_raw(&normalised_asset_ref))
                        .flatten()
                })
            })
            .flatten();
        let explicit_override = custom_shape_fill_override(node, ctx).or_else(|| {
            if is_svg {
                node.icon_tint_colour.or_else(|| {
                    node.icon_tint_colour_token
                        .as_deref()
                        .and_then(|token| resolve_colour_token(ctx, token))
                })
            } else {
                None
            }
        });
        // SVG `colorstyle:` brand recolour. HUD glyph SVGs author placeholder
        // fills and encode the brand role + opacity in the path ids. A UNIFORM
        // (one role, one opacity) SVG resolves to a single whole-image overlay
        // (`svg_colorstyle_fill_override`); a non-uniform / multi-role SVG —
        // the velocity cross-line (`Accent1` at 85/50) and cross-cap (`Critical`
        // at 100/70) — is recoloured PER PATH in the bytes (`recolour_colorstyle_svg`)
        // since one overlay cannot represent per-path opacity. Both are lowest
        // priority: an explicit custom-shape fill or icon tint still wins.
        let uniform_colorstyle = (explicit_override.is_none())
            .then(|| svg_bytes.as_deref().and_then(|bytes| svg_colorstyle_fill_override(bytes, ctx)))
            .flatten();
        let fill_override = explicit_override.or(uniform_colorstyle);
        let recoloured_svg: Option<Vec<u8>> = (fill_override.is_none())
            .then(|| {
                svg_bytes.as_deref().and_then(|bytes| {
                    crate::bb_svg::recolour_colorstyle_svg(bytes, |role| {
                        resolve_surface_colour_token(ctx, role)
                    })
                })
            })
            .flatten();
        let effective_svg_bytes = recoloured_svg.as_deref().or(svg_bytes.as_deref());
        let resolved_image = if is_overlay {
            None
        } else if is_svg {
            effective_svg_bytes
                .and_then(|bytes| rasterize_svg_for_node(node, bytes, iw, ih, fill_override))
        } else {
            atlas.resolve(asset_ref, iw, ih).or_else(|| {
                (normalised_asset_ref != asset_ref)
                    .then(|| atlas.resolve(&normalised_asset_ref, iw, ih))
                    .flatten()
            })
        };
        if let Some(mut img) = resolved_image {
            // Draw-rect probe (ledger 45): at-rest cards render at low alpha
            // (battery card = 0.2 "depleted"), so element width must be read from
            // the LAID-OUT draw rect, not pixel-scraped from a dim PNG. Gated by
            // `BB_DRAW_RECT_PROBE`: `1` prints every asset draw, any other value is
            // a case-insensitive substring filter on the node name / asset path.
            // Render-neutral (stderr only when the var is set).
            if let Ok(probe) = std::env::var("BB_DRAW_RECT_PROBE") {
                let probe_lc = probe.to_ascii_lowercase();
                if probe == "1"
                    || node.name.to_ascii_lowercase().contains(&probe_lc)
                    || normalised_asset_ref.to_ascii_lowercase().contains(&probe_lc)
                {
                    eprintln!(
                        "BB_DRAW_RECT_PROBE name={} rect=({:.0},{:.0},{:.0},{:.0}) raster={}x{} asset={}",
                        node.name, rect.x, rect.y, rect.w, rect.h,
                        img.width(), img.height(), normalised_asset_ref,
                    );
                }
            }
            let is_nine_slice_custom_shape = node
                .custom_shape
                .as_ref()
                .is_some_and(|shape| shape.enable_nine_slice_rect.unwrap_or(false));
            if normalised_asset_ref.ends_with(".svg")
                && fill_override.is_some()
                && !is_nine_slice_custom_shape
            {
                img = strip_custom_shape_uniform_matte(&img);
            }
            img = apply_asset_layout_flip(node, img);
            let (rotated_img, rot_dx, rot_dy) = apply_node_rotation(node, img);
            img = rotated_img;
            let draw_x = rect.x as i32 + rot_dx;
            let draw_y = rect.y as i32 + rot_dy;

            if fill_override.is_none()
                && let Some(mask_tint) = white_mask_overlay_tint(node, asset_ref, Some(&img), ctx)
            {
                blit_white_mask_overlay_linear(pixmap, &img, draw_x, draw_y, mask_tint, node.alpha);
            } else {
                let tint = image_tint_for_blit(node, asset_ref, fill_override, Some(&img), ctx);
                let blend_mode = image_blend_mode_for_node(node, asset_ref);
                blit_atlas_image_tinted_with_mode(pixmap, &img, draw_x, draw_y, tint, node.alpha, blend_mode);
            }
        }
    }

    if let Some(border) = &node.border
        && !border_uses_colour_tokens(border)
    {
        draw_ir_border(pixmap, rect, border, node.alpha, ctx, node.corner_radius);
    }

    if node_draws_rect_stroke(node)
        && let Some(stroke_colour) = node.stroke_colour
    {
        draw_rect_stroke_ts(
            pixmap,
            tsk_rect,
            stroke_colour,
            node.stroke_extent.unwrap_or(0.0),
            node.alpha,
        );
    }
}

/// Whether a node draws a rectangle stroke around its rect (the box outline for
/// shape/separator widgets). Requires a `stroke_colour`, a positive
/// `stroke_extent`, and that the node is neither a `render_shape` custom shape
/// (which strokes its own path) NOR a `WidgetTextField`: a text field's
/// `StrokeColor` is its GLYPH outline drawn via the text path, not a box around
/// the field rect (the DRAK velocity-num readouts author a white ~0.64α text
/// StrokeColor that must outline the digits, not box them).
pub(crate) fn node_draws_rect_stroke(node: &UiIrNode) -> bool {
    node.stroke_colour.is_some()
        && node.stroke_extent.unwrap_or(0.0) > 0.0
        && !node
            .custom_shape
            .as_ref()
            .and_then(|shape| shape.render_shape)
            .unwrap_or(false)
        && !node.node_type.eq_ignore_ascii_case("widget_text_field")
}

fn border_uses_colour_tokens(border: &UiIrBorder) -> bool {
    border.top.colour_token.is_some()
        || border.right.colour_token.is_some()
        || border.bottom.colour_token.is_some()
        || border.left.colour_token.is_some()
}

fn inset_fill_rect_for_token_border(node: &UiIrNode, rect: Rect, fallback: TskRect) -> TskRect {
    let Some(border) = &node.border else {
        return fallback;
    };
    if !border_uses_colour_tokens(border) {
        return fallback;
    }

    let inset = border
        .top
        .width
        .max(border.right.width)
        .max(border.bottom.width)
        .max(border.left.width)
        .max(0.0);
    if inset <= 0.0 {
        return fallback;
    }

    let inset_w = rect.w - inset * 2.0;
    let inset_h = rect.h - inset * 2.0;
    if inset_w <= 0.5 || inset_h <= 0.5 {
        return fallback;
    }

    TskRect::from_xywh(rect.x + inset, rect.y + inset, inset_w, inset_h).unwrap_or(fallback)
}

fn brand_slug_from_ir(document: &UiIrDocument, ctx: &ComposeContext<'_>) -> String {
    let source = document.selected_style_source.as_deref().unwrap_or(&ctx.style.name);
    let token = source
        .split(':')
        .next_back()
        .unwrap_or(source)
        .trim()
        .trim_start_matches("s_")
        .to_ascii_lowercase();

    token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
}

fn draw_manufacturer_logo_ir(
    node: &UiIrNode,
    document: &UiIrDocument,
    ctx: &ComposeContext<'_>,
    atlas: &AtlasLibrary<'_>,
    pixmap: &mut Pixmap,
) {
    let draw_rect = ir_rect_to_layout_rect(node.computed_rect);
    if draw_rect.w < 0.5 || draw_rect.h < 0.5 {
        return;
    }

    let brand = brand_slug_from_ir(document, ctx);
    let logo_brand = brand_logo_slug(&brand);
    let brand_title = brand_title(logo_brand);
    let candidates = [
        format!("UI/Textures/Signs/Brands/{logo_brand}/{brand_title}_logo.svg"),
        format!("UI/Textures/Vector/General/BrandLogos/logo_{logo_brand}_a.svg"),
        format!("UI/Textures/Signs/Brands/{logo_brand}/{brand_title}_logo.dds"),
    ];

    let iw = draw_rect.w.round().max(1.0) as u32;
    let ih = draw_rect.h.round().max(1.0) as u32;
    let tint = manufacturer_logo_tint(node, ctx);
    let fill_override = Some(tint);

    for raw_path in candidates {
        let norm = UiAssetResolver::normalise_path(&raw_path);
        if UiAssetResolver::is_reference_overlay(&norm) {
            continue;
        }

        if norm.ends_with(".svg") {
            // The engine renders the logo SVG per its AUTHORED asset layout —
            // contain-fit at the authored contain position (the medical
            // footer's Logo authors Contain/0/0: the square Bioticorp SVG
            // width-fits its 120×140 box top-anchored, reference rows
            // 1016–1055). No alpha-balance recentring (a prior recentring
            // offset drew the glyph ~11px low and the stretch ~15% tall).
            let contained = node
                .asset_layout
                .as_ref()
                .and_then(|layout| layout.scaling_behavior.as_deref())
                .is_some_and(|behavior| behavior.eq_ignore_ascii_case("Contain"));
            let rasterized = atlas.fetch_raw(&norm).and_then(|svg_bytes| {
                if contained {
                    let layout = node.asset_layout.as_ref().expect("contain layout");
                    let (cx, cy) = flip_adjusted_contain_position(layout);
                    crate::bb_svg::rasterize_svg_contained(
                        &svg_bytes,
                        iw,
                        ih,
                        fill_override,
                        cx,
                        cy,
                    )
                } else {
                    crate::bb_svg::rasterize_svg(&svg_bytes, iw, ih, fill_override)
                }
            });
            if let Some(img) = rasterized {
                blit_atlas_image_tinted(
                    pixmap,
                    &img,
                    draw_rect.x as i32,
                    draw_rect.y as i32,
                    [1.0, 1.0, 1.0, 1.0],
                    node.alpha,
                );
                return;
            }
            continue;
        }

        if let Some(img) = atlas.resolve(&norm, iw, ih) {
            blit_atlas_image_tinted(
                pixmap,
                &img,
                draw_rect.x as i32,
                draw_rect.y as i32,
                tint,
                node.alpha,
            );
            return;
        }
    }
}

pub(crate) fn manufacturer_logo_tint(node: &UiIrNode, ctx: &ComposeContext<'_>) -> [f32; 4] {
    node.icon_tint_colour
        .or_else(|| {
            node.icon_tint_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .or(node.stroke_colour)
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .unwrap_or([1.0, 1.0, 1.0, 1.0])
}

fn brand_logo_slug(slug: &str) -> &str {
    match slug {
        "bioc" => "bioticorp",
        other => other,
    }
}

fn brand_title(slug: &str) -> String {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// FOREGROUND (glyph/icon) token resolution. The enum truth is
/// [`crate::style::colour_roles::bb_colour_style_enum_index`]; divergences
/// here are reference-verified:
/// - `Accent1`/`Base` → slot 0: foreground accents render the brand's light
///   primary slot (medical icon overlays), with the primary tint as the
///   no-palette fallback.
/// - `Bright` → enum slot 6 (NO divergence — the muted light-grey backing
///   dim secondary text such as a caption's `Heading6` "200/200" value;
///   the surface/brand-apply resolver is the one that diverges to 0).
/// - `Background` resolves the parsed style background field (kept as a
///   field reference rather than a slot index).
/// Non-enum aliases were deleted 2026-06-12 — they occur in no DataCore
/// record (2026-06-12 token audit — no game record contains them).
pub(crate) fn resolve_colour_token(ctx: &ComposeContext<'_>, token: &str) -> Option<[f32; 4]> {
    let key = token.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }

    match key.as_str() {
        "accent1" | "base" => {
            return style_colour_slot_rgba(ctx, 0).or_else(|| Some(style_primary_rgba_local(ctx)));
        }
        "background" => {
            return Some([
                ctx.style.background.r as f32 / 255.0,
                ctx.style.background.g as f32 / 255.0,
                ctx.style.background.b as f32 / 255.0,
                ctx.style.background.a as f32 / 255.0,
            ]);
        }
        _ => {}
    }
    crate::style::colour_roles::bb_colour_style_enum_index(&key)
        .and_then(|slot| style_colour_slot_rgba(ctx, slot))
}

/// SURFACE (shape-fill) token resolution: pure enum indexing — `Accent1`
/// keeps its enum surface slot 4 (the medical fingerprint's darker blue),
/// unlike the foreground path above.
pub(crate) fn resolve_surface_colour_token(ctx: &ComposeContext<'_>, token: &str) -> Option<[f32; 4]> {
    let key = token.trim().to_ascii_lowercase();
    match key.as_str() {
        "accent1" => style_colour_slot_rgba(ctx, 4),
        _ => resolve_colour_token(ctx, token),
    }
}

/// Brand `colorstyle:` recolour for an SVG asset. HUD glyph SVGs (the g-force
/// diagram cross-line + caps, …) author every path with a placeholder `fill` and
/// encode the real brand role in the path id (`opacity:50_colorstyle:Accent1_…`);
/// see [`crate::bb_svg::parse_uniform_colorstyle`]. When the SVG is uniform (one
/// role — a single fill cannot represent a multi-role SVG) the role is resolved as
/// a SURFACE token (SVG paths are shape fills, so `Accent1` is the surface slot,
/// matching `custom_shape_fill_override`) and the encoded opacity scales the alpha.
/// `None` (non-colorstyle / multi-role) leaves the SVG's authored fills unchanged.
fn svg_colorstyle_fill_override(svg_bytes: &[u8], ctx: &ComposeContext<'_>) -> Option<[f32; 4]> {
    let (role, alpha) = crate::bb_svg::parse_uniform_colorstyle(svg_bytes)?;
    let mut rgba = resolve_surface_colour_token(ctx, &role)?;
    rgba[3] *= alpha;
    Some(rgba)
}

pub(crate) fn style_colour_slot_rgba(ctx: &ComposeContext<'_>, index: usize) -> Option<[f32; 4]> {
    ctx.style.colour_slots.get(index).map(|colour| {
        [
            colour.r as f32 / 255.0,
            colour.g as f32 / 255.0,
            colour.b as f32 / 255.0,
            colour.a as f32 / 255.0,
        ]
    })
}

fn style_primary_rgba_local(ctx: &ComposeContext<'_>) -> [f32; 4] {
    let pt = &ctx.style.primary_tint;
    [
        pt.r as f32 / 255.0,
        pt.g as f32 / 255.0,
        pt.b as f32 / 255.0,
        pt.a as f32 / 255.0,
    ]
}


pub(crate) fn custom_shape_fill_override(node: &UiIrNode, ctx: &ComposeContext<'_>) -> Option<[f32; 4]> {
    let render_shape = node
        .custom_shape
        .as_ref()
        .and_then(|shape| shape.render_shape)
        .unwrap_or(false);
    if !render_shape {
        return None;
    }

    // A custom-shape fill/stroke is a *surface* element, so its colour tokens
    // resolve with surface semantics (e.g. `Accent1` → surface slot 4), not the
    // foreground semantics used for text/icons (where `Accent1` → slot 0). Using
    // the foreground resolver here rendered authored `Accent1` shape fills (e.g.
    // the medical "fingerprint") in the light slot-0 blue instead of the darker
    // slot-4 blue.
    node.icon_tint_colour
        .or_else(|| {
            node.icon_tint_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        })
        .or(node.stroke_colour)
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        })
}

pub(crate) fn image_tint_for_blit(
    node: &UiIrNode,
    asset_ref: &str,
    fill_override: Option<[f32; 4]>,
    img: Option<&RgbaImage>,
    ctx: &ComposeContext<'_>,
) -> [f32; 4] {
    if fill_override.is_some()
        && UiAssetResolver::normalise_path(asset_ref).ends_with(".svg")
    {
        return [1.0, 1.0, 1.0, 1.0];
    }
    if let Some(tint) = node.icon_tint_colour.or_else(|| {
        node.icon_tint_colour_token
            .as_deref()
            .and_then(|token| resolve_colour_token(ctx, token))
    }) {
        return tint;
    }
    if let Some(base) = white_mask_overlay_tint(node, asset_ref, img, ctx) {
        return base;
    }
    [1.0, 1.0, 1.0, 1.0]
}

/// The brand `Base` overlay for an overlay-enabled image whose texture is a
/// pure-white alpha mask (the annunciator chiclet glow Annunciator_On.tif —
/// shape entirely in the alpha channel); the MRAI brand authors the same
/// mechanism as explicit FillColor entries on its white mask textures while
/// DRAK relies on the overlay default. Coloured textures (card photos, the
/// MFD body backplates) carry their own colours and the editor-default
/// overlay flag must not tint them. `None` when this path does not apply
/// (explicit tints are resolved by the caller first).
fn white_mask_overlay_tint(
    node: &UiIrNode,
    asset_ref: &str,
    img: Option<&RgbaImage>,
    ctx: &ComposeContext<'_>,
) -> Option<[f32; 4]> {
    if node.colour_overlay_enabled
        && node.icon_tint_colour.is_none()
        && node.icon_tint_colour_token.is_none()
        && !UiAssetResolver::normalise_path(asset_ref).ends_with(".svg")
        && img.is_some_and(image_is_white_alpha_mask)
    {
        resolve_colour_token(ctx, "Base")
    } else {
        None
    }
}

fn srgb_channel_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_channel_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// Source-over composite of a white-mask glow in LINEAR light. The engine
/// blends in linear space; tiny-skia blends in the stored sRGB space, which
/// crushes low-alpha bright-over-dark blends — the annunciator glow rendered
/// (15,9,3) where linear-light blending (and the in-game reference) gives
/// ~(68,38,8) at the chiclet edge. Scoped to the white-mask overlay path;
/// the renderer-wide linear migration is a separate gated workstream
/// (crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md item 10).
pub(crate) fn blit_white_mask_overlay_linear(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
) {
    let tint_lin = [
        srgb_channel_to_linear(tint[0].clamp(0.0, 1.0)),
        srgb_channel_to_linear(tint[1].clamp(0.0, 1.0)),
        srgb_channel_to_linear(tint[2].clamp(0.0, 1.0)),
    ];
    let node_alpha = (tint[3] * alpha).clamp(0.0, 1.0);
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let data = pixmap.data_mut();
    for (sy, row) in img.rows().enumerate() {
        let py = dy + sy as i32;
        if py < 0 || py >= ph {
            continue;
        }
        for (sx, px) in row.enumerate() {
            let pxx = dx + sx as i32;
            if pxx < 0 || pxx >= pw {
                continue;
            }
            let coverage = px.0[3] as f32 / 255.0 * node_alpha;
            if coverage <= 0.0 {
                continue;
            }
            let idx = ((py * pw + pxx) * 4) as usize;
            // Pixmap stores premultiplied sRGB-encoded channels.
            let dst_a = data[idx + 3] as f32 / 255.0;
            let out_a = coverage + dst_a * (1.0 - coverage);
            if out_a <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let dst_unpremul = if dst_a > 0.0 {
                    (data[idx + c] as f32 / 255.0 / dst_a).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let dst_lin = srgb_channel_to_linear(dst_unpremul);
                let out_lin = tint_lin[c] * coverage + dst_lin * dst_a * (1.0 - coverage);
                let out_srgb = linear_channel_to_srgb((out_lin / out_a).clamp(0.0, 1.0));
                data[idx + c] = (out_srgb * out_a * 255.0).clamp(0.0, 255.0) as u8;
            }
            data[idx + 3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
}

/// `true` when every visible pixel of a sampled grid is (near-)pure white —
/// the texture's shape lives entirely in its alpha channel (a mask), so its
/// rendered colour must come from an overlay.
fn image_is_white_alpha_mask(img: &RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let step_x = (w / 32).max(1) as usize;
    let step_y = (h / 32).max(1) as usize;
    let mut visible = 0u32;
    for y in (0..h).step_by(step_y) {
        for x in (0..w).step_by(step_x) {
            let p = img.get_pixel(x, y).0;
            if p[3] >= 16 {
                visible += 1;
                if p[0] < 245 || p[1] < 245 || p[2] < 245 {
                    return false;
                }
            }
        }
    }
    visible > 0
}

pub(crate) fn image_blend_mode_for_node(node: &UiIrNode, asset_ref: &str) -> BlendMode {
    let normalised = UiAssetResolver::normalise_path(asset_ref);
    let render_shape = node
        .custom_shape
        .as_ref()
        .and_then(|shape| shape.render_shape)
        .unwrap_or(false);
    if !render_shape {
        return BlendMode::SourceOver;
    }

    if normalised.ends_with(".svg") {
        if matches!(node.colour_blend_mode, Some(UiIrColourBlendMode::Additive)) {
            BlendMode::Plus
        } else {
            BlendMode::SourceOver
        }
    } else {
        BlendMode::Plus
    }
}

pub(crate) fn rasterize_custom_shape_svg(
    node: &UiIrNode,
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
) -> Option<RgbaImage> {
    let Some(shape) = node.custom_shape.as_ref() else {
        return crate::bb_svg::rasterize_svg(svg_bytes, target_w, target_h, fill_override);
    };

    let rendered = if shape.enable_nine_slice_rect.unwrap_or(false)
        && let Some(nine_slice_rect) = shape.nine_slice_rect
    {
        crate::bb_svg::rasterize_svg_nine_slice(
            svg_bytes,
            target_w,
            target_h,
            fill_override,
            nine_slice_rect,
            shape.nine_slice_scale.unwrap_or(1.0),
        )
    } else {
        crate::bb_svg::rasterize_svg(svg_bytes, target_w, target_h, fill_override)
    }?;

    if shape.render_shape.unwrap_or(false) {
        // Preserve authored SVG geometry without post-raster crop/stretch.
        Some(rendered)
    } else {
        Some(rendered)
    }
}

pub(crate) fn rasterize_svg_for_node(
    node: &UiIrNode,
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
) -> Option<RgbaImage> {
    // `scalingBehavior: Contain` is the universal authored default on
    // `svgFill`, but the engine's rendering differs by widget role:
    // icon-element instances (the expanded icon-widget glyph, framework tag
    // "icon-element-instance") render aspect-preserved — the MFD footer's
    // 7.3×12.8 pixel arrow draws 31×48 in-game, not stretched to its square
    // icon box — while plain custom shapes keep the nine-slice/stretch
    // pipeline (the medical menu cards' thin edge-line svgs stretch to full
    // card height in-game despite the same authored Contain).
    // Only the EXPANDED icon-widget instances (framework tag
    // `icon-element-instance`) render aspect-preserved. Authored own-`icon`
    // card/system glyphs (the battery/output card icons) are squashed to their
    // box in-game — the battery glyph fills its flex-shrunk slot at the box
    // aspect, not the SVG's — so they keep the nine-slice/stretch pipeline.
    let is_icon_element_instance = node
        .resolved_style_tags
        .iter()
        .any(|tag| {
            tag.tag_name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("icon-element-instance"))
        });
    if (node.custom_shape.is_none() || is_icon_element_instance)
        && node
            .asset_layout
            .as_ref()
            .and_then(|layout| layout.scaling_behavior.as_deref())
            .is_some_and(|behavior| behavior.eq_ignore_ascii_case("Contain"))
    {
        let layout = node.asset_layout.as_ref().expect("contain layout");
        let (cx, cy) = flip_adjusted_contain_position(layout);
        return crate::bb_svg::rasterize_svg_contained(
            svg_bytes,
            target_w,
            target_h,
            fill_override,
            cx,
            cy,
        );
    }

    rasterize_custom_shape_svg(node, svg_bytes, target_w, target_h, fill_override)
}

/// The contain position to render a flipped SVG at, given that the whole-box
/// `apply_asset_layout_flip` runs afterward. A contained glyph sits at
/// `contain_position` within its (letterboxed) box; flipping the whole box would
/// move it to the OPPOSITE end (the ball-cap chevrons authored
/// `contain_y = 1.0` + `flipVertical` rendered at the box top instead of the
/// bottom, ~200px short of the dotted-line end). Pre-inverting the position on
/// each flipped axis means the subsequent whole-box flip lands the (now
/// mirrored) glyph back at its authored end.
fn flip_adjusted_contain_position(layout: &UiIrAssetLayout) -> (f32, f32) {
    let mut cx = layout.contain_position_x.unwrap_or(0.5);
    let mut cy = layout.contain_position_y.unwrap_or(0.5);
    if layout.flip_horizontal.unwrap_or(false) {
        cx = 1.0 - cx;
    }
    if layout.flip_vertical.unwrap_or(false) {
        cy = 1.0 - cy;
    }
    (cx, cy)
}

pub(crate) fn apply_asset_layout_flip(node: &UiIrNode, image: RgbaImage) -> RgbaImage {
    let Some(layout) = node.asset_layout.as_ref() else {
        return image;
    };

    let mut out = image;
    if layout.flip_horizontal.unwrap_or(false) {
        out = imageops::flip_horizontal(&out);
    }
    if layout.flip_vertical.unwrap_or(false) {
        out = imageops::flip_vertical(&out);
    }
    out
}

/// Rotate `img` by the node's `rotation_deg` (`orientation.z`) around its pivot,
/// returning the rotated image and the `(dx, dy)` the draw origin must shift by
/// so the pivot stays fixed in screen space. The velocity / g-force ball caps
/// rotate the chevron around the dotted-line end (their pivot), so a left cap's
/// `orientation.z = 90` swings the up-chevron to point left without moving the
/// pivot. No-op when the node has no rotation.
fn apply_node_rotation(node: &UiIrNode, img: RgbaImage) -> (RgbaImage, i32, i32) {
    let Some(deg) = node.rotation_deg.filter(|d| d.abs() > f32::EPSILON) else {
        return (img, 0, 0);
    };
    let (w, h) = (img.width() as f32, img.height() as f32);
    if w < 1.0 || h < 1.0 {
        return (img, 0, 0);
    }
    let (sin, cos) = deg.to_radians().sin_cos();
    let (px, py) = (node.pivot[0] * w, node.pivot[1] * h);
    // Forward-rotate the source corners about the pivot to size the output.
    let fwd = |x: f32, y: f32| {
        let (dx, dy) = (x - px, y - py);
        (px + dx * cos - dy * sin, py + dx * sin + dy * cos)
    };
    let corners = [fwd(0.0, 0.0), fwd(w, 0.0), fwd(w, h), fwd(0.0, h)];
    let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max_x = corners.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_y = corners.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
    let out_w = (max_x - min_x).ceil().max(1.0) as u32;
    let out_h = (max_y - min_y).ceil().max(1.0) as u32;
    let mut out = RgbaImage::new(out_w, out_h);
    for oy in 0..out_h {
        for ox in 0..out_w {
            // Inverse-map (rotate by -deg) the output pixel back to the source.
            let (rx, ry) = (ox as f32 + min_x - px, oy as f32 + min_y - py);
            let sx = px + rx * cos + ry * sin;
            let sy = py - rx * sin + ry * cos;
            if let Some(p) = sample_bilinear_rgba(&img, sx, sy) {
                out.put_pixel(ox, oy, p);
            }
        }
    }
    (out, min_x.round() as i32, min_y.round() as i32)
}

/// Alpha-weighted bilinear sample of `img` at fractional `(x, y)`. Premultiplies
/// by alpha before interpolating so a rotated glyph does not pick up the RGB of
/// transparent neighbours (which would darken its edges). Returns `None` outside
/// the image.
fn sample_bilinear_rgba(img: &RgbaImage, x: f32, y: f32) -> Option<image::Rgba<u8>> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    if x < -0.5 || y < -0.5 || x > w as f32 - 0.5 || y > h as f32 - 0.5 {
        return None;
    }
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let mut rgb = [0f32; 3];
    let mut alpha = 0f32;
    let mut alpha_weight = 0f32;
    for (dx, dy, wgt) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let (xi, yi) = (x0 + dx, y0 + dy);
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            continue;
        }
        let p = img.get_pixel(xi as u32, yi as u32).0;
        let a = p[3] as f32 / 255.0;
        rgb[0] += p[0] as f32 * a * wgt;
        rgb[1] += p[1] as f32 * a * wgt;
        rgb[2] += p[2] as f32 * a * wgt;
        alpha += p[3] as f32 * wgt;
        alpha_weight += a * wgt;
    }
    if alpha <= 0.0 || alpha_weight <= 0.0 {
        return Some(image::Rgba([0, 0, 0, 0]));
    }
    let inv = 1.0 / alpha_weight;
    Some(image::Rgba([
        (rgb[0] * inv).round().clamp(0.0, 255.0) as u8,
        (rgb[1] * inv).round().clamp(0.0, 255.0) as u8,
        (rgb[2] * inv).round().clamp(0.0, 255.0) as u8,
        alpha.round().clamp(0.0, 255.0) as u8,
    ]))
}


pub(crate) fn strip_custom_shape_uniform_matte(img: &RgbaImage) -> RgbaImage {
    let (width, height) = img.dimensions();
    let total_pixels = (width as usize).saturating_mul(height as usize).max(1);

    let mut opaque_counts: HashMap<[u8; 4], usize> = HashMap::new();
    let mut transparent_count = 0usize;
    for chunk in img.as_raw().chunks_exact(4) {
        let px = [chunk[0], chunk[1], chunk[2], chunk[3]];
        if px[3] == 0 {
            transparent_count += 1;
        }
        if px[3] == 255 {
            *opaque_counts.entry(px).or_insert(0) += 1;
        }
    }

    if opaque_counts.is_empty() || transparent_count == 0 {
        return img.clone();
    }

    // Pick the dominant opaque colour as the matte candidate. `opaque_counts` is
    // a `HashMap`, so iteration order is non-deterministic; break count ties by
    // the colour value itself (largest RGBA wins) so the chosen matte — and the
    // resulting flood-fill — is reproducible across renders.
    let Some((matte_px, matte_count)) = opaque_counts
        .iter()
        .max_by(|(px_a, count_a), (px_b, count_b)| count_a.cmp(count_b).then_with(|| px_a.cmp(px_b)))
        .map(|(px, count)| (*px, *count))
    else {
        return img.clone();
    };

    let total_opaque: usize = opaque_counts.values().sum();
    let matte_fraction = matte_count as f32 / total_opaque.max(1) as f32;
    let full_image_fraction = matte_count as f32 / total_pixels as f32;
    if matte_fraction < 0.85 || full_image_fraction < 0.9 {
        return img.clone();
    }

    let is_matte = |px: image::Rgba<u8>| {
        px[0] == matte_px[0] && px[1] == matte_px[1] && px[2] == matte_px[2] && px[3] == matte_px[3]
    };

    // Only strip matte that is actually connected to the asset border. This
    // avoids deleting centered monochrome logos where the dominant opaque colour
    // is the glyph itself, not an authored matte backdrop.
    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();
    let mut visited = vec![false; total_pixels];
    let push_seed = |x: u32, y: u32, queue: &mut VecDeque<(u32, u32)>, visited: &mut [bool]| {
        let idx = (y as usize)
            .saturating_mul(width as usize)
            .saturating_add(x as usize);
        if !visited[idx] && is_matte(*img.get_pixel(x, y)) {
            visited[idx] = true;
            queue.push_back((x, y));
        }
    };

    for x in 0..width {
        push_seed(x, 0, &mut queue, &mut visited);
        if height > 1 {
            push_seed(x, height - 1, &mut queue, &mut visited);
        }
    }
    for y in 0..height {
        push_seed(0, y, &mut queue, &mut visited);
        if width > 1 {
            push_seed(width - 1, y, &mut queue, &mut visited);
        }
    }

    if queue.is_empty() {
        return img.clone();
    }

    while let Some((x, y)) = queue.pop_front() {
        if x > 0 {
            let nx = x - 1;
            let ny = y;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
        if x + 1 < width {
            let nx = x + 1;
            let ny = y;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
        if y > 0 {
            let nx = x;
            let ny = y - 1;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
        if y + 1 < height {
            let nx = x;
            let ny = y + 1;
            let idx = (ny as usize)
                .saturating_mul(width as usize)
                .saturating_add(nx as usize);
            if !visited[idx] && is_matte(*img.get_pixel(nx, ny)) {
                visited[idx] = true;
                queue.push_back((nx, ny));
            }
        }
    }

    let mut out = img.clone();
    for y in 0..height {
        for x in 0..width {
            let idx = (y as usize)
                .saturating_mul(width as usize)
                .saturating_add(x as usize);
            if visited[idx] {
                let px = out.get_pixel_mut(x, y);
                *px = image::Rgba([0, 0, 0, 0]);
            }
        }
    }

    out
}

fn draw_linear_progress_meter(
    node: &UiIrNode,
    ctx: &ComposeContext<'_>,
    pixmap: &mut Pixmap,
    rect: TskRect,
) {
    let glow = [
        ctx.style.backlight.r as f32 / 255.0,
        ctx.style.backlight.g as f32 / 255.0,
        ctx.style.backlight.b as f32 / 255.0,
        (ctx.style.backlight.a as f32 / 255.0).max(0.8),
    ];

    let progress = node.meter_progress.unwrap_or(1.0).clamp(0.0, 1.0);
    if progress <= 0.0 {
        return;
    }

    if let Some(segmented_fill) = node.segmented_fill.as_ref().filter(|fill| fill.enabled) {
        let segment_width = if segmented_fill.segment_spacing_size > 0.0 {
            segmented_fill.segment_spacing_size
        } else {
            segmented_fill.segment_size
        }
        .max(0.0);
        let segment_gap = segmented_fill.segment_size.max(0.0);
        let segment_stride = segment_width + segment_gap;
        let segment_count = segmented_count_for_width(rect.width(), segment_width, segment_gap);
        if segment_count > 0 && segment_stride > 0.0 {
            let active_width = rect.width() * progress;
            let segment_colour = segmented_fill.segment_colour.unwrap_or(glow);
            for idx in 0..segment_count {
                let x = rect.x() + segmented_fill.segment_x_offset + (idx as f32 * segment_stride);
                if x >= rect.right() {
                    break;
                }
                let right = (x + segment_width).min(rect.right());
                if right <= x {
                    continue;
                }
                let segment_end = right - rect.x();
                if segment_end <= active_width {
                    if let Some(segment_rect) =
                        TskRect::from_xywh(x, rect.y(), right - x, rect.height())
                    {
                        fill_rect_ts(pixmap, segment_rect, segment_colour, node.alpha);
                    }
                }
            }
            return;
        }
    }

    let filled_w = (rect.width() * progress).max(1.0);
    if let Some(fill_rect) = TskRect::from_xywh(rect.x(), rect.y(), filled_w, rect.height()) {
        fill_rect_ts(pixmap, fill_rect, glow, node.alpha);
    }
}

pub fn debug_linear_progress_meter_rect(node: &UiIrNode, document: &UiIrDocument) -> Option<Rect> {
    (node.meter_progress.is_some()).then(|| {
        resolved_linear_progress_meter_rect(node, document)
            .unwrap_or_else(|| ir_rect_to_layout_rect(node.computed_rect))
    })
}

pub fn debug_node_draw_rect(node: &UiIrNode, document: &UiIrDocument) -> Rect {
    if let Some(meter_rect) = debug_linear_progress_meter_rect(node, document) {
        return meter_rect;
    }

    ir_rect_to_layout_rect(node.computed_rect)
}

fn resolved_linear_progress_meter_rect(node: &UiIrNode, document: &UiIrDocument) -> Option<Rect> {
    if node.meter_progress.is_none() {
        return None;
    }

    let parent = node
        .parent_id
        .and_then(|parent_id| document.nodes.iter().find(|candidate| candidate.id == parent_id))?;
    if !parent
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
    {
        return None;
    }
    if (node.anchor[1] - 1.0).abs() > 0.01 || node.pivot[1].abs() > 0.01 || node.authored_position[1].abs() > 0.01 {
        return None;
    }

    let mut rect = ir_rect_to_layout_rect(node.computed_rect);
    let text_rects = debug_text_rects(parent)?;
    let (pair_offset_x, _pair_offset_y) = right_anchored_label_caption_pair_offset(
        parent,
        text_rects.primary.h,
        text_rects.secondary.map(|secondary_rect| secondary_rect.h),
    );
    rect.x += pair_offset_x;
    // Pin the meter to the value text's baseline — the bottom of its drawn glyphs —
    // rather than the line-box bottom. The value (e.g. "200/200") has no descenders, so
    // the line box's empty descent space would otherwise push the bar ~1 descent too low.
    rect.y = debug_text_drawn_bounds(parent)
        .and_then(|bounds| bounds.secondary)
        .map(|secondary| secondary.y + secondary.h)
        .or_else(|| {
            text_rects
                .secondary
                .map(|secondary_rect| secondary_rect.y + secondary_rect.h)
        })
        .unwrap_or_else(|| text_rects.primary.y + text_rects.primary.h);
    Some(rect)
}

pub(crate) fn segmented_count_for_width(total_width: f32, segment_width: f32, segment_gap: f32) -> usize {
    if total_width <= 0.0 || segment_width <= 0.0 {
        return 0;
    }
    let stride = segment_width + segment_gap.max(0.0);
    if stride <= 0.0 {
        return 0;
    }
    (total_width / stride).floor().max(0.0) as usize
}

fn draw_text_node(
    img: &mut RgbaImage,
    node: &UiIrNode,
    document: &UiIrDocument,
    renderer: &TextRenderer,
    ctx: &ComposeContext<'_>,
    seen_rects: &mut HashSet<(i32, i32, i32, i32)>,
) {
    let Some(text) = resolved_text_payload(node) else {
        return;
    };
    if text.is_empty() {
        return;
    }

    let rect = ir_rect_to_layout_rect(node.computed_rect);
    if rect.w < 0.5 || rect.h < 0.5 {
        return;
    }

    let key = (
        rect.x.round() as i32,
        rect.y.round() as i32,
        rect.w.round() as i32,
        rect.h.round() as i32,
    );
    if !seen_rects.insert(key) {
        return;
    }

    let text_rects = debug_text_rects_with_renderer(node, renderer, Some(document))
        .unwrap_or(DebugTextRects {
            primary: rect,
            secondary: None,
            primary_text_origin: (rect.x, rect.y),
            secondary_text_origin: None,
        });
    let primary_rect = text_rects.primary;
    let secondary_rect = text_rects.secondary.unwrap_or(rect);
    let nominal_font_size = node
        .text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(18.0)
        .max(1.0);
    let primary_font_style_scale = font_style_scale_modifier(node.text_style.as_ref());
    let fallback_font_size =
        (nominal_font_size * primary_font_style_scale).max(1.0);
    let secondary_nominal_font_size = node
        .secondary_text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(nominal_font_size)
        .max(1.0);
    let secondary_font_style_scale = font_style_scale_modifier(
        node.secondary_text_style
            .as_ref()
            .or(node.text_style.as_ref()),
    );
    let secondary_fallback_font_size =
        (secondary_nominal_font_size * secondary_font_style_scale).max(1.0);

    let center_anchored_heading = node
        .text_style
        .as_ref()
        .is_some_and(|style| {
            node.node_type.eq_ignore_ascii_case("widget_text_field")
                && style
                    .label_style
                    .as_deref()
                    .is_some_and(|label| label.eq_ignore_ascii_case("Heading1"))
                && style
                    .anchor_to_parent_x
                    .is_some_and(|anchor| (anchor - 0.5).abs() < f32::EPSILON)
                && style
                    .anchor_to_parent_y
                    .is_some_and(|anchor| (anchor - 0.5).abs() < f32::EPSILON)
                && node.anchor[0].abs() < f32::EPSILON
                && node.anchor[1].abs() < f32::EPSILON
                && node.pivot[0].abs() < f32::EPSILON
                && node.pivot[1].abs() < f32::EPSILON
        });

    let align = if center_anchored_heading {
        TextAlign::Centre
    } else {
        node
        .text_style
        .as_ref()
        .map(|style| TextAlign::from_bb_str(&style.alignment))
        .unwrap_or(TextAlign::Left)
    };
    let vertical_align = if center_anchored_heading && node.computed_rect.h > nominal_font_size * 3.0 {
        // Anchor-centred heading whose rect is a PARENT-FILL placeholder (an
        // authored Auto-sized card laid out at parent size — many times the
        // line height): the engine auto-sizes the field, and its line box
        // (the full em above the baseline) centres on the anchor, putting
        // the cap band one descent below naive cap-centring (Clipper
        // NO TARGET measured +18.6px at em 100 vs the in-game capture).
        // A heading in an authored-sized field (e.g. the footer name card,
        // rect ≈ 1.2 em) keeps line-box centring, which coincides with
        // cap-centring for a 0.8-ascent font.
        VerticalAlign::EmBaseline
    } else if center_anchored_heading {
        VerticalAlign::Centre
    } else {
        node
        .text_style
        .as_ref()
        .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
        .unwrap_or(VerticalAlign::Centre)
    };

    let mut colour = resolved_text_colour(node, node.text_style.as_ref(), ctx);
    colour[3] = ((colour[3] as f32) * node.alpha.clamp(0.0, 1.0)).round() as u8;

    let requested_font_symbol = font_symbol_from_text_style(node.text_style.as_ref()).unwrap_or("<none>");
    let selected_font = select_imported_ui_font(ctx, node.text_style.as_ref());
    let primary_line_spacing = draw_line_spacing_for_node(node, text, node.text_style.as_ref());
    // Imported SWF glyph symbols already encode their authored shape/weight;
    // applying style scaleModifier on top of SWF sizing causes undersized text
    // on some screens (for example door status labels). Keep scaleModifier for
    // fallback TTF path only.
    let primary_ttf_font_scale = primary_font_style_scale;
    // DATA-BACKED MODEL: IR font size is the authored em-pixel size; the SWF renderer
    // maps em→raster via the font's own units_per_em (= ascent + descent). No constant.
    let mut primary_swf_font_size = nominal_font_size.max(1.0);
    let mut primary_rect = apply_font_style_vertical_offset(primary_rect, node.text_style.as_ref());
    if let Some(selection) = selected_font.as_ref() {
        // Only `autoFontSize` text is fit to its rect (the engine grows/shrinks it to
        // fill the field). Non-auto text renders at its resolved style/brand size and
        // relies on word-wrap + clipping for overflow — it is never font-scaled, so a
        // long heading or a wrapped paragraph keeps its authored size.
        if node.auto_font_size {
            primary_swf_font_size = fit_swf_font_size_to_rect(
                renderer,
                text,
                selection.font,
                primary_rect,
                primary_swf_font_size,
                align,
                vertical_align,
                primary_line_spacing,
            );
        }
    }
    // Key the optional SB_UI_FONT_DUMP audit line to this element (no-op otherwise).
    crate::text::set_font_dump_ctx(
        document.canvas_name.as_deref().unwrap_or("<unnamed>"),
        &node.name,
    );
    let used_swf_font = selected_font.as_ref().is_some_and(|selection| {
        let mut effective_vertical_align = vertical_align;
        if let Some(inline_rect) = inline_nested_textfield_text_rect(
            node,
            primary_rect,
            document,
            renderer,
            ctx,
        ) {
            primary_rect = inline_rect;
            // The nested-pair half-rect model is calibrated for cap-centring
            // (footer heading cap centre on the nav-arrow line); the GFx
            // line-box baseline placement applies only to plain anchored
            // headings.
            if effective_vertical_align == VerticalAlign::EmBaseline {
                effective_vertical_align = VerticalAlign::Centre;
            }
        }
        renderer.draw_swf_font(
            img,
            text,
            primary_rect,
            selection.font,
            ctx.assets.font_edit_text_metrics(&selection.symbol),
            primary_swf_font_size,
            colour,
            align,
            effective_vertical_align,
            primary_line_spacing,
            node.text_style
                .as_ref()
                .and_then(|style| style.letter_spacing)
                .unwrap_or(0.0),
        )
    });
    if font_telemetry_enabled() {
        if let Some(selection) = selected_font.as_ref() {
            eprintln!(
                "text-font canvas='{}' node='{}' requested='{}' selected='{}' source='{}' fallback={} swf_used={} nominal_size={:.2} swf_draw_size={:.2} ttf_draw_size={:.2} rect_h={:.2} text='{}'",
                document.canvas_name.as_deref().unwrap_or("<unnamed-canvas>"),
                node.name,
                requested_font_symbol,
                selection.symbol,
                selection.source.as_str(),
                selection.source.is_fallback(),
                used_swf_font,
                nominal_font_size,
                primary_swf_font_size,
                fallback_font_size,
                primary_rect.h,
                text
            );
        } else {
            eprintln!(
                "text-font canvas='{}' node='{}' requested='{}' selected='<none>' source='none' fallback=false swf_used=false nominal_size={:.2} swf_draw_size=0.00 ttf_draw_size={:.2} rect_h={:.2} text='{}'",
                document.canvas_name.as_deref().unwrap_or("<unnamed-canvas>"),
                node.name,
                requested_font_symbol,
                nominal_font_size,
                fallback_font_size,
                primary_rect.h,
                text
            );
        }
    }
    if !used_swf_font {
        let mut effective_vertical_align = vertical_align;
        if let Some(inline_rect) = inline_nested_textfield_text_rect(
            node,
            primary_rect,
            document,
            renderer,
            ctx,
        ) {
            primary_rect = inline_rect;
            if effective_vertical_align == VerticalAlign::EmBaseline {
                effective_vertical_align = VerticalAlign::Centre;
            }
        }
        renderer.draw(
            img,
            text,
            primary_rect,
            FontKind::Sans,
            fallback_font_size,
            colour,
            align,
            effective_vertical_align,
            scale_line_spacing(primary_line_spacing, primary_ttf_font_scale),
        );
    }

    if let Some(UiIrTextPayload::Resolved { text: secondary }) = node.secondary_text_payload.as_ref() {
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
}

fn scale_line_spacing(line_spacing: Option<f32>, font_scale: f32) -> Option<f32> {
    line_spacing.map(|spacing| spacing * font_scale)
}

pub(crate) fn draw_line_spacing_for_node(
    node: &UiIrNode,
    text: &str,
    text_style: Option<&UiIrTextStyle>,
) -> Option<f32> {
    let spacing = text_style.and_then(|style| style.line_spacing);
    let leading = font_style_leading_modifier_px(text_style);
    let spacing = match (spacing, leading) {
        (Some(spacing), 0.0) => Some(spacing),
        (Some(spacing), leading) => Some(spacing + leading),
        (None, 0.0) => None,
        (None, leading) => Some(leading),
    };
    if is_large_wrapped_title3_heading(node, text, text_style) {
        spacing.map(|value| value + 4.0)
    } else {
        spacing
    }
}

pub(crate) fn apply_font_style_vertical_offset(rect: Rect, text_style: Option<&UiIrTextStyle>) -> Rect {
    let offset = font_style_top_margin_offset_px(text_style);
    if offset.abs() <= f32::EPSILON {
        rect
    } else {
        Rect { y: rect.y + offset, ..rect }
    }
}

fn is_large_wrapped_title3_heading(
    node: &UiIrNode,
    text: &str,
    text_style: Option<&UiIrTextStyle>,
) -> bool {
    let Some(style) = text_style else {
        return false;
    };
    if style.label_style.as_deref() != Some("Title3") {
        return false;
    }
    if !matches!(VerticalAlign::from_bb_str(&style.vertical_alignment), VerticalAlign::Centre) {
        return false;
    }
    let font_size = ir_value_to_px(&style.font_size);
    font_size >= 90.0
        && node.computed_rect.h >= font_size * 2.0
        && text.split_whitespace().count() >= 3
}

pub(crate) fn resolved_text_colour(
    node: &UiIrNode,
    style: Option<&crate::ui_ir::UiIrTextStyle>,
    ctx: &ComposeContext<'_>,
) -> [u8; 4] {
    let _ = node;
    style
        .and_then(|style| style.colour)
        .or_else(|| {
            style
                .and_then(|style| style.colour_token.as_deref())
                .and_then(|token| resolve_colour_token(ctx, token))
        })
        .map(rgba_to_u8)
        .unwrap_or([255, 255, 255, 255])
}

pub fn debug_text_rects(node: &UiIrNode) -> Option<DebugTextRects> {
    let renderer = TextRenderer::new();
    debug_text_rects_with_renderer(node, &renderer, None)
}

/// Like [`debug_text_rects`], with sibling/parent context so relationship-gated
/// rect models (the nested inline heading pair) resolve exactly as the render
/// path does.
pub fn debug_text_rects_in_document(
    node: &UiIrNode,
    document: &UiIrDocument,
) -> Option<DebugTextRects> {
    let renderer = TextRenderer::new();
    debug_text_rects_with_renderer(node, &renderer, Some(document))
}

pub fn debug_text_drawn_bounds(node: &UiIrNode) -> Option<DebugTextDrawnBounds> {
    let renderer = TextRenderer::new();
    debug_text_drawn_bounds_with_renderer(node, &renderer)
}

fn debug_text_rects_with_renderer(
    node: &UiIrNode,
    renderer: &TextRenderer,
    document: Option<&UiIrDocument>,
) -> Option<DebugTextRects> {
    let text = resolved_text_payload(node)?;
    let rect = ir_rect_to_layout_rect(node.computed_rect);
    if rect.w < 0.5 || rect.h < 0.5 {
        return None;
    }

    let nominal_font_size = node
        .text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(18.0)
        .max(1.0);
    let fallback_font_size = nominal_font_size.max(1.0);
    let is_label_caption_pair = node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair");
    if is_label_caption_pair && node.secondary_text_payload.is_some() {
        let secondary_nominal_font_size = node
            .secondary_text_style
            .as_ref()
            .map(|style| ir_value_to_px(&style.font_size))
            .unwrap_or(nominal_font_size)
            .max(1.0);
        let secondary_fallback_font_size =
            secondary_nominal_font_size.max(1.0);
        let (mut primary, mut secondary) = stacked_label_caption_pair_text_rects(
            rect,
            renderer.measure(text, FontKind::Sans, fallback_font_size).1,
            node.secondary_text_payload
                .as_ref()
                .and_then(|payload| match payload {
                    UiIrTextPayload::Resolved { text } => Some(text.as_str()),
                    UiIrTextPayload::UnresolvedKey { .. }
                    | UiIrTextPayload::IntentionallyEmpty { .. }
                    | UiIrTextPayload::Empty => None,
                })
                .map(|secondary| renderer.measure(secondary, FontKind::Sans, secondary_fallback_font_size).1)
                .unwrap_or(secondary_fallback_font_size),
            node.text_style.as_ref().and_then(|style| style.anchor_to_parent_y),
            node.anchor[0] >= 0.99 && node.pivot[0] >= 0.99,
        );
        let (pair_offset_x, pair_offset_y) = right_anchored_label_caption_pair_offset(
            node,
            primary.h,
            Some(secondary.h),
        );
        primary.x += pair_offset_x;
        primary.y += pair_offset_y;
        secondary.x += pair_offset_x;
        secondary.y += pair_offset_y;
        let primary_align = node
            .text_style
            .as_ref()
            .map(|style| TextAlign::from_bb_str(&style.alignment))
            .unwrap_or(TextAlign::Left);
        let primary_vertical = node
            .text_style
            .as_ref()
            .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
            .unwrap_or(VerticalAlign::Centre);
        let secondary_text = node.secondary_text_payload.as_ref().and_then(|payload| match payload {
            UiIrTextPayload::Resolved { text } => Some(text.as_str()),
            UiIrTextPayload::UnresolvedKey { .. }
            | UiIrTextPayload::IntentionallyEmpty { .. }
            | UiIrTextPayload::Empty => None,
        });
        let primary_text_origin = text_origin_in_rect(
            renderer,
            text,
            primary,
            FontKind::Sans,
            fallback_font_size,
            primary_align,
            primary_vertical,
        );
        let secondary_text_origin = secondary_text.map(|secondary_text| {
            text_origin_in_rect(
                renderer,
                secondary_text,
                secondary,
                FontKind::Sans,
                secondary_fallback_font_size,
                TextAlign::Left,
                VerticalAlign::Centre,
            )
        });
        Some(DebugTextRects {
            primary,
            secondary: Some(secondary),
            primary_text_origin,
            secondary_text_origin,
        })
    } else {
        let align = node
            .text_style
            .as_ref()
            .map(|style| TextAlign::from_bb_str(&style.alignment))
            .unwrap_or(TextAlign::Left);
        let vertical = node
            .text_style
            .as_ref()
            .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
            .unwrap_or(VerticalAlign::Centre);
        let primary_rect =
            center_anchored_heading_textfield_text_rect(node, rect, document).unwrap_or(rect);
        Some(DebugTextRects {
            primary: primary_rect,
            secondary: None,
            primary_text_origin: text_origin_in_rect(renderer, text, primary_rect, FontKind::Sans, fallback_font_size, align, vertical),
            secondary_text_origin: None,
        })
    }
}

fn center_anchored_heading_textfield_text_rect(
    node: &UiIrNode,
    rect: Rect,
    document: Option<&UiIrDocument>,
) -> Option<Rect> {
    let style = node.text_style.as_ref()?;
    if !node.node_type.eq_ignore_ascii_case("widget_text_field") {
        return None;
    }
    // The anchored half-rect model belongs to the nested inline heading pair
    // (the medical header: a Heading1 parent text field with a same-style
    // text-field child sharing its band — both render in the band's lower
    // half). A lone centre-anchored heading (the MFD footer's screen name)
    // centres its glyph block ON the anchor line instead — verified against the
    // Clipper target/power footer captures (cap centre == card midline).
    if let Some(document) = document {
        let has_same_style_text_child = document.nodes.iter().any(|candidate| {
            candidate.parent_id == Some(node.id)
                && candidate.node_type.eq_ignore_ascii_case("widget_text_field")
                && candidate
                    .text_style
                    .as_ref()
                    .is_some_and(|child| same_label_style(child, style))
        });
        if !has_same_style_text_child {
            return None;
        }
    }
    if !style
        .label_style
        .as_deref()
        .is_some_and(|label| label.eq_ignore_ascii_case("Heading1"))
    {
        return None;
    }
    if !style.vertical_alignment.eq_ignore_ascii_case("Center") {
        return None;
    }
    let anchor_to_parent_y = style.anchor_to_parent_y?;
    if (anchor_to_parent_y - 0.5).abs() > f32::EPSILON || node.pivot[1].abs() > f32::EPSILON {
        return None;
    }
    if node.anchor[1] > 0.0 && !(node.anchor[0] > 1.0 && node.pivot[0] >= 0.99) {
        return None;
    }

    let top = rect.y + rect.h * anchor_to_parent_y;
    let height = (rect.h * (1.0 - anchor_to_parent_y)).max(1.0);
    Some(Rect { y: top, h: height, ..rect })
}

pub(crate) fn inline_nested_textfield_text_rect(
    node: &UiIrNode,
    rect: Rect,
    document: &UiIrDocument,
    renderer: &TextRenderer,
    ctx: &ComposeContext<'_>,
) -> Option<Rect> {
    let style = node.text_style.as_ref()?;
    if !node.node_type.eq_ignore_ascii_case("widget_text_field")
        || node.pivot[0] < 0.99
        || node.anchor[0] <= 1.0
        || !style.vertical_alignment.eq_ignore_ascii_case("Center")
    {
        return None;
    }

    let parent_id = node.parent_id?;
    let parent = document.nodes.iter().find(|candidate| candidate.id == parent_id)?;
    let parent_style = parent.text_style.as_ref()?;
    if !parent.node_type.eq_ignore_ascii_case("widget_text_field")
        || !same_label_style(style, parent_style)
        || !parent_style.vertical_alignment.eq_ignore_ascii_case(&style.vertical_alignment)
    {
        return None;
    }

    let parent_text = resolved_text_payload(parent)?;
    let parent_rect = center_anchored_heading_textfield_text_rect(
        parent,
        ir_rect_to_layout_rect(parent.computed_rect),
        Some(document),
    )
    .unwrap_or_else(|| ir_rect_to_layout_rect(parent.computed_rect));
    let parent_nominal_size = parent_style_font_size(parent_style);
    let parent_font_style_scale = font_style_scale_modifier(Some(parent_style));
    // DRAW-METRIC model (plan P3.3+P3.4, 2026-06-13): the child continues the
    // parent's inline run, so its origin is the parent's advance end at the
    // parent's ACTUAL draw size — the glyphs' own side bearings supply the
    // visible separation (the medical1 reference shows the "3"→"M" ink gap at
    // letter-gap scale, ~3px, NOT a typeset word space). This retires the two
    // tuned constants that previously composed the same net offset
    // (`SWF_TEXT_RENDER_SIZE_CALIBRATION = 0.84` on the width and the
    // `INLINE_NESTED_TEXTFIELD_WORD_GAP = 0.33` word gap): ink-level
    // measurement of render vs capture put the tuned pair within ~1px of the
    // reference, and the advance-at-draw-size model reproduces the same
    // position from the font data alone.
    let parent_draw_size = (parent_nominal_size * parent_font_style_scale).max(1.0);
    let parent_width = select_imported_ui_font(ctx, Some(parent_style))
        .and_then(|selection| {
            renderer.measure_swf_advance_width(
                parent_text,
                selection.font,
                parent_draw_size,
                parent_style.letter_spacing.unwrap_or(0.0),
            )
        })
        .unwrap_or_else(|| {
            renderer.measure(parent_text, FontKind::Sans, parent_draw_size).0
        });
    let inline_x = parent_rect.x + parent_width;
    if !inline_x.is_finite() || inline_x >= rect.x || inline_x <= parent_rect.x {
        return None;
    }

    let right = rect.x + rect.w;
    Some(Rect {
        x: inline_x,
        w: (right - inline_x).max(1.0),
        ..rect
    })
}

fn same_label_style(left: &UiIrTextStyle, right: &UiIrTextStyle) -> bool {
    match (left.label_style.as_deref(), right.label_style.as_deref()) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

fn parent_style_font_size(style: &UiIrTextStyle) -> f32 {
    ir_value_to_px(&style.font_size).max(1.0)
}

fn debug_text_drawn_bounds_with_renderer(
    node: &UiIrNode,
    renderer: &TextRenderer,
) -> Option<DebugTextDrawnBounds> {
    let text = resolved_text_payload(node)?;
    let rects = debug_text_rects_with_renderer(node, renderer, None)?;

    let nominal_font_size = node
        .text_style
        .as_ref()
        .map(|style| ir_value_to_px(&style.font_size))
        .unwrap_or(18.0)
        .max(1.0);
    let fallback_font_size = nominal_font_size.max(1.0);
    let primary_align = node
        .text_style
        .as_ref()
        .map(|style| TextAlign::from_bb_str(&style.alignment))
        .unwrap_or(TextAlign::Left);
    let primary_vertical = node
        .text_style
        .as_ref()
        .map(|style| VerticalAlign::from_bb_str(&style.vertical_alignment))
        .unwrap_or(VerticalAlign::Centre);

    let primary = renderer.measure_drawn_bounds(
        text,
        apply_font_style_vertical_offset(rects.primary, node.text_style.as_ref()),
        FontKind::Sans,
        fallback_font_size,
        primary_align,
        primary_vertical,
        draw_line_spacing_for_node(node, text, node.text_style.as_ref()),
    )?;

    let secondary = if let Some(UiIrTextPayload::Resolved { text: secondary_text }) = node.secondary_text_payload.as_ref() {
        let secondary_nominal_font_size = node
            .secondary_text_style
            .as_ref()
            .map(|style| ir_value_to_px(&style.font_size))
            .unwrap_or(nominal_font_size)
            .max(1.0);
        let secondary_fallback_font_size =
            secondary_nominal_font_size.max(1.0);
        rects.secondary.and_then(|secondary_rect| {
            renderer.measure_drawn_bounds(
                secondary_text,
                apply_font_style_vertical_offset(
                    secondary_rect,
                    node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
                ),
                FontKind::Sans,
                secondary_fallback_font_size,
                TextAlign::Left,
                VerticalAlign::Centre,
                draw_line_spacing_for_node(
                    node,
                    secondary_text,
                    node.secondary_text_style.as_ref().or(node.text_style.as_ref()),
                ),
            )
        })
    } else {
        None
    };

    Some(DebugTextDrawnBounds { primary, secondary })
}

fn text_origin_in_rect(
    renderer: &TextRenderer,
    text: &str,
    rect: Rect,
    kind: FontKind,
    size_px: f32,
    align: TextAlign,
    vertical_align: VerticalAlign,
) -> (f32, f32) {
    let (text_w, text_h) = renderer.measure(text, kind, size_px.max(1.0));
    let x = match align {
        TextAlign::Left => rect.x,
        TextAlign::Centre => rect.x + ((rect.w - text_w) * 0.5).max(0.0),
        TextAlign::Right => rect.x + (rect.w - text_w).max(0.0),
    };
    let y = match vertical_align {
        VerticalAlign::Top => rect.y,
        VerticalAlign::Centre => rect.y + ((rect.h - text_h) * 0.5).max(0.0),
        VerticalAlign::Bottom => rect.y + (rect.h - text_h).max(0.0),
        // Baseline at rect centre + em/2 → block top one half-em below centre
        // minus the block height (see `VerticalAlign::EmBaseline`).
        VerticalAlign::EmBaseline => rect.y + rect.h * 0.5 + size_px * 0.5 - text_h,
    };
    (x, y)
}

pub(crate) fn stacked_label_caption_pair_text_rects(
    rect: Rect,
    primary_text_h: f32,
    secondary_text_h: f32,
    primary_anchor_y: Option<f32>,
    right_anchored_pair: bool,
) -> (Rect, Rect) {
    let primary_h = primary_text_h.max(1.0).min(rect.h.max(1.0));
    let secondary_h = secondary_text_h.max(1.0).min(rect.h.max(1.0));
    let total_h = primary_h + secondary_h;
    let max_top_padding = (rect.h - total_h).max(0.0);
    let anchor_y = primary_anchor_y.unwrap_or(0.0).clamp(0.0, 0.999);
    let derived_top_padding = if anchor_y > 0.0 {
        ((anchor_y * (primary_h + secondary_h)) - (primary_h * 0.5)) / (1.0 - anchor_y)
    } else {
        0.0
    };
    let top_padding = if right_anchored_pair {
        // Registered pin (crates/starbreaker-ui/docs/ui-fallback-register.md): the right-anchored
        // pair's COMPONENT RECT is misplaced by the med2 multi-canvas slot
        // composition (the eob MEDGELS rect computes y=-15.2 while the meter
        // anchored to its bottom lands ~35px above the capture's meter), so
        // the anchor-derived padding is correct only relative to a rect this
        // arc cannot fix. Reproduce the pre-P3.4 reference-verified placement
        // exactly (eob label cap-top 36 vs capture 34): the retired
        // TTF-inflated band heights entered this arm twice — ×1.5 on the
        // anchor-derived padding and (1.5−1)/2 = 0.25 of the label em as the
        // in-band glyph-centring offset. Em-height padding alone sat 20px
        // high. Retire with the med2 slot-composition fix.
        (derived_top_padding * 1.5 + primary_h * 0.25).max(0.0)
    } else {
        derived_top_padding.clamp(0.0, max_top_padding)
    };
    let primary_y = rect.y + top_padding;
    // PURE LINE-BOX stacking (plan P3.4, 2026-06-13): the caption-pair
    // component stacks label and value in a flex Column with rowSpacing 0,
    // and the engine's line box IS the em box (line advance == font size —
    // the paragraph model verified on the medical card descriptions), so the
    // value's line top sits exactly one label-em below the label's line top.
    // With the retired 1.5 TTF inflation gone, `primary_h` here IS the
    // label's em height, making the stack `primary_y + primary_h` with no
    // overlap subtraction and no tuned spacing. Verified against the
    // medical1 capture with ui_measure.py: MEDGELS cap-top 38, 200/200
    // cap-top 67 → top-to-top 29px; the model predicts label em 30 minus
    // ~1px cap-offset differential between the 30px/25px styles = 29. (The
    // retired pair `line_box_overlap` + `LABEL_CAPTION_PAIR_FLEX_ROW_SPACING
    // = -8.0` reproduced the same 28-29px only at the old inflated heights:
    // 45 - 7 - 8 ≈ 30.)
    let secondary_y = (primary_y + primary_h).max(rect.y);
    (
        Rect {
            x: rect.x,
            y: primary_y,
            w: rect.w,
            h: primary_h,
        },
        Rect {
            x: rect.x,
            y: secondary_y,
            w: rect.w,
            h: secondary_h,
        },
    )
}

fn right_anchored_label_caption_pair_offset(
    node: &UiIrNode,
    primary_text_h: f32,
    secondary_text_h: Option<f32>,
) -> (f32, f32) {
    if !node
        .node_type
        .eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        || node.secondary_text_payload.is_none()
        || node.anchor[0] < 0.99
        || node.pivot[0] < 0.99
    {
        return (0.0, 0.0);
    }

    let primary_h = primary_text_h.max(0.0);
    let secondary_h = secondary_text_h.unwrap_or(primary_h).max(0.0);
    let line_box_delta = (primary_h - secondary_h).max(0.0);
    let stroke_pair_span = node.stroke_extent.unwrap_or(0.0).max(0.0) * 2.0;
    // EXPERIMENT: zero the vertical offset (keep horizontal). The Y offset pushed
    // the right-anchored MEDGELS label down a stretched header band.
    (-(line_box_delta + stroke_pair_span), 0.0)
}

fn font_telemetry_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SB_UI_FONT_TELEMETRY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
            .unwrap_or(false)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontSelectionSource {
    ResolvedRecordSymbol,
    Title3ExportFallback,
    PreferredExportFallback,
    PreferredNameFallback,
}

impl FontSelectionSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::ResolvedRecordSymbol => "resolved-record-symbol",
            Self::Title3ExportFallback => "title3-export-fallback",
            Self::PreferredExportFallback => "preferred-export-fallback",
            Self::PreferredNameFallback => "preferred-name-fallback",
        }
    }

    fn is_fallback(self) -> bool {
        !matches!(self, Self::ResolvedRecordSymbol)
    }
}

pub(crate) struct SelectedImportedFont<'a> {
    pub(crate) symbol: String,
    pub(crate) font: &'a FontGlyphSet,
    source: FontSelectionSource,
}

fn select_imported_ui_font<'a>(
    ctx: &'a ComposeContext<'_>,
    text_style: Option<&UiIrTextStyle>,
) -> Option<SelectedImportedFont<'a>> {
    select_imported_ui_font_from_assets(
        ctx.assets,
        font_symbol_from_text_style(text_style),
        text_style.and_then(|style| style.label_style.as_deref()),
    )
}

/// Assets-level draw font selection — the single source of truth for which
/// imported SWF font a text element renders with. Shared by the compose-time
/// draw (via [`select_imported_ui_font`]) and the pipeline's pre-layout
/// [`crate::ui_ir::DrawTextMeasure`] so intrinsic text measurement selects
/// EXACTLY the font the draw will use (measure == draw by construction).
pub(crate) fn select_imported_ui_font_from_assets<'a>(
    assets: &'a SwfAssetLibrary,
    font_symbol: Option<&str>,
    label_style: Option<&str>,
) -> Option<SelectedImportedFont<'a>> {
    if let Some(symbol) = font_symbol
        && let Some(id) = assets.lookup_export(symbol)
        && let Some(font) = assets.get_font(id)
    {
        return Some(SelectedImportedFont {
            symbol: symbol.to_string(),
            font,
            source: FontSelectionSource::ResolvedRecordSymbol,
        });
    }

    if matches!(label_style, Some("Title3")) {
        let mut text1_fonts: Vec<(String, &'a FontGlyphSet)> = assets
            .export_entries()
            .filter(|(symbol, _)| symbol.starts_with("$Text1"))
            .filter_map(|(symbol, id)| assets.get_font(id).map(|font| (symbol.to_string(), font)))
            .collect();

        text1_fonts.sort_by(|(left_name, left_font), (right_name, right_font)| {
            title3_font_weight_rank(left_name, left_font)
                .cmp(&title3_font_weight_rank(right_name, right_font))
                .then_with(|| left_name.cmp(right_name))
        });

        if let Some((symbol, font)) = text1_fonts.into_iter().next() {
            return Some(SelectedImportedFont {
                symbol,
                font,
                source: FontSelectionSource::Title3ExportFallback,
            });
        }
    }

    let preferred_symbols: &[&str] = &["$Text1Book", "$Text1Med", "$OutfitRegular", "$Text1Bold", "$CIGDrake"];

    for symbol in preferred_symbols {
        if let Some(id) = assets.lookup_export(symbol)
            && let Some(font) = assets.get_font(id)
        {
            return Some(SelectedImportedFont {
                symbol: symbol.to_string(),
                font,
                source: FontSelectionSource::PreferredExportFallback,
            });
        }
    }

    let preferred_font_names: &[(&str, &str)] = match label_style {
        Some("Title3") => &[
            ("Blender Pro Light", "Blender Pro Light"),
            ("Blender Pro Regular", "Blender Pro Regular"),
            ("Blender Pro Thin", "Blender Pro Thin"),
            ("Blender Pro Book", "Blender Pro Book"),
            ("Blender Pro Medium", "Blender Pro Medium"),
            ("CIG Drake Font", "CIGDrake"),
        ],
        _ => &[
            ("Blender Pro Book", "Blender Pro Book"),
            ("Blender Pro Medium", "Blender Pro Medium"),
            ("Outfit", "Outfit"),
            ("Open Sans", "Open Sans"),
            ("CIG Drake Font", "CIGDrake"),
        ],
    };
    for (query, label) in preferred_font_names {
        if let Some(font) = assets.find_font_by_name(query) {
            return Some(SelectedImportedFont {
                symbol: label.to_string(),
                font,
                source: FontSelectionSource::PreferredNameFallback,
            });
        }
    }
    None
}

fn resolved_font_record_value(style: Option<&UiIrTextStyle>) -> Option<&serde_json::Value> {
    let record = style?.resolved_font_record.as_ref()?;
    Some(record.get("_RecordValue_").unwrap_or(record))
}

pub(crate) fn font_symbol_from_text_style(style: Option<&UiIrTextStyle>) -> Option<&str> {
    resolved_font_record_value(style)
        .and_then(|value| value.get("font"))
        .and_then(|value| value.as_str())
        .filter(|symbol| !symbol.is_empty())
}

/// Fit a SWF text size to its container rect using *real glyph metrics*.
///
/// The constraint the engine enforces is that the **visible glyph box** (cap /
/// ascender-to-descender extent of the actual characters — not the full em line-box)
/// fits the rect, in both axes:
///   * height: the per-line glyph height must fit `rect.h / line_count`;
///   * width: a single line's advance width must fit `rect.w` (wrapped paragraphs
///     are width-bounded by the wrap, so width-fit is single-line only).
///
/// This is only invoked for `autoFontSize` nodes, so it always *fills* the rect — the
/// size becomes the largest that still fits both axes. Because every metric is linear
/// in the size, the result is independent of `requested_size`'s magnitude (it only
/// seeds the measurement). Non-`autoFontSize` text is never passed here; it renders at
/// its resolved style size and relies on word-wrap/clipping for overflow.
fn fit_swf_font_size_to_rect(
    renderer: &TextRenderer,
    text: &str,
    font: &FontGlyphSet,
    rect: Rect,
    requested_size: f32,
    _align: TextAlign,
    _vertical_align: VerticalAlign,
    _line_spacing_px: Option<f32>,
) -> f32 {
    if text.trim().is_empty() || requested_size <= 1.0 || rect.w <= 1.0 || rect.h <= 1.0 {
        return requested_size.max(1.0);
    }

    let line_count = text.lines().filter(|line| !line.trim().is_empty()).count().max(1);
    let single_line = line_count <= 1;

    // Height: the text's line-box (one font em = the rendered `size_px`) must fit
    // `rect.h / line_count`. This is the engine's "a line of text fits the field"
    // rule: the glyphs sit within their em-box, which the field bounds. Using the em
    // (not just the cap extent) keeps short labels from ballooning until their width
    // happens to bind, so sibling labels in equal-height cells size uniformly.
    let line_box = swf_line_box_px(font, requested_size);
    let per_line_limit = rect.h / line_count as f32;
    let size_for_height = if line_box > 0.0 {
        requested_size * per_line_limit / line_box
    } else {
        requested_size
    };

    // Width: largest size whose single-line advance fits rect.w. Multi-line text is
    // already width-bounded by wrapping, so it imposes no extra width constraint — it
    // only acts as a safety bound for an over-long single line.
    let size_for_width = if single_line {
        match renderer
            .measure_swf_advance_width(text, font, requested_size, 0.0)
            .filter(|value| *value > 0.0)
        {
            Some(advance) => requested_size * rect.w / advance,
            None => f32::INFINITY,
        }
    } else {
        f32::INFINITY
    };

    size_for_height.min(size_for_width).max(1.0)
}

/// The rendered line-box height (px) of one text line at `size_px`: the font's full
/// em (`ascent + |descent|`) plus its leading, scaled to `size_px`. This is the
/// vertical space one line of this font occupies — what an `autoFontSize` field sizes
/// against so the line fits its rect.
pub(crate) fn swf_line_box_px(font: &FontGlyphSet, size_px: f32) -> f32 {
    let ascent = font.ascent.map(|value| value as f32).unwrap_or(820.0);
    let descent = font.descent.map(|value| value as f32).unwrap_or(-204.0);
    let leading = font.leading.map(|value| value as f32).unwrap_or(0.0);
    let units_per_em = (ascent.abs() + descent.abs()).max(1.0);
    ((units_per_em + leading) / units_per_em) * size_px
}

pub(crate) fn font_style_scale_modifier(style: Option<&UiIrTextStyle>) -> f32 {
    resolved_font_record_value(style)
        .and_then(|value| value.get("scaleModifier"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(1.0)
}

pub(crate) fn font_style_leading_modifier_px(style: Option<&UiIrTextStyle>) -> f32 {
    let modifier = resolved_font_record_value(style)
        .and_then(|value| value.get("leadingModifier"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(0.0);
    let size_px = style.map(|style| ir_value_to_px(&style.font_size)).unwrap_or(0.0);
    modifier * size_px
}

pub(crate) fn font_style_top_margin_offset_px(style: Option<&UiIrTextStyle>) -> f32 {
    let modifier = resolved_font_record_value(style)
        .and_then(|value| value.get("topMarginModifier"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(0.0);
    let size_px = style.map(|style| ir_value_to_px(&style.font_size)).unwrap_or(0.0);
    modifier * size_px
}

fn title3_font_weight_rank(symbol: &str, font: &FontGlyphSet) -> i32 {
    let symbol_lower = symbol.to_ascii_lowercase();
    let name_lower = font.name.to_ascii_lowercase();
    let combined = format!("{} {}", symbol_lower, name_lower);
    let name_rank = if combined.contains("thin") {
        0
    } else if combined.contains("light") {
        1
    } else if combined.contains("book") {
        2
    } else if combined.contains("regular") {
        3
    } else if combined.contains("med") || combined.contains("medium") {
        4
    } else if combined.contains("bold") {
        6
    } else {
        5
    };
    name_rank + if font.is_bold { 10 } else { 0 }
}

fn resolved_text_payload(node: &UiIrNode) -> Option<&str> {
    let payload = node.text_payload.as_ref()?;
    match payload {
        UiIrTextPayload::Resolved { text } => Some(text.as_str()),
        UiIrTextPayload::Empty
        | UiIrTextPayload::IntentionallyEmpty { .. }
        | UiIrTextPayload::UnresolvedKey { .. } => None,
    }
}

pub(crate) fn draw_ir_border(
    pixmap: &mut Pixmap,
    rect: Rect,
    border: &UiIrBorder,
    alpha: f32,
    ctx: &ComposeContext<'_>,
    corner_radius: Option<f32>,
) {
    if let Some(radius) = corner_radius.filter(|r| *r > 0.0)
        && draw_rounded_uniform_border(pixmap, rect, border, radius, alpha, ctx)
    {
        return;
    }

    let top_colour = border_side_colour(&border.top, ctx);
    draw_border_side(
        pixmap,
        Rect { x: rect.x, y: rect.y, w: rect.w, h: border.top.width },
        top_colour,
        alpha,
    );

    let right_colour = border_side_colour(&border.right, ctx);
    draw_border_side(
        pixmap,
        Rect {
            x: rect.x + rect.w - border.right.width,
            y: rect.y,
            w: border.right.width,
            h: rect.h,
        },
        right_colour,
        alpha,
    );

    let bottom_colour = border_side_colour(&border.bottom, ctx);
    draw_border_side(
        pixmap,
        Rect {
            x: rect.x,
            y: rect.y + rect.h - border.bottom.width,
            w: rect.w,
            h: border.bottom.width,
        },
        bottom_colour,
        alpha,
    );

    let left_colour = border_side_colour(&border.left, ctx);
    draw_border_side(
        pixmap,
        Rect { x: rect.x, y: rect.y, w: border.left.width, h: rect.h },
        left_colour,
        alpha,
    );
}

pub(crate) fn border_side_colour(side: &crate::ui_ir::UiIrBorderSide, ctx: &ComposeContext<'_>) -> Option<[f32; 4]> {
    side.colour.or_else(|| {
        side.colour_token
            .as_deref()
            .and_then(|token| resolve_surface_colour_token(ctx, token))
    })
}

fn draw_border_side(pixmap: &mut Pixmap, rect: Rect, colour: Option<[f32; 4]>, alpha: f32) {
    let Some(colour) = colour else {
        return;
    };
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let Some(tsk_rect) = TskRect::from_xywh(rect.x, rect.y, rect.w, rect.h) else {
        return;
    };
    fill_rect_ts(pixmap, tsk_rect, colour, alpha);
}

fn draw_widget_circle(ctx: &ComposeContext<'_>, node: &UiIrNode, pixmap: &mut Pixmap, rect: TskRect) {
    // Solid fill (`doFill` circles — the g-force ball's `circle_Cap*` dots) from
    // the `fillColor` surface token; the outline stroke (rings) is unchanged.
    let fill_colour = node
        .circle_fill_colour_token
        .as_deref()
        .and_then(|token| resolve_surface_colour_token(ctx, token))
        .filter(|colour| colour[3] > 0.005);
    let stroke_colour = node.stroke_colour.or(node.background_fill_colour);
    if fill_colour.is_none() && stroke_colour.is_none() {
        return;
    }

    let cx = rect.x() + rect.width() * 0.5;
    let cy = rect.y() + rect.height() * 0.5;
    let radius = rect.width().min(rect.height()) * 0.5;
    if radius <= 0.5 {
        return;
    }

    if let Some(fill) = fill_colour {
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, radius);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(to_skia_color(fill, node.alpha));
            paint.anti_alias = true;
            pixmap.as_mut().fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    if let Some(stroke_colour) = stroke_colour {
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, radius - 0.5);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(to_skia_color(stroke_colour, node.alpha));
            paint.anti_alias = true;
            let mut stroke = Stroke::default();
            stroke.width = node.stroke_extent.unwrap_or(1.5).max(0.5);
            pixmap
                .as_mut()
                .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

fn draw_widget_separator(
    ctx: &ComposeContext<'_>,
    pixmap: &mut Pixmap,
    node: &UiIrNode,
    rect: TskRect,
    alpha: f32,
) {
    let draw_rect = widget_separator_draw_rect(rect, node.stroke_extent);
    let colour = node
        .stroke_colour
        .or_else(|| {
            node.stroke_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        })
        .or(node.background_fill_colour)
        .or_else(|| {
            node.background_fill_colour_token
                .as_deref()
                .and_then(|token| resolve_surface_colour_token(ctx, token))
        });
    let Some(colour) = colour else {
        return;
    };
    fill_rect_ts_with_mode(pixmap, draw_rect, colour, alpha, node_colour_blend_mode(node));
}

fn node_colour_blend_mode(node: &UiIrNode) -> BlendMode {
    match node.colour_blend_mode {
        Some(UiIrColourBlendMode::Additive) => BlendMode::Plus,
        Some(UiIrColourBlendMode::SourceOver) | None => BlendMode::SourceOver,
    }
}

pub(crate) fn widget_separator_draw_rect(rect: TskRect, stroke_extent: Option<f32>) -> TskRect {
    let Some(stroke_extent) = stroke_extent else {
        return rect;
    };
    let thickness = (stroke_extent.max(0.5) * 2.0).min(rect.height()).max(1.0);
    TskRect::from_xywh(
        rect.x(),
        rect.y() + (rect.height() - thickness) * 0.5,
        rect.width(),
        thickness,
    )
    .unwrap_or(rect)
}

fn draw_rect_stroke_ts(
    pixmap: &mut Pixmap,
    rect: TskRect,
    rgba: [f32; 4],
    width: f32,
    alpha: f32,
) {
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    let Some(path) = pb.finish() else {
        return;
    };

    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.anti_alias = false;

    let mut stroke = Stroke::default();
    stroke.width = width.max(0.5);
    pixmap
        .as_mut()
        .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn fill_rect_ts(pixmap: &mut Pixmap, rect: TskRect, rgba: [f32; 4], alpha: f32) {
    fill_rect_ts_with_mode(pixmap, rect, rgba, alpha, BlendMode::SourceOver);
}

/// Fill a `corner_radius`-rounded rect (the radius is clamped to half the box,
/// so a large radius on a small square yields a circle — the velocity / g-force
/// ball centre dot). Falls back to a plain rect fill when the path degenerates.
fn fill_rounded_rect_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    radius: f32,
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let Some(path) = rounded_rect_path(rect, radius) else {
        fill_rect_ts_with_mode(pixmap, rect, rgba, alpha, blend_mode);
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.blend_mode = blend_mode;
    paint.anti_alias = true;
    pixmap.as_mut().fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn fill_rect_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let mut paint = Paint::default();
    paint.set_color(to_skia_color(rgba, alpha));
    paint.blend_mode = blend_mode;
    paint.anti_alias = false;
    pixmap
        .as_mut()
        .fill_rect(rect, &paint, Transform::identity(), None);
}

fn blit_atlas_image_tinted(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
) {
    blit_atlas_image_tinted_with_mode(
        pixmap,
        img,
        dx,
        dy,
        tint,
        alpha,
        BlendMode::SourceOver,
    );
}

fn blit_atlas_image_tinted_with_mode(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let w = img.width();
    let h = img.height();

    let mut premul: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    for chunk in img.as_raw().chunks_exact(4) {
        let r = chunk[0] as f32 / 255.0 * tint[0];
        let g = chunk[1] as f32 / 255.0 * tint[1];
        let b = chunk[2] as f32 / 255.0 * tint[2];
        let a = chunk[3] as f32 / 255.0 * tint[3];
        premul.push((r * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((g * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((b * a * 255.0).clamp(0.0, 255.0) as u8);
        premul.push((a * 255.0).clamp(0.0, 255.0) as u8);
    }

    let Some(size) = tiny_skia::IntSize::from_wh(w, h) else {
        return;
    };
    let Some(src_pixmap) = Pixmap::from_vec(premul, size) else {
        return;
    };

    let mut paint = PixmapPaint::default();
    paint.opacity = alpha.clamp(0.0, 1.0);
    paint.blend_mode = blend_mode;
    pixmap
        .as_mut()
        .draw_pixmap(dx, dy, src_pixmap.as_ref(), &paint, Transform::identity(), None);
}

fn pixmap_to_rgba_image(pixmap: Pixmap) -> Result<RgbaImage, UiError> {
    let w = pixmap.width();
    let h = pixmap.height();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for chunk in pixmap.data().chunks_exact(4) {
        let a = chunk[3] as f32 / 255.0;
        if a <= 0.0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        out.push(((chunk[0] as f32 / a).clamp(0.0, 255.0)) as u8);
        out.push(((chunk[1] as f32 / a).clamp(0.0, 255.0)) as u8);
        out.push(((chunk[2] as f32 / a).clamp(0.0, 255.0)) as u8);
        out.push(chunk[3]);
    }
    RgbaImage::from_raw(w, h, out)
        .ok_or_else(|| UiError::RenderError("failed to build image from pixmap".into()))
}

pub(crate) fn to_skia_color(rgba: [f32; 4], global_alpha: f32) -> Color {
    let a = (rgba[3] * global_alpha).clamp(0.0, 1.0);
    Color::from_rgba8(
        (rgba[0].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0) as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0) as u8,
        (a * 255.0) as u8,
    )
}

#[cfg(test)]
pub(crate) fn style_primary_rgba(ctx: &ComposeContext<'_>) -> [f32; 4] {
    let pt = &ctx.style.primary_tint;
    [
        pt.r as f32 / 255.0,
        pt.g as f32 / 255.0,
        pt.b as f32 / 255.0,
        pt.a as f32 / 255.0,
    ]
}

pub(crate) fn rgba_to_u8(rgba: [f32; 4]) -> [u8; 4] {
    [
        (rgba[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub(crate) fn ir_rect_to_layout_rect(rect: UiIrRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

pub(crate) fn ir_value_to_px(value: &UiIrValue) -> f32 {
    match value {
        UiIrValue::Fixed { value } | UiIrValue::Percent { value } | UiIrValue::Other { value, .. } => *value,
    }
}
