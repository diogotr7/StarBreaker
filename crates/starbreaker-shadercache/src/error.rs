//! Error type for shader-cache access.

use thiserror::Error;

/// Everything that can go wrong while opening a shader cache or a shader.
#[derive(Debug, Error)]
pub enum ShaderCacheError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("p4k error: {0}")]
    P4k(#[from] starbreaker_p4k::P4kError),
    #[error("no zip end-of-central-directory record found")]
    NoEndOfCentralDirectory,
    #[error("malformed zip structure: {0}")]
    MalformedZip(&'static str),
    #[error("unsupported compression method {0}")]
    UnsupportedCompression(u16),
    #[error("entry {0:?} not found")]
    EntryNotFound(String),
    #[error("shader cache entry is not an SHCB block")]
    BadShcbMagic,
    #[error("shader bin entry is not an FXB1 block")]
    BadShaderBinMagic,
    #[error("malformed shader bin: {0}")]
    MalformedShaderBin(&'static str),
    #[error("grammar file line {0} is malformed")]
    MalformedGrammar(usize),
    #[error(
        "shader bin cache version {found:#010x} is not supported (this build decodes {supported:#010x} only)"
    )]
    UnsupportedCacheVersion { found: u32, supported: u32 },
    #[error("shader bin does not decode cleanly: {0}")]
    UntrustedDecode(crate::hlsl::DecodeIssue),
    #[error("shader payload is not SPIR-V (magic {0:#010x})")]
    NotSpirv(u32),
    #[error("spir-v parse error: {0}")]
    SpirvParse(String),
    #[error("shader defaults: {0}")]
    Defaults(String),
}
