use starbreaker_chf::ChfError;
use starbreaker_chunks::ChunkFileError;
use starbreaker_cryxml::CryXmlError;
use starbreaker_datacore::error::{ExportError, QueryError};
use starbreaker_dds::DdsError;
use starbreaker_p4k::P4kError;
use starbreaker_wem::WemError;
use starbreaker_wwise::BnkError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    P4k(#[from] P4kError),
    #[error(transparent)]
    P4kOpen(#[from] starbreaker_p4k::discover::OpenError),
    #[error(transparent)]
    ChunkFile(#[from] ChunkFileError),
    #[error(transparent)]
    DataCoreExport(#[from] ExportError),
    #[error(transparent)]
    DataCoreQuery(#[from] QueryError),
    #[error(transparent)]
    DataCoreParse(#[from] starbreaker_datacore::error::ParseError),
    #[error(transparent)]
    Dds(#[from] DdsError),
    #[error(transparent)]
    Wwise(#[from] BnkError),
    #[error(transparent)]
    Wem(#[from] WemError),
    #[error(transparent)]
    CryXml(#[from] CryXmlError),
    #[error(transparent)]
    Chf(#[from] ChfError),
    #[error(transparent)]
    Gltf(#[from] starbreaker_3d::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)]
    ThreadPool(#[from] rayon::ThreadPoolBuildError),
    #[error(transparent)]
    ProgressTemplate(#[from] indicatif::style::TemplateError),

    #[error("failed to read '{path}': {source}")]
    IoPath { source: std::io::Error, path: String },
    #[error("{context}: {source}")]
    WithContext { source: Box<CliError>, context: String },

    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    MissingRequirement(String),
}

pub type Result<T> = std::result::Result<T, CliError>;
