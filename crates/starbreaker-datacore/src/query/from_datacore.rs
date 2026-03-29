use crate::database::Database;
use crate::enums::DataType;
use crate::error::QueryError;
use crate::types::{CigGuid, StringId};
use starbreaker_common::SpanReader;

/// Trait for types that can be extracted from a DataCore database.
///
/// The lifetime `'a` ties to the database's backing data. Types that don't
/// borrow (String, i32, etc.) simply ignore the lifetime.
pub trait FromDataCore<'a>: Sized {
    /// The DataCore `DataType` variants this type can be read from.
    fn expected_data_types() -> &'static [DataType];

    /// Read a value from an inline instance-data stream.
    fn read_from_reader(
        db: &'a Database<'a>,
        reader: &mut SpanReader,
        data_type: DataType,
    ) -> Result<Self, QueryError>;

    /// Read a value by index from the appropriate typed value array in the database.
    fn read_from_array(
        db: &'a Database<'a>,
        index: usize,
        data_type: DataType,
    ) -> Result<Self, QueryError>;

    /// Convert from a materialized `Value`.  Only `Value` itself overrides this;
    /// all other types use the default implementation which panics.
    fn from_value(_value: crate::query::value::Value<'a>) -> Result<Self, QueryError> {
        Err(QueryError::UnknownType(0))
    }
}

// ── Macro for repetitive numeric types ────────────────────────────────────────

macro_rules! impl_from_datacore_int {
    ($ty:ty, $dt:expr, $read_method:ident, $array_method:ident) => {
        impl<'a> FromDataCore<'a> for $ty {
            fn expected_data_types() -> &'static [DataType] {
                &[$dt]
            }

            fn read_from_reader(
                _db: &'a Database<'a>,
                reader: &mut SpanReader,
                _data_type: DataType,
            ) -> Result<Self, QueryError> {
                Ok(reader.$read_method()?)
            }

            fn read_from_array(
                db: &'a Database<'a>,
                index: usize,
                _data_type: DataType,
            ) -> Result<Self, QueryError> {
                Ok(db.$array_method(index)?)
            }
        }
    };
}

impl_from_datacore_int!(i8, DataType::SByte, read_i8, get_int8);
impl_from_datacore_int!(i16, DataType::Int16, read_i16, get_int16);
impl_from_datacore_int!(i32, DataType::Int32, read_i32, get_int32);
impl_from_datacore_int!(i64, DataType::Int64, read_i64, get_int64);
impl_from_datacore_int!(u8, DataType::Byte, read_u8, get_uint8);
impl_from_datacore_int!(u16, DataType::UInt16, read_u16, get_uint16);
impl_from_datacore_int!(u32, DataType::UInt32, read_u32, get_uint32);
impl_from_datacore_int!(u64, DataType::UInt64, read_u64, get_uint64);
impl_from_datacore_int!(f32, DataType::Single, read_f32, get_single);
impl_from_datacore_int!(f64, DataType::Double, read_f64, get_double);

// ── bool ──────────────────────────────────────────────────────────────────────

impl<'a> FromDataCore<'a> for bool {
    fn expected_data_types() -> &'static [DataType] {
        &[DataType::Boolean]
    }

    fn read_from_reader(
        _db: &'a Database<'a>,
        reader: &mut SpanReader,
        _data_type: DataType,
    ) -> Result<Self, QueryError> {
        Ok(reader.read_bool()?)
    }

    fn read_from_array(
        db: &'a Database<'a>,
        index: usize,
        _data_type: DataType,
    ) -> Result<Self, QueryError> {
        Ok(db.get_bool(index)?)
    }
}

// ── String ────────────────────────────────────────────────────────────────────

