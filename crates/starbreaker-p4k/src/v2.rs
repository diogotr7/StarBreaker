use crate::{P4kEntry, P4kError};
use starbreaker_common::{ParseError, SpanReader};
use std::io::{Read, Seek, SeekFrom};

const FOOTER_SIZE: usize = 175;
const ENTRY_SIZE: u64 = 204;

fn invalid(message: &str) -> P4kError {
    P4kError::Parse(ParseError::InvalidLayout(message.to_owned()))
}

pub(crate) fn is_v2(tail: &[u8]) -> bool {
    tail.len() >= FOOTER_SIZE && tail.ends_with(&[2, 0, 0x4a, 0x69, 0x6a, 0x69])
}

fn read_range(
    file: &mut (impl Read + Seek),
    offset: u64,
    size: u64,
    end: u64,
) -> Result<Vec<u8>, P4kError> {
    if offset.checked_add(size).is_none_or(|limit| limit > end) {
        return Err(invalid("v2 archive range is outside the payload"));
    }
    let size = usize::try_from(size).map_err(|_| invalid("v2 range exceeds address space"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| invalid("v2 range allocation failed"))?;
    bytes.resize(size, 0);
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn read_entries(
    file: &mut (impl Read + Seek),
    tail: &[u8],
    file_len: u64,
    progress: Option<&starbreaker_common::Progress>,
) -> Result<Vec<P4kEntry>, P4kError> {
    let mut footer = SpanReader::new(&tail[tail.len() - FOOTER_SIZE..]);
    let count = footer.read_u64()?;
    footer.advance(8)?;
    let directory_offset = footer.read_u64()?;
    let directory_size = footer.read_u64()?;
    footer.advance(8)?;
    let names_offset = footer.read_u64()?;
    let names_size = footer.read_u64()?;
    if count > u32::MAX as u64 || count.checked_mul(ENTRY_SIZE) != Some(directory_size) {
        return Err(invalid("v2 directory size does not match entry count"));
    }
    let end = file_len
        .checked_sub(FOOTER_SIZE as u64)
        .ok_or_else(|| invalid("truncated v2 footer"))?;
    let directory = read_range(file, directory_offset, directory_size, end)?;
    let names = read_range(file, names_offset, names_size, end)?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count as usize)
        .map_err(|_| invalid("v2 entry allocation failed"))?;
    for (index, bytes) in directory.chunks_exact(ENTRY_SIZE as usize).enumerate() {
        let mut reader = SpanReader::new(bytes);
        let compression_method = reader.read_u16()?;
        let time = reader.read_u16()?;
        let date = reader.read_u16()?;
        let crc32 = reader.read_u32()?;
        let compressed_size = reader.read_u64()?;
        let uncompressed_size = reader.read_u64()?;
        let offset = reader.read_u64()?;
        let name_offset = usize::try_from(reader.read_u64()?)
            .map_err(|_| invalid("v2 name offset exceeds address space"))?;
        reader.advance(128)?;
        let encryption = reader.read_u16()?;
        if encryption > 1
            || offset
                .checked_add(compressed_size)
                .is_none_or(|limit| limit > end)
        {
            return Err(invalid("invalid v2 payload range or encryption flag"));
        }
        let name_tail = names
            .get(name_offset..)
            .ok_or_else(|| invalid("v2 name offset outside filename table"))?;
        let name_end = name_tail
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| invalid("unterminated v2 filename"))?;
        let name = std::str::from_utf8(&name_tail[..name_end])
            .map_err(|_| invalid("non-UTF8 v2 filename"))?
            .replace('/', "\\");
        if name.is_empty() {
            return Err(invalid("empty v2 filename"));
        }
        entries.push(P4kEntry {
            name,
            compressed_size,
            uncompressed_size,
            compression_method,
            is_encrypted: encryption == 1,
            offset,
            crc32,
            last_modified: ((date as u32) << 16) | time as u32,
            has_local_header: false,
        });
        if index % 10_000 == 0 {
            starbreaker_common::progress::report(
                progress,
                index as f32 / count.max(1) as f32,
                "Reading P4K v2 directory",
            );
        }
    }
    Ok(entries)
}
