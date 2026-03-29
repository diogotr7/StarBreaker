//! unp4k-compatible XML export.
//!
//! Produces XML matching the community's unforge/unp4k tool output:
//! - Scalar fields → XML attributes on the parent element
//! - Arrays → child elements named by data type (`<Int32>`, `<Bool>`, etc.)
//! - Empty elements are omitted
//! - Root element: record name, with `__type`, `__ref`, `__path` appended as attributes
//! - Strong pointers: inlined (cycle-protected)
//! - References: inlined up to depth 1, then GUID attribute
//! - Weak pointers: `"StructName[XXXX]"` format
//! - Booleans in arrays: `1` / `0`
//! - No XML declaration

use std::io::Write;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use rustc_hash::FxHashSet;

use crate::database::Database;
use crate::enums::{ConversionType, DataType};
use crate::error::ExportError;
use crate::reader::SpanReader;
use crate::types::{CigGuid, Pointer, Record, Reference, StringId};

/// Export a record to unp4k-compatible XML bytes.
pub fn to_unp4k_xml(db: &Database, record: &Record) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    write_unp4k_xml(db, record, &mut buf)?;
    Ok(buf)
}

/// Export a record as unp4k-compatible XML to an arbitrary writer.
pub fn write_unp4k_xml(
    db: &Database,
    record: &Record,
    w: impl Write,
) -> Result<(), ExportError> {
    let writer = Writer::new_with_indent(w, b' ', 2);
    let mut ctx = Ctx {
        db,
        writer,
        struct_stack: FxHashSet::default(),
    };

    let record_name = db.resolve_string2(record.name_offset);
    let struct_name = db.resolve_string2(db.struct_def(record.struct_index).name_offset);
    let file_name = db.resolve_string(record.file_name_offset);

    // Walk the struct to collect attributes and children
    let attrs_and_children =
        walk_struct_at_index(&mut ctx, record.struct_index, record.instance_index as u32)?;

    // Write root element with all attributes + __type, __ref, __path
    let mut start = BytesStart::new(record_name);
    for (name, value) in &attrs_and_children.attrs {
        start.push_attribute((name.as_str(), value.as_str()));
    }
    start.push_attribute(("__type", struct_name));
    start.push_attribute(("__ref", record.id.to_string().as_str()));
    start.push_attribute(("__path", file_name));

    if attrs_and_children.children.is_empty() {
        ctx.writer.write_event(Event::Empty(start))?;
    } else {
        ctx.writer
            .write_event(Event::Start(start))?;
        for child in &attrs_and_children.children {
            write_xml_node(&mut ctx.writer, child)?;
        }
        ctx.writer
            .write_event(Event::End(BytesEnd::new(record_name)))?;
    }

    Ok(())
}

// ── Internal types ─────────────────────────────────────────────────────────

struct Ctx<'a, W: Write> {
    db: &'a Database<'a>,
    writer: Writer<W>,
    /// Cycle detection for struct pointer traversal (depth capped at 100).
    struct_stack: FxHashSet<(u32, u32)>,
}

/// Buffered XML output for a struct — attributes must come before children.
struct StructOutput {
    attrs: Vec<(String, String)>,
    children: Vec<XmlNode>,
}

/// A buffered XML element: `<tag attr="val"...>children...</tag>` or self-closing.
struct XmlNode {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<XmlNode>,
}

/// Helper: `<Tag value="val"/>` — unp4k's CreateElementWithValue for primitives.
fn value_element(tag: &str, val: impl ToString) -> XmlNode {
    let s = val.to_string();
    if s.is_empty() {
        return XmlNode {
            tag: tag.to_owned(),
            attrs: Vec::new(),
            children: Vec::new(),
        };
    }
    XmlNode {
        tag: tag.to_owned(),
        attrs: vec![("value".to_owned(), s)],
        children: Vec::new(),
    }
}

