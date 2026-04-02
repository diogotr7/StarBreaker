use std::io::Write;

use serde_json::ser::{Formatter, PrettyFormatter};

use crate::sink::ExportSink;
use crate::types::CigGuid;

/// A streaming JSON writer implementing `ExportSink`.
///
/// Delegates all formatting (indentation, commas, escaping) to serde_json's
/// `PrettyFormatter`, ensuring compliant output by construction.
pub struct JsonSink<W: Write> {
    writer: W,
    fmt: PrettyFormatter<'static>,
    /// Stack tracking whether current container has had its first element.
    first_stack: Vec<bool>,
}

impl<W: Write> JsonSink<W> {
    pub fn new(writer: W, indent: bool) -> Self {
        let fmt = if indent {
            PrettyFormatter::with_indent(b"  ")
        } else {
            PrettyFormatter::with_indent(b"")
        };
        JsonSink {
            writer,
            fmt,
            first_stack: Vec::new(),
        }
    }

    fn is_first(&self) -> bool {
        self.first_stack.last().copied().unwrap_or(true)
    }

    fn mark_not_first(&mut self) {
        if let Some(f) = self.first_stack.last_mut() {
            *f = false;
        }
    }

    /// Emit the prefix for a named value in an object: comma/newline + key + colon.
    fn key_prefix(&mut self, name: &str) -> Result<(), std::io::Error> {
        let first = self.is_first();
        self.fmt.begin_object_key(&mut self.writer, first)?;
        self.mark_not_first();
        serde_json::to_writer(&mut self.writer, name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        self.fmt.end_object_key(&mut self.writer)?;
        self.fmt.begin_object_value(&mut self.writer)?;
        Ok(())
    }

    /// Emit the prefix for an unnamed value in an array: comma/newline.
    fn array_value_prefix(&mut self) -> Result<(), std::io::Error> {
        let first = self.is_first();
        self.fmt.begin_array_value(&mut self.writer, first)?;
        self.mark_not_first();
        Ok(())
    }

    /// Emit the prefix for any value: key prefix if named, array prefix if unnamed.
    fn value_prefix(&mut self, name: Option<&str>) -> Result<(), std::io::Error> {
        if let Some(n) = name {
            self.key_prefix(n)
        } else if !self.first_stack.is_empty() {
            self.array_value_prefix()
        } else {
            Ok(())
        }
    }
}

impl<W: Write> ExportSink for JsonSink<W> {
    type Error = std::io::Error;

    fn extension(&self) -> &str {
        "json"
    }

    fn begin_object(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.begin_object(&mut self.writer)?;
        self.first_stack.push(true);
        Ok(())
    }

    fn end_object(&mut self) -> Result<(), Self::Error> {
        self.first_stack.pop();
        self.fmt.end_object(&mut self.writer)?;
        Ok(())
    }

    fn begin_array(&mut self, name: &str) -> Result<(), Self::Error> {
        self.key_prefix(name)?;
        self.fmt.begin_array(&mut self.writer)?;
        self.first_stack.push(true);
        Ok(())
    }

    fn end_array(&mut self) -> Result<(), Self::Error> {
        self.first_stack.pop();
        self.fmt.end_array(&mut self.writer)?;
        Ok(())
    }

    fn write_null(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_null(&mut self.writer)
    }

    fn write_bool(&mut self, name: Option<&str>, value: bool) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_bool(&mut self.writer, value)
    }

    fn write_i8(&mut self, name: Option<&str>, value: i8) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_i8(&mut self.writer, value)
    }

    fn write_i16(&mut self, name: Option<&str>, value: i16) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_i16(&mut self.writer, value)
    }

    fn write_i32(&mut self, name: Option<&str>, value: i32) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_i32(&mut self.writer, value)
    }

    fn write_i64(&mut self, name: Option<&str>, value: i64) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_i64(&mut self.writer, value)
    }

    fn write_u8(&mut self, name: Option<&str>, value: u8) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_u8(&mut self.writer, value)
    }

    fn write_u16(&mut self, name: Option<&str>, value: u16) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_u16(&mut self.writer, value)
    }

    fn write_u32(&mut self, name: Option<&str>, value: u32) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_u32(&mut self.writer, value)
    }

    fn write_u64(&mut self, name: Option<&str>, value: u64) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        self.fmt.write_u64(&mut self.writer, value)
    }

    fn write_f32(&mut self, name: Option<&str>, value: f32) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        if value.is_finite() {
            self.fmt.write_f32(&mut self.writer, value)
        } else {
            self.fmt.write_null(&mut self.writer)
        }
    }

    fn write_f64(&mut self, name: Option<&str>, value: f64) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        if value.is_finite() {
            self.fmt.write_f64(&mut self.writer, value)
        } else {
            self.fmt.write_null(&mut self.writer)
        }
    }

    fn write_str(&mut self, name: Option<&str>, value: &str) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        serde_json::to_writer(&mut self.writer, value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }

    fn write_guid(&mut self, name: Option<&str>, value: &CigGuid) -> Result<(), Self::Error> {
        self.value_prefix(name)?;
        // Format GUID to stack buffer, then write as JSON string
        let s = value.to_string();
        serde_json::to_writer(&mut self.writer, &s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}
