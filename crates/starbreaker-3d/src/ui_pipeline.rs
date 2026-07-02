//! Bridge between the decomposed export pipeline and `starbreaker-ui`.
//!
//! Sub-modules: `canvas_fetcher` — [`CanvasFetcher`] with O(1) name index (B2b);
//! `p4k_fetchers` — P4K-backed [`SwfFetcher`] and [`AssetFetcher`]; `style_fetcher` —
//! manufacturer style resolution. Exposes [`render_ui_binding_png`] and
//! [`UiLocData`] as the call-sites for `decomposed.rs`.

use std::collections::HashMap;
use std::str::FromStr;

use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;
use starbreaker_ui::pipeline::{CanvasFetcher, PipelineInputs, UiBindingView};

use crate::types::UiBinding;

mod canvas_fetcher;
mod hologram_fetcher;
mod p4k_fetchers;
pub(crate) mod screen_aspect;
mod ship_values;
mod style_fetcher;
use canvas_fetcher::DatacoreCanvasFetcher;
use hologram_fetcher::P4kHologramFetcher;
use p4k_fetchers::{P4kAssetFetcher, P4kSwfFetcher};
pub use ship_values::UiShipData;
use style_fetcher::ManufacturerStyleFetcher;

pub(super) fn datacore_ui_lookup_type_names() -> &'static [&'static str] {
    &[
        // DataCore stores the full name as "<Type>.<Stem>" in name_offset
        // (e.g. "BuildingBlocks_Canvas.M_Eng_MFDContent").  These are all
        // record families the UI resolver fetches through file-URL basenames.
        "BuildingBlocks_Canvas",
        "BuildingBlocks_Style",
        "BuildingBlocks_FontStyle",
        "BuildingBlocks_Timeline",
        "TagDatabase",
        // MFD responsive layout: the aspect→tag library (e.g.
        // `AspectRatioToTag_MFD`) the pipeline reads to map a screen aspect to a
        // "Content Canvas Scaling" layout tag.
        "BuildingBlocks_AspectRatioLibrary",
    ]
}

// ──────────────────────────────────────────────────────────────────────────────
// Shared per-export state (B2d)
// ──────────────────────────────────────────────────────────────────────────────

/// Localization data loaded once per export and shared across all binding renders.
///
/// Both fields are `Send + Sync`, so a reference can be borrowed from a `par_iter`
/// closure without cloning the underlying data.
pub struct UiLocData {
    /// Localization key→display-string map from `global.ini`.
    pub map: HashMap<String, String>,
    /// INI-backed loc fetcher, used as `Option<&dyn LocFetcher>`.
    pub ini: starbreaker_ui::bb_loc_p4k::IniLocFetcher,
}

impl UiLocData {
    /// Load localization from the P4K once; pass to every [`render_ui_binding_png`] call.
    pub fn load(p4k: &MappedP4k) -> Self {
        Self {
            map: crate::pipeline::load_localization_map(p4k),
            ini: starbreaker_ui::bb_loc_p4k::load_global_ini(|path| p4k.read_file(path).ok()),
        }
    }
}

/// Build the shared default-value registry once for an export.
///
/// The registry is a pure function of the localization map and the per-ship
/// derived values — both constant across an entire export — so constructing it
/// once here and passing it to every [`render_ui_binding_png`] call avoids
/// cloning and re-merging the multi-MB localization map for each of a ship's
/// hundreds of screen renders, which dominated capital-ship export time.
pub fn build_default_registry(
    loc_data: &UiLocData,
    ship_data: &UiShipData,
) -> starbreaker_ui::DefaultValueRegistry {
    starbreaker_ui::DefaultValueRegistry::with_pipeline_defaults_and_derived_values(
        Some(loc_data.map.clone()),
        ship_data.derived_values.as_ref(),
    )
}

// ──────────────────────────────────────────────────────────────────────────────
// Public entry points
// ──────────────────────────────────────────────────────────────────────────────

