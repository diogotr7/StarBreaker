use std::io::{Read, Seek, SeekFrom};

use rustc_hash::FxHashMap;
use starbreaker_common::SpanReader;

use crate::crypto;
use crate::error::P4kError;
use crate::types::*;

/// A single entry in a P4k archive.
#[derive(Debug, Clone)]
pub struct P4kEntry {
    pub name: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub compression_method: u16,
    pub is_encrypted: bool,
    pub offset: u64,
    pub crc32: u32,
    pub last_modified: u32,
}

/// An item returned by `list_dir` — either a file entry or a subdirectory name.
pub enum DirEntry<'a> {
    File(&'a P4kEntry),
    Directory(String),
}

/// A P4k archive backed by a borrowed byte slice.
pub struct P4kArchive<'a> {
    data: &'a [u8],
    entries: Vec<P4kEntry>,
    path_index: FxHashMap<String, usize>,
    lowercase_index: FxHashMap<String, usize>,
    sorted_index: Vec<u32>, // entry indices sorted by name, for prefix scanning
}

impl<'a> P4kArchive<'a> {
    /// Parse a P4k archive from a byte slice.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, P4kError> {
        let (entries, path_index, lowercase_index, sorted_index) =
            parse_central_directory(data, None)?;
        Ok(P4kArchive {
            data,
            entries,
            path_index,
            lowercase_index,
            sorted_index,
        })
    }

    /// Read and decompress an entry's data.
    pub fn read(&self, entry: &P4kEntry) -> Result<Vec<u8>, P4kError> {
        Self::read_from_data(self.data, entry)
    }

    /// Collect all unique parent directory paths from the entry list.
    ///
    /// Useful for pre-creating directories in bulk before parallel extraction,
    /// avoiding per-file `create_dir_all` overhead on NTFS.
    pub fn unique_directories(entries: &[P4kEntry]) -> Vec<&str> {
        let mut dirs = rustc_hash::FxHashSet::default();
        for entry in entries {
            // Walk up from the entry's parent to the root, collecting all
            // intermediate directories.
            let mut path = entry.name.as_str();
            while let Some(pos) = path.rfind('\\') {
                path = &path[..pos];
                if !dirs.insert(path) {
                    break; // already seen this and all its ancestors
                }
            }
        }
        let mut sorted: Vec<&str> = dirs.into_iter().collect();
        sorted.sort_unstable();
        sorted
    }

    /// Read and decompress an entry from raw archive data.
    ///
    /// This is a static method so both `P4kArchive` and `MappedP4k` can use it.
    pub fn read_from_data(data: &[u8], entry: &P4kEntry) -> Result<Vec<u8>, P4kError> {
        let offset = entry.offset as usize;
        if offset >= data.len() {
            return Err(P4kError::Parse(starbreaker_common::ParseError::Truncated {
                offset,
                need: size_of::<LocalFileHeader>(),
                have: 0,
            }));
        }

        let mut reader = SpanReader::new_at(data, offset);
        let local_header = reader.read_type::<LocalFileHeader>()?;
        let sig = local_header.signature;
        if sig != LOCAL_FILE_SIGNATURE && sig != LOCAL_FILE_CIG_SIGNATURE {
            return Err(P4kError::InvalidSignature {
                expected: LOCAL_FILE_SIGNATURE,
                got: sig,
            });
        }

        // Skip the file name and extra field to reach the raw data
        let skip =
            local_header.file_name_length as usize + local_header.extra_field_length as usize;
        reader.advance(skip)?;

        let raw = reader.read_bytes(entry.compressed_size as usize)?;

        let hint = entry.uncompressed_size as usize;
        match (entry.is_encrypted, entry.compression_method) {
            (true, 100) => {
                let decrypted = crypto::decrypt(raw)?;
                zstd_decompress(&decrypted, hint)
            }
            (false, 100) => zstd_decompress(raw, hint),
            (false, 8) => deflate_decompress(raw, hint),
            (false, 0) => Ok(raw.to_vec()),
            (true, method) => Err(P4kError::EncryptedNonZstd(method)),
            (_, method) => Err(P4kError::UnsupportedCompression(method)),
        }
    }

    /// Read and decompress an entry using a seekable file handle instead of mmap.
    ///
    /// Each caller should use its own file handle (opened with read + share-read)
    /// so multiple threads can extract concurrently without page-cache pressure.
    pub fn read_from_file(
        file: &mut (impl Read + Seek),
        entry: &P4kEntry,
    ) -> Result<Vec<u8>, P4kError> {
        // Seek to the local file header
        file.seek(SeekFrom::Start(entry.offset))?;

        // Read the local file header
        let mut header_buf = [0u8; size_of::<LocalFileHeader>()];
        file.read_exact(&mut header_buf)?;
        let local_header: LocalFileHeader =
            *zerocopy::FromBytes::ref_from_bytes(&header_buf).map_err(|_| {
                P4kError::Parse(starbreaker_common::ParseError::InvalidLayout(
                    "LocalFileHeader".to_string(),
                ))
            })?;

        let sig = local_header.signature;
        if sig != LOCAL_FILE_SIGNATURE && sig != LOCAL_FILE_CIG_SIGNATURE {
            return Err(P4kError::InvalidSignature {
                expected: LOCAL_FILE_SIGNATURE,
                got: sig,
            });
        }

        // Skip file name and extra field
        let skip =
            local_header.file_name_length as u64 + local_header.extra_field_length as u64;
        file.seek(SeekFrom::Current(skip as i64))?;

        // Read compressed data
        let mut raw = vec![0u8; entry.compressed_size as usize];
        file.read_exact(&mut raw)?;

        let hint = entry.uncompressed_size as usize;
        match (entry.is_encrypted, entry.compression_method) {
            (true, 100) => {
                let decrypted = crypto::decrypt(&raw)?;
                zstd_decompress(&decrypted, hint)
            }
            (false, 100) => zstd_decompress(&raw, hint),
            (false, 8) => deflate_decompress(&raw, hint),
            (false, 0) => Ok(raw),
            (true, method) => Err(P4kError::EncryptedNonZstd(method)),
            (_, method) => Err(P4kError::UnsupportedCompression(method)),
        }
    }

    /// Get all entries.
    pub fn entries(&self) -> &[P4kEntry] {
        &self.entries
    }

    /// Look up an entry by path.
    pub fn entry(&self, path: &str) -> Option<&P4kEntry> {
        self.path_index.get(path).map(|&i| &self.entries[i])
    }

    /// Look up an entry by path, case-insensitively.
    pub fn entry_case_insensitive(&self, path: &str) -> Option<&P4kEntry> {
        self.lowercase_index
            .get(&path.to_ascii_lowercase())
            .map(|&i| &self.entries[i])
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// List immediate children (files and subdirectories) under a directory path.
    ///
    /// `dir_path` should NOT have a trailing backslash (e.g., `"Data\\Objects"`).
    /// Returns files whose parent directory matches, and unique subdirectory names.
    pub fn list_dir(&self, dir_path: &str) -> Vec<DirEntry<'_>> {
        let prefix = if dir_path.is_empty() {
            String::new()
        } else {
            format!("{dir_path}\\")
        };

        // Binary search to find the first entry with our prefix
        let start = self
            .sorted_index
            .partition_point(|&idx| self.entries[idx as usize].name.as_str() < prefix.as_str());

        let mut result = Vec::new();
        let mut seen_dirs = rustc_hash::FxHashSet::default();

        for &idx in &self.sorted_index[start..] {
            let name = &self.entries[idx as usize].name;

            // Stop once we're past the prefix
            if !name.starts_with(&prefix) {
                break;
            }

            // Get the remainder after the prefix
            let rest = &name[prefix.len()..];

            if let Some(slash_pos) = rest.find('\\') {
                // Has a subdirectory — collect the directory name
                let subdir = &rest[..slash_pos];
                if seen_dirs.insert(subdir.to_string()) {
                    result.push(DirEntry::Directory(subdir.to_string()));
                }
            } else {
                // Direct child file
                result.push(DirEntry::File(&self.entries[idx as usize]));
            }
        }

        result
    }
}

