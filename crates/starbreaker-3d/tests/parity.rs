//! GLB ↔ .blend parity on real game data (plan phase F): each fixture is
//! exported both ways, read back into `parity::Summary`s and diffed; every
//! difference must be a known gap in `tests/parity_allowlist.txt`, and every
//! pattern there must still match something.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::collections::HashMap;
use std::path::PathBuf;

use starbreaker_3d::parity::{Allowlist, diff, from_blend, from_glb};
use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K_PATH: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";
const ALLOWLIST: &str = include_str!("parity_allowlist.txt");

fn p4k() -> Option<MappedP4k> {
    let path = std::env::var("SC_DATA_P4K").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(DEFAULT_P4K_PATH));
    if !path.exists() {
        eprintln!("Data.p4k not found at {}, skipping", path.display());
        return None;
    }
    Some(MappedP4k::open(&path).expect("open p4k"))
}

/// A standalone armour `.skin`: `skin_to_glb` vs `skin_to_blend`.
fn armour(p4k: &MappedP4k) -> Vec<String> {
    let base = r"Data\Objects\Characters\Human\male_v7\armor\kap\m_kap_combat_heavy_arms_01";
    let skin = p4k.read_file(&format!("{base}.skin")).unwrap();
    let skinm = p4k.read_file(&format!("{base}.skinm")).unwrap();
    let glb = from_glb(&starbreaker_3d::skin_to_glb(&skinm, Some(&skin)).unwrap()).unwrap();
    let blend = starbreaker_3d::skin_to_blend("m_kap_combat_heavy_arms_01", &skinm, Some(&skin)).unwrap();
    let blend = from_blend(&blend, "armour.blend", &|_| None).unwrap();
    // Not vacuous: the summaries hold the skinned armour.
    assert!(blend.meshes.values().any(|m| m.joints.len() > 50 && m.vertices > 10_000), "{:?}", blend.meshes.keys());
    diff("armour", &glb, &blend)
}

/// A CHF character playing a face and a body clip: the character GLB vs the
/// package's scene.blend (linked asset files included).
fn character(p4k: &MappedP4k) -> Vec<String> {
    use starbreaker_3d::character::{HeadIdentityContext, outfit::Outfit, gender_tag, identity::MALE_T1_PU_GENDER_HASH, items, clips::{CharacterAnimation, FaceAnimation, load_body_clip}, package::export_character_package};
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    let chf = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let caf = p4k
        .read_file(r"Data\Animations\Characters\Human\heads\male\ai\idlesets\happy\face\ai_civ_happy_idle_02_face.caf")
        .unwrap();
    let dba_path = r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba";
    let idle = load_body_clip(&p4k.read_file(dba_path).unwrap(), dba_path, Some("char_customizer_stand_idle_01")).unwrap();
    let animation = CharacterAnimation { face: Some(FaceAnimation { caf: &caf, intensity: 1.0 }), body: Some(&idle) };
    let parsed = starbreaker_chf::ChfFile::from_chf(&chf).unwrap().parse().unwrap();
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb).unwrap();
    // The same CHF items and body the package resolves.
    let items = items::load_head_items(p4k, &items::resolve_head_items(&db, &parsed.itemport, gender_tag(parsed.dna.gender_hash)));
    let outfit = Outfit::load(p4k, &db, &parsed, &[]);
    assert!(outfit.body.is_some());
    let glb = HeadIdentityContext::load(p4k, MALE_T1_PU_GENDER_HASH)
        .unwrap()
        .build_character_glb(&parsed.dna, &items, &outfit, &animation)
        .unwrap();

    let opts = ExportOptions { kind: ExportKind::Decomposed, format: ExportFormat::Blend, ..ExportOptions::default() };
    let export = export_character_package(p4k, &db, &chf, "masculinedefault", &opts, &animation, &[]).unwrap();
    let files: HashMap<String, Vec<u8>> = export.files.into_iter().map(|f| (f.relative_path, f.bytes)).collect();
    let (scene_path, scene) = files.iter().find(|(p, _)| p.ends_with("/scene.blend")).expect("scene.blend");
    let blend = from_blend(scene, scene_path, &|p| files.get(p).cloned()).unwrap();
    // Not vacuous: head + 3 sub-meshes + 5 items + the body, a rig on the
    // actor skeleton (weighted joints: body + face + items) and the one clip
    // playing face (176 facial bones, the morphs) and body (100+ bones).
    assert_eq!(blend.meshes.len(), 10, "{:?}", blend.meshes.keys());
    assert!(blend.bones.len() > 250 && blend.meshes["protos_human_male_face_t1_pu_head"].morphs.len() > 100, "{} bones", blend.bones.len());
    let clip = &blend.clips["char_customizer_stand_idle_01"];
    assert!(clip.channels.len() > 400, "{}", clip.channels.len());
    assert!(clip.channels.keys().any(|c| c.contains("Hips")) && clip.channels.keys().any(|c| c.contains("FACIAL_")), "{:?}", clip.channels.keys().take(5).collect::<Vec<_>>());
    diff("character", &from_glb(&glb).unwrap(), &blend)
}

