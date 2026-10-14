//! Wrinkle-map weights for HumanSkin_V2 (`%WRINKLE_BLENDING_2`).
//!
//! Drivers: RigLogic Section G/H (`rig.blend_region_bones` index lists +
//! `rig.region_weights`; also shipped standalone as
//! `Objects/Characters/RigLogic/shared_wrinkle_mask_human_face_t1.RigLogic`)
//! are 24 groups — L/R pairs of brow, eye, nose, mouth, chin and neck
//! expressions — each a weighted sum of entries of the activation vector
//! (appearance controls `0..205`, then correctives `PSD#n` at `205 + n`).
//!
//! Shader side (decoded from `HumanSkinWrinkleBlend_V2.cfib` in
//! `Engine/ShaderCache_D3D11.pak`): `PI_WrinklesMask` is `float4[6]` = 24
//! weights. `wrinklemask_tier1_512` holds two 512² RGBA panels (sampled at
//! `u/2 + panel/2`); each channel value `c` selects one of four 64-level
//! bands (`band = c·255 / 64`, ramp `sat((c·255 − 64·band)/63)`), and every
//! used (panel, channel, band) adds `ramp · weight` to accumulator `WM_0`
//! (blends wrinkle-atlas tile 1, the "stretch" set: forehead lines, neck
//! cords, lip pucker) or `WM_1` (tile 2, the "compress" set: frown lines,
//! crow's feet, nose lines, chin dimpling). [`SLOT_LAYOUT`] is that table in
//! `PI_WrinklesMask` flat order.
//!
//! The group → slot map is NOT the identity and is not stored in the shader,
//! the material, the `.dna` or the RigLogic files. [`WRINKLE_GROUP_TO_SLOT`]
//! is derived from data: L groups land on the panel whose region lies on the
//! character's left (−X); the region of each L/R pair is the optimal
//! assignment of the pairs' rig-evaluated displacement footprints to the mask
//! regions (unambiguous for 8 of 12 pairs on both genders); the remaining
//! pairs are fixed by which atlas tile each band blends — brow-down's frown
//! lines and the chin's dimpling exist only in tile 2, brow-raise's forehead
//! lines and the lips' pucker lines only in tile 1. See
//! `docs/riglogic/SCDR_ANALYSIS.md`.

use crate::riglogic::RigLogic;

/// Number of `PI_WrinklesMask` weights for `%WRINKLE_BLENDING_2`.
pub const NUM_WRINKLE_SLOTS: usize = 24;

/// `PI_WrinklesMask` flat slot driven by each RigLogic Section G group.
pub const WRINKLE_GROUP_TO_SLOT: [usize; NUM_WRINKLE_SLOTS] = [
    21, 9, // brow_down            L, R
    20, 8, // brow_lateral
    3, 15, // brow_raiseIn
    6, 18, // brow_raiseOut
    19, 7, // eye squeeze + squintInner
    14, 2, // eye cheekRaise
    17, 5, // noseWrinkle
    10, 22, // mouth cornerPull + mouthLeft/Right
    13, 1, // jaw chinCompress + chinRaise
    12, 0, // mouth cornerPull purse
    16, 4, // mouth cornerDepress purse
    23, 11, // neck_stretch
];

/// Which wrinkle-atlas tile a slot's band blends (`WM_0` → 1, `WM_1` → 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrinkleSlot {
    /// Mask panel (0 = left half of the mask, 1 = right half).
    pub panel: u8,
    /// Mask channel: 0 = R, 1 = G, 2 = B, 3 = A.
    pub channel: u8,
    /// 64-level band within the channel (0..=3).
    pub band: u8,
    /// Wrinkle-atlas tile blended (1 or 2; tile 0 is the neutral face).
    pub tile: u8,
}

const fn s(panel: u8, channel: u8, band: u8, tile: u8) -> WrinkleSlot {
    WrinkleSlot { panel, channel, band, tile }
}

