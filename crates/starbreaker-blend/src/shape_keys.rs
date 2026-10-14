//! Shape keys as Blender 5.1 `.blend` blocks: one relative `Key` ID with a
//! `Basis` `KeyBlock` plus one block per target ([`ShapeKeyData`]).
//!
//! `KeyBlock.data` holds absolute vertex positions (float3 per vertex). The
//! basis is the mesh's own positions; targets start at value 0 relative to
//! the basis. The mesh points at the key through `Mesh.key` (offset 416,
//! [`MESH_KEY_OFFSET`]) and the key back at the mesh through `Key.from`.
//! An animated key carries an `AnimData` ([`ShapeKeyData::set_anim_data`]).

use crate::{floats3_data, write_block, write_cstr_fixed, write_f32, write_i16, write_i32, write_id_name, write_ptr, PtrAlloc};

pub const SDNA_IDX_KEY: u32 = 244;
pub const SDNA_IDX_KEY_BLOCK: u32 = 243;
pub const KEY_SIZE: usize = 504;
pub const KEY_BLOCK_SIZE: usize = 184;
/// `Mesh.key` pointer offset.
pub const MESH_KEY_OFFSET: usize = 416;
/// `Key.type` for relative shape keys (`KEY_RELATIVE`).
const KEY_RELATIVE: u8 = 1;

/// A `Key` and its `KeyBlock`s, with their pointers.
pub struct ShapeKeyData {
    pub key_ptr: u64,
    key: Vec<u8>,
    /// (KeyBlock ptr, KeyBlock, data ptr, float3 data).
    blocks: Vec<(u64, Vec<u8>, u64, Vec<u8>)>,
    /// The key's `AnimData` (pointer, block), when its values are animated.
    anim: Option<(u64, Vec<u8>)>,
}

impl ShapeKeyData {
    /// Lay out the key `name` of the mesh at `mesh_ptr`: a `Basis` at `basis`
    /// and one target per `(name, absolute positions)`. Every target must have
    /// one position per basis vertex.
    pub fn new(
        name: &str,
        mesh_ptr: u64,
        basis: &[[f32; 3]],
        targets: &[(String, Vec<[f32; 3]>)],
        ptrs: &mut PtrAlloc,
    ) -> Result<Self, String> {
        if let Some((bad, positions)) = targets.iter().find(|(_, p)| p.len() != basis.len()) {
            return Err(format!("shape key '{bad}' has {} positions, the mesh {}", positions.len(), basis.len()));
        }
        let key_ptr = ptrs.alloc();
        let names = std::iter::once("Basis").chain(targets.iter().map(|(n, _)| n.as_str()));
        let datas = std::iter::once(basis).chain(targets.iter().map(|(_, p)| p.as_slice()));
        let entries: Vec<(&str, &[[f32; 3]], u64, u64)> =
            names.zip(datas).map(|(n, d)| (n, d, ptrs.alloc(), ptrs.alloc())).collect();
        let basis_ptr = entries[0].2;
        let blocks = entries
            .iter()
            .enumerate()
            .map(|(i, &(kb_name, positions, kb_ptr, data_ptr))| {
                let mut kb = vec![0u8; KEY_BLOCK_SIZE];
                write_ptr(&mut kb, 0, entries.get(i + 1).map_or(0, |e| e.2));
                write_ptr(&mut kb, 8, if i > 0 { entries[i - 1].2 } else { 0 });
                write_f32(&mut kb, 16, 0.1 * i as f32); // pos
                write_f32(&mut kb, 20, if i == 0 { 1.0 } else { 0.0 }); // curval
                write_i16(&mut kb, 28, 0); // relative: the basis
                write_i32(&mut kb, 32, positions.len() as i32); // totelem
                write_i32(&mut kb, 36, i as i32 + 1); // uid
                write_ptr(&mut kb, 40, data_ptr);
                write_cstr_fixed(&mut kb, 48, 64, kb_name);
                write_f32(&mut kb, 180, 1.0); // slidermax
                (kb_ptr, kb, data_ptr, floats3_data(positions))
            })
            .collect();
        let mut key = vec![0u8; KEY_SIZE];
        write_id_name(&mut key, "KE", name);
        write_ptr(&mut key, 416, basis_ptr); // refkey
        key[424] = 3; // elemstr: 3 floats per element
        key[425] = 4; // ... of IPO_FLOAT
        write_i32(&mut key, 456, 12); // elemsize
        write_ptr(&mut key, 464, basis_ptr);
        write_ptr(&mut key, 472, entries.last().map_or(0, |e| e.2));
        write_ptr(&mut key, 480, mesh_ptr); // from
        write_i32(&mut key, 488, entries.len() as i32); // totkey
        key[494] = KEY_RELATIVE;
        write_i32(&mut key, 500, entries.len() as i32 + 1); // uidgen
        Ok(Self { key_ptr, key, blocks, anim: None })
    }

