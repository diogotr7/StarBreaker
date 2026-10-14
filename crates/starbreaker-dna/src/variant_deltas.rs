//! Slot-31 per-variant packed-delta payload iterator.
//!
//! Each variant in the master `.dna` carries two arrays of variable-stride
//! records: MAIN (appearance) and SECONDARY (corrective). Both share the
//! same on-disk format, anchored against RigLogic Section D in
//! `docs/riglogic/PHASE_4B_INVESTIGATION.md` §5:
//!
//! ```text
//! repeat num_*_delta_records times:
//!   u16 output_idx           // output channel = joint * 3 + axis (§6.9)
//!   u16 payload_len (N)      // u32 count
//!   N x u32 packed_payload   // each decodes via decode_packed_delta
//! ```
//!
//! This module yields each record as a borrowed [`VariantDeltaRecord`];
//! the consumer decodes u32s on the fly via `u32::from_le_bytes`.

use crate::error::{DnaError, VariantDeltaSide};
use crate::parser::{DnaFile, VARIANT_ARRAY_SLOT};

/// One variant-delta record, borrowed from the `.dna` buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariantDeltaRecord<'a> {
    /// Output channel: `joint * 3 + axis` (PHASE_4B §6.9).
    pub output_idx: u16,
    /// Raw packed-payload bytes — exactly `payload_len * 4` bytes long.
    /// Consumer decodes via:
    /// ```ignore
    /// for chunk in record.packed_bytes.chunks_exact(4) {
    ///     let packed = u32::from_le_bytes(chunk.try_into().unwrap());
    ///     // ...
    /// }
    /// ```
    pub packed_bytes: &'a [u8],
}

/// Iterator over a variant's MAIN or SECONDARY delta records. Borrows
/// the `.dna` file buffer.
#[derive(Debug)]
pub struct VariantDeltaIter<'a> {
    bytes: &'a [u8],
    cursor: usize,
    /// Upper bound on the variant's payload region. Set to file size in
    /// the current implementation; the iterator stops after
    /// `expected_records` records, not at this bound (the bound only
    /// matters as a truncation guard).
    end: usize,
    remaining: u32,
    expected_records: u32,
    variant_idx: usize,
    side: VariantDeltaSide,
    /// Set to true once we've yielded a truncation error so we don't
    /// keep yielding the same error forever.
    fatal: bool,
}

impl<'a> Iterator for VariantDeltaIter<'a> {
    type Item = Result<VariantDeltaRecord<'a>, DnaError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 || self.fatal {
            return None;
        }

        let record_start = self.cursor;

        // Need 4 bytes for the (u16 output_idx, u16 N) header.
        if record_start + 4 > self.end {
            self.fatal = true;
            let got = self.expected_records - self.remaining;
            return Some(Err(DnaError::VariantDeltaTruncated {
                variant_idx: self.variant_idx,
                side: self.side,
                expected_records: self.expected_records,
                got_records: got,
                bytes_remaining: self.end.saturating_sub(record_start),
            }));
        }

        let output_idx = u16::from_le_bytes([
            self.bytes[self.cursor],
            self.bytes[self.cursor + 1],
        ]);
        let n = u16::from_le_bytes([
            self.bytes[self.cursor + 2],
            self.bytes[self.cursor + 3],
        ]) as usize;
        self.cursor += 4;

        let packed_len_bytes = n * 4;
        if self.cursor + packed_len_bytes > self.end {
            self.fatal = true;
            let got = self.expected_records - self.remaining;
            return Some(Err(DnaError::VariantDeltaTruncated {
                variant_idx: self.variant_idx,
                side: self.side,
                expected_records: self.expected_records,
                got_records: got,
                bytes_remaining: self.end.saturating_sub(record_start),
            }));
        }

        let packed_bytes = &self.bytes[self.cursor..self.cursor + packed_len_bytes];
        self.cursor += packed_len_bytes;
        self.remaining -= 1;

        Some(Ok(VariantDeltaRecord {
            output_idx,
            packed_bytes,
        }))
    }
}

