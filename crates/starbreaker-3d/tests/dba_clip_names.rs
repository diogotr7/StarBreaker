//! DBA clip names come out clean: the names table is 8-byte aligned, and its
//! padding is not always NUL bytes (`character_select_screen.dba`).
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";

fn p4k() -> Option<MappedP4k> {
    let path = std::env::var("SC_DATA_P4K").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(DEFAULT_P4K_PATH));
    if !path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", path.display());
        return None;
    }
    Some(MappedP4k::open(&path).expect("open p4k"))
}

#[test]
fn dba_clip_names_are_clean_paths() {
    let Some(p4k) = p4k() else { return };
    let db = starbreaker_3d::animation::parse_dba(&p4k.read_file(r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba").unwrap())
        .unwrap();
    assert!(!db.clips.is_empty());
    for clip in &db.clips {
        assert!(clip.name.starts_with("animations/"), "{:?}", clip.name);
    }
}