/// Render `binding` to a PNG byte vector using live DataCore + P4K access.
///
/// `loc_data` must be pre-loaded with [`UiLocData::load`] once per export and
/// reused across bindings — loading it per-binding is wasteful but not wrong.
///
/// Returns the PNG bytes on success, or a descriptive error string on failure.
/// Callers should log the error and set `generated_image_path = None` rather
/// than propagating.
pub fn render_ui_binding_png(
    binding: &UiBinding,
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_manufacturer_id: Option<&str>,
    loc_data: &UiLocData,
    ship_data: &UiShipData,
    defaults_registry: Option<&starbreaker_ui::DefaultValueRegistry>,
    root_entity_name: Option<&str>,
) -> Result<Vec<u8>, String> {
    let t_ui = std::env::var("SB_UI_TIMING").ok().map(|_| std::time::Instant::now());
    let canvas_fetcher = DatacoreCanvasFetcher::new(db);
    let hologram_fetcher = P4kHologramFetcher {
        p4k,
        db,
        root_entity_name,
        radar_heading_deg: ship_data.radar_heading_deg(),
    };
    let view = UiBindingView {
        canvas_guid: binding.canvas_guid.as_deref(),
        content_canvas_guid: binding.content_canvas_guid.as_deref(),
        binding_kind: Some(&binding.binding_kind),
        manufacturer_id: root_manufacturer_id,
        helper_name: binding.helper_name.as_deref(),
        default_view_index: binding.dashboard_view_index,
        default_screen_slot: binding.dashboard_screen_slot,
        screen_name_loc_key: binding.screen_name_loc_key.as_deref(),
        transit_location_loc_key: binding.transit_location_loc_key.as_deref(),
        host_swf_path: binding.owner_source_file.as_deref(),
        screen_aspect_w_over_h: binding.ui_screen_aspect_w_over_h,
    };
    let effective_guid = binding
        .content_canvas_guid
        .as_deref()
        .filter(|g| !g.is_empty())
        .or_else(|| binding.canvas_guid.as_deref().filter(|g| !g.is_empty()));
    let authored_canvas_size = effective_guid
        .and_then(|guid| canvas_fetcher.fetch_canvas_json(guid).ok())
        .and_then(|json| authored_canvas_size(&json));
    let target_size = binding_target_size(&binding.binding_kind, authored_canvas_size);
    let animation_sample_percent = if binding.binding_kind == "mfd" {
        Some(0.0)
    } else {
        Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT)
    };
    let inputs = PipelineInputs {
        binding: &view,
        canvas_fetcher: &canvas_fetcher,
        swf_fetcher: &P4kSwfFetcher { p4k },
        style_fetcher: &ManufacturerStyleFetcher { db },
        asset_fetcher: &P4kAssetFetcher { p4k },
        target_size,
        // Phase 11: postprocess is disabled while compose.rs is the magenta-grid
        // placeholder.  The tint/scanline/vignette passes assume *lit* pixels
        // come from a real canvas render; running them over the placeholder
        // would mask the "not yet rendered" signal.  Re-enable in Phase 13
        // once the paint engine produces real content.
        apply_postprocess: false,
        animation_sample_percent,
        // When the caller supplies a pre-built default registry, the
        // localization map and derived values are only needed to construct that
        // registry — so skip cloning them into every render's inputs (the clone
        // + per-render registry merge dominated UI-dense ship exports).
        localization_map: if defaults_registry.is_some() {
            None
        } else {
            Some(loc_data.map.clone())
        },
        loc_fetcher: Some(&loc_data.ini),
        derived_values: if defaults_registry.is_some() {
            None
        } else {
            ship_data.derived_values.clone()
        },
        hologram_fetcher: Some(&hologram_fetcher),
    };
    let _ = texture_mip; // size is fixed per binding_kind; mip is applied at texture level
    let result = match defaults_registry {
        Some(registry) => {
            starbreaker_ui::pipeline::render_for_binding_with_registry(&inputs, registry)
        }
        None => starbreaker_ui::pipeline::render_for_binding(&inputs),
    }
    .map_err(|e| e.to_string());
    if let Some(t) = t_ui {
        log::info!(
            "[timing][ui] binding={} kind={} total={:.3}s",
            binding.helper_name.as_deref().unwrap_or("?"),
            binding.binding_kind,
            t.elapsed().as_secs_f32(),
        );
    }
    result
}

