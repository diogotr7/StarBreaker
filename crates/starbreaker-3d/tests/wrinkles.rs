//! Wrinkle-map drivers against real data: RigLogic Section G/H, the
//! `wrinklemask_tier1_512` regions and the pinned group → slot table.
//!
//! Gated on `Data.p4k` being accessible; skips gracefully when absent.

use std::path::PathBuf;

use starbreaker_3d::character::{
    apply, blendshapes::parse_bshapes_gpu, controls::ControlInputs, dna_matrix,
    head_skin::{HeadSkin, parse_head_skin},
    morphing, submesh::skin_submeshes_to_head,
    wrinkles::{NUM_WRINKLE_SLOTS, SLOT_LAYOUT, WRINKLE_GROUP_TO_SLOT},
};
use starbreaker_3d::riglogic::{RigLogic, compose_correctives};
use starbreaker_chunks::ChunkFile;
use starbreaker_dds::{DdsFile, ReadSibling};
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

fn head_path(g: &str) -> String {
    format!(r"Data\Objects\Characters\Human\heads\{g}\pu\protos_human_{g}_face_t1_pu\protos_human_{g}_face_t1_pu_head")
}

fn load(p4k: &MappedP4k, g: &str) -> (HeadSkin, RigLogic, Vec<u8>, Vec<u8>) {
    let base = head_path(g);
    let skinm = p4k.read_file(&format!("{base}.skinm")).unwrap();
    let head = parse_head_skin(&p4k.read_file(&format!("{base}.skin")).unwrap(), &skinm).unwrap();
    let rig = RigLogic::read(&head.riglogic_chunk[8..]).unwrap();
    let dna = p4k.read_file(&format!("{base}.dna")).unwrap();
    (head, rig, dna, skinm)
}

struct P4kSibling<'a> {
    p4k: &'a MappedP4k,
    base: String,
}
impl ReadSibling for P4kSibling<'_> {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        self.p4k.read_file(&format!("{}{suffix}", self.base)).ok()
    }
}

/// `wrinklemask_tier1_512` as RGBA8, 1024 × 512 (two 512² panels).
fn wrinkle_mask(p4k: &MappedP4k) -> (Vec<u8>, usize, usize) {
    let base = r"Data\Objects\Characters\Human\heads\shared\base\head\textures\WrinkleMask_Tier1_512.dds".to_string();
    let data = p4k.read_file(&base).unwrap();
    let dds = DdsFile::from_split(&data, &P4kSibling { p4k, base: base.clone() }).unwrap();
    let (w, h) = dds.dimensions(0);
    (dds.decode_rgba(0).unwrap(), w as usize, h as usize)
}

/// Per slot, per head vertex: the band ramp (0 outside the slot's region).
fn slot_regions(p4k: &MappedP4k, head: &HeadSkin) -> Vec<Vec<f32>> {
    let (mask, w, _h) = wrinkle_mask(p4k);
    let panel_w = w / 2;
    SLOT_LAYOUT
        .iter()
        .map(|s| {
            head.mesh
                .uvs
                .iter()
                .map(|uv| {
                    let x = ((uv[0] * panel_w as f32) as usize).min(panel_w - 1) + s.panel as usize * panel_w;
                    let y = ((uv[1] * panel_w as f32) as usize).min(panel_w - 1);
                    let c = mask[(y * w + x) * 4 + s.channel as usize] as u32;
                    if (c / 64).min(3) == s.band as u32 && c % 64 != 0 { (c % 64) as f32 / 63.0 } else { 0.0 }
                })
                .collect()
        })
        .collect()
}

/// Section G (per-group `u16 count` + `u16` inputs) and H (`f32` weights) of a
/// RigLogic file, read straight from its section table (`0x68`, slots 6/7) —
/// the standalone wrinkle rig carries only these two sections.
fn raw_wrinkle_tables(rig: &[u8], groups: usize) -> (Vec<Vec<u16>>, Vec<f32>) {
    let u64_at = |o: usize| u64::from_le_bytes(rig[o..o + 8].try_into().unwrap()) as usize;
    let u16_at = |o: usize| u16::from_le_bytes(rig[o..o + 2].try_into().unwrap());
    let (g, h) = (u64_at(0x68 + 6 * 8), u64_at(0x68 + 7 * 8));
    let mut o = g;
    let lists: Vec<Vec<u16>> = (0..groups)
        .map(|_| {
            let n = u16_at(o) as usize;
            let l = (0..n).map(|i| u16_at(o + 2 + 2 * i)).collect();
            o += 2 + 2 * n;
            l
        })
        .collect();
    let total: usize = lists.iter().map(Vec::len).sum();
    let weights = (0..total).map(|i| f32::from_le_bytes(rig[h + 4 * i..h + 4 * i + 4].try_into().unwrap())).collect();
    (lists, weights)
}

