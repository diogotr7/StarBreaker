//! High-level rendering pipeline entry points.
//!
//! Key entry points: [`compile_ir_for_binding`] (canvas → UI IR), [`render_for_binding`] (→ PNG).
//! Phase 6: for `binding_kind == "mfd"` with distinct frame/content canvases, `compile_ir_for_binding`
//! uses the frame canvas and post-processes: patches `base_Root` alpha=0.0 → 1.0 and injects
//! `screen_name_loc_key` into `text_ScreenName` nodes.

use std::collections::BTreeMap;

use crate::canvas::CanvasWidgetTreeResolver;
use crate::compose::{ComposeContext, encode_png};
use crate::defaults::DefaultValueRegistry;
use crate::error::UiError;
use crate::hybrid_compose::render_ui_ir_with_swf_overlay;
use crate::ir_compose::render_ui_ir_document;
use crate::style::{ManufacturerStyle, StyleLoader};
use crate::ui_ir::{UiIrDocument, UiIrTextPayload, UiRendererHint};

mod asset_manifest;
mod aspect_tag;
mod canvas_aspect;
mod host_stage;
mod style_projection;
use style_projection::project_canvas_style_entries;
mod style_selection;
mod swf_selection;
mod text_measure;
mod timing;
#[cfg(test)]
mod tests;

use asset_manifest::build_asset_reference_manifest;
use canvas_aspect::frame_canvas_aspect;
use host_stage::{host_stage_size, host_stage_text_scale_from_size};
use style_selection::{build_style_selection_manifest, load_style_for_ir};
use swf_selection::{build_swf_selection_manifest, load_first_swf};
use timing::timed;

pub use crate::bb_atlas::AssetFetcher;
pub use swf_selection::flash_swf_candidates;

/// Static UI captures use midpoint sampling for authored animations.
pub const DEFAULT_STATIC_ANIMATION_SAMPLE_PERCENT: f32 = 50.0;

// `extract_record_name` now lives in the neutral `crate::record_name` module so
// the lower layers don't depend up into `pipeline`. Re-exported here for the
// existing `pipeline::extract_record_name` call sites (incl. external crates).
pub use crate::record_name::extract_record_name;

/// Fetch a BuildingBlocks canvas record as JSON.
pub trait CanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError>;

    fn fetch_canvas_by_path(&self, path_or_name: &str) -> Result<serde_json::Value, UiError> {
        let name = extract_record_name(path_or_name);
        self.fetch_canvas_by_name(&name)
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        Err(UiError::FetchFailed {
            guid: record_name.into(),
            source: "fetch_canvas_by_name not implemented".into(),
        })
    }

    /// Fetch a record as a shared [`Rc`], for read-only consumers that would
    /// otherwise force a deep clone of a large record on every call (e.g. tag
    /// resolution re-fetching the whole `TagDatabase` per style-tag). The default
    /// wraps [`Self::fetch_canvas_by_path`]; a memoising fetcher can return its
    /// cached `Rc` directly so repeated fetches are a refcount bump, not a clone.
    fn fetch_canvas_by_path_shared(
        &self,
        path_or_name: &str,
    ) -> Result<std::rc::Rc<serde_json::Value>, UiError> {
        self.fetch_canvas_by_path(path_or_name).map(std::rc::Rc::new)
    }
}

/// Fetch raw SWF bytes by P4K path and enumerate P4K SWF directories.
pub trait SwfFetcher {
    fn fetch_swf_bytes(&self, p4k_path: &str) -> Result<Vec<u8>, UiError>;

    /// List immediate child directory names under `prefix` in the P4K SWF tree.
    ///
    /// `prefix` is a Windows-style path ending with `\`, e.g.
    /// `Data\UI\ShipInterface\assets\SWF\DRA\`.  Returns bare names (no
    /// leading path), sorted lexicographically.  The default returns an empty
    /// list; P4K-backed implementations should override to enable
    /// deterministic ship-subdir enumeration.
    fn list_swf_dirs(&self, _prefix: &str) -> Vec<String> {
        vec![]
    }
}

/// Resolve a manufacturer style by short id.
pub trait StyleFetcher {
    fn fetch_manufacturer_style(&self, manufacturer_id: &str) -> Result<ManufacturerStyle, UiError>;
}

/// A rendered vehicle-hologram bitmap: straight (non-premultiplied) RGBA8,
/// row-major, exactly `width * height * 4` bytes.
pub struct HologramImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One authored radar spoke (`Circle_Line_*`) handed to the radar-plane fetcher.
/// Every field is AUTHORED DATA read from the spoke's IR node — geometry from
/// `anchor`/`sizing`/`orientation`, colour from its resolved `background` fill
/// (cardinals `Accent1`, diagonals `Base`). The fetcher projects these onto the
/// tilted disc; nothing is invented (count, spacing, lengths, colours all come
/// from `rc_radarmapscreen_hostplane_visuals_large.json`).
#[derive(Debug, Clone, Copy)]
pub struct RadarSpokeInput {
    /// Bar centre in the parent plane's normalized `[0,1]×[0,1]` space (`anchor`).
    pub anchor: [f32; 2],
    /// Bar length as a fraction of the parent plane (`sizing.height` Percent).
    pub length_frac: f32,
    /// Bar width as a fraction of the parent plane (`sizing.width` Percent).
    pub width_frac: f32,
    /// In-plane rotation in degrees (`orientation.z`).
    pub rotation_deg: f32,
    /// Resolved RGB of the spoke's brand colour token (`Accent1`/`Base`).
    pub colour: [f32; 3],
    /// Authored fill alpha (`background.color.alpha`).
    pub alpha: f32,
}

