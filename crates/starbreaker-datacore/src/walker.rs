use rustc_hash::{FxHashMap, FxHashSet};

use crate::database::Database;
use crate::enums::{ConversionType, DataType};
use crate::error::ExportError;
use crate::reader::SpanReader;
use crate::sink::ExportSink;
use crate::types::{CigGuid, Pointer, Record, Reference, StringId};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Write "ptr:{id}" into a stack buffer, returning the str slice.
fn write_ptr_label(buf: &mut [u8; 24], id: usize) -> &str {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(&mut buf[..]);
    let _ = write!(cursor, "ptr:{id}");
    let n = cursor.position() as usize;
    std::str::from_utf8(&buf[..n]).unwrap_or("ptr:?")
}

/// Write "_PointsTo_:ptr:{id}" into a stack buffer, returning the str slice.
fn write_points_to_label(buf: &mut [u8; 32], id: usize) -> &str {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(&mut buf[..]);
    let _ = write!(cursor, "_PointsTo_:ptr:{id}");
    let n = cursor.position() as usize;
    std::str::from_utf8(&buf[..n]).unwrap_or("_PointsTo_:ptr:?")
}

// ─── Walk context ────────────────────────────────────────────────────────────

/// Tracks state during a record export walk.
struct WalkContext {
    /// Map from (struct_index, instance_index) -> 1-based weak pointer ID.
    weak_pointers: FxHashMap<(i32, i32), usize>,
    /// Set of weak pointer targets not yet visited (for _Pointers_ fallback).
    pointed_to: FxHashSet<(i32, i32)>,
    /// The file_name_offset of the record being walked.
    file_name_offset: i32,
}

// ─── Weak pointer pre-scan ──────────────────────────────────────────────────

/// Pre-scan a record's object graph to discover all weak pointer targets.
/// Returns a map from (struct_index, instance_index) -> 1-based ID.
pub fn prescan_weak_pointers(db: &Database, record: &Record) -> FxHashMap<(i32, i32), usize> {
    let mut map = FxHashMap::default();
    let file_name_offset = record.file_name_offset.0;
    let instance_bytes = db.get_instance(record.struct_index, record.instance_index as i32);
    let mut reader = SpanReader::new(instance_bytes);
    prescan_struct(
        db,
        record.struct_index,
        &mut reader,
        &mut map,
        file_name_offset,
    );
    map
}

fn prescan_struct(
    db: &Database,
    struct_index: i32,
    reader: &mut SpanReader,
    map: &mut FxHashMap<(i32, i32), usize>,
    file_name_offset: i32,
) {
    let prop_indices = db.all_property_indices(struct_index);
    let property_defs = db.property_defs();
    for &idx in prop_indices {
        let prop = &property_defs[idx as usize];
        let data_type = match DataType::try_from(prop.data_type) {
            Ok(dt) => dt,
            Err(_) => continue,
        };
        let conv_type = match ConversionType::try_from(prop.conversion_type) {
            Ok(ct) => ct,
            Err(_) => continue,
        };

        if conv_type == ConversionType::Attribute {
            prescan_attribute(
                db,
                data_type,
                prop.struct_index as i32,
                reader,
                map,
                file_name_offset,
            );
        } else {
            prescan_array(
                db,
                data_type,
                prop.struct_index as i32,
                reader,
                map,
                file_name_offset,
            );
        }
    }
}

fn prescan_attribute(
    db: &Database,
    data_type: DataType,
    prop_struct_index: i32,
    reader: &mut SpanReader,
    map: &mut FxHashMap<(i32, i32), usize>,
    file_name_offset: i32,
) {
    match data_type {
        DataType::WeakPointer => {
            if let Ok(ptr) = reader.read_type::<Pointer>()
                && !ptr.is_null()
            {
                let key = (ptr.struct_index, ptr.instance_index);
                if !map.contains_key(&key) {
                    let id = map.len() + 1; // 1-based
                    map.insert(key, id);
                }
            }
        }
        DataType::StrongPointer => {
            if let Ok(ptr) = reader.read_type::<Pointer>()
                && !ptr.is_null()
            {
                let instance_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                let mut sub_reader = SpanReader::new(instance_bytes);
                prescan_struct(db, ptr.struct_index, &mut sub_reader, map, file_name_offset);
            }
        }
        DataType::Reference => {
            if let Ok(reference) = reader.read_type::<Reference>()
                && !reference.is_null()
                && let Some(target) = db.record_by_id(&reference.record_id)
                && !db.is_main_record(target)
                && target.file_name_offset.0 == file_name_offset
            {
                let instance_bytes =
                    db.get_instance(target.struct_index, target.instance_index as i32);
                let mut sub_reader = SpanReader::new(instance_bytes);
                prescan_struct(
                    db,
                    target.struct_index,
                    &mut sub_reader,
                    map,
                    file_name_offset,
                );
            }
        }
        DataType::Class => {
            prescan_struct(db, prop_struct_index, reader, map, file_name_offset);
        }
        other => {
            let _ = reader.advance(other.inline_size());
        }
    }
}

