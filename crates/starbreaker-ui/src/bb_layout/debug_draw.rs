//! Debug-overlay drawing helpers for the layout visualiser: stable
//! type-name→colour hashing and simple filled/outline rect blitting used by
//! the layout debug render (split from `engine_01` by responsibility —
//! review F1 / line-cap guard).

#[allow(unused_imports)]
use super::*;
use image::{Rgba, RgbaImage};

/// Hash a type-name string to a stable RGBA fill colour at 30 % alpha.
pub(crate) fn type_colour_fill(type_name: &str) -> Rgba<u8> {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    type_name.hash(&mut hasher);
    let hash = hasher.finish();
    // Map the hash to a hue in [0, 360).
    let hue = (hash % 360) as f32;
    let (r, g, b) = hsv_to_rgb(hue, 0.70, 0.85);
    Rgba([
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
        (0.30 * 255.0) as u8,
    ])
}

/// Convert HSV (h in 0–360, s and v in 0–1) to linear RGB (0–1 per channel).
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let h1 = h / 60.0;
    let x = c * (1.0 - (h1 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h1 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r1 + m, g1 + m, b1 + m)
}

/// Alpha-blend a single pixel.  `overlay` alpha drives the blend; the output
/// alpha is always 255 (fully opaque canvas).
pub(crate) fn blend_pixel(base: &mut Rgba<u8>, overlay: Rgba<u8>) {
    let a = overlay[3] as f32 / 255.0;
    let ia = 1.0 - a;
    for i in 0..3 {
        base[i] = (base[i] as f32 * ia + overlay[i] as f32 * a) as u8;
    }
    base[3] = 255;
}

/// Draw a filled rectangle with alpha blending.  Clips to image bounds.
pub(crate) fn draw_rect_filled(img: &mut RgbaImage, rect: Rect, colour: Rgba<u8>) {
    let (iw, ih) = img.dimensions();
    let x0 = (rect.x.floor() as i32).max(0) as u32;
    let y0 = (rect.y.floor() as i32).max(0) as u32;
    let x1 = ((rect.x + rect.w).ceil() as i32).min(iw as i32) as u32;
    let y1 = ((rect.y + rect.h).ceil() as i32).min(ih as i32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = img.get_pixel_mut(x, y);
            blend_pixel(px, colour);
        }
    }
}

/// Draw a 1-pixel outline of a rectangle with alpha blending.  Clips to bounds.
pub(crate) fn draw_rect_outline(img: &mut RgbaImage, rect: Rect, colour: Rgba<u8>) {
    let (iw, ih) = img.dimensions();
    let x0 = rect.x.floor() as i32;
    let y0 = rect.y.floor() as i32;
    let x1 = (rect.x + rect.w).ceil() as i32 - 1;
    let y1 = (rect.y + rect.h).ceil() as i32 - 1;

    // Draw all four sides.
    for x in x0..=x1 {
        for &y in &[y0, y1] {
            if x >= 0 && x < iw as i32 && y >= 0 && y < ih as i32 {
                let px = img.get_pixel_mut(x as u32, y as u32);
                blend_pixel(px, colour);
            }
        }
    }
    for y in y0..=y1 {
        for &x in &[x0, x1] {
            if x >= 0 && x < iw as i32 && y >= 0 && y < ih as i32 {
                let px = img.get_pixel_mut(x as u32, y as u32);
                blend_pixel(px, colour);
            }
        }
    }
}
