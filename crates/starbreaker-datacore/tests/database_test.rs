mod test_helpers;

use starbreaker_datacore::database::{Database, OwnedDatabase};
use starbreaker_datacore::error::ParseError;
use starbreaker_datacore::starbreaker_common::ParseError as CommonParseError;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, PropertyDefinition, Record, StringId2, StructDefinition,
};
use test_helpers::DcbBuilder;

// ─── 1. parse_empty_database ────────────────────────────────────────────────

#[test]
fn parse_empty_database() {
    let data = DcbBuilder::new().build();
    let db = Database::from_bytes(&data).unwrap();

    assert_eq!(db.struct_defs().len(), 0);
    assert_eq!(db.property_defs().len(), 0);
    assert_eq!(db.enum_defs().len(), 0);
    assert_eq!(db.data_mappings().len(), 0);
    assert_eq!(db.records().len(), 0);
}

// ─── 2. reject_version_5 ───────────────────────────────────────────────────

#[test]
fn reject_version_5() {
    let data = DcbBuilder::new().build();
    let mut patched = data.clone();
    // Version is at bytes 4..8, patch it to 5
    patched[4..8].copy_from_slice(&5u32.to_le_bytes());
    let result = Database::from_bytes(&patched);
    assert!(result.is_err());
    match result.unwrap_err() {
        ParseError::UnsupportedVersion(v) => assert_eq!(v, 5),
        other => panic!("expected UnsupportedVersion, got: {other}"),
    }
}

// ─── 3. reject_truncated_data ───────────────────────────────────────────────

#[test]
fn reject_truncated_data() {
    let data = vec![0u8; 50];
    let result = Database::from_bytes(&data);
    assert!(result.is_err());
    match result.unwrap_err() {
        ParseError::Common(CommonParseError::Truncated {
            offset: _,
            need,
            have,
        }) => {
            assert_eq!(need, 120);
            assert_eq!(have, 50);
        }
        other => panic!("expected Truncated, got: {other}"),
    }
}

// ─── 4. parse_with_one_struct_and_record ────────────────────────────────────

#[test]
fn parse_with_one_struct_and_record() {
    let mut b = DcbBuilder::new();

    // String tables
    let file_name = b.add_string1("test_file.dcb");
    let struct_name = b.add_string2("TestStruct");
    let prop_name = b.add_string2("health");

    // One struct with one property, size = 4 (one i32)
    b.struct_defs.push(StructDefinition {
        name_offset: struct_name,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 0,
        struct_size: 4,
    });

    // One property: Int32, Attribute
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_name,
        struct_index: 0,
        data_type: 0x0004,     // Int32
        conversion_type: 0x00, // Attribute
        _padding: 0,
    });

    // Data mapping: 1 instance of struct 0
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 0,
    });

    let record_id = CigGuid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);

    b.records.push(Record {
        name_offset: struct_name,
        file_name_offset: file_name,
        tag_offset: StringId2(-1),
        struct_index: 0,
        id: record_id,
        instance_index: 0,
        struct_size: 4,
    });

    // Instance data: one i32 = 42
    b.instance_data.extend_from_slice(&42i32.to_le_bytes());

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    assert_eq!(db.struct_defs().len(), 1);
    assert_eq!(db.property_defs().len(), 1);
    assert_eq!(db.records().len(), 1);
    assert_eq!(db.data_mappings().len(), 1);

    // Verify string resolution
    assert_eq!(db.resolve_string(file_name), "test_file.dcb");
    assert_eq!(db.resolve_string2(struct_name), "TestStruct");
    assert_eq!(db.resolve_string2(prop_name), "health");

    // Verify record
    let rec = &db.records()[0];
    assert_eq!(rec.id, record_id);
    // Use local copy to avoid unaligned access on packed struct
    let si = rec.struct_index;
    assert_eq!(si, 0);
}

// ─── 5. get_instance_returns_correct_bytes ──────────────────────────────────

#[test]
fn get_instance_returns_correct_bytes() {
    let mut b = DcbBuilder::new();

    let name = b.add_string2("S");

    // Struct with size = 4
    b.struct_defs.push(StructDefinition {
        name_offset: name,
        parent_type_index: -1,
        attribute_count: 0,
        first_attribute_index: 0,
        struct_size: 4,
    });

    // 2 instances of struct 0
    b.data_mappings.push(DataMapping {
        struct_count: 2,
        struct_index: 0,
    });

    // Instance 0: bytes [0xAA, 0xBB, 0xCC, 0xDD]
    b.instance_data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    // Instance 1: bytes [0x11, 0x22, 0x33, 0x44]
    b.instance_data.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    let inst0 = db.get_instance(0, 0);
    assert_eq!(inst0, &[0xAA, 0xBB, 0xCC, 0xDD]);

    let inst1 = db.get_instance(0, 1);
    assert_eq!(inst1, &[0x11, 0x22, 0x33, 0x44]);
}

// ─── 6. all_properties_flattens_inheritance ─────────────────────────────────

