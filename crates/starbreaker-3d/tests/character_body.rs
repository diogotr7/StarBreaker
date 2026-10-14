//! The CHF character's body (`character::body`): the item on the CHF's root
//! port, its gender geometry and CHF-customized material, and the player
//! actor's skeleton; in the character GLB the body joins the head on one
//! skeleton built on that actor skeleton.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_3d::character::body::{load_body, resolve_body};
use starbreaker_3d::character::customization::{MaterialLookup, resolve_chf_materials};
use starbreaker_3d::character::outfit::Outfit;
use starbreaker_3d::character::{HeadIdentityContext, gender_tag, items};
use starbreaker_datacore::Database;
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

fn chf(p4k: &MappedP4k, name: &str) -> starbreaker_chf::ChfData {
    let bytes = p4k.read_file(&format!(r"Data\Libs\CharacterCustomizer\{name}.chf")).unwrap();
    starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap()
}

fn db(p4k: &MappedP4k) -> Vec<u8> {
    p4k.read_file(r"Data\Game2.dcb").or_else(|_| p4k.read_file(r"Data\Game.dcb")).unwrap()
}

/// The root port's item resolves to the gender's body geometry, the CHF's
/// `BodyMaterial` `.mtl` (when the CHF selects one) and the player actor's
/// skeleton for that gender.
#[test]
fn default_characters_resolve_their_body() {
    let Some(p4k) = p4k() else { return };
    let dcb = db(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let lookup = MaterialLookup::load(&db).unwrap();
    for (name, geometry, skeleton) in [
        ("masculinedefault", "male_v7/body/m_body.skin", "male_v7/export/bhm_skeleton_v7.chr"),
        ("femininedefault", "female_v2/body/f_body.skin", "female_v2/export/bhf_skeleton_v2.chr"),
    ] {
        let chf = chf(&p4k, name);
        let customization = resolve_chf_materials(&lookup, &chf.materials);
        let body = resolve_body(&db, &chf.itemport, gender_tag(chf.dna.gender_hash), &customization).expect(name);
        assert_eq!(body.port, "body_itemport");
        assert!(body.geometry.to_lowercase().ends_with(geometry), "{name}: {}", body.geometry);
        assert!(body.skeleton.to_lowercase().ends_with(skeleton), "{name}: {}", body.skeleton);
        let selected = customization.iter().find(|m| m.name == "BodyMaterial").and_then(|m| m.mtl_path.clone());
        match selected {
            Some(mtl) => assert_eq!(body.material, mtl, "{name}: the CHF's body material"),
            None => assert!(body.material.to_lowercase().ends_with(".mtl"), "{name}: {}", body.material),
        }
        let loaded = load_body(&p4k, &body).unwrap();
        assert!(loaded.actor.len() >= 200, "{name}: actor skeleton has {} bones", loaded.actor.len());
        for bone in &loaded.skeleton.bones {
            assert!(loaded.actor.iter().any(|b| b.name == bone.name), "{name}: body bone {} not in the actor skeleton", bone.name);
        }
    }
}

/// The character GLB carries the body on one skeleton with the head: built
/// on the actor skeleton (`Spine3` under `Spine2`, the facial bones under
/// `Head`), the body skinned to it at rest, and the head's neck ring meeting
/// the body's neck.
#[test]
fn character_glb_includes_the_body() {
    let Some(p4k) = p4k() else { return };
    let dcb = db(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let gender = gender_tag(chf.dna.gender_hash);
    let customization = resolve_chf_materials(&MaterialLookup::load(&db).unwrap(), &chf.materials);
    let body = load_body(&p4k, &resolve_body(&db, &chf.itemport, gender, &customization).unwrap()).unwrap();
    let items = items::load_head_items(&p4k, &items::resolve_head_items(&db, &chf.itemport, gender));
    let glb = HeadIdentityContext::load(&p4k, chf.dna.gender_hash)
        .unwrap()
        .build_character_glb(&chf.dna, &items, &Outfit::with_body(Some(body.clone())), &Default::default())
        .unwrap();

    let (doc, buffers, _) = gltf::import_slice(&glb).unwrap();
    let parent_of = |name: &str| -> Option<String> {
        let child = doc.nodes().find(|n| n.name() == Some(name))?.index();
        doc.nodes().find(|n| n.children().any(|c| c.index() == child)).and_then(|n| n.name().map(str::to_owned))
    };
    // The whole actor skeleton (not only the bones the meshes weight).
    for bone in &body.actor {
        assert!(doc.nodes().any(|n| n.name() == Some(bone.name.as_str())), "actor bone {} missing", bone.name);
    }
    assert_eq!(parent_of("Spine3").as_deref(), Some("Spine2"), "the actor's hierarchy");
    assert_eq!(parent_of("Hips").as_deref(), Some("World"));
    let mut bone = "FACIAL_C_Neck1Root".to_owned();
    while bone != "World" && bone != "Head" && bone != "Neck1" {
        bone = parent_of(&bone).expect("parent");
    }
    assert_ne!(bone, "World", "facial bones hang under the neck/head");

    let positions = |mesh: &str| -> Vec<[f32; 3]> {
        let m = doc.meshes().find(|m| m.name() == Some(mesh)).unwrap_or_else(|| panic!("no mesh {mesh}"));
        // Primitives (submeshes) share one vertex buffer: read each once.
        let mut seen = Vec::new();
        m.primitives()
            .filter(|p| {
                let accessor = p.get(&gltf::Semantic::Positions).unwrap().index();
                !seen.contains(&accessor) && {
                    seen.push(accessor);
                    true
                }
            })
            .flat_map(|p| p.reader(|b| Some(&buffers[b.index()])).read_positions().unwrap().collect::<Vec<_>>())
            .collect()
    };
    let body_positions = positions("m_body");
    assert_eq!(body_positions.len(), body.mesh.positions.len(), "the whole body");
    // Rest: the body's GLB vertices are its source vertices (mesh data stays
    // in CryEngine space; the root node turns it Y-up).
    for (glb, src) in body_positions.iter().zip(&body.mesh.positions).step_by(97) {
        assert!(glb.iter().zip(src.iter()).all(|(a, b)| (a - b).abs() < 1e-4), "{glb:?} vs {src:?}");
    }
    // Neck seam: the head's open boundary in its lower third (the neck ring;
    // eye and mouth openings sit higher) lies on the body.
    let head_mesh = doc.meshes().find(|m| m.name() == Some("protos_human_male_face_t1_pu_head")).unwrap();
    let head = positions("protos_human_male_face_t1_pu_head");
    // Weld split vertices (UV seams) by position first, or seams look open.
    let key = |p: &[f32; 3]| p.map(|c| (c * 1e5).round() as i64);
    let mut welded: std::collections::HashMap<[i64; 3], u32> = Default::default();
    let id: Vec<u32> = head.iter().enumerate().map(|(i, p)| *welded.entry(key(p)).or_insert(i as u32)).collect();
    let mut edges: std::collections::HashMap<(u32, u32), u32> = Default::default();
    for p in head_mesh.primitives() {
        let indices: Vec<u32> = p.reader(|b| Some(&buffers[b.index()])).read_indices().unwrap().into_u32().collect();
        for t in indices.chunks(3) {
            let t = [id[t[0] as usize], id[t[1] as usize], id[t[2] as usize]];
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
    }
    let (lo, hi) = head.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[2]), hi.max(p[2])));
    let mut ring: Vec<&[f32; 3]> = edges
        .iter()
        .filter(|(_, n)| **n == 1)
        .flat_map(|((a, b), _)| [*a, *b])
        .map(|v| &head[v as usize])
        .filter(|p| p[2] < lo + (hi - lo) / 3.0)
        .collect();
    ring.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ring.dedup();
    assert!(ring.len() > 10, "{} ring vertices", ring.len());
    let worst = ring
        .iter()
        .map(|h| body_positions.iter().map(|b| (0..3).map(|k| (h[k] - b[k]).powi(2)).sum::<f32>().sqrt()).fold(f32::MAX, f32::min))
        .fold(0.0f32, f32::max);
    eprintln!("neck seam: {} ring vertices, worst gap {:.2} mm", ring.len(), worst * 1000.0);
    assert!(worst < 0.002, "neck seam open by {:.2} mm", worst * 1000.0);
}

