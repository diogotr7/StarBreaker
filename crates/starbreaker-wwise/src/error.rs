#[derive(Debug, thiserror::Error)]
pub enum BnkError {
    #[error(transparent)]
    Parse(#[from] starbreaker_common::ParseError),

    #[error("missing required section: {tag}")]
    MissingSection { tag: String },

    #[error("unknown section tag: {0:#010x}")]
    UnknownSection(u32),

    #[error("WEM entry {id} not found in data index")]
    WemNotFound { id: u32 },

    #[error("data section too small: entry at offset {offset} + size {size} exceeds {data_len}")]
    DataOverflow {
        offset: u32,
        size: u32,
        data_len: usize,
    },

    #[error(transparent)]
    Wem(#[from] starbreaker_wem::WemError),

    #[error("XML parse error: {0}")]
    Xml(String),

    #[error("P4k read error: {0}")]
    P4k(#[from] starbreaker_p4k::P4kError),
}