fn prescan_array(
    db: &Database,
    data_type: DataType,
    prop_struct_index: i32,
    reader: &mut SpanReader,
    map: &mut FxHashMap<(i32, i32), usize>,
    file_name_offset: i32,
) {
    let count = reader.read_i32().unwrap_or(0);
    let first_index = reader.read_i32().unwrap_or(0);

    for i in first_index..first_index + count {
        let idx = i as usize;
        match data_type {
            DataType::WeakPointer => {
                let ptr = &db.weak_values[idx];
                if !ptr.is_null() {
                    let key = (ptr.struct_index, ptr.instance_index);
                    if !map.contains_key(&key) {
                        let id = map.len() + 1;
                        map.insert(key, id);
                    }
                }
            }
            DataType::StrongPointer => {
                let ptr = &db.strong_values[idx];
                if !ptr.is_null() {
                    let instance_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(instance_bytes);
                    prescan_struct(db, ptr.struct_index, &mut sub_reader, map, file_name_offset);
                }
            }
            DataType::Reference => {
                let reference = &db.reference_values[idx];
                if !reference.is_null()
                    && let Some(target) = db.record_by_id(&reference.record_id)
                    && !db.is_main_record(target)
                    && target.file_name_offset.0 == file_name_offset
                {
                    let instance_bytes =
                        db.get_instance(target.struct_index, target.instance_index as i32);
                    let mut sub_reader = SpanReader::new(instance_bytes);
                    prescan_struct(
                        db,
                        target.struct_index,
                        &mut sub_reader,
                        map,
                        file_name_offset,
                    );
                }
            }
            DataType::Class => {
                let instance_bytes = db.get_instance(prop_struct_index, i);
                let mut sub_reader = SpanReader::new(instance_bytes);
                prescan_struct(
                    db,
                    prop_struct_index,
                    &mut sub_reader,
                    map,
                    file_name_offset,
                );
            }
            _ => {
                // No action needed for pre-scan of primitive arrays
            }
        }
    }
}

// ─── Full graph traversal ───────────────────────────────────────────────────

/// Walk a record's object graph, emitting structured data to the sink.
pub fn walk_record<S: ExportSink>(
    db: &Database,
    record: &Record,
    sink: &mut S,
) -> Result<(), ExportError>
where
    ExportError: From<S::Error>,
{
    // 1. Pre-scan weak pointers (skip if struct can't contain any)
    let weak_pointers = if db.struct_has_weak_pointers(record.struct_index) {
        prescan_weak_pointers(db, record)
    } else {
        FxHashMap::default()
    };
    let pointed_to: FxHashSet<(i32, i32)> = weak_pointers.keys().copied().collect();

    let mut ctx = WalkContext {
        weak_pointers,
        pointed_to,
        file_name_offset: record.file_name_offset.0,
    };

    let record_name = db.resolve_string2(record.name_offset);

    // 2. Top-level object
    sink.begin_object(None)?;
    sink.write_str(Some("_RecordName_"), record_name)?;
    sink.write_guid(Some("_RecordId_"), &record.id)?;

    // 2b. Record tag/domain (v8+)
    if record.tag_offset.0 != -1 {
        let tag = db.resolve_string2(record.tag_offset);
        sink.write_str(Some("_RecordTag_"), tag)?;
    }

    // 3. _RecordValue_ object
    sink.begin_object(Some("_RecordValue_"))?;
    walk_instance(
        db,
        record.struct_index,
        record.instance_index as i32,
        sink,
        &mut ctx,
    )?;
    sink.end_object()?;

    // 4. If there are unvisited weak pointer targets, write _Pointers_ fallback
    if !ctx.pointed_to.is_empty() {
        sink.begin_object(Some("_Pointers_"))?;
        // Collect remaining targets sorted by ID for deterministic output
        let mut remaining: Vec<_> = ctx
            .pointed_to
            .iter()
            .filter_map(|key| ctx.weak_pointers.get(key).map(|id| (*key, *id)))
            .collect();
        remaining.sort_by_key(|&(_, id)| id);

        for ((struct_index, instance_index), id) in remaining {
            let mut label_buf = [0u8; 24];
            let label = write_ptr_label(&mut label_buf, id);
            sink.begin_object(Some(label))?;
            let struct_name = db.resolve_string2(db.struct_def(struct_index).name_offset);
            sink.write_str(Some("_Type_"), struct_name)?;
            let instance_bytes = db.get_instance(struct_index, instance_index);
            let mut reader = SpanReader::new(instance_bytes);
            walk_struct_fields(db, struct_index, &mut reader, sink, &mut ctx)?;
            sink.end_object()?;
        }
        sink.end_object()?;
    }

    // 5. End top-level object
    sink.end_object()?;

    Ok(())
}

