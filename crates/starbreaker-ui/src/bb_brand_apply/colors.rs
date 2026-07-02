use crate::bb_scene::{BbBorder, BbNode, BbNodeType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorStyleRole {
    Foreground,
    Surface,
}

pub(super) fn color_style_role_for_field(field_name: &str, node: &BbNode) -> ColorStyleRole {
    // An icon/image tinted via a colour overlay reads as a *foreground* element
    // (e.g. `Accent1` → the bright slot 0). A custom *shape* fill, by contrast, is
    // a *surface* (e.g. `Accent1` → the darker slot 4): a filled vector shape such
    // as the medical "fingerprint" is part of the screen surface, not a foreground
    // glyph. Treating custom-shape fills as Foreground rendered them in the light
    // slot-0 blue instead of the authored darker slot-4 blue.
    if field_name.eq_ignore_ascii_case("FillColor")
        && matches!(node.ty, BbNodeType::WidgetIcon | BbNodeType::WidgetImage)
        && color_overlay_enabled(node)
    {
        return ColorStyleRole::Foreground;
    }
    ColorStyleRole::Surface
}

fn color_overlay_enabled(node: &BbNode) -> bool {
    node.raw
        .get("enableColorOverlay")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || node
            .raw
            .get("svgFill")
            .and_then(|v| v.get("enableColorOverlay"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

pub(super) fn parse_color_value(
    value: &serde_json::Value,
    palette_source: &serde_json::Value,
    role: ColorStyleRole,
) -> Option<[f32; 4]> {
    if value
        .get("_Type_")
        .and_then(|ty| ty.as_str())
        .is_some_and(|ty| ty == "BuildingBlocks_ColorSolid")
    {
        return value
            .get("color")
            .and_then(|color| parse_color_value(color, palette_source, role));
    }

    if value.get("color").and_then(|v| v.as_str()).is_some() && value.get("r").is_none() {
        return parse_named_color(value, palette_source, role);
    }
    value.as_object().map(parse_literal_color)
}

fn parse_literal_color(color_obj: &serde_json::Map<String, serde_json::Value>) -> [f32; 4] {
    let r = color_obj
        .get("r")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let g = color_obj
        .get("g")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let b = color_obj
        .get("b")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let a = color_obj
        .get("a")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;

    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        [r / 255.0, g / 255.0, b / 255.0, a / 255.0]
    } else {
        [r, g, b, a]
    }
}

fn parse_named_color(
    value: &serde_json::Value,
    palette_source: &serde_json::Value,
    role: ColorStyleRole,
) -> Option<[f32; 4]> {
    let name = color_style_token(value)?;
    let alpha = value.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    let slot = color_style_slot_index(name, role)?;
    let color_styles = palette_source
        .get("colorStyles")
        .or_else(|| palette_source.get("_RecordValue_").and_then(|v| v.get("colorStyles")))?
        .as_array()?;
    let color_obj = color_styles.get(slot)?.get("color")?.as_object()?;
    let mut color = parse_literal_color(color_obj);
    color[3] *= alpha.clamp(0.0, 1.0);
    Some(color)
}

/// Map a `BuildingBlocks_ColorStyle` role name to an index into a brand style's
/// `colorStyles` palette array.
///
/// ## Authoritative source
/// The role name is the `BB_ColorStyle` DataCore enum; its integer value is the
/// direct index into `colorStyles`. Dumped from `Game2.dcb` (via
/// `Database::enum_options`), the canonical order is:
///
/// ```text
///  0 Base          1 Positive            2 Moderate         3 Critical
///  4 Accent1       5 Accent2             6 Bright           7 Selected
///  8 Disabled      9 Background         10 ContactNeutral  11 ContactParty
/// 12 ContactPositiveRep 13 ContactNegativeRep 14 ContactAgressive
/// 15 ContactUnknown 16 MissionObjectives
/// ```
///
/// To re-dump: a throwaway example over `Data\Game2.dcb` (needs `SC_DATA_P4K`)
/// iterating `Database::{enum_defs, enum_options, resolve_string2}` and printing
/// the enum whose options contain `Base`/`Bright`/`Accent1`.
///
/// ## Role divergences (each reference-verified; everything else is the enum)
/// The shared enum truth lives in [`crate::style::colour_roles`]; this
/// surface/brand-apply resolver diverges ONLY where a reference capture
/// proves the engine does:
/// 1. `Bright` → slot 0 (enum 6): medical `Bright` custom-shapes render
///    s_bioc slot-0 light-blue and the MFD footer's `Bright` selected-name
///    renders drak slot-0 orange. The compose-time *text* path
///    (`ir_compose::resolve_colour_token`) resolves `Bright`→6 for glyphs.
/// 2. `Accent1` foreground → slot 0 (enum 4): icon/image colour overlays
///    read as foreground (light blue), while custom-shape fills keep the
///    enum surface slot 4 (the medical fingerprint's darker blue) — see
///    [`ColorStyleRole`] / [`color_style_role_for_field`].
///
/// Non-enum aliases (`Accent3..5`, `Mid`, `Light`, `Gold`, `Surface`, …)
/// were deleted 2026-06-12: they occur in no DataCore record
/// (2026-06-12 token audit — no game record contains them; see crates/starbreaker-ui/docs/ui-process-improvements.md Part D).
fn color_style_slot_index(name: &str, role: ColorStyleRole) -> Option<usize> {
    if name.eq_ignore_ascii_case("Bright") {
        return Some(0);
    }
    if name.eq_ignore_ascii_case("Accent1") && role == ColorStyleRole::Foreground {
        return Some(0);
    }
    crate::style::colour_roles::bb_colour_style_enum_index(name)
}

pub(super) fn color_style_token(value: &serde_json::Value) -> Option<&str> {
    value
        .get("color")
        .and_then(|v| v.as_str())
        .filter(|name| !name.trim().is_empty())
}

/// Ensure `node.border` is `Some(…)`, initializing to default if `None`.
pub(super) fn ensure_border(node: &mut BbNode) {
    if node.border.is_none() {
        node.border = Some(BbBorder::default());
    }
}

/// Write a color to `node.raw` as an object `{r, g, b, a}`.
pub(super) fn write_color_to_raw(field_name: &str, color: [f32; 4], node: &mut BbNode) {
    let color_obj = serde_json::json!({
        "r": color[0],
        "g": color[1],
        "b": color[2],
        "a": color[3],
    });
    node.raw
        .as_object_mut()
        .and_then(|obj| obj.insert(field_name.to_string(), color_obj));
}

pub(super) fn write_color_token_to_raw(field_name: &str, token: Option<&str>, node: &mut BbNode) {
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        // A token-less (literal / `ColorSolid`) application must DROP any stale
        // lower-tier token — a leftover token shadows the literal at draw time
        // (the Filled-button icon kept the overlay `Base` tint; review F5 /
        // ledger 103). Applies to every colour field, not just FillColor.
        node.raw
            .as_object_mut()
            .and_then(|obj| obj.remove(&format!("{field_name}Token")));
        return;
    };
    node.raw.as_object_mut().and_then(|obj| {
        obj.insert(
            format!("{field_name}Token"),
            serde_json::Value::String(token.to_string()),
        )
    });
}

/// Palettes for resolving named colour roles in style modifiers.
///
/// `chrome` covers `Background*` / `Border*` fields and may be the brand's
/// `BuildingBlocks_Style` record when the entry's own container carries no
/// `colorStyles` (verified against the MFD footer reference). `fills` covers
/// `FillColor`/`StrokeColor`/other fields and keeps the container-only
/// behaviour: the medical platinum references show authored Fill roles do NOT
/// map 1:1 onto the compose slot resolvers (e.g. the BIOC bottom-bar's
/// authored `Base` renders the darker surface slot), so widening fills to the
/// fetched palette is deferred until that mapping is reference-verified.
pub struct PaletteSources<'a> {
    pub fills: &'a serde_json::Value,
    pub chrome: &'a serde_json::Value,
}

impl<'a> PaletteSources<'a> {
    /// Both field classes resolve against the same palette.
    pub fn uniform(palette: &'a serde_json::Value) -> Self {
        Self { fills: palette, chrome: palette }
    }

    pub(super) fn for_field(&self, field_name: &str) -> &'a serde_json::Value {
        if field_name.starts_with("Background") || field_name.starts_with("Border") {
            self.chrome
        } else {
            self.fills
        }
    }
}
