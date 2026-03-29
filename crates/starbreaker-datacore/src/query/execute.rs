use crate::database::Database;
use crate::enums::{ConversionType, DataType};
use crate::error::QueryError;
use crate::query::compile::{CompiledMultiPath, CompiledPath, ResolvedSegment};
use crate::query::from_datacore::FromDataCore;
use crate::query::value::Value;
use crate::types::{CigGuid, Pointer, Record, Reference};
use starbreaker_common::SpanReader;

/// Execute a compiled path query against a record, expecting exactly one result.
///
/// Returns `QueryError::StructMismatch` if the record's struct index does not
/// match the compiled path's root. Returns `QueryError::CardinalityMismatch`
/// if the path resolves to zero or more than one value.
pub fn query_one<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    path: &CompiledPath,
    record: &Record,
) -> Result<T, QueryError> {
    let results = query_all::<T>(db, path, record)?;
    if results.len() != 1 {
        return Err(QueryError::CardinalityMismatch {
            count: results.len(),
        });
    }
    results.into_iter().next().ok_or(QueryError::CardinalityMismatch { count: 0 })
}

/// Execute a compiled path query against a record, returning all matching values.
///
/// Returns `QueryError::StructMismatch` if the record's struct index does not
/// match the compiled path's root.
pub fn query_all<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    path: &CompiledPath,
    record: &Record,
) -> Result<Vec<T>, QueryError> {
    // Validate root struct index matches
    if record.struct_index != path.root_struct_index {
        return Err(QueryError::StructMismatch {
            expected: path.root_struct_index,
            actual: record.struct_index,
        });
    }

    // Get the record's instance data
    let instance_bytes = db.get_instance(record.struct_index, record.instance_index as i32);
    let mut reader = SpanReader::new(instance_bytes);

    let mut results = Vec::new();
    execute_segments::<T>(db, &path.segments, 0, &mut reader, &mut results, false)?;
    Ok(results)
}

/// Execute a compiled path query against a record, returning all matching Value trees,
/// but without expanding References — References are returned as `Value::Guid(record_id)`
/// instead of recursing into the referenced entity. All inline data is still materialized.
///
/// Use this when materializing large entities whose References fan out into enormous graphs.
pub fn query_all_no_refs<'a>(
    db: &'a Database<'a>,
    path: &CompiledPath,
    record: &Record,
) -> Result<Vec<Value<'a>>, QueryError> {
    if record.struct_index != path.root_struct_index {
        return Err(QueryError::StructMismatch {
            expected: path.root_struct_index,
            actual: record.struct_index,
        });
    }
    let instance_bytes = db.get_instance(record.struct_index, record.instance_index as i32);
    let mut reader = SpanReader::new(instance_bytes);
    let mut results = Vec::new();
    execute_segments::<Value>(db, &path.segments, 0, &mut reader, &mut results, true)?;
    Ok(results)
}

/// Execute a compiled path query, returning the first matching value or None.
/// Zero allocation — short-circuits after the first match.
pub fn query_first<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    path: &CompiledPath,
    record: &Record,
) -> Result<Option<T>, QueryError> {
    if record.struct_index != path.root_struct_index {
        return Err(QueryError::StructMismatch {
            expected: path.root_struct_index,
            actual: record.struct_index,
        });
    }
    let instance_bytes = db.get_instance(record.struct_index, record.instance_index as i32);
    let mut reader = SpanReader::new(instance_bytes);
    execute_segments_first::<T>(db, &path.segments, 0, &mut reader, false)
}