fn write_xml_node<W: Write>(writer: &mut Writer<W>, node: &XmlNode) -> Result<(), std::io::Error> {
    let mut start = BytesStart::new(node.tag.as_str());
    for (k, v) in &node.attrs {
        start.push_attribute((k.as_str(), v.as_str()));
    }
    if node.children.is_empty() {
        writer.write_event(Event::Empty(start))?;
    } else {
        writer.write_event(Event::Start(start))?;
        for child in &node.children {
            write_xml_node(writer, child)?;
        }
        writer
            .write_event(Event::End(BytesEnd::new(node.tag.as_str())))?;
    }
    Ok(())
}

// ── Walk logic ─────────────────────────────────────────────────────────────

/// Walk a struct at a given (struct_index, variant_index), returning buffered output.
fn walk_struct_at_index<W: Write>(
    ctx: &mut Ctx<W>,
    struct_index: i32,
    variant_index: u32,
) -> Result<StructOutput, ExportError> {
    let key = (struct_index as u32, variant_index);
    if ctx.struct_stack.len() >= 100 || ctx.struct_stack.contains(&key) {
        return Ok(StructOutput {
            attrs: Vec::new(),
            children: Vec::new(),
        });
    }
    ctx.struct_stack.insert(key);

    let instance = ctx.db.get_instance(struct_index, variant_index as i32);
    let mut reader = SpanReader::new(instance);

    let result = walk_struct_fields(ctx, struct_index, &mut reader)?;

    ctx.struct_stack.remove(&key);
    Ok(result)
}

/// Walk all properties of a struct, returning attributes and child elements.
fn walk_struct_fields<W: Write>(
    ctx: &mut Ctx<W>,
    struct_index: i32,
    reader: &mut SpanReader,
) -> Result<StructOutput, ExportError> {
    let mut output = StructOutput {
        attrs: Vec::new(),
        children: Vec::new(),
    };

    let prop_indices = ctx.db.all_property_indices(struct_index);
    let property_defs = ctx.db.property_defs();

    for &idx in prop_indices {
        let prop = &property_defs[idx as usize];
        let data_type = DataType::try_from(prop.data_type)?;
        let conv_type = ConversionType::try_from(prop.conversion_type)?;
        let name = ctx.db.resolve_string2(prop.name_offset);

        if conv_type == ConversionType::Attribute {
            walk_attribute(ctx, data_type, prop.struct_index as i32, name, reader, &mut output)?;
        } else {
            walk_array(ctx, data_type, prop.struct_index as i32, name, reader, &mut output)?;
        }
    }

    Ok(output)
}

