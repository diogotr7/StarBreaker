//! Weak-pointer prescan for the classic DataCore XML exporter.
//!
//! This mirrors the legacy C# diff exporter instead of the generic walker so
//! classic XML parity does not change the behavior of other export formats.

use rustc_hash::FxHashMap;

use crate::database::Database;
use crate::enums::{ConversionType, DataType};
use crate::reader::SpanReader;
use crate::types::{Pointer, Record, Reference};

/// Return true when a legacy C# reference should be treated as null.
pub(super) fn reference_is_null(reference: &Reference) -> bool {
    reference.record_id.is_empty() || reference.instance_index == -1
}

/// Pre-scan a record's classic XML graph to assign weak pointer IDs.
pub(super) fn prescan_weak_pointers(
    db: &Database,
    record: &Record,
) -> FxHashMap<(i32, i32), usize> {
    let mut map = FxHashMap::default();
    walk_instance(
        db,
        record.struct_index,
        record.instance_index as i32,
        record.file_name_offset.0,
        &mut map,
    );
    map
}

fn walk_instance(
    db: &Database,
    struct_index: i32,
    instance_index: i32,
    file_name_offset: i32,
    map: &mut FxHashMap<(i32, i32), usize>,
) {
    let instance_bytes = db.get_instance(struct_index, instance_index);
    let mut reader = SpanReader::new(instance_bytes);
    walk_struct(db, struct_index, &mut reader, file_name_offset, map);
}

fn walk_struct(
    db: &Database,
    struct_index: i32,
    reader: &mut SpanReader,
    file_name_offset: i32,
    map: &mut FxHashMap<(i32, i32), usize>,
) {
    let properties = db.property_defs();
    for &idx in db.all_property_indices(struct_index) {
        let prop = &properties[idx as usize];
        let Ok(data_type) = DataType::try_from(prop.data_type) else {
            continue;
        };
        let Ok(conversion_type) = ConversionType::try_from(prop.conversion_type) else {
            continue;
        };

        if conversion_type == ConversionType::Attribute {
            walk_attribute(db, data_type, prop.struct_index as i32, reader, file_name_offset, map);
        } else {
            walk_array(db, data_type, prop.struct_index as i32, reader, file_name_offset, map);
        }
    }
}

fn walk_attribute(
    db: &Database,
    data_type: DataType,
    prop_struct_index: i32,
    reader: &mut SpanReader,
    file_name_offset: i32,
    map: &mut FxHashMap<(i32, i32), usize>,
) {
    match data_type {
        DataType::Reference => {
            if let Ok(reference) = reader.read_type::<Reference>() {
                walk_reference(db, reference, file_name_offset, map);
            }
        }
        DataType::WeakPointer => {
            if let Ok(pointer) = reader.read_type::<Pointer>() {
                walk_weak_pointer(pointer, map);
            }
        }
        DataType::StrongPointer => {
            if let Ok(pointer) = reader.read_type::<Pointer>() {
                walk_strong_pointer(db, pointer, file_name_offset, map);
            }
        }
        DataType::Class => walk_struct(db, prop_struct_index, reader, file_name_offset, map),
        other => {
            let _ = reader.advance(other.inline_size());
        }
    }
}

fn walk_array(
    db: &Database,
    data_type: DataType,
    prop_struct_index: i32,
    reader: &mut SpanReader,
    file_name_offset: i32,
    map: &mut FxHashMap<(i32, i32), usize>,
) {
    let count = reader.read_i32().unwrap_or(0);
    let first_index = reader.read_i32().unwrap_or(0);
    for i in first_index..first_index + count {
        let idx = i as usize;
        match data_type {
            DataType::Reference => walk_reference(db, &db.reference_values[idx], file_name_offset, map),
            DataType::WeakPointer => walk_weak_pointer(&db.weak_values[idx], map),
            DataType::StrongPointer => {
                walk_strong_pointer(db, &db.strong_values[idx], file_name_offset, map)
            }
            DataType::Class => walk_instance(db, prop_struct_index, i, file_name_offset, map),
            _ => {}
        }
    }
}

fn walk_reference(
    db: &Database,
    reference: &Reference,
    file_name_offset: i32,
    map: &mut FxHashMap<(i32, i32), usize>,
) {
    if reference_is_null(reference) {
        return;
    }

    let Some(record) = db.record_by_id(&reference.record_id) else {
        return;
    };
    if db.is_main_record(record) {
        return;
    }
    if record.file_name_offset.0 != file_name_offset {
        return;
    }

    walk_instance(
        db,
        record.struct_index,
        record.instance_index as i32,
        file_name_offset,
        map,
    );
}

fn walk_strong_pointer(
    db: &Database,
    pointer: &Pointer,
    file_name_offset: i32,
    map: &mut FxHashMap<(i32, i32), usize>,
) {
    if pointer.is_null() {
        return;
    }

    walk_instance(
        db,
        pointer.struct_index,
        pointer.instance_index,
        file_name_offset,
        map,
    );
}

fn walk_weak_pointer(pointer: &Pointer, map: &mut FxHashMap<(i32, i32), usize>) {
    if pointer.is_null() {
        return;
    }

    let key = (pointer.struct_index, pointer.instance_index);
    if !map.contains_key(&key) {
        map.insert(key, map.len() + 1);
    }
}