/// Execute a multi-field query: traverse the shared prefix once, read all leaf fields.
pub fn query_multi<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    path: &CompiledMultiPath,
    record: &Record,
) -> Result<Vec<Option<T>>, QueryError> {
    if record.struct_index != path.root_struct_index {
        return Err(QueryError::StructMismatch {
            expected: path.root_struct_index,
            actual: record.struct_index,
        });
    }

    let n = path.leaf_segments.len();
    let instance_bytes = db.get_instance(record.struct_index, record.instance_index as i32);
    let mut reader = SpanReader::new(instance_bytes);

    // Traverse prefix to collect byte slices at the leaf struct level.
    let mut leaf_slices = Vec::new();
    collect_leaf_positions(db, &path.prefix_segments, 0, &mut reader, &mut leaf_slices)?;

    let mut results: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for leaf_bytes in &leaf_slices {
        for (i, leaf_seg) in path.leaf_segments.iter().enumerate() {
            if results[i].is_some() {
                continue;
            }
            let offset =
                db.property_byte_offset(leaf_seg.context_struct_index, leaf_seg.property_position);
            let mut leaf_reader = SpanReader::new(leaf_bytes);
            if offset > 0 {
                leaf_reader.advance(offset)?;
            }
            let value = T::read_from_reader(db, &mut leaf_reader, leaf_seg.data_type)?;
            results[i] = Some(value);
        }
        // All found?
        if results.iter().all(|r| r.is_some()) {
            break;
        }
    }

    Ok(results)
}

/// Recursively execute path segments, returning the first leaf value found.
fn execute_segments_first<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    segments: &[ResolvedSegment],
    seg_idx: usize,
    reader: &mut SpanReader,
    skip_references: bool,
) -> Result<Option<T>, QueryError> {
    if seg_idx >= segments.len() {
        return Ok(None);
    }

    let seg = &segments[seg_idx];
    let is_leaf = seg_idx == segments.len() - 1;

    skip_properties(db, seg, reader)?;

    // Helper closure to materialize a struct at a leaf node, respecting skip_references.
    macro_rules! mat {
        ($si:expr, $r:expr) => {
            if skip_references {
                materialize_struct_as_value_no_refs(db, $si, $r)
            } else {
                materialize_struct_as_value(db, $si, $r)
            }
        };
    }

    if seg.conversion_type == ConversionType::Attribute {
        match seg.data_type {
            DataType::Class => {
                if is_leaf {
                    let target_si = seg.target_struct_index
                        .ok_or(QueryError::MissingTargetStructIndex { segment: "Class".to_owned() })?;
                    let val = mat!(target_si, reader)?;
                    Ok(Some(T::from_value(val)?))
                } else {
                    execute_segments_first::<T>(db, segments, seg_idx + 1, reader, skip_references)
                }
            }
            DataType::StrongPointer | DataType::WeakPointer => {
                let ptr = *reader.read_type::<Pointer>()?;
                if ptr.is_null() {
                    return Ok(None);
                }
                if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                    return Ok(None);
                }
                let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                let mut sub_reader = SpanReader::new(target_bytes);
                if is_leaf {
                    let val = mat!(ptr.struct_index, &mut sub_reader)?;
                    Ok(Some(T::from_value(val)?))
                } else {
                    execute_segments_first::<T>(db, segments, seg_idx + 1, &mut sub_reader, skip_references)
                }
            }
            DataType::Reference => {
                let reference = *reader.read_type::<Reference>()?;
                if reference.is_null() {
                    return Ok(None);
                }
                let target_record = match db.record_by_id(&reference.record_id) {
                    Some(r) => r,
                    None => return Ok(None),
                };
                let target_bytes = db.get_instance(
                    target_record.struct_index,
                    target_record.instance_index as i32,
                );
                let mut sub_reader = SpanReader::new(target_bytes);
                if is_leaf {
                    let val = mat!(target_record.struct_index, &mut sub_reader)?;
                    Ok(Some(T::from_value(val)?))
                } else {
                    execute_segments_first::<T>(db, segments, seg_idx + 1, &mut sub_reader, skip_references)
                }
            }
            _ => {
                if is_leaf {
                    let value = T::read_from_reader(db, reader, seg.data_type)?;
                    Ok(Some(value))
                } else {
                    Ok(None)
                }
            }
        }
    } else {
        // Array property
        let count = reader.read_i32()?;
        let first_index = reader.read_i32()?;

        match seg.data_type {
            DataType::StrongPointer => {
                for i in 0..count {
                    let idx = (first_index + i) as usize;
                    let ptr = db.strong_values[idx];
                    if ptr.is_null() {
                        continue;
                    }
                    if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                        continue;
                    }
                    let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(target_bytes);
                    if is_leaf {
                        let val = mat!(ptr.struct_index, &mut sub_reader)?;
                        return Ok(Some(T::from_value(val)?));
                    } else if let Some(v) =
                        execute_segments_first::<T>(db, segments, seg_idx + 1, &mut sub_reader, skip_references)?
                    {
                        return Ok(Some(v));
                    }
                }
            }
            DataType::WeakPointer => {
                for i in 0..count {
                    let idx = (first_index + i) as usize;
                    let ptr = db.weak_values[idx];
                    if ptr.is_null() {
                        continue;
                    }
                    if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                        continue;
                    }
                    let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(target_bytes);
                    if is_leaf {
                        let val = mat!(ptr.struct_index, &mut sub_reader)?;
                        return Ok(Some(T::from_value(val)?));
                    } else if let Some(v) =
                        execute_segments_first::<T>(db, segments, seg_idx + 1, &mut sub_reader, skip_references)?
                    {
                        return Ok(Some(v));
                    }
                }
            }
            DataType::Class => {
                let target_si = seg.target_struct_index
                    .ok_or(QueryError::MissingTargetStructIndex { segment: "ClassArray".to_owned() })?;
                for i in 0..count {
                    let instance_bytes = db.get_instance(target_si, first_index + i);
                    let mut sub_reader = SpanReader::new(instance_bytes);
                    if is_leaf {
                        let val = mat!(target_si, &mut sub_reader)?;
                        return Ok(Some(T::from_value(val)?));
                    } else if let Some(v) =
                        execute_segments_first::<T>(db, segments, seg_idx + 1, &mut sub_reader, skip_references)?
                    {
                        return Ok(Some(v));
                    }
                }
            }
            _ => {
                if is_leaf && count > 0 {
                    let idx = first_index as usize;
                    let value = T::read_from_array(db, idx, seg.data_type)?;
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }
}