/// `%WRINKLE_BLENDING_2` slot table in `PI_WrinklesMask` flat order
/// (`WRINKLEMASK_{panel}_WEIGHTS_{channel}` / `_CHANNELMAP_`).
pub const SLOT_LAYOUT: [WrinkleSlot; NUM_WRINKLE_SLOTS] = {
    // Per channel: (band, tile) of the used bands, identical on both panels.
    const R: [(u8, u8); 4] = [(0, 1), (1, 2), (2, 1), (3, 1)];
    const G: [(u8, u8); 3] = [(0, 1), (1, 1), (2, 1)];
    const B: [(u8, u8); 2] = [(0, 2), (1, 2)];
    const A: [(u8, u8); 3] = [(0, 2), (1, 2), (2, 1)];
    let mut out = [s(0, 0, 0, 0); NUM_WRINKLE_SLOTS];
    let mut i = 0;
    let mut panel = 0;
    while panel < 2 {
        let mut k = 0;
        while k < 4 { out[i] = s(panel, 0, R[k].0, R[k].1); i += 1; k += 1; }
        k = 0;
        while k < 3 { out[i] = s(panel, 1, G[k].0, G[k].1); i += 1; k += 1; }
        k = 0;
        while k < 2 { out[i] = s(panel, 2, B[k].0, B[k].1); i += 1; k += 1; }
        k = 0;
        while k < 3 { out[i] = s(panel, 3, A[k].0, A[k].1); i += 1; k += 1; }
        panel += 1;
    }
    out
};

/// `PI_WrinklesMask` weights (flat slot order) for one activation vector
/// (appearance + correctives, as from [`crate::riglogic::compose_correctives`]).
/// Each group's output is its weighted sum of activations, clamped to
/// `[0, 1]`: raw sums dip below 0 when a group's `−1` corrective row outweighs
/// it (−0.12 in `ai_civ_sad_idle_07_face`) and reach 1.5 in
/// `wrinkle_rom_t1_face`; a negative weight would invert the wrinkle relief.
/// Returns zeros if the rig carries no 24-group wrinkle table.
pub fn wrinkle_slot_weights(rig: &RigLogic, activations: &[f32]) -> [f32; NUM_WRINKLE_SLOTS] {
    let mut out = [0.0f32; NUM_WRINKLE_SLOTS];
    if rig.blend_region_bones.len() != NUM_WRINKLE_SLOTS {
        return out;
    }
    let mut weights = rig.region_weights.iter();
    for (g, group) in rig.blend_region_bones.iter().enumerate() {
        let mut sum = 0.0f32;
        for &input in &group.weight_indices {
            let w = weights.next().copied().unwrap_or(0.0);
            sum += w * activations.get(input as usize).copied().unwrap_or(0.0);
        }
        out[WRINKLE_GROUP_TO_SLOT[g]] = sum.clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_to_slot_is_a_permutation() {
        let mut seen = [false; NUM_WRINKLE_SLOTS];
        for &s in &WRINKLE_GROUP_TO_SLOT {
            assert!(!seen[s], "slot {s} used twice");
            seen[s] = true;
        }
    }

    #[test]
    fn left_right_pairs_use_mirrored_panels() {
        for pair in WRINKLE_GROUP_TO_SLOT.chunks(2) {
            let (l, r) = (SLOT_LAYOUT[pair[0]], SLOT_LAYOUT[pair[1]]);
            assert_ne!(l.panel, r.panel, "{pair:?}");
            assert_eq!((l.channel, l.band, l.tile), (r.channel, r.band, r.tile), "{pair:?}");
        }
    }

    #[test]
    fn slot_layout_matches_shader_band_usage() {
        // 12 slots per panel: R 4 bands, G 3, B 2, A 3.
        for panel in 0..2u8 {
            let per: Vec<usize> = (0..4u8)
                .map(|c| SLOT_LAYOUT.iter().filter(|s| s.panel == panel && s.channel == c).count())
                .collect();
            assert_eq!(per, [4, 3, 2, 3]);
        }
        // Tile-2 ("compress") bands: R1, B0, B1, A0, A1.
        let t2: Vec<(u8, u8)> = SLOT_LAYOUT[..12].iter().filter(|s| s.tile == 2).map(|s| (s.channel, s.band)).collect();
        assert_eq!(t2, [(0, 1), (2, 0), (2, 1), (3, 0), (3, 1)]);
    }

    #[test]
    fn frown_and_chin_use_the_compress_tile() {
        // Pair 0 (brow_down) and pair 8 (chinCompress/chinRaise): tile 2.
        assert_eq!(SLOT_LAYOUT[WRINKLE_GROUP_TO_SLOT[0]].tile, 2);
        assert_eq!(SLOT_LAYOUT[WRINKLE_GROUP_TO_SLOT[16]].tile, 2);
        // Pair 2 (brow_raiseIn), pair 10 (cornerDepress purse), pair 11 (neck): tile 1.
        assert_eq!(SLOT_LAYOUT[WRINKLE_GROUP_TO_SLOT[4]].tile, 1);
        assert_eq!(SLOT_LAYOUT[WRINKLE_GROUP_TO_SLOT[20]].tile, 1);
        assert_eq!(SLOT_LAYOUT[WRINKLE_GROUP_TO_SLOT[22]].tile, 1);
    }
}
