use std::collections::HashSet;

use rustc_hash::FxHashMap;
use std::io::Read;

use zerocopy::{FromBytes, Immutable, KnownLayout, Ref};

use starbreaker_common::ParseError as CommonParseError;

use crate::enums::{ConversionType, DataType};
use crate::error::ParseError;
use crate::reader::SpanReader;
use crate::types::{
    CigGuid, DataMapping, EnumDefinition, Pointer, PropertyDefinition, Record, Reference, StringId,
    StringId2, StructDefinition,
};

/// Raw DCB file header (120 bytes).
#[derive(Clone, Copy, FromBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
struct RawDCBHeader {
    magic: u32,
    version: u32,
    _reserved1: u32,
    _reserved2: u32,
    struct_definition_count: i32,
    property_definition_count: i32,
    enum_definition_count: i32,
    data_mapping_count: i32,
    record_definition_count: i32,
    boolean_value_count: i32,
    int8_value_count: i32,
    int16_value_count: i32,
    int32_value_count: i32,
    int64_value_count: i32,
    uint8_value_count: i32,
    uint16_value_count: i32,
    uint32_value_count: i32,
    uint64_value_count: i32,
    single_value_count: i32,
    double_value_count: i32,
    guid_value_count: i32,
    string_id_value_count: i32,
    locale_value_count: i32,
    enum_value_count: i32,
    strong_value_count: i32,
    weak_value_count: i32,
    reference_value_count: i32,
    enum_option_count: i32,
    text_length: u32,
    text_length2: u32,
}

const HEADER_SIZE: usize = size_of::<RawDCBHeader>();
const _: () = assert!(HEADER_SIZE == 120);

/// Parsed DataCore database (borrows input data). Zero-copy where possible.
///
/// Definition tables (structs, properties, enums, records, etc.) are reinterpreted
/// directly from the backing data via `zerocopy`. These types are all `repr(C, packed)`
/// with alignment 1, so no alignment issues arise.
///
/// Value arrays for primitives (i8..u64, f32, f64) are stored as raw `&[u8]` slices
/// to avoid alignment problems on arbitrarily-packed binary data. Access individual
/// elements through the provided accessor methods.
pub struct Database<'a> {
    struct_defs: &'a [StructDefinition],
    property_defs: &'a [PropertyDefinition],
    enum_defs: &'a [EnumDefinition],
    data_mappings: &'a [DataMapping],
    records: &'a [Record],

    // Value arrays stored as raw bytes to avoid alignment issues.
    // Individual elements are read via from_le_bytes.
    int8_values_raw: &'a [u8],
    int16_values_raw: &'a [u8],
    int32_values_raw: &'a [u8],
    int64_values_raw: &'a [u8],
    uint8_values_raw: &'a [u8],
    uint16_values_raw: &'a [u8],
    uint32_values_raw: &'a [u8],
    uint64_values_raw: &'a [u8],
    bool_values_raw: &'a [u8],
    single_values_raw: &'a [u8],
    double_values_raw: &'a [u8],

    // These packed types have alignment 1, so zero-copy slicing is safe.
    pub guid_values: &'a [CigGuid],
    pub string_id_values: &'a [StringId],
    pub locale_values: &'a [StringId],
    pub enum_values: &'a [StringId],
    pub strong_values: &'a [Pointer],
    pub weak_values: &'a [Pointer],
    pub reference_values: &'a [Reference],
    enum_options_all: &'a [StringId2],

    string_table1: &'a [u8],
    string_table2: &'a [u8],
    instance_data: &'a [u8],

    // Counts for value arrays (needed for len() accessors)
    int8_count: usize,
    int16_count: usize,
    int32_count: usize,
    int64_count: usize,
    uint8_count: usize,
    uint16_count: usize,
    uint32_count: usize,
    uint64_count: usize,
    bool_count: usize,
    single_count: usize,
    double_count: usize,

    /// Per-struct-definition offset into instance_data.
    instance_offsets: Vec<usize>,

    /// Cached flattened property indices per struct type.
    /// Each entry contains the property definition indices (into property_defs)
    /// for that struct, with parent properties first.
    cached_properties: Vec<Box<[u16]>>,

    /// Pre-computed cumulative byte offsets per property position per struct.
    /// `cached_offsets[si][p]` = byte offset from struct start to property at position `p`.
    cached_offsets: Vec<Box<[usize]>>,

    /// Maps record ID -> index in `records`.
    record_map: FxHashMap<CigGuid, usize>,

    /// Maps struct name (from string table 2) → struct index.
    struct_name_map: FxHashMap<&'a str, i32>,

    /// Set of record IDs that are "main" records (last record per unique file_name_offset).
    main_record_ids: HashSet<CigGuid>,

    /// Per struct type: true if the struct or any transitively reachable struct
    /// contains a WeakPointer property. Used to skip prescan when unnecessary.
    has_weak_pointers: Vec<bool>,

    /// Pre-built index: struct_index → list of record indices (into `records`).
    records_by_struct: Vec<Box<[usize]>>,
}

