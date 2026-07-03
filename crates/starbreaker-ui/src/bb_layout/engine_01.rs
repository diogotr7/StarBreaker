#[allow(unused_imports)]
use super::*;
// Consolidated engine chunk 01 (formerly: part_01.part, part_02.part, part_03.part, part_04.part, part_05.part, part_06.part, part_07.part, part_08.part).
//   part_01.part: BuildingBlocks layout engine — pixel-space rect resolver.

// BuildingBlocks layout engine — pixel-space rect resolver.
//
// Turns a merged [`BbScene`] (produced by [`crate::bb_resolve`]) into a
// [`LayoutResult`] that maps every [`BbNodeId`] to a [`Rect`] in screen-pixel
// coordinates, plus a deterministic DFS draw order.
//
// # Coordinate system
// Screen-space, +x right, +y down, units = pixels.  The BB authoring system
// also uses +x right, +y down (verified: in the Clipper reference screenshots
// a node at `position.y = 100` canvas units appears below the top edge).
//
// # Percent sizing
// `BbValue::Percent(p)` stores the raw `value` from the DataCore JSON.  In
// every tested fixture `1.0` represents 100 % of the parent dimension (i.e.
// the value is already a fraction, **not** a 0–100 percentage).  The task
// specification stated "0–100" but fixture inspection (`MC_S_Target_Master`,
// `BB_ScreenRadar`, etc.) shows `value: 1` for "fill parent" and `value: 0.08`
// for an 8 % column.  We therefore compute `parent_inner.w * p` directly.
//
// # Margin simplification (Phase A1)
// For Phase A1, `margin` is applied as a top-left offset only: positive
// `margin.left` shifts the outer rect rightward, positive `margin.top` shifts
// it downward.  Full TRBL margin layout (e.g. shrinking the available space
// for siblings) is deferred to Phase A3.
//
// # Stacking
// BB does not use flexbox by default.  All siblings are laid out using the
// same parent inner rect as origin (z-order overlay).  If a parent node's
// `_Type_` is `BuildingBlocks_FlexContainer` (or its raw JSON indicates a flex
// layout policy), a warning is logged and the same overlay fallback is used.
// Flex support is deferred to Phase A6.

use std::collections::BTreeMap;

use image::{Rgba, RgbaImage};
use log::warn;

use crate::bb_scene::{BbCoordinateMethod, BbNode, BbNodeId, BbNodeType, BbScene, BbValue};

const SYNTHETIC_NODE_ID_BASE: BbNodeId = 0x8000_0000;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Axis-aligned rectangle in pixel space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge (pixels from canvas left).
    pub x: f32,
    /// Top edge (pixels from canvas top).
    pub y: f32,
    /// Width in pixels.
    pub w: f32,
    /// Height in pixels.
    pub h: f32,
}

impl Rect {
    /// Return a new rect inset by `(top, right, bottom, left)` pixels.
    ///
    /// If the inset would make the dimension negative the result is clamped to
    /// zero size at the centre of the corresponding axis.
    pub fn inset(&self, t: f32, r: f32, b: f32, l: f32) -> Rect {
        let x = self.x + l;
        let y = self.y + t;
        let w = (self.w - l - r).max(0.0);
        let h = (self.h - t - b).max(0.0);
        Rect { x, y, w, h }
    }

    /// Return `true` if `(px, py)` lies inside (or on the edge of) this rect.
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    /// Return the intersection of this rect with `other`, or `None` when they
    /// do not overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        if x1 > x0 && y1 > y0 {
            Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
        } else {
            None
        }
    }

    /// Centre point of this rect.
    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

