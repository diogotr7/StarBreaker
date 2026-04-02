use crate::types::{
    CigGuid, DataMapping, EnumDefinition, Pointer, PropertyDefinition, Record, Reference, StringId,
    StringId2, StructDefinition,
};
use zerocopy::IntoBytes;

/// A builder that programmatically constructs valid DCB v6 binary data for unit testing.
pub struct DcbBuilder {
    pub struct_defs: Vec<StructDefinition>,
    pub property_defs: Vec<PropertyDefinition>,
    pub enum_defs: Vec<EnumDefinition>,
    pub data_mappings: Vec<DataMapping>,
    pub records: Vec<Record>,

    pub int8_values: Vec<i8>,
    pub int16_values: Vec<i16>,
    pub int32_values: Vec<i32>,
    pub int64_values: Vec<i64>,
    pub uint8_values: Vec<u8>,
    pub uint16_values: Vec<u16>,
    pub uint32_values: Vec<u32>,
    pub uint64_values: Vec<u64>,
    pub bool_values: Vec<u8>,
    pub single_values: Vec<f32>,
    pub double_values: Vec<f64>,
    pub guid_values: Vec<CigGuid>,
    pub string_id_values: Vec<StringId>,
    pub locale_values: Vec<StringId>,
    pub enum_values: Vec<StringId>,
    pub strong_values: Vec<Pointer>,
    pub weak_values: Vec<Pointer>,
    pub reference_values: Vec<Reference>,
    pub enum_options: Vec<StringId2>,

    pub string_table1: Vec<u8>,
    pub string_table2: Vec<u8>,
    pub instance_data: Vec<u8>,
}

impl Default for DcbBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DcbBuilder {
    pub fn new() -> Self {
        DcbBuilder {
            struct_defs: Vec::new(),
            property_defs: Vec::new(),
            enum_defs: Vec::new(),
            data_mappings: Vec::new(),
            records: Vec::new(),
            int8_values: Vec::new(),
            int16_values: Vec::new(),
            int32_values: Vec::new(),
            int64_values: Vec::new(),
            uint8_values: Vec::new(),
            uint16_values: Vec::new(),
            uint32_values: Vec::new(),
            uint64_values: Vec::new(),
            bool_values: Vec::new(),
            single_values: Vec::new(),
            double_values: Vec::new(),
            guid_values: Vec::new(),
            string_id_values: Vec::new(),
            locale_values: Vec::new(),
            enum_values: Vec::new(),
            strong_values: Vec::new(),
            weak_values: Vec::new(),
            reference_values: Vec::new(),
            enum_options: Vec::new(),
            string_table1: Vec::new(),
            string_table2: Vec::new(),
            instance_data: Vec::new(),
        }
    }

    /// Adds a null-terminated string to string table 1 and returns the byte offset as a StringId.
    pub fn add_string1(&mut self, s: &str) -> StringId {
        let offset = self.string_table1.len() as i32;
        self.string_table1.extend_from_slice(s.as_bytes());
        self.string_table1.push(0); // null terminator
        StringId(offset)
    }

    /// Adds a null-terminated string to string table 2 and returns the byte offset as a StringId2.
    pub fn add_string2(&mut self, s: &str) -> StringId2 {
        let offset = self.string_table2.len() as i32;
        self.string_table2.extend_from_slice(s.as_bytes());
        self.string_table2.push(0); // null terminator
        StringId2(offset)
    }

    /// Produces the complete binary blob in DCB v6 format.
    pub fn build(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Header: 120 bytes total
        // magic (4), version (4), reserved (4), reserved (4), then 26 count fields (26*4=104)
        buf.extend_from_slice(&0xDCB0u32.to_le_bytes()); // magic
        buf.extend_from_slice(&6u32.to_le_bytes()); // version
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved

        // 26 count fields (i32 except last two are u32)
        buf.extend_from_slice(&(self.struct_defs.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.property_defs.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.enum_defs.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.data_mappings.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.records.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.bool_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.int8_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.int16_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.int32_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.int64_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.uint8_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.uint16_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.uint32_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.uint64_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.single_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.double_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.guid_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.string_id_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.locale_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.enum_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.strong_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.weak_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.reference_values.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.enum_options.len() as i32).to_le_bytes());
        buf.extend_from_slice(&(self.string_table1.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(self.string_table2.len() as u32).to_le_bytes());

        assert_eq!(buf.len(), 120, "header must be exactly 120 bytes");

        // Sections in order:
        // 1. StructDefinition
        for s in &self.struct_defs {
            buf.extend_from_slice(s.as_bytes());
        }
        // 2. PropertyDefinition
        for p in &self.property_defs {
            buf.extend_from_slice(p.as_bytes());
        }
        // 3. EnumDefinition
        for e in &self.enum_defs {
            buf.extend_from_slice(e.as_bytes());
        }
        // 4. DataMapping
        for m in &self.data_mappings {
            buf.extend_from_slice(m.as_bytes());
        }
        // 5. Record (v6 format: 32 bytes, skipping tag_offset)
        for r in &self.records {
            buf.extend_from_slice(r.name_offset.as_bytes());
            buf.extend_from_slice(r.file_name_offset.as_bytes());
            buf.extend_from_slice(&r.struct_index.to_le_bytes());
            buf.extend_from_slice(r.id.as_bytes());
            buf.extend_from_slice(&r.instance_index.to_le_bytes());
            buf.extend_from_slice(&r.struct_size.to_le_bytes());
        }

        // 6. int8_values
        for v in &self.int8_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 7. int16_values
        for v in &self.int16_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 8. int32_values
        for v in &self.int32_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 9. int64_values
        for v in &self.int64_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 10. uint8_values
        for v in &self.uint8_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 11. uint16_values
        for v in &self.uint16_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 12. uint32_values
        for v in &self.uint32_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 13. uint64_values
        for v in &self.uint64_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 14. bool_values (AFTER all integers)
        buf.extend_from_slice(&self.bool_values);
        // 15. single_values
        for v in &self.single_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 16. double_values
        for v in &self.double_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 17. guid_values
        for v in &self.guid_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 18. string_id_values
        for v in &self.string_id_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 19. locale_values
        for v in &self.locale_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 20. enum_values
        for v in &self.enum_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 21. strong_values
        for v in &self.strong_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 22. weak_values
        for v in &self.weak_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 23. reference_values
        for v in &self.reference_values {
            buf.extend_from_slice(v.as_bytes());
        }
        // 24. enum_options
        for v in &self.enum_options {
            buf.extend_from_slice(v.as_bytes());
        }
        // 25. string_table1
        buf.extend_from_slice(&self.string_table1);
        // 26. string_table2
        buf.extend_from_slice(&self.string_table2);
        // 27. instance data
        buf.extend_from_slice(&self.instance_data);

        buf
    }
}
