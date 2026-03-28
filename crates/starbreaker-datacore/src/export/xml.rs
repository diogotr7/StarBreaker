use std::io::Write;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::sink::ExportSink;
use crate::types::CigGuid;

/// A streaming XML writer implementing `ExportSink`.
///
/// Structs become XML elements; primitive values become child elements.
/// `name = Some("field")` uses the field name as the tag; `name = None`
/// (array elements) uses `<Item>`.
pub struct XmlSink<W: Write> {
    writer: Writer<W>,
    /// Stack of currently-open element tag names (for writing closing tags).
    tag_stack: Vec<String>,
}

impl<W: Write> XmlSink<W> {
    /// Create a new `XmlSink` writing to `writer`.
    /// If `indent` is true, output is pretty-printed with 2-space indentation.
    pub fn new(writer: W, indent: bool) -> Self {
        let xml_writer = if indent {
            Writer::new_with_indent(writer, b' ', 2)
        } else {
            Writer::new(writer)
        };
        XmlSink {
            writer: xml_writer,
            tag_stack: Vec::new(),
        }
    }

    /// Resolve the effective tag name: use `name` if `Some`, else `"Item"`.
    fn tag(name: Option<&str>) -> &str {
        name.unwrap_or("Item")
    }

    /// Write a self-closing element: `<name />` (for null values).
    fn write_empty(&mut self, name: Option<&str>) -> Result<(), std::io::Error> {
        let tag = Self::tag(name);
        // Use a start event followed by an end — quick_xml will merge as self-closing
        // when using BytesStart with no content and immediately ended.
        let start = BytesStart::new(tag);
        self.writer.write_event(Event::Empty(start))?;
        Ok(())
    }

    /// Write `<name>text</name>`.
    fn write_text_element(&mut self, name: Option<&str>, text: &str) -> Result<(), std::io::Error> {
        let tag = Self::tag(name);
        let tag_owned = tag.to_owned();
        self.writer
            .write_event(Event::Start(BytesStart::new(tag_owned.as_str())))?;
        self.writer.write_event(Event::Text(BytesText::new(text)))?;
        self.writer
            .write_event(Event::End(BytesEnd::new(tag_owned.as_str())))?;
        Ok(())
    }
}

impl<W: Write> ExportSink for XmlSink<W> {
    type Error = std::io::Error;

    fn begin_object(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        let tag = Self::tag(name).to_owned();
        self.writer
            .write_event(Event::Start(BytesStart::new(tag.as_str())))?;
        self.tag_stack.push(tag);
        Ok(())
    }

    fn end_object(&mut self) -> Result<(), Self::Error> {
        if let Some(tag) = self.tag_stack.pop() {
            self.writer
                .write_event(Event::End(BytesEnd::new(tag.as_str())))?;
        }
        Ok(())
    }

    fn begin_array(&mut self, name: &str) -> Result<(), Self::Error> {
        let tag = name.to_owned();
        self.writer
            .write_event(Event::Start(BytesStart::new(tag.as_str())))?;
        self.tag_stack.push(tag);
        Ok(())
    }

    fn end_array(&mut self) -> Result<(), Self::Error> {
        if let Some(tag) = self.tag_stack.pop() {
            self.writer
                .write_event(Event::End(BytesEnd::new(tag.as_str())))?;
        }
        Ok(())
    }

    fn write_null(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.write_empty(name)
    }

    fn write_bool(&mut self, name: Option<&str>, value: bool) -> Result<(), Self::Error> {
        self.write_text_element(name, if value { "true" } else { "false" })
    }

    fn write_i8(&mut self, name: Option<&str>, value: i8) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_i16(&mut self, name: Option<&str>, value: i16) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_i32(&mut self, name: Option<&str>, value: i32) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_i64(&mut self, name: Option<&str>, value: i64) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_u8(&mut self, name: Option<&str>, value: u8) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_u16(&mut self, name: Option<&str>, value: u16) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_u32(&mut self, name: Option<&str>, value: u32) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_u64(&mut self, name: Option<&str>, value: u64) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }

    fn write_f32(&mut self, name: Option<&str>, value: f32) -> Result<(), Self::Error> {
        if value.is_finite() {
            self.write_text_element(name, &value.to_string())
        } else {
            self.write_empty(name)
        }
    }

    fn write_f64(&mut self, name: Option<&str>, value: f64) -> Result<(), Self::Error> {
        if value.is_finite() {
            self.write_text_element(name, &value.to_string())
        } else {
            self.write_empty(name)
        }
    }

    fn write_str(&mut self, name: Option<&str>, value: &str) -> Result<(), Self::Error> {
        self.write_text_element(name, value)
    }

    fn write_guid(&mut self, name: Option<&str>, value: &CigGuid) -> Result<(), Self::Error> {
        self.write_text_element(name, &value.to_string())
    }
}