    /// Animate the key: `Key.adt` points at the `AnimData` block `data`.
    pub fn set_anim_data(&mut self, adt_ptr: u64, data: Vec<u8>) {
        write_ptr(&mut self.key, crate::action::ID_ADT_OFFSET, adt_ptr);
        self.anim = Some((adt_ptr, data));
    }

    /// Write the `KE` block, its `AnimData` and its key blocks with their data.
    pub fn write(&self, out: &mut Vec<u8>) {
        write_block(out, b"KE\0\0", SDNA_IDX_KEY, self.key_ptr, 1, &self.key);
        if let Some((ptr, data)) = &self.anim {
            write_block(out, b"DATA", crate::action::SDNA_IDX_ANIM_DATA, *ptr, 1, data);
        }
        for (kb_ptr, kb, data_ptr, data) in &self.blocks {
            write_block(out, b"DATA", SDNA_IDX_KEY_BLOCK, *kb_ptr, 1, kb);
            write_block(out, b"DATA", 0, *data_ptr, 1, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptr(d: &[u8], off: usize) -> u64 {
        u64::from_le_bytes(d[off..off + 8].try_into().unwrap())
    }

    fn sample(ptrs: &mut PtrAlloc) -> ShapeKeyData {
        let basis = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let mut open = basis.clone();
        open[0][2] = 0.5;
        ShapeKeyData::new("Key", 0x77, &basis, &[("bs_000".into(), open)], ptrs).unwrap()
    }

    #[test]
    fn key_chains_basis_and_targets() {
        let key = sample(&mut PtrAlloc::new(0x1000));
        assert_eq!(&key.key[40..46], b"KEKey\0");
        let (basis_ptr, basis, ..) = &key.blocks[0];
        let (target_ptr, target, ..) = &key.blocks[1];
        assert_eq!(ptr(&key.key, 416), *basis_ptr, "refkey");
        assert_eq!((ptr(&key.key, 464), ptr(&key.key, 472)), (*basis_ptr, *target_ptr));
        assert_eq!(ptr(&key.key, 480), 0x77, "from = mesh");
        assert_eq!(i32::from_le_bytes(key.key[488..492].try_into().unwrap()), 2);
        assert_eq!((ptr(basis, 0), ptr(target, 8)), (*target_ptr, *basis_ptr));
        assert_eq!(&basis[48..54], b"Basis\0");
        assert_eq!(&target[48..55], b"bs_000\0");
        assert_eq!(f32::from_le_bytes(target[20..24].try_into().unwrap()), 0.0, "targets start at 0");
    }

    #[test]
    fn key_block_data_is_absolute_positions() {
        let key = sample(&mut PtrAlloc::new(0x1000));
        let (_, target, data_ptr, data) = &key.blocks[1];
        assert_eq!(ptr(target, 40), *data_ptr);
        assert_eq!(i32::from_le_bytes(target[32..36].try_into().unwrap()), 3, "totelem");
        assert_eq!(data.len(), 36);
        assert_eq!(f32::from_le_bytes(data[8..12].try_into().unwrap()), 0.5);
    }

    #[test]
    fn mismatched_target_is_an_error() {
        let basis = vec![[0.0; 3]; 3];
        let err = ShapeKeyData::new("Key", 1, &basis, &[("x".into(), vec![[0.0; 3]; 2])], &mut PtrAlloc::new(0x1000));
        assert!(err.is_err());
    }

    #[test]
    fn written_blocks_cover_key_blocks_and_data() {
        let key = sample(&mut PtrAlloc::new(0x1000));
        let mut out = Vec::new();
        key.write(&mut out);
        assert_eq!(out.len(), 5 * 32 + KEY_SIZE + 2 * KEY_BLOCK_SIZE + 2 * 36);
    }
}
