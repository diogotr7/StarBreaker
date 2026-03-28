use crate::error::ExportError;

/// The data type of a DataCore property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum DataType {
    Boolean = 0x0001,
    SByte = 0x0002,
    Int16 = 0x0003,
    Int32 = 0x0004,
    Int64 = 0x0005,
    Byte = 0x0006,
    UInt16 = 0x0007,
    UInt32 = 0x0008,
    UInt64 = 0x0009,
    String = 0x000A,
    Single = 0x000B,
    Double = 0x000C,
    Locale = 0x000D,
    Guid = 0x000E,
    EnumChoice = 0x000F,
    Class = 0x0010,
    StrongPointer = 0x0110,
    WeakPointer = 0x0210,
    Reference = 0x0310,
}

impl TryFrom<u16> for DataType {
    type Error = ExportError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0001 => Ok(DataType::Boolean),
            0x0002 => Ok(DataType::SByte),
            0x0003 => Ok(DataType::Int16),
            0x0004 => Ok(DataType::Int32),
            0x0005 => Ok(DataType::Int64),
            0x0006 => Ok(DataType::Byte),
            0x0007 => Ok(DataType::UInt16),
            0x0008 => Ok(DataType::UInt32),
            0x0009 => Ok(DataType::UInt64),
            0x000A => Ok(DataType::String),
            0x000B => Ok(DataType::Single),
            0x000C => Ok(DataType::Double),
            0x000D => Ok(DataType::Locale),
            0x000E => Ok(DataType::Guid),
            0x000F => Ok(DataType::EnumChoice),
            0x0010 => Ok(DataType::Class),
            0x0110 => Ok(DataType::StrongPointer),
            0x0210 => Ok(DataType::WeakPointer),
            0x0310 => Ok(DataType::Reference),
            _ => Err(ExportError::UnknownDataType(value)),
        }
    }
}

impl DataType {
    /// Returns the byte size of this type when stored as an `Attribute` (scalar) value.
    /// Returns 0 for `Class` since it is variable-size/recursive.
    pub fn inline_size(self) -> usize {
        match self {
            DataType::Boolean => 1,
            DataType::SByte => 1,
            DataType::Byte => 1,
            DataType::Int16 => 2,
            DataType::UInt16 => 2,
            DataType::Int32 => 4,
            DataType::UInt32 => 4,
            DataType::EnumChoice => 4,
            DataType::Int64 => 8,
            DataType::UInt64 => 8,
            DataType::Single => 4,
            DataType::Double => 8,
            DataType::String => 4,
            DataType::Locale => 4,
            DataType::Guid => 16,
            DataType::StrongPointer => 8,
            DataType::WeakPointer => 8,
            DataType::Reference => 20,
            // Class is variable/recursive — no fixed inline size
            DataType::Class => 0,
        }
    }
}

/// How a property's data is stored in the binary stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ConversionType {
    Attribute = 0x00,
    ComplexArray = 0x01,
    SimpleArray = 0x02,
    ClassArray = 0x03,
}

impl TryFrom<u16> for ConversionType {
    type Error = ExportError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(ConversionType::Attribute),
            0x01 => Ok(ConversionType::ComplexArray),
            0x02 => Ok(ConversionType::SimpleArray),
            0x03 => Ok(ConversionType::ClassArray),
            _ => Err(ExportError::UnknownDataType(value)),
        }
    }
}
