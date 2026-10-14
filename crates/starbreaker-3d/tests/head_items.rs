//! CHF head items (hair, brows, lashes, beard, scalp) against real data:
//! resolution through DataCore and wrapping onto the per-character head.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_3d::character::items::{is_wrap_deformer, load_head_items, resolve_head_items};
use starbreaker_datacore::Database;
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

fn chf(p4k: &MappedP4k, name: &str) -> starbreaker_chf::ChfData {
    let bytes = p4k.read_file(&format!(r"Data\Libs\CharacterCustomizer\{name}.chf")).unwrap();
    starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap()
}

#[test]
fn default_characters_resolve_their_wrap_items() {
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    for (name, gender) in [("masculinedefault", "Male"), ("femininedefault", "Female")] {
        let items = resolve_head_items(&db, &chf(&p4k, name).itemport, gender);
        for i in &items {
            eprintln!("{name}: {} [{}]", i.geometry, i.deformer);
            assert!(is_wrap_deformer(&i.deformer));
            let dir = format!("/{}/", gender.to_ascii_lowercase());
            assert!(i.geometry.to_ascii_lowercase().contains(&dir), "{name}: wrong gender {}", i.geometry);
        }
        assert!(items.len() >= 2, "{name}: only {} wrap items", items.len());
        let loaded = load_head_items(&p4k, &items);
        assert_eq!(loaded.len(), items.len(), "{name}: some items failed to load");
        assert!(loaded.iter().all(|m| !m.mesh.positions.is_empty()));
    }
}

/// Distance from each point to its nearest point of `cloud` (brute force,
/// fine for test-sized meshes).
fn clearance(points: &[[f32; 3]], cloud: &[[f32; 3]]) -> Vec<f32> {
    points
        .iter()
        .map(|p| {
            cloud
                .iter()
                .map(|c| (0..3).map(|k| (p[k] - c[k]).powi(2)).sum::<f32>())
                .fold(f32::MAX, f32::min)
                .sqrt()
        })
        .collect()
}

