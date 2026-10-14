//! Ground-truth checks of the `.dna` identity data against the game's own
//! archetype head meshes.
//!
//! - slot 22 region weights are a per-vertex partition of unity;
//! - slot 2 names index the archetype heads (entry 0 = protos base);
//! - slot 18 decoded for one variant (all regions on) equals that archetype's
//!   `<name>_head.skinm` positions minus the protos base positions;
//! - slot 15 variant 0 equals the `.skin` parent-local joint translations, and
//!   the rest skeleton's locals reproduce its world records by FK;
//! - the neutral-identity GLB re-binds the per-character skeleton without
//!   deforming the rest pose.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_3d::character::head_skin::{parse_head_skin, parse_skin_mesh};
use starbreaker_dna::DnaFile;
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

fn rig_dir(g: &str) -> String {
    format!(r"Data\Objects\Characters\Human\heads\{g}\pu\protos_human_{g}_face_t1_pu")
}

fn read_head_positions(p4k: &MappedP4k, skin_path: &str) -> Vec<[f32; 3]> {
    let skin = p4k.read_file(skin_path).expect("read .skin");
    let skinm = p4k.read_file(&format!("{skin_path}m")).expect("read .skinm");
    match parse_head_skin(&skin, &skinm) {
        Ok(h) => h.mesh.positions,
        // Archetype heads carry no RigLogic/ProtosInfo chunks; mesh only.
        Err(_) => parse_skin_mesh(&skin, &skinm).expect("parse mesh").0.positions,
    }
}

/// Find `<name>_<part>.skin` anywhere under the gender's heads directory.
fn find_archetype_part(p4k: &MappedP4k, g: &str, name: &str, part: &str) -> Option<String> {
    let suffix = format!(r"\{name}_{part}.skin").to_ascii_lowercase();
    let prefix = format!(r"data\objects\characters\human\heads\{g}\").to_ascii_lowercase();
    p4k.search(&format!("{name}_{part}.skin"))
        .into_iter()
        .map(|i| p4k.entries()[i as usize].name.clone())
        .find(|n| {
            let l = n.to_ascii_lowercase();
            l.ends_with(&suffix) && l.starts_with(&prefix)
        })
}

fn find_archetype_skin(p4k: &MappedP4k, g: &str, name: &str) -> Option<String> {
    find_archetype_part(p4k, g, name, "head")
}

/// The DNA-driven protos parts: the head plus the `ProtosMisc` eyes/teeth,
/// each with its own `.dna` (the eyes/teeth ones cover only their regions).
const DNA_PARTS: [&str; 3] = ["head", "eyes", "teeth"];

#[test]
fn slot22_region_weights_are_partition_of_unity() {
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        for part in DNA_PARTS {
            let dir = rig_dir(g);
            let nv = read_head_positions(&p4k, &format!(r"{dir}\protos_human_{g}_face_t1_pu_{part}.skin")).len();
            let bytes = p4k.read_file(&format!(r"{dir}\protos_human_{g}_face_t1_pu_{part}.dna")).unwrap();
            let dna = DnaFile::read(&bytes).unwrap();
            let regions = dna.header.mesh_blends_per_pool as usize;
            let rw = dna.region_weights_with_num_vertices(nv).unwrap();
            for v in 0..nv {
                let sum: f32 = rw[v * regions..(v + 1) * regions].iter().sum();
                assert!((sum - 1.0).abs() < 1e-4, "{g} {part} vertex {v}: region weights sum {sum}");
            }
        }
    }
}

/// The eyes/teeth `.dna` files cover only their own face parts (slot 12):
/// eyes = EYE_L/EYE_R, teeth = NOSE_C/MOUTH_C.
#[test]
fn part_dna_region_lists() {
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        for (part, want) in [("head", (0..13).collect::<Vec<_>>()), ("eyes", vec![2, 3]), ("teeth", vec![4, 9])] {
            let bytes = p4k.read_file(&format!(r"{}\protos_human_{g}_face_t1_pu_{part}.dna", rig_dir(g))).unwrap();
            assert_eq!(DnaFile::read(&bytes).unwrap().mesh_regions().unwrap(), want, "{g} {part}");
        }
    }
}

