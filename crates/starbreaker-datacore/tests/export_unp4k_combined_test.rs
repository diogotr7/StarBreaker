//! The combined unp4k document (`export::write_unp4k_combined`): classic
//! unforge's `Game2.xml` — every record under one `<DataForge>` root, in
//! record-table order, main and non-main alike — and unforge's element
//! names for records.

mod test_helpers;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::export::write_unp4k_combined;
use starbreaker_datacore::types::{CigGuid, DataMapping, PropertyDefinition, Record, StringId2, StructDefinition};
use test_helpers::DcbBuilder;

/// One struct `Thing { value: Int32 }`; records `Thing.A` and `Thing.B`
/// share `x.xml` (B is the file's main record, A a non-main record), and
/// `Thing.C` is alone in `y.xml`; `Thing.Genmod Seeds (2)` (in `z.xml`) has
/// characters an XML name cannot hold.
fn build_db() -> Vec<u8> {
    let mut b = DcbBuilder::new();
    let file_x = b.add_string1("x.xml");
    let file_y = b.add_string1("y.xml");
    let file_z = b.add_string1("z.xml");
    let s_thing = b.add_string2("Thing");
    let p_value = b.add_string2("value");
    let names = [b.add_string2("Thing.A"), b.add_string2("Thing.B"), b.add_string2("Thing.C"), b.add_string2("Thing.Genmod Seeds (2)")];

    b.struct_defs.push(StructDefinition {
        name_offset: s_thing,
        parent_type_index: -1,
        attribute_count: 1,
        first_attribute_index: 0,
        struct_size: 4,
    });
    // Int32 (0x0003), Attribute.
    b.property_defs.push(PropertyDefinition { name_offset: p_value, struct_index: 0, data_type: 0x0003, conversion_type: 0x00, _padding: 0 });
    b.data_mappings.push(DataMapping { struct_count: 4, struct_index: 0 });

    let files = [file_x, file_x, file_y, file_z];
    for (i, (name, file)) in names.into_iter().zip(files).enumerate() {
        b.records.push(Record {
            name_offset: name,
            file_name_offset: file,
            tag_offset: StringId2(-1),
            struct_index: 0,
            id: CigGuid::from_bytes([i as u8 + 1; 16]),
            instance_index: i as u16,
            struct_size: 4,
        });
        b.instance_data.extend_from_slice(&(i as i32 + 1).to_le_bytes());
    }
    b.build()
}

#[test]
fn combined_document_holds_every_record_in_table_order() {
    let data = build_db();
    let db = Database::from_bytes(&data).unwrap();
    assert!(!db.is_main_record(&db.records()[0]), "Thing.A shares x.xml with the later Thing.B");

    let records: Vec<&Record> = db.records().iter().collect();
    let mut out = Vec::new();
    write_unp4k_combined(&db, &records, &mut out).unwrap();
    let xml = String::from_utf8(out).unwrap();

    assert!(xml.starts_with("<DataForge>"), "{xml}");
    assert!(xml.trim_end().ends_with("</DataForge>"), "{xml}");
    let at = |needle: &str| xml.find(needle).unwrap_or_else(|| panic!("{needle} missing from {xml}"));
    let (a, b, c) = (at("<Thing.A "), at("<Thing.B "), at("<Thing.C "));
    assert!(a < b && b < c, "record-table order");
    for (name, value, guid, path) in [
        ("Thing.A", 1, [1u8; 16], "x.xml"),
        ("Thing.B", 2, [2u8; 16], "x.xml"),
        ("Thing.C", 3, [3u8; 16], "y.xml"),
    ] {
        let element = &xml[at(&format!("<{name} "))..];
        let element = &element[..element.find('>').unwrap()];
        assert!(element.contains(&format!("value=\"{value}\"")), "{element}");
        assert!(element.contains(&format!("__ref=\"{}\"", CigGuid::from_bytes(guid))), "{element}");
        assert!(element.contains(&format!("__path=\"{path}\"")), "{element}");
    }
}

/// Only the records asked for are written (a filtered extract).
#[test]
fn combined_document_writes_the_given_records() {
    let data = build_db();
    let db = Database::from_bytes(&data).unwrap();
    let records: Vec<&Record> = db.records().iter().skip(2).take(1).collect();
    let mut out = Vec::new();
    write_unp4k_combined(&db, &records, &mut out).unwrap();
    let xml = String::from_utf8(out).unwrap();
    assert!(xml.contains("<Thing.C ") && !xml.contains("<Thing.A ") && !xml.contains("<Thing.B "), "{xml}");
}

/// unforge names a record's element after the record, with every character
/// outside `[a-zA-Z0-9.]` replaced by `_`, so record names with spaces or
/// brackets still make well-formed XML.
#[test]
fn record_elements_use_unforges_clean_names() {
    let data = build_db();
    let db = Database::from_bytes(&data).unwrap();
    let record = &db.records()[3];
    let xml = String::from_utf8(starbreaker_datacore::export::to_unp4k_xml(&db, record).unwrap()).unwrap();
    assert!(xml.starts_with("<Thing.Genmod_Seeds__2_ "), "{xml}");
    assert!(!xml.contains("Genmod Seeds"), "{xml}");
}