/// Wrapped onto a strongly reshaped head (pure `silas_t1`), the scalp keeps
/// its authored clearance from the head surface; left unwrapped it does not.
/// Also checks the identity GLB carries every item mesh.
#[test]
fn wrapped_scalp_keeps_its_clearance_on_a_reshaped_head() {
    use starbreaker_3d::character::{
        HeadIdentityContext, dna_matrix, head_skin::parse_head_skin, identity::MALE_T1_PU_GENDER_HASH,
        morphing, wrap::HeadDeltaField,
    };
    use starbreaker_chf::{DnaBlend, FacePart::*};
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let items = load_head_items(&p4k, &resolve_head_items(&db, &chf(&p4k, "masculinedefault").itemport, "Male"));
    let scalp = items.iter().find(|i| i.item.geometry.contains("universal_scalp")).expect("scalp item");

    let dir = r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu";
    let skin = p4k.read_file(&format!(r"{dir}\protos_human_male_face_t1_pu_head.skin")).unwrap();
    let skinm = p4k.read_file(&format!(r"{dir}\protos_human_male_face_t1_pu_head.skinm")).unwrap();
    let head = parse_head_skin(&skin, &skinm).unwrap();
    let dna_bytes = p4k.read_file(&format!(r"{dir}\protos_human_male_face_t1_pu_head.dna")).unwrap();
    let dna = starbreaker_dna::DnaFile::read(&dna_bytes).unwrap();
    let silas = dna.variant_names().unwrap().iter().position(|n| n == "silas_t1").unwrap() as u16;

    let none = DnaBlend { value: 0, head_id: 0 };
    let full = DnaBlend { value: u16::MAX, head_id: silas };
    let parts = [EyebrowLeft, EyebrowRight, EyeLeft, EyeRight, Nose, EarLeft, EarRight, CheekLeft, CheekRight, Mouth, Jaw, Crown, Neck];
    let chf_dna = starbreaker_chf::Dna {
        raw_bytes: Vec::new(),
        gender_hash: MALE_T1_PU_GENDER_HASH,
        variant_hash: starbreaker_common::NameHash(0),
        part_count: 13,
        blends_per_part: 4,
        header_unknown: 0,
        max_head_id: silas,
        face_parts: parts.into_iter().map(|p| (p, [full, none, none, none])).collect(),
    };
    let (nv, bpp) = (dna.variant_records.len(), dna.header.mesh_blends_per_pool as usize);
    let w = dna_matrix::compute_chf_weights(&chf_dna, nv, bpp).unwrap();
    let m = dna_matrix::build_dna_matrix(&dna_matrix::prune_top_k(&w, nv, bpp).unwrap(), nv, bpp).unwrap();
    let base = &head.mesh.positions;
    let deltas = morphing::compute_neutral_vertex_deltas(&dna.neutral_morphs(base.len()).unwrap(), &m);
    let morphed: Vec<[f32; 3]> = base.iter().zip(&deltas).map(|(p, d)| [0, 1, 2].map(|k| p[k] + d[k])).collect();

    let authored = clearance(&scalp.mesh.positions, base);
    let mut wrapped = scalp.mesh.positions.clone();
    HeadDeltaField::new(base, &deltas).apply(&mut wrapped);
    let mean_change = |pos: &[[f32; 3]]| {
        let c = clearance(pos, &morphed);
        c.iter().zip(&authored).map(|(a, b)| (a - b).abs()).sum::<f32>() / c.len() as f32
    };
    let (unwrapped_err, wrapped_err) = (mean_change(&scalp.mesh.positions), mean_change(&wrapped));
    eprintln!("scalp clearance change: unwrapped {:.3} mm, wrapped {:.3} mm", unwrapped_err * 1e3, wrapped_err * 1e3);
    assert!(wrapped_err < 0.5e-3, "wrapped scalp drifts {:.3} mm from its authored clearance", wrapped_err * 1e3);
    assert!(wrapped_err < unwrapped_err / 3.0);

    let ctx = HeadIdentityContext::load(&p4k, MALE_T1_PU_GENDER_HASH).unwrap();
    let glb = ctx.build_character_glb(&chf_dna, &items, &Default::default(), &Default::default()).unwrap();
    let doc = gltf::Gltf::from_slice(&glb).unwrap();
    let counts: Vec<usize> = doc
        .meshes()
        .flat_map(|m| m.primitives().collect::<Vec<_>>())
        .filter_map(|p| p.get(&gltf::Semantic::Positions).map(|a| a.count()))
        .collect();
    for i in &items {
        assert!(counts.contains(&i.mesh.positions.len()), "{} missing from GLB", i.item.geometry);
    }
}

