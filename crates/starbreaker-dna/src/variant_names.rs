//! Slot-2 per-variant archetype name pool reader.
//!
//! Slot 2 holds one fixed 40-byte, NUL-padded ASCII name per variant, in
//! variant order. Entry 0 is the protos base (`protos_human_<g>_face_t1_pu`,
//! the head `.skin` bind pose); entries `1..num_variants` are the selectable
//! archetype heads (`male_archetype_v001_t1`, `silas_t1`, `macken_t2`, …).
//!
//! A CHF `DnaBlend.head_id` indexes this table directly, and each name is the
//! stem of that archetype's head mesh (`<name>_head.skinm`), whose positions
//! minus the protos base equal the slot-18 per-variant neutral deltas.

use crate::error::DnaError;
use crate::parser::DnaFile;

/// Pointer-table slot index for the per-variant archetype name pool.
pub const VARIANT_NAMES_SLOT: usize = 2;

/// Fixed byte stride of one name entry.
pub const VARIANT_NAME_STRIDE: usize = 40;

impl<'a> DnaFile<'a> {
    /// Read the `num_variants` archetype names from slot 2, in variant
    /// (= CHF `head_id`) order. Index 0 is the protos base.
    ///
    /// # Errors
    /// - [`DnaError::VariantNamesUnset`] if slot 2 is null.
    /// - [`DnaError::VariantNamesMalformed`] if the slot is too short for
    ///   `num_variants` entries or an entry is not ASCII.
    pub fn variant_names(&self) -> Result<Vec<String>, DnaError> {
        let data = self
            .slot_bytes(VARIANT_NAMES_SLOT)
            .ok_or(DnaError::VariantNamesUnset)?;
        let n = self.header.num_variants as usize;
        if data.len() < n * VARIANT_NAME_STRIDE {
            return Err(DnaError::VariantNamesMalformed(format!(
                "slot 2 has {} bytes, need {} for {n} names",
                data.len(),
                n * VARIANT_NAME_STRIDE
            )));
        }
        (0..n)
            .map(|i| {
                let entry = &data[i * VARIANT_NAME_STRIDE..(i + 1) * VARIANT_NAME_STRIDE];
                let len = entry.iter().position(|&b| b == 0).unwrap_or(entry.len());
                let name = &entry[..len];
                if !name.is_ascii() {
                    return Err(DnaError::VariantNamesMalformed(format!(
                        "name {i} is not ASCII"
                    )));
                }
                Ok(String::from_utf8_lossy(name).into_owned())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{VARIANT_NAME_STRIDE, VARIANT_NAMES_SLOT};
    use crate::parser::{DnaFile, MAGIC, MIN_FILE_SIZE, POINTER_TABLE_OFFSET};

    fn build(names: &[&str], num_variants: u16) -> Vec<u8> {
        let body = MIN_FILE_SIZE;
        let mut buf = vec![0u8; body + names.len() * VARIANT_NAME_STRIDE];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x28] = 13;
        buf[0x2a..0x2c].copy_from_slice(&num_variants.to_le_bytes());
        let slot_off = POINTER_TABLE_OFFSET + VARIANT_NAMES_SLOT * 8;
        buf[slot_off..slot_off + 8].copy_from_slice(&(body as u64).to_le_bytes());
        for (i, n) in names.iter().enumerate() {
            let o = body + i * VARIANT_NAME_STRIDE;
            buf[o..o + n.len()].copy_from_slice(n.as_bytes());
        }
        buf
    }

    #[test]
    fn reads_names_in_variant_order() {
        let buf = build(&["protos_human_male_face_t1_pu", "male_archetype_v001_t1", "silas_t1"], 3);
        let dna = DnaFile::read(&buf).expect("parse");
        assert_eq!(
            dna.variant_names().expect("names"),
            ["protos_human_male_face_t1_pu", "male_archetype_v001_t1", "silas_t1"]
        );
    }

    #[test]
    fn errors_when_slot_too_short_for_num_variants() {
        let buf = build(&["a", "b"], 3);
        let dna = DnaFile::read(&buf).expect("parse");
        assert!(dna.variant_names().is_err());
    }

    #[test]
    fn errors_when_slot2_unset() {
        let mut buf = build(&["a"], 1);
        let slot_off = POINTER_TABLE_OFFSET + VARIANT_NAMES_SLOT * 8;
        buf[slot_off..slot_off + 8].fill(0);
        let dna = DnaFile::read(&buf).expect("parse");
        assert!(matches!(
            dna.variant_names(),
            Err(crate::error::DnaError::VariantNamesUnset)
        ));
    }
}
