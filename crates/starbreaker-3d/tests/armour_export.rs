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

/// A worn piece takes the palette of the gender node it is worn as. A core's
/// top-level geometry is its storage crate, with no palette; the set's
/// colours are on its `Male`/`Female` nodes, the same palette the helmet
/// carries at the top level.
#[test]
fn worn_pieces_carry_their_gender_node_palette() {
    use starbreaker_3d::character::loadout::worn_palette;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    let helmet = worn_palette(&db, index.find_record("kap_combat_light_helmet_01_01_01").unwrap(), "Male")
        .expect("helmet palette");
    for piece in ["kap_combat_light_core_01_01_01", "kap_combat_light_arms_01_01_01", "kap_combat_light_legs_01_01_01"] {
        for gender in ["Male", "Female"] {
            let palette = worn_palette(&db, index.find_record(piece).unwrap(), gender)
                .unwrap_or_else(|| panic!("{piece} {gender}: no palette"));
            assert_eq!(
                (palette.primary, palette.secondary, palette.tertiary),
                (helmet.primary, helmet.secondary, helmet.tertiary),
                "{piece} {gender}: same set colours as the helmet"
            );
        }
    }
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

/// A backpack hangs on its core's `backpack` bone port (not skinned: it has
/// no `.skin`), as a prop on the port helper's bone with the item's helper
/// that goes there; a Quirinus heavy core picks the sub-helper its backpack's
/// tags select.
#[test]
fn backpacks_hang_on_their_cores_bone() {
    use starbreaker_3d::character::loadout::resolve_garments;
    use starbreaker_3d::character::wear::dress_entries;
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    for (core, backpack, sub_helper) in [
        ("cds_combat_medium_core_04_01_01", "cds_combat_medium_backpack_02_01_01", false),
        ("qrt_utility_heavy_core_01_01_01", "qrt_utility_heavy_backpack_01_01_01", true),
    ] {
        let picked: Vec<String> = ["cds_undersuit_01_01_01", core, backpack].iter().map(|s| s.to_string()).collect();
        let placed = dress_entries(&db, &chf, &[], &picked).unwrap();
        assert!(placed.unplaced.is_empty(), "{backpack}: {:?}", placed.unplaced);
        let worn = resolve_garments(&db, &placed.entries, "Male", "body_itemport");
        assert!(worn.garments.iter().all(|g| g.item != backpack), "a backpack is not a garment");
        let prop = worn.props.iter().find(|p| p.item == backpack).unwrap_or_else(|| panic!("{backpack}: no prop in {:?}", worn.props));
        assert_eq!(prop.port, "backpack");
        assert_eq!(prop.helper.bone, "backpack_attach_1_override");
        assert_eq!(prop.helper.item_helper, "backpack_attach_1_loc");
        assert!(prop.geometry.to_lowercase().ends_with(".cga"), "{}", prop.geometry);
        assert_eq!(prop.helper.position != [0.0; 3], sub_helper, "{backpack}: {:?}", prop.helper);
    }
}

/// SRVL helmets' item ports resolve: the port query must not expand every
/// referenced record (it hit the DataCore materialization limit).
#[test]
fn srvl_helmets_resolve_their_ports() {
    use starbreaker_3d::character::wear::{dress_entries, wearable};
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    for helmet in ["srvl_helmet_01_01_12", "srvl_helmet_01_06_01"] {
        let w = wearable(&db, &index, helmet).unwrap_or_else(|e| panic!("{helmet}: {e}")).expect("a wearable");
        assert_eq!(w.item_type, "Char_Armor_Helmet");
    }
    let chf = chf(&p4k, "masculinedefault");
    let picked: Vec<String> = ["cds_undersuit_01_01_12", "srvl_helmet_01_06_01"].iter().map(|s| s.to_string()).collect();
    let placed = dress_entries(&db, &chf, &[], &picked).unwrap();
    assert!(placed.unplaced.is_empty(), "{:?}", placed.unplaced);
}

/// World transforms of a GLB's nodes by name (scene roots down).
fn node_worlds(glb: &[u8]) -> std::collections::HashMap<String, glam::Mat4> {
    fn walk(node: gltf::Node, parent: glam::Mat4, out: &mut std::collections::HashMap<String, glam::Mat4>) {
        let world = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(name) = node.name() {
            out.insert(name.to_owned(), world);
        }
        for child in node.children() {
            walk(child, world, out);
        }
    }
    let (doc, ..) = gltf::import_slice(glb).unwrap();
    let mut out = std::collections::HashMap::new();
    for scene in doc.scenes() {
        for node in scene.nodes() {
            walk(node, glam::Mat4::IDENTITY, &mut out);
        }
    }
    out
}

/// A backpack picked with no core hangs on a stand-in core: a core of the
/// backpack's own line when there is one, else one of the backpack's size.
/// The stand-in is flagged and not a garment (its mesh is not written); its
/// `.skin`'s mount bone joins the skeleton exactly where wearing that core
/// puts it, male and female.
#[test]
fn backpacks_without_a_core_hang_on_a_stand_in_core() {
    use starbreaker_3d::character::loadout::resolve_garments;
    use starbreaker_3d::character::outfit::Outfit;
    use starbreaker_3d::character::stand_in::dress_with_stand_ins;
    use starbreaker_3d::character::wear::{dress_entries, wearable};
    use starbreaker_3d::character::{Gender, HeadIdentityContext, default_wearer_chf, gender_tag, items};
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let index = EntityIndex::new(&db);
    const SUIT: &str = "cds_undersuit_01_01_12";
    const BONE: &str = "backpack_attach_1_override";
    for gender in [Gender::Male, Gender::Female] {
        let bytes = default_wearer_chf(&p4k, gender).unwrap();
        let chf = starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap();
        let tag = gender_tag(chf.dna.gender_hash);
        let context = HeadIdentityContext::load(&p4k, chf.dna.gender_hash).unwrap();
        let head_items = items::load_head_items(&p4k, &items::resolve_head_items(&db, &chf.itemport, tag));
        for (backpack, own_line) in [("kap_combat_heavy_backpack_01_01_15", Some("kap_combat_heavy_core_")), ("cds_combat_superheavy_backpack_01_01_01", None)] {
            let picked = [SUIT.to_string(), backpack.to_string()];
            assert_eq!(dress_entries(&db, &chf, &[], &picked).unwrap().unplaced.len(), 1, "no core: no port without a stand-in");
            let placed = dress_with_stand_ins(&p4k, &db, &chf, &[], &picked).unwrap();
            assert!(placed.unplaced.is_empty(), "{tag} {backpack}: {:?}", placed.unplaced);
            let core = placed.stand_ins.iter().find(|s| s.item == backpack).map(|s| s.core.clone()).unwrap_or_else(|| panic!("{tag} {backpack}: no stand-in"));
            eprintln!("{tag} {backpack}: stand-in {core}");
            match own_line {
                Some(line) => assert!(core.starts_with(line), "{backpack}: {core} is not of its line"),
                None => {
                    let pack = starbreaker_3d::character::wear::Wearable { rigid: true, ..wearable(&db, &index, backpack).unwrap().unwrap() };
                    let size = pack.size;
                    let port = wearable(&db, &index, &core).unwrap().unwrap().ports.into_iter().find(|p| p.bone.is_some() && p.accepts(&pack)).unwrap();
                    assert!(size.is_some() && port.max_size == size, "{core}: port size {:?}, backpack {size:?}", port.max_size);
                }
            }

            let worn = resolve_garments(&db, &placed.entries, tag, "body_itemport");
            assert!(worn.garments.iter().all(|g| g.item != core), "{core}: a stand-in is not a garment");
            assert_eq!(worn.stand_ins.iter().map(|s| s.item.as_str()).collect::<Vec<_>>(), [core.as_str()]);
            assert_eq!(worn.stand_ins[0].items, [backpack]);
            assert_eq!(worn.stand_ins[0].bones, [BONE], "only the bone of the port the backpack is on");
            let prop = worn.props.iter().find(|p| p.item == backpack).expect("the backpack is a prop");
            assert_eq!(prop.helper.bone, BONE);

            let outfit = Outfit::load(&p4k, &db, &chf, &placed.entries);
            assert_eq!(outfit.mounts.len(), 1, "{core}: its mount bone");
            let glb = context.build_character_glb(&chf.dna, &head_items, &outfit, &Default::default()).unwrap();
            let stand_in_mount = node_worlds(&glb).get(BONE).copied().unwrap_or_else(|| panic!("{tag} {backpack}: no {BONE} in the skeleton"));

            let real = dress_entries(&db, &chf, &[], &[SUIT.to_string(), core.clone(), backpack.to_string()]).unwrap();
            assert!(real.unplaced.is_empty(), "{:?}", real.unplaced);
            let real_outfit = Outfit::load(&p4k, &db, &chf, &real.entries);
            let real_glb = context.build_character_glb(&chf.dna, &head_items, &real_outfit, &Default::default()).unwrap();
            // The mount is where the core's own `.skin` binds the bone (glTF
            // Y-up: CryEngine (x, y, z) is (x, z, -y)), not the undersuit's.
            let skin = starbreaker_3d::character::stand_in::skin_skeleton(&p4k, &worn.stand_ins[0].geometry).unwrap();
            let bound = skin.bones.iter().find(|b| b.name == BONE).unwrap().world_translation;
            let expected = glam::Vec3::new(bound[0], bound[2], -bound[1]);
            let (_, _, at) = stand_in_mount.to_scale_rotation_translation();
            assert!(at.distance(expected) < 1e-3, "{tag} {backpack}: stand-in mount at {at}, {core}'s skin binds it at {expected}");
            assert!(at.z > 0.1 && at.y > 1.2, "{tag} {backpack}: mount {at} is not on the back");

            let core_meshes = real_outfit.garments.iter().find(|g| g.garment.item == core).map(|g| g.garment.geometry.clone()).unwrap();
            let stem = core_meshes.rsplit('/').next().unwrap().trim_end_matches(".skin").to_lowercase();
            let names = |glb: &[u8]| -> Vec<String> { gltf::import_slice(glb).unwrap().0.meshes().filter_map(|m| m.name().map(str::to_lowercase)).collect() };
            assert!(names(&real_glb).iter().any(|n| n.starts_with(&stem)), "{stem}: the worn core's mesh: {:?}", names(&real_glb));
            assert!(!names(&glb).iter().any(|n| n.starts_with(&stem)), "{stem}: the stand-in's mesh is written: {:?}", names(&glb));
        }
    }
}

/// A worn core's backpack mount is the core's own: the undersuit's `.skin`
/// carries a `backpack_attach_1_override` bone too (at the undersuit's
/// back), and the bone of the core whose port the backpack is on wins over
/// it. The same backpack then sits where its stand-in core puts it.
#[test]
fn a_worn_cores_mount_wins_over_the_undersuits() {
    use starbreaker_3d::character::outfit::Outfit;
    use starbreaker_3d::character::stand_in::{dress_with_stand_ins, skin_skeleton};
    use starbreaker_3d::character::wear::dress_entries;
    use starbreaker_3d::character::{Gender, HeadIdentityContext, default_wearer_chf, gender_tag, items};
    let Some(p4k) = p4k() else { return };
    let dcb = db_bytes(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    const SUIT: &str = "cds_undersuit_01_01_12";
    const CORE: &str = "kap_combat_heavy_core_01_01_01";
    const BACKPACK: &str = "kap_combat_heavy_backpack_01_01_15";
    const BONE: &str = "backpack_attach_1_override";
    for gender in [Gender::Male, Gender::Female] {
        let bytes = default_wearer_chf(&p4k, gender).unwrap();
        let chf = starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap();
        let tag = gender_tag(chf.dna.gender_hash);
        let context = HeadIdentityContext::load(&p4k, chf.dna.gender_hash).unwrap();
        let head_items = items::load_head_items(&p4k, &items::resolve_head_items(&db, &chf.itemport, tag));
        let mount = |picked: &[&str], stand_in: bool| {
            let picked: Vec<String> = picked.iter().map(|s| s.to_string()).collect();
            let placed = if stand_in { dress_with_stand_ins(&p4k, &db, &chf, &[], &picked) } else { dress_entries(&db, &chf, &[], &picked) }.unwrap();
            assert!(placed.unplaced.is_empty(), "{tag}: {:?}", placed.unplaced);
            let outfit = Outfit::load(&p4k, &db, &chf, &placed.entries);
            let glb = context.build_character_glb(&chf.dna, &head_items, &outfit, &Default::default()).unwrap();
            (node_worlds(&glb)[BONE].to_scale_rotation_translation().2, outfit)
        };
        let (worn, outfit) = mount(&[SUIT, CORE, BACKPACK], false);
        let bind = |item: &str| {
            let g = outfit.garments.iter().find(|g| g.garment.item == item).unwrap();
            let b = skin_skeleton(&p4k, &g.garment.geometry).unwrap().bones.into_iter().find(|b| b.name == BONE).unwrap().world_translation;
            glam::Vec3::new(b[0], b[2], -b[1])
        };
        let (core_bind, suit_bind) = (bind(CORE), bind(SUIT));
        assert!(core_bind.distance(suit_bind) > 0.05, "{tag}: the two skins' mounts differ: {core_bind} / {suit_bind}");
        assert!(worn.distance(core_bind) < 1e-3, "{tag}: mount at {worn}, the core binds it at {core_bind} (the undersuit at {suit_bind})");
        let (stood, _) = mount(&[SUIT, BACKPACK], true);
        assert!(stood.distance(worn) < 1e-3, "{tag}: stand-in mount {stood} vs worn core {worn}");
    }
}
