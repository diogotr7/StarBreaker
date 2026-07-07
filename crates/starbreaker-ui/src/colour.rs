//! Colour-space conversions and linear-light blend primitives shared by the IR
//! compositor (`ir_compose/`) and the text glyph rasterisers (`text/`).
//!
//! The engine composites in LINEAR light; the u8-premultiplied-sRGB framebuffer
//! (`tiny_skia::Pixmap`) and the straight-alpha `image::RgbaImage` both blend in
//! their STORED sRGB space unless a site converts. These helpers do the
//! per-pixel sRGB→linear→blend→sRGB round trip. `u8_to_linear` is a 256-entry
//! LUT so the sRGB→linear direction (whose inputs are always u8) costs a lookup
//! instead of a `powf`.
//!
//! Key items: `srgb_channel_to_linear` / `linear_channel_to_srgb` (channel
//! EOTF/OETF), `u8_to_linear` (LUT), and the three blend primitives
//! `blend_straight_linear`, `blend_premul_linear`, `blend_premul_add_linear`.

use std::sync::OnceLock;

/// sRGB electro-optical transfer function (gamma-decode) for a single 0..1
/// channel: piecewise 12.92 toe + 2.4 gamma.
pub(crate) fn srgb_channel_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse of [`srgb_channel_to_linear`] (gamma-encode) for a single 0..1 linear
/// channel.
pub(crate) fn linear_channel_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB→linear for a u8 channel, via a 256-entry LUT (built once). Every blend
/// input that is a stored u8 sRGB byte goes through here, avoiding a per-pixel
/// `powf`.
#[allow(dead_code)] // wired in B4 Task 3 (blit texel loop)
pub(crate) fn u8_to_linear(v: u8) -> f32 {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_channel_to_linear(i as f32 / 255.0)))[v as usize]
}

/// Straight-alpha source-over in LINEAR light. `dst` is straight-alpha u8 sRGB
/// (an `image::RgbaImage` pixel's `.0`); `src_rgb` is the straight u8 sRGB
/// source colour; `src_a` in 0..1 already folds coverage × source alpha.
#[allow(dead_code)] // wired in B4 Tasks 4-5 (clip-image + ttf glyph blends)
pub(crate) fn blend_straight_linear(dst: &mut [u8; 4], src_rgb: [u8; 3], src_a: f32) {
    let src_a = src_a.clamp(0.0, 1.0);
    if src_a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = src_a + da * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s_lin = u8_to_linear(src_rgb[c]);
        let d_lin = u8_to_linear(dst[c]);
        let out_lin = (s_lin * src_a + d_lin * da * (1.0 - src_a)) / out_a;
        dst[c] = (linear_channel_to_srgb(out_lin.clamp(0.0, 1.0)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Premultiplied source-over in LINEAR light. Both `dst` and `src` are
/// premultiplied u8 sRGB (a `tiny_skia::Pixmap` pixel). Generalises the landed
/// white-mask carve-out (`ir_compose/engine_01.rs` history) to any source.
pub(crate) fn blend_premul_linear(dst: &mut [u8; 4], src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s_lin = srgb_channel_to_linear(((src[c] as f32 / 255.0) / sa).clamp(0.0, 1.0));
        let d_lin = if da > 0.0 {
            srgb_channel_to_linear(((dst[c] as f32 / 255.0) / da).clamp(0.0, 1.0))
        } else {
            0.0
        };
        let out_lin = (s_lin * sa + d_lin * da * (1.0 - sa)) / out_a;
        dst[c] = (linear_channel_to_srgb(out_lin.clamp(0.0, 1.0)) * out_a * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Premultiplied additive (`tiny_skia::BlendMode::Plus`) in LINEAR light — used
/// by glow layers (hologram, radar disc, `*_glow.tif`). Sums premultiplied
/// linear channels, clamped to the output alpha.
pub(crate) fn blend_premul_add_linear(dst: &mut [u8; 4], src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let out_a = (sa + da).min(1.0);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s_pl = if sa > 0.0 {
            srgb_channel_to_linear(((src[c] as f32 / 255.0) / sa).clamp(0.0, 1.0)) * sa
        } else {
            0.0
        };
        let d_pl = if da > 0.0 {
            srgb_channel_to_linear(((dst[c] as f32 / 255.0) / da).clamp(0.0, 1.0)) * da
        } else {
            0.0
        };
        let out_pl = (s_pl + d_pl).min(out_a);
        let out_straight = (out_pl / out_a).clamp(0.0, 1.0);
        dst[c] = (linear_channel_to_srgb(out_straight) * out_a * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_matches_powf_helper_for_all_bytes() {
        for v in 0u16..=255 {
            let v = v as u8;
            assert_eq!(
                u8_to_linear(v),
                srgb_channel_to_linear(v as f32 / 255.0),
                "LUT[{v}] must equal the powf helper exactly"
            );
        }
        assert_eq!(u8_to_linear(0), 0.0);
        assert!((u8_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn channel_round_trip_is_near_identity() {
        for v in 0u16..=255 {
            let c = v as f32 / 255.0;
            let back = linear_channel_to_srgb(srgb_channel_to_linear(c));
            assert!((back - c).abs() < 1e-4, "round trip drift at {c}: {back}");
        }
    }

    // 50% coverage of white over OPAQUE black: linear blend lands ~188, NOT the
    // sRGB midpoint 128. This is the whole point of the migration.
    #[test]
    fn blend_straight_linear_white_over_black_is_linear_not_srgb() {
        let mut dst = [0u8, 0, 0, 255];
        blend_straight_linear(&mut dst, [255, 255, 255], 0.5);
        assert!(
            (dst[0] as i32 - 188).abs() <= 2 && dst[3] == 255,
            "linear 50% white over black expected ~188, got {dst:?}"
        );
        assert!((dst[0] as i32 - 128).abs() > 20, "must NOT be the sRGB midpoint 128");
    }

    #[test]
    fn blend_premul_linear_half_white_over_black_is_linear() {
        // premultiplied white at alpha 0.5 => (128,128,128,128) over opaque black.
        let mut dst = [0u8, 0, 0, 255];
        blend_premul_linear(&mut dst, [128, 128, 128, 128]);
        assert!(
            (dst[0] as i32 - 188).abs() <= 3 && dst[3] == 255,
            "linear premul 50% white over black expected ~188, got {dst:?}"
        );
    }

    #[test]
    fn blend_premul_add_linear_sums_in_linear() {
        // Additive of premultiplied opaque mid-grey onto itself: linear(0.502)
        // + linear(0.502) ≈ 0.442 -> srgb ≈ 0.70 -> ~178. A naive sRGB byte sum
        // would clamp to 128+128=255 immediately.
        let mut dst = [128u8, 128, 128, 255];
        blend_premul_add_linear(&mut dst, [128, 128, 128, 255]);
        assert!(
            dst[0] < 255 && dst[0] > 150,
            "additive-in-linear grey+grey expected ~178, got {dst:?}"
        );
    }
}
