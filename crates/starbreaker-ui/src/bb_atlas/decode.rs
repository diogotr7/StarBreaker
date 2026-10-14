//! Decode and resize helpers for atlas bitmap/SVG assets.

use image::{GenericImageView, RgbaImage, imageops};
use log::{debug, warn};

pub(super) fn decode_bytes(bytes: &[u8], ext: &str, target_w: u32, target_h: u32) -> Option<RgbaImage> {
    let img = match ext {
        "svg" => decode_svg(bytes, target_w, target_h)?,
        "dds" => decode_dds(bytes)?,
        _ => {
            let dyn_img = image::load_from_memory(bytes)
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("format could not be determined") {
                        debug!("atlas: skipping non-image bytes");
                    } else {
                        warn!("atlas: image decode failed: {}", e);
                    }
                    e
                })
                .ok()?;
            dyn_img.to_rgba8()
        }
    };

    Some(resize_to(img, target_w, target_h))
}

pub(super) fn source_dimensions(bytes: &[u8], ext: &str) -> Option<(u32, u32)> {
    match ext {
        "dds" => {
            let dds = starbreaker_dds::DdsFile::from_bytes(bytes)
                .map_err(|e| {
                    warn!("atlas: DDS dimension read failed: {}", e);
                    e
                })
                .ok()?;
            Some(dds.dimensions(0))
        }
        "svg" => {
            let opts = usvg::Options::default();
            let tree = usvg::Tree::from_data(bytes, &opts)
                .map_err(|e| {
                    warn!("atlas: SVG parse failed: {}", e);
                    e
                })
                .ok()?;
            let size = tree.size();
            let w = size.width().round().max(1.0) as u32;
            let h = size.height().round().max(1.0) as u32;
            Some((w, h))
        }
        _ => {
            let dyn_img = image::load_from_memory(bytes)
                .map_err(|e| {
                    warn!("atlas: image dimension read failed: {}", e);
                    e
                })
                .ok()?;
            Some(dyn_img.dimensions())
        }
    }
}

fn decode_svg(bytes: &[u8], target_w: u32, target_h: u32) -> Option<RgbaImage> {
    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opts)
        .map_err(|e| {
            warn!("atlas: SVG parse failed: {}", e);
            e
        })
        .ok()?;

    let source_w = tree.size().width();
    let source_h = tree.size().height();
    if source_w <= 0.0 || source_h <= 0.0 {
        warn!("atlas: SVG has invalid size {}x{}", source_w, source_h);
        return None;
    }

    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)?;
    let transform = tiny_skia::Transform::from_scale(
        target_w as f32 / source_w,
        target_h as f32 / source_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    RgbaImage::from_raw(target_w, target_h, pixmap.take())
}

fn decode_dds(bytes: &[u8]) -> Option<RgbaImage> {
    let dds = starbreaker_dds::DdsFile::from_bytes(bytes)
        .map_err(|e| {
            warn!("atlas: DDS decode failed: {}", e);
            e
        })
        .ok()?;
    let (w, h) = dds.dimensions(0);
    let rgba = dds
        .decode_rgba(0)
        .map_err(|e| {
            warn!("atlas: DDS RGBA extract failed: {}", e);
            e
        })
        .ok()?;
    RgbaImage::from_raw(w, h, rgba)
}

fn resize_to(img: RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.width() == w && img.height() == h {
        return img;
    }
    imageops::resize(&img, w, h, imageops::FilterType::Lanczos3)
}
