//! Character loadouts (`character::loadout`, `character::clothing`): the
//! engine's armour loadouts resolve to gendered garments with their layers,
//! and the character exports wear them with the covered zones hidden.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_3d::character::loadout::{parse_loadout, resolve_garments};
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";
const MARINE_HEAVY: &str = r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\UEE\marine_Armor_Heavy.xml";
const PIRATE_HEAVY: &str = r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\Pirate\Pirate_Heavy_01.xml";

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

/// The marine heavy loadout: an undersuit on the body, three armour pieces
/// on the undersuit and the mobiGlas, each a gendered `.skin` (with a
/// `.mtl`, or none: the `.skin`'s own); the armour has clothing layers. The
/// body entry (empty item) is not a garment.
#[test]
fn armour_loadout_resolves_to_gendered_garments() {
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let entries = parse_loadout(&p4k.read_file(MARINE_HEAVY).unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].port.as_str(), entries[0].item.as_str()), ("Body_ItemPort", ""));

    for (gender, prefix) in [("Male", "objects/characters/human/male_v7/"), ("Female", "objects/characters/human/female_v2/")] {
        let worn = resolve_garments(&db, &entries, gender, "body_itemport");
        let ports: Vec<&str> = worn.garments.iter().map(|g| g.port.as_str()).collect();
        assert_eq!(ports, ["Armor_Undersuit", "Armor_Arms", "Armor_Legs", "Armor_Torso", "mobiglas_attach"], "{gender}");
        for g in &worn.garments {
            assert!(g.geometry.ends_with(".skin"), "{gender} {}: {}", g.item, g.geometry);
            assert!(!g.port.starts_with("Armor_") || g.geometry.to_lowercase().starts_with(prefix), "{gender} {}: {}", g.item, g.geometry);
            assert!(g.material.is_empty() || g.material.to_lowercase().ends_with(".mtl"), "{gender} {}: {}", g.item, g.material);
            assert!(!g.port.starts_with("Armor_") || !g.clothing.chunks.is_empty(), "{}: no clothing zones", g.item);
        }
        let layer = |port: &str| worn.garments.iter().find(|g| g.port == port).unwrap().clothing.chunks[0].layer;
        assert!(layer("Armor_Undersuit") < layer("Armor_Arms"), "armour sits over the undersuit");
    }
}

/// A full helmet hides the head's port; the hidden ports are reported for
/// the character's own (CHF) items.
#[test]
fn helmets_hide_the_head_port() {
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let entries = parse_loadout(&p4k.read_file(PIRATE_HEAVY).unwrap()).unwrap();
    let worn = resolve_garments(&db, &entries, "Male", "body_itemport");
    assert!(worn.garments.iter().any(|g| g.port == "Armor_Helmet"), "{:?}", worn.garments.iter().map(|g| &g.port).collect::<Vec<_>>());
    assert!(worn.hidden_ports.iter().any(|p| p.eq_ignore_ascii_case("Head_ItemPort")), "{:?}", worn.hidden_ports);
}

fn chf(p4k: &MappedP4k, name: &str) -> starbreaker_chf::ChfData {
    let bytes = p4k.read_file(&format!(r"Data\Libs\CharacterCustomizer\{name}.chf")).unwrap();
    starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap()
}

/// Mesh names → vertex counts of a GLB (shared vertex buffers counted once).
fn mesh_vertices(glb: &[u8]) -> std::collections::HashMap<String, usize> {
    let (doc, _, _) = gltf::import_slice(glb).unwrap();
    doc.meshes()
        .map(|m| {
            let mut seen = std::collections::HashSet::new();
            let count = m
                .primitives()
                .filter_map(|p| p.get(&gltf::Semantic::Positions))
                .filter(|a| seen.insert(a.index()))
                .map(|a| a.count())
                .sum();
            (m.name().unwrap_or_default().to_owned(), count)
        })
        .collect()
}

/// A character in the marine heavy armour: the undersuit covers the whole
/// body (not written), the armour arms cover the undersuit's gloves (its
/// hand submeshes are not written), and the armour and mobiGlas are worn on
/// the character's skeleton.
#[test]
fn armoured_character_hides_what_the_armour_covers() {
    use starbreaker_3d::character::outfit::Outfit;
    use starbreaker_3d::character::{HeadIdentityContext, gender_tag, items};
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let loadout = parse_loadout(&p4k.read_file(MARINE_HEAVY).unwrap()).unwrap();
    let outfit = Outfit::load(&p4k, &db, &chf, &loadout);
    assert_eq!(outfit.garments.len(), 5);
    assert_eq!(outfit.body_hidden_zones.len(), 32, "every zone the body claims");
    let suit = outfit.garments.iter().find(|g| g.garment.port == "Armor_Undersuit").unwrap();
    let zone = starbreaker_3d::character::clothing::zone_hash;
    assert!(suit.hidden_zones.contains(&zone("l_hand_zone")) && suit.hidden_zones.contains(&zone("r_hand_zone")), "the armour arms' gloves cover the undersuit's");
    assert!(!suit.hidden_zones.contains(&zone("torso01_zone")), "nothing worn over the undersuit claims its torso");

    let items = items::load_head_items(&p4k, &items::resolve_head_items(&db, &chf.itemport, gender_tag(chf.dna.gender_hash)));
    let glb = HeadIdentityContext::load(&p4k, chf.dna.gender_hash).unwrap().build_character_glb(&chf.dna, &items, &outfit, &Default::default()).unwrap();
    let meshes = mesh_vertices(&glb);
    assert!(!meshes.contains_key("m_body"), "the body is covered: {:?}", meshes.keys());
    for worn in ["m_cds_undersuit_armor_01", "m_cds_heavy_armor_01_arms", "m_cds_heavy_armor_01_legs", "m_cds_heavy_armor_01_core"] {
        assert!(meshes.contains_key(worn), "{worn} missing: {:?}", meshes.keys());
    }
    assert!(meshes.contains_key("m_hair_31"), "no helmet: the hair stays");
    let full = suit.mesh.positions.len();
    let written = meshes["m_cds_undersuit_armor_01"];
    assert!(written < full && written > full / 2, "undersuit without its gloves: {written} of {full} vertices");
}

/// A full helmet hides the CHF items under the head's port.
#[test]
fn helmets_hide_the_chf_head_items() {
    use starbreaker_3d::character::outfit::Outfit;
    use starbreaker_3d::character::{gender_tag, items};
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let loadout = parse_loadout(&p4k.read_file(PIRATE_HEAVY).unwrap()).unwrap();
    let outfit = Outfit::load(&p4k, &db, &chf, &loadout);
    let items = items::load_head_items(&p4k, &items::resolve_head_items(&db, &chf.itemport, gender_tag(chf.dna.gender_hash)));
    assert!(!items.is_empty());
    for item in &items {
        assert!(!outfit.wears(item), "{} on {} is under the helmet", item.item.geometry, item.item.port);
    }
    assert!(Outfit::with_body(None).wears(&items[0]), "nothing worn hides nothing");
}
