use std::fmt;

use crate::error::ParseError;
use zerocopy::little_endian::{F32, F64, I16, I32, I64, U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// A cursor over a `&[u8]` slice for reading little-endian binary data.
pub struct SpanReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> SpanReader<'a> {
    /// Create a new reader starting at position 0.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        SpanReader { data, position: 0 }
    }

    /// Create a reader starting at a given position.
    #[inline]
    pub fn new_at(data: &'a [u8], position: usize) -> Self {
        SpanReader { data, position }
    }

    /// Current read position in bytes.
    #[inline]
    pub fn position(&self) -> usize {
        self.position
    }

    /// Set the read position. Must be within the data bounds.
    #[inline]
    pub fn set_position(&mut self, position: usize) {
        assert!(position <= self.data.len());
        self.position = position;
    }

    /// Bytes remaining from the current position to the end of the slice.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// Returns `true` when every byte has been consumed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.position >= self.data.len()
    }

    /// Returns the remaining unread bytes from the current position.
    #[inline]
    pub fn remaining_bytes(&self) -> &'a [u8] {
        &self.data[self.position..]
    }

    /// Advance the cursor by `count` bytes without reading data.
    #[inline]
    pub fn advance(&mut self, count: usize) -> Result<(), ParseError> {
        if count > self.remaining() {
            return Err(self.truncated_err(count));
        }
        self.position += count;
        Ok(())
    }

    /// Read exactly `count` raw bytes, advancing the cursor.
    #[inline]
    pub fn read_bytes(&mut self, count: usize) -> Result<&'a [u8], ParseError> {
        let rest = self.remaining_bytes();
        if count > rest.len() {
            return Err(self.truncated_err(count));
        }
        self.position += count;
        Ok(&rest[..count])
    }

    /// Read a zerocopy struct `T` directly from the current position.
    #[inline]
    pub fn read_type<T>(&mut self) -> Result<&'a T, ParseError>
    where
        T: FromBytes + KnownLayout + Immutable,
    {
        let size = size_of::<T>();
        let rest = self.remaining_bytes();
        if size > rest.len() {
            return Err(self.truncated_err(size));
        }
        let value = T::ref_from_bytes(&rest[..size]).map_err(|_| Self::layout_err::<T>())?;
        self.position += size;
        Ok(value)
    }

    /// Peek at a zerocopy struct `T` at the current position without advancing.
    #[inline]
    pub fn peek_type<T>(&self) -> Result<&'a T, ParseError>
    where
        T: FromBytes + KnownLayout + Immutable,
    {
        let size = size_of::<T>();
        let rest = self.remaining_bytes();
        if size > rest.len() {
            return Err(self.truncated_err(size));
        }
        T::ref_from_bytes(&rest[..size]).map_err(|_| Self::layout_err::<T>())
    }

    /// Read a `T` and assert it equals `expected`, returning an error otherwise.
    #[inline]
    pub fn expect<T>(&mut self, expected: T) -> Result<&'a T, ParseError>
    where
        T: FromBytes + KnownLayout + Immutable + IntoBytes + PartialEq + fmt::Debug,
    {
        let offset = self.position;
        let value = self.read_type::<T>()?;
        if *value != expected {
            return Err(ParseError::UnexpectedValue {
                offset,
                expected: format!("{expected:?}"),
                actual: format!("{value:?}"),
            });
        }
        Ok(value)
    }

    /// Read a `T` and assert it equals one of `values`, returning an error otherwise.
    #[inline]
    pub fn expect_any<T>(&mut self, values: &[T]) -> Result<&'a T, ParseError>
    where
        T: FromBytes + KnownLayout + Immutable + IntoBytes + PartialEq + fmt::Debug,
    {
        let offset = self.position;
        let value = self.read_type::<T>()?;
        if !values.contains(value) {
            return Err(ParseError::UnexpectedValue {
                offset,
                expected: format!("{values:?}"),
                actual: format!("{value:?}"),
            });
        }
        Ok(value)
    }

    /// Read a slice of `count` zero-copy elements of type `T`.
    #[inline]
    pub fn read_slice<T>(&mut self, count: usize) -> Result<&'a [T], ParseError>
    where
        T: FromBytes + KnownLayout + Immutable,
    {
        let total = size_of::<T>() * count;
        let rest = self.remaining_bytes();
        if total > rest.len() {
            return Err(self.truncated_err(total));
        }
        let values = <[T]>::ref_from_bytes_with_elems(&rest[..total], count)
            .map_err(|_| Self::layout_err::<T>())?;
        self.position += total;
        Ok(values)
    }

    /// Split off a sub-reader of `len` bytes starting at the current position.
    ///
    /// The current reader is advanced past those bytes.
    #[inline]
    pub fn split_off(&mut self, len: usize) -> Result<SpanReader<'a>, ParseError> {
        let bytes = self.read_bytes(len)?;
        Ok(SpanReader::new(bytes))
    }

    #[cold]
    fn layout_err<T>() -> ParseError {
        ParseError::InvalidLayout(std::any::type_name::<T>().to_string())
    }

    #[cold]
    fn truncated_err(&self, need: usize) -> ParseError {
        ParseError::Truncated {
            offset: self.position,
            need,
            have: self.data.len() - self.position,
        }
    }

    // ── Typed primitive readers ───────────────────────────────────────────

    #[inline]
    pub fn read_bool(&mut self) -> Result<bool, ParseError> {
        Ok(*self.read_type::<u8>()? != 0)
    }

    #[inline]
    pub fn read_u8(&mut self) -> Result<u8, ParseError> {
        Ok(*self.read_type::<u8>()?)
    }

    #[inline]
    pub fn read_i8(&mut self) -> Result<i8, ParseError> {
        Ok(*self.read_type::<i8>()?)
    }

    #[inline]
    pub fn read_u16(&mut self) -> Result<u16, ParseError> {
        Ok(self.read_type::<U16>()?.get())
    }

    #[inline]
    pub fn read_i16(&mut self) -> Result<i16, ParseError> {
        Ok(self.read_type::<I16>()?.get())
    }

    #[inline]
    pub fn read_u32(&mut self) -> Result<u32, ParseError> {
        Ok(self.read_type::<U32>()?.get())
    }

    #[inline]
    pub fn read_i32(&mut self) -> Result<i32, ParseError> {
        Ok(self.read_type::<I32>()?.get())
    }

    #[inline]
    pub fn read_u64(&mut self) -> Result<u64, ParseError> {
        Ok(self.read_type::<U64>()?.get())
    }

    #[inline]
    pub fn read_i64(&mut self) -> Result<i64, ParseError> {
        Ok(self.read_type::<I64>()?.get())
    }

    #[inline]
    pub fn read_f32(&mut self) -> Result<f32, ParseError> {
        Ok(self.read_type::<F32>()?.get())
    }

    #[inline]
    pub fn read_f64(&mut self) -> Result<f64, ParseError> {
        Ok(self.read_type::<F64>()?.get())
    }
}