/// Render the loaded vehicle's hologram for a `WidgetRuntimeImage`/`Primitive`
/// node (the SELF-STATUS "Own Vehicle Hologram"), which the engine draws as a
/// live 3D primitive and the 2D UI compositor cannot. The implementor (the
/// export pipeline) decodes the render scene's root-vehicle hull mesh and
/// rasterises it to a neutral greyscale hologram; `tint` is the node's authored
/// background fill (the per-manufacturer holo colour) applied over that
/// greyscale. The compositor draws the returned image into the node's rect.
/// Returns `None` when no vehicle geometry is available (e.g. non-vehicle
/// exports), leaving the node unrendered.
pub trait HologramFetcher {
    fn fetch_vehicle_hologram(&self, width: u32, height: u32, tint: [f32; 4]) -> Option<HologramImage>;

    /// Render the MFD-radar scope for a `BuildingBlocks_WidgetWindow`/`Primitive`
    /// node (the cockpit radar's `WindowContainer`), which the engine draws as a
    /// live render-to-texture 3D scene the 2D compositor cannot. The implementor
    /// loads the REAL disc texture bound by `disc_material_path` (the
    /// `Circle_Radial_Grid` node's `ui_r_radarmapscreen_radial_grid.mtl` →
    /// `r_radarmapscreen_radial_gradients.dds`: concentric rings + tick scale +
    /// axis) and projects it as the RTT camera would (tilted disc → ellipse),
    /// tinted by `tint` (the brand palette colour). Returns `None` when the
    /// material/texture is unavailable, leaving the window unrendered. Default
    /// `None` so non-radar callers / tests need not implement it.
    /// `spokes` are the authored `Circle_Line_*` bars (geometry + per-spoke colour
    /// from their IR nodes); `spoke_material_path` is their brand-resolved
    /// `line_a` material, from which the implementor reads the soft-glow
    /// appearance (`Gradient`/`InnerAlpha`/`OuterAlpha`/`Glow`). Both are data —
    /// nothing about the spokes is invented.
    ///
    /// `heading` describes the outer heading-tape ring (the `HeadingTape` Primitive:
    /// the `coordinates_novalue` tick-marker atlas tiled around the perimeter): its
    /// brand-resolved material + the node's authored `UVStart`/`UVSize` atlas
    /// window. `None` when no heading tape is loaded.
    fn fetch_radar_plane(
        &self,
        width: u32,
        height: u32,
        tint: [f32; 3],
        disc_material_path: &str,
        sweep_material_path: Option<&str>,
        spoke_material_path: Option<&str>,
        spokes: &[RadarSpokeInput],
        heading: Option<RadarHeadingTape<'_>>,
    ) -> Option<HologramImage> {
        let _ = (
            width,
            height,
            tint,
            disc_material_path,
            sweep_material_path,
            spoke_material_path,
            spokes,
            heading,
        );
        None
    }
}

/// The outer heading-tape ring inputs: the brand-resolved tape material and the
/// `HeadingTape` node's authored atlas window (`UVStart` + `UVSize`). All data —
/// the implementor wraps the material's atlas tick-row around the perimeter.
#[derive(Debug, Clone, Copy)]
pub struct RadarHeadingTape<'a> {
    pub material_path: &'a str,
    /// `primitiveSettings.UVStart` (atlas-space origin of the tape's window).
    pub uv_start: [f32; 2],
    /// `primitiveSettings.UVSize` (atlas-space extent; `x>1` = tiled around the
    /// ring, `y` = the tick-row band height).
    pub uv_size: [f32; 2],
    /// The tape's authored `background` fill alpha (`HeadingTape` authors `Base`
    /// alpha ~0.3) — data, per-manufacturer (the brand palette). The implementor
    /// scales it by the emissive brightness; not a picked constant.
    pub fill_alpha: f32,
}

/// Borrowed snapshot of UiBinding fields needed by the pipeline.
pub struct UiBindingView<'a> {
    pub canvas_guid: Option<&'a str>,
    pub content_canvas_guid: Option<&'a str>,
    pub binding_kind: Option<&'a str>,
    pub manufacturer_id: Option<&'a str>,
    pub helper_name: Option<&'a str>,
    pub default_view_index: Option<u32>,
    pub default_screen_slot: Option<u32>,
    /// Localization key for the MFD screen name (e.g. `"@ui_MFD_View_TargetStatus"`).
    /// Injected into `text_ScreenName` nodes when rendering the MFD frame canvas.
    pub screen_name_loc_key: Option<&'a str>,
    /// Localization key of the transit location this screen serves (the
    /// exporter's nearest-`SCTransitDestination` association, e.g.
    /// `"@ui_interactor_carrack_garage"` → "Sub Deck"). When set, the render's
    /// default registry is overlaid with the LOCALIZED floor name at
    /// [`TRANSIT_PANEL_LOCATION_PATH`] — the engine variable the transit
    /// display component pushes and the console canvas's
    /// `BindingsLocalizedVariable` caption reads. `None` for non-transit
    /// screens.
    pub transit_location_loc_key: Option<&'a str>,
    /// P4K path of the Flash movie hosting this screen's render-target (the
    /// binding's `owner_source_file`, e.g.
    /// `UI/BuildingBlocks/assets/SWF/BuildingBlocks_root.swf`). The engine
    /// renders BB canvases inside this GFx stage; textfield font sizes are in
    /// stage units, so the stage→target view scale applies to text. `None` when
    /// the binding has no recorded host movie (text renders unscaled).
    pub host_swf_path: Option<&'a str>,
    /// Physical display aspect (width / height, ≥ 1.0 for landscape) of the
    /// cockpit screen mesh this UI renders onto, derived at export from the
    /// render-target faces of the screen quad (`starbreaker-3d`
    /// `ui_pipeline::screen_aspect`). For `physical`/`radar` screens this sizes
    /// the render target to the real screen shape — square gauges render square,
    /// the annunciator as its wide strip — instead of the shared 16:9
    /// `M_Physical_Screen` canvas or the SWF content-bounds heuristic. `None`
    /// keeps the prior canvas/SWF sizing; the `mfd` frame path ignores it.
    pub screen_aspect_w_over_h: Option<f32>,
}

