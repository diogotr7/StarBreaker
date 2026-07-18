//! SWF static-atom extractors (bitmaps, shapes, fonts, exports, edit-text,
//! timeline labels, font metrics).
//!
//! Each extractor has a `*_from_tags(&[Tag])` core that iterates an
//! already-parsed tag slice, plus a thin `extract_*(bytes)` wrapper that
//! decompresses+parses once via [`parse_tags`] and delegates. The wrappers keep
//! the standalone public API (examples/tests) working; `SwfAssetLibrary` calls
//! the `*_from_tags` cores directly so one construction decompresses+parses the
//! SWF exactly once.

use std::collections::HashMap;

use image::RgbaImage;
use swf::{CharacterId, Tag};

use crate::error::UiError;

use super::decode::{decode_jpeg3, decode_lossless};
use super::types::{EditTextRecord, FontGlyph, FontGlyphSet, ShapeRecord, SwfEditTextMetrics};

#[cfg(test)]
pub(crate) static SWF_PARSE_COUNT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Decompress a SWF exactly once. Returns the owned buffer; the caller calls
/// `swf::parse_swf(&buf)` to borrow its tags (which cannot cross this fn
/// boundary — `Tag<'a>` borrows `buf`). Single choke point so tests can assert
/// one decompress+parse per library construction.
pub(crate) fn parse_tags(bytes: &[u8]) -> Result<swf::SwfBuf, UiError> {
    #[cfg(test)]
    SWF_PARSE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(swf::decompress_swf(std::io::Cursor::new(bytes))?)
}

pub fn extract_bitmaps(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, RgbaImage>, UiError> {
    let buf = parse_tags(swf_bytes)?;
    let parsed = swf::parse_swf(&buf)?;
    extract_bitmaps_from_tags(&parsed.tags)
}

pub(crate) fn extract_bitmaps_from_tags(
    tags: &[Tag],
) -> Result<HashMap<CharacterId, RgbaImage>, UiError> {
    let mut out: HashMap<CharacterId, RgbaImage> = HashMap::new();
    let mut skipped_action_tags = 0u32;
    for tag in tags {
        match tag {
            Tag::DefineBitsLossless(bmp) => match decode_lossless(bmp) {
                Ok(img) => {
                    out.insert(bmp.id, img);
                }
                Err(e) => {
                    log::warn!("skipping DefineBitsLossless id={}: {e}", bmp.id);
                }
            },
            Tag::DefineBitsJpeg2 { id, jpeg_data } => match image::load_from_memory(jpeg_data) {
                Ok(dyn_img) => {
                    out.insert(*id, dyn_img.to_rgba8());
                }
                Err(e) => {
                    log::warn!("skipping DefineBitsJpeg2 id={id}: {e}");
                }
            },
            Tag::DefineBitsJpeg3(jpeg3) => match decode_jpeg3(jpeg3) {
                Ok(img) => {
                    out.insert(jpeg3.id, img);
                }
                Err(e) => {
                    log::warn!("skipping DefineBitsJpeg3/4 id={}: {e}", jpeg3.id);
                }
            },
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                skipped_action_tags += 1;
            }
            _ => {}
        }
    }
    if skipped_action_tags > 0 {
        log::debug!("extract_bitmaps: skipped {skipped_action_tags} action tag(s)");
    }
    Ok(out)
}

pub fn extract_shapes(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, ShapeRecord>, UiError> {
    let buf = parse_tags(swf_bytes)?;
    let parsed = swf::parse_swf(&buf)?;
    extract_shapes_from_tags(&parsed.tags)
}

pub(crate) fn extract_shapes_from_tags(
    tags: &[Tag],
) -> Result<HashMap<CharacterId, ShapeRecord>, UiError> {
    let mut out: HashMap<CharacterId, ShapeRecord> = HashMap::new();
    let mut skipped_action_tags = 0u32;
    for tag in tags {
        match tag {
            Tag::DefineShape(shape) => {
                let rec = ShapeRecord {
                    id: shape.id,
                    shape_bounds: shape.shape_bounds.clone(),
                    fill_styles: shape.styles.fill_styles.clone(),
                    line_styles: shape.styles.line_styles.clone(),
                    records: shape.shape.clone(),
                };
                out.insert(shape.id, rec);
            }
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                skipped_action_tags += 1;
            }
            _ => {}
        }
    }
    if skipped_action_tags > 0 {
        log::debug!("extract_shapes: skipped {skipped_action_tags} action tag(s)");
    }
    Ok(out)
}