#[test]
fn slot18_variant_equals_archetype_mesh_minus_base() {
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        let head_dna = p4k.read_file(&format!(r"{}\protos_human_{g}_face_t1_pu_head.dna", rig_dir(g))).unwrap();
        let names = DnaFile::read(&head_dna).unwrap().variant_names().unwrap();
        assert_eq!(names[0], format!("protos_human_{g}_face_t1_pu"));
        for part in DNA_PARTS {
            let dir = rig_dir(g);
            let base = read_head_positions(&p4k, &format!(r"{dir}\protos_human_{g}_face_t1_pu_{part}.skin"));
            let bytes = p4k.read_file(&format!(r"{dir}\protos_human_{g}_face_t1_pu_{part}.dna")).unwrap();
            let dna = DnaFile::read(&bytes).unwrap();
            assert_eq!(dna.header.num_variants as usize, names.len(), "{g} {part}: variant count");
            let morphs = dna.neutral_morphs(base.len()).unwrap();
            let local = morphs.regions().len();
            let mut checked = 0;
            for variant in [1, 2, names.len() / 2, names.len() - 1] {
                let Some(skin_path) = find_archetype_part(&p4k, g, &names[variant], part) else {
                    eprintln!("{g} {part}: variant {variant} ({}) has no mesh, skipped", names[variant]);
                    continue;
                };
                let arch = read_head_positions(&p4k, &skin_path);
                assert_eq!(arch.len(), base.len(), "{skin_path} topology differs from base");
                let mut coef = vec![0.0f32; morphs.num_controls()];
                for r in 0..local {
                    coef[(variant - 1) * local + r] = 1.0;
                }
                let decoded = morphs.decode(&coef);
                let mut worst = 0.0f32;
                for v in 0..base.len() {
                    for k in 0..3 {
                        worst = worst.max((decoded[v][k] - (arch[v][k] - base[v][k])).abs());
                    }
                }
                // slot 18 is quantised to 8-bit codes per (vertex, control).
                assert!(worst < 2e-4, "{g} {part} {}: slot-18 vs mesh worst {:.4} mm", names[variant], worst * 1e3);
                checked += 1;
            }
            assert!(checked >= 2, "{g} {part}: too few archetype meshes found");
        }
    }
}

/// The head `.skin` rest skeleton's parent-local records reproduce its world
/// records by forward kinematics — so a re-bind (FK refresh after changing
/// locals) is consistent with the authored bind pose.
#[test]
fn rest_skeleton_fk_reproduces_world_records() {
    use starbreaker_3d::character::apply::refresh_world_from_locals;
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        let (head, _, _) = load_rig(&p4k, g);
        let mut bones = head.skeleton.bones.clone();
        refresh_world_from_locals(&mut bones);
        for (a, b) in bones.iter().zip(&head.skeleton.bones) {
            for k in 0..3 {
                assert!(
                    (a.world_translation[k] - b.world_translation[k]).abs() < 1e-4,
                    "{g} {}: FK {:?} vs authored {:?}",
                    b.name,
                    a.world_translation,
                    b.world_translation
                );
            }
        }
    }
}

/// Slot 15 holds PARENT-LOCAL joint translations: variant 0 equals the head
/// `.skin` bone locals, so variant-0-relative folds are local deltas (the
/// frame `apply_deltas` / `rebind_skeleton` add them in).
#[test]
fn slot15_variant0_equals_skin_local_translations() {
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        let (head, rig, dna_bytes) = load_rig(&p4k, g);
        let dna = DnaFile::read(&dna_bytes).unwrap();
        let s15 = dna.variant_neutral_joints(rig.num_joints as usize).unwrap();
        for (j, crc) in rig.skeleton_bone_name_crcs.iter().enumerate() {
            let b = head.skeleton.bones.iter().find(|b| b.name_crc == *crc).unwrap();
            for k in 0..3 {
                assert!(
                    (s15[j * 3 + k] - b.bind_translation[k]).abs() < 1e-5,
                    "{g} {}: slot15 {:?} vs local {:?}",
                    b.name,
                    &s15[j * 3..j * 3 + 3],
                    b.bind_translation
                );
            }
        }
    }
}