/// All inputs required by pipeline entrypoints.
pub struct PipelineInputs<'a> {
    pub binding: &'a UiBindingView<'a>,
    pub canvas_fetcher: &'a dyn CanvasFetcher,
    pub swf_fetcher: &'a dyn SwfFetcher,
    pub style_fetcher: &'a dyn StyleFetcher,
    pub asset_fetcher: &'a dyn crate::bb_atlas::AssetFetcher,
    pub target_size: (u32, u32),
    pub apply_postprocess: bool,
    pub animation_sample_percent: Option<f32>,
    pub localization_map: Option<std::collections::HashMap<String, String>>,
    pub loc_fetcher: Option<&'a dyn crate::bb_loc::LocFetcher>,
    /// Per-ship binding-path values DERIVED from game data by the caller (the
    /// export pipeline's DataCore derivation — power pools, temps, emissions).
    /// Applied over the compiled-in well-known defaults; `None` keeps the
    /// static at-rest registry.
    pub derived_values: Option<std::collections::HashMap<String, crate::canvas::Value>>,
    /// Renders the loaded vehicle's hologram for `WidgetRuntimeImage`/`Primitive`
    /// own-vehicle nodes. `None` leaves those nodes unrendered (the prior
    /// behaviour for callers/tests that don't supply ship geometry).
    pub hologram_fetcher: Option<&'a dyn HologramFetcher>,
}

/// Diagnostics captured while rendering a UI image.
#[derive(Debug, Clone)]
pub struct UiRenderDiagnostics {
    pub resolved_canvas_ids: Vec<String>,
    pub resolved_canvas_names: Vec<String>,
    pub selected_style_source: String,
    pub selected_swf_source: String,
    pub render_backend: String,
    pub fallback_counters: BTreeMap<String, u32>,
    pub unresolved_references: Vec<String>,
    pub confidence: u8,
}

/// Render output plus diagnostics for provenance metadata.
#[derive(Debug, Clone)]
pub struct UiRenderOutput {
    pub png: Vec<u8>,
    pub diagnostics: UiRenderDiagnostics,
}

/// The engine variable a transit display component pushes its floor name
/// into; the transit panel canvases read it via a `BindingsLocalizedVariable`
/// caption (`OLD_TransitUIPanelExterior*` `Label_ThisFloor`). A binding path
/// (schema identifier, like the compass `SVehicleHudParams` paths), not a
/// game-data value — the VALUE comes from the socpak's authored
/// `SCTransitDestination` metadata via the binding's loc key.
const TRANSIT_PANEL_LOCATION_PATH: &str = "transitdisplay.panelLocation";

/// Per-binding registry overlay for a transit screen: resolve the binding's
/// floor loc key to its LOCALIZED name and pin it at
/// [`TRANSIT_PANEL_LOCATION_PATH`] (the engine pushes the already-localized
/// string; `BindingsLocalizedVariable` stringifies the variable verbatim).
/// Returns `None` for non-transit bindings so the shared registry is reused
/// untouched — the clone (incl. the localization map) is paid only by the few
/// transit screens per ship.
fn transit_overlay_registry(
    binding: &UiBindingView<'_>,
    defaults: &DefaultValueRegistry,
) -> Option<DefaultValueRegistry> {
    let key = binding.transit_location_loc_key?.trim();
    if key.is_empty() {
        return None;
    }
    let localized = defaults
        .lookup_localization(key)
        .unwrap_or(key.trim_start_matches('@'))
        .to_string();
    let mut overlay = defaults.clone();
    overlay.insert_path(
        TRANSIT_PANEL_LOCATION_PATH,
        crate::canvas::Value::Str(localized),
    );
    Some(overlay)
}

/// Compile canonical UI IR for a binding.
pub fn compile_ir_for_binding(inputs: &PipelineInputs<'_>) -> Result<UiIrDocument, UiError> {
    let defaults = DefaultValueRegistry::with_pipeline_defaults_and_derived_values(
        inputs.localization_map.clone(),
        inputs.derived_values.as_ref(),
    );
    compile_ir_for_binding_with(inputs, &defaults)
}

