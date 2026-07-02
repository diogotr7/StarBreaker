//! Rect fill primitives for the IR compositor: square / uniform-rounded /
//! per-corner (radius+chamfer) path builders and their blend-mode fills
//! (split from the engine modules by responsibility — line-cap guard;
//! per-corner geometry = review ledger 106).

#[allow(unused_imports)]
use super::*;
use tiny_skia::{BlendMode, Paint, PathBuilder, Pixmap, Rect as TskRect, Transform};

pub(crate) fn fill_rounded_rect_ts_with_mode(
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

pub(crate) fn fill_rect_ts_with_mode(
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

/// Fill a rect with PER-CORNER radii/chamfers (`corner_geometry_path`,
/// ledger 106). Falls back to a square fill when the path degenerates.
pub(crate) fn fill_corner_geometry_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    radii: [f32; 4],
    chamfers: [bool; 4],
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let Some(path) = corner_geometry_path(rect, radii, chamfers) else {
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

// Rounded border chrome: the generic border renderer strokes a rounded-rect
// path when a node carries a uniform border and an authored corner radius
// (e.g. the modular-kit ghost button's `RootGhost` 3px Accent1 border with
// 6px corners). Non-uniform or square borders keep per-side fills.

pub(crate) fn rounded_rect_path(rect: TskRect, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.max(0.0).min(rect.width() * 0.5).min(rect.height() * 0.5);
    let x = rect.x();
    let y = rect.y();
    let w = rect.width();
    let h = rect.height();

    let mut pb = PathBuilder::new();
    // A FULL ellipse/circle — the corner radius consumes both half-extents — must
    // use CUBIC corner arcs (standard 90°-arc constant k = 4/3·tan(π/8) ≈ 0.5523,
    // accurate to <0.03% of r). A quadratic Bezier bulges toward the corner, so a
    // full-radius rounded rect rendered as a squircle, not a circle — the g-force
    // / velocity centre dot (114.86² node, corner_radius 100 clamped to half-size)
    // showed as a rounded square. PARTIAL rounded corners keep the quadratic arc:
    // the deviation there is sub-pixel (the cards/borders on the frozen MFD
    // baselines) and quadratics are what those baselines were frozen with.
    let is_full_ellipse = r >= w * 0.5 - 0.5 && r >= h * 0.5 - 0.5;
    if is_full_ellipse {
        const ARC_K: f32 = 0.552_284_75;
        let c = r * ARC_K;
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.cubic_to(x + w - r + c, y, x + w, y + r - c, x + w, y + r); // top-right
        pb.line_to(x + w, y + h - r);
        pb.cubic_to(x + w, y + h - r + c, x + w - r + c, y + h, x + w - r, y + h); // bottom-right
        pb.line_to(x + r, y + h);
        pb.cubic_to(x + r - c, y + h, x, y + h - r + c, x, y + h - r); // bottom-left
        pb.line_to(x, y + r);
        pb.cubic_to(x, y + r - c, x + r - c, y, x + r, y); // top-left
    } else {
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
    }
    pb.close();
    pb.finish()
}

/// Rect path with PER-CORNER geometry: radii in `[TL, TR, BR, BL]` order, a
/// chamfered corner drawing a straight cut of its radius instead of an arc
/// (the button standards' Filled state — ledger 106). Arcs use the same
/// quadratic form as [`rounded_rect_path`]'s partial-corner branch.
pub(crate) fn corner_geometry_path(
    rect: TskRect,
    radii: [f32; 4],
    chamfers: [bool; 4],
) -> Option<tiny_skia::Path> {
    let clamp = |r: f32| r.max(0.0).min(rect.width() * 0.5).min(rect.height() * 0.5);
    let [tl, tr, br, bl] = [clamp(radii[0]), clamp(radii[1]), clamp(radii[2]), clamp(radii[3])];
    let x = rect.x();
    let y = rect.y();
    let w = rect.width();
    let h = rect.height();

    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    if tr > 0.0 {
        if chamfers[1] {
            pb.line_to(x + w, y + tr);
        } else {
            pb.quad_to(x + w, y, x + w, y + tr);
        }
    }
    pb.line_to(x + w, y + h - br);
    if br > 0.0 {
        if chamfers[2] {
            pb.line_to(x + w - br, y + h);
        } else {
            pb.quad_to(x + w, y + h, x + w - br, y + h);
        }
    }
    pb.line_to(x + bl, y + h);
    if bl > 0.0 {
        if chamfers[3] {
            pb.line_to(x, y + h - bl);
        } else {
            pb.quad_to(x, y + h, x, y + h - bl);
        }
    }
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        if chamfers[0] {
            pb.line_to(x + tl, y);
        } else {
            pb.quad_to(x, y, x + tl, y);
        }
    }
    pb.close();
    pb.finish()
}