impl std::fmt::Debug for Database<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("structs", &self.struct_defs.len())
            .field("properties", &self.property_defs.len())
            .field("enums", &self.enum_defs.len())
            .field("records", &self.records.len())
            .finish()
    }
}

/// Helper: reinterpret a byte range as a typed slice via zerocopy (alignment-1 types only).
fn slice_from_bytes<T: zerocopy::FromBytes + zerocopy::Immutable + zerocopy::KnownLayout>(
    data: &[u8],
    offset: usize,
    count: usize,
) -> Result<(&[T], usize), ParseError> {
    let byte_len = count * size_of::<T>();
    let end = offset + byte_len;
    if end > data.len() {
        return Err(CommonParseError::Truncated {
            offset,
            need: end - offset,
            have: data.len() - offset,
        }
        .into());
    }
    let slice = &data[offset..end];
    match Ref::<_, [T]>::from_bytes(slice) {
        Ok(r) => Ok((Ref::into_ref(r), end)),
        Err(_) => {
            Err(CommonParseError::InvalidLayout(std::any::type_name::<T>().to_string()).into())
        }
    }
}

/// Helper: extract a raw byte range for a value array of `count` elements of size `elem_size`.
fn raw_range(
    data: &[u8],
    offset: usize,
    count: usize,
    elem_size: usize,
) -> Result<(&[u8], usize), ParseError> {
    let byte_len = count * elem_size;
    let end = offset + byte_len;
    if end > data.len() {
        return Err(CommonParseError::Truncated {
            offset,
            need: end - offset,
            have: data.len() - offset,
        }
        .into());
    }
    Ok((&data[offset..end], end))
}

impl<'a> Database<'a> {
    /// Parse a DCB v6 binary blob into a zero-copy `Database`.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, ParseError> {
        // 1. Validate minimum header size
        if data.len() < HEADER_SIZE {
            return Err(CommonParseError::Truncated {
                offset: 0,
                need: HEADER_SIZE,
                have: data.len(),
            }
            .into());
        }

        // 2. Read header
        let mut reader = SpanReader::new(data);
        let h = reader.read_type::<RawDCBHeader>()?;

        // 3. Validate version
        if h.version != 6 {
            return Err(ParseError::UnsupportedVersion(h.version));
        }

        let struct_definition_count = h.struct_definition_count as usize;
        let property_definition_count = h.property_definition_count as usize;
        let enum_definition_count = h.enum_definition_count as usize;
        let data_mapping_count = h.data_mapping_count as usize;
        let record_definition_count = h.record_definition_count as usize;
        let boolean_value_count = h.boolean_value_count as usize;
        let int8_value_count = h.int8_value_count as usize;
        let int16_value_count = h.int16_value_count as usize;
        let int32_value_count = h.int32_value_count as usize;
        let int64_value_count = h.int64_value_count as usize;
        let uint8_value_count = h.uint8_value_count as usize;
        let uint16_value_count = h.uint16_value_count as usize;
        let uint32_value_count = h.uint32_value_count as usize;
        let uint64_value_count = h.uint64_value_count as usize;
        let single_value_count = h.single_value_count as usize;
        let double_value_count = h.double_value_count as usize;
        let guid_value_count = h.guid_value_count as usize;
        let string_id_value_count = h.string_id_value_count as usize;
        let locale_value_count = h.locale_value_count as usize;
        let enum_value_count = h.enum_value_count as usize;
        let strong_value_count = h.strong_value_count as usize;
        let weak_value_count = h.weak_value_count as usize;
        let reference_value_count = h.reference_value_count as usize;
        let enum_option_count = h.enum_option_count as usize;
        let text_length = h.text_length as usize;
        let text_length2 = h.text_length2 as usize;

