use std::io::{Read, Seek, SeekFrom};
use std::cmp::Ordering;

use rustc_hash::FxHasher;
use hashbrown::HashTable;
use std::hash::{Hash, Hasher};
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
    /// Raw ZIP "last mod" timestamp: lower 16 bits = DOS time (h:m:s/2),
    /// upper 16 bits = DOS date (year-1980, month, day). Use
    /// [`Self::last_modified_unix`] to get Unix seconds.
    pub last_modified: u32,
}

impl P4kEntry {
    /// Decode the ZIP DOS `last_modified` field into Unix seconds since the
    /// epoch. Returns 0 if the timestamp is unset or the encoded date is
    /// invalid (e.g. month 0).
    pub fn last_modified_unix(&self) -> i64 {
        if self.last_modified == 0 {
            return 0;
        }
        let time = self.last_modified & 0xFFFF;
        let date = (self.last_modified >> 16) & 0xFFFF;
        let year = ((date >> 9) & 0x7F) + 1980;
        let month = (date >> 5) & 0x0F;
        let day = date & 0x1F;
        let hour = (time >> 11) & 0x1F;
        let minute = (time >> 5) & 0x3F;
        let second = (time & 0x1F) * 2;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return 0;
        }
        let days = days_from_civil(year as i32, month, day);
        days * 86_400 + (hour as i64) * 3600 + (minute as i64) * 60 + (second as i64)
    }
}

/// Howard Hinnant's days-from-civil algorithm. Returns days since 1970-01-01,
/// negative for earlier dates. Proleptic Gregorian, no time-zone or leap-second
/// awareness — fine for ZIP DOS dates which are local-time-naive.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146_097 + doe as i64 - 719_468
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
    path_index: HashTable<u32>,
    sorted_index: Vec<u32>,
    lowercase_names: Vec<String>,
    sorted_lower_index: Vec<u32>,
}

