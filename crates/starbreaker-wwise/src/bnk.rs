use std::collections::HashMap;

use starbreaker_common::SpanReader;

use crate::bkhd::BankHeader;
use crate::didx::{self, DataIndexEntry};
use crate::error::BnkError;
use crate::hirc::HircSection;
use crate::section::{RawSection, SectionTag};
use crate::stid;

/// A parsed BNK soundbank file.
pub struct BnkFile<'a> {
    data: &'a [u8],
    pub header: BankHeader,
    pub data_index: Vec<DataIndexEntry>,
    /// Offset and length of the DATA section within the original buffer.
    data_section_offset: usize,
    data_section_size: usize,
    pub hirc: Option<HircSection>,
    pub string_ids: HashMap<u32, String>,
    /// Raw sections we don't deeply parse yet (INIT, STMG, ENVS, PLAT).
    pub raw_sections: Vec<RawSection>,
}

impl<'a> BnkFile<'a> {
    /// Parse a BNK file from raw bytes.
    pub fn parse(data: &'a [u8]) -> Result<Self, BnkError> {
        let mut reader = SpanReader::new(data);

        let mut header: Option<BankHeader> = None;
        let mut data_index: Vec<DataIndexEntry> = Vec::new();
        let mut data_section_offset: usize = 0;
        let mut data_section_size: usize = 0;
        let mut hirc: Option<HircSection> = None;
        let mut string_ids: HashMap<u32, String> = HashMap::new();
        let mut raw_sections: Vec<RawSection> = Vec::new();

        while reader.remaining() >= 8 {
            let tag_val = reader.read_u32()?;
            let section_size = reader.read_u32()?;
            let section_offset = reader.position();
            let tag = SectionTag::from_u32(tag_val);

            let section_data = reader.read_bytes(section_size as usize)?;

            match tag {
                SectionTag::Bkhd => {
                    header = Some(BankHeader::parse(section_data, section_size)?);
                }
                SectionTag::Didx => {
                    data_index = didx::parse_didx(section_data)?;
                }
                SectionTag::Data => {
                    data_section_offset = section_offset;
                    data_section_size = section_size as usize;
                }
                SectionTag::Hirc => {
                    hirc = Some(HircSection::parse(section_data)?);
                }
                SectionTag::Stid => {
                    string_ids = stid::parse_stid(section_data)?;
                }
                _ => {
                    raw_sections.push(RawSection {
                        tag,
                        data_offset: section_offset,
                        data_size: section_size as usize,
                    });
                }
            }
        }

        let header = header.ok_or(BnkError::MissingSection {
            tag: "BKHD".into(),
        })?;

        Ok(BnkFile {
            data,
            header,
            data_index,
            data_section_offset,
            data_section_size,
            hirc,
            string_ids,
            raw_sections,
        })
    }

    /// Get the raw WEM bytes for a DIDX entry.
    pub fn wem_data(&self, entry: &DataIndexEntry) -> Result<&'a [u8], BnkError> {
        let start = self.data_section_offset + entry.offset as usize;
        let end = start + entry.size as usize;
        if end > self.data_section_offset + self.data_section_size {
            return Err(BnkError::DataOverflow {
                offset: entry.offset,
                size: entry.size,
                data_len: self.data_section_size,
            });
        }
        Ok(&self.data[start..end])
    }

    /// Find a WEM entry by ID.
    pub fn wem_entry(&self, id: u32) -> Option<&DataIndexEntry> {
        self.data_index.iter().find(|e| e.id == id)
    }

    /// Get WEM data by ID.
    pub fn wem_data_by_id(&self, id: u32) -> Result<&'a [u8], BnkError> {
        let entry = self.wem_entry(id).ok_or(BnkError::WemNotFound { id })?;
        self.wem_data(entry)
    }

    /// Number of embedded WEM files.
    pub fn wem_count(&self) -> usize {
        self.data_index.len()
    }

    /// Whether this bank has a DATA section with embedded audio.
    pub fn has_embedded_audio(&self) -> bool {
        self.data_section_size > 0 && !self.data_index.is_empty()
    }

    /// All section tags present in this bank (for info display).
    pub fn section_tags(&self) -> Vec<SectionTag> {
        let mut tags = vec![SectionTag::Bkhd];
        if !self.data_index.is_empty() {
            tags.push(SectionTag::Didx);
        }
        if self.data_section_size > 0 {
            tags.push(SectionTag::Data);
        }
        if self.hirc.is_some() {
            tags.push(SectionTag::Hirc);
        }
        if !self.string_ids.is_empty() {
            tags.push(SectionTag::Stid);
        }
        for s in &self.raw_sections {
            tags.push(s.tag);
        }
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal BNK with BKHD + DIDX + DATA sections.
    fn minimal_bnk() -> Vec<u8> {
        let mut buf = Vec::new();

        // BKHD section
        buf.extend_from_slice(&0x44484B42u32.to_le_bytes()); // "BKHD"
        buf.extend_from_slice(&8u32.to_le_bytes()); // section size
        buf.extend_from_slice(&134u32.to_le_bytes()); // version
        buf.extend_from_slice(&1001u32.to_le_bytes()); // bank_id

        // DIDX section — one entry
        buf.extend_from_slice(&0x58444944u32.to_le_bytes()); // "DIDX"
        buf.extend_from_slice(&12u32.to_le_bytes()); // section size (1 entry × 12 bytes)
        buf.extend_from_slice(&42u32.to_le_bytes()); // wem id
        buf.extend_from_slice(&0u32.to_le_bytes()); // offset in DATA
        buf.extend_from_slice(&4u32.to_le_bytes()); // size

        // DATA section — 4 bytes of dummy audio
        buf.extend_from_slice(&0x41544144u32.to_le_bytes()); // "DATA"
        buf.extend_from_slice(&4u32.to_le_bytes()); // section size
        buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        buf
    }

    /// Build a minimal BNK with only BKHD (metadata-only bank).
    fn header_only_bnk() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x44484B42u32.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&134u32.to_le_bytes());
        buf.extend_from_slice(&2002u32.to_le_bytes());
        buf
    }

    #[test]
    fn test_parse_minimal_bnk() {
        let data = minimal_bnk();
        let bnk = BnkFile::parse(&data).unwrap();
        assert_eq!(bnk.header.version, 134);
        assert_eq!(bnk.header.bank_id, 1001);
        assert_eq!(bnk.wem_count(), 1);
        assert!(bnk.has_embedded_audio());
    }

    #[test]
    fn test_wem_extraction() {
        let data = minimal_bnk();
        let bnk = BnkFile::parse(&data).unwrap();
        let wem = bnk.wem_data_by_id(42).unwrap();
        assert_eq!(wem, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_wem_not_found() {
        let data = minimal_bnk();
        let bnk = BnkFile::parse(&data).unwrap();
        let err = bnk.wem_data_by_id(999).unwrap_err();
        assert!(matches!(err, BnkError::WemNotFound { id: 999 }));
    }

    #[test]
    fn test_header_only_bnk() {
        let data = header_only_bnk();
        let bnk = BnkFile::parse(&data).unwrap();
        assert_eq!(bnk.header.bank_id, 2002);
        assert_eq!(bnk.wem_count(), 0);
        assert!(!bnk.has_embedded_audio());
    }

    #[test]
    fn test_section_tags() {
        let data = minimal_bnk();
        let bnk = BnkFile::parse(&data).unwrap();
        let tags = bnk.section_tags();
        assert!(tags.contains(&SectionTag::Bkhd));
        assert!(tags.contains(&SectionTag::Didx));
        assert!(tags.contains(&SectionTag::Data));
    }
}
