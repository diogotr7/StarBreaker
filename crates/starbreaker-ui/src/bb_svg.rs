//! SVG rasterisation helper for BuildingBlocks UI widgets.
//!
//! Provides [`rasterize_svg`] and [`rasterize_svg_nine_slice`] for BuildingBlocks
//! SVG fills. Both paths optionally apply a fill-colour override and return RGBA
//! images sized to caller-supplied target dimensions.
//!
//! # Fill override
//! Many Star Citizen UI SVGs are monochrome masks coloured at runtime by a brand
//! modifier's `FillColor`.  When `fill_override` is `Some([r, g, b, a])`, every
//! non-transparent pixel in the rendered output is recoloured to the override RGB
//! while preserving the rendered SVG alpha mask and scaling opacity by `fill[3]`.

use image::{imageops, RgbaImage};
use log::warn;

/// Rasterise `svg_bytes` into an RGBA image of `target_w × target_h` pixels.
///
/// If `fill_override` is `Some([r, g, b, a])` (components in `0.0..=1.0`), every
/// non-transparent pixel is recoloured to the override RGB after rendering.
///
/// Returns `None` when:
/// - `target_w` or `target_h` is zero,
/// - the SVG cannot be parsed (logged at `warn`),
/// - the internal pixmap allocation fails.
pub fn rasterize_svg(
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
) -> Option<RgbaImage> {
    if target_w == 0 || target_h == 0 {
        return None;
    }

    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opts)
        .map_err(|e| {
            warn!("bb_svg: SVG parse failed: {}", e);
            e
        })
        .ok()?;

    let source_w = tree.size().width();
    let source_h = tree.size().height();
    if source_w <= 0.0 || source_h <= 0.0 {
        warn!("bb_svg: SVG has invalid size {}×{}", source_w, source_h);
        return None;
    }

    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)?;
    let transform = tiny_skia::Transform::from_scale(
        target_w as f32 / source_w,
        target_h as f32 / source_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut bytes = pixmap.take();

    // Apply fill_override as a colour overlay. SVG UI glyphs are authored as
    // monochrome masks (often black strokes), so multiplying by the source RGB
    // would keep black glyphs black instead of applying the BuildingBlocks tint.
    if let Some(fill) = fill_override {
        for chunk in bytes.chunks_exact_mut(4) {
            if chunk[3] > 0 {
                let alpha = (chunk[3] as f32 * fill[3]).clamp(0.0, 255.0);
                chunk[0] = (fill[0].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[1] = (fill[1].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[2] = (fill[2].clamp(0.0, 1.0) * 255.0) as u8;
                chunk[3] = alpha as u8;
            }
        }
    }

    RgbaImage::from_raw(target_w, target_h, bytes)
}

pub fn rasterize_svg_contained(
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
    contain_position_x: f32,
    contain_position_y: f32,
) -> Option<RgbaImage> {
    if target_w == 0 || target_h == 0 {
        return None;
    }

    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opts)
        .map_err(|e| {
            warn!("bb_svg: SVG parse failed: {}", e);
            e
        })
        .ok()?;

    let source_w = tree.size().width();
    let source_h = tree.size().height();
    if source_w <= 0.0 || source_h <= 0.0 {
        warn!("bb_svg: SVG has invalid size {}×{}", source_w, source_h);
        return None;
    }

    let scale = (target_w as f32 / source_w).min(target_h as f32 / source_h);
    let render_w = (source_w * scale).round().max(1.0).min(target_w as f32) as u32;
    let render_h = (source_h * scale).round().max(1.0).min(target_h as f32) as u32;
    let rendered = rasterize_svg(svg_bytes, render_w, render_h, fill_override)?;

    let mut out = RgbaImage::new(target_w, target_h);
    let free_x = target_w.saturating_sub(render_w);
    let free_y = target_h.saturating_sub(render_h);
    let draw_x = (free_x as f32 * contain_position_x.clamp(0.0, 1.0)).round() as i64;
    let draw_y = (free_y as f32 * contain_position_y.clamp(0.0, 1.0)).round() as i64;
    imageops::overlay(&mut out, &rendered, draw_x, draw_y);
    Some(out)
}

/// Rasterise an SVG using BuildingBlocks-style nine-slice scaling.
///
/// `nine_slice_rect` is `[left, top, right, bottom]` in normalized source-space
/// coordinates. The source image is divided on those cuts; edge/corner regions
/// keep their source pixel widths while the center bands stretch to the target.
pub fn rasterize_svg_nine_slice(
    svg_bytes: &[u8],
    target_w: u32,
    target_h: u32,
    fill_override: Option<[f32; 4]>,
    nine_slice_rect: [f32; 4],
    nine_slice_scale: f32,
) -> Option<RgbaImage> {
    if target_w == 0 || target_h == 0 {
        return None;
    }

    let opts = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opts)
        .map_err(|e| {
            warn!("bb_svg: SVG parse failed: {}", e);
            e
        })
        .ok()?;

    let source_w = tree.size().width().round().max(1.0) as u32;
    let source_h = tree.size().height().round().max(1.0) as u32;
    let source = rasterize_svg(svg_bytes, source_w, source_h, fill_override)?;
    let [left, top, right, bottom] = nine_slice_rect;
    let left = (left.clamp(0.0, 1.0) * source_w as f32).round() as u32;
    let right = (right.clamp(0.0, 1.0) * source_w as f32).round() as u32;
    let top = (top.clamp(0.0, 1.0) * source_h as f32).round() as u32;
    let bottom = (bottom.clamp(0.0, 1.0) * source_h as f32).round() as u32;
    if left >= right || top >= bottom || right > source_w || bottom > source_h {
        return rasterize_svg(svg_bytes, target_w, target_h, fill_override);
    }

    let edge_scale = nine_slice_scale.max(0.0);
    let left_dst = ((left as f32 * edge_scale).round() as u32).min(target_w);
    let right_src_w = source_w - right;
    let right_dst_w = ((right_src_w as f32 * edge_scale).round() as u32).min(target_w.saturating_sub(left_dst));
    let top_dst = ((top as f32 * edge_scale).round() as u32).min(target_h);
    let bottom_src_h = source_h - bottom;
    let bottom_dst_h = ((bottom_src_h as f32 * edge_scale).round() as u32).min(target_h.saturating_sub(top_dst));

    let src_x = [0, left, right, source_w];
    let src_y = [0, top, bottom, source_h];
    let dst_x = [0, left_dst, target_w - right_dst_w, target_w];
    let dst_y = [0, top_dst, target_h - bottom_dst_h, target_h];
    let mut out = RgbaImage::new(target_w, target_h);

    for y_index in 0..3 {
        for x_index in 0..3 {
            let sx = src_x[x_index];
            let sy = src_y[y_index];
            let sw = src_x[x_index + 1].saturating_sub(sx);
            let sh = src_y[y_index + 1].saturating_sub(sy);
            let dx = dst_x[x_index];
            let dy = dst_y[y_index];
            let dw = dst_x[x_index + 1].saturating_sub(dx);
            let dh = dst_y[y_index + 1].saturating_sub(dy);
            if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
                continue;
            }
            let patch = imageops::crop_imm(&source, sx, sy, sw, sh).to_image();
            let resized = if patch.width() == dw && patch.height() == dh {
                patch
            } else {
                imageops::resize(&patch, dw, dh, imageops::FilterType::Nearest)
            };
            imageops::overlay(&mut out, &resized, dx.into(), dy.into());
        }
    }

    Some(out)
}