fn load_rig(
    p4k: &MappedP4k,
    g: &str,
) -> (
    starbreaker_3d::character::head_skin::HeadSkin,
    starbreaker_3d::riglogic::RigLogic,
    Vec<u8>,
) {
    let dir = rig_dir(g);
    let skin_path = format!(r"{dir}\protos_human_{g}_face_t1_pu_head.skin");
    let skin = p4k.read_file(&skin_path).unwrap();
    let skinm = p4k.read_file(&format!("{skin_path}m")).unwrap();
    let head = parse_head_skin(&skin, &skinm).unwrap();
    let rig = starbreaker_3d::riglogic::RigLogic::read(&head.riglogic_chunk[8..]).unwrap();
    let dna = p4k.read_file(&format!(r"{dir}\protos_human_{g}_face_t1_pu_head.dna")).unwrap();
    (head, rig, dna)
}

/// Joint world transforms (from the node hierarchy) of the first skin, paired
/// with its inverse bind matrices, and the joints' names and positions.
fn skin_rest_products(glb: &[u8]) -> (Vec<glam::Mat4>, Vec<(String, glam::Vec3)>) {
    let (doc, buffers, _) = gltf::import_slice(glb).expect("import glb");
    let mut global = vec![glam::Mat4::IDENTITY; doc.nodes().len()];
    fn walk(n: gltf::Node, parent: glam::Mat4, out: &mut [glam::Mat4]) {
        let m = parent * glam::Mat4::from_cols_array_2d(&n.transform().matrix());
        out[n.index()] = m;
        for c in n.children() {
            walk(c, m, out);
        }
    }
    for scene in doc.scenes() {
        for n in scene.nodes() {
            walk(n, glam::Mat4::IDENTITY, &mut global);
        }
    }
    let skin = doc.skins().next().expect("skin");
    let ibms: Vec<glam::Mat4> = skin
        .reader(|b| Some(&buffers[b.index()]))
        .read_inverse_bind_matrices()
        .expect("ibms")
        .map(|m| glam::Mat4::from_cols_array_2d(&m))
        .collect();
    // A skinned vertex renders at `jointGlobal * ibm * v`; at rest that must
    // equal the skinned node's own placement, so compare relative to it.
    let skinned_node = doc
        .nodes()
        .find(|n| n.skin().is_some_and(|s| s.index() == skin.index()))
        .expect("skinned node");
    let node_inverse = global[skinned_node.index()].inverse();
    let joints: Vec<_> = skin.joints().collect();
    let products = joints.iter().zip(&ibms).map(|(j, ibm)| node_inverse * global[j.index()] * *ibm).collect();
    let positions = joints
        .iter()
        .map(|j| (j.name().unwrap_or_default().to_owned(), global[j.index()].w_axis.truncate()))
        .collect();
    (products, positions)
}

/// The neutral-identity GLB re-binds the slot-15 per-character skeleton: its
/// joints sit at the character's positions (differ from the base rig), yet
/// every `joint_global × inverse_bind` is identity at rest, so skinning does
/// not deform the slot-18 head shape.
#[test]
fn identity_glb_rebinds_skeleton_without_rest_deformation() {
    use starbreaker_3d::character::HeadIdentityContext;
    let Some(p4k) = p4k() else { return };
    for (chf, g) in [("masculinedefault", "male"), ("femininedefault", "female")] {
        let bytes = p4k
            .read_file(&format!(r"Data\Libs\CharacterCustomizer\{chf}.chf"))
            .unwrap();
        let dna = starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap().dna;
        let glb = HeadIdentityContext::load(&p4k, dna.gender_hash).unwrap().build_glb(&dna, true, false).unwrap();
        let (products, positions) = skin_rest_products(&glb);
        let worst = products
            .iter()
            .map(|m| m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-4))
            .filter(|ok| !ok)
            .count();
        assert_eq!(worst, 0, "{chf}: {worst} joints deform the rest pose");

        // The base rig's bind pose, compared by joint-to-joint distance (free
        // of the glTF axis conversion).
        let (head, _, _) = load_rig(&p4k, g);
        let base: std::collections::HashMap<&str, glam::Vec3> = head
            .skeleton
            .bones
            .iter()
            .map(|b| (b.name.as_str(), glam::Vec3::from(b.world_translation)))
            .collect();
        let shared: Vec<_> = positions.iter().filter_map(|(n, p)| Some((*p, *base.get(n.as_str())?))).collect();
        assert!(shared.len() > 100, "{chf}: only {} GLB joints are head-rig bones", shared.len());
        let (anchor, base_anchor) = shared[0];
        let moved = shared
            .iter()
            .map(|(p, b)| (p.distance(anchor) - b.distance(base_anchor)).abs())
            .fold(0.0f32, f32::max);
        assert!(moved > 1e-4, "{chf}: identity skeleton identical to the base rig");
        eprintln!("{chf}: max joint relocation {:.2} mm", moved * 1e3);
    }
}

