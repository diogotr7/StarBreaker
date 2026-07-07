//! SWF overlay compositors: blend a rasterised SWF scratch layer against the
//! IR framebuffer. `composite_rgba_over_pixmap` lays a straight-alpha
//! `RgbaImage` (text glyphs) onto a premultiplied `tiny_skia::Pixmap`;
//! `composite_pixmap_over_rgba` lays a premultiplied `Pixmap` (SWF stage/shape)
//! onto a straight-alpha `RgbaImage`. Both blend in LINEAR light via the shared
//! `crate::colour` primitives so the SWF/Hybrid overlay path matches the rest of
//! the renderer (B4 — no sRGB carve-outs).

use image::RgbaImage;
use tiny_skia::Pixmap;

/// Composite straight-alpha RGBA image over premultiplied pixmap.
///
/// Pixels from `src` (straight alpha) are blended on top of `dst`
/// (premultiplied alpha).  Only pixels with non-zero source alpha are written,
/// so a sparse text-only image composites efficiently.
pub(super) fn composite_rgba_over_pixmap(src: &RgbaImage, dst: &mut Pixmap) {
    let dst_w = dst.width();
    let dst_h = dst.height();
    let w = src.width().min(dst_w);
    let h = src.height().min(dst_h);
    let dst_data = dst.data_mut();

    for y in 0..h {
        for x in 0..w {
            let px = src.get_pixel(x, y);
            let sa = px[3] as u32;
            if sa == 0 {
                continue;
            }
            let idx = ((y * dst_w + x) as usize) * 4;
            // Convert src from straight to premultiplied sRGB, then blend it over
            // the premultiplied dst in LINEAR light.
            let src_pm = [
                (px[0] as u32 * sa / 255) as u8,
                (px[1] as u32 * sa / 255) as u8,
                (px[2] as u32 * sa / 255) as u8,
                sa as u8,
            ];
            let dst_slice: &mut [u8; 4] = (&mut dst_data[idx..idx + 4]).try_into().unwrap();
            crate::colour::blend_premul_linear(dst_slice, src_pm);
        }
    }
}

/// Composite premultiplied pixmap over straight-alpha RGBA image.
pub(super) fn composite_pixmap_over_rgba(pixmap: &Pixmap, img: &mut RgbaImage) {
    let w = img.width();
    let h = img.height();
    let pix = pixmap.data();
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) as usize) * 4;
            let a_top = pix[idx + 3] as u32;
            if a_top == 0 {
                continue;
            }
            // Unpremultiply the top to straight sRGB, then blend it over the
            // straight-alpha base in LINEAR light. `blend_straight_linear`
            // computes out_a = src_a + dst_a*(1 - src_a) (alpha stays linear).
            let src_rgb = [
                ((pix[idx] as u32 * 255) / a_top.max(1)).min(255) as u8,
                ((pix[idx + 1] as u32 * 255) / a_top.max(1)).min(255) as u8,
                ((pix[idx + 2] as u32 * 255) / a_top.max(1)).min(255) as u8,
            ];
            let p = img.get_pixel_mut(x, y);
            crate::colour::blend_straight_linear(&mut p.0, src_rgb, a_top as f32 / 255.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use tiny_skia::Color;

    // 50% straight white over OPAQUE black must composite in LINEAR light:
    // linear-half -> ~188 sRGB, NOT the sRGB midpoint 128.
    #[test]
    fn composite_rgba_over_pixmap_blends_in_linear() {
        let mut dst = Pixmap::new(1, 1).expect("pixmap");
        dst.fill(Color::from_rgba8(0, 0, 0, 255)); // premul (0,0,0,255)
        let src = RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 128])); // STRAIGHT alpha

        composite_rgba_over_pixmap(&src, &mut dst);

        let out = dst.data();
        assert!(
            (out[0] as i32 - 188).abs() <= 3,
            "expected ~188 (linear), got {:?}",
            &out[0..4]
        );
        assert!(out[0] > 160, "must NOT be the sRGB midpoint 128, got {}", out[0]);
        assert_eq!(out[3], 255, "opaque over opaque stays opaque");
    }

    // Premultiplied 50% white pixmap over opaque-black straight image must
    // composite in LINEAR light: ~188, not 128.
    #[test]
    fn composite_pixmap_over_rgba_blends_in_linear() {
        let mut src = Pixmap::new(1, 1).expect("pixmap");
        src.fill(Color::from_rgba8(255, 255, 255, 128)); // tiny_skia stores premul (128,128,128,128)
        // Confirm the stored premul bytes so the fixture can't drift silently.
        assert_eq!(src.data(), &[128, 128, 128, 128], "expected premul 50% white");

        let mut img = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255])); // opaque black, STRAIGHT

        composite_pixmap_over_rgba(&src, &mut img);

        let p = img.get_pixel(0, 0).0;
        assert!(
            (p[0] as i32 - 188).abs() <= 3,
            "expected ~188 (linear), got {p:?}"
        );
        assert!(p[0] > 160, "must NOT be the sRGB midpoint 128, got {}", p[0]);
        assert_eq!(p[3], 255, "opaque over opaque stays opaque");
    }
}
