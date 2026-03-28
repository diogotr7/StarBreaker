mod test_helpers;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::export;
use starbreaker_datacore::types::{
    CigGuid, DataMapping, PropertyDefinition, Record, StructDefinition,
};
use test_helpers::DcbBuilder;
use zerocopy::IntoBytes;

// ─── 1. export_simple_record_to_json ────────────────────────────────────────

#[test]
fn export_simple_record_to_json() {
    let mut b = DcbBuilder::new();

    // String tables
    let file_name = b.add_string1("test/file.dcb");
    let hello_str = b.add_string1("hello world");
    let struct_name = b.add_string2("TestStruct");
    let prop_health = b.add_string2("health");
    let prop_label = b.add_string2("label");

    // Struct 0: "TestStruct" with two properties: Int32 health, String label
    // struct_size = 4 (Int32) + 4 (StringId) = 8
    b.struct_defs.push(StructDefinition {
        name_offset: struct_name,
        parent_type_index: -1,
        attribute_count: 2,
        first_attribute_index: 0,
        struct_size: 8,
    });

    // Property 0: "health" — Int32 (0x0004), Attribute (0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_health,
        struct_index: 0,
        data_type: 0x0004,
        conversion_type: 0x00,
        _padding: 0,
    });

    // Property 1: "label" — String (0x000A), Attribute (0x00)
    b.property_defs.push(PropertyDefinition {
        name_offset: prop_label,
        struct_index: 0,
        data_type: 0x000A,
        conversion_type: 0x00,
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
        struct_index: 0,
        id: record_id,
        instance_index: 0,
        struct_size: 8,
    });

    // Instance data: health=42 (i32), label=StringId pointing to "hello world"
    let mut instance = Vec::new();
    instance.extend_from_slice(&42i32.to_le_bytes()); // health
    instance.extend_from_slice(hello_str.as_bytes()); // label (StringId is an i32 offset)
    b.instance_data.extend_from_slice(&instance);

    let data = b.build();
    let db = Database::from_bytes(&data).unwrap();

    let record = &db.records()[0];
    let json_bytes = export::to_json(&db, record).unwrap();
    let json_str = std::str::from_utf8(&json_bytes).unwrap();

    // Parse as JSON to verify structure
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();

    // Top-level has _RecordName_, _RecordId_, _RecordValue_
    assert_eq!(parsed["_RecordName_"].as_str().unwrap(), "TestStruct");
    assert!(parsed["_RecordId_"].as_str().is_some());

    let record_value = &parsed["_RecordValue_"];
    assert_eq!(record_value["_Type_"].as_str().unwrap(), "TestStruct");
    assert_eq!(record_value["health"].as_i64().unwrap(), 42);
    assert_eq!(record_value["label"].as_str().unwrap(), "hello world");
}