pub fn extract_fonts(swf_bytes: &[u8]) -> Result<HashMap<CharacterId, FontGlyphSet>, UiError> {
    let buf = parse_tags(swf_bytes)?;
    let parsed = swf::parse_swf(&buf)?;
    extract_fonts_from_tags(&parsed.tags)
}

pub(crate) fn extract_fonts_from_tags(
    tags: &[Tag],
) -> Result<HashMap<CharacterId, FontGlyphSet>, UiError> {
    let mut out: HashMap<CharacterId, FontGlyphSet> = HashMap::new();
    let mut skipped_action_tags = 0u32;
    for tag in tags {
        match tag {
            Tag::DefineFont(fv1) => {
                let glyphs = fv1
                    .glyphs
                    .iter()
                    .map(|records| FontGlyph {
                        code: None,
                        advance: None,
                        shape_records: records.clone(),
                    })
                    .collect();
                out.insert(
                    fv1.id,
                    FontGlyphSet {
                        id: fv1.id,
                        name: String::new(),
                        is_bold: false,
                        is_italic: false,
                        ascent: None,
                        descent: None,
                        leading: None,
                        glyphs,
                    },
                );
            }
            Tag::DefineFont2(font) => {
                let glyphs = font
                    .glyphs
                    .iter()
                    .map(|g| FontGlyph {
                        code: Some(g.code),
                        advance: Some(g.advance),
                        shape_records: g.shape_records.clone(),
                    })
                    .collect();
                let (ascent, descent, leading) = font
                    .layout
                    .as_ref()
                    .map(|l| (Some(l.ascent), Some(l.descent), Some(l.leading)))
                    .unwrap_or((None, None, None));
                out.insert(
                    font.id,
                    FontGlyphSet {
                        id: font.id,
                        name: font.name.to_string_lossy(swf::UTF_8),
                        is_bold: font.flags.contains(swf::FontFlag::IS_BOLD),
                        is_italic: font.flags.contains(swf::FontFlag::IS_ITALIC),
                        ascent,
                        descent,
                        leading,
                        glyphs,
                    },
                );
            }
            Tag::DefineFont4(font4) => {
                out.insert(
                    font4.id,
                    FontGlyphSet {
                        id: font4.id,
                        name: font4.name.to_string_lossy(swf::UTF_8),
                        is_bold: font4.is_bold,
                        is_italic: font4.is_italic,
                        ascent: None,
                        descent: None,
                        leading: None,
                        glyphs: vec![],
                    },
                );
            }
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                skipped_action_tags += 1;
            }
            _ => {}
        }
    }

    for tag in tags {
        if let Tag::DefineFontInfo(info) = tag
            && let Some(font) = out.get_mut(&info.id)
        {
            if font.name.is_empty() {
                font.name = info.name.to_string_lossy(swf::UTF_8);
            }
            font.is_bold = info.flags.contains(swf::FontInfoFlag::IS_BOLD);
            font.is_italic = info.flags.contains(swf::FontInfoFlag::IS_ITALIC);
            for (glyph, &code) in font.glyphs.iter_mut().zip(info.code_table.iter()) {
                glyph.code = Some(code);
            }
        }
    }

    if skipped_action_tags > 0 {
        log::debug!("extract_fonts: skipped {skipped_action_tags} action tag(s)");
    }
    Ok(out)
}

pub fn extract_exported_symbols(swf_bytes: &[u8]) -> Result<HashMap<String, CharacterId>, UiError> {
    let buf = parse_tags(swf_bytes)?;
    let parsed = swf::parse_swf(&buf)?;
    extract_exported_symbols_from_tags(&parsed.tags)
}

pub(crate) fn extract_exported_symbols_from_tags(
    tags: &[Tag],
) -> Result<HashMap<String, CharacterId>, UiError> {
    let mut out: HashMap<String, CharacterId> = HashMap::new();
    let mut skipped_action_tags = 0u32;
    for tag in tags {
        match tag {
            Tag::ExportAssets(exports) => {
                for asset in exports {
                    out.insert(asset.name.to_string_lossy(swf::UTF_8), asset.id);
                }
            }
            Tag::SymbolClass(links) => {
                for link in links {
                    out.insert(link.class_name.to_string_lossy(swf::UTF_8), link.id);
                }
            }
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                skipped_action_tags += 1;
            }
            _ => {}
        }
    }
    if skipped_action_tags > 0 {
        log::debug!("extract_exported_symbols: skipped {skipped_action_tags} action tag(s)");
    }
    Ok(out)
}

