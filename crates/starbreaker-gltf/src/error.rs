use starbreaker_chunks::ChunkFileError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    ChunkFile(#[from] ChunkFileError),

    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),

    #[error("CrCh format not supported, expected IVO")]
    UnsupportedFormat,

    #[error("missing required chunk type: 0x{chunk_type:08X}")]
    MissingChunk { chunk_type: u32 },

    #[error("unexpected stream element size: expected {expected}, got {got}")]
    UnexpectedElementSize { expected: u32, got: u32 },

    #[error("vertex count mismatch: header says {expected}, stream has {got}")]
    VertexCountMismatch { expected: u32, got: u32 },

    #[error("submesh references out-of-bounds indices")]
    SubmeshOutOfBounds,

    #[error("glTF serialization failed: {0}")]
    Gltf(String),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("record '{record_name}' has no SGeometryResourceParams component")]
    NoGeometryComponent { record_name: String },

    #[error("file not found in P4k: {path}")]
    FileNotFoundInP4k { path: String },

    #[error("DataCore query error: {0}")]
    DataCoreQuery(#[from] starbreaker_datacore::QueryError),

    #[error("P4k error: {0}")]
    P4k(starbreaker_p4k::P4kError),

    #[error(transparent)]
    CryXml(#[from] starbreaker_cryxml::CryXmlError),

    #[error("socpak not found in P4k: {0}")]
    MissingSocpak(String),

    #[error("P4k read error: {0}")]
    P4kRead(String),

    #[error("chunk parse error: {0}")]
    ChunkParse(String),

    #[error("{0}")]
    Other(String),
}
