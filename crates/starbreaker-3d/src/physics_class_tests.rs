//! Tests for `physics_class`: the node-name collision classes, as surveyed in ship geometry.
use super::collision_class;

#[test]
fn interaction_points_are_interaction_volumes() {
    for name in ["$IP_SeatAccess_Pilot", "$IP_exit_left", "$ip_open_close", "$IP_proxy"] {
        assert_eq!(collision_class(name), Some("interaction"), "{name}");
    }
}

#[test]
fn surface_filters_keep_their_classes() {
    assert_eq!(collision_class("TIRES_ONLY_COLLISIONS"), Some("tires_only"));
    assert_eq!(collision_class("tires_only_collision"), Some("tires_only"));
    assert_eq!(collision_class("PLAYER_ONLY_COLLISIONS"), Some("player_only"));
    assert_eq!(collision_class("IGNORE_TERRAIN_COLLIDE"), Some("no_terrain"));
    assert_eq!(collision_class("IGNORE_TERRAIN_COLLISION"), Some("no_terrain"));
    assert_eq!(collision_class("NO_TERRAIN_COLLISION"), Some("no_terrain"));
}

#[test]
fn grid_extensions_are_grid_volumes() {
    for name in ["physicsgrid_allow_no_collide_cargobay", "PHYSICSGRID_allow_no_collide_Rear", "LOCALGRID_allow_no_collide_001",
                 "localgrid_allow_no_collide_left"] {
        assert_eq!(collision_class(name), Some("grid_volume"), "{name}");
    }
}

#[test]
fn ordinary_colliders_and_proxies_have_no_class() {
    for name in ["Body", "$proxy_Bed_Bunk_01", "$physics_ramp_01", "Sidedoor_Right", "Interior_Collision", "IP_Exterior",
                 "player_collision_r", "$shield_Wing_Left"] {
        assert_eq!(collision_class(name), None, "{name}");
    }
}