/// Traverse prefix segments, collecting byte slices at positions where the prefix ends.
/// Similar to execute_segments but instead of reading leaf values, it collects the
/// instance data slices at the point where all prefix segments have been traversed.
fn collect_leaf_positions<'a>(
    db: &'a Database<'a>,
    segments: &[ResolvedSegment],
    seg_idx: usize,
    reader: &mut SpanReader<'a>,
    out: &mut Vec<&'a [u8]>,
) -> Result<(), QueryError> {
    if seg_idx >= segments.len() {
        // We've traversed all prefix segments. The reader is positioned at the
        // start of the leaf struct. Capture the remaining bytes from the original
        // instance slice (the reader's underlying data from current position).
        out.push(reader.remaining_bytes());
        return Ok(());
    }

    let seg = &segments[seg_idx];
    skip_properties(db, seg, reader)?;

    if seg.conversion_type == ConversionType::Attribute {
        match seg.data_type {
            DataType::Class => collect_leaf_positions(db, segments, seg_idx + 1, reader, out),
            DataType::StrongPointer | DataType::WeakPointer => {
                let ptr = *reader.read_type::<Pointer>()?;
                if ptr.is_null() {
                    return Ok(());
                }
                if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                    return Ok(());
                }
                let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                let mut sub_reader = SpanReader::new(target_bytes);
                collect_leaf_positions(db, segments, seg_idx + 1, &mut sub_reader, out)
            }
            DataType::Reference => {
                let reference = *reader.read_type::<Reference>()?;
                if reference.is_null() {
                    return Ok(());
                }
                let target_record = match db.record_by_id(&reference.record_id) {
                    Some(r) => r,
                    None => return Ok(()),
                };
                let target_bytes = db.get_instance(
                    target_record.struct_index,
                    target_record.instance_index as i32,
                );
                let mut sub_reader = SpanReader::new(target_bytes);
                collect_leaf_positions(db, segments, seg_idx + 1, &mut sub_reader, out)
            }
            _ => Ok(()),
        }
    } else {
        // Array
        let count = reader.read_i32()?;
        let first_index = reader.read_i32()?;

        match seg.data_type {
            DataType::StrongPointer => {
                for i in 0..count {
                    let ptr = db.strong_values[(first_index + i) as usize];
                    if ptr.is_null() {
                        continue;
                    }
                    if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                        continue;
                    }
                    let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(target_bytes);
                    collect_leaf_positions(db, segments, seg_idx + 1, &mut sub_reader, out)?;
                }
            }
            DataType::WeakPointer => {
                for i in 0..count {
                    let ptr = db.weak_values[(first_index + i) as usize];
                    if ptr.is_null() {
                        continue;
                    }
                    if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                        continue;
                    }
                    let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(target_bytes);
                    collect_leaf_positions(db, segments, seg_idx + 1, &mut sub_reader, out)?;
                }
            }
            DataType::Class => {
                let target_si = seg.target_struct_index
                    .ok_or(QueryError::MissingTargetStructIndex { segment: "ClassArray".to_owned() })?;
                for i in 0..count {
                    let instance_bytes = db.get_instance(target_si, first_index + i);
                    let mut sub_reader = SpanReader::new(instance_bytes);
                    collect_leaf_positions(db, segments, seg_idx + 1, &mut sub_reader, out)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Recursively execute path segments, collecting leaf values into `results`.
fn execute_segments<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    segments: &[ResolvedSegment],
    seg_idx: usize,
    reader: &mut SpanReader,
    results: &mut Vec<T>,
    skip_references: bool,
) -> Result<(), QueryError> {
    if seg_idx >= segments.len() {
        return Ok(());
    }

    let seg = &segments[seg_idx];
    let is_leaf = seg_idx == segments.len() - 1;

    // Skip to the target property within the current struct
    skip_properties(db, seg, reader)?;

    // Helper macro to materialize a struct at a leaf node, respecting skip_references.
    macro_rules! mat {
        ($si:expr, $r:expr) => {
            if skip_references {
                materialize_struct_as_value_no_refs(db, $si, $r)
            } else {
                materialize_struct_as_value(db, $si, $r)
            }
        };
    }

    if seg.conversion_type == ConversionType::Attribute {
        // Scalar property
        match seg.data_type {
            DataType::Class => {
                if is_leaf {
                    let target_si = seg.target_struct_index
                        .ok_or(QueryError::MissingTargetStructIndex { segment: "Class".to_owned() })?;
                    let val = mat!(target_si, reader)?;
                    results.push(T::from_value(val)?);
                } else {
                    // Inline struct: the reader is now positioned inside the nested struct.
                    // Don't create a new reader — just continue with the next segment.
                    execute_segments::<T>(db, segments, seg_idx + 1, reader, results, skip_references)?;
                }
            }
            DataType::StrongPointer | DataType::WeakPointer => {
                let ptr = *reader.read_type::<Pointer>()?;
                if ptr.is_null() {
                    return Ok(());
                }
                // For scalar pointers with a type filter, check that the
                // actual pointed-to type matches (equals or inherits from)
                // the filter type. Skip silently if not.
                if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                    return Ok(());
                }
                let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                let mut sub_reader = SpanReader::new(target_bytes);
                if is_leaf {
                    let val = mat!(ptr.struct_index, &mut sub_reader)?;
                    results.push(T::from_value(val)?);
                } else {
                    execute_segments::<T>(db, segments, seg_idx + 1, &mut sub_reader, results, skip_references)?;
                }
            }
            DataType::Reference => {
                let reference = *reader.read_type::<Reference>()?;
                if reference.is_null() {
                    return Ok(());
                }
                let target_record = match db.record_by_id(&reference.record_id) {
                    Some(r) => r,
                    None => return Ok(()),
                };
                let target_bytes = db.get_instance(
                    target_record.struct_index,
                    target_record.instance_index as i32,
                );
                let mut sub_reader = SpanReader::new(target_bytes);
                if is_leaf {
                    let val = mat!(target_record.struct_index, &mut sub_reader)?;
                    results.push(T::from_value(val)?);
                } else {
                    execute_segments::<T>(db, segments, seg_idx + 1, &mut sub_reader, results, skip_references)?;
                }
            }
            _ => {
                // Primitive leaf
                if is_leaf {
                    let value = T::read_from_reader(db, reader, seg.data_type)?;
                    results.push(value);
                }
            }
        }
    } else {
        // Array property (ComplexArray, SimpleArray, or ClassArray)
        let count = reader.read_i32()?;
        let first_index = reader.read_i32()?;

        match seg.data_type {
            DataType::StrongPointer => {
                for i in 0..count {
                    let idx = (first_index + i) as usize;
                    let ptr = db.strong_values[idx];
                    if ptr.is_null() {
                        continue;
                    }
                    if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                        continue;
                    }
                    let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(target_bytes);
                    if is_leaf {
                        let val = mat!(ptr.struct_index, &mut sub_reader)?;
                        results.push(T::from_value(val)?);
                    } else {
                        execute_segments::<T>(db, segments, seg_idx + 1, &mut sub_reader, results, skip_references)?;
                    }
                }
            }
            DataType::WeakPointer => {
                for i in 0..count {
                    let idx = (first_index + i) as usize;
                    let ptr = db.weak_values[idx];
                    if ptr.is_null() {
                        continue;
                    }
                    if !type_matches(db, ptr.struct_index, seg.type_filter_struct_index) {
                        continue;
                    }
                    let target_bytes = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub_reader = SpanReader::new(target_bytes);
                    if is_leaf {
                        let val = mat!(ptr.struct_index, &mut sub_reader)?;
                        results.push(T::from_value(val)?);
                    } else {
                        execute_segments::<T>(db, segments, seg_idx + 1, &mut sub_reader, results, skip_references)?;
                    }
                }
            }
            DataType::Class => {
                // ClassArray: instances stored contiguously in instance data
                let target_si = seg.target_struct_index
                    .ok_or(QueryError::MissingTargetStructIndex { segment: "ClassArray".to_owned() })?;
                for i in 0..count {
                    let instance_bytes = db.get_instance(target_si, first_index + i);
                    let mut sub_reader = SpanReader::new(instance_bytes);
                    if is_leaf {
                        let val = mat!(target_si, &mut sub_reader)?;
                        results.push(T::from_value(val)?);
                    } else {
                        execute_segments::<T>(db, segments, seg_idx + 1, &mut sub_reader, results, skip_references)?;
                    }
                }
            }
            _ => {
                // Primitive array at leaf
                if is_leaf {
                    for i in 0..count {
                        let idx = (first_index + i) as usize;
                        let value = T::read_from_array(db, idx, seg.data_type)?;
                        results.push(value);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Advance the reader to the target property using pre-computed byte offsets. O(1).
fn skip_properties(
    db: &Database,
    seg: &ResolvedSegment,
    reader: &mut SpanReader,
) -> Result<(), QueryError> {
    if seg.property_position > 0 {
        reader.advance(db.property_byte_offset(seg.context_struct_index, seg.property_position))?;
    }
    Ok(())
}

/// Check whether `actual_si` equals `filter_si` or inherits from it.
/// If `filter` is `None`, all types match.
fn type_matches(db: &Database, actual_si: i32, filter: Option<i32>) -> bool {
    let filter_si = match filter {
        Some(si) => si,
        None => return true,
    };

    let mut current = actual_si;
    loop {
        if current == filter_si {
            return true;
        }
        let sd = db.struct_def(current);
        if sd.parent_type_index == -1 {
            return false;
        }
        current = sd.parent_type_index;
    }
}

// ── Value materializer ───────────────────────────────────────────────────────

/// Safety-net depth limit for Value materialization. Inline struct nesting
/// (Class, Pointer) is bounded by the record's data, but deep schemas can
/// exceed small limits. The visited-record set prevents Reference cycles,
/// and the depth limit caps fan-out from following many distinct References.
/// 24 is enough for SubGeometry paths (deepest: ~8 levels within a component,
/// plus ~6 levels of loadout nesting above) without cascading through the
/// entire entity graph.
const MAX_MATERIALIZE_DEPTH: usize = 24;

/// Materialize a struct instance as a Value::Object, reading all fields recursively.
pub fn materialize_struct_as_value<'a>(
    db: &'a Database<'a>,
    struct_index: i32,
    reader: &mut SpanReader,
) -> Result<Value<'a>, QueryError> {
    let mut visited = std::collections::HashSet::new();
    materialize_struct_depth(db, struct_index, reader, 0, &mut visited, false)
}

/// Materialize a struct instance as a Value::Object, but return `Value::Guid(record_id)`
/// for any Reference fields instead of recursing into them. All inline data (structs,
/// arrays, pointers) is still fully materialized. Use this to avoid memory explosions
/// when materializing entities whose References fan out into large entity graphs.
pub fn materialize_struct_as_value_no_refs<'a>(
    db: &'a Database<'a>,
    struct_index: i32,
    reader: &mut SpanReader,
) -> Result<Value<'a>, QueryError> {
    let mut visited = std::collections::HashSet::new();
    materialize_struct_depth(db, struct_index, reader, 0, &mut visited, true)
}

fn materialize_struct_depth<'a>(
    db: &'a Database<'a>,
    struct_index: i32,
    reader: &mut SpanReader,
    depth: usize,
    visited: &mut std::collections::HashSet<crate::types::CigGuid>,
    skip_references: bool,
) -> Result<Value<'a>, QueryError> {
    let type_name = db.resolve_string2(db.struct_def(struct_index).name_offset);

    if depth >= MAX_MATERIALIZE_DEPTH {
        log::warn!("Value materialization hit depth limit ({MAX_MATERIALIZE_DEPTH}) at struct '{type_name}'");
        reader.advance(db.struct_def(struct_index).struct_size as usize)?;
        return Ok(Value::Object {
            type_name,
            fields: Vec::new(),
            record_id: None,
        });
    }

    let prop_indices = db.all_property_indices(struct_index);
    let property_defs = db.property_defs();
    let mut fields = Vec::with_capacity(prop_indices.len());

    for &pi in prop_indices {
        let prop = &property_defs[pi as usize];
        let name = db.resolve_string2(prop.name_offset);
        let dt = DataType::try_from(prop.data_type)
            .map_err(|_| QueryError::UnknownType(prop.data_type))?;
        let ct = ConversionType::try_from(prop.conversion_type)
            .map_err(|_| QueryError::UnknownType(prop.conversion_type))?;

        let value = if ct != ConversionType::Attribute {
            materialize_array_depth(db, dt, prop.struct_index as i32, reader, depth, visited, skip_references)?
        } else {
            materialize_attribute_depth(db, dt, prop.struct_index as i32, reader, depth, visited, skip_references)?
        };

        fields.push((name, value));
    }

    Ok(Value::Object { type_name, fields, record_id: None })
}

