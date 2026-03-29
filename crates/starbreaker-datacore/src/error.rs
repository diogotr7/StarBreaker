use starbreaker_common::ParseError as CommonParseError;

use crate::enums::DataType;
use crate::types::CigGuid;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Common(#[from] CommonParseError),

    #[error("unsupported version: {0} (only v6 supported)")]
    UnsupportedVersion(u32),
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("invalid pointer: struct {struct_index} instance {instance_index}")]
    InvalidPointer {
        struct_index: i32,
        instance_index: i32,
    },

    #[error("invalid reference: record {record_id}")]
    InvalidReference { record_id: CigGuid },

    #[error("unknown data type: {0:#06x}")]
    UnknownDataType(u16),

    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error(transparent)]
    CommonParse(#[from] CommonParseError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Query(#[from] QueryError),
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("parse error in path at position {position}: {message}")]
    PathParse { position: usize, message: String },

    #[error("property '{property}' not found on struct '{struct_name}'")]
    PropertyNotFound {
        property: String,
        struct_name: String,
    },

    #[error("type filter '{filter}' does not match or inherit from '{expected}'")]
    TypeFilterMismatch { filter: String, expected: String },

    #[error("struct type '{name}' not found in DataCore schema")]
    StructNotFound { name: String },

    #[error(
        "type filter required: property '{property}' is a polymorphic array (StrongPointer/WeakPointer)"
    )]
    TypeFilterRequired { property: String },

    #[error("type filter not allowed on non-array property '{property}'")]
    TypeFilterOnScalar { property: String },

    #[error("expected leaf type {expected:?}, but property '{property}' is {actual:?}")]
    LeafTypeMismatch {
        property: String,
        expected: &'static [DataType],
        actual: DataType,
    },

    #[error("record struct index {actual} does not match compiled path root {expected}")]
    StructMismatch { expected: i32, actual: i32 },

    #[error("query_one matched {count} values (expected exactly 1)")]
    CardinalityMismatch { count: usize },

    #[error("null pointer encountered at '{segment}'")]
    NullPointer { segment: String },

    #[error("null reference encountered at '{segment}'")]
    NullReference { segment: String },

    #[error("unknown data type {0:#06x} or conversion type")]
    UnknownType(u16),

    #[error(transparent)]
    Read(#[from] starbreaker_common::ParseError),

    #[error("missing target struct index for {segment} segment")]
    MissingTargetStructIndex { segment: String },
}

/// Extension trait for `Result<T, QueryError>` to handle optional components.
pub trait QueryResultExt<T> {
    /// Convert a `TypeFilterMismatch` into `Ok(None)`, propagate all other errors.
    ///
    /// Use when querying an optional polymorphic component — the type filter
    /// not matching means "this component doesn't exist here", not a bug.
    fn optional(self) -> Result<Option<T>, QueryError>;
}

impl<T> QueryResultExt<T> for Result<T, QueryError> {
    fn optional(self) -> Result<Option<T>, QueryError> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(QueryError::TypeFilterMismatch { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