/// The shared tier-1 wrinkle rig is exactly the head rigs' Section G/H.
#[test]
fn standalone_wrinkle_rig_matches_head_rig_tables() {
    let Some(p4k) = p4k() else { return };
    let standalone = p4k
        .read_file(r"Data\Objects\Characters\RigLogic\shared_wrinkle_mask_human_face_t1.RigLogic")
        .unwrap();
    let (lists, weights) = raw_wrinkle_tables(&standalone, NUM_WRINKLE_SLOTS);
    for g in ["male", "female"] {
        let (_, rig, _, _) = load(&p4k, g);
        let a: Vec<_> = rig.blend_region_bones.iter().map(|r| r.weight_indices.clone()).collect();
        assert_eq!(a, lists, "{g}: Section G differs from the shared wrinkle rig");
        assert_eq!(rig.region_weights, weights, "{g}: Section H differs");
    }
}

#[test]
fn left_groups_drive_left_side_regions() {
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        let (head, _, _, _) = load(&p4k, g);
        let regions = slot_regions(&p4k, &head);
        for (group, &slot) in WRINKLE_GROUP_TO_SLOT.iter().enumerate() {
            let (sum, x): (f32, f32) = regions[slot]
                .iter()
                .zip(&head.mesh.positions)
                .fold((0.0, 0.0), |(s, x), (w, p)| (s + w, x + w * p[0]));
            let cx = x / sum;
            // Character's left is −X on this rig.
            let left = group % 2 == 0;
            assert!(if left { cx < 0.0 } else { cx > 0.0 }, "{g} group {group} → slot {slot}: centroid x {cx}");
        }
    }
}

/// Per group: |displacement| per head vertex when the group's positive-weight
/// appearance controls are driven to 1 on the base character (rig joints with
/// CPU skinning + the head's `BShapesGpu`), relative to rest.
fn group_footprints(head: &HeadSkin, rig: &RigLogic, dna_bytes: &[u8], skinm: &[u8]) -> Vec<Vec<f32>> {
    let dna = DnaFile::read(dna_bytes).unwrap();
    let ChunkFile::Ivo(ivo) = ChunkFile::from_bytes(skinm).unwrap() else { panic!("not IVO") };
    let e = ivo.chunks().iter().find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::BSHAPES_GPU).unwrap();
    let bshapes = parse_bshapes_gpu(ivo.chunk_data(e)).unwrap().targets;
    let (nv, bpp) = (dna.variant_records.len(), dna.header.mesh_blends_per_pool as usize);
    let mut w = vec![0.0f64; nv * bpp];
    w[..bpp].fill(1.0); // base character: every face part on variant 0
    let matrix = dna_matrix::build_dna_matrix(&dna_matrix::prune_top_k(&w, nv, bpp).unwrap(), nv, bpp).unwrap();
    let nj = rig.num_joints as usize;
    let jw = dna.joint_weights(nj).unwrap();
    let eval = |set: &[usize]| -> Vec<[f32; 3]> {
        let mut input = ControlInputs::rest_pose(rig);
        for &i in set {
            input.set_idx(i, 1.0).unwrap();
        }
        let act = compose_correctives(rig, &input.into_appearance_weights()).unwrap();
        let d = morphing::compute_joint_deltas(&matrix, &jw, &dna, &act, nj, act.len(), 9).unwrap();
        let posed = apply::apply_deltas(rig, &d, &head.skeleton).unwrap();
        let mut p = skin_submeshes_to_head(&[(head.mesh.clone(), head.skeleton.clone())], &posed).remove(0).positions;
        for (b, t) in bshapes.iter().enumerate() {
            let a = act.get(b).copied().unwrap_or(0.0);
            if a != 0.0 {
                for (v, dv) in t.positions.iter().enumerate() {
                    for k in 0..3 {
                        p[v][k] += a * dv[k];
                    }
                }
            }
        }
        p
    };
    let rest = eval(&[]);
    let na = rig.num_appearance_blendshapes as usize;
    let mut weights = rig.region_weights.iter();
    rig.blend_region_bones
        .iter()
        .map(|group| {
            let set: Vec<usize> = group
                .weight_indices
                .iter()
                .map(|&i| (i as usize, *weights.next().unwrap()))
                .filter(|&(i, w)| i < na && w > 0.0)
                .map(|(i, _)| i)
                .collect();
            let posed = eval(&set);
            let mags: Vec<f32> = posed
                .iter()
                .zip(&rest)
                .map(|(p, r)| (0..3).map(|k| (p[k] - r[k]).powi(2)).sum::<f32>().sqrt())
                .collect();
            let max = mags.iter().copied().fold(0.0f32, f32::max).max(1e-12);
            mags.into_iter().map(|m| m / max).collect()
        })
        .collect()
}