fn materialize_attribute_depth<'a>(
    db: &'a Database<'a>,
    data_type: DataType,
    prop_struct_index: i32,
    reader: &mut SpanReader,
    depth: usize,
    visited: &mut std::collections::HashSet<crate::types::CigGuid>,
    skip_references: bool,
) -> Result<Value<'a>, QueryError> {
    match data_type {
        DataType::Class => materialize_struct_depth(db, prop_struct_index, reader, depth + 1, visited, skip_references),
        DataType::StrongPointer | DataType::WeakPointer => {
            let ptr = reader.read_type::<Pointer>()?;
            if ptr.is_null() {
                return Ok(Value::Null);
            }
            let instance = db.get_instance(ptr.struct_index, ptr.instance_index);
            let mut sub = SpanReader::new(instance);
            materialize_struct_depth(db, ptr.struct_index, &mut sub, depth + 1, visited, skip_references)
        }
        DataType::Reference => {
            let reference = reader.read_type::<Reference>()?;
            if reference.is_null() {
                return Ok(Value::Null);
            }
            // When skip_references is set, return Guid immediately without recursing.
            if skip_references {
                return Ok(Value::Guid(reference.record_id));
            }
            // Cycle detection: if we've already materialized this record, return Guid.
            if !visited.insert(reference.record_id) {
                return Ok(Value::Guid(reference.record_id));
            }
            if depth >= MAX_MATERIALIZE_DEPTH {
                log::warn!("Reference materialization hit depth limit ({MAX_MATERIALIZE_DEPTH})");
                return Ok(Value::Guid(reference.record_id));
            }
            let result = match db.record_by_id(&reference.record_id) {
                Some(target) => {
                    let inst = db.get_instance(target.struct_index, target.instance_index as i32);
                    let mut sub = SpanReader::new(inst);
                    let mut val = materialize_struct_depth(db, target.struct_index, &mut sub, depth + 1, visited, false)?;
                    if let Value::Object { ref mut record_id, .. } = val {
                        *record_id = Some(reference.record_id);
                    }
                    Ok(val)
                }
                None => Ok(Value::Null),
            };
            // Remove from visited so sibling references to the same record can materialize.
            visited.remove(&reference.record_id);
            result
        }
        DataType::Boolean => Ok(Value::Bool(reader.read_bool()?)),
        DataType::SByte => Ok(Value::Int8(reader.read_i8()?)),
        DataType::Int16 => Ok(Value::Int16(reader.read_i16()?)),
        DataType::Int32 => Ok(Value::Int32(reader.read_i32()?)),
        DataType::Int64 => Ok(Value::Int64(reader.read_i64()?)),
        DataType::Byte => Ok(Value::UInt8(reader.read_u8()?)),
        DataType::UInt16 => Ok(Value::UInt16(reader.read_u16()?)),
        DataType::UInt32 => Ok(Value::UInt32(reader.read_u32()?)),
        DataType::UInt64 => Ok(Value::UInt64(reader.read_u64()?)),
        DataType::Single => Ok(Value::Float(reader.read_f32()?)),
        DataType::Double => Ok(Value::Double(reader.read_f64()?)),
        DataType::String => {
            let sid = *reader.read_type::<crate::types::StringId>()?;
            Ok(Value::String(db.resolve_string(sid)))
        }
        DataType::Locale => {
            let sid = *reader.read_type::<crate::types::StringId>()?;
            Ok(Value::Locale(db.resolve_string(sid)))
        }
        DataType::EnumChoice => {
            let sid = *reader.read_type::<crate::types::StringId>()?;
            Ok(Value::Enum(db.resolve_string(sid)))
        }
        DataType::Guid => {
            let guid = *reader.read_type::<CigGuid>()?;
            Ok(Value::Guid(guid))
        }
    }
}