// ── Internal parsing ─────────────────────────────────────────────────────────

/// Parsed central directory: entries, exact-case index, lowercase index, sorted offsets.
pub(crate) type CentralDirectory = (
    Vec<P4kEntry>,
    FxHashMap<String, usize>,
    FxHashMap<String, usize>,
    Vec<u32>,
);

/// Location of the central directory within an archive.
struct CdLocation {
    total_entries: u64,
    cd_offset: u64,
    cd_size: u64,
    is_zip64: bool,
}

/// Locate the central directory from the tail of an archive.
///
/// `tail_data` is the last N bytes of the file.
/// `tail_file_offset` is the absolute file offset where `tail_data` starts.
fn locate_central_directory(
    tail_data: &[u8],
    tail_file_offset: u64,
) -> Result<CdLocation, P4kError> {
    let eocd_offset = find_eocd(tail_data)?;

    let mut reader = SpanReader::new_at(tail_data, eocd_offset);
    let eocd = reader.read_type::<EocdRecord>()?;

    if eocd.signature != EOCD_SIGNATURE {
        return Err(P4kError::InvalidSignature {
            expected: EOCD_SIGNATURE,
            got: eocd.signature,
        });
    }

    let is_zip64 = eocd.is_zip64();

    if is_zip64 {
        let locator_offset = find_zip64_locator(tail_data, eocd_offset)?;
        let mut loc_reader = SpanReader::new_at(tail_data, locator_offset);
        let locator = loc_reader.read_type::<Zip64Locator>()?;

        if locator.signature != ZIP64_LOCATOR_SIGNATURE {
            return Err(P4kError::InvalidSignature {
                expected: ZIP64_LOCATOR_SIGNATURE,
                got: locator.signature,
            });
        }

        // eocd64_offset is absolute — convert to buffer-relative
        let eocd64_abs = locator.eocd64_offset;
        let eocd64_rel = eocd64_abs
            .checked_sub(tail_file_offset)
            .ok_or(P4kError::EocdNotFound)? as usize;
        let mut eocd64_reader = SpanReader::new_at(tail_data, eocd64_rel);
        let eocd64 = eocd64_reader.read_type::<Eocd64Record>()?;

        if eocd64.signature != EOCD64_SIGNATURE {
            return Err(P4kError::InvalidSignature {
                expected: EOCD64_SIGNATURE,
                got: eocd64.signature,
            });
        }

        Ok(CdLocation {
            total_entries: eocd64.total_entries,
            cd_offset: eocd64.central_directory_offset,
            cd_size: eocd64.central_directory_size,
            is_zip64: true,
        })
    } else {
        Ok(CdLocation {
            total_entries: eocd.total_entries as u64,
            cd_offset: eocd.central_directory_offset as u64,
            cd_size: eocd.central_directory_size as u64,
            is_zip64: false,
        })
    }
}

