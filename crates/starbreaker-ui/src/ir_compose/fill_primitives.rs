//! Rect fill primitives for the IR compositor: square / uniform-rounded /
//! per-corner (radius+chamfer) path builders and their blend-mode fills
//! (split from the engine modules by responsibility — line-cap guard;
//! per-corner geometry = review ledger 106).

#[allow(unused_imports)]
use super::*;
use crate::colour::{blend_premul_add_linear, blend_premul_linear};
use tiny_skia::{BlendMode, Paint, Path, PathBuilder, Pixmap, Rect as TskRect, Stroke, Transform};

/// Render `draw` (a tiny-skia shape fill/stroke) into a transparent scratch
/// pixmap sized to `bounds` (padded for AA / stroke bleed), then composite the
/// scratch onto `dst` in LINEAR light, honouring `SourceOver` / `Plus`. The
/// closure receives a translation `Transform` mapping `bounds`'s origin to the
/// scratch's (0,0) so callers draw in absolute coordinates. Generalises the
/// white-mask carve-out to arbitrary tiny-skia draws so every AA edge blends in
/// linear, matching the engine.
pub(crate) fn fill_linear(
    dst: &mut Pixmap,
    bounds: TskRect,
    blend_mode: BlendMode,
    draw: impl FnOnce(&mut Pixmap, Transform),
) {
    const PAD: f32 = 2.0; // AA / stroke half-width bleed
    let x0 = (bounds.x() - PAD).floor().max(0.0) as u32;
    let y0 = (bounds.y() - PAD).floor().max(0.0) as u32;
    let x1 = ((bounds.x() + bounds.width() + PAD).ceil().max(0.0) as u32).min(dst.width());
    let y1 = ((bounds.y() + bounds.height() + PAD).ceil().max(0.0) as u32).min(dst.height());
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (sw, sh) = (x1 - x0, y1 - y0);
    let Some(mut scratch) = Pixmap::new(sw, sh) else {
        return;
    };
    draw(&mut scratch, Transform::from_translate(-(x0 as f32), -(y0 as f32)));

    let sd = scratch.data();
    let dw = dst.width();
    let dd = dst.data_mut();
    for ly in 0..sh {
        for lx in 0..sw {
            let si = ((ly * sw + lx) * 4) as usize;
            let s = [sd[si], sd[si + 1], sd[si + 2], sd[si + 3]];
            if s[3] == 0 && blend_mode != BlendMode::Plus {
                continue;
            }
            let di = (((y0 + ly) * dw + (x0 + lx)) * 4) as usize;
            let mut d = [dd[di], dd[di + 1], dd[di + 2], dd[di + 3]];
            match blend_mode {
                BlendMode::Plus => blend_premul_add_linear(&mut d, s),
                _ => blend_premul_linear(&mut d, s),
            }
            dd[di..di + 4].copy_from_slice(&d);
        }
    }
}

/// Stroke `path` into the linear scratch composite (bounds auto-inflated by the
/// stroke width). Companion to [`fill_linear`] for outline strokes so every
/// stroke AA edge blends in linear light.
pub(crate) fn stroke_linear(
    dst: &mut Pixmap,
    path: &Path,
    paint: &Paint,
    stroke: &Stroke,
    blend_mode: BlendMode,
) {
    let b = path.bounds();
    let w = stroke.width;
    let bounds = TskRect::from_xywh(b.x() - w, b.y() - w, b.width() + w * 2.0, b.height() + w * 2.0)
        .unwrap_or(b);
    fill_linear(dst, bounds, blend_mode, |scratch, tf| {
        scratch.as_mut().stroke_path(path, paint, stroke, tf, None);
    });
}

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
    fill_linear(pixmap, rect, blend_mode, |scratch, tf| {
        let mut paint = Paint::default();
        paint.set_color(to_skia_color(rgba, alpha));
        paint.blend_mode = BlendMode::SourceOver; // into transparent scratch
        paint.anti_alias = true;
        scratch
            .as_mut()
            .fill_path(&path, &paint, tiny_skia::FillRule::Winding, tf, None);
    });
}

pub(crate) fn fill_rect_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    fill_linear(pixmap, rect, blend_mode, |scratch, tf| {
        let mut paint = Paint::default();
        paint.set_color(to_skia_color(rgba, alpha));
        paint.blend_mode = BlendMode::SourceOver; // into transparent scratch
        paint.anti_alias = false;
        scratch.as_mut().fill_rect(rect, &paint, tf, None);
    });
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
    fill_linear(pixmap, rect, blend_mode, |scratch, tf| {
        let mut paint = Paint::default();
        paint.set_color(to_skia_color(rgba, alpha));
        paint.blend_mode = BlendMode::SourceOver; // into transparent scratch
        paint.anti_alias = true;
        scratch
            .as_mut()
            .fill_path(&path, &paint, tiny_skia::FillRule::Winding, tf, None);
    });
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

#[cfg(test)]
mod linear_fill_tests {
    use super::*;
    use tiny_skia::{BlendMode, Color, Pixmap, Rect as TskRect};

    // A 50%-alpha white rect over opaque black must land ~188 (linear), not 128 (sRGB).
    #[test]
    fn fill_rect_composites_in_linear_light() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        pm.fill(Color::from_rgba8(0, 0, 0, 255));
        let rect = TskRect::from_xywh(0.0, 0.0, 4.0, 4.0).unwrap();
        fill_rect_ts_with_mode(&mut pm, rect, [1.0, 1.0, 1.0, 1.0], 0.5, BlendMode::SourceOver);
        let px = pm.pixel(1, 1).unwrap();
        assert!(
            (px.red() as i32 - 188).abs() <= 3,
            "linear 50% white fill over black expected ~188, got {}",
            px.red()
        );
        assert!((px.red() as i32 - 128).abs() > 20, "must not be sRGB 128");
    }

    #[test]
    fn additive_fill_sums_in_linear() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        pm.fill(Color::from_rgba8(128, 128, 128, 255));
        let rect = TskRect::from_xywh(0.0, 0.0, 4.0, 4.0).unwrap();
        fill_rect_ts_with_mode(&mut pm, rect, [0.502, 0.502, 0.502, 1.0], 1.0, BlendMode::Plus);
        let px = pm.pixel(1, 1).unwrap();
        assert!(px.red() < 255 && px.red() > 150, "additive-in-linear expected ~178, got {}", px.red());
    }
}
