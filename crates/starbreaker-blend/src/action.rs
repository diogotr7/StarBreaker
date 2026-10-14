//! Animation actions as Blender 5.1 `.blend` blocks: a layered ("slotted")
//! `bAction` with one layer → one keyframe strip → one channel bag holding
//! the F-Curves, one slot for the animated ID ([`ActionData`]), and the
//! owner's `AnimData` ([`build_anim_data`]).
//!
//! Layouts and default values follow a file Blender 5.1 wrote. Arrays in the
//! layered structs are pointer arrays (`T **`), stored as raw DATA blocks.
//! Keys are linear (`BEZT_IPO_LIN`); handles are placed a third of the way
//! to the neighbouring keys. For forward compatibility Blender also chains
//! the F-Curves (and groups) into the action's legacy `curves` / `groups`
//! lists; so does this writer.

use crate::{PtrAlloc, write_block, write_cstr_fixed, write_f32, write_i16, write_i32, write_id_name, write_ptr};

pub const SDNA_IDX_ACTION: u32 = 28;
pub const SDNA_IDX_ACTION_GROUP: u32 = 27;
pub const SDNA_IDX_ACTION_LAYER: u32 = 33;
pub const SDNA_IDX_ACTION_SLOT: u32 = 34;
pub const SDNA_IDX_ACTION_STRIP: u32 = 35;
pub const SDNA_IDX_STRIP_KEYFRAME_DATA: u32 = 36;
pub const SDNA_IDX_CHANNEL_BAG: u32 = 37;
pub const SDNA_IDX_FCURVE: u32 = 52;
pub const SDNA_IDX_BEZTRIPLE: u32 = 150;
pub const SDNA_IDX_ANIM_DATA: u32 = 58;
pub const ACTION_SIZE: usize = 536;
pub const ANIM_DATA_SIZE: usize = 632;
/// `ID.adt` (Object, Key, ...): the ID's `AnimData` pointer.
pub const ID_ADT_OFFSET: usize = 408;
const BEZTRIPLE_SIZE: usize = 72;
/// `ID_OB` / `ID_KE` as `short` (the two-letter ID code, little-endian).
pub const ID_OB: i16 = i16::from_le_bytes(*b"OB");
pub const ID_KE: i16 = i16::from_le_bytes(*b"KE");
/// The handle of the one slot each action has.
const SLOT_HANDLE: i32 = 1;

/// One F-Curve: `rna_path[array_index]` keyed at `(frame, value)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub rna_path: String,
    pub array_index: i32,
    /// Channel group (Blender groups pose-bone curves by bone name).
    pub group: Option<String>,
    pub keys: Vec<[f32; 2]>,
}

/// A `bAction` and its blocks, with its pointer.
pub struct ActionData {
    pub action_ptr: u64,
    action: Vec<u8>,
    /// (sdna, ptr, count, bytes) DATA blocks after the `AC` block.
    blocks: Vec<(u32, u64, u32, Vec<u8>)>,
    slot_name: String,
}

fn ptr_array(ptrs: &[u64]) -> Vec<u8> {
    ptrs.iter().flat_map(|p| p.to_le_bytes()).collect()
}

fn bezier_keys(keys: &[[f32; 2]]) -> Vec<u8> {
    let mut out = vec![0u8; keys.len() * BEZTRIPLE_SIZE];
    for (i, &[frame, value]) in keys.iter().enumerate() {
        let d = &mut out[i * BEZTRIPLE_SIZE..(i + 1) * BEZTRIPLE_SIZE];
        let (prev, next) = (keys.get(i.wrapping_sub(1)).copied(), keys.get(i + 1).copied());
        let left = prev.map_or([frame - 1.0, value], |[f, v]| [frame - (frame - f) / 3.0, value - (value - v) / 3.0]);
        let right = next.map_or([frame + 1.0, value], |[f, v]| [frame + (f - frame) / 3.0, value + (v - value) / 3.0]);
        for (k, [x, y]) in [left, [frame, value], right].into_iter().enumerate() {
            write_f32(d, 12 * k, x);
            write_f32(d, 12 * k + 4, y);
        }
        d[48] = 1; // ipo: BEZT_IPO_LIN
        d[49] = 4; // h1: HD_AUTO_ANIM
        d[50] = 4; // h2
        write_f32(d, 56, 1.70158); // back
        write_f32(d, 60, 0.8); // amplitude
        write_f32(d, 64, 4.1); // period
        d[68] = 1; // auto_handle_type
    }
    out
}