/// The CHF character in the marine heavy armour loadout, playing a body
/// clip: `chf_to_character_glb` vs the package.
fn armoured(p4k: &MappedP4k) -> Vec<String> {
    use starbreaker_3d::character::chf_to_character_glb;
    use starbreaker_3d::character::clips::{CharacterAnimation, load_body_clip};
    use starbreaker_3d::character::loadout::parse_loadout;
    use starbreaker_3d::character::package::export_character_package;
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    let chf = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb).unwrap();
    let loadout = parse_loadout(&p4k.read_file(r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\UEE\marine_Armor_Heavy.xml").unwrap()).unwrap();
    let dba_path = r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba";
    let idle = load_body_clip(&p4k.read_file(dba_path).unwrap(), dba_path, Some("char_customizer_stand_idle_01")).unwrap();
    let animation = CharacterAnimation { face: None, body: Some(&idle) };
    let glb = chf_to_character_glb(&chf, p4k, &db, &animation, &loadout).unwrap();

    let opts = ExportOptions { kind: ExportKind::Decomposed, format: ExportFormat::Blend, ..ExportOptions::default() };
    let export = export_character_package(p4k, &db, &chf, "masculinedefault", &opts, &animation, &loadout).unwrap();
    let files: HashMap<String, Vec<u8>> = export.files.into_iter().map(|f| (f.relative_path, f.bytes)).collect();
    let (scene_path, scene) = files.iter().find(|(p, _)| p.ends_with("/scene.blend")).expect("scene.blend");
    let blend = from_blend(scene, scene_path, &|p| files.get(p).cloned()).unwrap();
    // Not vacuous: head + 3 sub-meshes + 5 items + 5 garments (the body is
    // covered by the undersuit), animated by the body clip.
    assert_eq!(blend.meshes.len(), 14, "{:?}", blend.meshes.keys());
    assert!(!blend.meshes.keys().any(|m| m.starts_with("m_body")), "the covered body is not written");
    assert!(blend.meshes.keys().any(|m| m.starts_with("m_cds_heavy_armor_01_core")));
    assert!(blend.clips["char_customizer_stand_idle_01"].channels.len() > 100);
    diff("armoured", &from_glb(&glb).unwrap(), &blend)
}

/// The CHF character in a loadout with a full helmet, playing a face and a
/// body clip: the helmet hides the head's port, so the DNA head is written
/// without geometry and its eyes/teeth and the CHF hair/brows/… go, in both.
fn helmeted(p4k: &MappedP4k) -> Vec<String> {
    use starbreaker_3d::character::chf_to_character_glb;
    use starbreaker_3d::character::clips::{CharacterAnimation, FaceAnimation, load_body_clip};
    use starbreaker_3d::character::loadout::parse_loadout;
    use starbreaker_3d::character::package::export_character_package;
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    let chf = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb).unwrap();
    let loadout = parse_loadout(&p4k.read_file(r"Data\Scripts\Loadouts\Character\Human\Armor\NPC\Pirate\Pirate_Heavy_01.xml").unwrap()).unwrap();
    let caf = p4k
        .read_file(r"Data\Animations\Characters\Human\heads\male\ai\idlesets\happy\face\ai_civ_happy_idle_02_face.caf")
        .unwrap();
    let dba_path = r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba";
    let idle = load_body_clip(&p4k.read_file(dba_path).unwrap(), dba_path, Some("char_customizer_stand_idle_01")).unwrap();
    let animation = CharacterAnimation { face: Some(FaceAnimation { caf: &caf, intensity: 1.0 }), body: Some(&idle) };
    let glb = chf_to_character_glb(&chf, p4k, &db, &animation, &loadout).unwrap();

    let opts = ExportOptions { kind: ExportKind::Decomposed, format: ExportFormat::Blend, ..ExportOptions::default() };
    let export = export_character_package(p4k, &db, &chf, "masculinedefault", &opts, &animation, &loadout).unwrap();
    let files: HashMap<String, Vec<u8>> = export.files.into_iter().map(|f| (f.relative_path, f.bytes)).collect();
    let (scene_path, scene) = files.iter().find(|(p, _)| p.ends_with("/scene.blend")).expect("scene.blend");
    let blend = from_blend(scene, scene_path, &|p| files.get(p).cloned()).unwrap();
    // Not vacuous: the 6 garments (helmet included), no head or head items,
    // the face clip's bones still playing.
    assert!(!blend.meshes.keys().any(|m| m.starts_with("protos_human_male_face_t1_pu") || m.starts_with("m_hair")), "{:?}", blend.meshes.keys());
    assert!(blend.meshes.keys().any(|m| m.starts_with("m_slaver_heavy_armor_helmet_01")), "{:?}", blend.meshes.keys());
    let clip = &blend.clips["char_customizer_stand_idle_01"];
    assert!(clip.channels.keys().any(|c| c.contains("FACIAL_")), "the face clip still moves the facial bones");
    // The helmet mesh has a slot past its .mtl: it draws with the engine
    // default material, which its sidecar carries at that slot.
    let (_, sidecar) = files
        .iter()
        .find(|(p, _)| p.to_lowercase().contains("m_slaver_heavy_armor_helmet_01_01_01_tex") && p.ends_with(".materials.json"))
        .unwrap_or_else(|| panic!("helmet sidecar: {:?}", files.keys().filter(|p| p.contains("helmet")).collect::<Vec<_>>()));
    let sidecar: serde_json::Value = serde_json::from_slice(sidecar).unwrap();
    let names: Vec<&str> = sidecar["submaterials"].as_array().unwrap().iter().filter_map(|s| s["submaterial_name"].as_str()).collect();
    assert_eq!(names.last(), Some(&"material_default"), "{names:?}");
    diff("helmeted", &from_glb(&glb).unwrap(), &blend)
}