fn mean_angle_deg(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    let sum: f32 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = (x[0] * y[0] + x[1] * y[1] + x[2] * y[2]).clamp(-1.0, 1.0);
            d.acos().to_degrees()
        })
        .sum();
    sum / a.len() as f32
}

/// After the slot-18 morph, the transferred normals agree with the morphed
/// surface better than the stale base normals do.
#[test]
fn transferred_normals_track_the_morphed_surface() {
    use starbreaker_3d::character::{dna_matrix, morphing, normals::transfer_normals};
    let Some(p4k) = p4k() else { return };
    let chf_bytes = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let chf = starbreaker_chf::ChfFile::from_chf(&chf_bytes).unwrap().parse().unwrap();
    let (head, _, dna_bytes) = load_rig(&p4k, "male");
    let dna = DnaFile::read(&dna_bytes).unwrap();
    let (nv, bpp) = (dna.variant_records.len(), dna.header.mesh_blends_per_pool as usize);
    let w = dna_matrix::compute_chf_weights(&chf.dna, nv, bpp).unwrap();
    let m = dna_matrix::build_dna_matrix(&dna_matrix::prune_top_k(&w, nv, bpp).unwrap(), nv, bpp).unwrap();
    let mesh = &head.mesh;
    let deltas = morphing::compute_neutral_vertex_deltas(&dna.neutral_morphs(mesh.positions.len()).unwrap(), &m);
    let morphed: Vec<[f32; 3]> =
        mesh.positions.iter().zip(&deltas).map(|(p, d)| [p[0] + d[0], p[1] + d[1], p[2] + d[2]]).collect();

    // Reference: the morphed surface's own area-weighted geometric normals.
    let mut acc = vec![[0.0f32; 3]; morphed.len()];
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| morphed[i as usize]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for i in [t[0], t[1], t[2]] {
            for k in 0..3 {
                acc[i as usize][k] += n[k];
            }
        }
    }
    let geo: Vec<[f32; 3]> = acc
        .iter()
        .map(|n| {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            [n[0] / l, n[1] / l, n[2] / l]
        })
        .collect();
    let transferred = transfer_normals(&mesh.positions, &morphed, &mesh.indices, &mesh.normals);
    let stale = mean_angle_deg(&mesh.normals, &geo);
    let fresh = mean_angle_deg(&transferred, &geo);
    eprintln!("mean angle to morphed surface: base normals {stale:.3}°, transferred {fresh:.3}°");
    assert!(fresh < stale, "transfer did not improve normals ({fresh} vs {stale})");
}

