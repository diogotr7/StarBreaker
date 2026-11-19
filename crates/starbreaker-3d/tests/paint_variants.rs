//! Hull paint variants against real data: paints stored in
//! `Material.materialVariants[]` (the 300 series), filtered by each hull's
//! paint port, and paints stored in `SubGeometry[]` (the Gladius); FPS weapon
//! finishes that are separate entities on the base geometry.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::collections::HashMap;
use std::path::PathBuf;

use starbreaker_3d::enumerate_paint_variants_for_entity;
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::{EntityIndex, resolve_loadout};
use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str =
    r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";

fn p4k() -> Option<MappedP4k> {
    let path = std::env::var("SC_DATA_P4K")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_P4K_PATH));
    if !path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", path.display());
        return None;
    }
    Some(MappedP4k::open(&path).expect("open p4k"))
}

fn paint_tags(p4k: &MappedP4k, db: &Database, entity: &str) -> Vec<String> {
    let index = EntityIndex::new(db);
    let record = index.find_record(entity).unwrap_or_else(|| panic!("{entity} not in DataCore"));
    let tree = resolve_loadout(db, record);
    enumerate_paint_variants_for_entity(db, p4k, record, &tree.root, &HashMap::new())
        .into_iter()
        .map(|v| v.subgeometry_tag.to_ascii_lowercase())
        .collect()
}

#[test]
fn hull_paints_come_from_material_variants_and_sub_geometry() {
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();

    let orig = paint_tags(&p4k, &db, "ORIG_300i");
    eprintln!("ORIG_300i: {orig:?}");
    for tag in ["300i_base", "300_series_metallic_blue"] {
        assert!(orig.contains(&tag.to_string()), "ORIG_300i lacks {tag}");
    }
    // The 315p's paints live on the same geometry but its port, not the 300i's, accepts them.
    assert!(!orig.iter().any(|t| t.starts_with("315p_")), "ORIG_300i lists 315p paints: {orig:?}");

    let gladius = paint_tags(&p4k, &db, "AEGS_Gladius");
    eprintln!("AEGS_Gladius: {gladius:?}");
    assert!(gladius.contains(&"gladiuspirate".to_string()), "Gladius lost its SubGeometry paints: {gladius:?}");
}

/// FPS weapon finishes that are their own entities (the base CDF with the
/// finish's own palette) are the base's variants; finishes the base already
/// carries as tagged nodes are not repeated.
#[test]
fn weapon_finish_entities_are_variants_of_the_base() {
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    let record = index.find_record("gmni_lmg_ballistic_01").expect("F55 base");
    let finishes: Vec<String> = starbreaker_3d::enumerate_sibling_finishes(&db, &p4k, record, &HashMap::new())
        .into_iter()
        .map(|v| v.subgeometry_tag.to_ascii_lowercase())
        .collect();
    eprintln!("gmni_lmg_ballistic_01 finishes: {finishes:?}");
    for name in ["gmni_lmg_ballistic_01_blue_gold", "gmni_lmg_ballistic_01_green_red01"] {
        assert!(finishes.contains(&name.to_string()), "missing {name}: {finishes:?}");
    }
    // Collector01 is a SubGeometry tag of the base: its entity adds no new look.
    assert!(!finishes.iter().any(|f| f.ends_with("_collector01")), "{finishes:?}");

    // A hull gets its paints through its paint port, not sibling entities.
    let hull = index.find_record("RSI_Apollo_Triage").expect("Apollo Triage");
    assert!(starbreaker_3d::enumerate_sibling_finishes(&db, &p4k, hull, &HashMap::new()).is_empty());
}
