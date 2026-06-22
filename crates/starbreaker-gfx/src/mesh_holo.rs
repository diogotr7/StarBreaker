//! CPU rasteriser for the SELF-STATUS vehicle hologram.
//!
//! Projects a decoded ship mesh (positions + triangle indices, model space) to
//! 2D and draws it as a **neutral greyscale** hologram: semi-transparent white
//! filled faces + white wireframe edges, composited back-to-front (painter's
//! algorithm) so overlapping translucent faces read as a see-through hologram.
//! A per-manufacturer `tint` (default white = neutral) is multiplied over the
//! greyscale result, so the colour is data-driven by the caller rather than
//! baked in here.
//!
//! The headless UI render pipeline (via `starbreaker-3d`) calls
//! [`render_vehicle_hologram`] to populate `WidgetRuntimeImage`/`Primitive`
//! own-vehicle nodes that the engine would otherwise render as a live 3D
//! primitive. Input is intentionally generic (raw positions + indices) so this
//! crate keeps depending only on `image` and has no knowledge of mesh formats.

use image::{Rgba, RgbaImage};

/// Camera and style parameters for [`render_vehicle_hologram`].
#[derive(Debug, Clone)]
pub struct HologramParams {
    /// Tilt back from straight top-down, in degrees. `0.0` is a pure top-down
    /// view; the SELF-STATUS reference is ~15° back so the upper rear is
    /// visible. Applied as a pitch about the model's right (X) axis.
    pub tilt_back_deg: f32,
    /// Yaw about the model's up (Z) axis, in degrees, to orient the nose in the
    /// image. `0.0` keeps the model's forward (+Y) pointing up in the frame.
    pub yaw_deg: f32,
    /// RGBA multiply tint applied to the neutral greyscale render, channels in
    /// `0.0..=1.0`. `[1,1,1,1]` is neutral (pure greyscale).
    pub tint: [f32; 4],
    /// Alpha of each semi-transparent filled face, `0.0..=1.0`.
    pub face_alpha: f32,
    /// Alpha of the wireframe edges, `0.0..=1.0`.
    pub wire_alpha: f32,
    /// Fraction of the smaller image dimension the projected model fills,
    /// `0.0..=1.0` (the rest is margin).
    pub fit: f32,
    /// Perspective camera distance in units of the model's projected radius.
    /// Smaller = stronger perspective (more 3D foreshortening — the nearer rear
    /// reads larger than the receding nose); large (≳ 10) is near-orthographic.
    /// Gives a flat ship a convincing angled-hologram look.
    pub perspective: f32,
    /// Index-array offset (a multiple of 3) at which appended shield-pane
    /// triangles begin. Triangles at or beyond it are filled at
    /// `face_alpha × shield_alpha_scale` so the shield faces read fainter than
    /// the hull. `None` = no shields (every triangle uses `face_alpha`).
    pub shield_index_start: Option<usize>,
    /// Alpha multiplier applied to shield-pane faces relative to hull faces
    /// (e.g. `0.5` = shields at half the hull's face alpha). Ignored when
    /// `shield_index_start` is `None`.
    pub shield_alpha_scale: f32,
}

impl Default for HologramParams {
    fn default() -> Self {
        Self {
            tilt_back_deg: 15.0,
            yaw_deg: 0.0,
            tint: [1.0, 1.0, 1.0, 1.0],
            face_alpha: 0.18,
            wire_alpha: 0.55,
            fit: 0.86,
            perspective: 2.5,
            shield_index_start: None,
            shield_alpha_scale: 1.0,
        }
    }
}

