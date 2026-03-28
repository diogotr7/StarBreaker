mod test_helpers;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, Pointer, PropertyDefinition, Record, StructDefinition,
};
use starbreaker_datacore::walker::prescan_weak_pointers;
use test_helpers::DcbBuilder;
use zerocopy::IntoBytes;

// ─── 1. prescan_finds_weak_pointer ──────────────────────────────────────────

#[test]
fn prescan_finds_weak_pointer() {
    let mut b = DcbBuilder::new();

    let file_name = b.add_string1("test.dcb");
    let struct_name = b.add_string2("MyStruct");
    let prop_name = b.add_string2("myWeak");
    let target_name = b.add_string2("Target");

    // Struct 0: "MyStruct" with one WeakPointer attribute
    b.struct_defs.push(StructDefinition {
        name_offset: struct_name,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 0,
        struct_size: 8, // Pointer is 8 bytes
    });

    // Struct 1: "Target" (the thing being pointed to)
    b.struct_defs.push(StructDefinition {
        name_offset: target_name,
        parent_type_index: -1,
        attribute_count: 0,
        first_attribute_index: 0,
        struct_size: 4,
    });

    // Property 0: WeakPointer (0x0210), Attribute (0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_name,
        struct_index: 1, // points to struct 1
        data_type: 0x0210,
        conversion_type: 0x00,
        _padding: 0,
    });

    // Data mappings: 1 instance of struct 0, 1 instance of struct 1
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 0,
    });
    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 1,
    });

    let record_id = CigGuid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);

    b.records.push(Record {
        name_offset: struct_name,
        file_name_offset: file_name,
        struct_index: 0,
        id: record_id,
        instance_index: 0,
        struct_size: 8,
    });

    // Instance data for struct 0, instance 0: a Pointer { struct_index: 1, instance_index: 0 }
    let ptr = Pointer {
        struct_index: 1,
        instance_index: 0,
    };
    b.instance_data.extend_from_slice(ptr.as_bytes());

    // Instance data for struct 1, instance 0: 4 bytes of padding
    b.instance_data.extend_from_slice(&[0u8; 4]);

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    let record = &db.records()[0];
    let map = prescan_weak_pointers(&db, record);

    // Should find the weak pointer target (struct 1, instance 0) with ID 1
    assert_eq!(map.len(), 1);
    assert_eq!(map[&(1, 0)], 1);
}

// ─── 2. prescan_null_weak_pointer_ignored ───────────────────────────────────

#[test]
fn prescan_null_weak_pointer_ignored() {
    let mut b = DcbBuilder::new();

    let file_name = b.add_string1("test.dcb");
    let struct_name = b.add_string2("MyStruct");
    let prop_name = b.add_string2("myWeak");

    // Struct 0: "MyStruct" with one WeakPointer attribute
    b.struct_defs.push(StructDefinition {
        name_offset: struct_name,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 0,
        struct_size: 8,
    });

    // Property 0: WeakPointer (0x0210), Attribute (0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_name,
        struct_index: 0,
        data_type: 0x0210,
        conversion_type: 0x00,
        _padding: 0,
    });

    b.data_mappings.push(DataMapping {
        struct_count: 1,
        struct_index: 0,
    });

    let record_id = CigGuid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);

    b.records.push(Record {
        name_offset: struct_name,
        file_name_offset: file_name,
        struct_index: 0,
        id: record_id,
        instance_index: 0,
        struct_size: 8,
    });

    // Instance data for struct 0, instance 0: a null Pointer (-1, -1)
    let ptr = Pointer {
        struct_index: -1,
        instance_index: -1,
    };
    b.instance_data.extend_from_slice(ptr.as_bytes());

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    let record = &db.records()[0];
    let map = prescan_weak_pointers(&db, record);

    // Null pointer should be ignored
    assert!(map.is_empty());
}
