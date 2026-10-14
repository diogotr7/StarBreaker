//! Classic DataCore XML export matching the legacy C# diff report shape.
//!
//! This module writes records with the record name as the XML root, DataCore
//! metadata as attributes, typed arrays, inline class instances, and legacy
//! weak/reference pointer attributes. It intentionally does not use the generic
//! sink XML shape because the diff command needs byte-stable reports compatible
//! with older StarBreaker exports.

use std::fmt::Display;
use std::io::Write;

use rustc_hash::FxHashMap;

use crate::database::Database;
use crate::enums::{ConversionType, DataType};
use crate::error::ExportError;
use crate::reader::SpanReader;
use crate::types::{Pointer, Record, Reference, StringId};

mod prescan;
mod text;
use prescan::{prescan_weak_pointers, reference_is_null};
use text::{
    data_type_element_name, encode_xml_name, format_bool, format_double, format_single,
    write_escaped_attr, write_escaped_text,
};

/// Export a record to classic DataCore XML bytes.
pub fn to_classic_xml(db: &Database, record: &Record) -> Result<Vec<u8>, ExportError> {
    let mut buf = Vec::new();
    write_classic_xml(db, record, &mut buf)?;
    Ok(buf)
}

/// Export a record as classic DataCore XML to an arbitrary writer.
pub fn write_classic_xml(
    db: &Database,
    record: &Record,
    w: impl Write,
) -> Result<(), ExportError> {
    let pointers = prescan_weak_pointers(db, record);
    let path = db.resolve_string(record.file_name_offset).to_string();
    let mut ctx = ClassicXmlContext::new(db, w, pointers, path, record.file_name_offset.0);

    let root_name = encode_xml_name(db.resolve_string2(record.name_offset));
    let record_id = record.id.to_string();
    let mut attrs = vec![("RecordId", record_id.as_str())];
    let record_tag;
    if record.tag_offset.0 != -1 {
        record_tag = db.resolve_string2(record.tag_offset);
        attrs.push(("RecordTag", record_tag));
    }

    ctx.write_instance_element(
        &root_name,
        &attrs,
        record.struct_index,
        record.instance_index as i32,
    )?;
    ctx.finish()
}

struct ClassicXmlContext<'a, W: Write> {
    db: &'a Database<'a>,
    writer: W,
    pointers: FxHashMap<(i32, i32), usize>,
    path: String,
    file_name_offset: i32,
    indent: usize,
}

impl<'a, W: Write> ClassicXmlContext<'a, W> {
    fn new(
        db: &'a Database<'a>,
        writer: W,
        pointers: FxHashMap<(i32, i32), usize>,
        path: String,
        file_name_offset: i32,
    ) -> Self {
        Self {
            db,
            writer,
            pointers,
            path,
            file_name_offset,
            indent: 0,
        }
    }

    fn finish(mut self) -> Result<(), ExportError> {
        self.writer.flush()?;
        Ok(())
    }

    fn write_instance_element(
        &mut self,
        name: &str,
        base_attrs: &[(&str, &str)],
        struct_index: i32,
        instance_index: i32,
    ) -> Result<(), ExportError> {
        let struct_name = self.db.resolve_string2(self.db.struct_def(struct_index).name_offset);
        let pointer = self.pointer_label(struct_index, instance_index);
        if self.db.all_property_indices(struct_index).is_empty() {
            return self.empty_instance_element(name, base_attrs, pointer.as_deref(), struct_name);
        }
        self.start_instance_element(name, base_attrs, pointer.as_deref(), struct_name)?;

        let instance_bytes = self.db.get_instance(struct_index, instance_index);
        let mut reader = SpanReader::new(instance_bytes);
        self.write_struct_fields(struct_index, &mut reader)?;
        self.end_element(name)
    }

    fn write_struct_element(
        &mut self,
        name: &str,
        base_attrs: &[(&str, &str)],
        struct_index: i32,
        reader: &mut SpanReader,
    ) -> Result<(), ExportError> {
        let struct_name = self.db.resolve_string2(self.db.struct_def(struct_index).name_offset);
        if self.db.all_property_indices(struct_index).is_empty() {
            return self.empty_instance_element(name, base_attrs, None, struct_name);
        }
        self.start_instance_element(name, base_attrs, None, struct_name)?;
        self.write_struct_fields(struct_index, reader)?;
        self.end_element(name)
    }