/// The head `.skinm` authors up to 8 influences per vertex (`IVOBONEMAP32`);
/// all 8 are kept (top 4 in `joints`/`weights`, the rest in
/// `joints_hi`/`weights_hi`), sum to 1 together, and reach the GLB as
/// `JOINTS_1`/`WEIGHTS_1`.
#[test]
fn head_keeps_all_eight_skin_influences() {
    use starbreaker_3d::character::HeadIdentityContext;
    let Some(p4k) = p4k() else { return };
    let (head, _, _) = load_rig(&p4k, "male");
    let m = &head.mesh;
    assert_eq!(m.joints_hi.len(), m.positions.len());
    assert_eq!(m.weights_hi.len(), m.positions.len());
    let mut over_four = 0;
    for v in 0..m.positions.len() {
        let lo: f32 = m.weights[v].iter().sum();
        let hi: f32 = m.weights_hi[v].iter().sum();
        assert!((lo + hi - 1.0).abs() < 1e-4, "vertex {v}: weights sum {}", lo + hi);
        if hi > 0.0 {
            over_four += 1;
        }
    }
    eprintln!("{over_four}/{} head vertices use more than 4 influences", m.positions.len());
    assert!(over_four > 1000, "only {over_four} vertices kept >4 influences");

    let chf = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let dna = starbreaker_chf::ChfFile::from_chf(&chf).unwrap().parse().unwrap().dna;
    let glb = HeadIdentityContext::load(&p4k, dna.gender_hash).unwrap().build_glb(&dna, true, false).unwrap();
    let doc = gltf::Gltf::from_slice(&glb).unwrap();
    let head_prim = doc.meshes().next().unwrap().primitives().next().unwrap();
    assert!(head_prim.get(&gltf::Semantic::Joints(1)).is_some(), "JOINTS_1 missing");
    assert!(head_prim.get(&gltf::Semantic::Weights(1)).is_some(), "WEIGHTS_1 missing");
}

fn bshapes_of(p4k: &MappedP4k, skin_path: &str) -> Vec<Vec<[f32; 3]>> {
    use starbreaker_chunks::ChunkFile;
    let bytes = p4k.read_file(&format!("{skin_path}m")).unwrap();
    let ChunkFile::Ivo(ivo) = ChunkFile::from_bytes(&bytes).unwrap() else { panic!("not IVO") };
    let e = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::BSHAPES_GPU)
        .expect("BShapesGpu chunk");
    starbreaker_3d::character::blendshapes::parse_bshapes_gpu(ivo.chunk_data(e))
        .unwrap()
        .targets
        .into_iter()
        .map(|t| t.positions)
        .collect()
}

/// Slot 19 cell `(v, b)` is archetype `v`'s own blendshape `b`: decoded
/// entries match `BShapesGpu[b]` of `<name>_head.skinm` (variant 0 = base) to
/// within one of the cell's per-axis quantisation steps (8-bit, truncated).
/// Over all 60 male archetypes ≥ 99.86 % of entries do; the rare outliers
/// (≤ 0.14 mm) reflect drift between the `.dna` bake and the shipped meshes.
#[test]
fn slot19_cells_equal_archetype_blendshapes() {
    let Some(p4k) = p4k() else { return };
    let g = "male";
    let (_, _, dna_bytes) = load_rig(&p4k, g);
    let dna = DnaFile::read(&dna_bytes).unwrap();
    let names = dna.variant_names().unwrap();
    let morphs = dna.expression_morphs().unwrap();
    for v in [0, 1, names.len() / 2, names.len() - 1] {
        let Some(skin) = find_archetype_skin(&p4k, g, &names[v]) else { continue };
        let truth = bshapes_of(&p4k, &skin);
        let (mut entries, mut beyond_step, mut worst) = (0usize, 0usize, 0.0f32);
        for b in 1..morphs.num_blendshapes().min(truth.len()) {
            let Some(rec) = morphs.record(v, b) else { continue };
            for (vtx, d) in rec.entries() {
                let t = truth[b][vtx as usize];
                let err = (0..3).map(|k| (d[k] - t[k]).abs()).fold(0.0f32, f32::max);
                let step = rec.scale.iter().copied().fold(0.0f32, f32::max);
                entries += 1;
                if err > step + 1e-6 {
                    beyond_step += 1;
                }
                worst = worst.max(err);
            }
        }
        assert!(entries > 100_000, "{}: only {entries} entries", names[v]);
        let frac = beyond_step as f64 / entries as f64;
        assert!(frac < 0.005, "{}: {:.3}% entries beyond one step", names[v], frac * 100.0);
        assert!(worst < 2e-4, "{}: worst entry error {:.3} mm", names[v], worst * 1e3);
    }
}

