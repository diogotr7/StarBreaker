use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

// Re-export CigGuid and GuidParseError from starbreaker-common.
pub use starbreaker_common::{CigGuid, GuidParseError};

// ─── StringId ───────────────────────────────────────────────────────────────

/// An offset into string table 1 (filename strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct StringId(pub i32);

// ─── StringId2 ──────────────────────────────────────────────────────────────

/// An offset into string table 2 (name strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct StringId2(pub i32);

// ─── StructDefinition ───────────────────────────────────────────────────────

/// Describes a struct type (16 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct StructDefinition {
    pub name_offset: StringId2,
    pub parent_type_index: i32,
    pub attribute_count: u16,
    pub first_attribute_index: u16,
    pub struct_size: u32,
}

// ─── PropertyDefinition ─────────────────────────────────────────────────────

/// Describes a property on a struct (12 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct PropertyDefinition {
    pub name_offset: StringId2,
    pub struct_index: u16,
    pub data_type: u16,
    pub conversion_type: u16,
    pub _padding: u16,
}

// ─── EnumDefinition ─────────────────────────────────────────────────────────

/// Describes an enum type (8 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct EnumDefinition {
    pub name_offset: StringId2,
    pub value_count: u16,
    pub first_value_index: u16,
}

// ─── DataMapping ────────────────────────────────────────────────────────────

/// Maps struct counts to their starting index in the instance arrays (8 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct DataMapping {
    pub struct_count: u32,
    pub struct_index: i32,
}

// ─── Record ─────────────────────────────────────────────────────────────────

/// A top-level DataCore record (32 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct Record {
    pub name_offset: StringId2,
    pub file_name_offset: StringId,
    pub struct_index: i32,
    pub id: CigGuid,
    pub instance_index: u16,
    pub struct_size: u16,
}

// ─── StructId ───────────────────────────────────────────────────────────────

/// Opaque identifier for a DataCore struct type.
///
/// Consumers receive these from [`Database::struct_id`] or [`Record::struct_id`]
/// and pass them back to query methods. The inner value is not accessible
/// outside the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(pub(crate) i32);

impl Record {
    /// The struct type of this record, as an opaque identifier.
    pub fn struct_id(&self) -> StructId {
        StructId(self.struct_index)
    }
}

// ─── Pointer ────────────────────────────────────────────────────────────────

/// A DataCore pointer (8 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct Pointer {
    pub struct_index: i32,
    pub instance_index: i32,
}

impl Pointer {
    pub fn is_null(&self) -> bool {
        self.struct_index == -1 && self.instance_index == -1
    }
}

// ─── Reference ──────────────────────────────────────────────────────────────

/// A DataCore cross-record reference (20 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct Reference {
    pub instance_index: i32,
    pub record_id: CigGuid,
}

impl Reference {
    pub fn is_null(&self) -> bool {
        self.instance_index == 0 && self.record_id.is_empty()
    }
}

// ─── Size assertions ─────────────────────────────────────────────────────────

const _: () = {
    assert!(size_of::<StringId>() == 4);
    assert!(size_of::<StringId2>() == 4);
    assert!(size_of::<StructDefinition>() == 16);
    assert!(size_of::<PropertyDefinition>() == 12);
    assert!(size_of::<EnumDefinition>() == 8);
    assert!(size_of::<DataMapping>() == 8);
    assert!(size_of::<Record>() == 32);
    assert!(size_of::<Pointer>() == 8);
    assert!(size_of::<Reference>() == 20);
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn guid_round_trip() {
        let original = CigGuid::from_bytes([
            0x78, 0x56, 0x34, 0x12, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0x9A, 0xBC,
            0xDE, 0xF0,
        ]);
        let s = original.to_string();
        let parsed = CigGuid::from_str(&s).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn guid_parse_invalid() {
        assert!(CigGuid::from_str("not-a-guid").is_err());
        assert!(CigGuid::from_str("0123456789abcdef0123456789abcdefxx").is_err());
    }

    #[test]
    fn guid_empty_round_trip() {
        let empty = CigGuid::EMPTY;
        let s = empty.to_string();
        let parsed = CigGuid::from_str(&s).unwrap();
        assert_eq!(empty, parsed);
    }
}