/// Like [`compile_ir_for_binding`], but reuses a caller-supplied default
/// registry instead of constructing one. The registry is a pure function of the
/// localization map + derived values — both constant across a whole export — so
/// a caller rendering many bindings (a ship export with hundreds of screens)
/// builds it once and shares it here. Constructing it per binding (cloning and
/// re-merging the multi-MB localization map every time) dominated capital-ship
/// export time. See [`render_for_binding_with_registry`].
pub fn compile_ir_for_binding_with(
    inputs: &PipelineInputs<'_>,
    defaults: &DefaultValueRegistry,
) -> Result<UiIrDocument, UiError> {
    // A transit screen overlays its floor name onto the shared registry
    // (per-binding value; see `transit_overlay_registry`).
    let transit_overlay = transit_overlay_registry(inputs.binding, defaults);
    let defaults = transit_overlay.as_ref().unwrap_or(defaults);
    let b = inputs.binding;

    // Phase 6: for mfd bindings with a distinct frame canvas, compile the frame
    // (canvas_guid) instead of the content canvas so that the footer chrome is
    // included.  For all other bindings keep the existing content-first priority.
    let frame_guid = b.canvas_guid.filter(|g| !g.is_empty());
    let content_guid = b.content_canvas_guid.filter(|g| !g.is_empty());
    let use_frame_canvas = b.binding_kind == Some("mfd")
        && frame_guid.is_some()
        && content_guid.is_some()
        && frame_guid != content_guid;

    let effective_guid = if use_frame_canvas {
        frame_guid
    } else {
        content_guid.or(frame_guid)
    }
    .ok_or_else(|| {
        UiError::RenderError(format!(
            "no canvas GUID available for helper {:?} (kind {:?})",
            b.helper_name, b.binding_kind,
        ))
    })?;

    let raw_root_json = timed("fetch", || inputs.canvas_fetcher.fetch_canvas_json(effective_guid))?;
    let resolver = CanvasWidgetTreeResolver::new();
    let resolved = timed("graph1", || resolver.resolve(effective_guid, |guid| {
        inputs.canvas_fetcher.fetch_canvas_json(guid)
    }))?;
    let canvas_name = raw_root_json
        .get("_RecordName_")
        .and_then(|v| v.as_str());

    let style_manifest = build_style_selection_manifest(
        &raw_root_json,
        b.manufacturer_id,
        inputs.canvas_fetcher,
        inputs.style_fetcher,
    );
    let effective_manufacturer_id = b
        .manufacturer_id
        .or_else(|| {
            style_manifest
                .selected_source
                .as_deref()
                .and_then(|source| source.strip_prefix("manufacturer:"))
        })
        // Keep brand/style resolution deterministic when binding-level
        // manufacturer metadata is absent.
        .or(Some("drak"));

    // For an MFD frame canvas, the binding's content canvas selects which of the
    // frame's mutually-exclusive content-view slots is instantiated (the frame
    // embeds every view; the runtime view-selector boolean has no static default).
    // Resolve the bound content's `_RecordName_` so the resolver can instantiate
    // the matching slot and skip its peers during Pass 2.
    let bound_view_record_name: Option<String> = if use_frame_canvas {
        let name = content_guid
            .and_then(|cguid| inputs.canvas_fetcher.fetch_canvas_json(cguid).ok())
            .and_then(|json| {
                json.get("_RecordName_")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            });
        if name.is_none() {
            // Without the bound content's record name the frame falls back to its
            // arbitrary static-default view, which usually renders the wrong
            // content. Surface it rather than failing silently.
            log::warn!(
                "mfd frame {:?}: could not resolve content canvas record name for guid {:?}; \
                 view selection skipped (frame may render the wrong content view)",
                b.helper_name,
                content_guid,
            );
        }
        name
    } else {
        None
    };

    // The host movie's stage size (from its header) drives both the MFD
    // content-view slot fractions (mfd_view, plan P5.4) and the host-path
    // text scale below; read it once.
    let frame_host_stage = if use_frame_canvas {
        host_stage_size(b.host_swf_path, inputs.swf_fetcher)
    } else {
        None
    };
    let mut scene = timed("graph2", || {
        crate::bb_resolve::resolve_canvas_graph_with_defaults_and_host_stage(
            &raw_root_json,
            effective_manufacturer_id,
            &|p| {
                inputs
                    .canvas_fetcher
                    .fetch_canvas_by_path(p)
                    .map_err(|e| e.to_string())
            },
            inputs.loc_fetcher,
            bound_view_record_name.as_deref(),
            &defaults,
            frame_host_stage,
        )
        .map_err(UiError::RenderError)
    })?;

    project_canvas_style_entries(
        &mut scene,
        &raw_root_json,
        effective_manufacturer_id,
        inputs.loc_fetcher,
    );
    // The projection above runs the WEAKEST tiers (root `defaultStyles` +
    // brand) sequentially last; re-apply the modular-kit component sheets so
    // the StandardModule tier stays the authority for widget-standard chrome
    // (tier precedence over application order — the transit button's
    // Filled-state corner geometry, ledger 106).
    if let Some(style_id) =
        crate::bb_resolve::modular_style_identifier(&raw_root_json, effective_manufacturer_id)
    {
        crate::bb_resolve::apply_modular_kit_sheets(
            &mut scene,
            &style_id,
            &|p| {
                inputs
                    .canvas_fetcher
                    .fetch_canvas_by_path(p)
                    .map_err(|e| e.to_string())
            },
            inputs.loc_fetcher,
        );
    }

    let swf_manifest = build_swf_selection_manifest(
        &raw_root_json,
        &resolved,
        effective_manufacturer_id.unwrap_or("drak"),
        inputs.swf_fetcher,
    );
    let selected_swf_source = swf_manifest
        .valid_candidates
        .first()
        .map(|candidate| candidate.path.clone());

    let mut effective_target_size = inputs.target_size;
    // Set when a per-screen mesh aspect override is applied to a `useRaw` canvas
    // whose authored aspect differs: the content COVERS the mesh-aspect target
    // (origin-anchored) instead of letterboxing. See the `screen_aspect_w_over_h`
    // branch below.
    let mut cover_fit_mesh_aspect = false;
    // An MFD binding wraps a (often 16:9) content canvas in a distinct screen
    // frame; the physical screen proportions come from the frame, so derive the
    // render aspect from it and let relatively-laid-out content reflow. Scoped to
    // `mfd` — `physical` annunciators keep their native SWF/stage-driven sizing.
    let frame_aspect = if b.binding_kind == Some("mfd") {
        frame_canvas_aspect(b.canvas_guid, b.content_canvas_guid, inputs.canvas_fetcher)
    } else {
        None
    };
    // Per-screen physical aspect from the cockpit screen mesh, for non-MFD
    // render-to-texture screens (`physical`/`radar`). The display is mapped onto
    // a quad whose proportions ARE the engine's render-target shape, so a square
    // g-force gauge must render square and the annunciator as its wide strip —
    // not the shared 16:9 `M_Physical_Screen` canvas nor the SWF content-bounds
    // heuristic this replaces. `mfd` keeps the frame-canvas aspect above.
    let screen_aspect_w_over_h = if matches!(b.binding_kind, Some("physical") | Some("radar")) {
        b.screen_aspect_w_over_h.filter(|a| a.is_finite() && *a > 0.0)
    } else {
        None
    };
    if let Some(aspect) = frame_aspect {
        let width = inputs.target_size.0.max(1);
        let height = ((width as f32) * aspect).round().max(1.0) as u32;
        if width <= 8192 && height <= 8192 {
            effective_target_size = (width, height);
        }
    } else if let Some(aspect_w_over_h) = screen_aspect_w_over_h {
        // height = width / (w/h); the mesh aspect IS the physical screen shape.
        let width = inputs.target_size.0.max(1);
        let height = ((width as f32) / aspect_w_over_h).round().max(1.0) as u32;
        if width <= 8192 && height <= 8192 {
            effective_target_size = (width, height);
            // The cockpit screen MESH aspect is the authoritative display shape. A
            // `useRaw` canvas whose authored aspect differs from it (the g-force /
            // velocity ball gauges author 16:9 but their screen is square) must
            // COVER the mesh-aspect target, not letterbox back to 16:9: the square
            // `aspectRatio` ball-area fills the screen and the trailing readouts
            // overflow off the edge and crop — matching the in-game gauge. A no-op
            // for aspect-matched screens (cover == contain) and for screens whose
            // merged canvas fills via `aspectOverrides*` (door, annunciator), so the
            // frozen interior screens are untouched.
            cover_fit_mesh_aspect = true;
        }
    } else if let Some(swf_source) = selected_swf_source.as_deref()
        && let Ok(swf_bytes) = inputs.swf_fetcher.fetch_swf_bytes(swf_source)
        && let Ok(swf_library) = crate::swf_assets::SwfAssetLibrary::new(swf_bytes)
    {
        let (sw, sh) = swf_library.stage_size();
        let content_aspect = swf_library
            .stage_visual_bounds(0)
            .and_then(|(x0, y0, x1, y1)| {
                let w = (x1 - x0).abs();
                let h = (y1 - y0).abs();
                if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
                    Some(h / w)
                } else {
                    None
                }
            });
        if sw.is_finite() && sh.is_finite() && sw > 0.0 && sh > 0.0 {
            let aspect = content_aspect.unwrap_or(sh / sw);
            if aspect > 0.0 && aspect.is_finite() {
                let width = inputs.target_size.0.max(1);
                let swf_height = ((width as f32) * aspect).round().max(1.0) as u32;
                let height = swf_height.max(1);
                // Avoid pathological SWF headers from collapsing layout.
                if width <= 8192 && height <= 8192 {
                    effective_target_size = (width, height);
                }
            }
        }
    }

    // Data-driven MFD responsive layout: the engine maps the physical screen
    // aspect (from the frame canvas) to a layout tag and, on a 4:3/16:9/1:1
    // match, widens the content view to `SizeX × height` ("Content Canvas
    // Scaling" embeddedStyle). The content-view slot is the tagged content
    // canvas, so widening it reflows the cards to their in-game width instead of
    // the measured host-inset width. Scoped to the MFD frame path; a no-op when
    // the screen has no matching aspect tag or content-scaling style.
    if use_frame_canvas && let Some(aspect_hw) = frame_aspect {
        apply_mfd_content_canvas_scaling(&mut scene, aspect_hw, &raw_root_json, inputs.canvas_fetcher);
    }

    let asset_manifest = timed("manifest", || build_asset_reference_manifest(&scene, inputs.asset_fetcher));

    // Textfield font sizes are host-stage units on the MFD frame path; see
    // `host_stage::host_stage_text_scale` for the engine model.
    let design_text_scale = if use_frame_canvas {
        host_stage_text_scale_from_size(frame_host_stage, effective_target_size)
    } else {
        1.0
    };

    // Pre-layout draw-faithful text measurement: load the SAME SWF assets the
    // compose-time draw will use so intrinsic text boxes hug the painted
    // glyphs (measure == draw; see pipeline/text_measure.rs). An empty
    // library (no SWF source) selects no font and the TTF estimate stays.
    let measure_swf_paths: Vec<String> = selected_swf_source.iter().cloned().collect();
    let measure_assets =
        timed("swf_load_measure", || load_first_swf(&measure_swf_paths, inputs.swf_fetcher));
    let text_measure = Some(text_measure::SwfDrawTextMeasure::new(&measure_assets));

    // A 2D `physical` mesh-aspect screen (ball gauges, countermeasure panels) FILLS
    // its square screen non-uniformly; the `radar` 3D-RTT scope keeps the uniform
    // COVER its disc projection + chrome were tuned for. Both are `cover_fit_mesh_aspect`.
    let mesh_aspect_fill = cover_fit_mesh_aspect && b.binding_kind == Some("physical");

    let mut ir = timed("ir_compile", || crate::ui_ir::compile_ui_ir_from_scene_with_animation_sample(
        &scene,
        Some(inputs.canvas_fetcher),
        effective_guid,
        canvas_name,
        effective_target_size,
        &defaults,
        style_manifest.selected_source,
        selected_swf_source,
        &[],
        asset_manifest.resolved_asset_refs,
        asset_manifest.missing_asset_refs,
        inputs.animation_sample_percent,
        100,
        design_text_scale,
        text_measure.as_ref().map(|m| m as &dyn crate::ui_ir::DrawTextMeasure),
        cover_fit_mesh_aspect,
        mesh_aspect_fill,
    ));
    ir.warnings.extend(fallback_counter_warnings(
        style_manifest
            .fallback_counters
            .iter()
            .chain(swf_manifest.fallback_counters.iter())
            .map(|(key, value): (&String, &u32)| (key.as_str(), *value)),
    ));

    if use_frame_canvas {
        // NOTE: the page-in start-state alpha (frame `base_Root` authored alpha=0.0)
        // is resolved structurally during IR compilation by `is_pagein_start_root`
        // in `local_alpha_for_node`, so its settled 1.0 cascades correctly through
        // `inheritsAlpha` descendants. A post-compile name-based patch here was
        // ineffective (it ran after inheritance was already baked into children).

        // Inject the resolved screen name into text_ScreenName nodes.
        if let Some(raw_key) = b.screen_name_loc_key {
            let bare_key = raw_key.trim_start_matches('@');
            let text = inputs
                .loc_fetcher
                .and_then(|f| f.fetch_loc(bare_key))
                .unwrap_or_else(|| raw_key.to_string());
            for node in &mut ir.nodes {
                if node.name == "text_ScreenName" {
                    // Apply the label field's authored case modifier (the footer
                    // screen name uses `caseModifier = "Upper"` → "TARGET STATUS"),
                    // read from the scene node so it stays data-driven.
                    let cased = match scene
                        .nodes
                        .get(&node.id)
                        .and_then(|n| n.raw.get("labelProperties"))
                        .and_then(|lp| lp.get("caseModifier"))
                        .and_then(|v| v.as_str())
                    {
                        Some("Upper") | Some("AllCaps") => text.to_uppercase(),
                        Some("Lower") => text.to_lowercase(),
                        _ => text.clone(),
                    };
                    node.text_payload = Some(UiIrTextPayload::Resolved { text: cased });
                }
            }
        }
    }

    Ok(ir)
}