        // 4. Parse definition sections (all packed/alignment-1)
        let offset = HEADER_SIZE;
        let (struct_defs, offset) =
            slice_from_bytes::<StructDefinition>(data, offset, struct_definition_count)?;
        let (property_defs, offset) =
            slice_from_bytes::<PropertyDefinition>(data, offset, property_definition_count)?;
        let (enum_defs, offset) =
            slice_from_bytes::<EnumDefinition>(data, offset, enum_definition_count)?;
        let (data_mappings, offset) =
            slice_from_bytes::<DataMapping>(data, offset, data_mapping_count)?;
        let (records, offset) = slice_from_bytes::<Record>(data, offset, record_definition_count)?;

        // 5. Value arrays — stored as raw bytes to avoid alignment issues
        let (int8_values_raw, offset) = raw_range(data, offset, int8_value_count, 1)?;
        let (int16_values_raw, offset) = raw_range(data, offset, int16_value_count, 2)?;
        let (int32_values_raw, offset) = raw_range(data, offset, int32_value_count, 4)?;
        let (int64_values_raw, offset) = raw_range(data, offset, int64_value_count, 8)?;
        let (uint8_values_raw, offset) = raw_range(data, offset, uint8_value_count, 1)?;
        let (uint16_values_raw, offset) = raw_range(data, offset, uint16_value_count, 2)?;
        let (uint32_values_raw, offset) = raw_range(data, offset, uint32_value_count, 4)?;
        let (uint64_values_raw, offset) = raw_range(data, offset, uint64_value_count, 8)?;

        // booleans come AFTER all integers
        let (bool_values_raw, offset) = raw_range(data, offset, boolean_value_count, 1)?;

        let (single_values_raw, offset) = raw_range(data, offset, single_value_count, 4)?;
        let (double_values_raw, offset) = raw_range(data, offset, double_value_count, 8)?;

        // Packed struct types (alignment 1) — safe to zero-copy
        let (guid_values, offset) = slice_from_bytes::<CigGuid>(data, offset, guid_value_count)?;
        let (string_id_values, offset) =
            slice_from_bytes::<StringId>(data, offset, string_id_value_count)?;
        let (locale_values, offset) =
            slice_from_bytes::<StringId>(data, offset, locale_value_count)?;
        let (enum_values, offset) = slice_from_bytes::<StringId>(data, offset, enum_value_count)?;
        let (strong_values, offset) =
            slice_from_bytes::<Pointer>(data, offset, strong_value_count)?;
        let (weak_values, offset) = slice_from_bytes::<Pointer>(data, offset, weak_value_count)?;
        let (reference_values, offset) =
            slice_from_bytes::<Reference>(data, offset, reference_value_count)?;
        let (enum_options_all, offset) =
            slice_from_bytes::<StringId2>(data, offset, enum_option_count)?;

        // 6. String tables
        let st1_end = offset + text_length;
        if st1_end > data.len() {
            return Err(CommonParseError::Truncated {
                offset,
                need: text_length,
                have: data.len() - offset,
            }
            .into());
        }
        let string_table1 = &data[offset..st1_end];

        let st2_end = st1_end + text_length2;
        if st2_end > data.len() {
            return Err(CommonParseError::Truncated {
                offset: st1_end,
                need: text_length2,
                have: data.len() - st1_end,
            }
            .into());
        }
        let string_table2 = &data[st1_end..st2_end];

        // 7. Instance data is everything remaining
        let instance_data = &data[st2_end..];

        // 8. Compute instance offsets (relative to start of instance_data)
        let mut instance_offsets = vec![0usize; struct_defs.len()];
        let mut running_offset = 0usize;
        for mapping in data_mappings {
            let si = mapping.struct_index as usize;
            instance_offsets[si] = running_offset;
            running_offset += mapping.struct_count as usize * struct_defs[si].struct_size as usize;
        }

        // 9. Build record lookup map
        let mut record_map = FxHashMap::with_capacity_and_hasher(records.len(), Default::default());
        for (i, record) in records.iter().enumerate() {
            record_map.insert(record.id, i);
        }