/// Output of [`layout`]: pixel-space rects for every node and a DFS draw order.
pub struct LayoutResult {
    /// Canvas rect: always `(0, 0, target_w, target_h)`.
    pub canvas: Rect,
    /// Pixel-space outer rect for every node keyed by [`BbNodeId`].
    ///
    /// Inactive nodes (`is_active == false`) are still present in `rects` —
    /// their geometry may affect parent layout — but they are absent from
    /// [`draw_order`].
    pub rects: BTreeMap<BbNodeId, Rect>,
    /// Uniform authoring-canvas-to-target scale applied to fixed measurements.
    pub canvas_scale: f32,
    /// Render order.
    ///
    /// DFS from each root, parent before children.  Siblings are sorted by
    /// `(layer ascending, node-id ascending)`.  Node id order matches the
    /// declaration order in the source JSON (ptr values increase monotonically
    /// with array position).  Inactive nodes are excluded.
    pub draw_order: Vec<BbNodeId>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Layout entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Compute pixel-space rects for every node in `scene` at `(target_w, target_h)`.
///
/// # Scale and letterboxing
/// The BB canvas declares an authoring coordinate size (`scene.canvas_size`).  A
/// **uniform** scale factor `scale = min(target_w / canvas_w, target_h / canvas_h)`
/// is applied so that `Fixed`-unit node positions / sizes are never stretched or
/// squeezed disproportionately.  The scaled canvas is centred within the target:
///
/// ```text
/// letterbox_x = (target_w − canvas_w × scale) / 2
/// letterbox_y = (target_h − canvas_h × scale) / 2
/// ```
///
/// Roots receive the centred canvas rect as their `parent_inner`; percent-based
/// children fill that rect naturally.
///
/// # Panics
/// Never panics on well-formed input.  Unknown `BbValue` behaviors produce
/// warnings and fall back to filling the parent dimension.
pub fn layout(scene: &BbScene, target_w: u32, target_h: u32) -> LayoutResult {
    layout_with_animation_sample(scene, target_w, target_h, None, false, false)
}

/// Compute pixel-space rects while applying sampled animated SizeX/SizeY
/// modifiers when `animation_sample_percent` is provided.
///
/// `cover_fit` selects the scale/anchor for a `UseRaw` canvas whose authored
/// aspect differs from the target: `false` (default) CONTAINS — uniform `min`
/// scale, centred (letterbox); `true` COVERS — uniform `max` scale, origin-anchored
/// so the canvas fills the target and the trailing edge overflows and crops. The
/// pipeline passes `true` only when a per-screen cockpit mesh aspect override is
/// applied (the g-force / velocity ball gauges author 16:9 but their screen is
/// square), so the square `aspectRatio` ball-area covers the screen while the
/// readouts crop. A no-op when aspects match (cover == contain) and for
/// `aspectOverrides*` / `auto` canvases, which FILL the target (non-uniform `sx`/
/// `sy`) regardless of `cover_fit` — the compass authors `auto` on its wide screen.
pub fn layout_with_animation_sample(
    scene: &BbScene,
    target_w: u32,
    target_h: u32,
    animation_sample_percent: Option<f32>,
    cover_fit: bool,
    mesh_aspect_fill: bool,
) -> LayoutResult {
    let (canvas_rect, csx, csy, canvas_scale) = if scene.canvas_size.0 > 0.0 && scene.canvas_size.1 > 0.0 {
        let sx = target_w as f32 / scene.canvas_size.0;
        let sy = target_h as f32 / scene.canvas_size.1;
        match scene.coordinate_method {
            // `auto` fills the target like `aspectOverrides*` (non-uniform `sx`/`sy`,
            // root = full target) — NOT the uniform contain/cover of `useRaw`. The
            // compass authors `auto` on a wide screen with a 16:9 canvas; without
            // filling, its bottom-anchored tick band overflows the shorter target and
            // crops off-screen. No frozen cockpit screen is `auto` (they are `useRaw`
            // / `aspectOverridesWidth`), so this only changes the compass.
            BbCoordinateMethod::AspectOverridesWidth
            | BbCoordinateMethod::AspectOverridesHeight
            | BbCoordinateMethod::Auto => (
                Rect { x: 0.0, y: 0.0, w: target_w as f32, h: target_h as f32 },
                sx,
                sy,
                sx.min(sy),
            ),
            BbCoordinateMethod::UseRaw if cover_fit && mesh_aspect_fill => {
                // A 2D `useRaw` cockpit screen (ball gauges, countermeasure panels)
                // FILLS its square MESH-aspect target non-uniformly (`sx`/`sy`), like
                // `auto`: `PercentOfX/Y` content (the ball) stays proportional, full-
                // width `Percent` content (panels) stretches; Fixed/text use the smaller
                // axis (true screen size, not ×max). Radar's RTT scope keeps COVER below.
                (
                    Rect { x: 0.0, y: 0.0, w: target_w as f32, h: target_h as f32 },
                    sx,
                    sy,
                    sx.min(sy),
                )
            }
            BbCoordinateMethod::UseRaw => {
                // COVER (uniform max, origin-anchored, recentred below) for a mesh-
                // aspect RTT scope (radar); CONTAIN (uniform min, letterboxed) otherwise.
                let canvas_scale = if cover_fit { sx.max(sy) } else { sx.min(sy) };
                let scaled_w = scene.canvas_size.0 * canvas_scale;
                let scaled_h = scene.canvas_size.1 * canvas_scale;
                let (offset_x, offset_y) = if cover_fit {
                    (0.0, 0.0)
                } else {
                    (
                        ((target_w as f32 - scaled_w) * 0.5).max(0.0),
                        ((target_h as f32 - scaled_h) * 0.5).max(0.0),
                    )
                };
                (
                    Rect { x: offset_x, y: offset_y, w: scaled_w, h: scaled_h },
                    canvas_scale,
                    canvas_scale,
                    canvas_scale,
                )
            }
        }
    } else {
        (
            Rect { x: 0.0, y: 0.0, w: target_w as f32, h: target_h as f32 },
            1.0,
            1.0,
            1.0,
        )
    };

    // The LayoutResult.canvas always spans the full target.
    let canvas = Rect { x: 0.0, y: 0.0, w: target_w as f32, h: target_h as f32 };

    let mut rects: BTreeMap<BbNodeId, Rect> = BTreeMap::new();
    let mut draw_order: Vec<BbNodeId> = Vec::new();

    // Collect roots and sort deterministically while preserving authored order
    // when synthetic ids are present (pointerless nodes get high synthetic ids).
    let mut roots: Vec<BbNodeId> = scene.roots.clone();
    let has_synthetic_roots = roots.iter().any(|id| *id >= SYNTHETIC_NODE_ID_BASE);
    if has_synthetic_roots {
        roots.sort_by_key(|&id| scene.nodes.get(&id).map(|n| n.layer).unwrap_or(0));
    } else {
        roots.sort_by_key(|&id| {
            let layer = scene.nodes.get(&id).map(|n| n.layer).unwrap_or(0);
            (layer, id)
        });
    }

    for root_id in roots {
        layout_node(
            root_id,
            canvas_rect,
            scene,
            csx,
            csy,
            animation_sample_percent,
            &mut rects,
            &mut draw_order,
        );
    }

    // Scrollbar thumbs reflect the laid-out scroll model (see part_13.part).
    apply_scroll_thumb_rects(scene, &mut rects);

    // Cover-fit centring (`cover_fit_recentre`): centre the gauge ball-area, but only
    // when the canvas OVERFLOWS the target (cover/radar path; FILL has no overflow).
    let canvas_overflows =
        canvas_rect.w > target_w as f32 + 0.5 || canvas_rect.h > target_h as f32 + 0.5;
    if cover_fit && canvas_overflows {
        cover_fit_recentre(
            &mut rects,
            |id| scene.nodes.get(&id).is_some_and(|n| n.is_active),
            target_w as f32,
            target_h as f32,
            canvas_rect.w,
            canvas_rect.h,
        );
        cover_fit_full_bleed_to_viewport(
            &mut rects,
            scene,
            target_w as f32,
            target_h as f32,
            canvas_rect.w,
            canvas_rect.h,
        );
    }

    LayoutResult { canvas, rects, draw_order, canvas_scale }
}

/// Translate all laid-out `rects` so the gauge BALL-AREA is centred in the
/// `target`, clamped only so the ball-area stays on-screen (coverage of the
/// viewport is the full-bleed background's job, snapped right after by
/// `cover_fit_full_bleed_to_viewport`, so centring exposes no gap).
///
/// The covered canvas is laid out origin-anchored, so its trailing edge overflows
/// the target. The ball-area is the largest LOCALISED node — the largest active
/// rect that does NOT span the full canvas (`canvas_w`/`canvas_h`; this excludes
/// the full-bleed background image and the content root) AND whose centre lies
/// within the canvas (this excludes an off-canvas animation overlay parked outside
/// the canvas by an extreme authored pivot — the countermeasure hold-to-fire
/// circle). It is the primary gauge content and is larger than the readouts panel, so centring on it frames the
/// ball while the readouts overflow and crop. Position-independent: the g-force
/// ball-area sits at canvas-LEFT, the velocity ball's at canvas-RIGHT (mirrored),
/// but both centre. Keyed on relative size + full-canvas span, never a node name.
/// No-op when no localised node exists.
fn cover_fit_recentre(
    rects: &mut BTreeMap<BbNodeId, Rect>,
    is_active: impl Fn(BbNodeId) -> bool,
    target_w: f32,
    target_h: f32,
    canvas_w: f32,
    canvas_h: f32,
) {
    let ball_area = rects
        .iter()
        .filter(|(id, _)| is_active(**id))
        .filter(|(_, r)| {
            // Localised: strictly smaller than the full (cover-scaled) canvas on at
            // least one axis — excludes the full-bleed background and content root.
            let localised = r.w > 0.0 && r.h > 0.0 && !(r.w >= 0.95 * canvas_w && r.h >= 0.95 * canvas_h);
            // On-canvas: the rect's CENTRE lies within the canvas bounds. An
            // animation overlay parked OUTSIDE the canvas by an extreme authored
            // pivot (the countermeasure hold-to-fire circle sits at canvas x≈−4074
            // via pivot.x=22 and slides in only while firing) is not the primary
            // on-screen gauge content; centring on it would shove the real content
            // off-screen. The g-force/velocity ball-areas sit inside the canvas, so
            // they stay eligible. Keyed on geometry, never a node name.
            let cx = r.x + r.w * 0.5;
            let cy = r.y + r.h * 0.5;
            let on_canvas = cx >= 0.0 && cx <= canvas_w && cy >= 0.0 && cy <= canvas_h;
            localised && on_canvas
        })
        .max_by(|a, b| {
            (a.1.w * a.1.h)
                .partial_cmp(&(b.1.w * b.1.h))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, r)| *r);
    let Some(ball) = ball_area else { return };
    // Centre the ball-area in the viewport, but ONLY on an axis where the
    // cover-scaled canvas OVERFLOWS the target — that is the gauge case (the
    // useRaw canvas is scaled wider/taller than the square screen mesh, so its
    // origin-anchored layout must shift to frame the ball). When the canvas
    // exactly fills the target on an axis there is nothing to re-frame, so that
    // axis must NOT shift: aspectOverrides screens (door, annunciator) lay their
    // canvas out at the full target and would otherwise have their content
    // re-centred and pushed off-place. The full-bleed background
    // (`cover_fit_full_bleed_to_viewport`, applied immediately after) fills the
    // viewport independently, so centring never exposes a gap at a leading edge;
    // the only constraint is to keep the ball-area itself on-screen.
    let centre_shift = |target: f32, pos: f32, size: f32| -> f32 {
        let want = target * 0.5 - (pos + size * 0.5);
        let a = -pos; // ball leading edge at viewport origin
        let b = target - size - pos; // ball trailing edge at viewport end
        want.clamp(a.min(b), a.max(b))
    };
    let dx = if canvas_w > target_w + 0.5 { centre_shift(target_w, ball.x, ball.w) } else { 0.0 };
    let dy = if canvas_h > target_h + 0.5 { centre_shift(target_h, ball.y, ball.h) } else { 0.0 };
    if dx != 0.0 || dy != 0.0 {
        for r in rects.values_mut() {
            r.x += dx;
            r.y += dy;
        }
    }
}

/// Snap a cover-fit screen's full-bleed background IMAGE to the visible
/// viewport `(0,0,target)`.
///
/// The covered canvas is laid out wider than the square screen mesh, so a
/// full-canvas background image (`WidgetImage` with an `imagePath`) ends up
/// spanning the off-screen canvas — its square texture stretched to the
/// canvas's 16:9 rect with its bright centre pushed outside the viewport (the
/// velocity/g-force ball's green glow rendered off to one side). The background
/// is a leaf (no children to carry), so snapping its rect to the visible
/// viewport un-stretches it and centres its bright middle on the screen.
/// Containers (laid out absolutely via their children) and localised content
/// are untouched. Keyed on full-canvas span + an image path, never a name.
fn cover_fit_full_bleed_to_viewport(
    rects: &mut BTreeMap<BbNodeId, Rect>,
    scene: &BbScene,
    target_w: f32,
    target_h: f32,
    canvas_w: f32,
    canvas_h: f32,
) {
    for (id, r) in rects.iter_mut() {
        let Some(node) = scene.nodes.get(id) else { continue };
        let renders_image = node
            .raw
            .get("imagePath")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        let spans_canvas = r.w >= 0.95 * canvas_w && r.h >= 0.95 * canvas_h;
        if node.is_active && node.children.is_empty() && renders_image && spans_canvas {
            *r = Rect { x: 0.0, y: 0.0, w: target_w, h: target_h };
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recursive layout
// ─────────────────────────────────────────────────────────────────────────────

fn layout_node(
    node_id: BbNodeId,
    parent_inner: Rect,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let Some(node) = scene.nodes.get(&node_id) else { return };

    // ── 1. Resolve outer dimensions ─────────────────────────────────────────
    //
    // Two-pass resolve: a `PercentOfX` height or `PercentOfY` width is a
    // percentage of THIS NODE's OTHER axis, not the parent's other axis.
    // First compute each axis using parent's other-axis as cross_dim (naïve);
    // then re-resolve any cross-axis behaviour using the node's own naïve
    // opposite dimension. This handles the common "square icon" idiom
    // (e.g. `width: Percent(0.8), height: PercentOfX(1.0)`) correctly while
    // remaining a no-op for non-cross-axis sizing.
    let fills_body_background_surface = fills_body_background_surface(node);
    let width_value = sampled_sizing_value(&node.sizing.width, &node.raw, "SizeX", animation_sample_percent);
    let height_value = sampled_sizing_value(&node.sizing.height, &node.raw, "SizeY", animation_sample_percent);
    let naive_w = resolve_value_for_node(node, &width_value, parent_inner.w, parent_inner.h, csx, true);
    let naive_h = resolve_value_for_node(node, &height_value, parent_inner.h, parent_inner.w, csy, false);
    let base_outer_w = if fills_body_background_surface {
        parent_inner.w
    } else if matches!(width_value, BbValue::Other { ref behavior, .. } if behavior == "PercentOfY") {
        resolve_value_for_node(node, &width_value, parent_inner.w, naive_h, csx, true)
    } else {
        naive_w
    };
    let base_outer_h = if fills_body_background_surface {
        parent_inner.h
    } else if matches!(height_value, BbValue::Other { ref behavior, .. } if behavior == "PercentOfX") {
        resolve_value_for_node(node, &height_value, parent_inner.h, naive_w, csy, false)
    } else {
        naive_h
    };
    let (scale_x, scale_y) = authored_node_scale(node, scene);
    let mut outer_w = base_outer_w * scale_x;
    let mut outer_h = base_outer_h * scale_y;

    // A parent's explicit padding defines its content box and the engine fits
    // fixed-size children into it (the modular-kit ghost button's 64px icon
    // instance inside the Root entry's 15px-padded 64px chrome renders 34px on
    // the medical reference). Unpadded parents keep overlay overflow.
    let parent_has_padding = node
        .parent
        .and_then(|pid| scene.nodes.get(&pid))
        .is_some_and(|p| {
            p.padding.top != 0.0
                || p.padding.right != 0.0
                || p.padding.bottom != 0.0
                || p.padding.left != 0.0
        });
    if parent_has_padding {
        if matches!(width_value, BbValue::Fixed(_)) {
            outer_w = outer_w.min(parent_inner.w);
        }
        if matches!(height_value, BbValue::Fixed(_)) {
            outer_h = outer_h.min(parent_inner.h);
        }
    }

    // ── 2. Anchor / pivot / position ────────────────────────────────────────
    //
    // anchor: normalised point within the *parent inner rect* that the node
    //         "attaches" to.
    // pivot:  normalised point within the *node itself* that lands on the
    //         anchored position.
    // position / positionOffset: additional offset in canvas authoring units.
    //
    // Formula:
    //   anchor_world_x = parent_inner.x + parent_inner.w * anchor.x
    //                    + (position.x + positionOffset.x) * csx
    //   outer.x        = anchor_world_x - outer_w * pivot.x

    let offset_x = sampled_position_offset(
        &node.raw,
        "PosXOffset",
        node.position_offset.x,
        animation_sample_percent,
    );
    let offset_y = sampled_position_offset(
        &node.raw,
        "PosYOffset",
        node.position_offset.y,
        animation_sample_percent,
    );
    let pos_x = (node.position.x + offset_x) * csx;
    let pos_y = (node.position.y + offset_y) * csy;

    let is_root_fullscreen_canvas = matches!(node.ty, BbNodeType::WidgetCanvas)
        && node.parent.is_none_or(|pid| scene.roots.contains(&pid))
        && matches!(width_value, BbValue::Percent(p) if (p - 1.0).abs() < 0.0001)
        && matches!(height_value, BbValue::Percent(p) if p > 0.90)
        && (node.anchor.x - 0.5).abs() < 0.01
        && (node.pivot.x - 0.5).abs() < 0.01;
    let is_child_canvas_surface_root = matches!(node.ty, BbNodeType::DisplayWidget)
        && parent_canvas_is_surface_host(node, scene)
        && matches!(width_value, BbValue::Percent(p) if (p - 1.0).abs() < 0.0001)
        && matches!(height_value, BbValue::Percent(p) if p > 0.90)
        && node.position.x.abs() < 0.0001
        && node.position.y.abs() < 0.0001
        && offset_x.abs() < 0.0001
        && offset_y.abs() < 0.0001;
    let (outer_x, outer_y) = if is_root_fullscreen_canvas || is_child_canvas_surface_root || fills_body_background_surface {
        // Full-bleed containers are parent-space overlays; authoring anchor/pivot
        // offsets should not shift them out of the parent rect.
        (parent_inner.x + pos_x, parent_inner.y + pos_y)
    } else {
        let mirrored_anchor_x = node.anchor.x >= 0.0
            && node.anchor.x <= 1.0
            && node.pivot.x >= 0.99
            && matches!(
                width_value,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 0.0 && value < 1.0
            );
        let anchor_x = if mirrored_anchor_x {
            1.0 - node.anchor.x
        } else {
            node.anchor.x
        };
        let anchor_world_x = parent_inner.x + parent_inner.w * anchor_x + pos_x;
        let anchor_world_y = parent_inner.y + parent_inner.h * node.anchor.y + pos_y;
        (
            anchor_world_x - outer_w * node.pivot.x,
            anchor_world_y - outer_h * node.pivot.y,
        )
    };

    // ── 3. Margin (Phase A1: top-left offset only) ───────────────────────────
    let outer_x = outer_x + node.margin.left * csx;
    let outer_y = outer_y + node.margin.top * csy;

    let outer_rect = Rect { x: outer_x, y: outer_y, w: outer_w, h: outer_h };

    layout_node_with_rect(
        node_id,
        outer_rect,
        scene,
        csx,
        csy,
        animation_sample_percent,
        rects,
        draw_order,
    );
}

fn layout_node_with_rect(
    node_id: BbNodeId,
    outer_rect: Rect,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let Some(node) = scene.nodes.get(&node_id) else { return };

    // ── 4. Inner rect = outer rect inset by padding ──────────────────────────
    let inner_rect = outer_rect.inset(
        node.padding.top * csy,
        node.padding.right * csx,
        node.padding.bottom * csy,
        node.padding.left * csx,
    );

    rects.insert(node_id, outer_rect);

    // Add to draw order only when the node is active.
    if !node.is_active {
        return;
    }
    draw_order.push(node_id);

    // ── 5. Recurse into children ─────────────────────────────────────────────
    // If the sibling set includes synthetic node IDs (pointerless authored
    // nodes), keep authored order for equal layers. Otherwise keep the prior
    // deterministic (layer, id) order.
    let mut children: Vec<BbNodeId> = node.children.clone();
    let has_synthetic = children.iter().any(|id| *id >= SYNTHETIC_NODE_ID_BASE);
    if has_synthetic {
        children.sort_by_key(|&child_id| scene.nodes.get(&child_id).map(|n| n.layer).unwrap_or(0));
    } else {
        children.sort_by_key(|&child_id| {
            let layer = scene.nodes.get(&child_id).map(|n| n.layer).unwrap_or(0);
            (layer, child_id)
        });
    }

    // Detect FlexContainer layout policy — if present, use flex layout instead
    // of overlay for child positioning.
    let flex_policy = node.raw.get("layoutPolicy").filter(|v| {
        v.get("_Type_")
            .and_then(|t| t.as_str())
            .map(|t| t.contains("FlexContainer"))
            .unwrap_or(false)
    });

    if let Some(flex) = flex_policy {
        // FlexContainer FLOW order is the AUTHORED child order with each
        // item's `layoutItemCommon.order` override (CSS flex `order`) — layer
        // is a draw-order concern only (the power item authors its pip list
        // on layer 5 next to the layer-0 heat-bar container, flowing
        // pips-then-gauge). The MFD footer's Prev/Name/Next relies on the
        // `order` overrides so the nav carats sit at the bar's far ends.
        let mut flow_children: Vec<BbNodeId> = node.children.clone();
        flow_children.sort_by_key(|&child_id| {
            scene
                .nodes
                .get(&child_id)
                .and_then(|n| n.raw.get("layoutItemCommon"))
                .and_then(|c| c.get("order"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        });
        // Scrolling containers keep child overflow (the pip list's columns
        // run past the viewport by design); plain flows flex-shrink to fit.
        let container_scrollable = node
            .raw
            .get("scrollPolicy")
            .is_some_and(|policy| !policy.is_null());
        layout_flex_children(
            &flow_children,
            inner_rect,
            flex,
            node.pivot.x,
            container_scrollable,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    } else {
        for child_id in children {
            layout_node(
                child_id,
                inner_rect,
                scene,
                csx,
                csy,
                animation_sample_percent,
                rects,
                draw_order,
            );
        }
    }
}

fn fills_body_background_surface(node: &crate::bb_scene::BbNode) -> bool {
    if !matches!(node.ty, BbNodeType::WidgetBodyBackground) {
        return false;
    }

    let uses_texture_background = node
        .raw
        .get("backgroundType")
        .and_then(|value| {
            value
                .as_str()
                .map(|text| text.eq_ignore_ascii_case("Texture"))
                .or_else(|| value.as_i64().map(|number| number == 1))
        })
        .unwrap_or(false);

    uses_texture_background && node.raw.get("textureProperties").is_some()
}

fn parent_canvas_is_surface_host(node: &crate::bb_scene::BbNode, scene: &BbScene) -> bool {
    node.parent
        .and_then(|parent_id| scene.nodes.get(&parent_id))
        .is_some_and(|parent| {
            matches!(parent.ty, BbNodeType::WidgetCanvas)
                && matches!(parent.sizing.width, BbValue::Percent(p) if (p - 1.0).abs() < 0.0001)
                && matches!(parent.sizing.height, BbValue::Percent(p) if p > 0.90)
        })
}

fn sampled_sizing_value(
    authored: &BbValue,
    raw: &serde_json::Value,
    field_name: &str,
    animation_sample_percent: Option<f32>,
) -> BbValue {
    let Some(sample_percent) = animation_sample_percent else {
        return authored.clone();
    };
    let Some(sampled_value) = sampled_animation_number(raw, field_name, sample_percent) else {
        return authored.clone();
    };

    match authored {
        BbValue::Fixed(_) => BbValue::Fixed(sampled_value),
        BbValue::Percent(_) => BbValue::Percent(sampled_value),
        BbValue::Other { behavior, .. } => BbValue::Other {
            value: sampled_value,
            behavior: behavior.clone(),
        },
    }
}

fn sampled_position_offset(
    raw: &serde_json::Value,
    field_name: &str,
    authored_offset: f32,
    animation_sample_percent: Option<f32>,
) -> f32 {
    animation_sample_percent
        .and_then(|sample_percent| {
            sampled_animation_number(
                raw,
                field_name,
                position_animation_sample_percent(raw, sample_percent),
            )
        })
        .unwrap_or(authored_offset)
}

fn position_animation_sample_percent(raw: &serde_json::Value, sample_percent: f32) -> f32 {
    let Some(animation) = raw.get("animation") else {
        return sample_percent;
    };
    let direction = animation
        .get("direction")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let loops = animation
        .get("loopIndefinitely")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let additive = animation
        .get("additive")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    // The hand-tuned 2/3 phase ratio for additive AlternateReverse loops was
    // deleted 2026-06-12: no frozen pin referenced it (remediation plan
    // Phase 4 audit) — the midpoint sample stands until an at-rest capture
    // discriminates a phase.
    let _ = (direction, loops, additive);
    sample_percent
}

fn animation_number_keyframes(raw: &serde_json::Value, field_name: &str) -> Vec<(f64, f32)> {
    let Some(keyframes) = raw
        .get("animation")
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

fn layout_flex_children(
    children: &[BbNodeId],
    container: Rect,
    flex: &serde_json::Value,
    container_pivot_x: f32,
    container_scrollable: bool,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
    let direction = flex
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("Row")
        .to_ascii_lowercase();
    let is_row = !direction.starts_with("column");
    // `ColumnReverse`/`RowReverse`: the main-start edge is the container's
    // far end — items stack bottom-up / right-to-left in authored order
    // (the power screen's `list_PowerBars` pip stack).
    let reversed = direction.ends_with("reverse");

    // Spacing between items (columnSpacing for Row, rowSpacing for Column).
    let item_spacing = if is_row {
        flex.get("columnSpacing")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32
            * csx
    } else {
        flex.get("rowSpacing")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32
            * csy
    };

    // Separate flex children (with growProportion) from non-flex children.
    struct FlexChild {
        id: BbNodeId,
        grow: f32,
    }
    let mut flex_children: Vec<FlexChild> = Vec::new();
    let mut flow_non_grow: Vec<BbNodeId> = Vec::new();
    let mut overlay_non_flex: Vec<BbNodeId> = Vec::new();

    for &child_id in children {
        let Some(child_node) = scene.nodes.get(&child_id) else { continue };
        if !child_node.is_active {
            // Keep inactive nodes laid out for diagnostics, but do not let them
            // consume flex-flow slots that push active siblings off-screen.
            overlay_non_flex.push(child_id);
            continue;
        }
        let affects_layout = child_node
            .raw
            .get("affectsLayout")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let grow = child_node
            .raw
            .get("layoutPolicyItem")
            .and_then(|lpi| lpi.get("growProportion"))
            .and_then(|v| v.as_f64())
            .map(|g| g as f32)
            .unwrap_or(0.0);
        if affects_layout && grow > 0.0 {
            flex_children.push(FlexChild { id: child_id, grow });
        } else if affects_layout {
            flow_non_grow.push(child_id);
        } else {
            overlay_non_flex.push(child_id);
        }
    }

    // Children that do not participate in flex layout are overlayed.
    for child_id in overlay_non_flex {
        layout_node(
            child_id,
            container,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    }

    if reversed {
        flex_children.reverse();
    }

    if flex_children.is_empty() {
        if !flow_non_grow.is_empty() {
            layout_flex_no_grow_children(
                &flow_non_grow,
                container,
                flex,
                container_pivot_x,
                container_scrollable,
                scene,
                csx,
                csy,
                animation_sample_percent,
                rects,
                draw_order,
                is_row,
                reversed,
            );
        } else {
            for child_id in flow_non_grow {
                layout_node(
                    child_id,
                    container,
                    scene,
                    csx,
                    csy,
                    animation_sample_percent,
                    rects,
                    draw_order,
                );
            }
        }
        return;
    }

    for child_id in flow_non_grow {
        layout_node(
            child_id,
            container,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    }

    let n = flex_children.len();
    let total_spacing = item_spacing * (n as f32 - 1.0).max(0.0);
    let total_grow: f32 = flex_children.iter().map(|c| c.grow).sum();

    // Main axis: available space divided by grow proportions.
    let main_available = if is_row {
        (container.w - total_spacing).max(0.0)
    } else {
        (container.h - total_spacing).max(0.0)
    };

    let mut cursor = if is_row { container.x } else { container.y };

    for FlexChild { id, grow } in &flex_children {
        let main_size = (grow / total_grow) * main_available;
        // Cross axis: stretch to fill container (BB default: itemAlignment=Stretch).
        let child_rect = if is_row {
            Rect {
                x: cursor,
                y: container.y,
                w: main_size,
                h: container.h,
            }
        } else {
            Rect {
                x: container.x,
                y: cursor,
                w: container.w,
                h: main_size,
            }
        };
        cursor += main_size + item_spacing;
        layout_node_with_rect(
            *id,
            child_rect,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
    }
}

fn layout_flex_no_grow_children(
    children: &[BbNodeId],
    container: Rect,
    flex: &serde_json::Value,
    container_pivot_x: f32,
    container_scrollable: bool,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
    is_row: bool,
    reversed: bool,
) {
    if children.is_empty() {
        return;
    }
    let mut item_spacing = if is_row {
        flex.get("columnSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csx
    } else {
        flex.get("rowSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csy
    };
    let tighten_centered_intrinsic_text_column = centered_intrinsic_text_column_spacing_applies(children, flex, scene, is_row);
    let axis_just = flex.get("axisJustification").and_then(|v| v.as_str()).unwrap_or("Start");
    let cross_just = flex
        .get("crossAxisJustification")
        .and_then(|v| v.as_str())
        .or_else(|| flex.get("itemAlignment").and_then(|v| v.as_str()))
        .unwrap_or("Stretch");
    let wrap_enabled = is_row
        && flex
        .get("wrap")
        .and_then(|v| v.as_str())
        .is_some_and(|w| w.eq_ignore_ascii_case("Wrap"));
    let cross_spacing = if is_row {
        flex.get("rowSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csy
    } else {
        flex.get("columnSpacing").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 * csx
    };

    let mut sizes: Vec<(BbNodeId, f32, f32, bool)> = Vec::with_capacity(children.len());
    let mut right_aligned_flow_items = 0usize;
    let mut total_main = 0.0f32;
    for &child_id in children {
        let Some(node) = scene.nodes.get(&child_id) else { continue };
        if !node.is_active {
            continue;
        }
        if node
            .raw
            .get("affectsLayout")
            .and_then(|v| v.as_bool())
            == Some(false)
        {
            continue;
        }
        if (node.name.is_empty() || node.name == "<unnamed>")
            && matches!(&node.ty, BbNodeType::Other(kind) if kind == "BuildingBlocks_WidgetLinearProgressMeter")
        {
            continue;
        }
        let mut w = resolve_value_for_node(node, &node.sizing.width, container.w, container.h, csx, true);
        let mut h = resolve_value_for_node(node, &node.sizing.height, container.h, container.w, csy, false);
        if matches!(node.sizing.width, BbValue::Other { ref behavior, .. } if behavior == "PercentOfY")
        {
            w = resolve_value_for_node(node, &node.sizing.width, container.w, h, csx, true);
        }
        if matches!(node.sizing.height, BbValue::Other { ref behavior, .. } if behavior == "PercentOfX")
        {
            h = resolve_value_for_node(node, &node.sizing.height, container.h, w, csy, false);
        }
        // A padded flex container's content box caps fixed-size items (the
        // modular-kit ghost button's 64px icon inside the 15px-padded chrome
        // renders 34px on the medical reference). Twin of the overlay-path
        // rule in `layout_node`; unpadded containers keep legacy overflow.
        let parent_padded = node
            .parent
            .and_then(|pid| scene.nodes.get(&pid))
            .is_some_and(|p| {
                p.padding.top != 0.0
                    || p.padding.right != 0.0
                    || p.padding.bottom != 0.0
                    || p.padding.left != 0.0
            });
        if parent_padded {
            if matches!(node.sizing.width, BbValue::Fixed(_)) {
                w = w.min(container.w);
            }
            if matches!(node.sizing.height, BbValue::Fixed(_)) {
                h = h.min(container.h);
            }
        }
        // In non-grow flex flow, "Auto" on main axis behaves like content-fit.
        // We do not have content measurement here, so treat it as zero so fixed/
        // percent children can still be axis-justified (instead of Auto filling
        // the whole container and pushing everything out of view).
        let mut auto_main = if is_row {
            matches!(node.sizing.width, BbValue::Other { ref behavior, .. } if behavior == "Auto")
        } else {
            matches!(node.sizing.height, BbValue::Other { ref behavior, .. } if behavior == "Auto")
        };
        let is_button_component = matches!(
            node.ty,
            BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
        );
        let auto_intrinsic_hint = if is_row {
            matches!(
                node.sizing.width,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 1.0
            )
        } else {
            matches!(
                node.sizing.height,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 1.0
            )
        };
        let is_label_caption_pair_node = matches!(
            &node.ty,
            BbNodeType::Other(kind)
                if kind.eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
        );
        let has_main_axis_intrinsic_override = if is_row {
            textfield_auto_intrinsic_override(node, &node.sizing.width, container.w, csx, true).is_some()
                || component_button_auto_intrinsic_override(node, &node.sizing.width, csx).is_some()
        } else {
            textfield_auto_intrinsic_override(node, &node.sizing.height, container.h, csy, false).is_some()
                || component_button_auto_intrinsic_override(node, &node.sizing.height, csy).is_some()
        };
        if auto_main
            && (is_button_component && auto_intrinsic_hint
                || (has_main_axis_intrinsic_override && !is_label_caption_pair_node))
        {
            auto_main = false;
        }
        let right_edge_auto_hint = node.pivot.x >= 0.99
            && matches!(
                node.sizing.width,
                BbValue::Other {
                    value,
                    ref behavior
                } if behavior == "Auto" && value > 0.0 && value < 1.0
            );
        if auto_main {
            let normalized_auto = if is_row {
                match &node.sizing.width {
                    BbValue::Other { value, behavior } if behavior == "Auto" && *value > 0.0 && *value < 1.0 => Some(*value),
                    _ => None,
                }
            } else {
                match &node.sizing.height {
                    BbValue::Other { value, behavior } if behavior == "Auto" && *value > 0.0 && *value < 1.0 => Some(*value),
                    _ => None,
                }
            };
            let is_label_caption_pair = is_label_caption_pair_node;
            if let Some(auto_ratio) = normalized_auto {
                if is_row {
                    w = container.w * auto_ratio;
                } else if let Some(intrinsic) = auto_text_intrinsic_main_wrapped(child_id, scene, csy, false, Some(w)) {
                    // A COLUMN child authored non-zero `Auto` sizes to its TEXT
                    // content, exactly like the 0.0 (pure-hint) case below: "Auto"
                    // means fit-to-content, and the value is only the NO-CONTENT
                    // fallback fraction (the `else` fill). This lands the power
                    // battery card's OFFLINE row at the reference — the old fill
                    // `v×container` made `base_ValuesContainer` over-tall (0.9 →
                    // 141.9px on 93px of "0/0") and pushed OFFLINE ~35px low
                    // (handoff P14). Safe for the frozen targets: medical's header
                    // is a ROW child (cross-axis), so the fill those baselines pin
                    // is untouched, and the OUTPUT/emissions containers author
                    // value≥1.0 hints (handled above) — verified by ui_check --full.
                    h = intrinsic;
                } else {
                    h = container.h * auto_ratio;
                }
                auto_main = false;
            } else if is_label_caption_pair {
                if is_row {
                    let is_right_anchored_pair = node.anchor.x >= 0.99 || node.pivot.x >= 0.99;
                    let has_active_children = node.children.iter().any(|child_id| {
                        scene
                            .nodes
                            .get(child_id)
                            .is_some_and(|child| child.is_active)
                    });

                    // Child-backed pairs (e.g. progress-meter stacks) derive
                    // width from authored Auto plus active child footprint.
                    if has_active_children {
                        if let BbValue::Other { value, behavior } = &node.sizing.width
                            && behavior == "Auto" && *value > 1.0
                        {
                            w = *value * csx;
                        }
                    }

                    // Childless non-right-anchored row pairs share available
                    // width with peer pairs to avoid clipping long text labels.
                    if !has_active_children && !is_right_anchored_pair {
                        let active_pair_count = children
                            .iter()
                            .filter_map(|candidate_id| scene.nodes.get(candidate_id))
                            .filter(|candidate| {
                                if !candidate.is_active {
                                    return false;
                                }
                                let is_pair = matches!(
                                    &candidate.ty,
                                    BbNodeType::Other(kind)
                                        if kind.eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
                                );
                                if !is_pair {
                                    return false;
                                }
                                if is_placeholder_only_label_caption_pair(candidate) {
                                    return false;
                                }
                                let candidate_has_active_children = candidate.children.iter().any(|child_id| {
                                    scene
                                        .nodes
                                        .get(child_id)
                                        .is_some_and(|child| child.is_active)
                                });
                                let candidate_right_anchored = candidate.anchor.x >= 0.99 || candidate.pivot.x >= 0.99;
                                !candidate_has_active_children && !candidate_right_anchored
                            })
                            .count();
                        if active_pair_count > 0 {
                            let total_spacing = item_spacing * (active_pair_count.saturating_sub(1) as f32);
                            let shared_main = (container.w - total_spacing) / active_pair_count as f32;
                            w = shared_main.max(0.0);
                        }
                    }

                    // Childless right-side metric pairs stay at authored Auto
                    // intrinsic width so they do not consume full row width.
                    if !has_active_children && is_right_anchored_pair {
                        if let BbValue::Other { value, behavior } = &node.sizing.width
                            && behavior == "Auto" && *value > 1.0
                        {
                            w = *value * csx;
                        }
                    }

                    if let BbValue::Other { value, behavior } = &node.sizing.height
                        && behavior == "Auto" && *value > 1.0
                    {
                        if !has_active_children && !is_right_anchored_pair {
                            h = container.h;
                        } else {
                            h = *value * csy;
                        }
                    }
                    for child_id in &node.children {
                        let Some(child) = scene.nodes.get(child_id) else {
                            continue;
                        };
                        if !child.is_active {
                            continue;
                        }
                        let child_w = resolve_value_for_node(
                            child,
                            &child.sizing.width,
                            container.w,
                            container.h,
                            csx,
                            true,
                        );
                        let child_h = resolve_value_for_node(
                            child,
                            &child.sizing.height,
                            container.h,
                            container.w,
                            csy,
                            false,
                        );
                        w = w.max(child_w.max(0.0));
                        h = h.max(child_h.max(0.0));
                    }
                    auto_main = false;
                } else {
                    h = (container.h * 0.22).max(48.0 * csy);
                    auto_main = false;
                }
            } else if is_row
                && let Some(intrinsic) =
                    auto_text_intrinsic_main(child_id, scene, csx, true)
            {
                // Text-backed Auto children in a ROW flow at their measured
                // intrinsic width — both all-Auto value pairs (the OUTPUT
                // card's "2" + "/ 16", which the zero rule collapses onto one
                // spot) and mixed rows (the battery header's icon + separator
                // + Auto BATTERY title, which the fill placement paints over
                // the icon).
                w = intrinsic;
                auto_main = false;
            } else if !is_row
                && matches!(
                    node.sizing.height,
                    BbValue::Other { value, ref behavior } if behavior == "Auto" && value == 0.0
                )
                && let Some(intrinsic) =
                    auto_text_intrinsic_main_wrapped(child_id, scene, csy, false, Some(w))
            {
                // Text-backed Auto ZERO-hint (value 0.0 = pure content hint)
                // children in a COLUMN stack at measured text heights — the
                // emissions Numbers Container stacks emitted above ambient.
                // Scoped to 0.0 only by default: the medical platinum pins the
                // fill placement for NON-zero Auto hints in columns (a
                // column-wide intrinsic drifted the medical header h 78→18,
                // y +27). The CENTER-justified case is carved out below.
                h = intrinsic;
                auto_main = false;
            } else if !is_row
                && axis_just.eq_ignore_ascii_case("Center")
                && matches!(
                    node.sizing.height,
                    BbValue::Other { value, ref behavior }
                        if behavior == "Auto"
                            && (value > 1.0
                                || (value >= 1.0 && matches!(node.ty, BbNodeType::WidgetCard)))
                )
                && let Some(intrinsic) =
                    auto_text_intrinsic_main_wrapped(child_id, scene, csy, false, Some(w))
            {
                // Text-backed Auto-hint children in a CENTER-justified column fit
                // their measured TEXT content, like the 0.0 pure-hint case above
                // and the (0,1.0) fraction case in `normalized_auto`. The
                // velocity-num HUD (HC_HUD_Ship_Velocity_Num_Master) stacks two
                // such WidgetCards — each wrapping one readout text field ("0m/s",
                // "0.0 G", value 64). The master-mode display
                // (HC_HUD_Ship_Master_Mode_Display_Master) stacks "SCM" / submode
                // WidgetCards at value EXACTLY 1.0; the fill placement made both
                // cards full-canvas height and overlapping (the big grey box).
                //
                // Discriminator for the value==1.0 boundary: only a WidgetCard
                // (a content WRAPPER that should hug its text) takes the `>= 1.0`
                // extension. A BARE WidgetTextField at value==1.0 keeps the fill
                // fallback — the power screen's gold "2 / 16" readout
                // (`564/565:widget_text_field`) lives in a center-justified column
                // too but is authored Top/Left-aligned to fill its slot; content-
                // fitting it re-centred the text +101px (live-IR guard caught it).
                // Scoped to axisJustification == Center so the medical header (its
                // column is NOT center-justified) keeps its platinum fill (§10).
                h = intrinsic;
                auto_main = false;
            } else if !is_row
                && node
                    .raw
                    .get("_MaterialisedEntry_")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                && matches!(
                    node.sizing.height,
                    BbValue::Other { ref behavior, .. } if behavior == "Auto"
                )
                && let Some(intrinsic) = auto_text_intrinsic_main_wrapped(child_id, scene, csy, false, Some(w))
            {
                // A materialised LIST ENTRY (cloned per array element, flagged
                // `_MaterialisedEntry_` by `apply_array_variable_lists`) with Auto
                // height fits its content so the list's flex justification can
                // DISTRIBUTE entries down the column instead of overlaying every clone
                // at one anchor (the countermeasure list shows one launcher panel per
                // entry). Scoped to materialised + Auto height, so authored column
                // children (medical/power fill pins, Fixed/Percent pips) are untouched.
                h = intrinsic;
                auto_main = false;
            } else if is_row {
                w = 0.0;
            } else {
                h = 0.0;
            }
        }
        // Cross-axis content-fit for a CENTER-justified column's Auto text card:
        // shrink the snug box on the cross axis (width in a column) too, so the
        // crossAxisJustification placement below centres it. Without this the
        // velocity-num readout cards stay full-canvas-wide and their left-aligned
        // glyphs render at the (off-screen) left edge even after the main-axis
        // fit stacks them. Same axisJustification == Center scope as the
        // main-axis intrinsic above, so no frozen screen is reached.
        if axis_just.eq_ignore_ascii_case("Center") {
            // value==1.0 only for a WidgetCard wrapper (see the main-axis gate
            // above); bare fields at 1.0 keep the fill so the power "2 / 16"
            // readout is untouched.
            let is_card = matches!(node.ty, BbNodeType::WidgetCard);
            let cross_auto_ok = |value: f32| value > 1.0 || (value >= 1.0 && is_card);
            let cross_is_nonzero_auto = if is_row {
                matches!(node.sizing.height, BbValue::Other { value, ref behavior } if behavior == "Auto" && cross_auto_ok(value))
            } else {
                matches!(node.sizing.width, BbValue::Other { value, ref behavior } if behavior == "Auto" && cross_auto_ok(value))
            };
            if cross_is_nonzero_auto {
                if is_row {
                    if let Some(intrinsic) = auto_text_intrinsic_main(child_id, scene, csy, false) {
                        h = intrinsic;
                    }
                } else if let Some(intrinsic) = auto_text_intrinsic_main(child_id, scene, csx, true) {
                    w = intrinsic;
                }
            }
        }
        // A RIGHT-anchored materialised LIST ENTRY in a COLUMN content-fits its
        // CROSS width so the placement below right-aligns it per `anchor.x=1.0` —
        // count card at the panel's RIGHT (ref), dot overlay (entry-left, pivot.x=22)
        // at the left. Scoped to materialised rows + right anchor + Auto width.
        if !is_row
            && node
                .raw
                .get("_MaterialisedEntry_")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            && (node.anchor.x >= 0.99 || node.pivot.x >= 0.99)
            && matches!(node.sizing.width, BbValue::Other { ref behavior, .. } if behavior == "Auto")
            && let Some(intrinsic) = auto_text_intrinsic_main(child_id, scene, csx, true)
        {
            w = intrinsic;
        }
        if (node.anchor.x >= 0.99 && node.pivot.x >= 0.99) || right_edge_auto_hint {
            right_aligned_flow_items += 1;
        }
        total_main += if is_row { w.max(0.0) } else { h.max(0.0) };
        sizes.push((child_id, w.max(0.0), h.max(0.0), auto_main));
    }
    if sizes.is_empty() {
        return;
    }
    let mut centered_intrinsic_main_offset_shift = 0.0f32;
    if tighten_centered_intrinsic_text_column {
        let (adjusted_spacing, main_offset_shift) = centered_intrinsic_text_column_adjustment(
            item_spacing,
            &sizes,
            scene,
            container.h,
            is_row,
        );
        item_spacing = adjusted_spacing;
        centered_intrinsic_main_offset_shift = main_offset_shift;
    }

    total_main += item_spacing * (sizes.len().saturating_sub(1) as f32);
    // Standard flex SHRINK — policy and exemptions in `apply_flex_no_grow_shrink`.
    total_main = apply_flex_no_grow_shrink(
        &mut sizes, scene, container, is_row, item_spacing, total_main,
        container_scrollable, wrap_enabled,
    );
    // Reverse-direction flex lays authored order from the container's far
    // end: lay the REVERSED order top-down/left-right with the block aligned
    // to the end (Start justification means the far edge there).
    if reversed {
        sizes.reverse();
    }
    let avail_main = if is_row { container.w } else { container.h };
    let axis_just_lc = axis_just.to_ascii_lowercase();
    let pivot_start_from_end = is_row && axis_just_lc == "start" && container_pivot_x >= 0.99;
    let child_start_from_end = is_row
        && axis_just_lc == "start"
        && !sizes.is_empty()
        && right_aligned_flow_items * 2 >= sizes.len();
    let start_from_end = pivot_start_from_end || child_start_from_end;
    let cross_just_lc = cross_just.to_ascii_lowercase();
    let cross_start_from_end = !is_row
        && cross_just_lc == "start"
        && (container_pivot_x >= 0.99 || (!sizes.is_empty() && right_aligned_flow_items * 2 >= sizes.len()));
    let mut main_offset = match axis_just_lc.as_str() {
        "center" => ((avail_main - total_main) * 0.5).max(0.0),
        "end" | "right" | "bottom" => (avail_main - total_main).max(0.0),
        "start" if start_from_end || reversed => (avail_main - total_main).max(0.0),
        _ => 0.0,
    };
    if tighten_centered_intrinsic_text_column {
        main_offset = (main_offset - centered_intrinsic_main_offset_shift).max(0.0);
    }
    // Space-distributing justification (authored SpaceBetween/SpaceAround/
    // SpaceEvenly) shares the free main-axis space out as equal gaps instead of
    // packing every item against one edge (the match above leaves these at
    // main_offset 0, i.e. Start). `total_main` already includes the base
    // inter-item spacing, so `slack` is the surplus to distribute. The wrap path
    // keeps its own per-line handling and is unaffected.
    let item_count = sizes.len();
    let slack = (avail_main - total_main).max(0.0);
    let mut extra_between = 0.0f32;
    match axis_just_lc.as_str() {
        "spacebetween" if item_count >= 2 => {
            main_offset = 0.0;
            extra_between = slack / (item_count as f32 - 1.0);
        }
        "spacearound" if item_count >= 1 => {
            extra_between = slack / item_count as f32;
            main_offset = extra_between * 0.5;
        }
        "spaceevenly" if item_count >= 1 => {
            extra_between = slack / (item_count as f32 + 1.0);
            main_offset = extra_between;
        }
        _ => {}
    }
    let mut cursor = if is_row { container.x + main_offset } else { container.y + main_offset };
    if wrap_enabled {
        layout_flex_wrap_children(
            sizes,
            container,
            item_spacing,
            cross_spacing,
            is_row,
            &axis_just_lc,
            cross_just,
            start_from_end,
            cross_start_from_end,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
        return;
    }
    // Equal-gap spread for space-distributing justification; zero (no-op) for
    // every other mode.
    item_spacing += extra_between;
    for (id, w, h, auto_main) in sizes {
        let column_right_edge_auto = !is_row
            && scene.nodes.get(&id).is_some_and(|node| {
                node.pivot.x >= 0.99
                    && matches!(
                        node.sizing.width,
                        BbValue::Other {
                            value,
                            ref behavior
                        } if behavior == "Auto" && value > 0.0 && value < 1.0
                    )
            });

        if auto_main || column_right_edge_auto {
            if !is_row && let Some(node) = scene.nodes.get(&id) {
                let right_edge_auto_hint = node.pivot.x >= 0.99
                    && matches!(
                        node.sizing.width,
                        BbValue::Other {
                            value,
                            ref behavior
                        } if behavior == "Auto" && value > 0.0 && value < 1.0
                    );
                if right_edge_auto_hint {
                    let slot_x = container.x + (container.w - w).max(0.0);
                    let slot_y = cursor;
                    let anchor_x = node.anchor.x.clamp(0.0, 1.0);
                    // Y is the main axis of this column: follow the flex cursor.
                    // anchor.y belongs to overlay/non-flex positioning only.
                    let resolved_x = container.x + (slot_x - container.x) * (1.0 - anchor_x);
                    let resolved_y = slot_y;
                    let rect = Rect {
                        x: resolved_x,
                        y: resolved_y,
                        w,
                        h,
                    };
                    layout_node_with_rect(
                        id,
                        rect,
                        scene,
                        csx,
                        csy,
                        animation_sample_percent,
                        rects,
                        draw_order,
                    );
                    cursor += h;
                    cursor += item_spacing;
                    continue;
                }
            }
            // Auto-sized text-like items still contribute spacing/alignment
            // slots, but keep their own overlay layout so they can render with
            // intrinsic content bounds.
            layout_node(
                id,
                container,
                scene,
                csx,
                csy,
                animation_sample_percent,
                rects,
                draw_order,
            );
            cursor += if is_row { w } else { h };
            cursor += item_spacing;
            continue;
        }
        let rect = if is_row {
            let mut x = cursor;
            if let Some(node) = scene.nodes.get(&id) {
                if let Some(anchor_offset) = row_flex_start_anchor_offset(node, container.w, w, csx) {
                    x += anchor_offset;
                }
            }
            let y = match cross_just.to_ascii_lowercase().as_str() {
                "center" => container.y + (container.h - h) * 0.5,
                "end" | "right" | "bottom" => container.y + (container.h - h),
                _ => container.y,
            };
            let ch = if cross_just.eq_ignore_ascii_case("stretch") { container.h } else { h };
            Rect { x, y, w, h: ch }
        } else {
            let mut x = match cross_just.to_ascii_lowercase().as_str() {
                "center" => container.x + (container.w - w) * 0.5,
                "end" | "right" | "bottom" => container.x + (container.w - w),
                "start" if cross_start_from_end => container.x + (container.w - w),
                _ => container.x,
            };
            if let Some(node) = scene.nodes.get(&id) {
                let pos_x = (node.position.x + node.position_offset.x) * csx;
                if cross_just.eq_ignore_ascii_case("center") {
                    // The `(container.w - w) * 0.5` base already CENTRES the box.
                    // The authored anchor/pivot pair `anchor.x*W - pivot.x*w` is an
                    // OVERLAY offset that only makes sense when the item authors a
                    // cross-axis anchor (then it self-positions, e.g. anchor 0.5 /
                    // pivot 0.5 cancels to stay centred; anchor 0.043 nudges). With
                    // NO authored cross anchor (anchor.x == 0) the item relies on
                    // flex box-centring, so re-subtracting `pivot.x * w` would push
                    // a centred-pivot (0.5) item a half-width off-centre — the
                    // medical close ✕ (anchor 0, pivot 0.5) rendered left of centre.
                    if node.anchor.x.abs() > f32::EPSILON {
                        x += (node.anchor.x * container.w) + pos_x - (node.pivot.x * w);
                    } else {
                        x += pos_x;
                    }
                } else if cross_just.eq_ignore_ascii_case("start") && !cross_start_from_end {
                    x += (node.anchor.x * container.w) + pos_x - (node.pivot.x * w);
                }
            }
            let cw = if cross_just.eq_ignore_ascii_case("stretch") { container.w } else { w };
            Rect { x, y: cursor, w: cw, h }
        };
        layout_node_with_rect(
            id,
            rect,
            scene,
            csx,
            csy,
            animation_sample_percent,
            rects,
            draw_order,
        );
        cursor += if is_row { w } else { h };
        cursor += item_spacing;
    }
}

fn layout_flex_wrap_children(
    sizes: Vec<(BbNodeId, f32, f32, bool)>,
    container: Rect,
    item_spacing: f32,
    cross_spacing: f32,
    is_row: bool,
    axis_just_lc: &str,
    cross_just: &str,
    start_from_end: bool,
    cross_start_from_end: bool,
    scene: &BbScene,
    csx: f32,
    csy: f32,
    animation_sample_percent: Option<f32>,
    rects: &mut BTreeMap<BbNodeId, Rect>,
    draw_order: &mut Vec<BbNodeId>,
) {
        // Build wrapped lines first so axis justification (e.g. Center) can be
        // applied per line instead of always starting at the container edge.
        let mut lines: Vec<Vec<(BbNodeId, f32, f32, bool)>> = Vec::new();
        let mut current: Vec<(BbNodeId, f32, f32, bool)> = Vec::new();
        let mut current_main = 0.0f32;
        let avail_main = if is_row { container.w } else { container.h };
        for item in sizes {
            let (_, w, h, auto_main) = item;
            if auto_main {
                // Keep auto-main nodes on the current line; they are laid out
                // with their own pass and do not consume wrapping width here.
                current.push(item);
                continue;
            }
            let main = if is_row { w } else { h };
            let proposed = if current.is_empty() {
                main
            } else {
                current_main + item_spacing + main
            };
            if !current.is_empty() && proposed > avail_main + 0.5 {
                lines.push(current);
                current = Vec::new();
                current_main = 0.0;
            }
            current_main = if current.is_empty() {
                main
            } else {
                current_main + item_spacing + main
            };
            current.push(item);
        }
        if !current.is_empty() {
            lines.push(current);
        }

        let mut line_cross_cursor = if is_row { container.y } else { container.x };
        for line in lines {
            let mut line_main = 0.0f32;
            let mut line_cross = 0.0f32;
            let mut line_items = 0usize;
            for &(_, w, h, auto_main) in &line {
                if auto_main { continue; }
                line_main += if is_row { w } else { h };
                line_cross = line_cross.max(if is_row { h } else { w });
                line_items += 1;
            }
            if line_items > 1 {
                line_main += item_spacing * (line_items - 1) as f32;
            }
            let line_main_offset = match axis_just_lc {
                "center" => ((avail_main - line_main) * 0.5).max(0.0),
                "end" | "right" | "bottom" => (avail_main - line_main).max(0.0),
                "start" if start_from_end => (avail_main - line_main).max(0.0),
                _ => 0.0,
            };
            let mut line_main_cursor = if is_row {
                container.x + line_main_offset
            } else {
                container.y + line_main_offset
            };

            for (id, w, h, auto_main) in line {
                if auto_main {
                    layout_node(
                        id,
                        container,
                        scene,
                        csx,
                        csy,
                        animation_sample_percent,
                        rects,
                        draw_order,
                    );
                    continue;
                }
                let rect = if is_row {
                    let mut x = line_main_cursor;
                    if let Some(node) = scene.nodes.get(&id) {
                        if let Some(anchor_offset) = row_flex_start_anchor_offset(node, avail_main, w, csx) {
                            x += anchor_offset;
                        }
                    }
                    let y = match cross_just.to_ascii_lowercase().as_str() {
                        "center" => line_cross_cursor + (line_cross - h) * 0.5,
                        "end" | "right" | "bottom" => line_cross_cursor + (line_cross - h),
                        _ => line_cross_cursor,
                    };
                    let ch = if cross_just.eq_ignore_ascii_case("stretch") { line_cross } else { h };
                    Rect { x, y, w, h: ch }
                } else {
                    let mut x = match cross_just.to_ascii_lowercase().as_str() {
                        "center" => line_cross_cursor + (line_cross - w) * 0.5,
                        "end" | "right" | "bottom" => line_cross_cursor + (line_cross - w),
                        "start" if cross_start_from_end => line_cross_cursor + (line_cross - w),
                        _ => line_cross_cursor,
                    };
                    if let Some(node) = scene.nodes.get(&id) {
                        let pos_x = (node.position.x + node.position_offset.x) * csx;
                        if cross_just.eq_ignore_ascii_case("center") {
                            x += (node.anchor.x * line_cross) + pos_x - (node.pivot.x * w);
                        } else if cross_just.eq_ignore_ascii_case("start") && !cross_start_from_end {
                            x += (node.anchor.x * line_cross) + pos_x - (node.pivot.x * w);
                        }
                    }
                    let cw = if cross_just.eq_ignore_ascii_case("stretch") { line_cross } else { w };
                    Rect { x, y: line_main_cursor, w: cw, h }
                };
                layout_node_with_rect(
                    id,
                    rect,
                    scene,
                    csx,
                    csy,
                    animation_sample_percent,
                    rects,
                    draw_order,
                );
                line_main_cursor += if is_row { w } else { h };
                line_main_cursor += item_spacing;
            }

            line_cross_cursor += line_cross + cross_spacing;
        }
}

fn centered_intrinsic_text_column_spacing_applies(
    children: &[BbNodeId],
    flex: &serde_json::Value,
    scene: &BbScene,
    is_row: bool,
) -> bool {
    if is_row {
        return false;
    }
    let direction = flex.get("direction").and_then(|v| v.as_str()).unwrap_or("Row");
    let wrap = flex.get("wrap").and_then(|v| v.as_str()).unwrap_or("");
    let axis = flex.get("axisJustification").and_then(|v| v.as_str()).unwrap_or("Start");
    if !direction.eq_ignore_ascii_case("Column")
        || !wrap.eq_ignore_ascii_case("NoWrapInfinite")
        || !axis.eq_ignore_ascii_case("Center")
    {
        return false;
    }

    let intrinsic_textfields = children
        .iter()
        .filter_map(|id| scene.nodes.get(id))
        .filter(|node| node.is_active && matches!(node.ty, BbNodeType::WidgetTextField))
        .filter(|node| {
            textfield_auto_intrinsic_override(node, &node.sizing.height, 1.0, 1.0, false)
                .is_some()
        })
        .count();
    intrinsic_textfields >= 2
}

fn is_placeholder_only_label_caption_pair(node: &BbNode) -> bool {
    if !matches!(
        &node.ty,
        BbNodeType::Other(kind)
            if kind.eq_ignore_ascii_case("BuildingBlocks_ComponentLabelCaptionPair")
    ) {
        return false;
    }

    let caption = node
        .raw
        .get("captionProperties")
        .and_then(|cp| cp.get("caption"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or_default();
    if !caption.eq_ignore_ascii_case("@LOC_PLACEHOLDER") {
        return false;
    }

    let secondary_caption = node
        .raw
        .get("captionProperties")
        .and_then(|cp| cp.get("caption2"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or_default();

    secondary_caption.is_empty() || secondary_caption.eq_ignore_ascii_case("@LOC_PLACEHOLDER")
}

fn centered_intrinsic_text_column_adjustment(
    base_item_spacing: f32,
    sizes: &[(BbNodeId, f32, f32, bool)],
    scene: &BbScene,
    container_main: f32,
    is_row: bool,
) -> (f32, f32) {
    if is_row || sizes.len() < 2 || base_item_spacing <= 0.0 {
        return (base_item_spacing, 0.0);
    }

    let flow_count = sizes.len() as f32;
    let mut intrinsic_textfields = 0.0f32;
    let mut total_item_main = 0.0f32;
    for (child_id, w, h, _auto_main) in sizes {
        let node_main = if is_row { *w } else { *h };
        total_item_main += node_main.max(0.0);
        let Some(node) = scene.nodes.get(child_id) else {
            continue;
        };
        if !node.is_active || !matches!(node.ty, BbNodeType::WidgetTextField) {
            continue;
        }
        if textfield_auto_intrinsic_override(node, &node.sizing.height, 1.0, 1.0, false).is_some() {
            intrinsic_textfields += 1.0;
        }
    }
    if intrinsic_textfields < 2.0 {
        return (base_item_spacing, 0.0);
    }

    // Derive spacing compression from how much of the column is occupied by
    // intrinsic content and how text-dominant the flow is.
    let coverage = if container_main > 0.0 {
        (total_item_main / container_main).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let textfield_share = (intrinsic_textfields / flow_count).clamp(0.0, 1.0);
    let spacing_scale = (1.0 - (coverage * textfield_share)).clamp(0.0, 1.0);
    let adjusted_spacing = (base_item_spacing * spacing_scale).max(0.0);

    // Keep centered columns from drifting downward as spacing shrinks by
    // shifting upward in proportion to collapsed spacing across text blocks.
    let collapsed_spacing = (base_item_spacing - adjusted_spacing).max(0.0);
    let main_offset_shift = collapsed_spacing * (intrinsic_textfields - 1.0).max(0.0);

    (adjusted_spacing, main_offset_shift)
}

pub(crate) fn row_flex_start_anchor_offset(
    node: &crate::bb_scene::BbNode,
    available_main: f32,
    item_w: f32,
    csx: f32,
) -> Option<f32> {
    let pos_x = (node.position.x + node.position_offset.x) * csx;
    let is_start_anchored = node.anchor.x > 0.0 && node.anchor.x < 0.5 && node.pivot.x <= 0.01;
    let has_position = pos_x.abs() > f32::EPSILON;
    (is_start_anchored || has_position)
        .then_some((node.anchor.x * available_main) + pos_x - (node.pivot.x * item_w))
}


///
/// - `Fixed(v)` → `v * canvas_scale`  
/// - `Percent(p)` → `primary_dim * p` (p is a fraction 0–1; value `1.0` = 100 %)
/// - `Other` with a recognised cross-axis behavior (`PercentOfY` for a width
///   dimension, `PercentOfX` for height) → `cross_dim * value`.  All other
///   unknown behaviors log a warning and fall back to `primary_dim` (fill).
fn resolve_value(v: &BbValue, primary_dim: f32, cross_dim: f32, canvas_scale: f32, is_width: bool) -> f32 {
    match v {
        BbValue::Fixed(px) => px * canvas_scale,
        BbValue::Percent(p) => primary_dim * p,
        BbValue::Other { value, behavior } => {
            match behavior.as_str() {
                // Cross-axis percent: width as % of parent height, or vice-versa.
                "PercentOfY" if is_width => cross_dim * value,
                "PercentOfX" if !is_width => cross_dim * value,
                // "Auto" is overloaded in authored data:
                // - values in (0, 1] often behave like normalized extents,
                //   especially for flex/header containers.
                // - larger values are content hints but we currently lack
                //   robust measurement, so keep prior fill behavior there.
                "Auto" if *value > 0.0 && *value <= 1.0 => primary_dim * *value,
                "Auto" => primary_dim,
                other => {
                    warn!(
                        "bb_layout: unknown sizing behavior {:?} (value={}) — \
                         falling back to fill",
                        other, value,
                    );
                    primary_dim
                }
            }
        }
    }
}

/// Return true when the value represents a "tiny" fixed dimension that should
/// be expanded to fill the parent for image/icon nodes.  Values below 2 canvas
/// units are almost certainly authoring artefacts (e.g. placeholder 0.7×0.4
/// rects) rather than intentional sizing, and should be treated as fill.
fn is_tiny_fixed_value(v: &BbValue, primary_dim: f32, canvas_scale: f32) -> bool {
    if let BbValue::Fixed(px) = v {
        let canvas_units = px / canvas_scale;
        canvas_units > 0.0 && canvas_units < 2.0 && canvas_units < primary_dim * 0.1
    } else {
        false
    }
}

fn resolve_value_for_node(
    node: &crate::bb_scene::BbNode,
    v: &BbValue,
    primary_dim: f32,
    cross_dim: f32,
    canvas_scale: f32,
    is_width: bool,
) -> f32 {
    if let Some(override_value) = textfield_auto_intrinsic_override(node, v, primary_dim, canvas_scale, is_width) {
        return override_value;
    }

    if let Some(override_value) = component_button_auto_intrinsic_override(node, v, canvas_scale) {
        return override_value;
    }

    if matches!(node.ty, BbNodeType::WidgetText)
        && let BbValue::Other { value, behavior } = v
        && behavior == "Auto"
    {
        // Text widgets authored with Auto typically carry a small content-fit
        // hint (for example 64). Treat that hint as size instead of filling
        // the parent, which causes header/label overlap in medical canvases.
        *value * canvas_scale
    } else if matches!(node.ty, BbNodeType::WidgetTextField)
        && matches!(v, BbValue::Other { value, behavior } if behavior == "Auto" && *value > 1.0)
        && (if is_width { node.anchor.x } else { node.anchor.y }) > 1.0
        && let Some((text_w, text_h)) =
            node_resolved_text_size(node, canvas_scale, &crate::text::TextRenderer::new())
    {
        // An Auto CONTENT-HINT (>1) textfield ANCHORED BEYOND the parent edge
        // (anchor > 1.0 on this axis) hangs outside its parent — filling is
        // meaningless there, so it sizes to its measured text. The heat
        // gauge's `CelsiusSymbol` (anchor y 1.02, pivot y 1.0, Auto 64) hangs
        // below the bar; the fill fallback stretched it over the gauge,
        // painting "ºC" mid-bar. In-parent Auto-hint textfields keep the
        // long-verified fill behaviour the gold baselines pin.
        if is_width { text_w } else { text_h }
    } else if matches!(node.ty, BbNodeType::WidgetImage | BbNodeType::WidgetIcon)
        && is_tiny_fixed_value(v, primary_dim, canvas_scale)
    {
        // Image/icon nodes with tiny fixed dimensions (e.g. 0.7×0.4 canvas
        // units) are almost certainly authoring artefacts — placeholder rects
        // that should expand to fill the parent container.  Treat them as
        // fill rather than rendering a 1-pixel sliver.
        primary_dim
    } else {
        resolve_value(v, primary_dim, cross_dim, canvas_scale, is_width)
    }
}

fn authored_node_scale(node: &crate::bb_scene::BbNode, scene: &BbScene) -> (f32, f32) {
    if matches!(node.ty, BbNodeType::WidgetCanvas)
        && node
            .parent
            .and_then(|parent_id| scene.nodes.get(&parent_id))
            .is_some_and(|parent| matches!(parent.ty, BbNodeType::WidgetTextField))
    {
        return (1.0, 1.0);
    }

    let Some(scale) = node.raw.get("scale") else {
        return (1.0, 1.0);
    };
    let x = scale
        .get("x")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    let y = scale
        .get("y")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    (x, y)
}

fn component_button_auto_intrinsic_override(
    node: &crate::bb_scene::BbNode,
    v: &BbValue,
    canvas_scale: f32,
) -> Option<f32> {
    let is_supported_node = matches!(
        node.ty,
        BbNodeType::ComponentGeneralButton | BbNodeType::ComponentGeneralButtonSecondary
    );

    if !is_supported_node {
        return None;
    }

    let (value, behavior) = match v {
        BbValue::Other { value, behavior } => (*value, behavior.as_str()),
        _ => return None,
    };
    if behavior != "Auto" || value <= 1.0 {
        return None;
    }

    Some(value * canvas_scale)
}

fn textfield_auto_intrinsic_override(
    node: &crate::bb_scene::BbNode,
    v: &BbValue,
    primary_dim: f32,
    canvas_scale: f32,
    is_width: bool,
) -> Option<f32> {
    if !matches!(node.ty, BbNodeType::WidgetTextField) {
        return None;
    }

    if is_width
        && node.pivot.x >= 0.99
        && node.anchor.x > 1.0
        && let BbValue::Other { value, behavior } = v
        && behavior == "Auto"
        && *value > 0.0
        && *value <= 1.0
    {
        return Some(primary_dim);
    }

    let (value, behavior) = match v {
        BbValue::Other { value, behavior } => (*value, behavior.as_str()),
        _ => return None,
    };
    if behavior != "Auto" || value <= 0.0 || value > 1.0 {
        return None;
    }

    let style = node
        .raw
        .get("labelProperties")
        .and_then(|lp| lp.get("style"))
        .and_then(|s| s.as_str())
        .unwrap_or("");

    let has_tag = |needle: &str| node.style_tag_uuids.iter().any(|id| id.eq_ignore_ascii_case(needle));
    let is_primary = has_tag("e6003a83-9795-4478-a61c-349f14016e5b");
    let is_bright = has_tag("174b3e40-1b7b-4f01-a7dc-6420b7367d6b");
    let is_prompt = has_tag("5e5c7c8f-847b-46c5-ad80-a57c941391ab");
    let affects_layout = node
        .raw
        .get("affectsLayout")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if is_width && is_bright && style == "Heading2" {
        return Some(180.0 * canvas_scale);
    }
    if is_width && !affects_layout && is_bright && style == "Title3" {
        return Some((180.0 * value * value) * canvas_scale);
    }

    if !is_width && style == "Title3" && is_primary {
        return Some(270.0 * canvas_scale);
    }
    if !is_width && style == "Heading2" && is_bright {
        return Some(270.0 * canvas_scale);
    }
    if !is_width && style == "Heading2" && is_prompt {
        return Some(60.0 * canvas_scale);
    }

    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Wireframe renderer
// ─────────────────────────────────────────────────────────────────────────────

/// Render a debug wireframe overlay of `scene` at `(target_w × target_h)`.
///
/// Each laid-out node is drawn as:
/// - A filled rect with a per-type colour at ~30 % alpha.
/// - A 1-pixel white outline at ~80 % alpha.
///
/// The background is `#202020`.  Inactive nodes are not drawn.
///
/// The colour is derived by hashing the node's type-name string to an HSV hue,
/// so no hard-coded type→colour table is needed.
pub fn render_wireframe(scene: &BbScene, target_w: u32, target_h: u32) -> RgbaImage {
    let result = layout(scene, target_w, target_h);

    // hardcoding-guard: synthetic — diagnostic wireframe canvas grey, not game data
    let mut img = RgbaImage::from_pixel(target_w, target_h, Rgba([0x20, 0x20, 0x20, 0xFF]));

    for &node_id in &result.draw_order {
        let Some(node) = scene.nodes.get(&node_id) else { continue };
        let Some(&outer) = result.rects.get(&node_id) else { continue };

        let type_name = type_name_str(&node.ty);
        let fill_colour = type_colour_fill(type_name);
        let outline_colour = Rgba([0xFF, 0xFF, 0xFF, (0.80 * 255.0) as u8]);

        draw_rect_filled(&mut img, outer, fill_colour);
        draw_rect_outline(&mut img, outer, outline_colour);
    }

    img
}

fn type_name_str(ty: &BbNodeType) -> &str {
    match ty {
        BbNodeType::DisplayWidget => "DisplayWidget",
        BbNodeType::WidgetCanvas => "WidgetCanvas",
        BbNodeType::WidgetIcon => "WidgetIcon",
        BbNodeType::WidgetCard => "WidgetCard",
        BbNodeType::WidgetTextField => "WidgetTextField",
        BbNodeType::ComponentGeneralButton => "ComponentGeneralButton",
        BbNodeType::ComponentGeneralButtonSecondary => "ComponentGeneralButtonSecondary",
        BbNodeType::WidgetImage => "WidgetImage",
        BbNodeType::WidgetText => "WidgetText",
        BbNodeType::WidgetCustomShape => "WidgetCustomShape",
        BbNodeType::WidgetBodyBackground => "WidgetBodyBackground",
        BbNodeType::Other(s) => {
            s.strip_prefix("BuildingBlocks_").unwrap_or(s.as_str())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::bb_scene::parse_bb_canvas;

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/canvas/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
    }

    /// R5.I-A: `PercentOfX` height (and `PercentOfY` width) must be evaluated
    /// against THIS NODE's OWN other-axis dimension, not the parent's other
    /// dimension.
    #[test]
    fn percent_of_x_uses_own_width_not_parent() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget, name: "parent".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(200.0), height: BbValue::Fixed(400.0) },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 2, parent: Some(1), children: vec![],
            ty: BbNodeType::WidgetIcon, name: "icon".into(),
            style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
            position: Vec3::default(), position_offset: Vec3::default(),
            sizing: BbSizing {
                width: BbValue::Percent(0.8),
                height: BbValue::Other { value: 1.0, behavior: "PercentOfX".into() },
            },
            padding: BbTrbl::default(), margin: BbTrbl::default(),
            pivot: Vec2::default(), anchor: Vec2::default(),
            background: None, border: None, radial: None, text: None, icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 400.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 400);
        let r = result.rects[&2];
        // width = 0.8 × parent_w(200) = 160
        // height = 1.0 × own_w(160)   = 160   (NOT 1.0 × parent_w(200) = 200 or × parent_h(400))
        assert!((r.w - 160.0).abs() < 0.5, "expected width ≈ 160, got {}", r.w);
        assert!((r.h - 160.0).abs() < 0.5, "expected height ≈ 160 (square), got {}", r.h);
    }

    #[test]
    fn texture_body_background_fills_canvas_surface() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let background = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetBodyBackground,
            name: "body background".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(64.0), height: BbValue::Fixed(64.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "backgroundType": "Texture",
                "textureProperties": {
                    "_Type_": "BuildingBlocks_ComponentTextureProperties",
                    "orientation": "Landscape"
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, background);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (800.0, 450.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 800, 450);
        let rect = result.rects[&1];
        assert_eq!(rect, Rect { x: 0.0, y: 0.0, w: 800.0, h: 450.0 });
    }

    /// A `useRaw` 16:9 canvas in a SQUARE target LETTERBOXES (uniform contain +
    /// centre → 1920×1080 at y=420), while the same canvas under
    /// `AspectOverridesWidth` FILLS the target (root rect = full target; content
    /// reflows and any overflow crops). The g-force / velocity ball gauges are
    /// authored `useRaw` but their cockpit screen MESH is square, so the pipeline
    /// forces `AspectOverridesWidth` when the per-screen mesh aspect is applied —
    /// the ball fills the square instead of letterboxing and the readouts overflow
    /// off-screen. This guards both branches of the canvas-rect rule.
    #[test]
    fn aspect_overrides_fills_square_target_where_useraw_letterboxes() {
        use crate::bb_scene::{BbCoordinateMethod, BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};
        let mk_scene = |method: BbCoordinateMethod| {
            let root = BbNode {
                id: 1, parent: None, children: vec![],
                ty: BbNodeType::WidgetCanvas, name: "root".into(),
                style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
                position: Vec3::default(), position_offset: Vec3::default(),
                sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
                padding: BbTrbl::default(), margin: BbTrbl::default(),
                pivot: Vec2::default(), anchor: Vec2::default(),
                background: None, border: None, radial: None, text: None, icon: None,
                raw: serde_json::Value::Null,
            };
            let mut nodes = BTreeMap::new();
            nodes.insert(1, root);
            BbScene { coordinate_method: method, canvas_size: (1920.0, 1080.0), roots: vec![1], nodes, operations: vec![] }
        };
        let raw = layout(&mk_scene(BbCoordinateMethod::UseRaw), 1920, 1920).rects[&1];
        assert!(
            (raw.h - 1080.0).abs() < 1.0 && (raw.y - 420.0).abs() < 1.0,
            "useRaw must letterbox the 16:9 canvas in a square target, got {raw:?}"
        );
        let fill = layout(&mk_scene(BbCoordinateMethod::AspectOverridesWidth), 1920, 1920).rects[&1];
        assert_eq!(
            fill,
            Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1920.0 },
            "AspectOverrides must fill the square target"
        );
    }

    /// `coordinateMethod: "auto"` FILLS the target (non-uniform, like
    /// `aspectOverrides*`), NOT the uniform contain/cover of `useRaw`. The Clipper
    /// compass authors `auto` on a WIDE (3.27) screen with a 16:9 canvas; under the
    /// `useRaw` uniform fit the 1080-tall canvas overflows the 587 target and its
    /// bottom-anchored tick band falls off-screen, whereas `auto` fills the target
    /// so the ticks span the width and fit the height. No frozen cockpit screen is
    /// `auto` (they are `useRaw` / `aspectOverridesWidth`), so this is compass-only.
    #[test]
    fn auto_coordinate_method_fills_target_unlike_useraw() {
        use crate::bb_scene::{BbCoordinateMethod, BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};
        let mk_scene = |method: BbCoordinateMethod| {
            let root = BbNode {
                id: 1, parent: None, children: vec![],
                ty: BbNodeType::WidgetCanvas, name: "root".into(),
                style_tag_uuids: vec![], is_active: true, layer: 0, alpha: 1.0,
                position: Vec3::default(), position_offset: Vec3::default(),
                sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
                padding: BbTrbl::default(), margin: BbTrbl::default(),
                pivot: Vec2::default(), anchor: Vec2::default(),
                background: None, border: None, radial: None, text: None, icon: None,
                raw: serde_json::Value::Null,
            };
            let mut nodes = BTreeMap::new();
            nodes.insert(1, root);
            BbScene { coordinate_method: method, canvas_size: (1920.0, 1080.0), roots: vec![1], nodes, operations: vec![] }
        };
        // Compass strip: a 16:9 canvas in a wide 1920×587 target.
        let auto = layout(&mk_scene(BbCoordinateMethod::Auto), 1920, 587).rects[&1];
        assert_eq!(
            auto,
            Rect { x: 0.0, y: 0.0, w: 1920.0, h: 587.0 },
            "auto must FILL the wide target, got {auto:?}"
        );
        let raw = layout(&mk_scene(BbCoordinateMethod::UseRaw), 1920, 587).rects[&1];
        assert!(
            raw.w < 1100.0 && raw.x > 1.0,
            "useRaw must letterbox (uniform contain), not fill, got {raw:?}"
        );
    }

    /// `cover_fit_recentre` centres the gauge BALL-AREA — the largest LOCALISED
    /// node (excludes the full-bleed background/root) — clamped to keep coverage.
    /// Models the VELOCITY ball: a cover-scaled 3413×1920 canvas with the ball-area
    /// (1920², larger than the readouts panel) at canvas-RIGHT. The ball must move
    /// to screen centre; the full-canvas background must NOT be chosen; the leading
    /// edge must not gap (dx ≤ 0).
    #[test]
    fn cover_fit_recentre_centres_ball_area_velocity_mirror() {
        let mut rects: BTreeMap<BbNodeId, Rect> = BTreeMap::new();
        // node 1: full-bleed background/root 3413×1920 — must be EXCLUDED.
        rects.insert(1, Rect { x: 0.0, y: 0.0, w: 3413.0, h: 1920.0 });
        // node 2: ball-area at canvas-RIGHT (centre x = 2453), the largest localised.
        rects.insert(2, Rect { x: 1493.0, y: 0.0, w: 1920.0, h: 1920.0 });
        // node 3: readouts panel (smaller) at canvas-LEFT.
        rects.insert(3, Rect { x: 0.0, y: 384.0, w: 1615.0, h: 1152.0 });
        cover_fit_recentre(&mut rects, |_| true, 1920.0, 1920.0, 3413.0, 1920.0);
        let ball = rects[&2];
        assert!((ball.x + ball.w * 0.5 - 960.0).abs() <= 1.0, "ball-area must centre, got {ball:?}");
        assert!(rects[&1].x <= 0.0 && rects[&1].x + 3413.0 >= 1920.0, "canvas must still cover target");
    }

    /// The MIRROR — g-force: a ball-area at canvas-LEFT (and NOT square —
    /// 1795×1920, the horizontal layout constrains its width) wants a POSITIVE
    /// shift toward centre. The full-bleed background (snapped to the viewport by
    /// `cover_fit_full_bleed_to_viewport`, applied right after) fills the screen
    /// independently, so centring the ball-area exposes no gap — it CENTRES
    /// (the in-game g-force ball is centred; it rendered ~63px left before this).
    /// Clamped only to keep the ball-area on-screen. Proves the rule is size-based
    /// (not square-based, which previously mis-picked the coincidentally-square
    /// readouts container) and generic for both mirrored layouts.
    #[test]
    fn cover_fit_recentre_left_ball_centres() {
        let mut rects: BTreeMap<BbNodeId, Rect> = BTreeMap::new();
        rects.insert(1, Rect { x: 0.0, y: 0.0, w: 3413.0, h: 1920.0 });
        // non-square ball-area at canvas-left (centre x = 897.5), largest localised.
        rects.insert(2, Rect { x: 0.0, y: 0.0, w: 1795.0, h: 1920.0 });
        rects.insert(3, Rect { x: 1795.0, y: 384.0, w: 1615.0, h: 1152.0 });
        cover_fit_recentre(&mut rects, |_| true, 1920.0, 1920.0, 3413.0, 1920.0);
        let ball = rects[&2];
        assert!(
            (ball.x + ball.w * 0.5 - 960.0).abs() <= 1.0,
            "left ball-area must centre, got {ball:?}"
        );
    }

    /// An aspectOverrides screen (door / annunciator) lays its canvas out at the
    /// FULL target (canvas == target on both axes), so there is no overflow to
    /// re-frame: cover_fit_recentre must be a NO-OP. Centring its largest content
    /// node would push the door/annunciator content off-place (a regression the
    /// whole-image visual guard caught). Only an axis whose cover-scaled canvas
    /// exceeds the target shifts.
    #[test]
    fn cover_fit_recentre_full_target_canvas_does_not_shift() {
        let mut rects: BTreeMap<BbNodeId, Rect> = BTreeMap::new();
        // canvas == target (aspectOverrides fill): no overflow on either axis.
        rects.insert(1, Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1132.0 });
        // a localised content node, NOT centred (a header strip near the top).
        rects.insert(2, Rect { x: 0.0, y: 0.0, w: 1920.0, h: 200.0 });
        cover_fit_recentre(&mut rects, |_| true, 1920.0, 1132.0, 1920.0, 1132.0);
        assert_eq!(
            rects[&2].y, 0.0,
            "full-target (aspectOverrides) canvas must not be re-centred vertically"
        );
        assert_eq!(rects[&2].x, 0.0, "nor horizontally");
    }

    /// An OFF-CANVAS animation overlay must never be chosen as the ball-area.
    /// Models the COUNTERMEASURES screen: a cover-scaled 3413×1920 canvas whose
    /// largest localised rect by area is the hold-to-fire circle overlay
    /// (`card_CountermeasureFireHoldCirlce`, 2304²), parked far off-canvas
    /// (centre x ≈ −2922) by an authored pivot of x=22 — at rest it sits outside
    /// the canvas and slides in only while firing. Centring on it shoves all the
    /// real content (the two countermeasure panels) +3882px off the right edge,
    /// blanking the screen. The recentre must instead ignore it and centre the
    /// on-canvas content, so the canvas keeps covering the target.
    #[test]
    fn cover_fit_recentre_ignores_off_canvas_overlay() {
        let mut rects: BTreeMap<BbNodeId, Rect> = BTreeMap::new();
        // node 1: full-canvas background/root 3413×1920 — must keep covering target.
        rects.insert(1, Rect { x: 0.0, y: 0.0, w: 3413.0, h: 1920.0 });
        // node 2: real content (countermeasure list), localised, centred in canvas
        // (centre x = 1706.5) — the largest ON-canvas localised node.
        rects.insert(2, Rect { x: 341.5, y: 0.0, w: 2730.0, h: 1920.0 });
        // node 3: hold-to-fire overlay — LARGER (2304² > 2730×1920) but off-canvas
        // (centre x = −2922), parked there by the authored pivot.x=22.
        rects.insert(3, Rect { x: -4074.0, y: -192.0, w: 2304.0, h: 2304.0 });
        cover_fit_recentre(&mut rects, |_| true, 1920.0, 1920.0, 3413.0, 1920.0);
        assert!(
            rects[&1].x <= 0.0 && rects[&1].x + 3413.0 >= 1920.0,
            "off-canvas overlay must not be centred; canvas must cover target, got {:?}",
            rects[&1]
        );
        let content = rects[&2];
        assert!(
            (content.x + content.w * 0.5 - 960.0).abs() <= 1.0,
            "on-canvas content must centre, got {content:?}"
        );
    }

    #[test]
    fn sampled_size_animation_overrides_sizing_value() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let parent = BbNode {
            id: 1,
            parent: None,
            children: vec![2, 3],
            ty: BbNodeType::DisplayWidget,
            name: "parent".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(400.0), height: BbValue::Fixed(300.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::WidgetCustomShape,
            name: "animated shape".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(0.85), height: BbValue::Percent(0.85) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "animation": {
                    "animationTimeline": {
                        "keyframes": [
                            {
                                "percent": 0.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 0.77}},
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeY", "value": 0.77}}
                                ]
                            },
                            {
                                "percent": 1.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeX", "value": 0.9}},
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "SizeY", "value": 0.9}}
                                ]
                            }
                        ]
                    }
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, parent);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (400.0, 300.0), roots: vec![1], nodes, operations: vec![] };

        let static_result = layout(&scene, 400, 300);
        let sampled_result = layout_with_animation_sample(&scene, 400, 300, Some(50.0), false, false);
        assert!((static_result.rects[&2].w - 340.0).abs() < 0.5);
        assert!((sampled_result.rects[&2].w - 334.0).abs() < 0.5);
        assert!((sampled_result.rects[&2].h - 250.5).abs() < 0.5);
    }

    #[test]
    fn sampled_position_offset_animation_moves_rect() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetImage,
            name: "animated image".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "animation": {
                    "animationTimeline": {
                        "keyframes": [
                            {
                                "percent": 0.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 50.0}}
                                ]
                            },
                            {
                                "percent": 1.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 0.0}}
                                ]
                            }
                        ]
                    }
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout_with_animation_sample(&scene, 200, 100, Some(50.0), false, false);
        let rect = result.rects[&1];
        assert!((rect.x - 25.0).abs() < 0.5, "expected sampled PosXOffset x 25, got {}", rect.x);
    }

    #[test]
    fn alternate_reverse_position_animation_samples_at_midpoint() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let node = BbNode {
            id: 1,
            parent: None,
            children: vec![],
            ty: BbNodeType::WidgetImage,
            name: "looping slide image".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(100.0), height: BbValue::Fixed(50.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "animation": {
                    "animationTimeline": {
                        "keyframes": [
                            {
                                "percent": 0.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 50.0}}
                                ]
                            },
                            {
                                "percent": 1.0,
                                "modifiers": [
                                    {"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "PosXOffset", "value": 0.0}}
                                ]
                            }
                        ]
                    },
                    "direction": "AlternateReverse",
                    "loopIndefinitely": true,
                    "additive": true
                }
            }),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, node);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout_with_animation_sample(&scene, 200, 100, Some(50.0), false, false);
        let rect = result.rects[&1];
        // The hand-tuned 2/3 phase ratio was deleted (remediation plan
        // Phase 4 audit: no frozen pin referenced it) — additive
        // AlternateReverse loops sample at the requested midpoint like
        // every other animation.
        assert!((rect.x - 25.0).abs() < 0.5, "expected midpoint-sampled PosXOffset x 25, got {}", rect.x);
    }

    #[test]
    fn full_fill_flex_container_preserves_authored_pivot() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let root = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.0, y: 0.03 },
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::json!({
                "layoutPolicy": {
                    "_Type_": "BuildingBlocks_FlexContainer",
                    "direction": "Row",
                    "wrap": "Wrap",
                    "axisJustification": "Start",
                    "crossAxisJustification": "Start"
                }
            }),
        };
        let child = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "child".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(20.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, root);
        nodes.insert(2, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let root_rect = result.rects[&1];
        let child_rect = result.rects[&2];
        assert!((root_rect.y + 3.0).abs() < 0.5, "expected root y -3, got {}", root_rect.y);
        assert!((child_rect.y + 3.0).abs() < 0.5, "expected child y -3, got {}", child_rect.y);
    }

    #[test]
    fn child_canvas_surface_root_uses_host_origin() {
        use crate::bb_scene::{BbNode, BbNodeType, BbSizing, BbTrbl, BbValue, Vec2, Vec3};

        let host = BbNode {
            id: 1,
            parent: None,
            children: vec![2],
            ty: BbNodeType::WidgetCanvas,
            name: "host_canvas".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(0.94) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.45 },
            anchor: Vec2 { x: 0.5, y: 0.5 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child_root = BbNode {
            id: 2,
            parent: Some(1),
            children: vec![3],
            ty: BbNodeType::DisplayWidget,
            name: "root".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Percent(1.0), height: BbValue::Percent(1.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2 { x: 0.5, y: 0.5 },
            anchor: Vec2 { x: 0.5, y: 0.4 },
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };
        let child = BbNode {
            id: 3,
            parent: Some(2),
            children: vec![],
            ty: BbNodeType::DisplayWidget,
            name: "child".into(),
            style_tag_uuids: vec![],
            is_active: true,
            layer: 0,
            alpha: 1.0,
            position: Vec3::default(),
            position_offset: Vec3::default(),
            sizing: BbSizing { width: BbValue::Fixed(50.0), height: BbValue::Fixed(20.0) },
            padding: BbTrbl::default(),
            margin: BbTrbl::default(),
            pivot: Vec2::default(),
            anchor: Vec2::default(),
            background: None,
            border: None,
            radial: None,
            text: None,
            icon: None,
            raw: serde_json::Value::Null,
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(1, host);
        nodes.insert(2, child_root);
        nodes.insert(3, child);
        let scene = BbScene { coordinate_method: crate::bb_scene::BbCoordinateMethod::UseRaw, canvas_size: (200.0, 100.0), roots: vec![1], nodes, operations: vec![] };

        let result = layout(&scene, 200, 100);
        let host_rect = result.rects[&1];
        let root_rect = result.rects[&2];
        assert!((host_rect.y - 0.0).abs() < 0.5, "expected host y 0, got {}", host_rect.y);
        assert!((root_rect.y - host_rect.y).abs() < 0.5, "expected child root y {}, got {}", host_rect.y, root_rect.y);
    }
}
