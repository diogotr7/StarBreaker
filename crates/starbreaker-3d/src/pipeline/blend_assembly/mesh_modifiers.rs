//! The modifier stack the `.blend` writers give a mesh object, in order:
//! Armature (skinned meshes), Weld + Weighted Normal (meshes without authored
//! normals), Displace (decal-offset meshes). [`MeshModifiers`] holds one
//! pointer per modifier (0 = absent), links the present ones as
//! `Object.modifiers` ([`MeshModifiers::first`] / [`MeshModifiers::last`]) and
//! writes their blocks ([`MeshModifiers::write`]); the flat, NMC-hierarchy and
//! scene-wrapper writers share it.
//!
//! Weld + Weighted Normal exist to synthesize smooth shading across the split
//! vertices of a mesh that has no normals of its own. A mesh with authored
//! normals carries them as `custom_normal` instead (see `custom_normals`), and
//! there both modifiers are wrong: Weighted Normal overwrites custom normals
//! with face-area-weighted ones, and Weld merges split vertices into larger
//! corner fans than the custom normals were encoded against, so Blender would
//! average and misdecode them. [`writes_normal_modifiers`] is the rule.

use starbreaker_blend::armature::{build_armature_modifier, SDNA_IDX_ARMATURE_MODIFIER};
use starbreaker_blend::{
    build_displace_modifier, build_weighted_normal_modifier, build_weld_modifier, write_block, PtrAlloc,
    SDNA_IDX_DISPLACE_MODIFIER, SDNA_IDX_WEIGHTED_NORMAL_MODIFIER, SDNA_IDX_WELD_MODIFIER,
};

use super::{rig, DECAL_OFFSET_GROUP_NAME, DECAL_OFFSET_MIDLEVEL, DECAL_OFFSET_MODIFIER_NAME};
use crate::types::Mesh;

const WELD_MODIFIER_NAME: &str = "StarBreaker Weld";
const WELD_MERGE_DISTANCE: f32 = 0.0005;
const WEIGHTED_NORMAL_MODIFIER_NAME: &str = "StarBreaker Weighted Normal";
const WEIGHTED_NORMAL_WEIGHT: i16 = 50;
const WEIGHTED_NORMAL_THRESHOLD: f32 = 0.01;

/// Whether a mesh gets the Weld + Weighted Normal modifiers: only when it has
/// no authored normals (see the module docs).
pub(super) fn writes_normal_modifiers(mesh: &Mesh) -> bool {
    mesh.normals.is_none()
}

/// One mesh object's modifier pointers, in stack order; 0 = absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MeshModifiers {
    pub armature: u64,
    pub weld: u64,
    pub weighted_normal: u64,
    pub decal_offset: u64,
}

impl MeshModifiers {
    /// Allocate the non-armature modifiers: Weld + Weighted Normal when
    /// `normal_modifiers`, Displace when `decal_offset`. Set `armature`
    /// separately for a skinned mesh.
    pub fn allocate(ptrs: &mut PtrAlloc, normal_modifiers: bool, decal_offset: bool) -> Self {
        let (weld, weighted_normal) = if normal_modifiers { (ptrs.alloc(), ptrs.alloc()) } else { (0, 0) };
        let decal_offset = if decal_offset { ptrs.alloc() } else { 0 };
        Self { armature: 0, weld, weighted_normal, decal_offset }
    }

    fn present(&self) -> Vec<u64> {
        [self.armature, self.weld, self.weighted_normal, self.decal_offset]
            .into_iter()
            .filter(|&ptr| ptr != 0)
            .collect()
    }

    fn neighbours(&self, ptr: u64) -> (u64, u64) {
        let present = self.present();
        let at = present.iter().position(|&p| p == ptr);
        let prev = at.and_then(|i| i.checked_sub(1)).map_or(0, |i| present[i]);
        let next = at.and_then(|i| present.get(i + 1).copied()).unwrap_or(0);
        (next, prev)
    }

    /// `Object.modifiers.first` (0 for an empty stack).
    pub fn first(&self) -> u64 {
        self.present().first().copied().unwrap_or(0)
    }

    /// `Object.modifiers.last` (0 for an empty stack).
    pub fn last(&self) -> u64 {
        self.present().last().copied().unwrap_or(0)
    }