/// Parse the UNIFORM `colorstyle:` brand-palette convention from an
/// Illustrator-exported UI SVG's path `id`s.
///
/// UI HUD glyph SVGs author every path with a placeholder `fill="#…"` and encode
/// the real brand role in the path id, e.g.
/// `id="opacity:50_colorstyle:Accent1_<hash>_"`. The engine recolours the path to
/// the brand palette's `Accent1` at 50% alpha; the literal fill is just an Adobe
/// export artefact. Returns `Some((role, alpha))` ONLY when every `colorstyle:`
/// path shares ONE role (and one `opacity:`) — a single fill recolour cannot
/// represent a multi-role SVG, so those (and non-colorstyle SVGs) return `None`
/// and render their authored fills unchanged. `alpha` is `opacity/100` (default
/// 1.0). The `opacity:` value is only read in the `opacity:<digits>_` id form, so
/// a CSS `opacity:0.5` style is ignored.
pub fn parse_uniform_colorstyle(svg_bytes: &[u8]) -> Option<(String, f32)> {
    let text = std::str::from_utf8(svg_bytes).ok()?;
    let mut role: Option<&str> = None;
    for (idx, _) in text.match_indices("colorstyle:") {
        let after = &text[idx + "colorstyle:".len()..];
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        let candidate = &after[..end];
        if candidate.is_empty() {
            return None;
        }
        match role {
            Some(existing) if existing != candidate => return None, // multi-role SVG
            _ => role = Some(candidate),
        }
    }
    let role = role?.to_string();

    let mut opacity_pct: Option<u32> = None;
    for (idx, _) in text.match_indices("opacity:") {
        let after = &text[idx + "opacity:".len()..];
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        // Only the `opacity:<digits>_` id-token form (not CSS `opacity:0.5`).
        if end == 0 || after.as_bytes().get(end) != Some(&b'_') {
            continue;
        }
        if let Ok(value) = after[..end].parse::<u32>() {
            match opacity_pct {
                Some(existing) if existing != value => return None, // non-uniform opacity
                _ => opacity_pct = Some(value),
            }
        }
    }
    let alpha = opacity_pct
        .map(|pct| (pct as f32 / 100.0).clamp(0.0, 1.0))
        .unwrap_or(1.0);
    Some((role, alpha))
}