impl<'a> DnaFile<'a> {
    /// Iterate the variant's MAIN (appearance) delta records.
    pub fn variant_main_deltas(
        &self,
        variant_idx: usize,
    ) -> Result<VariantDeltaIter<'a>, DnaError> {
        self.variant_deltas_inner(variant_idx, VariantDeltaSide::Main)
    }

    /// Iterate the variant's SECONDARY (corrective) delta records.
    pub fn variant_secondary_deltas(
        &self,
        variant_idx: usize,
    ) -> Result<VariantDeltaIter<'a>, DnaError> {
        self.variant_deltas_inner(variant_idx, VariantDeltaSide::Secondary)
    }

    fn variant_deltas_inner(
        &self,
        variant_idx: usize,
        side: VariantDeltaSide,
    ) -> Result<VariantDeltaIter<'a>, DnaError> {
        // The variant_records vec is empty when slot 31 is null (LOD files);
        // surface that as VariantArrayUnset before any per-variant indexing.
        if self.pointer_table[VARIANT_ARRAY_SLOT] == 0 {
            return Err(DnaError::VariantArrayUnset);
        }
        let num_variants = self.variant_records.len();
        if variant_idx >= num_variants {
            return Err(DnaError::VariantOutOfRange {
                variant_idx,
                num_variants,
            });
        }

        let rec = &self.variant_records[variant_idx];
        let (offset, count) = match side {
            VariantDeltaSide::Main => (
                rec.main_delta_entries_offset as usize,
                rec.num_main_delta_records,
            ),
            VariantDeltaSide::Secondary => (
                rec.secondary_delta_entries_offset as usize,
                rec.num_secondary_delta_records,
            ),
        };

        Ok(VariantDeltaIter {
            bytes: self.bytes,
            cursor: offset,
            end: self.bytes.len(),
            remaining: count,
            expected_records: count,
            variant_idx,
            side,
            fatal: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{
        DnaFile, MAGIC, MIN_FILE_SIZE, POINTER_TABLE_OFFSET, VARIANT_ARRAY_SLOT,
        VARIANT_RECORD_STRIDE,
    };

    /// Build a synthetic `.dna` buffer with a populated slot-31 variant
    /// array. `variants` is a list of
    /// `(num_main_records, num_secondary_records, primary_scale,
    ///   secondary_scale, main_payload_bytes, secondary_payload_bytes)`
    /// tuples; each variant's payloads are placed contiguously after
    /// the variant-record header table.
    fn build_dna_with_variants(
        variants: &[(u32, u32, f64, f64, Vec<u8>, Vec<u8>)],
    ) -> Vec<u8> {
        let nv = variants.len() as u16;
        let body_off = MIN_FILE_SIZE; // 0x140
        let header_bytes = nv as usize * VARIANT_RECORD_STRIDE;
        let total_payload_bytes: usize = variants
            .iter()
            .map(|(_, _, _, _, m, s)| m.len() + s.len())
            .sum();
        let total = body_off + header_bytes + total_payload_bytes;

        let mut buf = vec![0u8; total];
        buf[..16].copy_from_slice(&MAGIC);
        buf[0x2a..0x2c].copy_from_slice(&nv.to_le_bytes());

        // Slot 31 points at body_off.
        let slot31_off = POINTER_TABLE_OFFSET + VARIANT_ARRAY_SLOT * 8;
        buf[slot31_off..slot31_off + 8]
            .copy_from_slice(&(body_off as u64).to_le_bytes());

        // Place variant-record headers, then payloads, tracking offsets.
        let mut payload_cursor = body_off + header_bytes;
        for (i, (nm, ns, pscale, sscale, mpay, spay)) in variants.iter().enumerate() {
            let hdr_off = body_off + i * VARIANT_RECORD_STRIDE;
            buf[hdr_off..hdr_off + 4].copy_from_slice(&nm.to_le_bytes());
            buf[hdr_off + 4..hdr_off + 8].copy_from_slice(&ns.to_le_bytes());
            buf[hdr_off + 8..hdr_off + 16].copy_from_slice(&pscale.to_le_bytes());
            buf[hdr_off + 16..hdr_off + 24].copy_from_slice(&sscale.to_le_bytes());

            let main_off = payload_cursor;
            buf[main_off..main_off + mpay.len()].copy_from_slice(mpay);
            payload_cursor += mpay.len();

            let sec_off = payload_cursor;
            buf[sec_off..sec_off + spay.len()].copy_from_slice(spay);
            payload_cursor += spay.len();

            buf[hdr_off + 24..hdr_off + 32]
                .copy_from_slice(&(main_off as u64).to_le_bytes());
            buf[hdr_off + 32..hdr_off + 40]
                .copy_from_slice(&(sec_off as u64).to_le_bytes());
        }

        buf
    }

    /// Encode one variant-delta record into bytes: `(u16 output_idx, u16 N,
    /// N x u32 packed)`.
    fn encode_record(output_idx: u16, packed: &[u32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + packed.len() * 4);
        out.extend_from_slice(&output_idx.to_le_bytes());
        out.extend_from_slice(&(packed.len() as u16).to_le_bytes());
        for &p in packed {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out
    }

    #[test]
    fn parses_synthetic_main_payload() {
        // One variant with one main record (output_idx=5, packed=[0x12345678]),
        // zero secondary records.
        let main_payload = encode_record(5, &[0x12345678]);
        let buf = build_dna_with_variants(&[(1, 0, 0.5, 1.5, main_payload, Vec::new())]);

        let dna = DnaFile::read(&buf).expect("synthetic .dna parses");
        let mut iter = dna.variant_main_deltas(0).expect("main iter ok");
        let rec = iter.next().expect("one record").expect("ok");
        assert_eq!(rec.output_idx, 5);
        assert_eq!(rec.packed_bytes.len(), 4);
        assert_eq!(
            u32::from_le_bytes(rec.packed_bytes.try_into().unwrap()),
            0x12345678,
        );
        assert!(iter.next().is_none(), "iterator exhausted after one record");
    }

    #[test]
    fn walks_multiple_records() {
        // Three records with N = 2, 1, 3 respectively.
        let mut main_payload = encode_record(10, &[0xAAAAAAAA, 0xBBBBBBBB]);
        main_payload.extend_from_slice(&encode_record(20, &[0xCCCCCCCC]));
        main_payload.extend_from_slice(&encode_record(
            30,
            &[0xDDDDDDDD, 0xEEEEEEEE, 0xFFFFFFFF],
        ));
        let buf = build_dna_with_variants(&[(3, 0, 1.0, 1.0, main_payload, Vec::new())]);

        let dna = DnaFile::read(&buf).expect("parses");
        let recs: Vec<_> = dna
            .variant_main_deltas(0)
            .expect("iter ok")
            .collect::<Result<Vec<_>, _>>()
            .expect("all records ok");
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].output_idx, 10);
        assert_eq!(recs[0].packed_bytes.len(), 8);
        assert_eq!(recs[1].output_idx, 20);
        assert_eq!(recs[1].packed_bytes.len(), 4);
        assert_eq!(recs[2].output_idx, 30);
        assert_eq!(recs[2].packed_bytes.len(), 12);
    }

    #[test]
    fn variant_out_of_range_errors() {
        // 2 variants total.
        let buf = build_dna_with_variants(&[
            (0, 0, 1.0, 1.0, Vec::new(), Vec::new()),
            (0, 0, 1.0, 1.0, Vec::new(), Vec::new()),
        ]);
        let dna = DnaFile::read(&buf).expect("parses");
        let err = dna.variant_main_deltas(5).unwrap_err();
        match err {
            DnaError::VariantOutOfRange {
                variant_idx,
                num_variants,
            } => {
                assert_eq!(variant_idx, 5);
                assert_eq!(num_variants, 2);
            }
            other => panic!("expected VariantOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn truncated_payload_errors() {
        // Declare 5 main records but only place 2 records worth of bytes;
        // the iterator yields 2 Ok then VariantDeltaTruncated.
        let mut main_payload = encode_record(1, &[0x11111111]);
        main_payload.extend_from_slice(&encode_record(2, &[0x22222222]));
        // (Declared count is 5; only 2 records of bytes follow.)
        let buf =
            build_dna_with_variants(&[(5, 0, 1.0, 1.0, main_payload, Vec::new())]);

        let dna = DnaFile::read(&buf).expect("parses");
        let mut iter = dna.variant_main_deltas(0).expect("iter ok");
        assert!(iter.next().expect("rec 0").is_ok());
        assert!(iter.next().expect("rec 1").is_ok());
        let err = iter.next().expect("trunc err").unwrap_err();
        match err {
            DnaError::VariantDeltaTruncated {
                variant_idx,
                side,
                expected_records,
                got_records,
                ..
            } => {
                assert_eq!(variant_idx, 0);
                assert_eq!(side, VariantDeltaSide::Main);
                assert_eq!(expected_records, 5);
                assert_eq!(got_records, 2);
            }
            other => panic!("expected VariantDeltaTruncated, got {other:?}"),
        }
        // Subsequent calls return None — once truncated, the iterator is done.
        assert!(iter.next().is_none());
    }

    #[test]
    fn secondary_iterator_independent_from_main() {
        // Main and secondary payloads differ; verify each iter sees its own.
        let main_payload = encode_record(100, &[0xAAAA_AAAA]);
        let sec_payload = encode_record(200, &[0xBBBB_BBBB]);
        let buf =
            build_dna_with_variants(&[(1, 1, 1.0, 1.0, main_payload, sec_payload)]);

        let dna = DnaFile::read(&buf).expect("parses");
        let main_rec = dna
            .variant_main_deltas(0)
            .expect("main iter")
            .next()
            .expect("rec")
            .expect("ok");
        assert_eq!(main_rec.output_idx, 100);
        assert_eq!(
            u32::from_le_bytes(main_rec.packed_bytes.try_into().unwrap()),
            0xAAAA_AAAA
        );

        let sec_rec = dna
            .variant_secondary_deltas(0)
            .expect("sec iter")
            .next()
            .expect("rec")
            .expect("ok");
        assert_eq!(sec_rec.output_idx, 200);
        assert_eq!(
            u32::from_le_bytes(sec_rec.packed_bytes.try_into().unwrap()),
            0xBBBB_BBBB
        );
    }

    #[test]
    fn record_with_zero_packed_entries() {
        // A variant-delta record with N = 0 produces a valid 4-byte header
        // and zero payload bytes. The iterator must handle this without
        // advancing the cursor into a payload region, and `packed_bytes`
        // should be empty.
        let mut main_payload = encode_record(42, &[]);
        // Add a second record so we verify the cursor advanced correctly
        // past the zero-payload entry into the next record.
        main_payload.extend_from_slice(&encode_record(99, &[0xCAFEBABE]));
        let buf = build_dna_with_variants(&[(2, 0, 1.0, 1.0, main_payload, Vec::new())]);

        let dna = DnaFile::read(&buf).expect("parses");
        let recs: Vec<_> = dna
            .variant_main_deltas(0)
            .expect("iter ok")
            .collect::<Result<Vec<_>, _>>()
            .expect("all records ok");

        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].output_idx, 42);
        assert!(
            recs[0].packed_bytes.is_empty(),
            "zero-N record must have empty packed_bytes, got len={}",
            recs[0].packed_bytes.len()
        );
        assert_eq!(recs[1].output_idx, 99);
        assert_eq!(recs[1].packed_bytes.len(), 4);
        assert_eq!(
            u32::from_le_bytes(recs[1].packed_bytes.try_into().unwrap()),
            0xCAFEBABE,
        );
    }
}
