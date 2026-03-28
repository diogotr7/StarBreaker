use starbreaker_datacore::export::json::JsonSink;
use starbreaker_datacore::sink::ExportSink;
use starbreaker_datacore::types::CigGuid;
use std::io::Write;

/// A JSON sink for UI display that emits structured objects for main-record
/// references instead of plain path strings, making them clickable in the frontend.
pub struct UiJsonSink<W: Write> {
    inner: JsonSink<W>,
}

impl<W: Write> UiJsonSink<W> {
    pub fn new(writer: W) -> Self {
        Self {
            inner: JsonSink::new(writer, true),
        }
    }
}

impl<W: Write> ExportSink for UiJsonSink<W> {
    type Error = std::io::Error;

    fn begin_object(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.inner.begin_object(name)
    }
    fn end_object(&mut self) -> Result<(), Self::Error> {
        self.inner.end_object()
    }
    fn begin_array(&mut self, name: &str) -> Result<(), Self::Error> {
        self.inner.begin_array(name)
    }
    fn end_array(&mut self) -> Result<(), Self::Error> {
        self.inner.end_array()
    }
    fn write_null(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        self.inner.write_null(name)
    }
    fn write_bool(&mut self, name: Option<&str>, value: bool) -> Result<(), Self::Error> {
        self.inner.write_bool(name, value)
    }
    fn write_i8(&mut self, name: Option<&str>, value: i8) -> Result<(), Self::Error> {
        self.inner.write_i8(name, value)
    }
    fn write_i16(&mut self, name: Option<&str>, value: i16) -> Result<(), Self::Error> {
        self.inner.write_i16(name, value)
    }
    fn write_i32(&mut self, name: Option<&str>, value: i32) -> Result<(), Self::Error> {
        self.inner.write_i32(name, value)
    }
    fn write_i64(&mut self, name: Option<&str>, value: i64) -> Result<(), Self::Error> {
        self.inner.write_i64(name, value)
    }
    fn write_u8(&mut self, name: Option<&str>, value: u8) -> Result<(), Self::Error> {
        self.inner.write_u8(name, value)
    }
    fn write_u16(&mut self, name: Option<&str>, value: u16) -> Result<(), Self::Error> {
        self.inner.write_u16(name, value)
    }
    fn write_u32(&mut self, name: Option<&str>, value: u32) -> Result<(), Self::Error> {
        self.inner.write_u32(name, value)
    }
    fn write_u64(&mut self, name: Option<&str>, value: u64) -> Result<(), Self::Error> {
        self.inner.write_u64(name, value)
    }
    fn write_f32(&mut self, name: Option<&str>, value: f32) -> Result<(), Self::Error> {
        self.inner.write_f32(name, value)
    }
    fn write_f64(&mut self, name: Option<&str>, value: f64) -> Result<(), Self::Error> {
        self.inner.write_f64(name, value)
    }
    fn write_str(&mut self, name: Option<&str>, value: &str) -> Result<(), Self::Error> {
        self.inner.write_str(name, value)
    }
    fn write_guid(&mut self, name: Option<&str>, value: &CigGuid) -> Result<(), Self::Error> {
        self.inner.write_guid(name, value)
    }

    fn write_record_ref(
        &mut self,
        name: Option<&str>,
        record_id: &CigGuid,
        record_name: &str,
        path: &str,
    ) -> Result<(), Self::Error> {
        self.inner.begin_object(name)?;
        self.inner.write_str(Some("_RecordPath_"), path)?;
        self.inner.write_str(Some("_RecordName_"), record_name)?;
        self.inner.write_guid(Some("_RecordId_"), record_id)?;
        self.inner.end_object()
    }
}