/// Extract all `DefineEditText` characters from a SWF.
///
/// Each record captures the character ID, layout bounds, HTML flag, initial
/// text (with `@` loc keys unresolved), and font reference.
pub fn extract_edit_text_records(
    swf_bytes: &[u8],
) -> Result<HashMap<CharacterId, EditTextRecord>, UiError> {
    let buf = parse_tags(swf_bytes)?;
    let parsed = swf::parse_swf(&buf)?;
    extract_edit_text_records_from_tags(&parsed.tags)
}

pub(crate) fn extract_edit_text_records_from_tags(
    tags: &[Tag],
) -> Result<HashMap<CharacterId, EditTextRecord>, UiError> {
    let mut out: HashMap<CharacterId, EditTextRecord> = HashMap::new();
    for tag in tags {
        let Tag::DefineEditText(edit) = tag else {
            continue;
        };
        let rec = EditTextRecord {
            id: edit.id(),
            bounds: edit.bounds().clone(),
            is_html: edit.is_html(),
            initial_text: edit.initial_text().map(|s| s.to_string_lossy(swf::UTF_8)),
            font_id: edit.font_id(),
            font_height_px: edit.height().map(|h| h.to_pixels() as f32),
        };
        out.insert(rec.id, rec);
    }
    Ok(out)
}

/// Extract a `label → frame_index` map from the SWF main timeline.
///
/// `FrameLabel` tags before the first `ShowFrame` map to frame 0; before the
/// second `ShowFrame` to frame 1; and so on.  Only the main timeline is
/// scanned (not sprites).
pub(crate) fn extract_main_timeline_labels_from_tags(
    tags: &[Tag],
) -> Result<HashMap<String, u32>, UiError> {
    let mut out: HashMap<String, u32> = HashMap::new();
    let mut current_frame: u32 = 0;
    for tag in tags {
        match tag {
            Tag::FrameLabel(label) => {
                out.insert(label.label.to_string_lossy(swf::UTF_8), current_frame);
            }
            Tag::ShowFrame => {
                current_frame += 1;
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn extract_font_edit_text_metrics(
    swf_bytes: &[u8],
) -> Result<HashMap<String, SwfEditTextMetrics>, UiError> {
    let buf = parse_tags(swf_bytes)?;
    let parsed = swf::parse_swf(&buf)?;
    extract_font_edit_text_metrics_from_tags(&parsed.tags)
}

pub(crate) fn extract_font_edit_text_metrics_from_tags(
    tags: &[Tag],
) -> Result<HashMap<String, SwfEditTextMetrics>, UiError> {
    let mut out: HashMap<String, SwfEditTextMetrics> = HashMap::new();
    let mut imported_font_symbols: HashMap<CharacterId, String> = HashMap::new();
    let mut skipped_action_tags = 0u32;

    for tag in tags {
        match tag {
            Tag::ImportAssets { imports, .. } => {
                for import in imports {
                    imported_font_symbols.insert(import.id, import.name.to_string_lossy(swf::UTF_8));
                }
            }
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                skipped_action_tags += 1;
            }
            _ => {}
        }
    }

    for tag in tags {
        let Tag::DefineEditText(edit) = tag else {
            continue;
        };
        let Some(font_id) = edit.font_id() else {
            continue;
        };
        let Some(symbol) = imported_font_symbols.get(&font_id) else {
            continue;
        };
        let Some(height) = edit.height() else {
            continue;
        };
        let bounds = edit.bounds();
        let metrics = SwfEditTextMetrics {
            height_px: height.get() as f32 / 20.0,
            bounds_y_min_px: bounds.y_min.get() as f32 / 20.0,
            bounds_y_max_px: bounds.y_max.get() as f32 / 20.0,
        };
        out.entry(symbol.clone()).or_insert(metrics);
    }

    if skipped_action_tags > 0 {
        log::debug!("extract_font_edit_text_metrics: skipped {skipped_action_tags} action tag(s)");
    }
    Ok(out)
}