/// Compile `binding` to canonical UI IR JSON using the same live DataCore + P4K
/// inputs as [`render_ui_binding_png`].
pub fn compile_ui_binding_ir_json(
    binding: &UiBinding,
    db: &Database<'_>,
    p4k: &MappedP4k,
    texture_mip: u32,
    root_manufacturer_id: Option<&str>,
    loc_data: &UiLocData,
    ship_data: &UiShipData,
) -> Result<String, String> {
    let canvas_fetcher = DatacoreCanvasFetcher::new(db);
    let view = UiBindingView {
        canvas_guid: binding.canvas_guid.as_deref(),
        content_canvas_guid: binding.content_canvas_guid.as_deref(),
        binding_kind: Some(&binding.binding_kind),
        manufacturer_id: root_manufacturer_id,
        helper_name: binding.helper_name.as_deref(),
        default_view_index: binding.dashboard_view_index,
        default_screen_slot: binding.dashboard_screen_slot,
        screen_name_loc_key: binding.screen_name_loc_key.as_deref(),
        transit_location_loc_key: binding.transit_location_loc_key.as_deref(),
        host_swf_path: binding.owner_source_file.as_deref(),
        screen_aspect_w_over_h: binding.ui_screen_aspect_w_over_h,
    };
    let effective_guid = binding
        .content_canvas_guid
        .as_deref()
        .filter(|g| !g.is_empty())
        .or_else(|| binding.canvas_guid.as_deref().filter(|g| !g.is_empty()));
    let authored_canvas_size = effective_guid
        .and_then(|guid| canvas_fetcher.fetch_canvas_json(guid).ok())
        .and_then(|json| authored_canvas_size(&json));
    let target_size = binding_target_size(&binding.binding_kind, authored_canvas_size);
    let animation_sample_percent = if binding.binding_kind == "mfd" {
        Some(0.0)
    } else {
        Some(starbreaker_ui::pipeline::DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT)
    };
    let inputs = PipelineInputs {
        binding: &view,
        canvas_fetcher: &canvas_fetcher,
        swf_fetcher: &P4kSwfFetcher { p4k },
        style_fetcher: &ManufacturerStyleFetcher { db },
        asset_fetcher: &P4kAssetFetcher { p4k },
        target_size,
        apply_postprocess: false,
        animation_sample_percent,
        localization_map: Some(loc_data.map.clone()),
        loc_fetcher: Some(&loc_data.ini),
        derived_values: ship_data.derived_values.clone(),
        // IR compilation does not rasterise the hologram bitmap (the node is
        // present in the IR regardless); only the PNG render path needs it.
        hologram_fetcher: None,
    };
    let _ = texture_mip;
    let ir = starbreaker_ui::pipeline::compile_ir_for_binding(&inputs).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&ir).map_err(|e| e.to_string())
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Map `binding_kind` to a canvas raster size.
fn binding_target_size(binding_kind: &str, authored_canvas_size: Option<(u32, u32)>) -> (u32, u32) {
    match binding_kind {
        "mfd" => (1600, 900),
        "radar" => (1024, 1024),
        _ => authored_canvas_size.unwrap_or((2048, 1024)),
    }
}

fn authored_canvas_size(canvas_json: &serde_json::Value) -> Option<(u32, u32)> {
    let record = canvas_json.get("_RecordValue_")?;
    let size = record.get("size")?;
    let width = size.get("x")?.as_f64()?;
    let height = size.get("y")?.as_f64()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let width = width.round() as u32;
    let height = height.round() as u32;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

/// Parse a GUID string, tolerating surrounding braces and optional hyphens.
pub(super) fn parse_guid(value: &str) -> Option<starbreaker_datacore::starbreaker_common::CigGuid> {
    use starbreaker_datacore::starbreaker_common::CigGuid;
    let trimmed = value.trim().trim_matches('{').trim_matches('}');
    CigGuid::from_str(trimmed).ok()
}

#[cfg(test)]
mod tests;