/// Walk a scalar attribute — emits an XML attribute or child element.
fn walk_attribute<W: Write>(
    ctx: &mut Ctx<W>,
    dt: DataType,
    prop_struct_index: i32,
    name: &str,
    reader: &mut SpanReader,
    output: &mut StructOutput,
) -> Result<(), ExportError> {
    match dt {
        // Scalars → XML attributes
        DataType::Boolean => {
            let v = reader.read_bool()?;
            output.attrs.push((name.to_owned(), format!("{}", v as u8)));
        }
        DataType::SByte => {
            let v = reader.read_i8()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::Int16 => {
            let v = reader.read_i16()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::Int32 => {
            let v = reader.read_i32()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::Int64 => {
            let v = reader.read_i64()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::Byte => {
            let v = reader.read_u8()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::UInt16 => {
            let v = reader.read_u16()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::UInt32 => {
            let v = reader.read_u32()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::UInt64 => {
            let v = reader.read_u64()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::Single => {
            let v = reader.read_f32()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::Double => {
            let v = reader.read_f64()?;
            output.attrs.push((name.to_owned(), v.to_string()));
        }
        DataType::String | DataType::Locale | DataType::EnumChoice => {
            let sid = reader.read_type::<StringId>()?;
            let s = ctx.db.resolve_string(*sid);
            if !s.is_empty() {
                output.attrs.push((name.to_owned(), s.to_owned()));
            }
        }
        DataType::Guid => {
            let guid = reader.read_type::<CigGuid>()?;
            if *guid != CigGuid::EMPTY {
                output.attrs.push((name.to_owned(), guid.to_string()));
            }
        }

        // Complex types → child elements
        DataType::Class => {
            let sub = walk_struct_fields(ctx, prop_struct_index, reader)?;
            if !sub.attrs.is_empty() || !sub.children.is_empty() {
                output.children.push(XmlNode {
                    tag: name.to_owned(),
                    attrs: sub.attrs,
                    children: sub.children,
                });
            }
        }
        DataType::StrongPointer => {
            let ptr = reader.read_type::<Pointer>()?;
            if !ptr.is_null() {
                let struct_def = ctx.db.struct_def(ptr.struct_index);
                let struct_name = ctx.db.resolve_string2(struct_def.name_offset);
                // unp4k reads VariantIndex as UInt16
                let variant = ptr.instance_index as u16 as u32;
                let sub = walk_struct_at_index(ctx, ptr.struct_index, variant)?;
                if !sub.attrs.is_empty() || !sub.children.is_empty() {
                    // Wrap: <fieldName><StructName attrs...>children</StructName></fieldName>
                    let inner = XmlNode {
                        tag: struct_name.to_owned(),
                        attrs: sub.attrs,
                        children: sub.children,
                    };
                    output.children.push(XmlNode {
                        tag: name.to_owned(),
                        attrs: Vec::new(),
                        children: vec![inner],
                    });
                }
            }
        }
        DataType::WeakPointer => {
            let ptr = reader.read_type::<Pointer>()?;
            if !ptr.is_null() {
                let struct_def = ctx.db.struct_def(ptr.struct_index);
                let struct_name = ctx.db.resolve_string2(struct_def.name_offset);
                // unp4k reads VariantIndex as UInt16 — use low 16 bits only
                let variant = ptr.instance_index as u16;
                let label = format!("{struct_name}[{variant:04X}]");
                // unp4k uses CreateElementWithValue → <name value="StructName[XXXX]"/>
                output.children.push(XmlNode {
                    tag: name.to_owned(),
                    attrs: vec![("value".to_owned(), label)],
                    children: Vec::new(),
                });
            }
        }
        DataType::Reference => {
            // unp4k: references are GUID attributes, skipped when GUID is empty.
            // unp4k's IsNull checks `Value == Guid.Empty` (just the GUID, not instance_index).
            let reference = reader.read_type::<Reference>()?;
            if reference.record_id != CigGuid::EMPTY {
                output
                    .attrs
                    .push((name.to_owned(), reference.record_id.to_string()));
            }
        }
    }
    Ok(())
}

/// Walk an array property — emits a container element with type-named children.
fn walk_array<W: Write>(
    ctx: &mut Ctx<W>,
    dt: DataType,
    prop_struct_index: i32,
    name: &str,
    reader: &mut SpanReader,
    output: &mut StructOutput,
) -> Result<(), ExportError> {
    let count = reader.read_i32()?;
    let first_index = reader.read_i32()?;

    let mut children = Vec::new();

    for i in first_index..first_index + count {
        let idx = i as usize;
        let child = match dt {
            DataType::Boolean => Some(value_element("Bool", if ctx.db.get_bool(idx)? { 1 } else { 0 })),
            DataType::Single =>  Some(value_element("Single", ctx.db.get_single(idx)?)),
            DataType::Double =>  Some(value_element("Double", ctx.db.get_double(idx)?)),
            DataType::SByte =>   Some(value_element("Int8", ctx.db.get_int8(idx)?)),
            DataType::Int16 =>   Some(value_element("Int16", ctx.db.get_int16(idx)?)),
            DataType::Int32 =>   Some(value_element("Int32", ctx.db.get_int32(idx)?)),
            DataType::Int64 =>   Some(value_element("Int64", ctx.db.get_int64(idx)?)),
            DataType::Byte =>    Some(value_element("UInt8", ctx.db.get_uint8(idx)?)),
            DataType::UInt16 =>  Some(value_element("UInt16", ctx.db.get_uint16(idx)?)),
            DataType::UInt32 =>  Some(value_element("UInt32", ctx.db.get_uint32(idx)?)),
            DataType::UInt64 =>  Some(value_element("UInt64", ctx.db.get_uint64(idx)?)),
            DataType::Guid =>    Some(value_element("Guid", &ctx.db.guid_values[idx])),
            DataType::String => {
                let s = ctx.db.resolve_string(ctx.db.string_id_values[idx]);
                if s.is_empty() { None } else { Some(value_element("String", s)) }
            }
            DataType::Locale => {
                let s = ctx.db.resolve_string(ctx.db.locale_values[idx]);
                if s.is_empty() { None } else { Some(value_element("LocID", s)) }
            }
            DataType::EnumChoice => {
                let s = ctx.db.resolve_string(ctx.db.enum_values[idx]);
                if s.is_empty() { None } else { Some(value_element("Enum", s)) }
            }
            DataType::Reference => {
                let reference = &ctx.db.reference_values[idx];
                if reference.record_id == CigGuid::EMPTY {
                    None
                } else {
                    // unp4k: <Reference value="guid"/>
                    Some(XmlNode {
                        tag: "Reference".to_owned(),
                        attrs: vec![("value".to_owned(), reference.record_id.to_string())],
                        children: Vec::new(),
                    })
                }
            }
            DataType::StrongPointer => {
                let ptr = &ctx.db.strong_values[idx];
                if ptr.is_null() {
                    None
                } else {
                    let struct_def = ctx.db.struct_def(ptr.struct_index);
                    let struct_name = ctx.db.resolve_string2(struct_def.name_offset);
                    let sub = walk_struct_at_index(
                        ctx,
                        ptr.struct_index,
                        ptr.instance_index as u16 as u32,
                    )?;
                    if !sub.attrs.is_empty() || !sub.children.is_empty() {
                        Some(XmlNode {
                            tag: struct_name.to_owned(),
                            attrs: sub.attrs,
                            children: sub.children,
                        })
                    } else {
                        None
                    }
                }
            }
            DataType::WeakPointer => {
                let ptr = &ctx.db.weak_values[idx];
                if ptr.is_null() {
                    None
                } else {
                    let struct_def = ctx.db.struct_def(ptr.struct_index);
                    let struct_name = ctx.db.resolve_string2(struct_def.name_offset);
                    let sub = walk_struct_at_index(
                        ctx,
                        ptr.struct_index,
                        ptr.instance_index as u16 as u32,
                    )?;
                    if !sub.attrs.is_empty() || !sub.children.is_empty() {
                        Some(XmlNode {
                            tag: struct_name.to_owned(),
                            attrs: sub.attrs,
                            children: sub.children,
                        })
                    } else {
                        None
                    }
                }
            }
            DataType::Class => {
                let struct_def = ctx.db.struct_def(prop_struct_index);
                let struct_name = ctx.db.resolve_string2(struct_def.name_offset);
                let sub = walk_struct_at_index(ctx, prop_struct_index, i as u32)?;
                if !sub.attrs.is_empty() || !sub.children.is_empty() {
                    Some(XmlNode {
                        tag: struct_name.to_owned(),
                        attrs: sub.attrs,
                        children: sub.children,
                    })
                } else {
                    None
                }
            }
        };

        if let Some(c) = child {
            children.push(c);
        }
    }

    if !children.is_empty() {
        output.children.push(XmlNode {
            tag: name.to_owned(),
            attrs: Vec::new(),
            children,
        });
    }

    Ok(())
}
