//! Armour export (spec `docs/superpowers/specs/2026-10-13-armour-export-design.md`):
//! wearable items export as their worn, gendered `.skin` — alone through the
//! entity pipeline, or in sets worn by a character.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_3d::character::loadout::worn_geometry;
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";
const ARMS: &str = "cds_armor_heavy_arms_01_01_01";

fn p4k() -> Option<MappedP4k> {
    let path = std::env::var("SC_DATA_P4K").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(DEFAULT_P4K_PATH));
    if !path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", path.display());
        return None;
    }
    Some(MappedP4k::open(&path).expect("open p4k"))
}

fn db_bytes(p4k: &MappedP4k) -> Vec<u8> {
    p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap()
}

/// Armour has a worn mesh per gender; a ship has none.
#[test]
fn armour_has_a_worn_mesh_per_gender() {
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    let arms = index.find_record(ARMS).unwrap();
    let male = worn_geometry(&db, arms, "Male", &["Male".into()]).unwrap();
    assert!(male.geometry.to_lowercase().ends_with("male_v7/armor/cds/m_cds_heavy_armor_01_arms.skin"), "{}", male.geometry);
    assert!(!male.other_gender);
    let female = worn_geometry(&db, arms, "Female", &["Female".into()]).unwrap();
    assert!(female.geometry.to_lowercase().ends_with("female_v2/armor/cds/f_cds_heavy_armor_01_arms.skin"), "{}", female.geometry);
    let ship = index.find_record("AEGS_Gladius").unwrap();
    assert!(worn_geometry(&db, ship, "Male", &["Male".into()]).is_none());
}

fn export(p4k: &MappedP4k, db: &Database, name: &str, opts: &starbreaker_3d::ExportOptions) -> starbreaker_3d::ExportResult {
    let index = EntityIndex::new(db);
    let record = index.find_record(name).unwrap();
    let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&index, record);
    starbreaker_3d::assemble_glb_with_loadout(db, p4k, record, &tree, opts).unwrap()
}

/// `entity export --geometry-tag Male|Female` of an armour piece is the
/// piece (its gendered .skin, skinned to its own bones), not its storage
/// crate — GLB and package.
#[test]
fn armour_pieces_export_as_worn() {
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    for (tag, file) in [("Male", "m_cds_heavy_armor_01_arms.skin"), ("Female", "f_cds_heavy_armor_01_arms.skin")] {
        let opts = ExportOptions { geometry_tag: Some(tag.into()), lod_level: 0, ..ExportOptions::default() };
        let result = export(&p4k, &db, ARMS, &opts);
        assert!(result.geometry_path.to_lowercase().ends_with(file), "{tag}: {}", result.geometry_path);
        let (doc, _, _) = gltf::import_slice(result.bundled_bytes().unwrap()).unwrap();
        assert!(doc.skins().any(|s| s.joints().count() > 20), "{tag}: skinned to its bones");
    }
    let opts = ExportOptions {
        kind: ExportKind::Decomposed,
        format: ExportFormat::Blend,
        geometry_tag: Some("Male".into()),
        lod_level: 0,
        ..ExportOptions::default()
    };
    let result = export(&p4k, &db, ARMS, &opts);
    let files = &result.decomposed.as_ref().unwrap().files;
    assert!(
        files.iter().any(|f| f.relative_path.to_lowercase().contains("m_cds_heavy_armor_01_arms") && f.relative_path.ends_with(".blend")),
        "{:?}",
        files.iter().map(|f| &f.relative_path).collect::<Vec<_>>()
    );
}

fn chf(p4k: &MappedP4k, name: &str) -> starbreaker_chf::ChfData {
    let bytes = p4k.read_file(&format!(r"Data\Libs\CharacterCustomizer\{name}.chf")).unwrap();
    starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap()
}