/// Render `positions`/`indices` (a triangle list, CryEngine Z-up / +Y-forward
/// model space) into a `width`×`height` RGBA hologram image with a transparent
/// background. Returns a fully transparent image when there is no drawable
/// geometry.
pub fn render_vehicle_hologram(
    positions: &[[f32; 3]],
    indices: &[u32],
    width: u32,
    height: u32,
    params: &HologramParams,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width.max(1), height.max(1), Rgba([0, 0, 0, 0]));
    if width == 0 || height == 0 || positions.is_empty() || indices.len() < 3 {
        return img;
    }

    // 1. Rotate every vertex into view space: yaw about Z (up), then pitch back
    //    about X (right). Top-down baseline maps model X→screen x, model Y
    //    (forward)→screen y, model Z (up)→depth toward the camera.
    let (sy, cy) = params.yaw_deg.to_radians().sin_cos();
    let (sp, cp) = params.tilt_back_deg.to_radians().sin_cos();
    let rotated: Vec<[f32; 3]> = positions
        .iter()
        .map(|&[x, y, z]| {
            // yaw about Z
            let x1 = x * cy - y * sy;
            let y1 = x * sy + y * cy;
            let z1 = z;
            // pitch about X (mix forward Y and up Z)
            let y2 = y1 * cp - z1 * sp;
            let z2 = y1 * sp + z1 * cp;
            [x1, y2, z2]
        })
        .collect();

    // 1b. Perspective divide. Place the camera in front of the nearest point
    //     along the view depth axis; dividing x/y by (cam − depth) makes the
    //     nearer rear read larger than the receding nose, so a flat ship gains a
    //     genuine 3D angle instead of a foreshortened-but-flat look. The depth
    //     `z` is preserved (in `rotated`) for normals and the painter's sort.
    let mut zmin = f32::MAX;
    let mut zmax = f32::MIN;
    let (mut rx0, mut ry0, mut rx1, mut ry1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for v in &rotated {
        zmin = zmin.min(v[2]);
        zmax = zmax.max(v[2]);
        rx0 = rx0.min(v[0]);
        ry0 = ry0.min(v[1]);
        rx1 = rx1.max(v[0]);
        ry1 = ry1.max(v[1]);
    }
    let radius = ((rx1 - rx0).max(ry1 - ry0).max(zmax - zmin)) * 0.5;
    let cam_z = zmax + params.perspective.max(0.1) * radius.max(1e-3);
    // Focal ≈ camera distance keeps the centre scale near 1 before the fit.
    let focal = cam_z;
    let proj: Vec<[f32; 2]> = rotated
        .iter()
        .map(|v| {
            let denom = (cam_z - v[2]).max(1e-3);
            [v[0] * focal / denom, v[1] * focal / denom]
        })
        .collect();

    // 2. Fit about the model ORIGIN (which projects to (0, 0), so the origin
    //    lands at the image centre). Two competing constraints, take the smaller
    //    scale of the two — this is what makes the framing GENERIC across hulls of
    //    any aspect:
    //      • HULL fit: the hull's longest axis fills `fit` of the frame (the
    //        shields are excluded so they don't shrink the hull).
    //      • BOX fit: the hull + shields must stay inside `BOX_FIT` of the frame
    //        so the shield cage never clips — binds only when the shields would
    //        otherwise overflow (e.g. a hull whose aspect differs from the frame).
    //    For a hull that frames comfortably the HULL fit wins (target size); for
    //    an awkward aspect the BOX fit wins (hull shrinks so the shields fit).
    let shield_start = params.shield_index_start.unwrap_or(usize::MAX);
    let hull_end = shield_start.min(indices.len());
    let (mut hull_x, mut hull_y) = (1e-6_f32, 1e-6_f32);
    let (mut box_x, mut box_y) = (1e-6_f32, 1e-6_f32);
    for p in &proj {
        box_x = box_x.max(p[0].abs());
        box_y = box_y.max(p[1].abs());
    }
    if hull_end >= 3 {
        for &vi in &indices[0..hull_end] {
            if let Some(p) = proj.get(vi as usize) {
                hull_x = hull_x.max(p[0].abs());
                hull_y = hull_y.max(p[1].abs());
            }
        }
    } else {
        hull_x = box_x;
        hull_y = box_y;
    }
    let fit = params.fit.clamp(0.05, 1.0);
    const BOX_FIT: f32 = 0.98; // shields stay within 98% of the frame (never clip)
    let scale = ((width as f32 * fit * 0.5) / hull_x)
        .min((height as f32 * fit * 0.5) / hull_y)
        .min((width as f32 * BOX_FIT * 0.5) / box_x)
        .min((height as f32 * BOX_FIT * 0.5) / box_y);
    let cx = 0.0;
    let cy_mid = 0.0;
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    // Screen y grows downward; model +Y (forward) should point up in the frame.
    // Depth carried from `rotated` for the painter's-algorithm sort.
    let project = |i: usize| -> (f32, f32, f32) {
        let sx = half_w + (proj[i][0] - cx) * scale;
        let syp = half_h - (proj[i][1] - cy_mid) * scale;
        (sx, syp, rotated[i][2])
    };

    // 3. Painter's algorithm: gather triangles with their mean depth, draw
    //    far→near so translucent faces composite into a see-through hologram.
    //    (`shield_start` was computed above for the hull/shield fit split.)
    let mut tris: Vec<(f32, [usize; 3], bool)> = Vec::with_capacity(indices.len() / 3);
    for (ti, chunk) in indices.chunks_exact(3).enumerate() {
        let (a, b, c) = (chunk[0] as usize, chunk[1] as usize, chunk[2] as usize);
        if a >= rotated.len() || b >= rotated.len() || c >= rotated.len() {
            continue;
        }
        let depth = (rotated[a][2] + rotated[b][2] + rotated[c][2]) / 3.0;
        let is_shield = ti * 3 >= shield_start;
        tris.push((depth, [a, b, c], is_shield));
    }
    tris.sort_by(|l, r| l.0.partial_cmp(&r.0).unwrap_or(std::cmp::Ordering::Equal));

    let face_a = (params.face_alpha.clamp(0.0, 1.0) * 255.0) as u8;
    let shield_face_a =
        ((params.face_alpha * params.shield_alpha_scale).clamp(0.0, 1.0) * 255.0) as u8;
    let wire_a = (params.wire_alpha.clamp(0.0, 1.0) * 255.0) as u8;
    for (_, [a, b, c], is_shield) in &tris {
        let p0 = project(*a);
        let p1 = project(*b);
        let p2 = project(*c);
        // Shade each face by how square-on it faces the camera (view depth axis
        // in rotated space), so filled faces convey 3D form without wireframe.
        // Greyscale (r=g=b) is preserved for the data-driven tint multiply.
        let shade = face_shade(&rotated[*a], &rotated[*b], &rotated[*c]);
        let v = (255.0 * shade) as u8;
        let fa = if *is_shield { shield_face_a } else { face_a };
        if fa > 0 {
            fill_triangle(&mut img, p0, p1, p2, [v, v, v, fa]);
        }
        if wire_a > 0 {
            draw_line(&mut img, p0, p1, [255, 255, 255, wire_a]);
            draw_line(&mut img, p1, p2, [255, 255, 255, wire_a]);
            draw_line(&mut img, p2, p0, [255, 255, 255, wire_a]);
        }
    }

    // 4. Apply the data-driven tint as a final multiply over the greyscale.
    if params.tint != [1.0, 1.0, 1.0, 1.0] {
        let t = params.tint;
        for px in img.pixels_mut() {
            let [r, g, b, a] = px.0;
            px.0 = [
                mul_u8(r, t[0]),
                mul_u8(g, t[1]),
                mul_u8(b, t[2]),
                mul_u8(a, t[3]),
            ];
        }
    }
    img
}

