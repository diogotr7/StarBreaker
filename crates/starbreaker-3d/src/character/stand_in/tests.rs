//! Unit tests for [`super`]: the line rule, ranking, mount bones and the
//! stand-in placement, on synthetic items and skeletons.

use super::*;
use crate::character::head_skin::Bone;
use crate::character::props::PortHelper;
use crate::character::wear::PortType;

fn candidate(core: &str, skin: &str, max_size: i32) -> Candidate {
    Candidate { core: core.into(), skin: skin.into(), max_size: Some(max_size), bone: "mount".into(), mount: None }
}

/// Many skins share one mount (where the bone sits): the mount most cores
/// of the group share wins over the skin most of them share.
#[test]
fn the_common_mount_beats_the_common_skin() {
    let at = |c: Candidate, mount: [i32; 7]| Candidate { mount: Some(mount), ..c };
    let (usual, other) = ([80, -154, 0, 0, 707, 0, 707], [104, -155, 0, 0, 700, 50, 700]);
    let candidates = [
        at(candidate("eee_light_core_01_01_01", "shared.skin", 1), other),
        at(candidate("fff_light_core_01_01_01", "shared.skin", 1), other),
        at(candidate("ccc_light_core_01_01_01", "c.skin", 1), usual),
        at(candidate("bbb_light_core_01_01_01", "b.skin", 1), usual),
        at(candidate("ddd_light_core_01_01_01", "d.skin", 1), usual),
    ];
    let ranked: Vec<&str> = rank(&candidates, "zzz_line_light_backpack_01_01_01", Some(1)).iter().map(|c| c.core.as_str()).collect();
    assert_eq!(ranked, ["bbb_light_core_01_01_01", "ccc_light_core_01_01_01", "ddd_light_core_01_01_01", "eee_light_core_01_01_01", "fff_light_core_01_01_01"]);
}

#[test]
fn a_line_is_the_name_before_the_piece_word() {
    assert_eq!(line("kap_combat_heavy_backpack_01_01_15").as_deref(), Some("kap_combat_heavy"));
    assert_eq!(line("kap_combat_heavy_core_01_01_01").as_deref(), Some("kap_combat_heavy"));
    assert_eq!(line("outlaw_legacy_medium_armor_core_01_01_01").as_deref(), Some("outlaw_legacy_medium_armor"));
    assert_eq!(line("backpack_01"), None, "a piece word alone is no line");
}

/// A core of the prop's own line wins over any of its size; among them the
/// `.skin` most of them share, then the name.
#[test]
fn own_line_cores_rank_first_by_their_common_skin() {
    let candidates = [
        candidate("aaa_armor_heavy_core_01_01_01", "a.skin", 3),
        candidate("syn_line_heavy_core_02_01_01", "rare.skin", 3),
        candidate("syn_line_heavy_core_01_01_02", "usual.skin", 3),
        candidate("syn_line_heavy_core_01_01_01", "usual.skin", 3),
    ];
    let ranked: Vec<&str> = rank(&candidates, "syn_line_heavy_backpack_01_01_01", Some(3)).iter().map(|c| c.core.as_str()).collect();
    assert_eq!(ranked, ["syn_line_heavy_core_01_01_01", "syn_line_heavy_core_01_01_02", "syn_line_heavy_core_02_01_01", "aaa_armor_heavy_core_01_01_01"]);
}

/// No core of the prop's line: one whose port is the prop's size, the skin
/// most of those share first; larger ports after; smaller ports never.
#[test]
fn without_a_line_the_size_and_its_common_skin_decide() {
    let candidates = [
        candidate("zzz_light_core_01_01_01", "light.skin", 1),
        candidate("bbb_medium_core_01_01_01", "rare.skin", 2),
        candidate("ccc_medium_core_01_01_01", "usual.skin", 2),
        candidate("ddd_medium_core_01_01_01", "usual.skin", 2),
        candidate("aaa_heavy_core_01_01_01", "heavy.skin", 3),
    ];
    let ranked: Vec<&str> = rank(&candidates, "other_line_medium_backpack_01_01_01", Some(2)).iter().map(|c| c.core.as_str()).collect();
    assert_eq!(ranked, ["ccc_medium_core_01_01_01", "ddd_medium_core_01_01_01", "bbb_medium_core_01_01_01", "aaa_heavy_core_01_01_01"]);
}

