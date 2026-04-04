use starbreaker_common::GuidParseError;
use starbreaker_datacore::error::{ExportError, ParseError, QueryError};
use starbreaker_3d;
use starbreaker_p4k::P4kError;
use starbreaker_wem::WemError;
use starbreaker_wwise::BnkError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    P4k(#[from] P4kError),
    #[error(transparent)]
    DataCoreExport(#[from] ExportError),
    #[error(transparent)]
    DataCoreParse(#[from] ParseError),
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Wwise(#[from] BnkError),
    #[error(transparent)]
    Wem(#[from] WemError),
    #[error(transparent)]
    Gltf(#[from] starbreaker_3d::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Guid(#[from] GuidParseError),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    CryXml(#[from] starbreaker_cryxml::CryXmlError),
    #[error(transparent)]
    Dds(#[from] starbreaker_dds::DdsError),

    #[error("{0}")]
    Internal(String),
}

// Tauri 2 requires `Into<tauri::ipc::InvokeError>` for command return types.
impl From<AppError> for tauri::ipc::InvokeError {
    fn from(err: AppError) -> Self {
        tauri::ipc::InvokeError::from(err.to_string())
    }
}