/// Lambert-style face shading: ambient + diffuse × |normal · view|, where the
/// view axis is the rotated-space depth (+Z, toward the camera). Faces square-on
/// to the camera are brightest; edge-on faces fade. Returns `0.0..=1.0`.
#[inline]
fn face_shade(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    // normal = e1 × e2
    let nx = e1[1] * e2[2] - e1[2] * e2[1];
    let ny = e1[2] * e2[0] - e1[0] * e2[2];
    let nz = e1[0] * e2[1] - e1[1] * e2[0];
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
    let facing = (nz / len).abs(); // view axis is +Z in rotated space
    const AMBIENT: f32 = 0.35;
    const DIFFUSE: f32 = 0.65;
    (AMBIENT + DIFFUSE * facing).clamp(0.0, 1.0)
}

#[inline]
fn mul_u8(v: u8, f: f32) -> u8 {
    ((v as f32) * f.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8
}

/// Alpha-composite `src` (straight RGBA) "over" the pixel at `(x, y)`.
#[inline]
fn blend_over(img: &mut RgbaImage, x: i32, y: i32, src: [u8; 4]) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 || src[3] == 0 {
        return;
    }
    let dst = img.get_pixel(x as u32, y as u32).0;
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    let blend = |s: u8, d: u8| -> u8 {
        let s = s as f32 / 255.0;
        let d = d as f32 / 255.0;
        let v = (s * sa + d * da * (1.0 - sa)) / out_a;
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    img.put_pixel(
        x as u32,
        y as u32,
        Rgba([
            blend(src[0], dst[0]),
            blend(src[1], dst[1]),
            blend(src[2], dst[2]),
            (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
        ]),
    );
}

/// Scanline-fill a triangle given by three projected `(x, y, depth)` points.
fn fill_triangle(img: &mut RgbaImage, p0: (f32, f32, f32), p1: (f32, f32, f32), p2: (f32, f32, f32), col: [u8; 4]) {
    let minx = p0.0.min(p1.0).min(p2.0).floor().max(0.0) as i32;
    let maxx = p0.0.max(p1.0).max(p2.0).ceil().min(img.width() as f32 - 1.0) as i32;
    let miny = p0.1.min(p1.1).min(p2.1).floor().max(0.0) as i32;
    let maxy = p0.1.max(p1.1).max(p2.1).ceil().min(img.height() as f32 - 1.0) as i32;
    if minx > maxx || miny > maxy {
        return;
    }
    let area = edge(p0, p1, p2);
    if area.abs() < 1e-6 {
        return; // degenerate
    }
    for y in miny..=maxy {
        for x in minx..=maxx {
            let p = (x as f32 + 0.5, y as f32 + 0.5, 0.0);
            let w0 = edge(p1, p2, p);
            let w1 = edge(p2, p0, p);
            let w2 = edge(p0, p1, p);
            // Inside if all edge functions share the winding sign.
            let inside = (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0)
                || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0);
            if inside {
                blend_over(img, x, y, col);
            }
        }
    }
}

#[inline]
fn edge(a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Bresenham line between two projected points.
fn draw_line(img: &mut RgbaImage, a: (f32, f32, f32), b: (f32, f32, f32), col: [u8; 4]) {
    let (mut x0, mut y0) = (a.0.round() as i32, a.1.round() as i32);
    let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        blend_over(img, x0, y0, col);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube centred at the origin (8 verts, 12 triangles).
    fn cube() -> (Vec<[f32; 3]>, Vec<u32>) {
        let p = vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        let idx = vec![
            0, 1, 2, 0, 2, 3, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            2, 3, 7, 2, 7, 6, // back
            1, 2, 6, 1, 6, 5, // right
            3, 0, 4, 3, 4, 7, // left
        ];
        (p, idx)
    }

    #[test]
    fn renders_non_empty_centred_hologram() {
        let (p, idx) = cube();
        let img = render_vehicle_hologram(&p, &idx, 64, 64, &HologramParams::default());
        // Some pixels must be drawn.
        let drawn = img.pixels().filter(|px| px.0[3] > 0).count();
        assert!(drawn > 50, "expected a populated hologram, got {drawn} px");
        // Centre pixel should be lit (cube fills the middle).
        let centre = img.get_pixel(32, 32).0;
        assert!(centre[3] > 0, "centre should be drawn, got {centre:?}");
        // Corners stay transparent (margin).
        assert_eq!(img.get_pixel(0, 0).0[3], 0, "corner must be transparent margin");
    }

    #[test]
    fn neutral_render_is_greyscale_white() {
        let (p, idx) = cube();
        let img = render_vehicle_hologram(&p, &idx, 48, 48, &HologramParams::default());
        for px in img.pixels() {
            if px.0[3] > 0 {
                let [r, g, b, _] = px.0;
                assert!(r == g && g == b, "neutral render must be greyscale, got {:?}", px.0);
            }
        }
    }

    #[test]
    fn shield_triangles_render_at_reduced_alpha() {
        let (p, idx) = cube();
        // Baseline: every face at full face_alpha, no wireframe.
        let mut base = HologramParams::default();
        base.wire_alpha = 0.0;
        base.face_alpha = 0.5;
        let full = render_vehicle_hologram(&p, &idx, 48, 48, &base);
        // Same geometry but treat ALL triangles as shields at half alpha.
        let mut faded = base.clone();
        faded.shield_index_start = Some(0);
        faded.shield_alpha_scale = 0.5;
        let half = render_vehicle_hologram(&p, &idx, 48, 48, &faded);
        let max_a = |img: &RgbaImage| img.pixels().map(|px| px.0[3]).max().unwrap_or(0);
        assert!(
            max_a(&half) < max_a(&full),
            "shield-scaled render must be fainter: full={} faded={}",
            max_a(&full),
            max_a(&half)
        );
        assert!(max_a(&half) > 0, "shield faces should still be visible");
    }

    #[test]
    fn tint_multiplies_channels() {
        let (p, idx) = cube();
        let mut params = HologramParams::default();
        params.tint = [0.4, 0.7, 1.0, 1.0]; // light blue
        let img = render_vehicle_hologram(&p, &idx, 48, 48, &params);
        let mut saw_blueish = false;
        for px in img.pixels() {
            if px.0[3] > 0 {
                let [r, g, b, _] = px.0;
                assert!(b >= g && g >= r, "tint should keep b>=g>=r, got {:?}", px.0);
                if b > r {
                    saw_blueish = true;
                }
            }
        }
        assert!(saw_blueish, "expected tinted (blue-ish) pixels");
    }
}
