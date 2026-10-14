//! TDD tests for Phase 4: SWF EditText / Flash HTML rendering infrastructure.
//!
//! Covers: Flash HTML parser, EditText extraction from asset library,
//! and pixel-level rendering of an EditText placed on the stage timeline.

mod swf_helpers;

use starbreaker_ui::swf_render::edit_text::{FlashTextRun, parse_swf_html};
use starbreaker_ui::text::TextAlign;

// ── HTML parser tests ─────────────────────────────────────────────────────────

/// The real TargetStatus.swf id=22 initial_text fragment — all attributes parsed.
#[test]
fn parse_html_single_run_all_attrs() {
    let html = r##"<p align="center"><font face="$Furore" size="6" color="#ffffff" letterSpacing="0.600000">@hud_NoTarget</font></p>"##;
    let runs = parse_swf_html(html);
    assert_eq!(runs.len(), 1, "expected 1 run, got {}", runs.len());
    let r = &runs[0];
    assert_eq!(r.font_face, "$Furore");
    assert!((r.size_swf - 6.0).abs() < 0.001, "size_swf {}", r.size_swf);
    assert_eq!(r.color, [255u8, 255, 255, 255]);
    assert!((r.letter_spacing - 0.6).abs() < 0.001, "letter_spacing {}", r.letter_spacing);
    assert_eq!(r.align, TextAlign::Centre);
    assert_eq!(r.text, "@hud_NoTarget");
}

/// Non-loc-key text, left-aligned, red colour.
#[test]
fn parse_html_plain_text_with_color() {
    let html = r##"<p align="left"><font face="$OrbitronLight" size="8" color="#ff0000" letterSpacing="0">HELLO</font></p>"##;
    let runs = parse_swf_html(html);
    assert_eq!(runs.len(), 1);
    let r = &runs[0];
    assert_eq!(r.font_face, "$OrbitronLight");
    assert!((r.size_swf - 8.0).abs() < 0.001);
    assert_eq!(r.color, [255u8, 0, 0, 255]);
    assert_eq!(r.align, TextAlign::Left);
    assert_eq!(r.text, "HELLO");
}

/// Empty / whitespace-only inputs produce no runs.
#[test]
fn parse_html_empty_returns_empty() {
    assert!(parse_swf_html("").is_empty(), "empty string should give no runs");
    assert!(parse_swf_html("   ").is_empty(), "whitespace-only should give no runs");
}

/// Entity and character references stay inside the run they appear in.
#[test]
fn parse_html_entities_stay_in_one_run() {
    let html = r##"<p align="left"><font face="$Furore" size="8" color="#ffffff" letterSpacing="0">A &amp; B &lt;3&gt; &#169;</font></p>"##;
    let runs = parse_swf_html(html);
    assert_eq!(runs.len(), 1, "expected 1 run, got {runs:?}");
    assert_eq!(runs[0].text, "A & B <3> \u{a9}");
}

/// `is_loc_key` / `loc_key` helpers.
#[test]
fn text_run_loc_key_detection() {
    let loc_run = FlashTextRun {
        font_face: "$Furore".into(),
        size_swf: 6.0,
        color: [255, 255, 255, 255],
        letter_spacing: 0.0,
        align: TextAlign::Centre,
        text: "@hud_NoTarget".into(),
    };
    assert!(loc_run.is_loc_key(), "@hud_NoTarget must be a loc key");
    assert_eq!(loc_run.loc_key(), Some("hud_NoTarget"));

    let plain = FlashTextRun { text: "HELLO".into(), ..loc_run.clone() };
    assert!(!plain.is_loc_key(), "HELLO must not be a loc key");
    assert_eq!(plain.loc_key(), None);
}

// ── EditText extraction tests ─────────────────────────────────────────────────

/// `SwfAssetLibrary::get_edit_text` returns the id=8 HTML EditText from the
/// `make_state_sprites_swf` fixture (mirrors TargetStatus.swf id=22).
#[test]
fn extract_edit_text_from_state_sprites_fixture() {
    use starbreaker_ui::swf_assets::SwfAssetLibrary;

    let swf = swf_helpers::make_state_sprites_swf();
    let lib = SwfAssetLibrary::new(swf).expect("SwfAssetLibrary::new failed");

    let et = lib.get_edit_text(8).expect("EditText id=8 not found in library");
    assert!(et.is_html, "id=8 should be marked is_html=true");
    let text = et.initial_text.as_deref().unwrap_or("");
    assert!(
        text.contains("@hud_NoTarget"),
        "initial_text should contain @hud_NoTarget, got {text:?}"
    );
    assert_eq!(et.font_id, Some(5), "font_id should be 5 (imported $Furore)");
}

// ── Pixel rendering test ──────────────────────────────────────────────────────

/// An EditText placed on the stage timeline must be rasterised into non-zero
/// pixels when `draw_swf_stage_rgba` is called.
#[test]
fn draw_stage_with_edit_text_produces_pixels() {
    use starbreaker_ui::swf_assets::SwfAssetLibrary;
    use starbreaker_ui::swf_render::draw_swf_stage_rgba;
    use tiny_skia::Color;

    let swf = swf_helpers::make_edit_text_with_font_swf();
    let lib = SwfAssetLibrary::new(swf).expect("SwfAssetLibrary::new failed");
    let mut img = image::RgbaImage::new(100, 100);
    let white = Color::from_rgba8(255, 255, 255, 255);
    let drew = draw_swf_stage_rgba(&mut img, &lib, white, 1.0);
    assert!(drew, "draw_swf_stage_rgba returned false — EditText not rendered");
    let non_transparent = img.pixels().filter(|p| p[3] > 0).count();
    assert!(
        non_transparent > 0,
        "draw_swf_stage_rgba drew but produced 0 non-transparent pixels"
    );
}
