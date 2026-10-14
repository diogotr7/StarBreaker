//! `clothing` rules on synthetic items: layers, `VisibleLayers` and
//! `VisibilityConditions` as the engine evaluates them.

use super::*;

fn tag(n: u8) -> CigGuid {
    CigGuid::from_bytes([n; 16])
}

fn chunk(zone: &str, layer: i32) -> ClothingChunk {
    ClothingChunk { zone: zone.into(), layer, visible_layers: Vec::new(), conditions: Vec::new() }
}

fn item(layer: i32, zones: &[&str]) -> Clothing {
    Clothing { chunks: zones.iter().map(|z| chunk(z, layer)).collect(), ..Clothing::default() }
}

fn on<'a>(port: &'a str, clothing: &'a Clothing) -> Worn<'a> {
    Worn { port, clothing }
}

fn set(zones: &[&str]) -> HashSet<u32> {
    zones.iter().map(|z| zone_hash(z)).collect()
}

/// Hashes as `m_body.skinm` / the undersuit store them.
#[test]
fn zone_hashes_match_the_meshes() {
    assert_eq!(zone_hash("l_hand_zone"), 3335329176);
    assert_eq!(zone_hash("r_hand_zone"), 998859158);
    assert_eq!(zone_hash("torso01_zone"), zone_hash("TORSO01_ZONE"), "case-insensitive");
}

/// An undersuit (3) covers the body (0) where both claim a zone; armour
/// (4) on the hands covers the undersuit's gloves and the body's hands.
#[test]
fn higher_layers_hide_lower_ones_per_zone() {
    let body = item(0, &["torso01_zone", "l_hand_zone", "head_zone"]);
    let suit = item(3, &["torso01_zone", "l_hand_zone"]);
    let arms = item(4, &["l_hand_zone", "omega_arms_zone"]);
    let hidden = hidden_zones(&[on("body", &body), on("suit", &suit), on("arms", &arms)]);
    assert_eq!(hidden[0], set(&["torso01_zone", "l_hand_zone"]), "the head zone stays");
    assert_eq!(hidden[1], set(&["l_hand_zone"]));
    assert!(hidden[2].is_empty(), "nothing covers the top layer");
}

/// `VisibleLayers` on the upper chunk names the lower layers that stay
/// (a short-sleeved shirt at 1 lists 0: the body's arm shows); layers are
/// walked from the top, so what the kept layer lists counts too, and the
/// same layer never hides itself.
#[test]
fn visible_layers_keep_lower_layers() {
    let body = item(0, &["l_arm04_zone"]);
    let mut shirt = item(1, &["l_arm04_zone"]);
    shirt.chunks[0].visible_layers = vec![0];
    let hidden = hidden_zones(&[on("body", &body), on("shirt", &shirt)]);
    assert!(hidden[0].is_empty() && hidden[1].is_empty(), "{hidden:?}");

    let mut vest = item(2, &["l_arm04_zone"]);
    vest.chunks[0].visible_layers = vec![1];
    let jacket = item(2, &["l_arm04_zone"]);
    let hidden = hidden_zones(&[on("body", &body), on("shirt", &shirt), on("vest", &vest), on("jacket", &jacket)]);
    assert!(hidden[1].is_empty() && hidden[0].is_empty(), "the vest keeps the shirt, the shirt the body: {hidden:?}");
    assert!(hidden[2].is_empty() && hidden[3].is_empty(), "same layer");

    let hidden = hidden_zones(&[on("body", &body), on("shirt", &shirt), on("jacket", &jacket)]);
    assert_eq!((hidden[0].len(), hidden[1].len()), (1, 1), "nothing at 2 keeps 1 or 0");
}

fn conditional(port: &str, active: Vec<CigGuid>) -> ClothingChunk {
    let mut c = chunk("theta_core_zone", 2);
    c.conditions = vec![VisibilityCondition {
        port_visibility: vec![PortVisibility { ports: vec![port.into()], active, ..Default::default() }],
    }];
    c
}

fn suit_with(c: ClothingChunk) -> Clothing {
    Clothing { chunks: vec![c], ports: vec!["Armor_Helmet".into(), "backpack".into()], ..Clothing::default() }
}