fn walk_instance<S: ExportSink>(
    db: &Database,
    struct_index: i32,
    instance_index: i32,
    sink: &mut S,
    ctx: &mut WalkContext,
) -> Result<(), ExportError>
where
    ExportError: From<S::Error>,
{
    let instance_bytes = db.get_instance(struct_index, instance_index);
    let mut reader = SpanReader::new(instance_bytes);

    // If this instance is a weak pointer target, emit _Pointer_ and mark as visited
    let key = (struct_index, instance_index);
    if let Some(&id) = ctx.weak_pointers.get(&key) {
        let mut buf = [0u8; 24];
        let label = write_ptr_label(&mut buf, id);
        sink.write_str(Some("_Pointer_"), label)?;
        ctx.pointed_to.remove(&key);
    }

    // Write _Type_ and fields
    let struct_name = db.resolve_string2(db.struct_def(struct_index).name_offset);
    sink.write_str(Some("_Type_"), struct_name)?;
    walk_struct_fields(db, struct_index, &mut reader, sink, ctx)?;

    Ok(())
}

fn walk_struct_fields<S: ExportSink>(
    db: &Database,
    struct_index: i32,
    reader: &mut SpanReader,
    sink: &mut S,
    ctx: &mut WalkContext,
) -> Result<(), ExportError>
where
    ExportError: From<S::Error>,
{
    let prop_indices = db.all_property_indices(struct_index);
    let property_defs = db.property_defs();
    for &idx in prop_indices {
        let prop = &property_defs[idx as usize];
        let data_type = DataType::try_from(prop.data_type)?;
        let conv_type = ConversionType::try_from(prop.conversion_type)?;

        if conv_type == ConversionType::Attribute {
            walk_attribute(
                db,
                data_type,
                prop.struct_index as i32,
                prop.name_offset,
                reader,
                sink,
                ctx,
            )?;
        } else {
            walk_array(
                db,
                data_type,
                prop.struct_index as i32,
                prop.name_offset,
                reader,
                sink,
                ctx,
            )?;
        }
    }
    Ok(())
}