impl ActionData {
    /// Lay out the action `name` animating the ID `owner_code` + `owner_name`
    /// (e.g. `ID_OB`, "Head_armature"). Curves of one group must be
    /// contiguous; every curve needs keys with ascending frames.
    pub fn new(name: &str, owner_code: i16, owner_name: &str, curves: &[Curve], ptrs: &mut PtrAlloc) -> Result<Self, String> {
        if curves.is_empty() {
            return Err(format!("action '{name}' has no curves"));
        }
        if let Some(bad) = curves.iter().find(|c| c.keys.is_empty() || c.keys.windows(2).any(|w| w[0][0] > w[1][0])) {
            return Err(format!("action '{name}': curve {}[{}] has no or unordered keys", bad.rna_path, bad.array_index));
        }
        // Groups: maximal runs of the same group name.
        let mut groups: Vec<(String, usize, usize)> = Vec::new(); // (name, first curve, count)
        for (i, curve) in curves.iter().enumerate() {
            if let Some(g) = &curve.group {
                match groups.last_mut() {
                    Some((name, first, count)) if name == g && *first + *count == i => *count += 1,
                    _ => {
                        if groups.iter().any(|(n, ..)| n == g) {
                            return Err(format!("action '{name}': group '{g}' is not contiguous"));
                        }
                        groups.push((g.clone(), i, 1));
                    }
                }
            }
        }

        let action_ptr = ptrs.alloc();
        let [layer_arr, layer, strip_arr, strip, slot_arr, slot, kd_arr, kd, bag_arr, bag, group_arr, fcurve_arr] =
            [(); 12].map(|_| ptrs.alloc());
        let group_ptrs: Vec<u64> = groups.iter().map(|_| ptrs.alloc()).collect();
        let curve_ptrs: Vec<(u64, u64, u64)> = curves.iter().map(|_| (ptrs.alloc(), ptrs.alloc(), ptrs.alloc())).collect();
        let first_curve = curve_ptrs[0].0;
        let last_curve = curve_ptrs[curve_ptrs.len() - 1].0;
        let slot_name = format!("{}{owner_name}", String::from_utf8_lossy(&owner_code.to_le_bytes()));

        let mut blocks: Vec<(u32, u64, u32, Vec<u8>)> = Vec::new();
        blocks.push((0, kd_arr, 1, ptr_array(&[kd])));
        let mut d = vec![0u8; 16];
        write_ptr(&mut d, 0, bag_arr);
        write_i32(&mut d, 8, 1);
        blocks.push((SDNA_IDX_STRIP_KEYFRAME_DATA, kd, 1, d));
        blocks.push((0, bag_arr, 1, ptr_array(&[bag])));
        let mut d = vec![0u8; 32];
        write_i32(&mut d, 0, SLOT_HANDLE);
        write_i32(&mut d, 4, groups.len() as i32);
        write_ptr(&mut d, 8, if groups.is_empty() { 0 } else { group_arr });
        write_i32(&mut d, 20, curves.len() as i32);
        write_ptr(&mut d, 24, fcurve_arr);
        blocks.push((SDNA_IDX_CHANNEL_BAG, bag, 1, d));
        if !groups.is_empty() {
            blocks.push((0, group_arr, 1, ptr_array(&group_ptrs)));
        }
        for (g, (group_name, first, count)) in groups.iter().enumerate() {
            let mut d = vec![0u8; 136];
            write_ptr(&mut d, 0, group_ptrs.get(g + 1).copied().unwrap_or(0));
            write_ptr(&mut d, 8, if g > 0 { group_ptrs[g - 1] } else { 0 });
            write_ptr(&mut d, 16, curve_ptrs[*first].0);
            write_ptr(&mut d, 24, curve_ptrs[first + count - 1].0);
            write_i32(&mut d, 32, *first as i32);
            write_i32(&mut d, 36, *count as i32);
            write_ptr(&mut d, 40, bag);
            write_i32(&mut d, 48, 1); // flag: AGRP_SELECTED
            write_cstr_fixed(&mut d, 56, 64, group_name);
            blocks.push((SDNA_IDX_ACTION_GROUP, group_ptrs[g], 1, d));
        }
        blocks.push((0, fcurve_arr, 1, ptr_array(&curve_ptrs.iter().map(|c| c.0).collect::<Vec<_>>())));
        for (i, (curve, &(curve_ptr, bezt_ptr, path_ptr))) in curves.iter().zip(&curve_ptrs).enumerate() {
            let mut d = vec![0u8; 120];
            write_ptr(&mut d, 0, curve_ptrs.get(i + 1).map_or(0, |c| c.0));
            write_ptr(&mut d, 8, if i > 0 { curve_ptrs[i - 1].0 } else { 0 });
            let group = groups.iter().position(|(_, first, count)| (*first..first + count).contains(&i));
            write_ptr(&mut d, 16, group.map_or(0, |g| group_ptrs[g]));
            write_ptr(&mut d, 48, bezt_ptr);
            write_i32(&mut d, 64, curve.keys.len() as i32);
            write_i16(&mut d, 76, 1); // flag: FCURVE_VISIBLE
            d[80] = 1; // auto_smoothing: continuous acceleration
            write_i32(&mut d, 84, curve.array_index);
            write_ptr(&mut d, 88, path_ptr);
            write_i32(&mut d, 96, if group.is_some() { 3 } else { 0 }); // color_mode: auto YRGB for bone channels
            blocks.push((SDNA_IDX_FCURVE, curve_ptr, 1, d));
            blocks.push((SDNA_IDX_BEZTRIPLE, bezt_ptr, curve.keys.len() as u32, bezier_keys(&curve.keys)));
            let mut path = curve.rna_path.as_bytes().to_vec();
            path.push(0);
            blocks.push((0, path_ptr, 1, path));
        }
        blocks.push((0, layer_arr, 1, ptr_array(&[layer])));
        let mut d = vec![0u8; 88];
        write_cstr_fixed(&mut d, 0, 64, "Layer");
        write_f32(&mut d, 64, 1.0); // influence
        write_ptr(&mut d, 72, strip_arr);
        write_i32(&mut d, 80, 1);
        blocks.push((SDNA_IDX_ACTION_LAYER, layer, 1, d));
        blocks.push((0, strip_arr, 1, ptr_array(&[strip])));
        let mut d = vec![0u8; 24];
        write_f32(&mut d, 8, f32::NEG_INFINITY);
        write_f32(&mut d, 12, f32::INFINITY);
        blocks.push((SDNA_IDX_ACTION_STRIP, strip, 1, d));
        blocks.push((0, slot_arr, 1, ptr_array(&[slot])));
        let mut d = vec![0u8; 280];
        write_cstr_fixed(&mut d, 0, 258, &slot_name);
        write_i16(&mut d, 258, owner_code);
        write_i32(&mut d, 260, SLOT_HANDLE);
        d[264] = 1; // slot_flags: expanded
        blocks.push((SDNA_IDX_ACTION_SLOT, slot, 1, d));

        let mut action = vec![0u8; ACTION_SIZE];
        write_id_name(&mut action, "AC", name);
        write_ptr(&mut action, 408, layer_arr);
        write_i32(&mut action, 416, 1);
        write_ptr(&mut action, 424, slot_arr);
        write_i32(&mut action, 432, 1);
        write_i32(&mut action, 436, SLOT_HANDLE); // last_slot_handle
        write_ptr(&mut action, 440, kd_arr);
        write_i32(&mut action, 448, 1);
        write_ptr(&mut action, 456, first_curve); // legacy `curves`
        write_ptr(&mut action, 464, last_curve);
        write_ptr(&mut action, 472, group_ptrs.first().copied().unwrap_or(0)); // legacy `groups`
        write_ptr(&mut action, 480, group_ptrs.last().copied().unwrap_or(0));
        write_i32(&mut action, 512, owner_code as i32); // idroot
        Ok(Self { action_ptr, action, blocks, slot_name })
    }