/// Parse central directory entries from a byte buffer and build indexes.
fn parse_entries(
    cd_data: &[u8],
    total_entries: u64,
    is_zip64: bool,
    progress: Option<&starbreaker_common::Progress>,
) -> Result<CentralDirectory, P4kError> {
    let mut cd_reader = SpanReader::new(cd_data);
    let mut entries = Vec::with_capacity(total_entries as usize);

    for i in 0..total_entries {
        entries.push(read_entry(&mut cd_reader, is_zip64)?);
        if i % 10_000 == 0 {
            starbreaker_common::progress::report(
                progress,
                i as f32 / total_entries.max(1) as f32,
                &format!("Parsing entries ({i}/{total_entries})"),
            );
        }
    }

    // Build path index — FxHashMap for exact lookup
    let mut path_index = FxHashMap::with_capacity_and_hasher(entries.len(), Default::default());
    for (i, entry) in entries.iter().enumerate() {
        path_index.insert(entry.name.clone(), i);
    }

    // Build lowercase index for case-insensitive lookup
    let mut lowercase_index =
        FxHashMap::with_capacity_and_hasher(entries.len(), Default::default());
    for (i, entry) in entries.iter().enumerate() {
        lowercase_index.insert(entry.name.to_ascii_lowercase(), i);
    }

    // Build sorted index for directory listing (prefix scan)
    let mut sorted_index: Vec<u32> = (0..entries.len() as u32).collect();
    sorted_index.sort_unstable_by(|&a, &b| entries[a as usize].name.cmp(&entries[b as usize].name));

    Ok((entries, path_index, lowercase_index, sorted_index))
}

/// Parse the central directory from raw archive data (in-memory byte slice).
///
/// Returns (entries, path_index, lowercase_index, sorted_index).
pub(crate) fn parse_central_directory(
    data: &[u8],
    progress: Option<&starbreaker_common::Progress>,
) -> Result<CentralDirectory, P4kError> {
    let loc = locate_central_directory(data, 0)?;
    let cd_data = &data[loc.cd_offset as usize..];
    parse_entries(cd_data, loc.total_entries, loc.is_zip64, progress)
}