/// CHF material overrides resolve through DataCore's
/// `SCharacterGenerationParams.materialLookupTable`: the default characters
/// select their per-character head `.mtl` and blemish textures, and carry the
/// skin tone (`BodyColor`) and eye colour.
#[test]
fn default_characters_resolve_customized_materials() {
    use starbreaker_3d::character::customization::{MaterialLookup, resolve_chf_materials};
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let lookup = MaterialLookup::load(&db).unwrap();
    assert!(lookup.materials.len() > 50 && lookup.textures.len() > 20);
    for name in ["masculinedefault", "femininedefault"] {
        let mats = resolve_chf_materials(&lookup, &chf(&p4k, name).materials);
        let head = mats.iter().find(|m| m.name == "Head Material").expect("Head Material");
        let mtl = head.mtl_path.as_deref().expect("head .mtl");
        assert!(mtl.to_ascii_lowercase().ends_with("_head_material.mtl"), "{name}: {mtl}");
        assert!(p4k.read_file(&format!(r"Data\{}", mtl.replace('/', r"\"))).is_ok(), "{name}: {mtl} missing");
        let sub = &head.submaterials[0];
        assert!(sub.textures.get("TexSlot5").is_some_and(|t| t.to_lowercase().contains("blemish_mask")), "{name}: {:?}", sub.textures);
        let body = mats.iter().find(|m| m.name == "BodyMaterial").expect("BodyMaterial");
        assert!(body.submaterials.iter().any(|s| s.colors.contains_key("BodyColor")));
        eprintln!("{name}: head {mtl}, tone {:?}", body.submaterials[0].colors.get("BodyColor"));
    }
}

/// The CHF character exports as a decomposed package: `.blend` parts, a
/// HumanSkin_V2 head sidecar for the CHF-selected `.mtl` with its texture
/// slots exported, and the resolved customization on the root instance.
#[test]
fn chf_character_exports_a_decomposed_package() {
    use starbreaker_3d::character::package::export_character_package;
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let opts = ExportOptions { kind: ExportKind::Decomposed, format: ExportFormat::Blend, ..ExportOptions::default() };
    let export = export_character_package(&p4k, &db, &chf, "masculinedefault", &opts, &Default::default(), &[]).unwrap();
    let file = |suffix: &str| export.files.iter().find(|f| f.relative_path.to_lowercase().ends_with(suffix)).map(|f| &f.bytes);

    let scene: serde_json::Value = serde_json::from_slice(file("/scene.json").expect("scene.json")).unwrap();
    let root = &scene["root_entity"];
    assert_eq!(root["entity_name"], "masculinedefault");
    assert_eq!(root["character"]["gender"], "Male");
    let customization = root["character"]["customization"].as_array().unwrap();
    assert!(customization.iter().any(|m| m["name"] == "BodyMaterial"));
    let sidecar_path = root["material_sidecar"].as_str().expect("head sidecar").to_lowercase();
    assert!(sidecar_path.contains("male17_t1_head_material"), "{sidecar_path}");
    let sidecar: serde_json::Value = serde_json::from_slice(file(&sidecar_path).expect("sidecar file")).unwrap();
    let sub = &sidecar["submaterials"][0];
    assert_eq!(sub["shader_family"], "HumanSkin_V2");
    let slots: Vec<&str> = sub["texture_slots"].as_array().unwrap().iter().filter_map(|t| t["slot"].as_str()).collect();
    for s in ["TexSlot1", "TexSlot2", "TexSlot3", "TexSlot4", "TexSlot7", "TexSlot8", "TexSlot11", "TexSlot12"] {
        assert!(slots.contains(&s), "head sidecar missing {s}: {slots:?}");
    }
    // CHF override textures (blemish mask/ID, makeup) ship as package PNGs.
    let head = customization.iter().find(|m| m["name"] == "Head Material").unwrap();
    let exports = head["submaterials"][0]["texture_exports"].as_object().expect("texture_exports");
    for slot in ["TexSlot5", "TexSlot10"] {
        let png = exports[slot].as_str().unwrap_or_else(|| panic!("{slot} not exported: {exports:?}"));
        assert!(png.ends_with(".png") && file(&png.to_lowercase()).is_some(), "{slot} -> {png}");
    }
    // Dye slots bind to their item's material (submaterial 0), colours named
    // after the HairPBR params.
    let hair = customization.iter().find(|m| m["name"] == "HairDyeMaterial").expect("hair dye");
    let target = hair["applies_to"]["mtl_path"].as_str().expect("hair dye applies_to");
    assert!(target.to_lowercase().ends_with(".mtl") && target.to_lowercase().contains("hair"), "{target}");
    assert_eq!(hair["applies_to"]["submaterial_index"], 0);
    assert!(hair["submaterials"][0]["colors"].get("DyeColor").is_some());
    assert!(hair["submaterials"][0]["colors"].get("BaseTintColor").is_some());
    // The skin tone slot binds to the body, which uses the CHF's body .mtl.
    let body = customization.iter().find(|m| m["name"] == "BodyMaterial").unwrap();
    let target = body["applies_to"]["mtl_path"].as_str().expect("body material applies_to");
    let norm = |p: &str| p.to_lowercase().replace('\\', "/").trim_start_matches("data/").to_owned();
    assert_eq!(norm(target), norm(body["mtl_path"].as_str().unwrap()), "the body carries the CHF's body .mtl");
    let children = scene["children"].as_array().unwrap();
    assert!(children.len() >= 8, "eyes/teeth/eye-details + 5 items + body, got {}", children.len());
    assert!(children.iter().all(|c| c["parent_entity_name"] == "masculinedefault"));
    assert!(export.files.iter().any(|f| { let p = f.relative_path.to_lowercase(); p.contains("male17_t1_head_diff") && p.ends_with(".png") }));
    eprintln!("{} files, {} children", export.files.len(), children.len());
}

/// The universal scalp has no fixed material: it picks `f_hair_NN.mtl` through
/// `materialVariants` keyed by gender and the `scalpVHair_NN` tag the hair
/// item publishes (`$scalpVHair_36+`).
#[test]
fn universal_scalp_takes_the_material_published_by_the_hair() {
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let items = resolve_head_items(&db, &chf(&p4k, "femininedefault").itemport, "Female");
    let scalp = items.iter().find(|i| i.geometry.contains("universal_scalp")).expect("scalp item");
    assert!(scalp.material.to_lowercase().ends_with("female/f_hair_36.mtl"), "{}", scalp.material);
}

fn package(p4k: &MappedP4k, db: &Database, chf: &str, animation: &starbreaker_3d::character::clips::CharacterAnimation) -> starbreaker_3d::DecomposedExport {
    use starbreaker_3d::{ExportFormat, ExportKind, ExportOptions};
    let bytes = p4k.read_file(&format!(r"Data\Libs\CharacterCustomizer\{chf}.chf")).unwrap();
    let opts = ExportOptions { kind: ExportKind::Decomposed, format: ExportFormat::Blend, ..ExportOptions::default() };
    starbreaker_3d::character::package::export_character_package(p4k, db, &bytes, chf, &opts, animation, &[]).unwrap()
}

/// Reshaped parts are each character's own geometry: two characters exported
/// into one folder must not write different meshes to the same path.
#[test]
fn characters_do_not_share_reshaped_meshes() {
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let a = package(&p4k, &db, "masculinedefault", &Default::default());
    let b = package(&p4k, &db, "femininedefault", &Default::default());
    let meshes = |e: &starbreaker_3d::DecomposedExport| -> std::collections::HashMap<String, Vec<u8>> {
        e.files.iter().filter(|f| f.relative_path.ends_with(".blend") && !f.relative_path.ends_with("scene.blend")).map(|f| (f.relative_path.clone(), f.bytes.clone())).collect()
    };
    let (ma, mb) = (meshes(&a), meshes(&b));
    assert!(ma.keys().any(|p| p.contains("Characters/masculinedefault/") && p.contains("_head_LOD0")), "{:?}", ma.keys());
    for (path, bytes) in &ma {
        if let Some(other) = mb.get(path) {
            assert!(other == bytes, "{path}: two characters write different meshes to one path");
        }
    }
}

/// A package with a face animation carries it as Blender actions: the
/// armature's in scene.blend, the shape keys' in the head's own asset. With
/// a body clip too, both play as one clip named after the body clip.
#[test]
fn animated_packages_carry_actions() {
    use starbreaker_3d::character::clips::{CharacterAnimation, FaceAnimation, load_body_clip};
    let Some(p4k) = p4k() else { return };
    let dcb = p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap();
    let db = Database::from_bytes(&dcb).unwrap();
    let caf = p4k.read_file(r"Data\Animations\Characters\Human\heads\male\ai\idlesets\happy\face\ai_civ_happy_idle_02_face.caf").unwrap();
    let face = Some(FaceAnimation { caf: &caf, intensity: 1.0 });
    let has = |bytes: &[u8], needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    let blend = |export: &starbreaker_3d::DecomposedExport, suffix: &str| {
        let f = export.files.iter().find(|f| f.relative_path.ends_with(suffix)).unwrap_or_else(|| panic!("{suffix}"));
        starbreaker_blend::decompress_blend_bytes_if_needed(&f.bytes).unwrap()
    };

    let export = package(&p4k, &db, "masculinedefault", &CharacterAnimation { face, body: None });
    assert!(has(&blend(&export, "scene.blend"), b"ACexpression\0"), "armature action in scene.blend");
    assert!(has(&blend(&export, "_head_LOD0.blend"), b"ACexpression_shapes\0"), "shape-key action with the head mesh");

    let dba_path = r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba";
    let idle = load_body_clip(&p4k.read_file(dba_path).unwrap(), dba_path, Some("char_customizer_stand_idle_01")).unwrap();
    let export = package(&p4k, &db, "masculinedefault", &CharacterAnimation { face, body: Some(&idle) });
    assert!(has(&blend(&export, "scene.blend"), b"ACchar_customizer_stand_idle_01\0"), "one combined armature action");
    assert!(!has(&blend(&export, "scene.blend"), b"ACexpression\0"), "the face clip joined the body clip");
    assert!(has(&blend(&export, "_head_LOD0.blend"), b"ACchar_customizer_stand_idle_01_shapes\0"));
}