    /// Write the present modifiers' blocks (part of the object's DATA chain).
    /// `armature_object_ptr` is the deforming armature object (used when
    /// `armature` is set); `decal_strength` the Displace strength.
    pub fn write(&self, out: &mut Vec<u8>, armature_object_ptr: u64, decal_strength: f32) {
        if self.armature != 0 {
            let (next, prev) = self.neighbours(self.armature);
            write_block(out, b"DATA", SDNA_IDX_ARMATURE_MODIFIER, self.armature, 1,
                &build_armature_modifier(rig::ARMATURE_MODIFIER_NAME, next, prev, armature_object_ptr));
        }
        if self.weld != 0 {
            let (next, prev) = self.neighbours(self.weld);
            write_block(out, b"DATA", SDNA_IDX_WELD_MODIFIER, self.weld, 1,
                &build_weld_modifier(WELD_MODIFIER_NAME, next, prev, WELD_MERGE_DISTANCE));
        }
        if self.weighted_normal != 0 {
            let (next, prev) = self.neighbours(self.weighted_normal);
            write_block(out, b"DATA", SDNA_IDX_WEIGHTED_NORMAL_MODIFIER, self.weighted_normal, 1,
                &build_weighted_normal_modifier(
                    WEIGHTED_NORMAL_MODIFIER_NAME,
                    next,
                    prev,
                    WEIGHTED_NORMAL_WEIGHT,
                    WEIGHTED_NORMAL_THRESHOLD,
                ));
        }
        if self.decal_offset != 0 {
            let (next, prev) = self.neighbours(self.decal_offset);
            write_block(out, b"DATA", SDNA_IDX_DISPLACE_MODIFIER, self.decal_offset, 1,
                &build_displace_modifier(
                    DECAL_OFFSET_MODIFIER_NAME,
                    next,
                    prev,
                    decal_strength,
                    DECAL_OFFSET_GROUP_NAME,
                    DECAL_OFFSET_MIDLEVEL,
                ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(block ptr, next, prev)` of each written modifier block.
    fn links(modifiers: &MeshModifiers) -> Vec<(u64, u64, u64)> {
        let mut out = Vec::new();
        let mut bytes = Vec::new();
        modifiers.write(&mut bytes, 0x900, 0.005);
        let mut offset = 0;
        while offset + 32 <= bytes.len() {
            let ptr = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            let len = u64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap()) as usize;
            let data = &bytes[offset + 32..offset + 32 + len];
            let next = u64::from_le_bytes(data[0..8].try_into().unwrap());
            let prev = u64::from_le_bytes(data[8..16].try_into().unwrap());
            out.push((ptr, next, prev));
            offset += 32 + len;
        }
        out
    }

    #[test]
    fn full_stack_links_in_order() {
        let m = MeshModifiers { armature: 0x10, weld: 0x20, weighted_normal: 0x30, decal_offset: 0x40 };
        assert_eq!((m.first(), m.last()), (0x10, 0x40));
        assert_eq!(links(&m), vec![(0x10, 0x20, 0), (0x20, 0x30, 0x10), (0x30, 0x40, 0x20), (0x40, 0, 0x30)]);
    }

    #[test]
    fn absent_modifiers_are_skipped_in_the_links() {
        let m = MeshModifiers { armature: 0x10, weld: 0, weighted_normal: 0, decal_offset: 0x40 };
        assert_eq!((m.first(), m.last()), (0x10, 0x40));
        assert_eq!(links(&m), vec![(0x10, 0x40, 0), (0x40, 0, 0x10)]);
    }

    #[test]
    fn empty_stack_writes_nothing() {
        let m = MeshModifiers::default();
        assert_eq!((m.first(), m.last()), (0, 0));
        assert!(links(&m).is_empty());
    }

    #[test]
    fn allocate_omits_normal_modifiers_on_request() {
        let mut ptrs = PtrAlloc::new(0x1000);
        let with = MeshModifiers::allocate(&mut ptrs, true, false);
        assert!(with.weld != 0 && with.weighted_normal != 0 && with.decal_offset == 0);
        let without = MeshModifiers::allocate(&mut ptrs, false, true);
        assert_eq!((without.weld, without.weighted_normal), (0, 0));
        assert_ne!(without.decal_offset, 0);
    }

    #[test]
    fn authored_normals_drop_the_normal_modifiers() {
        let mut mesh = Mesh::empty();
        assert!(writes_normal_modifiers(&mesh));
        mesh.normals = Some(Vec::new());
        assert!(!writes_normal_modifiers(&mesh));
    }
}
