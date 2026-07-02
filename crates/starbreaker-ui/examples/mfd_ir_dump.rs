//! Debugging helper: compile a (framed) MFD binding IR via the local
//! decompiled-record mirror and dump name-filtered nodes with rect, state,
//! fill, and asset fields.
//!
//! Usage:
//!   cargo run --release -p starbreaker-ui --example mfd_ir_dump -- \
//!     <canvas-guid> <content-guid> [name-filter] [WxH]
//!
//! The record mirror is expected at `../ships/dcb_canvas/libs/foundry/records`
//! relative to the workspace root (the standard decomposed-export layout).
//! Uses the pipeline's well-known defaults registry, no SWF/P4K assets.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use rayon::prelude::*;
use starbreaker_ui::pipeline::AssetFetcher;
use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView, UiError,
};

struct Fs {
    map: HashMap<String, PathBuf>,
    /// Parsed-record cache keyed by file path. The compile re-fetches the 6.2 MB
    /// `TagDatabase` thousands of times during tag resolution; without a cache the
    /// `Fs` fetcher re-reads + re-parses it each time, dwarfing everything else
    /// (ledger 42). Caching + the shared-`Rc` path mirror the export's DataCore
    /// fetcher so repeated fetches are a refcount bump.
    cache: std::cell::RefCell<HashMap<PathBuf, std::rc::Rc<serde_json::Value>>>,
}
impl Fs {
    fn new(map: HashMap<String, PathBuf>) -> Self {
        Self {
            map,
            cache: std::cell::RefCell::new(HashMap::new()),
        }
    }
    fn fetch_rc(&self, key: &str) -> Result<std::rc::Rc<serde_json::Value>, UiError> {
        let p = self
            .map
            .get(&key.to_ascii_lowercase())
            .ok_or_else(|| UiError::RenderError(format!("missing {key}")))?
            .clone();
        if let Some(v) = self.cache.borrow().get(&p) {
            return Ok(std::rc::Rc::clone(v));
        }
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_default())
                .map_err(|e| UiError::RenderError(format!("parse {}: {e}", p.display())))?;
        let rc = std::rc::Rc::new(value);
        self.cache.borrow_mut().insert(p, std::rc::Rc::clone(&rc));
        Ok(rc)
    }
}
impl CanvasFetcher for Fs {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        self.fetch_rc(guid).map(|rc| (*rc).clone())
    }
    fn fetch_canvas_by_name(&self, name: &str) -> Result<serde_json::Value, UiError> {
        self.fetch_canvas_json(name)
    }
    fn fetch_canvas_by_path_shared(
        &self,
        path_or_name: &str,
    ) -> Result<std::rc::Rc<serde_json::Value>, UiError> {
        // Return the cached Rc directly (no deep clone) — the hot path for tag
        // resolution, mirroring the DataCore fetcher.
        self.fetch_rc(&starbreaker_ui::pipeline::extract_record_name(path_or_name))
    }
}
struct NoSwf;
impl SwfFetcher for NoSwf {
    fn fetch_swf_bytes(&self, _: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("no swf in mirror".into()))
    }
}
struct NoStyle;
impl StyleFetcher for NoStyle {
    fn fetch_manufacturer_style(
        &self,
        _: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Err(UiError::RenderError("no style db in mirror".into()))
    }
}
struct NoAsset;
impl AssetFetcher for NoAsset {
    fn fetch_image_bytes(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
}

/// Read the first `n` bytes of `p` as a lossy string. Record header fields
/// (`_RecordId_`, `_RecordName_`) sit at the very top of every record, so a head
/// read avoids pulling multi-MB bodies into memory just to index by name/guid.
fn read_head(p: &Path, n: usize) -> String {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(p) else {
        return String::new();
    };
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf).unwrap_or(0);
    buf.truncate(read);
    String::from_utf8_lossy(&buf).into_owned()
}

/// Extract a top-level JSON string value, `"<field>": "value"` -> `value`.
/// Record id/name values contain no escaped quotes, so a plain quote scan is
/// sufficient (and far cheaper than a full serde parse).
fn extract_json_string_field(s: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let after_key = &s[s.find(&key)? + key.len()..];
    let after_colon = &after_key[after_key.find(':')? + 1..];
    let after_open_quote = &after_colon[after_colon.find('"')? + 1..];
    let end = after_open_quote.find('"')?;
    Some(after_open_quote[..end].to_string())
}

/// Minimal stderr logger so library `log::` probes (e.g. `BB_A3_STYLE_PROBE`)
/// are visible from this debug helper. Enabled via `MFD_IR_DUMP_LOG=1`.
struct StderrLogger;
impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}
static LOGGER: StderrLogger = StderrLogger;

