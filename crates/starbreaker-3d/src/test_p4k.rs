//! Test-only access to the game's Data.p4k: [`read`] returns a file's bytes,
//! or `None` (the test then skips) when no Data.p4k is available — at
//! `SC_DATA_P4K`, else the default LIVE install. The archive is opened once
//! per test binary. Paths of the male t1_pu head-rig fixtures: [`MALE_HEAD`].

use std::sync::OnceLock;

use starbreaker_p4k::MappedP4k;

/// The male t1_pu head rig directory plus file stem; append `.skin`,
/// `.skinm`, `.dna`, `_lod1.dna`, ….
pub const MALE_HEAD: &str = r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\protos_human_male_face_t1_pu_head";
/// The male t1_pu rig's CDF.
pub const MALE_CDF: &str = r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\dna_riglogicspu.cdf";
/// The male t1_pu rig as a standalone `.RigLogic` file.
pub const MALE_RIGLOGIC: &str = r"Data\Objects\Characters\RigLogic\protos_human_male_face_t1_pu.RigLogic";

const DEFAULT_P4K: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";

/// The Data.p4k, opened once; `None` when there is none.
pub fn p4k() -> Option<&'static MappedP4k> {
    static P4K: OnceLock<Option<MappedP4k>> = OnceLock::new();
    P4K.get_or_init(|| {
        let path = std::env::var("SC_DATA_P4K").unwrap_or_else(|_| DEFAULT_P4K.into());
        match MappedP4k::open(&path) {
            Ok(p4k) => Some(p4k),
            Err(e) => {
                eprintln!("skip: no Data.p4k at {path} ({e})");
                None
            }
        }
    })
    .as_ref()
}

/// The bytes of `path` in the Data.p4k; `None` when there is no Data.p4k.
/// A missing file in an available archive panics: the fixture moved.
pub fn read(path: &str) -> Option<Vec<u8>> {
    let p4k = p4k()?;
    Some(p4k.read_file(path).unwrap_or_else(|e| panic!("{path}: {e}")))
}