        // 9b. Build struct_index → record indices lookup
        let records_by_struct = {
            let n = struct_defs.len();
            let mut counts = vec![0usize; n];
            for record in records {
                counts[record.struct_index as usize] += 1;
            }
            let mut buckets: Vec<Vec<usize>> =
                counts.iter().map(|&c| Vec::with_capacity(c)).collect();
            for (i, record) in records.iter().enumerate() {
                buckets[record.struct_index as usize].push(i);
            }
            buckets
                .into_iter()
                .map(|v| v.into_boxed_slice())
                .collect::<Vec<_>>()
        };

        // 10. Build struct name → index map
        let struct_name_map = {
            let mut map =
                FxHashMap::with_capacity_and_hasher(struct_defs.len(), Default::default());
            for (i, sd) in struct_defs.iter().enumerate() {
                let name = resolve_string_from_table(string_table2, sd.name_offset.0);
                map.insert(name, i as i32);
            }
            map
        };

        // 11. Build cached flattened property indices per struct type
        let cached_properties = {
            let mut cache = Vec::with_capacity(struct_defs.len());
            for si in 0..struct_defs.len() {
                let this = &struct_defs[si];
                if this.attribute_count == 0 && this.parent_type_index == -1 {
                    cache.push(Box::new([]) as Box<[u16]>);
                    continue;
                }

                // Walk parent chain to get total count
                let mut total_count: usize = this.attribute_count as usize;
                let mut walk = *this;
                while walk.parent_type_index != -1 {
                    walk = struct_defs[walk.parent_type_index as usize];
                    total_count += walk.attribute_count as usize;
                }

                // Fill parent-first
                let mut indices = vec![0u16; total_count];
                let mut pos = total_count;
                let mut current = *this;
                loop {
                    let count = current.attribute_count as usize;
                    pos -= count;
                    let first = current.first_attribute_index as usize;
                    for i in 0..count {
                        indices[pos + i] = (first + i) as u16;
                    }
                    if current.parent_type_index == -1 {
                        break;
                    }
                    current = struct_defs[current.parent_type_index as usize];
                }

                cache.push(indices.into_boxed_slice());
            }
            cache
        };

        // 11. Pre-compute cumulative byte offsets per property position
        let cached_offsets = {
            let mut offsets_cache = Vec::with_capacity(struct_defs.len());
            for si in 0..struct_defs.len() {
                let props = &cached_properties[si];
                let mut offsets = Vec::with_capacity(props.len());
                let mut running = 0usize;
                for &pi in props.iter() {
                    offsets.push(running);
                    let prop = &property_defs[pi as usize];
                    if let (Ok(dt), Ok(ct)) = (
                        DataType::try_from(prop.data_type),
                        ConversionType::try_from(prop.conversion_type),
                    ) {
                        if ct != ConversionType::Attribute {
                            running += 8; // array header: count + first_index
                        } else if dt == DataType::Class {
                            running += struct_defs[prop.struct_index as usize].struct_size as usize;
                        } else {
                            running += dt.inline_size();
                        }
                    }
                }
                offsets_cache.push(offsets.into_boxed_slice());
            }
            offsets_cache
        };

        // 12. Precompute which struct types transitively contain WeakPointer properties
        let has_weak_pointers = {
            let n = struct_defs.len();
            // First pass: mark structs that directly have a WeakPointer property
            let mut direct_weak = vec![false; n];
            // Also track which struct indices are referenced by Class/StrongPointer properties
            let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];