    fn write_struct_fields(
        &mut self,
        struct_index: i32,
        reader: &mut SpanReader,
    ) -> Result<(), ExportError> {
        let prop_indices = self.db.all_property_indices(struct_index);
        let properties = self.db.property_defs();
        for &idx in prop_indices {
            let prop = &properties[idx as usize];
            let data_type = DataType::try_from(prop.data_type)?;
            let conversion_type = ConversionType::try_from(prop.conversion_type)?;
            if conversion_type == ConversionType::Attribute {
                self.write_attribute(
                    data_type,
                    prop.struct_index as i32,
                    self.db.resolve_string2(prop.name_offset),
                    reader,
                )?;
            } else {
                self.write_array(
                    data_type,
                    prop.struct_index as i32,
                    self.db.resolve_string2(prop.name_offset),
                    reader,
                )?;
            }
        }
        Ok(())
    }

    fn write_attribute(
        &mut self,
        data_type: DataType,
        prop_struct_index: i32,
        name: &str,
        reader: &mut SpanReader,
    ) -> Result<(), ExportError> {
        match data_type {
            DataType::Boolean => self.text_element(name, &format_bool(reader.read_bool()?))?,
            DataType::SByte => self.text_element_display(name, reader.read_i8()?)?,
            DataType::Int16 => self.text_element_display(name, reader.read_i16()?)?,
            DataType::Int32 => self.text_element_display(name, reader.read_i32()?)?,
            DataType::Int64 => self.text_element_display(name, reader.read_i64()?)?,
            DataType::Byte => self.text_element_display(name, reader.read_u8()?)?,
            DataType::UInt16 => self.text_element_display(name, reader.read_u16()?)?,
            DataType::UInt32 => self.text_element_display(name, reader.read_u32()?)?,
            DataType::UInt64 => self.text_element_display(name, reader.read_u64()?)?,
            DataType::Single => self.text_element(name, &format_single(reader.read_f32()?))?,
            DataType::Double => self.text_element(name, &format_double(reader.read_f64()?))?,
            DataType::String | DataType::Locale | DataType::EnumChoice => {
                let id = *reader.read_type::<StringId>()?;
                self.text_element(name, self.db.resolve_string(id))?;
            }
            DataType::Guid => {
                self.text_element_display(name, reader.read_type::<crate::types::CigGuid>()?)?;
            }
            DataType::Reference => {
                let reference = *reader.read_type::<Reference>()?;
                self.write_reference_element(name, &reference)?;
            }
            DataType::WeakPointer => {
                let pointer = *reader.read_type::<Pointer>()?;
                self.write_weak_pointer_element(name, &pointer)?;
            }
            DataType::StrongPointer => {
                let pointer = *reader.read_type::<Pointer>()?;
                self.write_strong_pointer_element(name, &pointer)?;
            }
            DataType::Class => {
                self.write_struct_element(name, &[], prop_struct_index, reader)?;
            }
        }
        Ok(())
    }

    fn write_array(
        &mut self,
        data_type: DataType,
        prop_struct_index: i32,
        name: &str,
        reader: &mut SpanReader,
    ) -> Result<(), ExportError> {
        let count = reader.read_i32()?;
        let first_index = reader.read_i32()?;
        let type_name = self
            .db
            .resolve_string2(self.db.struct_def(prop_struct_index).name_offset);
        let count_text = count.to_string();
        if count == 0 {
            return self.empty_element(
                name,
                &[("Type", type_name), ("Count", count_text.as_str())],
            );
        }
        self.start_element(name, &[("Type", type_name), ("Count", count_text.as_str())])?;

        for index in first_index..first_index + count {
            self.write_array_value(data_type, prop_struct_index, index)?;
        }

        self.end_element(name)
    }

