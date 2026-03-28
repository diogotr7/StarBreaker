use starbreaker_common::ParseError;
use starbreaker_common::reader::SpanReader;

#[derive(Debug, Clone)]
pub struct MaterialName {
    pub name: String,
}

impl MaterialName {
    pub fn read(data: &[u8]) -> Result<Self, ParseError> {
        let mut reader = SpanReader::new(data);
        let name_bytes = reader.read_bytes(128)?;
        let name = name_bytes
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect::<String>();
        Ok(Self { name })
    }
}
