use zerocopy::{Immutable, IntoBytes};

/// A write cursor over a growable `Vec<u8>` for producing little-endian binary data.
///
/// All methods are infallible because writing to a `Vec` cannot fail (barring OOM).
pub struct SpanWriter {
    buf: Vec<u8>,
}

impl SpanWriter {
    /// Create a new empty writer.
    pub fn new() -> Self {
        SpanWriter { buf: Vec::new() }
    }

    /// Create a writer with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        SpanWriter {
            buf: Vec::with_capacity(capacity),
        }
    }

    /// Write a zerocopy struct as raw bytes.
    pub fn write_val<T: IntoBytes + Immutable>(&mut self, value: &T) {
        self.buf.extend_from_slice(value.as_bytes());
    }

    /// Write raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Write a single byte.
    pub fn write_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    /// Write a little-endian u16.
    pub fn write_u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a little-endian u32.
    pub fn write_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a little-endian u64.
    pub fn write_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a little-endian i32.
    pub fn write_i32(&mut self, value: i32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a little-endian f32.
    pub fn write_f32(&mut self, value: f32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Consume the writer and return the underlying buffer.
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    /// Number of bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns `true` if no bytes have been written.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Default for SpanWriter {
    fn default() -> Self {
        Self::new()
    }
}