fn bone(name: &str, parent: Option<u32>, at: [f32; 3]) -> Bone {
    Bone {
        name: name.into(),
        name_crc: 0,
        name_crc_lower: 0,
        parent,
        bind_translation: at,
        bind_rotation: [0.0, 0.0, 0.0, 1.0],
        world_translation: at,
        world_rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// The mount bone keeps its ancestors (by which it hangs on the body) and
/// its own bind; other bones of the core's skin are dropped.
#[test]
fn mount_skeletons_keep_the_bone_and_its_ancestors() {
    let skin = BindPoseSkeleton {
        bones: vec![
            bone("Hips", None, [0.0, 0.0, 1.0]),
            bone("cloth_flap", Some(0), [0.0, 0.1, 0.0]),
            bone("Spine3", Some(0), [0.0, 0.0, 0.4]),
            bone("Mount", Some(2), [0.1, -0.2, 0.0]),
        ],
    };
    let mount = mount_skeleton(&skin, &["mount".into()]).unwrap();
    let names: Vec<&str> = mount.bones.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Hips", "Spine3", "Mount"]);
    assert_eq!(mount.bones[1].parent, Some(0));
    assert_eq!(mount.bones[2].parent, Some(1));
    assert_eq!(mount.bones[2].bind_translation, [0.1, -0.2, 0.0]);
    assert!(mount_skeleton(&skin, &["absent".into()]).is_none());
}

fn port(name: &str, ty: &str) -> PortDef {
    PortDef { name: name.into(), types: vec![PortType { name: ty.into(), sub_types: Vec::new() }], ..Default::default() }
}
fn item(name: &str, ty: &str, ports: Vec<PortDef>) -> Wearable {
    Wearable { item: name.into(), item_type: ty.into(), ports, ..Default::default() }
}
fn backpack() -> Wearable {
    Wearable { rigid: true, ..item("pack", "Char_Armor_Backpack", vec![]) }
}
fn core(name: &str) -> Wearable {
    let bone = PortHelper { bone: "mount".into(), ..Default::default() };
    item(name, "Char_Armor_Torso", vec![PortDef { bone: Some(bone), ..port("backpack", "Char_Armor_Backpack") }])
}
/// The body in an undersuit with a free torso port.
fn suited() -> Seed {
    let suit = Seed { port: "Armor_Undersuit".into(), item: "suit".into(), ports: vec![port("Armor_Torso", "Char_Armor_Torso")], children: Vec::new() };
    Seed { port: "body".into(), item: String::new(), ports: vec![port("Armor_Undersuit", "Char_Armor_Undersuit")], children: vec![suit] }
}

/// A backpack with no core goes on the stand-in the callback offers, which
/// is flagged in the tree and listed; the callback sees the free ports.
#[test]
fn unplaced_props_go_on_a_stand_in_core() {
    let mut asked = Vec::new();
    let p = place_with_stand_ins(&suited(), &[backpack()], |free, item| {
        asked.push((free.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), item.item.clone()));
        Some(core("stand"))
    });
    assert_eq!(asked, [(vec!["Armor_Torso".to_string()], "pack".to_string())]);
    assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
    let stand = &p.entries[0].children[0].children[0];
    assert_eq!((stand.port.as_str(), stand.item.as_str(), stand.stand_in), ("Armor_Torso", "stand", true));
    assert_eq!((stand.children[0].port.as_str(), stand.children[0].item.as_str()), ("backpack", "pack"));
    assert!(!stand.children[0].stand_in);
    assert_eq!(p.stand_ins, [StandIn { item: "pack".into(), core: "stand".into() }]);
}

/// A real core worn: the backpack goes on it and no stand-in is asked for.
#[test]
fn a_worn_core_needs_no_stand_in() {
    let p = place_with_stand_ins(&suited(), &[core("real"), backpack()], |_, _| panic!("no stand-in needed"));
    assert!(p.unplaced.is_empty() && p.stand_ins.is_empty(), "{p:?}");
    assert!(!p.entries[0].children[0].children[0].stand_in);
}

/// A stand-in that cannot be worn (no free port takes it), or none offered:
/// the backpack stays unplaced and the tree is as without it.
#[test]
fn a_stand_in_nothing_takes_is_not_worn() {
    let bare = Seed { children: Vec::new(), ..suited() };
    let p = place_with_stand_ins(&bare, &[backpack()], |_, _| Some(core("stand")));
    assert_eq!(p.unplaced.len(), 1);
    assert!(p.stand_ins.is_empty() && p.entries[0].children.is_empty(), "{p:?}");
    let p = place_with_stand_ins(&suited(), &[backpack()], |_, _| None);
    assert_eq!(p.unplaced.len(), 1);
    assert!(p.entries[0].children[0].children.is_empty());
}
