use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use starbreaker_common::{NameHash, ParseError, SpanReader, SpanWriter};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::read_helpers::{expect_name_hash, expect_u32, read_name_hash};

// ─── FacePart ────────────────────────────────────────────────────────────────

/// Enumeration of face parts in the DNA blend system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FacePart {
    EyebrowLeft = 0,
    EyebrowRight = 1,
    EyeLeft = 2,
    EyeRight = 3,
    Nose = 4,
    EarLeft = 5,
    EarRight = 6,
    CheekLeft = 7,
    CheekRight = 8,
    Mouth = 9,
    Jaw = 10,
    Crown = 11,
    Neck = 12, // v8 only
}

impl FacePart {
    fn from_index(index: u16) -> Option<Self> {
        match index {
            0 => Some(FacePart::EyebrowLeft),
            1 => Some(FacePart::EyebrowRight),
            2 => Some(FacePart::EyeLeft),
            3 => Some(FacePart::EyeRight),
            4 => Some(FacePart::Nose),
            5 => Some(FacePart::EarLeft),
            6 => Some(FacePart::EarRight),
            7 => Some(FacePart::CheekLeft),
            8 => Some(FacePart::CheekRight),
            9 => Some(FacePart::Mouth),
            10 => Some(FacePart::Jaw),
            11 => Some(FacePart::Crown),
            12 => Some(FacePart::Neck),
            _ => None,
        }
    }
}

// ─── DnaBlend ────────────────────────────────────────────────────────────────

/// A single DNA blend entry: a value (0..=65535 scaled percentage) and a head ID.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, FromBytes, IntoBytes, Immutable, KnownLayout,
)]
#[repr(C, packed)]
pub struct DnaBlend {
    pub value: u16,
    pub head_id: u16,
}

impl DnaBlend {
    /// Read a DnaBlend using alignment-safe primitive reads.
    fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let value = reader.read_u16()?;
        let head_id = reader.read_u16()?;
        Ok(DnaBlend { value, head_id })
    }
}

// ─── Dna ─────────────────────────────────────────────────────────────────────

/// Parsed DNA data containing face morph blend information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dna {
    /// Raw DNA bytes, preserved for exact round-tripping.
    #[serde(with = "hex_bytes")]
    pub raw_bytes: Vec<u8>,
    /// CRC32C hash identifying the gender variant (e.g. male/female head mesh).
    pub gender_hash: NameHash,
    /// CRC32C hash identifying the specific variant.
    pub variant_hash: NameHash,
    /// Number of face parts in the blend matrix.
    pub part_count: u16,
    /// Number of blends per part (always 4 in practice).
    pub blends_per_part: u16,
    /// Unknown header field, preserved for round-tripping.
    pub header_unknown: u16,
    /// Maximum head ID referenced by any blend.
    pub max_head_id: u16,
    /// Per-face-part blend data (4 blends per part).
    pub face_parts: BTreeMap<FacePart, [DnaBlend; 4]>,
}

impl Dna {
    /// Read DNA from the top-level reader. Reads a u64 size prefix then the raw bytes.
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let dna_size = reader.read_u64()? as usize;
        let dna_bytes = reader.read_bytes(dna_size)?;
        Self::read_raw(dna_bytes)
    }

    /// Parse raw DNA bytes (without the size prefix).
    pub fn read_raw(bytes: &[u8]) -> Result<Self, ParseError> {
        let raw_bytes = bytes.to_vec();
        let mut r = SpanReader::new(bytes);

        // Validate "dna matrix 1.0" hash
        expect_name_hash(&mut r, NameHash::from_string("dna matrix 1.0"))?;

        // Gender hash and variant hash -- read without validation
        let gender_hash = read_name_hash(&mut r)?;
        let variant_hash = read_name_hash(&mut r)?;

        // Zero u32
        expect_u32(&mut r, 0)?;

        // DNA header: part_count, blends_per_part (always 4), unknown, max_head_id
        let part_count = r.read_u16()?;
        let blends_per_part = r.read_u16()?;
        let header_unknown = r.read_u16()?;
        let max_head_id = r.read_u16()?;

        // Read interleaved DnaBlend entries: part_count * 4 entries total
        let total_blends = part_count as usize * 4;
        let mut face_parts: BTreeMap<FacePart, [DnaBlend; 4]> = BTreeMap::new();

        // Initialize all parts
        for i in 0..part_count {
            if let Some(part) = FacePart::from_index(i) {
                face_parts.insert(
                    part,
                    [DnaBlend {
                        value: 0,
                        head_id: 0,
                    }; 4],
                );
            }
        }

        // Read in interleaved order: part_index = i % part_count, blend_index = i / part_count
        for i in 0..total_blends {
            let blend = DnaBlend::read(&mut r)?;
            let part_index = (i % part_count as usize) as u16;
            let blend_index = i / part_count as usize;

            if let Some(part) = FacePart::from_index(part_index)
                && let Some(blends) = face_parts.get_mut(&part)
            {
                blends[blend_index] = blend;
            }
        }

        Ok(Dna {
            raw_bytes,
            gender_hash,
            variant_hash,
            part_count,
            blends_per_part,
            header_unknown,
            max_head_id,
            face_parts,
        })
    }

    /// Reconstruct raw DNA bytes from the structured fields.
    pub fn to_raw_bytes(&self) -> Vec<u8> {
        let mut w = SpanWriter::new();
        w.write_val(&NameHash::from_string("dna matrix 1.0"));
        w.write_val(&self.gender_hash);
        w.write_val(&self.variant_hash);
        w.write_u32(0);
        w.write_u16(self.part_count);
        w.write_u16(self.blends_per_part);
        w.write_u16(self.header_unknown);
        w.write_u16(self.max_head_id);

        // Interleaved: for each blend_index, for each part in BTreeMap order
        for blend_idx in 0..4usize {
            for (_part, blends) in &self.face_parts {
                w.write_u16(blends[blend_idx].value);
                w.write_u16(blends[blend_idx].head_id);
            }
        }
        w.into_inner()
    }

    /// Write DNA, reconstructing bytes from structured fields.
    pub fn write(&self, writer: &mut SpanWriter) {
        let bytes = self.to_raw_bytes();
        writer.write_u64(bytes.len() as u64);
        writer.write_bytes(&bytes);
    }
}

// ─── hex_bytes serde module ──────────────────────────────────────────────────

/// Serde module that serializes `Vec<u8>` as a hex string and deserializes back.
mod hex_bytes {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        serializer.serialize_str(&hex)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.len() % 2 != 0 {
            return Err(serde::de::Error::custom("hex string has odd length"));
        }
        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16)
                    .map_err(|e| serde::de::Error::custom(format!("invalid hex: {e}")))
            })
            .collect()
    }
}
