//! Manual clip-region composites for the IR compositor.
//!
//! Boundary-crossing nodes (a node partially inside its clipping ancestor's
//! region — see `with_node_clip` / `with_node_clip_image` in `engine_02`) are
//! drawn through a full-size scratch surface, then only the clip rectangle is
//! composited back onto the destination. Both composites blend in LINEAR light
//! (via `crate::colour`) so their AA/overlap edges match the rest of the
//! renderer:
//!   - `composite_clip_region_pixmap`: premultiplied `tiny_skia::Pixmap` source
//!     over the framebuffer pixmap (`blend_premul_linear`).
//!   - `composite_clip_region_image`: straight-alpha `image::RgbaImage` source
//!     over the text-pass image (`blend_straight_linear`).

use crate::colour::{blend_premul_linear, blend_straight_linear};
use crate::ui_ir::UiIrRect;
use image::RgbaImage;
use tiny_skia::Pixmap;

/// SourceOver-composite `src`'s clip region onto `dst` (premultiplied RGBA) in
/// LINEAR light.
pub(crate) fn composite_clip_region_pixmap(dst: &mut Pixmap, src: &Pixmap, clip: &UiIrRect) {
    let w = dst.width() as i32;
    let h = dst.height() as i32;
    let x0 = (clip.x.floor() as i32).clamp(0, w);
    let y0 = (clip.y.floor() as i32).clamp(0, h);
    let x1 = ((clip.x + clip.w).ceil() as i32).clamp(0, w);
    let y1 = ((clip.y + clip.h).ceil() as i32).clamp(0, h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for y in y0..y1 {
        let row = (y * w) as usize;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            if src_data[i + 3] == 0 {
                continue;
            }
            let src = [src_data[i], src_data[i + 1], src_data[i + 2], src_data[i + 3]];
            let mut d = [dst_data[i], dst_data[i + 1], dst_data[i + 2], dst_data[i + 3]];
            blend_premul_linear(&mut d, src);
            dst_data[i..i + 4].copy_from_slice(&d);
        }
    }
}

/// SourceOver-composite `src`'s clip region onto `dst` (straight RGBA) in LINEAR
/// light.
pub(crate) fn composite_clip_region_image(dst: &mut RgbaImage, src: &RgbaImage, clip: &UiIrRect) {
    let w = dst.width() as i32;
    let h = dst.height() as i32;
    let x0 = (clip.x.floor() as i32).clamp(0, w);
    let y0 = (clip.y.floor() as i32).clamp(0, h);
    let x1 = ((clip.x + clip.w).ceil() as i32).clamp(0, w);
    let y1 = ((clip.y + clip.h).ceil() as i32).clamp(0, h);
    for y in y0..y1 {
        for x in x0..x1 {
            let sp = src.get_pixel(x as u32, y as u32);
            let sa = sp[3] as f32 / 255.0;
            if sa <= 0.0 {
                continue;
            }
            let dp = dst.get_pixel_mut(x as u32, y as u32);
            blend_straight_linear(&mut dp.0, [sp[0], sp[1], sp[2]], sa);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Premultiplied 50% white over opaque black lands ~188 in linear light, not
    // the sRGB midpoint 128. `Color::from_rgba8` takes STRAIGHT alpha, so
    // straight (255,255,255,128) stores as premultiplied (128,128,128,128).
    #[test]
    fn composite_clip_pixmap_blends_in_linear() {
        let mut dst = Pixmap::new(2, 2).unwrap();
        dst.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
        let mut src = Pixmap::new(2, 2).unwrap();
        src.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 128));
        let clip = UiIrRect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 };
        composite_clip_region_pixmap(&mut dst, &src, &clip);
        let px = dst.pixel(0, 0).unwrap();
        assert!(
            (px.red() as i32 - 188).abs() <= 3,
            "linear clip pixmap expected ~188, got {}",
            px.red()
        );
        assert!((px.red() as i32 - 128).abs() > 20, "must not be sRGB 128");
    }

    // Straight-alpha 50% white over opaque black lands ~188 in linear light.
    #[test]
    fn composite_clip_image_blends_in_linear() {
        let mut dst = RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let src = RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 128]));
        let clip = UiIrRect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 };
        composite_clip_region_image(&mut dst, &src, &clip);
        let px = dst.get_pixel(0, 0);
        assert!(
            (px[0] as i32 - 188).abs() <= 3,
            "linear clip image expected ~188, got {}",
            px[0]
        );
        assert!((px[0] as i32 - 128).abs() > 20, "must not be sRGB 128");
    }
}