fn walk_attribute<S: ExportSink>(
    db: &Database,
    data_type: DataType,
    prop_struct_index: i32,
    name_offset: crate::types::StringId2,
    reader: &mut SpanReader,
    sink: &mut S,
    ctx: &mut WalkContext,
) -> Result<(), ExportError>
where
    ExportError: From<S::Error>,
{
    let name = db.resolve_string2(name_offset);

    match data_type {
        DataType::Boolean => {
            let val = reader.read_bool()?;
            sink.write_bool(Some(name), val)?;
        }
        DataType::SByte => {
            let val = reader.read_i8()?;
            sink.write_i8(Some(name), val)?;
        }
        DataType::Int16 => {
            let val = reader.read_i16()?;
            sink.write_i16(Some(name), val)?;
        }
        DataType::Int32 => {
            let val = reader.read_i32()?;
            sink.write_i32(Some(name), val)?;
        }
        DataType::Int64 => {
            let val = reader.read_i64()?;
            sink.write_i64(Some(name), val)?;
        }
        DataType::Byte => {
            let val = reader.read_u8()?;
            sink.write_u8(Some(name), val)?;
        }
        DataType::UInt16 => {
            let val = reader.read_u16()?;
            sink.write_u16(Some(name), val)?;
        }
        DataType::UInt32 => {
            let val = reader.read_u32()?;
            sink.write_u32(Some(name), val)?;
        }
        DataType::UInt64 => {
            let val = reader.read_u64()?;
            sink.write_u64(Some(name), val)?;
        }
        DataType::Single => {
            let val = reader.read_f32()?;
            sink.write_f32(Some(name), val)?;
        }
        DataType::Double => {
            let val = reader.read_f64()?;
            sink.write_f64(Some(name), val)?;
        }
        DataType::String => {
            let sid = reader.read_type::<StringId>()?;
            let s = db.resolve_string(*sid);
            sink.write_str(Some(name), s)?;
        }
        DataType::Locale => {
            let sid = reader.read_type::<StringId>()?;
            let s = db.resolve_string(*sid);
            sink.write_str(Some(name), s)?;
        }
        DataType::EnumChoice => {
            let sid = reader.read_type::<StringId>()?;
            let s = db.resolve_string(*sid);
            sink.write_str(Some(name), s)?;
        }
        DataType::Guid => {
            let guid = reader.read_type::<CigGuid>()?;
            sink.write_guid(Some(name), guid)?;
        }
        DataType::Class => {
            sink.begin_object(Some(name))?;
            let struct_name = db.resolve_string2(db.struct_def(prop_struct_index).name_offset);
            sink.write_str(Some("_Type_"), struct_name)?;
            walk_struct_fields(db, prop_struct_index, reader, sink, ctx)?;
            sink.end_object()?;
        }
        DataType::StrongPointer => {
            let ptr = reader.read_type::<Pointer>()?;
            if ptr.is_null() {
                sink.write_null(Some(name))?;
            } else {
                sink.begin_object(Some(name))?;
                walk_instance(db, ptr.struct_index, ptr.instance_index, sink, ctx)?;
                sink.end_object()?;
            }
        }
        DataType::WeakPointer => {
            let ptr = reader.read_type::<Pointer>()?;
            if ptr.is_null() {
                sink.write_null(Some(name))?;
            } else {
                let key = (ptr.struct_index, ptr.instance_index);
                if let Some(&id) = ctx.weak_pointers.get(&key) {
                    let mut buf = [0u8; 32];
                    let label = write_points_to_label(&mut buf, id);
                    sink.write_str(Some(name), label)?;
                } else {
                    sink.write_null(Some(name))?;
                }
            }
        }
        DataType::Reference => {
            let reference = reader.read_type::<Reference>()?;
            resolve_reference(db, reference, Some(name), sink, ctx)?;
        }
    }

    Ok(())
}

fn walk_array<S: ExportSink>(
    db: &Database,
    data_type: DataType,
    prop_struct_index: i32,
    name_offset: crate::types::StringId2,
    reader: &mut SpanReader,
    sink: &mut S,
    ctx: &mut WalkContext,
) -> Result<(), ExportError>
where
    ExportError: From<S::Error>,
{
    let name = db.resolve_string2(name_offset);
    let count = reader.read_i32()?;
    let first_index = reader.read_i32()?;

    sink.begin_array(name)?;

    for i in first_index..first_index + count {
        let idx = i as usize;
        match data_type {
            DataType::Boolean => {
                let val = db.get_bool(idx)?;
                sink.write_bool(None, val)?;
            }
            DataType::SByte => {
                let val = db.get_int8(idx)?;
                sink.write_i8(None, val)?;
            }
            DataType::Int16 => {
                let val = db.get_int16(idx)?;
                sink.write_i16(None, val)?;
            }
            DataType::Int32 => {
                let val = db.get_int32(idx)?;
                sink.write_i32(None, val)?;
            }
            DataType::Int64 => {
                let val = db.get_int64(idx)?;
                sink.write_i64(None, val)?;
            }
            DataType::Byte => {
                let val = db.get_uint8(idx)?;
                sink.write_u8(None, val)?;
            }
            DataType::UInt16 => {
                let val = db.get_uint16(idx)?;
                sink.write_u16(None, val)?;
            }
            DataType::UInt32 => {
                let val = db.get_uint32(idx)?;
                sink.write_u32(None, val)?;
            }
            DataType::UInt64 => {
                let val = db.get_uint64(idx)?;
                sink.write_u64(None, val)?;
            }
            DataType::Single => {
                let val = db.get_single(idx)?;
                sink.write_f32(None, val)?;
            }
            DataType::Double => {
                let val = db.get_double(idx)?;
                sink.write_f64(None, val)?;
            }
            DataType::String => {
                let sid = db.string_id_values[idx];
                let s = db.resolve_string(sid);
                sink.write_str(None, s)?;
            }
            DataType::Locale => {
                let sid = db.locale_values[idx];
                let s = db.resolve_string(sid);
                sink.write_str(None, s)?;
            }
            DataType::EnumChoice => {
                let sid = db.enum_values[idx];
                let s = db.resolve_string(sid);
                sink.write_str(None, s)?;
            }
            DataType::Guid => {
                let guid = &db.guid_values[idx];
                sink.write_guid(None, guid)?;
            }
            DataType::Class => {
                sink.begin_object(None)?;
                walk_instance(db, prop_struct_index, i, sink, ctx)?;
                sink.end_object()?;
            }
            DataType::StrongPointer => {
                let ptr = &db.strong_values[idx];
                if ptr.is_null() {
                    sink.write_null(None)?;
                } else {
                    sink.begin_object(None)?;
                    walk_instance(db, ptr.struct_index, ptr.instance_index, sink, ctx)?;
                    sink.end_object()?;
                }
            }
            DataType::WeakPointer => {
                let ptr = &db.weak_values[idx];
                if ptr.is_null() {
                    sink.write_null(None)?;
                } else {
                    let key = (ptr.struct_index, ptr.instance_index);
                    if let Some(&id) = ctx.weak_pointers.get(&key) {
                        let mut buf = [0u8; 32];
                        let label = write_points_to_label(&mut buf, id);
                        sink.write_str(None, label)?;
                    } else {
                        sink.write_null(None)?;
                    }
                }
            }
            DataType::Reference => {
                let reference = &db.reference_values[idx];
                resolve_reference(db, reference, None, sink, ctx)?;
            }
        }
    }

    sink.end_array()?;
    Ok(())
}