impl<'a> P4kArchive<'a> {
    /// Parse a P4k archive from a byte slice.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, P4kError> {
        let (entries, path_index, sorted_index, lowercase_names, sorted_lower_index) =
            parse_central_directory(data, None)?;
        Ok(P4kArchive {
            data,
            entries,
            path_index,
            sorted_index,
            lowercase_names,
            sorted_lower_index,
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

    /// Read and decompress an entry using positional reads on a borrowed
    /// `&File`.
    ///
    /// Passes the offset explicitly so concurrent callers don't coordinate
    /// on a cursor — the same `&File` handle is safe to share across threads.
    /// Two syscalls per read (header + data); no seeks.
    pub fn read_from_file_at(file: &std::fs::File, entry: &P4kEntry) -> Result<Vec<u8>, P4kError> {
        use crate::posread::pread_exact;

        // 1. Local file header
        let mut header_buf = [0u8; size_of::<LocalFileHeader>()];
        pread_exact(file, &mut header_buf, entry.offset)?;
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

        // 2. Compressed data starts past the variable-length filename + extra field
        let data_offset = entry.offset
            + size_of::<LocalFileHeader>() as u64
            + local_header.file_name_length as u64
            + local_header.extra_field_length as u64;

        let mut raw = vec![0u8; entry.compressed_size as usize];
        pread_exact(file, &mut raw, data_offset)?;

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
        let h = hash_path(path);
        let i = *self
            .path_index
            .find(h, |&j| self.entries[j as usize].name == path)?;
        Some(&self.entries[i as usize])
    }

    /// Returns entry indices whose lowercased name contains every
    /// whitespace-separated token in `query`. Order is unspecified;
    /// callers sort.
    pub fn search(&self, query: &str) -> Vec<u32> {
        use rayon::prelude::*;
        let tokens: smallvec::SmallVec<[String; 4]> = query
            .split_ascii_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        if tokens.is_empty() {
            return Vec::new();
        }

        self.lowercase_names
            .par_iter()
            .enumerate()
            .filter_map(|(i, name)| {
                tokens
                    .iter()
                    .all(|t| name.contains(t.as_str()))
                    .then_some(i as u32)
            })
            .collect()
    }

    /// Look up an entry by path, case-insensitively.
    ///
    /// Allocates nothing per call: lowercases the needle on the fly during
    /// the binary search and uses `eq_ignore_ascii_case` for the equality
    /// check.
    pub fn entry_case_insensitive(&self, path: &str) -> Option<&P4kEntry> {
        let pos = self.sorted_lower_index.partition_point(|&i| {
            cmp_lower_against(&self.lowercase_names[i as usize], path) == Ordering::Less
        });
        let idx = *self.sorted_lower_index.get(pos)? as usize;
        if self.lowercase_names[idx].eq_ignore_ascii_case(path) {
            Some(&self.entries[idx])
        } else {
            None
        }
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

/// Compare a fully-lowercased haystack against a possibly-mixed-case needle,
/// lowercasing the needle byte-by-byte without allocating.
///
/// Used for binary search over `sorted_lower_index`. Correct because
/// `to_ascii_lowercase` is a no-op on non-ASCII bytes, matching the order
/// produced by the materialised `lowercase_names` vec.
#[inline]
pub(crate) fn cmp_lower_against(haystack_lower: &str, needle_mixed: &str) -> Ordering {
    let h = haystack_lower.as_bytes();
    let n = needle_mixed.as_bytes();
    let len = h.len().min(n.len());
    for i in 0..len {
        let nb = n[i].to_ascii_lowercase();
        match h[i].cmp(&nb) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    h.len().cmp(&n.len())
}

// ── Internal parsing ─────────────────────────────────────────────────────────

/// Hash a path with FxHash for use as the key in `path_index`.
#[inline]
pub(crate) fn hash_path(s: &str) -> u64 {
    let mut h = FxHasher::default();
    s.hash(&mut h);
    h.finish()
}

/// Parsed central directory:
/// - entries
/// - path_index (exact case)
/// - sorted_index (case-sensitive, for prefix scans)
/// - lowercase_names (parallel to entries)
/// - sorted_lower_index (sorted by lowercase_names[i])
pub(crate) type CentralDirectory = (
    Vec<P4kEntry>,
    HashTable<u32>,            // path_index (exact case, keyed by entry index)
    Vec<u32>,                  // sorted_index (case-sensitive, for prefix scans)
    Vec<String>,               // lowercase_names (parallel to entries)
    Vec<u32>,                  // sorted_lower_index (sorted by lowercase_names[i])
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

    // Build path index — HashTable<u32> keyed by entry index, hashing through entries[i].name.
    // Avoids the full-path String key duplication of FxHashMap<String, usize>.
    let mut path_index: HashTable<u32> = HashTable::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        let h = hash_path(&entry.name);
        path_index.insert_unique(h, i as u32, |&j| hash_path(&entries[j as usize].name));
    }

    // Parallel lowercased view used by search and entry_case_insensitive.
    let lowercase_names: Vec<String> = entries.iter().map(|e| e.name.to_ascii_lowercase()).collect();

    // Case-sensitive sorted index for list_dir / list_subdirs.
    let mut sorted_index: Vec<u32> = (0..entries.len() as u32).collect();
    sorted_index.sort_unstable_by(|&a, &b| entries[a as usize].name.cmp(&entries[b as usize].name));

    // Case-insensitive sorted index for entry_case_insensitive.
    let mut sorted_lower_index: Vec<u32> = (0..entries.len() as u32).collect();
    sorted_lower_index.sort_unstable_by(|&a, &b| {
        lowercase_names[a as usize].cmp(&lowercase_names[b as usize])
    });

    Ok((entries, path_index, sorted_index, lowercase_names, sorted_lower_index))
}

/// Parse the central directory from raw archive data (in-memory byte slice).
///
/// Returns (entries, path_index, sorted_index, lowercase_names, sorted_lower_index).
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

    // Read file name — copy once, swap '/' to '\' in place, validate UTF-8.
    let name_bytes = reader.read_bytes(header.file_name_length as usize)?;
    let mut bytes = name_bytes.to_vec();
    for b in bytes.iter_mut() {
        if *b == b'/' {
            *b = b'\\';
        }
    }
    let name = String::from_utf8(bytes).map_err(|_| {
        P4kError::Parse(starbreaker_common::ParseError::InvalidLayout(
            "non-utf8 entry name".to_string(),
        ))
    })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str) -> P4kEntry {
        P4kEntry {
            name: name.to_string(),
            compressed_size: 0,
            uncompressed_size: 0,
            compression_method: 0,
            is_encrypted: false,
            offset: 0,
            crc32: 0,
            last_modified: 0,
        }
    }

    #[test]
    fn case_insensitive_lookup_finds_mixed_case_entries() {
        let entries = vec![
            make_entry("Data\\Foo.MTL"),
            make_entry("data\\BAR.xml"),
            make_entry("Other\\baz.dds"),
        ];
        let lowercase_names: Vec<String> =
            entries.iter().map(|e| e.name.to_ascii_lowercase()).collect();
        let mut sorted_lower_index: Vec<u32> = (0..entries.len() as u32).collect();
        sorted_lower_index.sort_unstable_by(|&a, &b| {
            lowercase_names[a as usize].cmp(&lowercase_names[b as usize])
        });

        let archive = P4kArchive {
            data: &[],
            entries,
            path_index: HashTable::new(),
            sorted_index: Vec::new(),
            lowercase_names,
            sorted_lower_index,
        };

        assert_eq!(
            archive.entry_case_insensitive("DATA\\foo.mtl").map(|e| e.name.as_str()),
            Some("Data\\Foo.MTL")
        );
        assert_eq!(
            archive.entry_case_insensitive("data\\bar.xml").map(|e| e.name.as_str()),
            Some("data\\BAR.xml")
        );
        assert!(archive.entry_case_insensitive("nope").is_none());
    }

    fn make_archive_for_search() -> P4kArchive<'static> {
        let entries = vec![
            make_entry("Data\\Objects\\Spaceships\\Ships\\AEGS\\Hornet\\hornet.cga"),
            make_entry("Data\\Objects\\Spaceships\\Ships\\AEGS\\Hornet\\hornet_glass.mtl"),
            make_entry("Data\\Objects\\Spaceships\\Ships\\RSI\\Aurora\\aurora.cga"),
            make_entry("Data\\Textures\\hornet_diffuse.dds"),
        ];
        let lowercase_names: Vec<String> =
            entries.iter().map(|e| e.name.to_ascii_lowercase()).collect();
        let mut sorted_lower_index: Vec<u32> = (0..entries.len() as u32).collect();
        sorted_lower_index.sort_unstable_by(|&a, &b| {
            lowercase_names[a as usize].cmp(&lowercase_names[b as usize])
        });

        P4kArchive {
            data: &[],
            entries,
            path_index: HashTable::new(),
            sorted_index: Vec::new(),
            lowercase_names,
            sorted_lower_index,
        }
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let a = make_archive_for_search();
        assert!(a.search("").is_empty());
        assert!(a.search("   ").is_empty());
    }

    #[test]
    fn search_single_token_substring_match() {
        let a = make_archive_for_search();
        let mut hits: Vec<&str> = a
            .search("hornet")
            .into_iter()
            .map(|i| a.entries[i as usize].name.as_str())
            .collect();
        hits.sort();
        assert_eq!(
            hits,
            vec![
                "Data\\Objects\\Spaceships\\Ships\\AEGS\\Hornet\\hornet.cga",
                "Data\\Objects\\Spaceships\\Ships\\AEGS\\Hornet\\hornet_glass.mtl",
                "Data\\Textures\\hornet_diffuse.dds",
            ]
        );
    }

    #[test]
    fn search_multi_token_is_and() {
        let a = make_archive_for_search();
        let mut hits: Vec<&str> = a
            .search("hornet glass")
            .into_iter()
            .map(|i| a.entries[i as usize].name.as_str())
            .collect();
        hits.sort();
        assert_eq!(hits, vec!["Data\\Objects\\Spaceships\\Ships\\AEGS\\Hornet\\hornet_glass.mtl"]);
    }

    #[test]
    fn search_is_case_insensitive() {
        let a = make_archive_for_search();
        let upper: Vec<u32> = a.search("HORNET");
        let lower: Vec<u32> = a.search("hornet");
        let mut u = upper.clone();
        let mut l = lower.clone();
        u.sort();
        l.sort();
        assert_eq!(u, l);
    }

    fn entry_with_dos(last_modified: u32) -> P4kEntry {
        let mut e = make_entry("x");
        e.last_modified = last_modified;
        e
    }

    #[test]
    fn dos_timestamp_zero_is_zero() {
        assert_eq!(entry_with_dos(0).last_modified_unix(), 0);
    }

    #[test]
    fn dos_timestamp_decodes_known_value() {
        // 2020-01-01 00:00:00 — easy to verify by hand.
        // year - 1980 = 40 (0b0101000); month = 1; day = 1.
        // date = (40 << 9) | (1 << 5) | 1 = 0x5021
        // time = 0
        let packed = 0x5021_0000u32;
        let secs = entry_with_dos(packed).last_modified_unix();
        // 2020-01-01T00:00:00Z (ZIP times are time-zone-naive; we treat them as UTC).
        assert_eq!(secs, 1_577_836_800);
    }

    #[test]
    fn dos_timestamp_invalid_month_returns_zero() {
        // month = 0 → invalid
        let packed = (0x5800u32 << 16) | 0x0000u32;
        assert_eq!(entry_with_dos(packed).last_modified_unix(), 0);
    }

    /// Lookup must not allocate even when the needle has uppercase bytes.
    /// We can't observe allocations directly here, so this test pins
    /// behavior across casings and around the binary-search boundary.
    #[test]
    fn case_insensitive_lookup_handles_mixed_case_and_boundaries() {
        let entries = vec![
            make_entry("a"),
            make_entry("aaa"),
            make_entry("AAB"), // sort-adjacent to "aaa" lowercased
            make_entry("z"),
        ];
        let lowercase_names: Vec<String> =
            entries.iter().map(|e| e.name.to_ascii_lowercase()).collect();
        let mut sorted_lower_index: Vec<u32> = (0..entries.len() as u32).collect();
        sorted_lower_index.sort_unstable_by(|&a, &b| {
            lowercase_names[a as usize].cmp(&lowercase_names[b as usize])
        });

        let archive = P4kArchive {
            data: &[],
            entries,
            path_index: HashTable::new(),
            sorted_index: Vec::new(),
            lowercase_names,
            sorted_lower_index,
        };

        // Exact case
        assert_eq!(archive.entry_case_insensitive("a").map(|e| e.name.as_str()), Some("a"));
        // Upper -> matches lowercase entry
        assert_eq!(archive.entry_case_insensitive("AAA").map(|e| e.name.as_str()), Some("aaa"));
        // Lower -> matches uppercase entry
        assert_eq!(archive.entry_case_insensitive("aab").map(|e| e.name.as_str()), Some("AAB"));
        // Miss in the middle of the sort range
        assert!(archive.entry_case_insensitive("aac").is_none());
        // Miss past the end
        assert!(archive.entry_case_insensitive("zz").is_none());
        // Miss before the start
        assert!(archive.entry_case_insensitive("").is_none());
    }

    /// Positional reads on a single shared `File` handle must not corrupt
    /// each other when called concurrently.
    #[test]
    fn pread_exact_is_thread_safe() {
        use std::fs::File;
        use std::io::Write;
        use std::sync::Arc;
        use std::thread;

        // Build a temp file with predictable content: byte at offset i is (i % 251).
        let mut path = std::env::temp_dir();
        path.push(format!("starbreaker-pread-{}.bin", std::process::id()));
        {
            let mut f = File::create(&path).expect("create temp");
            let mut buf = vec![0u8; 65_536];
            for (i, b) in buf.iter_mut().enumerate() {
                *b = (i % 251) as u8;
            }
            f.write_all(&buf).expect("write temp");
        }

        let f = Arc::new(File::open(&path).expect("open temp"));
        let mut handles = Vec::new();
        for t in 0..8 {
            let f = Arc::clone(&f);
            handles.push(thread::spawn(move || {
                let mut buf = [0u8; 17];
                for k in 0..1000 {
                    let off = ((t * 1000 + k) * 7) % (65_536 - 17);
                    crate::posread::pread_exact(&f, &mut buf, off as u64).expect("pread");
                    for (j, &b) in buf.iter().enumerate() {
                        assert_eq!(b, ((off + j) % 251) as u8, "mismatch at off={}", off + j);
                    }
                }
            }));
        }
        for h in handles { h.join().expect("thread"); }
        let _ = std::fs::remove_file(&path);
    }

    /// Build a `P4kArchive<'static>` from a list of entries, matching the
    /// production index construction.
    fn build_test_archive(entries: Vec<P4kEntry>) -> P4kArchive<'static> {
        let lowercase_names: Vec<String> =
            entries.iter().map(|e| e.name.to_ascii_lowercase()).collect();

        let mut sorted_index: Vec<u32> = (0..entries.len() as u32).collect();
        sorted_index.sort_unstable_by(|&a, &b| {
            entries[a as usize].name.cmp(&entries[b as usize].name)
        });

        let mut sorted_lower_index: Vec<u32> = (0..entries.len() as u32).collect();
        sorted_lower_index.sort_unstable_by(|&a, &b| {
            lowercase_names[a as usize].cmp(&lowercase_names[b as usize])
        });

        let mut path_index: hashbrown::HashTable<u32> =
            hashbrown::HashTable::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let h = hash_path(&e.name);
            path_index.insert_unique(h, i as u32, |&j| hash_path(&entries[j as usize].name));
        }