    fn write_array_value(
        &mut self,
        data_type: DataType,
        prop_struct_index: i32,
        index: i32,
    ) -> Result<(), ExportError> {
        let idx = index as usize;
        match data_type {
            DataType::Boolean => {
                self.text_element(data_type_element_name(data_type), &format_bool(self.db.get_bool(idx)?))?
            }
            DataType::SByte => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_int8(idx)?)?
            }
            DataType::Int16 => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_int16(idx)?)?
            }
            DataType::Int32 => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_int32(idx)?)?
            }
            DataType::Int64 => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_int64(idx)?)?
            }
            DataType::Byte => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_uint8(idx)?)?
            }
            DataType::UInt16 => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_uint16(idx)?)?
            }
            DataType::UInt32 => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_uint32(idx)?)?
            }
            DataType::UInt64 => {
                self.text_element_display(data_type_element_name(data_type), self.db.get_uint64(idx)?)?
            }
            DataType::Single => {
                self.text_element(data_type_element_name(data_type), &format_single(self.db.get_single(idx)?))?
            }
            DataType::Double => {
                self.text_element(data_type_element_name(data_type), &format_double(self.db.get_double(idx)?))?
            }
            DataType::String => {
                let value = self.db.resolve_string(self.db.string_id_values[idx]);
                self.text_element(data_type_element_name(data_type), value)?
            }
            DataType::Locale => {
                let value = self.db.resolve_string(self.db.locale_values[idx]);
                self.text_element(data_type_element_name(data_type), value)?
            }
            DataType::EnumChoice => {
                let value = self.db.resolve_string(self.db.enum_values[idx]);
                self.text_element(data_type_element_name(data_type), value)?
            }
            DataType::Guid => {
                self.text_element_display(data_type_element_name(data_type), &self.db.guid_values[idx])?
            }
            DataType::Reference => {
                let reference = self.db.reference_values[idx];
                let element_name = self.reference_element_name(prop_struct_index, &reference);
                self.write_reference_element(&element_name, &reference)?;
            }
            DataType::WeakPointer => {
                let pointer = self.db.weak_values[idx];
                let element_name = self.pointer_element_name(prop_struct_index, &pointer);
                self.write_weak_pointer_element(&element_name, &pointer)?;
            }
            DataType::StrongPointer => {
                let pointer = self.db.strong_values[idx];
                let element_name = self.pointer_element_name(prop_struct_index, &pointer);
                self.write_strong_pointer_element(&element_name, &pointer)?;
            }
            DataType::Class => {
                let element_name = self
                    .db
                    .resolve_string2(self.db.struct_def(prop_struct_index).name_offset)
                    .to_string();
                self.write_instance_element(&element_name, &[], prop_struct_index, index)?;
            }
        }
        Ok(())
    }

    fn write_reference_element(
        &mut self,
        name: &str,
        reference: &Reference,
    ) -> Result<(), ExportError> {
        if reference_is_null(reference) {
            return self.empty_element(name, &[]);
        }

        let target = self
            .db
            .record_by_id(&reference.record_id)
            .ok_or(ExportError::InvalidReference {
                record_id: reference.record_id,
            })?;
        let target_name = self.db.resolve_string2(target.name_offset);

        if self.db.is_main_record(target) {
            let referenced_file = crate::walker::compute_relative_path_buf(
                self.db.resolve_string(target.file_name_offset),
                &self.path,
            )
            .replace('\\', "/");
            return self.empty_element(name, &[("ReferencedFile", referenced_file.as_str())]);
        }

        if target.file_name_offset.0 == self.file_name_offset {
            let record_id = target.id.to_string();
            return self.write_instance_element(
                name,
                &[("RecordId", record_id.as_str()), ("RecordName", target_name)],
                target.struct_index,
                reference.instance_index,
            );
        }

        let record_ref = crate::walker::compute_relative_path_buf(
            self.db.resolve_string(target.file_name_offset),
            &self.path,
        )
        .replace('\\', "/");
        let record_id = target.id.to_string();
        self.empty_element(
            name,
            &[
                ("RecordReference", record_ref.as_str()),
                ("RecordName", target_name),
                ("RecordId", record_id.as_str()),
            ],
        )
    }

    fn write_weak_pointer_element(
        &mut self,
        name: &str,
        pointer: &Pointer,
    ) -> Result<(), ExportError> {
        if pointer.is_null() {
            return self.empty_element(name, &[]);
        }
        let points_to = self.pointer_label(pointer.struct_index, pointer.instance_index);
        match points_to.as_deref() {
            Some(points_to) => self.empty_element(name, &[("PointsTo", points_to)]),
            None => Err(ExportError::InvalidPointer {
                struct_index: pointer.struct_index,
                instance_index: pointer.instance_index,
            }),
        }
    }

    fn write_strong_pointer_element(
        &mut self,
        name: &str,
        pointer: &Pointer,
    ) -> Result<(), ExportError> {
        if pointer.is_null() {
            return self.empty_element(name, &[]);
        }

        self.write_instance_element(name, &[], pointer.struct_index, pointer.instance_index)
    }

    fn reference_element_name(&self, prop_struct_index: i32, reference: &Reference) -> String {
        if reference_is_null(reference) {
            return self
                .db
                .resolve_string2(self.db.struct_def(prop_struct_index).name_offset)
                .to_string();
        }
        self.db
            .record_by_id(&reference.record_id)
            .map(|record| self.db.resolve_string2(self.db.struct_def(record.struct_index).name_offset))
            .unwrap_or_else(|| self.db.resolve_string2(self.db.struct_def(prop_struct_index).name_offset))
            .to_string()
    }

    fn pointer_element_name(&self, prop_struct_index: i32, pointer: &Pointer) -> String {
        if pointer.is_null() {
            self.db
                .resolve_string2(self.db.struct_def(prop_struct_index).name_offset)
                .to_string()
        } else {
            self.db
                .resolve_string2(self.db.struct_def(pointer.struct_index).name_offset)
                .to_string()
        }
    }

    fn pointer_label(&self, struct_index: i32, instance_index: i32) -> Option<String> {
        self.pointers
            .get(&(struct_index, instance_index))
            .map(|id| format!("ptr:{id}"))
    }

    fn start_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), ExportError> {
        self.write_indent()?;
        write!(self.writer, "<{name}")?;
        self.write_attrs_inline(attrs)?;
        writeln!(self.writer, ">")?;
        self.indent += 1;
        Ok(())
    }

    fn start_instance_element(
        &mut self,
        name: &str,
        base_attrs: &[(&str, &str)],
        pointer: Option<&str>,
        type_name: &str,
    ) -> Result<(), ExportError> {
        self.write_indent()?;
        write!(self.writer, "<{name}")?;
        self.write_attrs_inline(base_attrs)?;
        if let Some(pointer) = pointer {
            write!(self.writer, " Pointer=\"")?;
            write_escaped_attr(&mut self.writer, pointer)?;
            write!(self.writer, "\"")?;
        }
        write!(self.writer, " Type=\"")?;
        write_escaped_attr(&mut self.writer, type_name)?;
        write!(self.writer, "\"")?;
        writeln!(self.writer, ">")?;
        self.indent += 1;
        Ok(())
    }

    fn end_element(&mut self, name: &str) -> Result<(), ExportError> {
        self.indent -= 1;
        self.write_indent()?;
        writeln!(self.writer, "</{name}>")?;
        Ok(())
    }

    fn empty_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), ExportError> {
        self.write_indent()?;
        write!(self.writer, "<{name}")?;
        self.write_attrs_inline(attrs)?;
        writeln!(self.writer, " />")?;
        Ok(())
    }

    fn empty_instance_element(
        &mut self,
        name: &str,
        base_attrs: &[(&str, &str)],
        pointer: Option<&str>,
        type_name: &str,
    ) -> Result<(), ExportError> {
        self.write_indent()?;
        write!(self.writer, "<{name}")?;
        self.write_attrs_inline(base_attrs)?;
        if let Some(pointer) = pointer {
            write!(self.writer, " Pointer=\"")?;
            write_escaped_attr(&mut self.writer, pointer)?;
            write!(self.writer, "\"")?;
        }
        write!(self.writer, " Type=\"")?;
        write_escaped_attr(&mut self.writer, type_name)?;
        write!(self.writer, "\"")?;
        writeln!(self.writer, " />")?;
        Ok(())
    }

    fn text_element(&mut self, name: &str, text: &str) -> Result<(), ExportError> {
        if text.is_empty() {
            return self.empty_element(name, &[]);
        }
        self.write_indent()?;
        write!(self.writer, "<{name}>")?;
        write_escaped_text(&mut self.writer, text)?;
        writeln!(self.writer, "</{name}>")?;
        Ok(())
    }

    fn text_element_display(
        &mut self,
        name: &str,
        value: impl Display,
    ) -> Result<(), ExportError> {
        self.write_indent()?;
        write!(self.writer, "<{name}>{value}</{name}>")?;
        writeln!(self.writer)?;
        Ok(())
    }

    fn write_indent(&mut self) -> Result<(), ExportError> {
        const SPACES: &[u8] = b"                                ";
        let mut remaining = self.indent * 2;
        while remaining >= SPACES.len() {
            self.writer.write_all(SPACES)?;
            remaining -= SPACES.len();
        }
        if remaining > 0 {
            self.writer.write_all(&SPACES[..remaining])?;
        }
        Ok(())
    }

    fn write_attrs_inline(&mut self, attrs: &[(&str, &str)]) -> Result<(), ExportError> {
        for (name, value) in attrs {
            write!(self.writer, " {name}=\"")?;
            write_escaped_attr(&mut self.writer, value)?;
            write!(self.writer, "\"")?;
        }
        Ok(())
    }
}
