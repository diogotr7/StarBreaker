//! Flash HTML parser and SWF `DefineEditText` rasteriser.
//!
//! Public items:
//! - [`FlashTextRun`]  — one styled text run extracted from Flash HTML.
//! - [`parse_swf_html`] — parse a Flash-HTML `initial_text` fragment.
//!
//! Internal item (used by `stage::draw_character`):
//! - `draw_edit_text` — render a `DefineEditText` character into a `Pixmap`.
//!
//! Flash HTML is an XML-shaped subset, not web HTML.  It supports only the
//! tags used by ActionScript's `TextField.html`: `<p align=…>` and
//! `<font face=… size=… color=… letterSpacing=… kerning=…>`.
//! Parse is done with `quick-xml 0.37`; no external HTML5 engine is needed.

use image::RgbaImage;
use tiny_skia::{Pixmap, Rect as TskRect};

use crate::bb_layout::Rect;
use crate::swf_assets::{EditTextRecord, SwfAssetLibrary};
use crate::text::{TextAlign, TextRenderer, VerticalAlign};

use super::rgba::composite_rgba_over_pixmap;

// ── Public types ──────────────────────────────────────────────────────────────

/// One styled text run extracted from a Flash HTML `initial_text` fragment.
///
/// A single `<p>` / `<font>` pair in the HTML produces one `FlashTextRun`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlashTextRun {
    /// Raw value of the `face` attribute (may begin with `$` e.g. `"$Furore"`).
    pub font_face: String,
    /// Font size from the `size` attribute (Flash units — caller scales to px).
    pub size_swf: f32,
    /// Text colour as RGBA.  Parsed from `color="#rrggbb"`.
    pub color: [u8; 4],
    /// Letter-spacing from the `letterSpacing` attribute.
    pub letter_spacing: f32,
    /// Paragraph alignment from `<p align=…>`.
    pub align: TextAlign,
    /// Text content of the `<font>` element (may start with `@` loc key).
    pub text: String,
}

impl FlashTextRun {
    /// Returns `true` when `text` begins with `@` (a loc key reference).
    pub fn is_loc_key(&self) -> bool {
        self.text.starts_with('@')
    }

    /// Returns the loc key name (text without leading `@`), or `None`.
    pub fn loc_key(&self) -> Option<&str> {
        self.text.strip_prefix('@')
    }
}

// ── HTML parser ───────────────────────────────────────────────────────────────