fn resolve_reference<S: ExportSink>(
    db: &Database,
    reference: &Reference,
    name: Option<&str>,
    sink: &mut S,
    ctx: &mut WalkContext,
) -> Result<(), ExportError>
where
    ExportError: From<S::Error>,
{
    if reference.is_null() {
        sink.write_null(name)?;
        return Ok(());
    }

    let target = match db.record_by_id(&reference.record_id) {
        Some(r) => r,
        None => {
            sink.write_null(name)?;
            return Ok(());
        }
    };

    let target_file_name = db.resolve_string(target.file_name_offset);
    let context_file_name = resolve_file_name_by_offset(db, ctx.file_name_offset);
    let target_record_name = db.resolve_string2(target.name_offset);

    if db.is_main_record(target) {
        let mut path_buf = compute_relative_path_buf(target_file_name, context_file_name);
        change_extension(&mut path_buf, "json");
        sink.write_record_ref(name, &target.id, target_record_name, &path_buf)?;
        return Ok(());
    }

    if target.file_name_offset.0 == ctx.file_name_offset {
        // Same-file sub-record: walk inline
        sink.begin_object(name)?;
        sink.write_guid(Some("_RecordId_"), &target.id)?;
        sink.write_str(Some("_RecordName_"), target_record_name)?;
        walk_instance(
            db,
            target.struct_index,
            target.instance_index as i32,
            sink,
            ctx,
        )?;
        sink.end_object()?;
        return Ok(());
    }

    // Cross-file sub-record: emit path + metadata
    sink.begin_object(name)?;
    let mut path_buf = compute_relative_path_buf(target_file_name, context_file_name);
    change_extension(&mut path_buf, "json");
    sink.write_str(Some("_RecordPath_"), &path_buf)?;
    sink.write_str(Some("_RecordName_"), target_record_name)?;
    sink.write_guid(Some("_RecordId_"), &target.id)?;
    sink.end_object()?;

    Ok(())
}

/// Look up the file name string for a given raw string table offset.
fn resolve_file_name_by_offset<'a>(db: &'a Database, offset: i32) -> &'a str {
    db.resolve_string(StringId(offset))
}

/// Replace (or append) the file extension in `path`, matching C#'s
/// `Path.ChangeExtension(path, ext)` behaviour: find the last `.` after the
/// last `/` and replace from there; if no `.` exists, append `.{ext}`.
pub fn change_extension(path: &mut String, ext: &str) {
    let last_slash = path.rfind('/').map_or(0, |i| i + 1);
    if let Some(dot) = path[last_slash..].rfind('.') {
        path.truncate(last_slash + dot);
    }
    path.push('.');
    path.push_str(ext);
}

/// Compute a relative path from a context file to a target file.
/// Matching the C# DataCoreUtils.ComputeRelativePath behavior.
pub fn compute_relative_path_buf(file_path: &str, context_file_name: &str) -> String {
    let slashes = context_file_name.chars().filter(|&c| c == '/').count();
    let mut result = String::from("file://./");
    for _ in 0..slashes {
        result.push_str("../");
    }
    result.push_str(file_path);
    result
}