/// Render via IR compilation and IR-only rendering.
pub fn render_for_binding_ir(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
    // Build the default registry once and share it with the IR-compile pass
    // below, instead of letting each pass build its own (the previous code built
    // it twice per render — once here, once inside `compile_ir_for_binding`).
    let defaults = DefaultValueRegistry::with_pipeline_defaults_and_derived_values(
        inputs.localization_map.clone(),
        inputs.derived_values.as_ref(),
    );
    render_for_binding_ir_with(inputs, &defaults)
}

/// Like [`render_for_binding_ir`], but reuses a caller-supplied default registry
/// (see [`compile_ir_for_binding_with`] / [`render_for_binding_with_registry`]).
pub fn render_for_binding_ir_with(
    inputs: &PipelineInputs<'_>,
    defaults: &DefaultValueRegistry,
) -> Result<Vec<u8>, UiError> {
    let ir = timed("compile", || compile_ir_for_binding_with(inputs, defaults))?;

    let mut style = timed("style_load", || load_style_for_ir(&ir, inputs))?;
    // A SWF-hosted screen whose authored screen-name background image is
    // INACTIVE at rest paints NO style background — the strip body is the
    // absence of emission (near pure black; the in-game capture's warmth is
    // CRT/capture-side, handled outside this renderer). Adjudicated on the
    // annunciator reference; an attempted deletion of this rule on
    // 2026-06-12 looked drift-free only because the whole-image guard was
    // comparing STALE exported PNGs — a fresh render regressed the
    // annunciator/door strips by a global warm haze. Tag names come from
    // the authored tag database (data); the open remediation item is the
    // DERIVED semantic (the P4.4/P5 re-audit kept this rule — see
    // crates/starbreaker-ui/docs/ui-fallback-register.md / ui-cascade-passes.md).
    let suppresses_placeholder_screen_background = ir.selected_swf_source.is_some()
        && ir.nodes.iter().any(|node| {
            node.node_type.eq_ignore_ascii_case("widget_image")
                && !node.is_active
                && node.resolved_style_tags.iter().any(|tag| {
                    tag.tag_name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case("ScreenNameBackground"))
                })
        });
    if suppresses_placeholder_screen_background {
        style.background = crate::canvas::RgbaColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
    }
    // A COCKPIT HUD screen (`HC_HUD_*` / `H_HUD_*` / `H_Eng_*`) renders against
    // the cockpit's BLACK screen bezel — the in-game captures' dark surround is
    // that bezel plus the front geometry, not the screen's own fill. Its
    // authored full-screen background ELEMENTS (the g-force/velocity/compass
    // gauges' `Screen Background` textures) cover the clear where present; where
    // none is authored (the LR-indicator) the surround must read black, not the
    // brand "Background" brown. Interior/door (`I_Door_*`) and MFD frame
    // (`MC_S_*`) screens keep the brand background clear — the door's panel and
    // the MFD frame body genuinely composite over it (the brand colour stays a
    // palette entry for those). Scoped by the cockpit-HUD canvas class
    // (`is_cockpit_hud_canvas`, the compass-arc classifier), not a name.
    if ir
        .canvas_name
        .as_deref()
        // canvas_name is `BuildingBlocks_Canvas.<RecordName>`; the classifier
        // expects the bare record name.
        .map(|name| name.rsplit('.').next().unwrap_or(name))
        .is_some_and(crate::bb_brand_style::is_cockpit_hud_canvas)
    {
        style.background = crate::canvas::RgbaColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
    }
    let swf_paths = ir
        .selected_swf_source
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let assets = timed("swf_load", || load_first_swf(&swf_paths, inputs.swf_fetcher));
    let ctx = ComposeContext {
        style: &style,
        defaults: &defaults,
        assets: &assets,
        hologram_fetcher: inputs.hologram_fetcher,
    };
    let atlas_manufacturer_id = inputs.binding.manufacturer_id.or_else(|| {
        ir.selected_style_source
            .as_deref()
            .and_then(|source| source.strip_prefix("manufacturer:"))
    });
    let atlas = crate::bb_atlas::AtlasLibrary::new(inputs.asset_fetcher, atlas_manufacturer_id);

    let image = timed("render", || -> Result<_, UiError> {
        match ir.renderer_hint {
            UiRendererHint::Bb => render_ui_ir_document(&ir, &ctx, &atlas),
            UiRendererHint::Swf | UiRendererHint::Hybrid => render_ui_ir_with_swf_overlay(
                &ir,
                &ctx,
                &atlas,
                &|key| inputs.loc_fetcher.and_then(|f| f.fetch_loc(key)),
            ),
        }
    })?;
    timed("encode", || encode_png(&image))
}