impl<'a> FromDataCore<'a> for String {
    fn expected_data_types() -> &'static [DataType] {
        &[DataType::String, DataType::Locale, DataType::EnumChoice]
    }

    fn read_from_reader(
        db: &'a Database<'a>,
        reader: &mut SpanReader,
        _data_type: DataType,
    ) -> Result<Self, QueryError> {
        let sid = *reader.read_type::<StringId>()?;
        Ok(db.resolve_string(sid).to_owned())
    }

    fn read_from_array(
        db: &'a Database<'a>,
        index: usize,
        data_type: DataType,
    ) -> Result<Self, QueryError> {
        let sid = match data_type {
            DataType::String => db.string_id_values[index],
            DataType::Locale => db.locale_values[index],
            DataType::EnumChoice => db.enum_values[index],
            _ => return Err(QueryError::LeafTypeMismatch {
                property: "String".to_owned(),
                expected: Self::expected_data_types(),
                actual: data_type,
            }),
        };
        Ok(db.resolve_string(sid).to_owned())
    }
}

// ── CigGuid ───────────────────────────────────────────────────────────────────

impl<'a> FromDataCore<'a> for CigGuid {
    fn expected_data_types() -> &'static [DataType] {
        &[DataType::Guid]
    }

    fn read_from_reader(
        _db: &'a Database<'a>,
        reader: &mut SpanReader,
        _data_type: DataType,
    ) -> Result<Self, QueryError> {
        Ok(*reader.read_type::<CigGuid>()?)
    }

    fn read_from_array(
        db: &'a Database<'a>,
        index: usize,
        _data_type: DataType,
    ) -> Result<Self, QueryError> {
        Ok(db.guid_values[index])
    }
}

// ── Value ──────────────────────────────────────────────────────────────────────

use crate::query::value::Value;

impl<'a> FromDataCore<'a> for Value<'a> {
    fn expected_data_types() -> &'static [DataType] {
        &[
            DataType::Boolean,
            DataType::SByte,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::Byte,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
            DataType::String,
            DataType::Single,
            DataType::Double,
            DataType::Locale,
            DataType::Guid,
            DataType::EnumChoice,
            DataType::Class,
            DataType::StrongPointer,
            DataType::WeakPointer,
            DataType::Reference,
        ]
    }

    fn read_from_reader(
        db: &'a Database<'a>,
        reader: &mut SpanReader,
        data_type: DataType,
    ) -> Result<Self, QueryError> {
        match data_type {
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
                let sid = *reader.read_type::<StringId>()?;
                Ok(Value::String(db.resolve_string(sid)))
            }
            DataType::Locale => {
                let sid = *reader.read_type::<StringId>()?;
                Ok(Value::Locale(db.resolve_string(sid)))
            }
            DataType::EnumChoice => {
                let sid = *reader.read_type::<StringId>()?;
                Ok(Value::Enum(db.resolve_string(sid)))
            }
            DataType::Guid => Ok(Value::Guid(*reader.read_type::<CigGuid>()?)),
            _ => Ok(Value::Null),
        }
    }

    fn read_from_array(
        db: &'a Database<'a>,
        index: usize,
        data_type: DataType,
    ) -> Result<Self, QueryError> {
        match data_type {
            DataType::Boolean => Ok(Value::Bool(db.get_bool(index)?)),
            DataType::SByte => Ok(Value::Int8(db.get_int8(index)?)),
            DataType::Int16 => Ok(Value::Int16(db.get_int16(index)?)),
            DataType::Int32 => Ok(Value::Int32(db.get_int32(index)?)),
            DataType::Int64 => Ok(Value::Int64(db.get_int64(index)?)),
            DataType::Byte => Ok(Value::UInt8(db.get_uint8(index)?)),
            DataType::UInt16 => Ok(Value::UInt16(db.get_uint16(index)?)),
            DataType::UInt32 => Ok(Value::UInt32(db.get_uint32(index)?)),
            DataType::UInt64 => Ok(Value::UInt64(db.get_uint64(index)?)),
            DataType::Single => Ok(Value::Float(db.get_single(index)?)),
            DataType::Double => Ok(Value::Double(db.get_double(index)?)),
            DataType::String => Ok(Value::String(db.resolve_string(db.string_id_values[index]))),
            DataType::Locale => Ok(Value::Locale(db.resolve_string(db.locale_values[index]))),
            DataType::EnumChoice => Ok(Value::Enum(db.resolve_string(db.enum_values[index]))),
            DataType::Guid => Ok(Value::Guid(db.guid_values[index])),
            _ => Ok(Value::Null),
        }
    }

    fn from_value(value: Value<'a>) -> Result<Self, QueryError> {
        Ok(value)
    }
}