/// Parse the central directory from a seekable file handle.
///
/// Reads only the EOCD tail and central directory — avoids mapping the entire file.
pub(crate) fn parse_central_directory_from_file(
    file: &mut (impl Read + Seek),
    progress: Option<&starbreaker_common::Progress>,
) -> Result<CentralDirectory, P4kError> {
    let file_len = file.seek(SeekFrom::End(0))?;

    // Read the tail of the file to find EOCD/EOCD64 structures.
    // Max EOCD search window: 22 (EOCD) + 65535 (comment) + 56 (EOCD64) + 20 (locator)
    let tail_size = (file_len as usize).min(22 + 65535 + 56 + 20);
    let tail_offset = file_len - tail_size as u64;
    file.seek(SeekFrom::Start(tail_offset))?;
    let mut tail = vec![0u8; tail_size];
    file.read_exact(&mut tail)?;

    let loc = locate_central_directory(&tail, tail_offset)?;

    // Read the central directory entries
    file.seek(SeekFrom::Start(loc.cd_offset))?;
    let mut cd_data = vec![0u8; loc.cd_size as usize];
    file.read_exact(&mut cd_data)?;

    parse_entries(&cd_data, loc.total_entries, loc.is_zip64, progress)
}

/// Search backward from the end of data for the EOCD signature.
fn find_eocd(data: &[u8]) -> Result<usize, P4kError> {
    // The EOCD record is at least 22 bytes, and at most 22 + 65535 bytes from the end
    let search_start = data.len().saturating_sub(22 + 65535);
    let search_end = data.len().saturating_sub(22);

    // Search backward for the magic bytes
    for i in (search_start..=search_end).rev() {
        if data[i..i + 4] == EOCD_MAGIC {
            return Ok(i);
        }
    }

    Err(P4kError::EocdNotFound)
}

/// Search backward from the EOCD for the ZIP64 locator.
fn find_zip64_locator(data: &[u8], eocd_offset: usize) -> Result<usize, P4kError> {
    let magic = ZIP64_LOCATOR_SIGNATURE.to_le_bytes();
    // The locator is typically right before the EOCD, search backward
    let search_start = eocd_offset.saturating_sub(22 + 65535);

    for i in (search_start..eocd_offset).rev() {
        if i + 4 <= data.len() && data[i..i + 4] == magic {
            return Ok(i);
        }
    }

    Err(P4kError::EocdNotFound)
}