/// Recolour every `colorstyle:` path of a HUD glyph SVG to its brand palette
/// role at the path's own id-encoded opacity, returning rewritten SVG bytes.
///
/// Adobe-exported HUD SVGs encode a PER-PATH brand role and opacity in each
/// element id (`id="opacity:70_colorstyle:Critical_<hash>_"`) over an arbitrary
/// placeholder `fill="#…"`. The engine recolours each path independently to its
/// role colour. Unlike the single whole-image [`rasterize_svg`] `fill_override`,
/// this represents SVGs whose paths carry DIFFERENT roles or DIFFERENT opacities
/// — the velocity cross-line (`Accent1` at 85/50) and cross-cap (`Critical` at
/// 100/70), which [`parse_uniform_colorstyle`] rejects. `resolve(role)` returns
/// the brand RGBA in `0.0..=1.0`; its alpha multiplies the path opacity into
/// `fill-opacity`. A path whose role does not resolve keeps its authored fill.
/// Returns `None` when no `colorstyle:` path was recoloured (a plain SVG, or one
/// whose roles all fail to resolve, is left untouched and rasterised as authored).
pub fn recolour_colorstyle_svg(
    svg_bytes: &[u8],
    resolve: impl Fn(&str) -> Option<[f32; 4]>,
) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(svg_bytes).ok()?;
    if !text.contains("colorstyle:") {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 64);
    let mut cursor = 0usize;
    let mut recoloured_any = false;
    while let Some(rel_lt) = text[cursor..].find('<') {
        let lt = cursor + rel_lt;
        let gt = match text[lt..].find('>') {
            Some(rel) => lt + rel + 1,
            None => break,
        };
        let tag = &text[lt..gt];
        out.push_str(&text[cursor..lt]);
        match colorstyle_role_opacity(tag).and_then(|(role, op)| resolve(&role).map(|rgba| (rgba, op))) {
            Some((rgba, op_pct)) => {
                out.push_str(&rewrite_tag_fill(tag, rgba, op_pct));
                recoloured_any = true;
            }
            None => out.push_str(tag),
        }
        cursor = gt;
    }
    out.push_str(&text[cursor..]);
    recoloured_any.then(|| out.into_bytes())
}

/// Extract `(role, opacity_pct)` from a single element tag's `colorstyle:` /
/// `opacity:<digits>_` id tokens. `opacity` defaults to 100 when absent.
fn colorstyle_role_opacity(tag: &str) -> Option<(String, u32)> {
    let idx = tag.find("colorstyle:")?;
    let after = &tag[idx + "colorstyle:".len()..];
    let end = after
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(after.len());
    let role = &after[..end];
    if role.is_empty() {
        return None;
    }
    // The id token form `opacity:<digits>_` (not a CSS `opacity:0.5` style).
    let opacity_pct = tag
        .match_indices("opacity:")
        .find_map(|(i, _)| {
            let a = &tag[i + "opacity:".len()..];
            let e = a.find(|c: char| !c.is_ascii_digit()).unwrap_or(a.len());
            (e > 0 && a.as_bytes().get(e) == Some(&b'_'))
                .then(|| a[..e].parse::<u32>().ok())
                .flatten()
        })
        .unwrap_or(100);
    Some((role.to_string(), opacity_pct))
}