/// Parse a Flash-HTML `initial_text` fragment into styled text runs.
///
/// Handles `<p align=…>` and `<font face=… size=… color=… letterSpacing=…>`.
/// Unknown elements are silently ignored.  Returns an empty `Vec` for empty
/// input or if the fragment contains no `<font>` text.
pub fn parse_swf_html(html: &str) -> Vec<FlashTextRun> {
    use quick_xml::events::Event;
    use quick_xml::escape::resolve_predefined_entity;
    use quick_xml::{Reader, XmlVersion};

    if html.trim().is_empty() {
        return vec![];
    }

    let mut runs = Vec::new();
    let mut reader = Reader::from_str(html);
    reader.config_mut().trim_text(false);

    let mut current_align = TextAlign::Left;
    let mut current_face = String::new();
    let mut current_size = 12.0f32;
    let mut current_color = [255u8, 255, 255, 255];
    let mut current_letter_spacing = 0.0f32;
    let mut in_font = false;

    // Text arrives as `Text` pieces split by `GeneralRef` (entity / character
    // reference) events; gather them until the next markup event, then emit
    // one run.
    let mut pending = String::new();
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf);
        if !matches!(event, Ok(Event::Text(_) | Event::GeneralRef(_))) && !pending.is_empty() {
            runs.push(FlashTextRun {
                font_face: current_face.clone(),
                size_swf: current_size,
                color: current_color,
                letter_spacing: current_letter_spacing,
                align: current_align,
                text: std::mem::take(&mut pending),
            });
        }
        match event {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                "p" | "P" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == "align" {
                            let val = attr
                                .normalized_value(XmlVersion::Implicit1_0)
                                .map(|c| c.into_owned())
                                .unwrap_or_default();
                            current_align = match val.to_lowercase().as_str() {
                                "center" | "centre" => TextAlign::Centre,
                                "right" => TextAlign::Right,
                                _ => TextAlign::Left,
                            };
                        }
                    }
                }
                "font" | "FONT" => {
                    for attr in e.attributes().flatten() {
                        let val = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map(|c| c.into_owned())
                            .unwrap_or_default();
                        match attr.key.as_ref() {
                            "face" => current_face = val,
                            "size" => current_size = val.parse().unwrap_or(12.0),
                            "color" => current_color = parse_color_hex(&val),
                            "letterSpacing" => {
                                current_letter_spacing = val.parse().unwrap_or(0.0);
                            }
                            _ => {}
                        }
                    }
                    in_font = true;
                }
                _ => {}
            },
            Ok(Event::Text(ref e)) if in_font => pending.push_str(&e.xml10_content()),
            Ok(Event::GeneralRef(ref e)) if in_font => match e.resolve_char_ref() {
                Ok(Some(c)) => pending.push(c),
                _ => match resolve_predefined_entity(e) {
                    Some(s) => pending.push_str(s),
                    // Unknown entity: keep it verbatim rather than lose the text.
                    None => {
                        pending.push('&');
                        pending.push_str(e);
                        pending.push(';');
                    }
                },
            },
            Ok(Event::End(ref e)) => {
                if matches!(e.name().as_ref(), "font" | "FONT") {
                    in_font = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    runs
}

// ── EditText renderer ─────────────────────────────────────────────────────────

/// Render a `DefineEditText` character (`edit`) into `pixmap`.
///
/// `matrix` is the fully-composed SWF transform for this character (from
/// `draw_character`).  `sw/sh` are stage dimensions, `sx/sy` are the
/// stage-to-pixmap scale factors, `origin` is the pixmap dest rect.
///
/// `loc_fn` resolves `@key` strings: it receives the key without the leading
/// `@` and returns the localised text, or `None` to keep the raw key.
pub(super) fn draw_edit_text(
    pixmap: &mut Pixmap,
    assets: &SwfAssetLibrary,
    edit: &EditTextRecord,
    matrix: &swf::Matrix,
    _sw: f32,
    _sh: f32,
    sx: f32,
    sy: f32,
    origin: TskRect,
    alpha: f32,
    loc_fn: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let raw_text = match &edit.initial_text {
        Some(t) => t.as_str(),
        None => return false,
    };

    // Build text runs: parse HTML or create a single plain-text run.
    let runs: Vec<FlashTextRun> = if edit.is_html {
        parse_swf_html(raw_text)
    } else {
        vec![FlashTextRun {
            font_face: String::new(),
            size_swf: edit.font_height_px.unwrap_or(12.0),
            color: [255, 255, 255, 255],
            letter_spacing: 0.0,
            align: TextAlign::Left,
            text: raw_text.to_string(),
        }]
    };

    if runs.is_empty() {
        return false;
    }

    // Transform the EditText bounds through the full placement matrix, then map
    // stage space into the dest rect via sx/sy.  Transforming all four corners
    // and taking their axis-aligned bounding box honours scale and translation
    // (and approximates rotation/skew, which the axis-aligned text renderer
    // cannot reproduce exactly).  For an identity/translation matrix this is
    // identical to using the raw bounds plus tx/ty.
    let bounds = &edit.bounds;
    let bx0 = bounds.x_min.to_pixels() as f32;
    let by0 = bounds.y_min.to_pixels() as f32;
    let bx1 = bounds.x_max.to_pixels() as f32;
    let by1 = bounds.y_max.to_pixels() as f32;

    let ma = matrix.a.to_f32();
    let mb = matrix.b.to_f32();
    let mc = matrix.c.to_f32();
    let md = matrix.d.to_f32();
    let mtx = matrix.tx.to_pixels() as f32;
    let mty = matrix.ty.to_pixels() as f32;
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (px, py) in [(bx0, by0), (bx1, by0), (bx1, by1), (bx0, by1)] {
        let stage_x = ma * px + mc * py + mtx;
        let stage_y = mb * px + md * py + mty;
        min_x = min_x.min(stage_x);
        min_y = min_y.min(stage_y);
        max_x = max_x.max(stage_x);
        max_y = max_y.max(stage_y);
    }
    let dest_x = origin.left() + min_x * sx;
    let dest_y = origin.top() + min_y * sy;
    let dest_w = ((max_x - min_x) * sx).max(1.0);
    let dest_h = ((max_y - min_y) * sy).max(1.0);

    // Font size scales by the matrix's vertical scale as well as the stage→dest
    // scale (|d| for an axis-aligned matrix; the vector length handles rotation).
    let matrix_scale_y = (mb * mb + md * md).sqrt();

    let text_rect = Rect { x: dest_x, y: dest_y, w: dest_w, h: dest_h };

    let pw = pixmap.width();
    let ph = pixmap.height();
    let mut img = RgbaImage::new(pw, ph);
    let renderer = TextRenderer::new();
    let mut drew_any = false;

    for run in &runs {
        let resolved = if let Some(key) = run.loc_key() {
            loc_fn(key).unwrap_or_else(|| run.text.clone())
        } else {
            run.text.clone()
        };
        if resolved.is_empty() {
            continue;
        }

        // Resolve font: HTML face attribute (strip leading `$`) → by ID fallback.
        let swf_font = find_font_by_face(assets, &run.font_face)
            .or_else(|| edit.font_id.and_then(|id| assets.get_font(id)));
        let Some(swf_font) = swf_font else {
            continue;
        };

        // size_swf is in Flash font-size units; scale by the matrix vertical
        // scale and the stage→dest scale to get destination pixels.
        let size_px = (run.size_swf * matrix_scale_y * sy).max(1.0);
        let colour = apply_alpha(run.color, alpha);

        if renderer.draw_swf_font(
            &mut img,
            &resolved,
            text_rect,
            swf_font,
            None,
            size_px,
            colour,
            run.align,
            VerticalAlign::Centre,
            // The HTML run's letterSpacing is per-glyph tracking, not an
            // interline value — route it to the tracking parameter.
            None,
            run.letter_spacing,
        ) {
            drew_any = true;
        }
    }

    if !drew_any {
        return false;
    }

    composite_rgba_over_pixmap(&img, pixmap);
    true
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Parse `#rrggbb` (or `#rgb`) to `[r, g, b, 255]`.  Returns white on error.
fn parse_color_hex(s: &str) -> [u8; 4] {
    let s = s.trim_start_matches('#');
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16);
            let g = u8::from_str_radix(&s[2..4], 16);
            let b = u8::from_str_radix(&s[4..6], 16);
            match (r, g, b) {
                (Ok(r), Ok(g), Ok(b)) => [r, g, b, 255],
                _ => [255, 255, 255, 255],
            }
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).map(|v| v * 17);
            let g = u8::from_str_radix(&s[1..2], 16).map(|v| v * 17);
            let b = u8::from_str_radix(&s[2..3], 16).map(|v| v * 17);
            match (r, g, b) {
                (Ok(r), Ok(g), Ok(b)) => [r, g, b, 255],
                _ => [255, 255, 255, 255],
            }
        }
        _ => [255, 255, 255, 255],
    }
}

/// Look up a font by the HTML `face` attribute value, stripping a leading `$`.
fn find_font_by_face<'a>(
    assets: &'a SwfAssetLibrary,
    face: &str,
) -> Option<&'a crate::swf_assets::FontGlyphSet> {
    let name = face.strip_prefix('$').unwrap_or(face);
    if name.is_empty() {
        return None;
    }
    assets.find_font_by_name(name)
}

/// Scale an RGBA colour's alpha channel by the global `alpha` factor.
fn apply_alpha(colour: [u8; 4], alpha: f32) -> [u8; 4] {
    let a = (colour[3] as f32 * alpha.clamp(0.0, 1.0)) as u8;
    [colour[0], colour[1], colour[2], a]
}

