use serde::{Deserialize, Serialize};
use starbreaker_common::{CigGuid, ParseError, SpanReader, SpanWriter};

use crate::dna::Dna;
use crate::itemport::ItemPort;
use crate::material::MaterialDefinition;
use crate::read_helpers::expect_u32;

/// Top-level parsed CHF character data.
///
/// The game's serialization API also includes `intParams` (per sub-material) and `Decals`
/// (v8+) sections, but these are only present in the GRPC network format — the local binary
/// CHF file does NOT contain them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChfData {
    /// Always 2.
    pub female_version: u32,
    /// 7 or 8. Game accepts versions 2-8 (`version - 2 < 7`).
    pub male_version: u32,
    /// The body type GUID. Game field name: `"modelTag"`.
    pub body_type_id: CigGuid,
    /// Voice variant GUID. Game field name: `"voiceTag"`.
    /// Always empty (all zeros) in current files, but semantically the voice selection.
    pub voice_tag: CigGuid,
    /// DNA morph blend data. Game field name: `"dnaByteArray"`.
    pub dna: Dna,
    /// Total number of item ports in the tree (stored in file, recomputed on write).
    pub total_itemport_count: u64,
    /// Root item port tree. Game field name: `"Loadout"`.
    pub itemport: ItemPort,
    /// Material definitions. Game field name: `"Materials"`.
    pub materials: Vec<MaterialDefinition>,
}

impl ChfData {
    /// Parse decompressed CHF binary data.
    pub fn read(data: &[u8]) -> Result<Self, ParseError> {
        let mut reader = SpanReader::new(data);

        let female_version = expect_u32(&mut reader, 2)?;
        let male_version = {
            let offset = reader.position();
            let val = reader.read_u32()?;
            if val != 7 && val != 8 {
                return Err(ParseError::UnexpectedValue {
                    offset,
                    expected: "[7, 8]".to_string(),
                    actual: format!("{val}"),
                });
            }
            val
        };
        let body_type_id = crate::read_helpers::read_guid(&mut reader)?;
        let voice_tag = crate::read_helpers::read_guid(&mut reader)?;

        let dna = Dna::read(&mut reader)?;

        let total_itemport_count = reader.read_u64()?;
        let itemport = ItemPort::read(&mut reader)?;

        // Group boundary marker (u32 = 5). The CHF data section uses CryEngine's binary
        // save game serialization format (confirmed: s_saveGame.format = "0: Binary").
        // The value 5 consistently appears at group boundaries — before the Materials
        // section, before each sub-material, and between sub-materials. It is a structural
        // marker in the binary serialization protocol (likely a BeginGroup type tag).
        expect_u32(&mut reader, 5)?;

        let mut materials = Vec::new();
        // Read materials until we run out of data or hit trailing zeros.
        // A valid material starts with a non-zero NameHash (4 bytes).
        while reader.remaining() >= 4 {
            let pos = reader.position();
            let next = reader.read_u32()?;
            reader.set_position(pos);
            if next == 0 {
                break; // trailing padding, not a material
            }
            materials.push(MaterialDefinition::read(&mut reader)?);
        }

        Ok(ChfData {
            female_version,
            male_version,
            body_type_id,
            voice_tag,
            dna,
            total_itemport_count,
            itemport,
            materials,
        })
    }

    /// Serialize ChfData back to binary bytes.
    pub fn write(&self) -> Vec<u8> {
        let mut writer = SpanWriter::new();

        writer.write_u32(self.female_version);
        writer.write_u32(self.male_version);
        writer.write_val(&self.body_type_id);
        writer.write_val(&self.voice_tag);

        self.dna.write(&mut writer);

        // Recompute total_itemport_count from the tree
        writer.write_u64(self.itemport.total_count());
        self.itemport.write(&mut writer);

        // Materials marker
        writer.write_u32(5);

        for (i, mat) in self.materials.iter().enumerate() {
            let is_last = i == self.materials.len() - 1;
            mat.write(&mut writer, is_last);
        }

        // v8 files have 4 trailing zero bytes after the last material
        if self.male_version >= 8 {
            writer.write_u32(0);
        }

        writer.into_inner()
    }
}
