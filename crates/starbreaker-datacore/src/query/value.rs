use crate::types::CigGuid;

#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float(f32),
    Double(f64),
    String(&'a str),
    Guid(CigGuid),
    Enum(&'a str),
    Locale(&'a str),
    Array(Vec<Value<'a>>),
    Object {
        type_name: &'a str,
        fields: Vec<(&'a str, Value<'a>)>,
        /// Source record ID when this Object was materialized from a Reference.
        /// `None` for inline structs and pointer-followed objects.
        record_id: Option<CigGuid>,
    },
}