fn main() {
    if std::env::var("MFD_IR_DUMP_LOG").as_deref() == Ok("1") {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Info);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../ships/dcb_canvas/libs/foundry/records")
        .canonicalize()
        .expect("record mirror at ../ships/dcb_canvas/libs/foundry/records");
    // Index ONLY the UI-support subtrees the canvas fetcher resolves against
    // (ledger 42): mfd_ir_dump fetches canvases / styles / fonts / timelines /
    // tags / screen presets by guid/name/stem, all of which live under these
    // dirs. Nothing under entities/, contracts/, loadouts/ etc. is ever fetched,
    // so skipping the rest of the 60k-file / 3 GB mirror is ~10x fewer files and
    // avoids the multi-MB loadout/contract records. Prefer the indexed tools for
    // routine work — `starbreaker ui render --dump-ir-dir` (bound screens) or the
    // `ui_ir_query` MCP tool (ad-hoc canvas pairs); this example is the no-P4K /
    // no-MCP fallback.
    let load_start = std::time::Instant::now();
    let mut files = Vec::new();
    for sub in ["ui", "tagdatabase", "scitemdisplayscreenpreset"] {
        collect(&root.join(sub), &mut files);
    }
    eprintln!(
        "mfd_ir_dump: indexing {} UI-support record files (parallel head-scan, a few seconds; harness load, NOT a pipeline loop)…",
        files.len()
    );
    // Build the name/id/stem -> path index from each file's HEAD only:
    // `_RecordId_` and `_RecordName_` sit at the top of every record, so a
    // head-scan + string extract avoids a full serde parse of multi-hundred-KB
    // canvases. Parsed in parallel.
    let parsed: Vec<(PathBuf, Option<String>, Option<String>)> = files
        .par_iter()
        .map(|p| {
            let head = read_head(p, 8192);
            (
                p.clone(),
                extract_json_string_field(&head, "_RecordId_"),
                extract_json_string_field(&head, "_RecordName_"),
            )
        })
        .collect();
    let mut map = HashMap::new();
    // Stems are the low-priority fallback (first occurrence wins).
    for p in &files {
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            map.entry(stem.to_ascii_lowercase()).or_insert_with(|| p.clone());
        }
    }
    // GUID + record name (and the BuildingBlocks_Canvas.-stripped name) override.
    for (p, id, name) in &parsed {
        if let Some(id) = id {
            map.insert(id.to_ascii_lowercase(), p.clone());
        }
        if let Some(n) = name {
            map.insert(n.to_ascii_lowercase(), p.clone());
            map.insert(
                n.strip_prefix("BuildingBlocks_Canvas.")
                    .unwrap_or(n)
                    .to_ascii_lowercase(),
                p.clone(),
            );
        }
    }
    eprintln!(
        "mfd_ir_dump: indexed ({} keys) in {:.1}s; compiling IR…",
        map.len(),
        load_start.elapsed().as_secs_f32()
    );
    let canvas = std::env::args().nth(1).expect("canvas guid/name");
    let content = std::env::args().nth(2).expect("content guid/name");
    let filter = std::env::args().nth(3).unwrap_or_default().to_ascii_lowercase();
    let size = std::env::args().nth(4).unwrap_or_else(|| "1600x1200".into());
    let (w, h) = size
        .split_once('x')
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .unwrap_or((1600, 1200));
    let fetcher = Fs::new(map);
    let binding = UiBindingView {
        canvas_guid: Some(&canvas),
        content_canvas_guid: Some(&content),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("mfd_ir_dump"),
        default_view_index: None,
        default_screen_slot: None,
        screen_name_loc_key: None,
        transit_location_loc_key: None,
        host_swf_path: None,
        screen_aspect_w_over_h: None,
    };
    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &fetcher,
        swf_fetcher: &NoSwf,
        style_fetcher: &NoStyle,
        asset_fetcher: &NoAsset,
        target_size: (w, h),
        apply_postprocess: false,
        animation_sample_percent: Some(0.0),
        localization_map: None,
        loc_fetcher: None,
        derived_values: None,
        hologram_fetcher: None,
    };
    let compile_start = std::time::Instant::now();
    let ir = starbreaker_ui::compile_ir_for_binding(&inputs).expect("compile");
    eprintln!(
        "mfd_ir_dump: IR compiled in {:.1}s",
        compile_start.elapsed().as_secs_f32()
    );
    println!("nodes: {}", ir.nodes.len());
    for n in &ir.nodes {
        if !filter.is_empty() && !n.name.to_ascii_lowercase().contains(&filter) {
            continue;
        }
        let r = &n.computed_rect;
        println!(
            "{} {} '{}' parent={:?} active={} rect=({:.0},{:.0},{:.0},{:.0}) alpha={} bg={:?} bg_token={:?}",
            n.id,
            n.node_type,
            n.name,
            n.parent_id,
            n.is_active,
            r.x,
            r.y,
            r.w,
            r.h,
            n.alpha,
            n.background_fill_colour,
            n.background_fill_colour_token,
        );
        if n.background_fill_alpha.is_some() {
            println!("    bg_alpha={:?}", n.background_fill_alpha);
        }
        if n.asset_ref.is_some() {
            println!("    asset={:?}", n.asset_ref);
        }
        if n.segmented_fill.is_some() {
            println!("    segmented={:?}", n.segmented_fill);
        }
        if n.icon_tint_colour.is_some() || n.icon_tint_colour_token.is_some() {
            println!("    icon_tint={:?} icon_tint_token={:?}", n.icon_tint_colour, n.icon_tint_colour_token);
        }
        if !n.resolved_style_tags.is_empty() {
            let tags: Vec<&str> = n
                .resolved_style_tags
                .iter()
                .filter_map(|t| t.tag_name.as_deref())
                .collect();
            println!("    tags={tags:?}");
        }
        if !n.children.is_empty() {
            println!("    children={:?}", n.children);
        }
    }
}