#[test]
fn all_properties_flattens_inheritance() {
    let mut b = DcbBuilder::new();

    let parent_name = b.add_string2("Parent");
    let child_name = b.add_string2("Child");
    let prop_a = b.add_string2("alpha");
    let prop_b = b.add_string2("beta");

    // Parent struct: index 0, one property "alpha"
    b.struct_defs.push(StructDefinition {
        name_offset: parent_name,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 0,
        struct_size: 8,
    });

    // Child struct: index 1, parent = 0, one property "beta"
    b.struct_defs.push(StructDefinition {
        name_offset: child_name,
        parent_type_index: 0,
        attribute_count: 1,
        first_attribute_index: 1,
        struct_size: 8,
    });

    // Property 0: "alpha" on parent
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_a,
        struct_index: 0,
        data_type: 0x0004,
        conversion_type: 0x00,
        _padding: 0,
    });

    // Property 1: "beta" on child
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_b,
        struct_index: 1,
        data_type: 0x0004,
        conversion_type: 0x00,
        _padding: 0,
    });

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    // Parent has just 1 property
    let parent_props = db.all_properties(0);
    assert_eq!(parent_props.len(), 1);
    assert_eq!(parent_props[0].name_offset, prop_a);

    // Child has 2: parent first, then child
    let child_props = db.all_properties(1);
    assert_eq!(child_props.len(), 2);
    assert_eq!(child_props[0].name_offset, prop_a); // parent prop first
    assert_eq!(child_props[1].name_offset, prop_b); // child prop second
}

// ─── 7. record_by_id_lookup ────────────────────────────────────────────────

#[test]
fn record_by_id_lookup() {
    let mut b = DcbBuilder::new();

    let name = b.add_string2("S");
    let file1 = b.add_string1("file1");
    let file2 = b.add_string1("file2");

    b.struct_defs.push(StructDefinition {
        name_offset: name,
        parent_type_index: -1,
        attribute_count: 0,
        first_attribute_index: 0,
        struct_size: 4,
    });

    b.data_mappings.push(DataMapping {
        struct_count: 2,
        struct_index: 0,
    });

    let id1 = CigGuid::from_bytes([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let id2 = CigGuid::from_bytes([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    b.records.push(Record {
        name_offset: name,
        file_name_offset: file1,
        tag_offset: StringId2(-1),
        struct_index: 0,
        id: id1,
        instance_index: 0,
        struct_size: 4,
    });

    b.records.push(Record {
        name_offset: name,
        file_name_offset: file2,
        tag_offset: StringId2(-1),
        struct_index: 0,
        id: id2,
        instance_index: 1,
        struct_size: 4,
    });

    // 2 instances of 4 bytes each
    b.instance_data.extend_from_slice(&[0u8; 8]);

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    let r1 = db.record_by_id(&id1).unwrap();
    assert_eq!(r1.id, id1);

    let r2 = db.record_by_id(&id2).unwrap();
    assert_eq!(r2.id, id2);

    // Non-existent GUID returns None
    let missing = CigGuid::from_bytes([99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(db.record_by_id(&missing).is_none());
}

// ─── 8. main_record_is_last_with_same_filename ─────────────────────────────

#[test]
fn main_record_is_last_with_same_filename() {
    let mut b = DcbBuilder::new();

    let name = b.add_string2("S");
    let shared_file = b.add_string1("shared.dcb");

    b.struct_defs.push(StructDefinition {
        name_offset: name,
        parent_type_index: -1,
        attribute_count: 0,
        first_attribute_index: 0,
        struct_size: 4,
    });

    b.data_mappings.push(DataMapping {
        struct_count: 2,
        struct_index: 0,
    });

    let id_first = CigGuid::from_bytes([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let id_last = CigGuid::from_bytes([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    // Both records share the same file_name_offset
    b.records.push(Record {
        name_offset: name,
        file_name_offset: shared_file,
        tag_offset: StringId2(-1),
        struct_index: 0,
        id: id_first,
        instance_index: 0,
        struct_size: 4,
    });

    b.records.push(Record {
        name_offset: name,
        file_name_offset: shared_file,
        tag_offset: StringId2(-1),
        struct_index: 0,
        id: id_last,
        instance_index: 1,
        struct_size: 4,
    });

    b.instance_data.extend_from_slice(&[0u8; 8]);

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    let first = db.record_by_id(&id_first).unwrap();
    let last = db.record_by_id(&id_last).unwrap();

    // Last record with same filename is the main one
    assert!(!db.is_main_record(first));
    assert!(db.is_main_record(last));
}

// ─── 9. owned_database_from_reader ─────────────────────────────────────────

#[test]
fn owned_database_from_vec() {
    let b = DcbBuilder::new();
    let data = b.build();
    let owned = OwnedDatabase::from_vec(data).unwrap();
    let db = owned.as_database().unwrap();
    assert_eq!(db.records().len(), 0);
}

#[test]
fn owned_database_from_vec_rejects_invalid() {
    let result = OwnedDatabase::from_vec(vec![0u8; 16]);
    assert!(result.is_err());
}

#[test]
fn owned_database_from_reader() {
    let mut b = DcbBuilder::new();
    let _name = b.add_string2("Test");
    let data = b.build();

    let cursor = std::io::Cursor::new(data.clone());
    let owned = OwnedDatabase::from_reader(cursor).unwrap();
    let db = owned.as_database().unwrap();

    assert_eq!(db.struct_defs().len(), 0);
    assert_eq!(db.records().len(), 0);

    // Verify string resolution still works through OwnedDatabase
    assert_eq!(db.resolve_string2(StringId2(0)), "Test");
}