/// Main entrypoint for rendering a UI binding to PNG bytes.
pub fn render_for_binding(inputs: &PipelineInputs<'_>) -> Result<Vec<u8>, UiError> {
    render_for_binding_ir(inputs)
}

/// Render a binding to PNG bytes reusing a caller-built [`DefaultValueRegistry`].
///
/// The registry depends only on the localization map and derived values, which
/// are identical for every binding in one export. Building it once and passing
/// it here (instead of per binding via [`render_for_binding`]) removes the
/// dominant per-render cost on UI-dense ships — cloning and re-merging the
/// multi-MB localization map for each of hundreds of screens. Output is
/// identical to [`render_for_binding`] given an equivalently-built registry.
pub fn render_for_binding_with_registry(
    inputs: &PipelineInputs<'_>,
    defaults: &DefaultValueRegistry,
) -> Result<Vec<u8>, UiError> {
    render_for_binding_ir_with(inputs, defaults)
}

/// Apply the engine's data-driven MFD "Content Canvas Scaling" to the content
/// view: map the screen aspect (frame canvas height/width) to a layout tag via
/// the `AspectRatioToTag_MFD` library, find that tag's `SizeX` (`PercentOfY`) in
/// any resolved canvas's "Content Canvas Scaling" `embeddedStyle`, and set the
/// landscape content-view slot's width to `SizeX × height`. This is the
/// responsive rule the engine applies at runtime (the slot is the content
/// canvas the rule targets), replacing the measured host inset on the WIDTH axis
/// (height keeps its inset). No-op when any datum is absent — the slot then
/// keeps its host-inset width, so non-MFD and unmatched screens are unchanged.
fn apply_mfd_content_canvas_scaling(
    scene: &mut crate::bb_scene::BbScene,
    frame_aspect_h_over_w: f32,
    frame_json: &serde_json::Value,
    fetcher: &dyn CanvasFetcher,
) {
    use crate::bb_scene::BbValue;
    if frame_aspect_h_over_w <= 0.0 || !frame_aspect_h_over_w.is_finite() {
        return;
    }
    let aspect_w_over_h = 1.0 / frame_aspect_h_over_w;
    // The MFD aspect→tag library is read for its authored ratios + tag refs (the
    // values come from DataCore, not source): MFD screens use the `_MFD` variant.
    let Ok(library) = fetcher.fetch_canvas_by_path("AspectRatioToTag_MFD") else {
        return;
    };
    let Some(tag_id) = aspect_tag::nearest_aspect_tag(&library, aspect_w_over_h) else {
        return;
    };
    // The "Content Canvas Scaling" embeddedStyle lives in the content-frame
    // canvas the frame embeds via a `canvas:` URL (not a guid sub-reference), so
    // walk the frame's canvas-reference graph (BFS, bounded) and match the entry
    // by STRUCTURE — no record-name gating.
    let Some(scaling) = find_content_scaling(frame_json, &tag_id, fetcher) else {
        return;
    };
    if !scaling.behavior_is_percent_of_y || scaling.size_x <= 0.0 {
        return;
    }
    let Some(slot_id) = crate::mfd_view::landscape_slot_id(scene) else {
        return;
    };
    let Some(slot) = scene.nodes.get_mut(&slot_id) else {
        return;
    };
    // The engine's rule: the content canvas width = `SizeX × its own height`
    // (`WidthBehavior = PercentOfY`). The two-pass layout resolves the cross-axis
    // (height) first, so this is faithful and does not couple width→width.
    //
    // `mfd_view` sets the slot height to the host-inset FRACTION (the bottom 44px
    // chrome is cut from the hosted view), but the engine sizes the content
    // canvas width against the FULL content height. Divide SizeX by that fraction
    // so the PercentOfY basis is the full height — otherwise the cards land ~6%
    // narrow (414 vs the ~432 reference) and the flex-shrunk battery icon is
    // squashed with them (53 vs ~69px).
    // The engine sizes the content canvas width = SizeX × the content canvas's
    // own height. `mfd_view` set the slot height to the host-VIEW inset (the
    // bottom 44px chrome cut from the hosted view), but the content canvas the
    // rule targets spans ~95% of the frame height (slightly taller than the
    // hosted view — it runs behind the footer chrome). Scale SizeX by that
    // content-height fraction so the PercentOfY basis matches: this lands the
    // cards at ~437px (≈ the in-game ~432) and, via the mixed-row flex shrink,
    // the BATTERY icon at ~69px wide — both measured against the in-game power
    // reference. (`MFD_CONTENT_HEIGHT_FRACTION` is a measured constant in the
    // same family as the host inset; the in-game content height is not in the
    // authored data.)
    const MFD_CONTENT_HEIGHT_FRACTION: f32 = 0.95;
    slot.sizing.width = BbValue::Other {
        value: scaling.size_x / MFD_CONTENT_HEIGHT_FRACTION,
        behavior: "PercentOfY".to_string(),
    };
}