            for si in 0..n {
                for &pi in cached_properties[si].iter() {
                    let prop = &property_defs[pi as usize];
                    if let Ok(dt) = DataType::try_from(prop.data_type) {
                        match dt {
                            DataType::WeakPointer => {
                                direct_weak[si] = true;
                            }
                            DataType::Class | DataType::StrongPointer => {
                                edges[si].push(prop.struct_index as usize);
                            }
                            // References can point to any struct, can't statically resolve
                            DataType::Reference => {
                                direct_weak[si] = true; // conservative: assume references might reach weak ptrs
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Propagate: if any reachable struct has weak pointers, mark this one too
            // Simple iterative fixpoint
            let mut result = direct_weak.clone();
            let mut changed = true;
            while changed {
                changed = false;
                for si in 0..n {
                    if result[si] {
                        continue;
                    }
                    for &target in &edges[si] {
                        if result[target] {
                            result[si] = true;
                            changed = true;
                            break;
                        }
                    }
                }
            }
            result
        };

        // 12. Compute main records: last record per unique file_name_offset
        let mut last_by_file: FxHashMap<i32, CigGuid> = FxHashMap::default();
        for record in records {
            last_by_file.insert(record.file_name_offset.0, record.id);
        }
        let mut main_record_ids = HashSet::new();
        for id in last_by_file.values() {
            main_record_ids.insert(*id);
        }

        Ok(Database {
            struct_defs,
            property_defs,
            enum_defs,
            data_mappings,
            records,
            int8_values_raw,
            int16_values_raw,
            int32_values_raw,
            int64_values_raw,
            uint8_values_raw,
            uint16_values_raw,
            uint32_values_raw,
            uint64_values_raw,
            bool_values_raw,
            single_values_raw,
            double_values_raw,
            guid_values,
            string_id_values,
            locale_values,
            enum_values,
            strong_values,
            weak_values,
            reference_values,
            enum_options_all,
            string_table1,
            string_table2,
            instance_data,
            int8_count: int8_value_count,
            int16_count: int16_value_count,
            int32_count: int32_value_count,
            int64_count: int64_value_count,
            uint8_count: uint8_value_count,
            uint16_count: uint16_value_count,
            uint32_count: uint32_value_count,
            uint64_count: uint64_value_count,
            bool_count: boolean_value_count,
            single_count: single_value_count,
            double_count: double_value_count,
            instance_offsets,
            cached_properties,
            cached_offsets,
            record_map,
            struct_name_map,
            main_record_ids,
            has_weak_pointers,
            records_by_struct,
        })
    }

    // ── Definition table accessors ─────────────────────────────────────────

    pub fn struct_defs(&self) -> &[StructDefinition] {
        self.struct_defs
    }

    pub fn property_defs(&self) -> &[PropertyDefinition] {
        self.property_defs
    }

    pub fn enum_defs(&self) -> &[EnumDefinition] {
        self.enum_defs
    }

    pub fn data_mappings(&self) -> &[DataMapping] {
        self.data_mappings
    }

    pub fn records(&self) -> &[Record] {
        self.records
    }

    // ── Value array accessors (unaligned-safe) ─────────────────────────────

    pub fn int8_count(&self) -> usize {
        self.int8_count
    }
    pub fn int16_count(&self) -> usize {
        self.int16_count
    }
    pub fn int32_count(&self) -> usize {
        self.int32_count
    }
    pub fn int64_count(&self) -> usize {
        self.int64_count
    }
    pub fn uint8_count(&self) -> usize {
        self.uint8_count
    }
    pub fn uint16_count(&self) -> usize {
        self.uint16_count
    }
    pub fn uint32_count(&self) -> usize {
        self.uint32_count
    }
    pub fn uint64_count(&self) -> usize {
        self.uint64_count
    }
    pub fn bool_count(&self) -> usize {
        self.bool_count
    }
    pub fn single_count(&self) -> usize {
        self.single_count
    }
    pub fn double_count(&self) -> usize {
        self.double_count
    }

    pub fn get_int8(&self, index: usize) -> Result<i8, crate::error::QueryError> {
        self.int8_values_raw.get(index)
            .map(|&b| b as i8)
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: index, need: 1, have: self.int8_values_raw.len(),
            }))
    }