/// A single armour piece through the entity pipeline: GLB vs package.
fn piece(p4k: &MappedP4k) -> Vec<String> {
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    use starbreaker_datacore::loadout::{EntityIndex, resolve_loadout_indexed};
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = starbreaker_datacore::Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    let record = index.find_record("cds_armor_heavy_arms_01_01_01").unwrap();
    let tree = resolve_loadout_indexed(&index, record);
    let glb_opts = ExportOptions { geometry_tag: Some("Male".into()), lod_level: 0, ..ExportOptions::default() };
    let glb = starbreaker_3d::assemble_glb_with_loadout(&db, p4k, record, &tree, &glb_opts).unwrap();
    let pkg_opts = ExportOptions {
        kind: ExportKind::Decomposed,
        format: ExportFormat::Blend,
        geometry_tag: Some("Male".into()),
        lod_level: 0,
        ..ExportOptions::default()
    };
    let pkg = starbreaker_3d::assemble_glb_with_loadout(&db, p4k, record, &tree, &pkg_opts).unwrap();
    let files: HashMap<String, Vec<u8>> = pkg.decomposed.unwrap().files.into_iter().map(|f| (f.relative_path, f.bytes)).collect();
    let (scene_path, scene) = files.iter().find(|(p, _)| p.ends_with("/scene.blend")).expect("scene.blend");
    let blend = from_blend(scene, scene_path, &|p| files.get(p).cloned()).unwrap();
    // Not vacuous: the arms, skinned.
    assert!(blend.meshes.values().any(|m| m.joints.len() > 20 && m.vertices > 10_000), "{:?}", blend.meshes.keys());
    diff("piece", &from_glb(glb.bundled_bytes().unwrap()).unwrap(), &blend)
}

#[test]
fn glb_and_blend_exports_are_in_parity() {
    let Some(p4k) = p4k() else { return };
    let mut diffs = armour(&p4k);
    diffs.extend(character(&p4k));
    diffs.extend(armoured(&p4k));
    diffs.extend(helmeted(&p4k));
    diffs.extend(piece(&p4k));
    eprintln!("parity differences: {diffs:#?}");
    Allowlist::parse(ALLOWLIST).check(&["armour", "character", "armoured", "helmeted", "piece"], &diffs, true).unwrap();
}