    /// Keep the action when nothing uses it (an extra clip of the same ID):
    /// `LIB_FAKEUSER` with one user.
    pub fn set_fake_user(&mut self) {
        write_i16(&mut self.action, 298, 1 << 9);
        write_i32(&mut self.action, 304, 1);
    }

    /// The slot name the owner's `AnimData` must name.
    pub fn slot_name(&self) -> &str {
        &self.slot_name
    }

    /// The owner's `AnimData` for this action.
    pub fn anim_data(&self) -> Vec<u8> {
        build_anim_data(self.action_ptr, SLOT_HANDLE, &self.slot_name)
    }

    /// Write the `AC` block and its data.
    pub fn write(&self, out: &mut Vec<u8>) {
        write_block(out, b"AC\0\0", SDNA_IDX_ACTION, self.action_ptr, 1, &self.action);
        for (sdna, ptr, count, data) in &self.blocks {
            write_block(out, b"DATA", *sdna, *ptr, *count, data);
        }
    }
}

/// An `AnimData` assigning `action_ptr` (slot `slot_handle` / `slot_name`).
pub fn build_anim_data(action_ptr: u64, slot_handle: i32, slot_name: &str) -> Vec<u8> {
    let mut d = vec![0u8; ANIM_DATA_SIZE];
    write_ptr(&mut d, 0, action_ptr);
    write_i32(&mut d, 8, slot_handle);
    write_cstr_fixed(&mut d, 12, 258, slot_name);
    write_f32(&mut d, 624, 1.0); // act_influence
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptr(d: &[u8], off: usize) -> u64 {
        u64::from_le_bytes(d[off..off + 8].try_into().unwrap())
    }
    fn int(d: &[u8], off: usize) -> i32 {
        i32::from_le_bytes(d[off..off + 4].try_into().unwrap())
    }

    fn jaw_curves() -> Vec<Curve> {
        let rot = |i| Curve {
            rna_path: "pose.bones[\"jaw\"].rotation_quaternion".into(),
            array_index: i,
            group: Some("jaw".into()),
            keys: vec![[1.0, if i == 0 { 1.0 } else { 0.0 }], [11.0, 0.5]],
        };
        (0..4).map(rot).collect()
    }

    fn block<'a>(a: &'a ActionData, sdna: u32) -> Vec<&'a (u32, u64, u32, Vec<u8>)> {
        a.blocks.iter().filter(|b| b.0 == sdna).collect()
    }

    #[test]
    fn action_chains_layer_strip_bag_and_curves() {
        let a = ActionData::new("expression", ID_OB, "Rig", &jaw_curves(), &mut PtrAlloc::new(0x1000)).unwrap();
        assert_eq!(&a.action[40..53], b"ACexpression\0");
        assert_eq!(int(&a.action, 512), 16975, "idroot = ID_OB");
        let bag = block(&a, SDNA_IDX_CHANNEL_BAG)[0];
        assert_eq!(int(&bag.3, 20), 4, "four curves");
        assert_eq!(int(&bag.3, 4), 1, "one group");
        let curves = block(&a, SDNA_IDX_FCURVE);
        assert_eq!(ptr(&a.action, 456), curves[0].1, "legacy curves first");
        assert_eq!(ptr(&a.action, 464), curves[3].1, "legacy curves last");
        assert_eq!(ptr(&curves[1].3, 8), curves[0].1, "curves are chained");
        let group = block(&a, SDNA_IDX_ACTION_GROUP)[0];
        assert_eq!(ptr(&curves[2].3, 16), group.1);
        assert_eq!(int(&group.3, 36), 4);
        let slot = block(&a, SDNA_IDX_ACTION_SLOT)[0];
        assert_eq!(&slot.3[0..6], b"OBRig\0");
        assert_eq!(a.slot_name(), "OBRig");
    }

    #[test]
    fn keys_are_linear_beztriples() {
        let a = ActionData::new("e", ID_OB, "Rig", &jaw_curves(), &mut PtrAlloc::new(0x1000)).unwrap();
        let keys = block(&a, SDNA_IDX_BEZTRIPLE)[0];
        assert_eq!(keys.2, 2, "count = keys");
        let k1 = &keys.3[BEZTRIPLE_SIZE..];
        let f = |o: usize| f32::from_le_bytes(k1[o..o + 4].try_into().unwrap());
        assert_eq!((f(12), f(16)), (11.0, 0.5), "key at frame 11");
        assert_eq!(k1[48], 1, "linear");
    }

    #[test]
    fn anim_data_names_the_slot() {
        let a = ActionData::new("e", ID_KE, "Key", &[Curve { rna_path: "key_blocks[\"bs_000\"].value".into(), array_index: 0, group: None, keys: vec![[1.0, 0.0]] }], &mut PtrAlloc::new(0x1000)).unwrap();
        let d = a.anim_data();
        assert_eq!(ptr(&d, 0), a.action_ptr);
        assert_eq!(int(&d, 8), 1);
        assert_eq!(&d[12..18], b"KEKey\0");
        assert!(block(&a, SDNA_IDX_ACTION_GROUP).is_empty(), "shape-key curves are ungrouped");
    }

    #[test]
    fn bad_curves_are_errors() {
        let mut pa = PtrAlloc::new(0x1000);
        assert!(ActionData::new("e", ID_OB, "Rig", &[], &mut pa).is_err());
        let mut curves = jaw_curves();
        curves[0].keys = vec![[5.0, 0.0], [1.0, 0.0]];
        assert!(ActionData::new("e", ID_OB, "Rig", &curves, &mut pa).is_err(), "unordered frames");
        let mut curves = jaw_curves();
        curves[2].group = Some("other".into());
        assert!(ActionData::new("e", ID_OB, "Rig", &curves, &mut pa).is_err(), "split group");
    }
}