/// Read a single central directory entry.
fn read_entry(reader: &mut SpanReader, is_zip64: bool) -> Result<P4kEntry, P4kError> {
    let header = reader.read_type::<CentralDirHeader>()?;

    if header.signature != CENTRAL_DIR_SIGNATURE {
        return Err(P4kError::InvalidSignature {
            expected: CENTRAL_DIR_SIGNATURE,
            got: header.signature,
        });
    }

    // Read file name — single allocation, normalize separators in-place
    let name_bytes = reader.read_bytes(header.file_name_length as usize)?;
    let mut name = String::with_capacity(name_bytes.len());
    for &b in name_bytes {
        if b == b'/' {
            name.push('\\');
        } else {
            name.push(b as char);
        }
    }

    let mut compressed_size = header.compressed_size as u64;
    let mut uncompressed_size = header.uncompressed_size as u64;
    let mut local_header_offset = header.local_header_offset as u64;
    let mut is_encrypted = false;

    if is_zip64 {
        // Parse extra fields for ZIP64 entries
        // The C# code reads extra fields in a specific order:
        // 1. Tag 0x0001 (standard ZIP64 extended info)
        // 2. Tag 0x5000 (CIG custom)
        // 3. Tag 0x5002 (CIG encryption flag)
        // 4. Tag 0x5003 (CIG custom)

        let extra_data = reader.read_bytes(header.extra_field_length as usize)?;
        let mut extra_reader = SpanReader::new(extra_data);

        // --- Extra field 0x0001: ZIP64 extended sizes ---
        let tag1 = extra_reader.read_u16()?;
        if tag1 != 0x0001 {
            return Err(P4kError::Parse(
                starbreaker_common::ParseError::UnexpectedValue {
                    offset: extra_reader.position(),
                    expected: "0x0001".to_string(),
                    actual: format!("{:#06x}", tag1),
                },
            ));
        }
        let _zip64_data_size = extra_reader.read_u16()?;

        // Read u64 values in order for fields that were 0xFFFFFFFF
        if header.uncompressed_size == 0xFFFFFFFF {
            uncompressed_size = extra_reader.read_u64()?;
        }
        if header.compressed_size == 0xFFFFFFFF {
            compressed_size = extra_reader.read_u64()?;
        }
        if header.local_header_offset == 0xFFFFFFFF {
            local_header_offset = extra_reader.read_u64()?;
        }
        if header.disk_number_start == 0xFFFF {
            let _disk = extra_reader.read_u32()?;
        }

        // --- Extra field 0x5000: CIG custom ---
        let tag2 = extra_reader.read_u16()?;
        if tag2 != 0x5000 {
            return Err(P4kError::Parse(
                starbreaker_common::ParseError::UnexpectedValue {
                    offset: extra_reader.position(),
                    expected: "0x5000".to_string(),
                    actual: format!("{:#06x}", tag2),
                },
            ));
        }
        let size_5000 = extra_reader.read_u16()?;
        // The C# code advances by size - 4, but we already read the tag+size header
        // outside the data portion. The "size" field here is the data length.
        // Looking at the C# code: it reads tag, then size, then advances size - 4.
        // This means the "size" includes 4 bytes already consumed (2 unknown u16 values?).
        // Let's match C# exactly: advance(size - 4)
        extra_reader.advance((size_5000 as usize).saturating_sub(4))?;

        // --- Extra field 0x5002: Encryption flag ---
        let tag3 = extra_reader.read_u16()?;
        if tag3 != 0x5002 {
            return Err(P4kError::Parse(
                starbreaker_common::ParseError::UnexpectedValue {
                    offset: extra_reader.position(),
                    expected: "0x5002".to_string(),
                    actual: format!("{:#06x}", tag3),
                },
            ));
        }
        let size_5002 = extra_reader.read_u16()?;
        if size_5002 != 6 {
            return Err(P4kError::Parse(
                starbreaker_common::ParseError::UnexpectedValue {
                    offset: extra_reader.position(),
                    expected: "6".to_string(),
                    actual: format!("{}", size_5002),
                },
            ));
        }

        let enc_flag = extra_reader.read_u16()?;
        is_encrypted = enc_flag == 1;

        // --- Extra field 0x5003: CIG custom ---
        let tag4 = extra_reader.read_u16()?;
        if tag4 != 0x5003 {
            return Err(P4kError::Parse(
                starbreaker_common::ParseError::UnexpectedValue {
                    offset: extra_reader.position(),
                    expected: "0x5003".to_string(),
                    actual: format!("{:#06x}", tag4),
                },
            ));
        }
        let size_5003 = extra_reader.read_u16()?;
        extra_reader.advance((size_5003 as usize).saturating_sub(4))?;
    } else {
        // Non-ZIP64: skip extra fields and file comment
        let skip = header.extra_field_length as usize;
        reader.advance(skip)?;
    }

    // Skip file comment
    if header.file_comment_length > 0 {
        if is_zip64 {
            // For ZIP64, the extra fields were already consumed above.
            // The file comment is read from the main reader.
        }
        reader.advance(header.file_comment_length as usize)?;
    }

    Ok(P4kEntry {
        name,
        compressed_size,
        uncompressed_size,
        compression_method: header.compression_method,
        is_encrypted,
        offset: local_header_offset,
        crc32: header.crc32,
        last_modified: header.last_modified,
    })
}

/// Decompress zstd data with a pre-allocation hint.
fn zstd_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, P4kError> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(cursor)
        .map_err(|e| P4kError::Decompression(format!("zstd init: {e}")))?;
    let mut output = Vec::with_capacity(size_hint);
    decoder
        .read_to_end(&mut output)
        .map_err(|e| P4kError::Decompression(format!("zstd: {e}")))?;
    Ok(output)
}

/// Decompress deflate data with a pre-allocation hint.
fn deflate_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, P4kError> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = flate2::read::DeflateDecoder::new(cursor);
    let mut output = Vec::with_capacity(size_hint);
    decoder
        .read_to_end(&mut output)
        .map_err(|e| P4kError::Decompression(format!("deflate: {e}")))?;
    Ok(output)
}
