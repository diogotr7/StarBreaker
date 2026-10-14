//! Test-only access to the game's Data.p4k: [`read`] returns a file's bytes,
//! or `None` (the test then skips) when no Data.p4k is available — at
//! `SC_DATA_P4K`, else the default LIVE install. Fixture paths: the male
//! t1_pu head rig's master [`MALE_DNA`] and [`MALE_DNA_LOD1`].

use std::sync::OnceLock;

use starbreaker_p4k::MappedP4k;

macro_rules! male_head {
    ($suffix:literal) => {
        concat!(
            r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\protos_human_male_face_t1_pu_head",
            $suffix
        )
    };
}

/// The male t1_pu master `.dna`.
pub const MALE_DNA: &str = male_head!(".dna");
/// The male t1_pu LOD1 `.dna`.
pub const MALE_DNA_LOD1: &str = male_head!("_lod1.dna");

const DEFAULT_P4K: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";

/// The bytes of `path` in the Data.p4k; `None` when there is no Data.p4k.
/// A missing file in an available archive panics: the fixture moved.
pub fn read(path: &str) -> Option<Vec<u8>> {
    static P4K: OnceLock<Option<MappedP4k>> = OnceLock::new();
    let p4k = P4K
        .get_or_init(|| {
            let path = std::env::var("SC_DATA_P4K").unwrap_or_else(|_| DEFAULT_P4K.into());
            MappedP4k::open(&path).inspect_err(|e| eprintln!("skip: no Data.p4k at {path} ({e})")).ok()
        })
        .as_ref()?;
    Some(p4k.read_file(path).unwrap_or_else(|e| panic!("{path}: {e}")))
}
