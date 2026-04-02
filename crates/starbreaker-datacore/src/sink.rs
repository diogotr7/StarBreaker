use crate::types::CigGuid;

/// Streaming output for DataCore record export.
/// `name`: `Some("field")` for named properties, `None` for array elements.
pub trait ExportSink {
    type Error: From<std::io::Error>;

    /// File extension used for cross-record reference paths (e.g. "json", "xml").
    fn extension(&self) -> &str;

    fn begin_object(&mut self, name: Option<&str>) -> Result<(), Self::Error>;
    fn end_object(&mut self) -> Result<(), Self::Error>;
    fn begin_array(&mut self, name: &str) -> Result<(), Self::Error>;
    fn end_array(&mut self) -> Result<(), Self::Error>;

    fn write_null(&mut self, name: Option<&str>) -> Result<(), Self::Error>;
    fn write_bool(&mut self, name: Option<&str>, value: bool) -> Result<(), Self::Error>;
    fn write_i8(&mut self, name: Option<&str>, value: i8) -> Result<(), Self::Error>;
    fn write_i16(&mut self, name: Option<&str>, value: i16) -> Result<(), Self::Error>;
    fn write_i32(&mut self, name: Option<&str>, value: i32) -> Result<(), Self::Error>;
    fn write_i64(&mut self, name: Option<&str>, value: i64) -> Result<(), Self::Error>;
    fn write_u8(&mut self, name: Option<&str>, value: u8) -> Result<(), Self::Error>;
    fn write_u16(&mut self, name: Option<&str>, value: u16) -> Result<(), Self::Error>;
    fn write_u32(&mut self, name: Option<&str>, value: u32) -> Result<(), Self::Error>;
    fn write_u64(&mut self, name: Option<&str>, value: u64) -> Result<(), Self::Error>;
    fn write_f32(&mut self, name: Option<&str>, value: f32) -> Result<(), Self::Error>;
    fn write_f64(&mut self, name: Option<&str>, value: f64) -> Result<(), Self::Error>;
    fn write_str(&mut self, name: Option<&str>, value: &str) -> Result<(), Self::Error>;
    fn write_guid(&mut self, name: Option<&str>, value: &CigGuid) -> Result<(), Self::Error>;

    /// Write a reference to a main record. Sinks can override this to emit
    /// structured data (e.g. record ID + name) instead of a plain path string.
    /// Default: writes the relative path as a string (file-export behavior).
    fn write_record_ref(
        &mut self,
        name: Option<&str>,
        record_id: &CigGuid,
        record_name: &str,
        path: &str,
    ) -> Result<(), Self::Error> {
        let _ = (record_id, record_name);
        self.write_str(name, path)
    }
}