/// The table is the one-to-one pair → region assignment the footprints
/// imply: pairs claim regions in order of confidence (best/runner-up score),
/// each taking its best region not yet claimed. On both genders this yields
/// exactly [`WRINKLE_GROUP_TO_SLOT`]'s regions (brow-lateral owns region 8
/// outright, leaving brow-down its runner-up 9; the diffuse smile footprint
/// gets the last free region, 10).
#[test]
fn footprint_assignment_reproduces_the_table() {
    let Some(p4k) = p4k() else { return };
    for g in ["male", "female"] {
        let (head, rig, dna, skinm) = load(&p4k, g);
        let regions = slot_regions(&p4k, &head);
        let foot = group_footprints(&head, &rig, &dna, &skinm);
        let score = |group: usize, slot: usize| -> f32 {
            let (num, den) = regions[slot]
                .iter()
                .zip(&foot[group])
                .fold((0.0, 0.0), |(n, d), (r, f)| (n + r * f, d + r));
            num / den.max(1e-9)
        };
        // Region r = slots {r, r + 12}; a pair scores its L group on the
        // left-side slot and its R group on the other.
        let pair_region = |pair: usize, r: usize| -> f32 {
            let left = WRINKLE_GROUP_TO_SLOT[2 * pair];
            let (l_slot, r_slot) = if SLOT_LAYOUT[r].panel == SLOT_LAYOUT[left].panel { (r, r + 12) } else { (r + 12, r) };
            score(2 * pair, l_slot) + score(2 * pair + 1, r_slot)
        };
        let scores: Vec<Vec<f32>> = (0..12).map(|p| (0..12).map(|r| pair_region(p, r)).collect()).collect();
        let confidence = |p: usize| {
            let mut s = scores[p].clone();
            s.sort_by(|a, b| b.total_cmp(a));
            s[0] / s[1]
        };
        let mut order: Vec<usize> = (0..12).collect();
        order.sort_by(|&a, &b| confidence(b).total_cmp(&confidence(a)));
        let mut claimed = [false; 12];
        let mut assigned = [usize::MAX; 12];
        for p in order {
            let r = (0..12).filter(|&r| !claimed[r]).max_by(|&a, &b| scores[p][a].total_cmp(&scores[p][b])).unwrap();
            claimed[r] = true;
            assigned[p] = r;
        }
        let table: Vec<usize> = (0..12).map(|p| WRINKLE_GROUP_TO_SLOT[2 * p] % 12).collect();
        assert_eq!(assigned.to_vec(), table, "{g}: footprint assignment differs from the table");
    }
}

/// The expression-animation GLB carries the per-frame `PI_WrinklesMask`
/// weights as a material-parameter track of its animation
/// (`extras.starbreaker_material_params`): one 24-vector per animation frame,
/// in [0, 1], moving with the clip, plus the slot layout.
#[test]
fn expression_glb_carries_per_frame_wrinkle_weights() {
    use starbreaker_3d::character::{HeadIdentityContext, identity::MALE_T1_PU_GENDER_HASH};
    let Some(p4k) = p4k() else { return };
    let chf = p4k.read_file(r"Data\Libs\CharacterCustomizer\masculinedefault.chf").unwrap();
    let dna = starbreaker_chf::ChfFile::from_chf(&chf).unwrap().parse().unwrap().dna;
    let caf = p4k
        .read_file(r"Data\Animations\Characters\Human\heads\male\ai\idlesets\happy\face\ai_civ_happy_idle_05_face.caf")
        .unwrap();
    let ctx = HeadIdentityContext::load(&p4k, MALE_T1_PU_GENDER_HASH).unwrap();
    let glb = ctx.build_expression_glb(&dna, &caf, 1.0).unwrap();
    let doc = gltf::Gltf::from_slice(&glb).unwrap();
    let animation = doc.animations().next().expect("animation");
    let extras: serde_json::Value = serde_json::from_str(animation.extras().as_ref().expect("extras").get()).unwrap();
    let w = extras["starbreaker_material_params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["param"] == "PI_WrinklesMask")
        .expect("wrinkle track");
    let times = w["times"].as_array().unwrap();
    let frames = w["values"].as_array().unwrap();
    assert_eq!(times.len(), frames.len());
    assert_eq!(w["info"]["slots"].as_array().unwrap().len(), NUM_WRINKLE_SLOTS);
    let anim_times = animation.channels().next().unwrap().sampler().input().count();
    assert_eq!(times.len(), anim_times, "wrinkle frames must line up with the animation");
    let mut peak = 0.0f64;
    for f in frames {
        let v = f.as_array().unwrap();
        assert_eq!(v.len(), NUM_WRINKLE_SLOTS);
        for x in v {
            let x = x.as_f64().unwrap();
            assert!((0.0..=1.0).contains(&x));
            peak = peak.max(x);
        }
    }
    eprintln!("happy idle: peak wrinkle weight {peak:.3}");
    assert!(peak > 0.05, "a smile clip should drive some wrinkles (peak {peak})");
}