/// A flightsuit collar shows only under a helmet carrying all of its
/// `ActiveTags`.
#[test]
fn active_tags_on_a_port_must_all_be_worn() {
    let suit = suit_with(conditional("Armor_Helmet", vec![tag(1), tag(2)]));
    let fits = Clothing { tags: vec![tag(1), tag(2), tag(9)], ..Clothing::default() };
    let other = Clothing { tags: vec![tag(1)], ..Clothing::default() };
    assert!(chunk_visible(&suit.chunks[0], &[on("suit", &suit), on("Armor_Helmet", &fits)]));
    assert!(!chunk_visible(&suit.chunks[0], &[on("suit", &suit), on("Armor_Helmet", &other)]));
    assert!(!chunk_visible(&suit.chunks[0], &[on("suit", &suit)]), "no helmet");
}

/// With no `ActiveTags` (and no `SkipTags`) the condition asks for an
/// empty port: it holds while nothing is worn there, `DisableTags` only
/// test a worn item (an armour core's backpack mount shows until a tagged
/// backpack is worn), and a port nothing defines holds nothing.
#[test]
fn an_empty_active_list_asks_for_an_empty_port() {
    let suit = suit_with(conditional("backpack", Vec::new()));
    let pack = Clothing { tags: vec![tag(3)], ..Clothing::default() };
    assert!(chunk_visible(&suit.chunks[0], &[on("suit", &suit)]));
    assert!(!chunk_visible(&suit.chunks[0], &[on("suit", &suit), on("backpack", &pack)]));
    let mut mount = conditional("backpack", Vec::new());
    mount.conditions[0].port_visibility[0].disable = vec![tag(3)];
    let mount = suit_with(mount);
    assert!(chunk_visible(&mount.chunks[0], &[on("suit", &mount)]), "no backpack: nothing disables");
    assert!(!chunk_visible(&mount.chunks[0], &[on("suit", &mount), on("backpack", &pack)]), "a tagged backpack disables");
    let nowhere = suit_with(conditional("Clothing_Torso_1", Vec::new()));
    assert!(!chunk_visible(&nowhere.chunks[0], &[on("suit", &nowhere)]), "the port does not exist");
}

/// `DisableTags` hide outright; `SkipTags` fail the condition (a later one
/// may still hold); an empty condition hides; no ports test everything worn.
#[test]
fn disable_skip_and_portless_entries() {
    let pack = Clothing { tags: vec![tag(3), tag(4)], ..Clothing::default() };
    let entry = |disable: Vec<CigGuid>, skip: Vec<CigGuid>, active: Vec<CigGuid>| PortVisibility {
        ports: vec!["backpack".into()],
        disable,
        skip,
        active,
    };
    let with = |conditions: Vec<Vec<PortVisibility>>| {
        let mut c = chunk("theta_core_zone", 3);
        c.conditions = conditions.into_iter().map(|port_visibility| VisibilityCondition { port_visibility }).collect();
        suit_with(c)
    };
    let worn = |suit: &Clothing| chunk_visible(&suit.chunks[0], &[on("suit", suit), on("backpack", &pack)]);

    let disable_first = vec![vec![entry(vec![tag(3)], vec![], vec![tag(4)])], vec![entry(vec![], vec![], vec![tag(4)])]];
    assert!(!worn(&with(disable_first)), "disable wins");
    assert!(!worn(&with(vec![vec![entry(vec![], vec![tag(3)], vec![tag(4)])]])), "skipped");
    let skip_then_hold = vec![vec![entry(vec![], vec![tag(3)], vec![tag(4)])], vec![entry(vec![], vec![], vec![tag(3)])]];
    assert!(worn(&with(skip_then_hold)), "the next condition holds");
    assert!(!worn(&with(vec![vec![]])), "an empty condition");
    let portless = PortVisibility { active: vec![tag(4)], ..Default::default() };
    assert!(worn(&with(vec![vec![portless]])), "any worn item's tags");
}

/// A chunk its conditions hide is not drawn, keeps nothing below it, and
/// still covers the layers under it.
#[test]
fn a_hidden_chunk_still_covers_lower_layers() {
    let body = item(0, &["theta_core_zone"]);
    let mut collar = conditional("Armor_Helmet", vec![tag(1)]);
    collar.visible_layers = vec![0];
    let suit = suit_with(collar);
    let hidden = hidden_zones(&[on("body", &body), on("suit", &suit)]);
    assert_eq!(hidden[1], set(&["theta_core_zone"]), "no helmet: the collar is hidden");
    assert_eq!(hidden[0], set(&["theta_core_zone"]), "and lists nothing to keep");
}