/// A body clip plays in the character GLB as one animation named after it,
/// driving the actor skeleton's bones.
#[test]
fn character_glb_plays_a_body_clip() {
    use starbreaker_3d::character::clips::{CharacterAnimation, load_body_clip};
    let Some(p4k) = p4k() else { return };
    let dcb = db(&p4k);
    let db = Database::from_bytes(&dcb).unwrap();
    let chf = chf(&p4k, "masculinedefault");
    let body = starbreaker_3d::character::body::load_chf_body(&p4k, &db, &chf).unwrap();
    let dba_path = r"Data\Animations\Characters\Human\male_v7\character_select_screen.dba";
    let idle = load_body_clip(&p4k.read_file(dba_path).unwrap(), dba_path, Some("char_customizer_stand_idle_01")).unwrap();
    let glb = HeadIdentityContext::load(&p4k, chf.dna.gender_hash)
        .unwrap()
        .build_character_glb(&chf.dna, &[], &Outfit::with_body(Some(body)), &CharacterAnimation { face: None, body: Some(&idle) })
        .unwrap();
    let (doc, _, _) = gltf::import_slice(&glb).unwrap();
    let animations: Vec<gltf::Animation> = doc.animations().collect();
    assert_eq!(animations.len(), 1);
    assert_eq!(animations[0].name(), Some("char_customizer_stand_idle_01"));
    let targets: std::collections::BTreeSet<String> =
        animations[0].channels().filter_map(|c| c.target().node().name().map(str::to_owned)).collect();
    assert!(targets.len() > 100, "{} animated bones", targets.len());
    for bone in ["Hips", "Spine3", "Head", "LeftForeArm"] {
        assert!(targets.contains(bone), "{bone} not animated");
    }
}