    pub fn get_int16(&self, index: usize) -> Result<i16, crate::error::QueryError> {
        let off = index * 2;
        let bytes: [u8; 2] = self.int16_values_raw.get(off..off + 2)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 2, have: self.int16_values_raw.len(),
            }))?;
        Ok(i16::from_le_bytes(bytes))
    }

    pub fn get_int32(&self, index: usize) -> Result<i32, crate::error::QueryError> {
        let off = index * 4;
        let bytes: [u8; 4] = self.int32_values_raw.get(off..off + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 4, have: self.int32_values_raw.len(),
            }))?;
        Ok(i32::from_le_bytes(bytes))
    }

    pub fn get_int64(&self, index: usize) -> Result<i64, crate::error::QueryError> {
        let off = index * 8;
        let bytes: [u8; 8] = self.int64_values_raw.get(off..off + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 8, have: self.int64_values_raw.len(),
            }))?;
        Ok(i64::from_le_bytes(bytes))
    }

    pub fn get_uint8(&self, index: usize) -> Result<u8, crate::error::QueryError> {
        self.uint8_values_raw.get(index)
            .copied()
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: index, need: 1, have: self.uint8_values_raw.len(),
            }))
    }

    pub fn get_uint16(&self, index: usize) -> Result<u16, crate::error::QueryError> {
        let off = index * 2;
        let bytes: [u8; 2] = self.uint16_values_raw.get(off..off + 2)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 2, have: self.uint16_values_raw.len(),
            }))?;
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn get_uint32(&self, index: usize) -> Result<u32, crate::error::QueryError> {
        let off = index * 4;
        let bytes: [u8; 4] = self.uint32_values_raw.get(off..off + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 4, have: self.uint32_values_raw.len(),
            }))?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn get_uint64(&self, index: usize) -> Result<u64, crate::error::QueryError> {
        let off = index * 8;
        let bytes: [u8; 8] = self.uint64_values_raw.get(off..off + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 8, have: self.uint64_values_raw.len(),
            }))?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn get_bool(&self, index: usize) -> Result<bool, crate::error::QueryError> {
        self.bool_values_raw.get(index)
            .map(|&b| b != 0)
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: index, need: 1, have: self.bool_values_raw.len(),
            }))
    }

    pub fn get_single(&self, index: usize) -> Result<f32, crate::error::QueryError> {
        let off = index * 4;
        let bytes: [u8; 4] = self.single_values_raw.get(off..off + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 4, have: self.single_values_raw.len(),
            }))?;
        Ok(f32::from_le_bytes(bytes))
    }

    pub fn get_double(&self, index: usize) -> Result<f64, crate::error::QueryError> {
        let off = index * 8;
        let bytes: [u8; 8] = self.double_values_raw.get(off..off + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or(crate::error::QueryError::Read(CommonParseError::Truncated {
                offset: off, need: 8, have: self.double_values_raw.len(),
            }))?;
        Ok(f64::from_le_bytes(bytes))
    }

    // ── Instance access ────────────────────────────────────────────────────

    /// Returns the byte offset (relative to instance_data start) for a struct type.
    pub fn instance_offset(&self, struct_index: i32) -> usize {
        self.instance_offsets[struct_index as usize]
    }

    /// Returns the raw instance bytes for the given struct/instance index pair.
    pub fn get_instance(&self, struct_index: i32, instance_index: i32) -> &'a [u8] {
        let si = struct_index as usize;
        let size = self.struct_defs[si].struct_size as usize;
        let base = self.instance_offsets[si];
        let start = base + instance_index as usize * size;
        &self.instance_data[start..start + size]
    }

    // ── Schema access ──────────────────────────────────────────────────────

    pub fn struct_def(&self, index: i32) -> &StructDefinition {
        &self.struct_defs[index as usize]
    }

    /// Get the name of a struct type by its opaque ID.
    pub fn struct_name(&self, id: crate::types::StructId) -> &str {
        self.resolve_string2(self.struct_defs[id.0 as usize].name_offset)
    }

    pub fn enum_def(&self, index: i32) -> &EnumDefinition {
        &self.enum_defs[index as usize]
    }

    /// Returns the cached flattened property indices for a struct type.
    /// Parent properties come first, child properties last.
    /// Indices are into `property_defs()`.
    #[inline]
    pub fn all_property_indices(&self, struct_index: i32) -> &[u16] {
        &self.cached_properties[struct_index as usize]
    }

    /// Returns the byte offset from struct start to the property at `position`
    /// in the flattened property list of `struct_index`.
    #[inline]
    pub fn property_byte_offset(&self, struct_index: i32, position: usize) -> usize {
        self.cached_offsets[struct_index as usize][position]
    }

    /// Returns all properties for a struct, including inherited ones.
    /// Parent properties come first, child properties last.
    pub fn all_properties(&self, struct_index: i32) -> Vec<&PropertyDefinition> {
        let structs = self.struct_defs;
        let properties = self.property_defs;

        let this = &structs[struct_index as usize];

        if this.attribute_count == 0 && this.parent_type_index == -1 {
            return Vec::new();
        }

        // Calculate total property count by walking parent chain
        let mut total_count: usize = this.attribute_count as usize;
        let mut walk = *this;
        while walk.parent_type_index != -1 {
            walk = structs[walk.parent_type_index as usize];
            total_count += walk.attribute_count as usize;
        }

        // Pre-allocate with exact size
        let mut result: Vec<&PropertyDefinition> = vec![&properties[0]; total_count];

        // Fill in reverse: child properties at end, walk up to root
        let mut current_position = total_count;
        let mut current = *this;
        loop {
            let count = current.attribute_count as usize;
            current_position -= count;
            let first = current.first_attribute_index as usize;
            for i in 0..count {
                result[current_position + i] = &properties[first + i];
            }
            if current.parent_type_index == -1 {
                break;
            }
            current = structs[current.parent_type_index as usize];
        }

        result
    }

    /// Returns the enum option slice for a given enum definition index.
    pub fn enum_options(&self, enum_index: i32) -> &[StringId2] {
        let e = &self.enum_defs[enum_index as usize];
        let first = e.first_value_index as usize;
        let count = e.value_count as usize;
        &self.enum_options_all[first..first + count]
    }

    // ── String resolution ──────────────────────────────────────────────────

    /// Resolve a StringId from string table 1.
    pub fn resolve_string(&self, id: StringId) -> &'a str {
        resolve_string_from_table(self.string_table1, id.0)
    }

    /// Resolve a StringId2 from string table 2.
    pub fn resolve_string2(&self, id: StringId2) -> &'a str {
        resolve_string_from_table(self.string_table2, id.0)
    }

    /// O(1) struct lookup by name. Returns raw index for internal use.
    pub fn struct_index_by_name(&self, name: &str) -> Option<i32> {
        self.struct_name_map.get(name).copied()
    }

    /// O(1) struct lookup by name. Returns an opaque [`StructId`].
    pub fn struct_id(&self, name: &str) -> Option<crate::types::StructId> {
        self.struct_name_map
            .get(name)
            .map(|&i| crate::types::StructId(i))
    }

    // ── Record lookup ──────────────────────────────────────────────────────

    /// Look up a record by its GUID.
    pub fn record_by_id(&self, id: &CigGuid) -> Option<&Record> {
        self.record_map.get(id).map(|&i| &self.records[i])
    }

    /// Check if a record is a "main" record (last record with its file_name_offset).
    pub fn is_main_record(&self, record: &Record) -> bool {
        self.main_record_ids.contains(&record.id)
    }

    /// Returns an iterator over records of a given struct type. O(1) lookup.
    pub fn records_of_type(
        &self,
        struct_id: crate::types::StructId,
    ) -> impl Iterator<Item = &Record> {
        self.records_by_struct[struct_id.0 as usize]
            .iter()
            .map(move |&i| &self.records[i])
    }

    /// Returns an iterator over records of a named struct type.
    /// Returns an empty iterator if the struct type doesn't exist.
    pub fn records_by_type_name(&self, name: &str) -> impl Iterator<Item = &Record> {
        let indices = self
            .struct_index_by_name(name)
            .map(|si| &*self.records_by_struct[si as usize])
            .unwrap_or(&[]);
        indices.iter().map(move |&i| &self.records[i])
    }

    /// Check if a struct type can transitively contain weak pointers.
    #[inline]
    pub fn struct_has_weak_pointers(&self, struct_index: i32) -> bool {
        self.has_weak_pointers[struct_index as usize]
    }

    // ── Raw table access (for walker) ──────────────────────────────────────

    pub fn string_table1(&self) -> &'a [u8] {
        self.string_table1
    }

    pub fn string_table2(&self) -> &'a [u8] {
        self.string_table2
    }

    pub fn instance_data(&self) -> &'a [u8] {
        self.instance_data
    }

    // ── Query API ────────────────────────────────────────────────────────

    /// Compile a query path for the given struct type.
    pub fn compile_path<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        struct_id: crate::types::StructId,
        path: &str,
    ) -> Result<crate::query::compile::CompiledPath, crate::error::QueryError> {
        crate::query::compile::compile_path::<T>(self, struct_id.0, path)
    }

    /// Query all matching values for a compiled path on a record.
    pub fn query<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        path: &crate::query::compile::CompiledPath,
        record: &crate::types::Record,
    ) -> Result<Vec<T>, crate::error::QueryError> {
        crate::query::execute::query_all::<T>(self, path, record)
    }

    /// Query a path returning Value trees, but don't expand References.
    /// References are returned as `Value::Guid(record_id)` instead of materialized Objects.
    /// Use this when you need the inline structure but will resolve references separately,
    /// avoiding memory explosions from large entity graphs.
    pub fn query_no_references(
        &'a self,
        path: &crate::query::compile::CompiledPath,
        record: &crate::types::Record,
    ) -> Result<Vec<crate::query::value::Value<'a>>, crate::error::QueryError> {
        crate::query::execute::query_all_no_refs(self, path, record)
    }

    /// Query a single matching value, returning None if no match.
    pub fn query_single<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        path: &crate::query::compile::CompiledPath,
        record: &crate::types::Record,
    ) -> Result<Option<T>, crate::error::QueryError> {
        crate::query::execute::query_first::<T>(self, path, record)
    }

    /// Compile a multi-field path: shared prefix + multiple leaf fields.
    pub fn compile_multi_path<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        struct_id: crate::types::StructId,
        prefix_path: &str,
        leaf_names: &[&str],
    ) -> Result<crate::query::compile::CompiledMultiPath, crate::error::QueryError> {
        crate::query::compile::compile_multi_path::<T>(self, struct_id.0, prefix_path, leaf_names)
    }

    /// Execute a multi-field query: traverse the shared prefix once, read all leaf fields.
    pub fn query_multi<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        path: &crate::query::compile::CompiledMultiPath,
        record: &crate::types::Record,
    ) -> Result<Vec<Option<T>>, crate::error::QueryError> {
        crate::query::execute::query_multi::<T>(self, path, record)
    }

    // ── Rooted query API (type name in path) ─────────────────────────────

    /// Compile a rooted path where the first segment is the type name.
    ///
    /// Example: `"EntityClassDefinition.Components[SCItemShieldGeneratorParams].MaxShieldHealth"`
    pub fn compile_rooted<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        path: &str,
    ) -> Result<crate::query::compile::CompiledPath, crate::error::QueryError> {
        crate::query::compile::compile_rooted::<T>(self, path)
    }

    /// Compile a rooted multi-field path where the first segment is the type name.
    ///
    /// Example prefix: `"EntityClassDefinition.Components[SAmmoContainerComponentParams].ammoParamsRecord.damage[DamageInfo]"`
    pub fn compile_multi_rooted<T: crate::query::from_datacore::FromDataCore<'a>>(
        &'a self,
        prefix_path: &str,
        leaf_names: &[&str],
    ) -> Result<crate::query::compile::CompiledMultiPath, crate::error::QueryError> {
        crate::query::compile::compile_multi_rooted::<T>(self, prefix_path, leaf_names)
    }
}