fn materialize_array_depth<'a>(
    db: &'a Database<'a>,
    data_type: DataType,
    prop_struct_index: i32,
    reader: &mut SpanReader,
    depth: usize,
    visited: &mut std::collections::HashSet<crate::types::CigGuid>,
    skip_references: bool,
) -> Result<Value<'a>, QueryError> {
    let count = reader.read_i32()?;
    let first_index = reader.read_i32()?;
    let mut elements = Vec::with_capacity(count as usize);

    for i in 0..count {
        let idx = (first_index + i) as usize;
        let val = match data_type {
            DataType::Class => {
                let inst = db.get_instance(prop_struct_index, first_index + i);
                let mut sub = SpanReader::new(inst);
                materialize_struct_depth(db, prop_struct_index, &mut sub, depth + 1, visited, skip_references)?
            }
            DataType::StrongPointer => {
                let ptr = &db.strong_values[idx];
                if ptr.is_null() {
                    Value::Null
                } else {
                    let inst = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub = SpanReader::new(inst);
                    materialize_struct_depth(db, ptr.struct_index, &mut sub, depth + 1, visited, skip_references)?
                }
            }
            DataType::WeakPointer => {
                let ptr = &db.weak_values[idx];
                if ptr.is_null() {
                    Value::Null
                } else {
                    let inst = db.get_instance(ptr.struct_index, ptr.instance_index);
                    let mut sub = SpanReader::new(inst);
                    materialize_struct_depth(db, ptr.struct_index, &mut sub, depth + 1, visited, skip_references)?
                }
            }
            DataType::Reference => {
                let reference = &db.reference_values[idx];
                if reference.is_null() {
                    Value::Null
                } else if skip_references {
                    // When skip_references is set, return Guid immediately without recursing.
                    Value::Guid(reference.record_id)
                } else if !visited.insert(reference.record_id) {
                    // Cycle: already materializing this record
                    Value::Guid(reference.record_id)
                } else if depth >= MAX_MATERIALIZE_DEPTH {
                    log::warn!("Array reference materialization hit depth limit ({MAX_MATERIALIZE_DEPTH})");
                    Value::Guid(reference.record_id)
                } else {
                    let result = match db.record_by_id(&reference.record_id) {
                        Some(target) => {
                            let inst =
                                db.get_instance(target.struct_index, target.instance_index as i32);
                            let mut sub = SpanReader::new(inst);
                            let mut val = materialize_struct_depth(db, target.struct_index, &mut sub, depth + 1, visited, false)?;
                            if let Value::Object { ref mut record_id, .. } = val {
                                *record_id = Some(reference.record_id);
                            }
                            val
                        }
                        None => Value::Null,
                    };
                    visited.remove(&reference.record_id);
                    result
                }
            }
            DataType::Boolean => Value::Bool(db.get_bool(idx)?),
            DataType::SByte => Value::Int8(db.get_int8(idx)?),
            DataType::Int16 => Value::Int16(db.get_int16(idx)?),
            DataType::Int32 => Value::Int32(db.get_int32(idx)?),
            DataType::Int64 => Value::Int64(db.get_int64(idx)?),
            DataType::Byte => Value::UInt8(db.get_uint8(idx)?),
            DataType::UInt16 => Value::UInt16(db.get_uint16(idx)?),
            DataType::UInt32 => Value::UInt32(db.get_uint32(idx)?),
            DataType::UInt64 => Value::UInt64(db.get_uint64(idx)?),
            DataType::Single => Value::Float(db.get_single(idx)?),
            DataType::Double => Value::Double(db.get_double(idx)?),
            DataType::String => Value::String(db.resolve_string(db.string_id_values[idx])),
            DataType::Locale => Value::Locale(db.resolve_string(db.locale_values[idx])),
            DataType::EnumChoice => Value::Enum(db.resolve_string(db.enum_values[idx])),
            DataType::Guid => Value::Guid(db.guid_values[idx]),
        };
        elements.push(val);
    }

    Ok(Value::Array(elements))
}