/// BFS the canvas-reference graph rooted at `frame_json` for the first canvas
/// carrying a "Content Canvas Scaling" entry for `tag_id`. Bounded by a visited
/// set and a fetch cap so a malformed/cyclic graph can't run away.
fn find_content_scaling(
    frame_json: &serde_json::Value,
    tag_id: &str,
    fetcher: &dyn CanvasFetcher,
) -> Option<aspect_tag::ContentScalingWidth> {
    use std::collections::{HashSet, VecDeque};
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    collect_canvas_references(frame_json, &mut queue);
    // The content frame is a direct `canvas:` embed of the MFD frame, so the
    // match is found within a hop or two; cap fetches so a wide reference graph
    // can't turn this into a deep crawl.
    let mut fetch_budget = 64usize;
    while let Some(reference) = queue.pop_front() {
        if !visited.insert(reference.clone()) {
            continue;
        }
        if fetch_budget == 0 {
            break;
        }
        fetch_budget -= 1;
        let Ok(record) = fetcher.fetch_canvas_by_path(&reference) else {
            continue;
        };
        if let Some(scaling) = aspect_tag::content_scaling_width(&record, tag_id) {
            return Some(scaling);
        }
        collect_canvas_references(&record, &mut queue);
    }
    None
}

/// Collect canvas record references from a record JSON into `out`: the `canvas:`
/// URL fields that embed a sub-canvas (file-URL or record-name form) and any
/// `BuildingBlocks_Canvas.*` record-ref strings. References are normalised to
/// record names via [`extract_record_name`].
fn collect_canvas_references(json: &serde_json::Value, out: &mut std::collections::VecDeque<String>) {
    match json {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_canvas_references(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                // Follow only `canvas:` sub-canvas embeds — the structural canvas
                // tree (m_eng_mfdcontent is referenced this way, as a file-URL,
                // not a guid sub-record). Following every `BuildingBlocks_Canvas.`
                // record-ref string instead explodes the crawl (icons, widgets…).
                if key == "canvas"
                    && let Some(s) = value.as_str()
                    && !s.is_empty()
                    && s != "null"
                {
                    out.push_back(extract_record_name(s));
                } else {
                    collect_canvas_references(value, out);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn fallback_counter_warnings<'a>(
    counters: impl IntoIterator<Item = (&'a str, u32)>,
) -> Vec<String> {
    counters
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(key, count)| format!("fallback path used: {key}={count}"))
        .collect()
}

pub(super) fn load_style(
    manufacturer_id: Option<&str>,
    fetcher: &dyn StyleFetcher,
) -> ManufacturerStyle {
    let id = manufacturer_id.unwrap_or("drak");
    match fetcher.fetch_manufacturer_style(id) {
        Ok(style) => style,
        Err(e) => {
            log::debug!(
                "pipeline: manufacturer style fetch failed for '{}': {}; using neutral fallback",
                id,
                e,
            );
            StyleLoader::for_manufacturer(id).neutral_fallback()
        }
    }
}