        P4kArchive {
            data: &[],
            entries,
            path_index,
            sorted_index,
            lowercase_names,
            sorted_lower_index,
        }
    }

    #[test]
    fn exact_lookup_roundtrip_all_entries() {
        let entries = vec![
            make_entry("Data\\Foo.MTL"),
            make_entry("data\\BAR.xml"),
            make_entry("Other\\baz.dds"),
        ];

        let archive = build_test_archive(entries);

        for e in &archive.entries {
            assert_eq!(
                archive.entry(&e.name).map(|x| x.name.as_str()),
                Some(e.name.as_str()),
                "exact-case roundtrip failed for {}",
                e.name
            );
        }
        assert!(archive.entry("Data\\foo.mtl").is_none(), "wrong case must miss");
        assert!(archive.entry("nope").is_none());
    }

    #[test]
    fn read_entry_rejects_non_utf8_name() {
        // Hand-built non-ZIP64 CentralDirHeader with a 1-byte name 0xFF.
        // 46-byte CentralDirHeader header + 1-byte name + 0 extra + 0 comment.
        let mut buf = Vec::with_capacity(46 + 1);
        // signature
        buf.extend_from_slice(&CENTRAL_DIR_SIGNATURE.to_le_bytes());
        // version_made_by, version_needed
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // flags
        buf.extend_from_slice(&[0, 0]);
        // compression_method = 0 (stored)
        buf.extend_from_slice(&[0, 0]);
        // last_modified u32
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // crc32 u32
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // compressed_size u32
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // uncompressed_size u32
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // file_name_length u16 = 1
        buf.extend_from_slice(&[1, 0]);
        // extra_field_length u16 = 0
        buf.extend_from_slice(&[0, 0]);
        // file_comment_length u16 = 0
        buf.extend_from_slice(&[0, 0]);
        // disk_number_start u16
        buf.extend_from_slice(&[0, 0]);
        // internal_file_attributes u16
        buf.extend_from_slice(&[0, 0]);
        // external_file_attributes u32
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // local_header_offset u32
        buf.extend_from_slice(&[0, 0, 0, 0]);
        // file name: a single non-UTF-8 byte
        buf.push(0xFF);
        assert_eq!(buf.len(), 46 + 1);

        let mut reader = SpanReader::new(&buf);
        let result = read_entry(&mut reader, /*is_zip64=*/ false);
        assert!(
            matches!(result, Err(P4kError::Parse(_))),
            "expected Parse error for non-UTF-8 name, got: {:?}",
            result
        );
    }
}
