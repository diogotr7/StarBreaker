pub mod chf_data;
pub mod container;
pub mod dna;
pub mod error;
pub mod itemport;
pub mod material;
pub(crate) mod read_helpers;

// Re-export primary types at crate root.
pub use chf_data::ChfData;
pub use container::ChfFile;
pub use dna::{Dna, DnaBlend, FacePart};
pub use error::ChfError;
pub use itemport::ItemPort;
pub use material::{MaterialDefinition, MaterialParam, SubMaterial, Texture};

use starbreaker_common::ParseError;

/// Parse decompressed CHF binary data into structured ChfData.
pub fn parse_chf(decompressed: &[u8]) -> Result<ChfData, ParseError> {
    ChfData::read(decompressed)
}

/// Serialize structured ChfData back to binary bytes.
pub fn write_chf(data: &ChfData) -> Vec<u8> {
    data.write()
}

/// Read a CHF container file, parse, and return pretty-printed JSON.
pub fn chf_to_json(chf_bytes: &[u8]) -> Result<String, ChfError> {
    let file = ChfFile::from_chf(chf_bytes)?;
    let data = file.parse()?;
    Ok(serde_json::to_string_pretty(&data)?)
}

/// Deserialize JSON into ChfData, write to binary, and compress into a CHF container.
pub fn json_to_chf(json: &str) -> Result<Vec<u8>, ChfError> {
    let data: ChfData = serde_json::from_str(json)?;
    let bin = write_chf(&data);
    let file = ChfFile::from_bin(bin);
    file.to_chf()
}

/// Decompress a CHF container and return the raw binary data.
pub fn decompress_chf(container: &[u8]) -> Result<Vec<u8>, ChfError> {
    let file = ChfFile::from_chf(container)?;
    Ok(file.data)
}

/// Compress raw binary data into a CHF container.
pub fn compress_chf(decompressed: &[u8]) -> Result<Vec<u8>, ChfError> {
    let file = ChfFile::from_bin(decompressed.to_vec());
    file.to_chf()
}

/// Parse raw DNA bytes (without size prefix) into a Dna struct.
pub fn parse_dna(bytes: &[u8]) -> Result<Dna, ParseError> {
    Dna::read_raw(bytes)
}

/// Return the raw bytes of a Dna struct.
pub fn write_dna(dna: &Dna) -> Vec<u8> {
    dna.raw_bytes.clone()
}