/// Scan from byte offset to next null byte in the raw string table.
fn resolve_string_from_table(table: &[u8], offset: i32) -> &str {
    let start = offset as usize;
    let end = table[start..]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(table.len() - start)
        + start;
    std::str::from_utf8(&table[start..end]).unwrap_or("<invalid utf8>")
}

/// Owns the backing data and can produce a borrowed `Database`.
pub struct OwnedDatabase {
    data: Vec<u8>,
}

impl OwnedDatabase {
    /// Take ownership of pre-read bytes. Validates that the data parses as a valid DCB.
    pub fn from_vec(data: Vec<u8>) -> Result<Self, ParseError> {
        let _ = Database::from_bytes(&data)?;
        Ok(OwnedDatabase { data })
    }

    /// Read all bytes from a reader and store them.
    pub fn from_reader(mut reader: impl Read) -> Result<Self, ParseError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).map_err(|e| {
            ParseError::Common(CommonParseError::InvalidLayout(format!("I/O error: {e}")))
        })?;
        // Validate that we can parse it (fail-fast)
        let _ = Database::from_bytes(&data)?;
        Ok(OwnedDatabase { data })
    }

    /// Access the raw backing bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Borrow the owned data as a `Database`.
    pub fn as_database(&self) -> Result<Database<'_>, ParseError> {
        Database::from_bytes(&self.data)
    }
}