/// A CHF DNA that puts every face part 100 % on archetype `head_id`.
fn pure_archetype_dna(gender_hash: starbreaker_common::NameHash, head_id: u16) -> starbreaker_chf::Dna {
    use starbreaker_chf::{DnaBlend, FacePart::*};
    let none = DnaBlend { value: 0, head_id: 0 };
    let full = DnaBlend { value: u16::MAX, head_id };
    let parts = [
        EyebrowLeft, EyebrowRight, EyeLeft, EyeRight, Nose, EarLeft, EarRight, CheekLeft,
        CheekRight, Mouth, Jaw, Crown, Neck,
    ];
    starbreaker_chf::Dna {
        raw_bytes: Vec::new(),
        gender_hash,
        variant_hash: starbreaker_common::NameHash(0),
        part_count: parts.len() as u16,
        blends_per_part: 4,
        header_unknown: 0,
        max_head_id: head_id,
        face_parts: parts.into_iter().map(|p| (p, [full, none, none, none])).collect(),
    }
}

/// All primitives' POSITION arrays of a GLB, in mesh/primitive order.
/// World-space (glTF scene) positions of every mesh primitive: each through
/// its node's global transform (at rest a skinned mesh renders there too).
fn glb_positions(glb: &[u8]) -> Vec<Vec<[f32; 3]>> {
    let (doc, buffers, _) = gltf::import_slice(glb).expect("import glb");
    let mut global = vec![glam::Mat4::IDENTITY; doc.nodes().len()];
    fn walk(n: gltf::Node, parent: glam::Mat4, out: &mut [glam::Mat4]) {
        let m = parent * glam::Mat4::from_cols_array_2d(&n.transform().matrix());
        out[n.index()] = m;
        for c in n.children() {
            walk(c, m, out);
        }
    }
    for scene in doc.scenes() {
        for n in scene.nodes() {
            walk(n, glam::Mat4::IDENTITY, &mut global);
        }
    }
    doc.nodes()
        .filter_map(|n| n.mesh().map(|m| (global[n.index()], m)))
        .flat_map(|(world, m)| m.primitives().map(move |p| (world, p)).collect::<Vec<_>>())
        .map(|(world, p)| {
            p.reader(|b| Some(&buffers[b.index()]))
                .read_positions()
                .unwrap()
                .map(|v| world.transform_point3(glam::Vec3::from(v)).to_array())
                .collect()
        })
        .collect()
}

/// End to end: a pure-archetype character assembles to that archetype's own
/// shipped meshes — head (slot 18 of the head `.dna`), and the `ProtosMisc`
/// eyes and teeth (their own `.dna` + re-pointed bind, no joint double-count).
#[test]
fn pure_archetype_character_matches_archetype_meshes() {
    use starbreaker_3d::character::{identity::MALE_T1_PU_GENDER_HASH, HeadIdentityContext};
    // CryEngine Z-up → glTF Y-up (the GLB root node applies the same).
    let cry_to_gltf = |v: [f32; 3]| [v[0], v[2], -v[1]];
    let Some(p4k) = p4k() else { return };
    let ctx = HeadIdentityContext::load(&p4k, MALE_T1_PU_GENDER_HASH).unwrap();
    let head_dna = p4k.read_file(&format!(r"{}\protos_human_male_face_t1_pu_head.dna", rig_dir("male"))).unwrap();
    let names = DnaFile::read(&head_dna).unwrap().variant_names().unwrap();
    for head_id in [1u16, 17] {
        let name = &names[head_id as usize];
        let glb = ctx.build_glb(&pure_archetype_dna(MALE_T1_PU_GENDER_HASH, head_id), true, false).unwrap();
        let prims = glb_positions(&glb);
        for part in DNA_PARTS {
            let skin = find_archetype_part(&p4k, "male", name, part).expect("archetype part");
            let truth: Vec<[f32; 3]> = read_head_positions(&p4k, &skin).into_iter().map(cry_to_gltf).collect();
            let prim = prims
                .iter()
                .find(|p| p.len() == truth.len())
                .unwrap_or_else(|| panic!("{name} {part}: no primitive with {} vertices", truth.len()));
            let worst = prim
                .iter()
                .zip(&truth)
                .flat_map(|(a, b)| (0..3).map(move |k| (a[k] - b[k]).abs()))
                .fold(0.0f32, f32::max);
            eprintln!("{name} {part}: worst {:.4} mm", worst * 1e3);
            assert!(worst < 2e-4, "{name} {part}: assembled vs archetype mesh worst {:.3} mm", worst * 1e3);
        }
    }
}