/// Rewrite a single element tag's `fill` to `rgba`'s hex and its `fill-opacity`
/// to `(op_pct/100) × rgba_alpha`, inserting either attribute if absent.
fn rewrite_tag_fill(tag: &str, rgba: [f32; 4], op_pct: u32) -> String {
    let hex = format!(
        "#{:02X}{:02X}{:02X}",
        (rgba[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    let alpha = trim_float((op_pct as f32 / 100.0 * rgba[3].clamp(0.0, 1.0)).clamp(0.0, 1.0));

    let mut result = tag.to_string();
    match find_attr_value_span(&result, "fill") {
        Some((s, e)) => result.replace_range(s..e, &hex),
        None => inject_attr(&mut result, &format!("fill=\"{hex}\"")),
    }
    match find_attr_value_span(&result, "fill-opacity") {
        Some((s, e)) => result.replace_range(s..e, &alpha),
        None => inject_attr(&mut result, &format!("fill-opacity=\"{alpha}\"")),
    }
    result
}

/// Span of an attribute's value between the quotes, e.g. `fill="#abc"` → the
/// `#abc` range. Matches ` name="` (leading space) so `fill` never matches a
/// `fill-opacity` / `fill-rule` attribute.
fn find_attr_value_span(tag: &str, name: &str) -> Option<(usize, usize)> {
    let needle = format!(" {name}=\"");
    let i = tag.find(&needle)?;
    let val_start = i + needle.len();
    let val_end = tag[val_start..].find('"')? + val_start;
    Some((val_start, val_end))
}

/// Insert ` attr` just before a tag's closing `/>` or `>`.
fn inject_attr(tag: &mut String, attr: &str) {
    let at = tag
        .rfind("/>")
        .or_else(|| tag.rfind('>'))
        .unwrap_or(tag.len());
    tag.insert_str(at, &format!(" {attr}"));
}

/// Format a `0.0..=1.0` float as a compact SVG attribute value (`1`, `0.85`).
fn trim_float(v: f32) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 4×4 white SVG used as a test fixture.
    const WHITE_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
        <rect width="4" height="4" fill="white"/>
    </svg>"#;

    /// The HUD glyph SVG convention: placeholder fills, brand role + opacity in
    /// the path ids. A uniform `colorstyle:Accent1` at `opacity:50` resolves to
    /// `("Accent1", 0.5)`; a plain fill SVG and a multi-role SVG resolve to `None`.
    #[test]
    fn parse_uniform_colorstyle_reads_role_and_opacity() {
        let uniform = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="opacity:50_colorstyle:Accent1_00000131_" fill="#6CB8C7" d="M0 0h1v1H0z"/>
            <path id="opacity:50_colorstyle:Accent1_00000161_" fill="#6CB8C7" d="M2 2h1v1H2z"/>
        </svg>"##;
        assert_eq!(parse_uniform_colorstyle(uniform), Some(("Accent1".to_string(), 0.5)));

        // No colorstyle ids → render the authored fill unchanged.
        assert_eq!(parse_uniform_colorstyle(WHITE_SVG), None);

        // Multi-role SVG cannot be one fill → leave it alone.
        let multi = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="colorstyle:Accent1_a_" fill="#000" d="M0 0h1v1H0z"/>
            <path id="colorstyle:Accent2_b_" fill="#000" d="M2 2h1v1H2z"/>
        </svg>"##;
        assert_eq!(parse_uniform_colorstyle(multi), None);

        // colorstyle without an opacity id-token defaults to alpha 1.0.
        let no_op = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="colorstyle:Base_x_" fill="#000" d="M0 0h1v1H0z"/>
        </svg>"##;
        assert_eq!(parse_uniform_colorstyle(no_op), Some(("Base".to_string(), 1.0)));
    }

    /// A minimal 4×4 red SVG used as a test fixture.
    const RED_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
        <rect width="4" height="4" fill="red"/>
    </svg>"#;

    /// A minimal 4×4 black SVG used as a mask-style test fixture.
    const BLACK_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
        <rect width="4" height="4" fill="black"/>
    </svg>"#;

    #[test]
    fn rasterizes_to_correct_size() {
        let img = rasterize_svg(WHITE_SVG, 32, 32, None).expect("should rasterize");
        assert_eq!((img.width(), img.height()), (32, 32));
    }

    #[test]
    fn rasterizes_to_non_empty_pixmap() {
        let img = rasterize_svg(WHITE_SVG, 8, 8, None).expect("should rasterize");
        // At least one pixel must be non-transparent.
        let any_visible = img.pixels().any(|p| p.0[3] > 0);
        assert!(any_visible, "rasterized image should have non-transparent pixels");
    }

    #[test]
    fn fill_override_tints_pixels() {
        // White SVG + pure-blue fill override → pixels should be blue-ish.
        let fill = Some([0.0, 0.0, 1.0, 1.0]);
        let img = rasterize_svg(WHITE_SVG, 8, 8, fill).expect("should rasterize");
        let centre = img.get_pixel(4, 4).0;
        // The pixel should have effectively zero red and green channels, and visible blue.
        // (tiny-skia stores premultiplied; white pixels become fully blue after override.)
        assert!(
            centre[0] < 30 && centre[2] > 100,
            "centre pixel should be blue-ish after fill override, got {centre:?}"
        );
    }

    #[test]
    fn fill_override_recolours_black_mask_pixels() {
        let fill = Some([115.0 / 255.0, 198.0 / 255.0, 254.0 / 255.0, 1.0]);
        let img = rasterize_svg(BLACK_SVG, 4, 4, fill).expect("should rasterize");
        let px = img.get_pixel(2, 2).0;
        assert!(
            px[0] >= 110 && px[1] >= 190 && px[2] >= 245,
            "black mask pixel should be recoloured cyan, got {px:?}"
        );
    }

    #[test]
    fn fill_override_preserves_straight_rgb_for_partial_alpha_masks() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <rect width="4" height="4" fill="black" opacity="0.5"/>
        </svg>"#;
        let fill = Some([115.0 / 255.0, 198.0 / 255.0, 254.0 / 255.0, 1.0]);
        let img = rasterize_svg(svg, 4, 4, fill).expect("should rasterize");
        let px = img.get_pixel(2, 2).0;
        assert!(px[3] > 80 && px[3] < 180, "expected partial alpha, got {px:?}");
        assert!(px[0] >= 110 && px[1] >= 190 && px[2] >= 245, "RGB should remain straight overlay colour, got {px:?}");
    }

    #[test]
    fn fill_override_none_preserves_red_svg() {
        let img = rasterize_svg(RED_SVG, 4, 4, None).expect("should rasterize");
        let px = img.get_pixel(2, 2).0;
        // Premultiplied red pixel: R > G,B.
        assert!(
            px[0] > px[1] && px[0] > px[2] && px[3] > 0,
            "centre pixel should be red-ish, got {px:?}"
        );
    }

    #[test]
    fn contain_raster_preserves_source_aspect_ratio() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5">
            <rect width="10" height="5" fill="white"/>
        </svg>"#;
        let img = rasterize_svg_contained(svg, 20, 20, None, 0.5, 0.5).expect("should rasterize");

        assert_eq!(img.get_pixel(10, 4).0[3], 0, "top padding should remain transparent");
        assert!(img.get_pixel(10, 5).0[3] > 0, "contained image should start below the top padding");
        assert!(img.get_pixel(10, 14).0[3] > 0, "contained image should fill the centered band");
        assert_eq!(img.get_pixel(10, 15).0[3], 0, "bottom padding should remain transparent");
    }

    #[test]
    fn nine_slice_preserves_edge_regions_while_stretching_center() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect x="3" y="0" width="1" height="10" fill="white"/>
            <rect x="6" y="0" width="1" height="10" fill="white"/>
        </svg>"#;
        let img = rasterize_svg_nine_slice(svg, 40, 10, None, [0.4, 0.0, 0.6, 1.0], 1.0)
            .expect("should rasterize");

        assert!(img.get_pixel(3, 5).0[3] > 0, "left preserved band should keep its original x");
        assert_eq!(img.get_pixel(12, 5).0[3], 0, "center stretch should not move left line inward");
        assert!(img.get_pixel(36, 5).0[3] > 0, "right preserved band should stay near target edge");
    }

    /// Per-path recolour: a cross-cap-style SVG with a UNIFORM role (Critical)
    /// but MIXED opacity (100/70) — which `parse_uniform_colorstyle` rejects —
    /// must still recolour every path to the resolved role colour at its own
    /// id-encoded opacity, replacing the Adobe placeholder fills.
    #[test]
    fn recolour_colorstyle_svg_recolours_each_path_by_role_and_opacity() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="opacity:100_colorstyle:Critical_a_" fill="#6CB8C7" d="M0 0h1v1H0z"/>
            <polygon id="opacity:70_colorstyle:Critical_b_" fill="#C70050" points="0,0 1,0 1,1"/>
        </svg>"##;
        let out = recolour_colorstyle_svg(svg, |role| {
            (role == "Critical").then_some([240.0 / 255.0, 120.0 / 255.0, 16.0 / 255.0, 1.0])
        })
        .expect("a colorstyle SVG should be recoloured");
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("#6CB8C7") && !text.contains("#C70050"),
            "placeholder fills should be replaced: {text}"
        );
        assert_eq!(text.matches("#F07810").count(), 2, "both paths recoloured: {text}");
        assert!(text.contains("fill-opacity=\"1\""), "opacity 100 -> 1: {text}");
        assert!(text.contains("fill-opacity=\"0.7\""), "opacity 70 -> 0.7: {text}");
    }

    /// Mixed-ROLE recolour: a cross-line path (Accent1) and a cap path (Critical)
    /// in one SVG each resolve to their OWN role colour — the case a single
    /// whole-image fill_override cannot represent.
    #[test]
    fn recolour_colorstyle_svg_resolves_each_role_independently() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="opacity:85_colorstyle:Accent1_a_" fill="#6CB8C7" d="M0 0h1v1H0z"/>
            <path id="opacity:70_colorstyle:Critical_b_" fill="#6CB8C7" d="M2 2h1v1H2z"/>
        </svg>"##;
        let out = recolour_colorstyle_svg(svg, |role| match role {
            "Accent1" => Some([1.0, 0.0, 0.0, 1.0]),
            "Critical" => Some([0.0, 1.0, 0.0, 1.0]),
            _ => None,
        })
        .expect("a colorstyle SVG should be recoloured");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("#FF0000"), "Accent1 path -> red: {text}");
        assert!(text.contains("#00FF00"), "Critical path -> green: {text}");
        assert!(text.contains("fill-opacity=\"0.85\""), "Accent1 opacity 85: {text}");
    }

    /// A plain SVG (no `colorstyle:` ids) is left untouched so its authored fills
    /// render unchanged.
    #[test]
    fn recolour_colorstyle_svg_leaves_plain_svg_untouched() {
        assert!(recolour_colorstyle_svg(WHITE_SVG, |_| Some([1.0, 0.0, 0.0, 1.0])).is_none());
    }

    /// When no path's role resolves (unknown brand role), nothing is rewritten
    /// and the caller keeps the authored fills.
    #[test]
    fn recolour_colorstyle_svg_returns_none_when_no_role_resolves() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg">
            <path id="colorstyle:Accent1_a_" fill="#6CB8C7" d="M0 0h1v1H0z"/>
        </svg>"##;
        assert!(recolour_colorstyle_svg(svg, |_| None).is_none());
    }

    #[test]
    fn returns_none_for_zero_dimensions() {
        assert!(rasterize_svg(WHITE_SVG, 0, 16, None).is_none());
        assert!(rasterize_svg(WHITE_SVG, 16, 0, None).is_none());
    }

    #[test]
    fn returns_none_for_invalid_svg() {
        let result = rasterize_svg(b"not an svg at all", 16, 16, None);
        assert!(result.is_none(), "invalid SVG bytes should return None");
    }
}