/// Hand-picked marine heavy pieces dress the character exactly as the
/// marine heavy loadout does (minus its mobiGlas), in any listing order.
#[test]
fn picked_pieces_dress_like_the_loadout() {
    use starbreaker_3d::character::loadout::{parse_loadout, resolve_garments};
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let picked: Vec<String> = ["cds_armor_heavy_core_01_01_01", "cds_armor_heavy_arms_01_01_01", "cds_undersuit_01_01_01", "cds_armor_heavy_legs_01_01_01"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let placed = dress_entries(&db, &chf, &[], &picked).unwrap();
    assert!(placed.unplaced.is_empty(), "{:?}", placed.unplaced);
    let worn = |entries| {
        let mut g: Vec<(String, String)> =
            resolve_garments(&db, entries, "Male", "body_itemport").garments.into_iter().map(|g| (g.port, g.geometry)).collect();
        g.sort();
        g
    };
    let loadout = parse_loadout(&p4k.read_file(r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\UEE\marine_Armor_Heavy.xml").unwrap()).unwrap();
    let mut expected = worn(&loadout);
    expected.retain(|(port, _)| port != "mobiglas_attach");
    assert_eq!(worn(&placed.entries), expected);
}

/// A shirt picked with an undersuit goes on the body's torso clothing port,
/// which the undersuit hides; an unknown name is reported, not guessed.
#[test]
fn undersuits_hide_picked_clothing_and_unknown_names_are_reported() {
    use starbreaker_3d::character::loadout::resolve_garments;
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let picked: Vec<String> = ["cds_undersuit_01_01_01", "ctl_shirt_06_01_01", "cds_armor_heavy"].iter().map(|s| s.to_string()).collect();
    let placed = dress_entries(&db, &chf, &[], &picked).unwrap();
    assert_eq!(placed.unplaced.len(), 1, "{:?}", placed.unplaced);
    assert!(placed.unplaced[0].starts_with("cds_armor_heavy:"), "a partial name is not an item: {:?}", placed.unplaced);
    let shirt_port = placed.entries[0].children.iter().find(|e| e.item == "ctl_shirt_06_01_01").map(|e| e.port.clone());
    assert_eq!(shirt_port.as_deref(), Some("Clothing_Torso_0"));
    let worn = resolve_garments(&db, &placed.entries, "Male", "body_itemport");
    assert!(worn.garments.iter().all(|g| g.item != "ctl_shirt_06_01_01"), "the undersuit hides the shirt");
    assert!(worn.garments.iter().any(|g| g.item == "cds_undersuit_01_01_01"));
}

/// Without a CHF, the wearer is the game's default character of the gender.
#[test]
fn default_wearers_match_their_gender() {
    use starbreaker_3d::character::{Gender, default_wearer_chf};
    let Some(p4k) = p4k() else { return };
    for gender in [Gender::Male, Gender::Female] {
        let bytes = default_wearer_chf(&p4k, gender).unwrap();
        let chf = starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap();
        assert_eq!(Gender::from_hash(chf.dna.gender_hash), gender);
    }
}

fn loadout(p4k: &MappedP4k, path: &str) -> Vec<starbreaker_3d::character::loadout::LoadoutEntry> {
    starbreaker_3d::character::loadout::parse_loadout(&p4k.read_file(path).unwrap()).unwrap()
}

/// Picked pieces join a loadout: the arms go on the loadout's undersuit,
/// and a helmet the loadout already wears keeps its port (a second one is
/// reported, not stacked).
#[test]
fn picked_pieces_join_a_loadout() {
    use starbreaker_3d::character::loadout::resolve_garments;
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let suit = loadout(&p4k, r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\UEE\marine_Undersuit.xml");
    let placed = dress_entries(&db, &chf, &suit, &["cds_armor_heavy_arms_01_01_01".to_string()]).unwrap();
    assert!(placed.unplaced.is_empty(), "{:?}", placed.unplaced);
    let worn = resolve_garments(&db, &placed.entries, "Male", "body_itemport");
    let ports: Vec<&str> = worn.garments.iter().map(|g| g.port.as_str()).collect();
    assert!(ports.contains(&"Armor_Undersuit") && ports.contains(&"Armor_Arms"), "{ports:?}");

    let pirate = loadout(&p4k, r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\Pirate\Pirate_Heavy_01.xml");
    let placed = dress_entries(&db, &chf, &pirate, &["slaver_armor_heavy_helmet_01_01_01".to_string()]).unwrap();
    assert_eq!(placed.unplaced.len(), 1, "the loadout's helmet keeps its port: {:?}", placed.unplaced);
    let helmets = resolve_garments(&db, &placed.entries, "Male", "body_itemport").garments.iter().filter(|g| g.port == "Armor_Helmet").count();
    assert_eq!(helmets, 1);
}

/// Hats go on the head's hat port: the CHF's own items (its head) are worn
/// items with ports too.
#[test]
fn hats_go_on_the_heads_port() {
    use starbreaker_3d::character::loadout::resolve_garments;
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let placed = dress_entries(&db, &chf, &[], &["ctl_hat_01_01_01".to_string()]).unwrap();
    assert!(placed.unplaced.is_empty(), "{:?}", placed.unplaced);
    let worn = resolve_garments(&db, &placed.entries, "Male", "body_itemport");
    assert_eq!(worn.garments.iter().map(|g| g.item.as_str()).collect::<Vec<_>>(), ["ctl_hat_01_01_01"], "only the hat: the CHF's own items are kept, not worn again");
}

/// A picked piece the wearer's gender has no mesh for is reported, not
/// placed and then silently dropped when the outfit is built.
#[test]
fn pieces_without_the_wearers_mesh_are_reported() {
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let placed = dress_entries(&db, &chf, &[], &["srvl_helmet_01_01_01_hair_extension".to_string()]).unwrap();
    assert_eq!(placed.unplaced.len(), 1, "{:?}", placed.unplaced);
    assert!(placed.unplaced[0].contains("no Male mesh"), "{:?}", placed.unplaced);
}

/// A CHF whose body record DataCore lacks is an error, not a body without
/// ports (every pick then "no free port").
#[test]
fn a_missing_body_record_is_an_error() {
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let mut chf = chf(&p4k, "masculinedefault");
    chf.itemport.id = "00000000-0000-0000-0000-000000000001".parse().unwrap();
    let err = dress_entries(&db, &chf, &[], &[ARMS.to_string()]).unwrap_err();
    assert!(err.to_string().contains("body"), "{err}");
}
