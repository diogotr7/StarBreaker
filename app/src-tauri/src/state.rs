use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use parking_lot::Mutex;

use starbreaker_p4k::MappedP4k;
use starbreaker_wwise::{AtlIndex, Hierarchy};

use crate::datacore_commands::RecordEntry;

pub struct AppState {
    pub p4k: Mutex<Option<Arc<MappedP4k>>>,
    pub dcb_bytes: Mutex<Option<Vec<u8>>>,
    pub export_cancel: Arc<AtomicBool>,
    /// Localization strings from Data\Localization\english\global.ini.
    /// Keys are lowercase for case-insensitive lookup.
    pub localization: Mutex<HashMap<String, String>>,
    /// Lightweight index of all main records for search + tree browsing.
    pub record_index: Mutex<Option<Vec<RecordEntry>>>,
    /// ATL trigger index, built once by audio_init.
    pub atl_index: Mutex<Option<AtlIndex>>,
    /// Lazily-loaded bank hierarchies, keyed by bank filename.
    pub bank_cache: Mutex<HashMap<String, Option<Arc<Hierarchy>>>>,
    /// Maps wwise filename (bnk/wem) to full P4k path (e.g. "Data\\Sounds\\wwise\\English(US)\\Foo.bnk").
    pub wwise_paths: Mutex<HashMap<String, String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            p4k: Mutex::new(None),
            dcb_bytes: Mutex::new(None),
            export_cancel: Arc::new(AtomicBool::new(false)),
            localization: Mutex::new(HashMap::new()),
            record_index: Mutex::new(None),
            atl_index: Mutex::new(None),
            bank_cache: Mutex::new(HashMap::new()),
            wwise_paths: Mutex::new(HashMap::new()),
        }
    }
}

/// Parse a global.ini localization file into a key→value map.
/// Keys are lowercased for case-insensitive lookup.
pub fn parse_localization(data: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(data);
    let mut map = HashMap::new();
    for line in text.lines() {
        // Skip BOM, empty lines, comments
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_lowercase(), value.trim().to_string());
        }
    }
    map
}
