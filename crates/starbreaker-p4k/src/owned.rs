use rustc_hash::FxHashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::archive::{DirEntry, P4kArchive, P4kEntry, parse_central_directory};
use crate::error::P4kError;

/// A P4k archive backed by a memory-mapped file.
///
/// Since `P4kEntry` fields are all owned types (`String`, `u64`, etc.),
/// the entries are parsed once during construction and stored separately
/// from the mmap.
pub struct MappedP4k {
    path: PathBuf,
    mmap: Mmap,
    entries: Vec<P4kEntry>,
    path_index: FxHashMap<String, usize>,
    lowercase_index: FxHashMap<String, usize>,
    sorted_index: Vec<u32>,
}

impl MappedP4k {
    /// Open a P4k file by memory-mapping it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, P4kError> {
        Self::open_with_progress(path, None)
    }

    /// Open a P4k file with progress reporting.
    pub fn open_with_progress(
        path: impl AsRef<Path>,
        progress: Option<&starbreaker_common::Progress>,
    ) -> Result<Self, P4kError> {
        let path_buf = path.as_ref().to_path_buf();
        let file = File::open(&path_buf)?;
        let mmap = unsafe { Mmap::map(&file)? };

        let (entries, path_index, lowercase_index, sorted_index) =
            parse_central_directory(&mmap, progress)?;

        Ok(MappedP4k {
            path: path_buf,
            mmap,
            entries,
            path_index,
            lowercase_index,
            sorted_index,
        })
    }

    /// Get the filesystem path to the underlying P4k file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read and decompress an entry's data.
    pub fn read(&self, entry: &P4kEntry) -> Result<Vec<u8>, P4kError> {
        P4kArchive::read_from_data(&self.mmap, entry)
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

    /// Look up and read a file by path (case-insensitive). Returns the decompressed data.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, P4kError> {
        let entry = self
            .entry_case_insensitive(path)
            .ok_or_else(|| P4kError::EntryNotFound(path.to_string()))?;
        self.read(entry)
    }

    /// List immediate children (files and subdirectories) under a directory path.
    ///
    /// `dir_path` should NOT have a trailing backslash (e.g., `"Data\\Objects"`).
    pub fn list_dir(&self, dir_path: &str) -> Vec<DirEntry<'_>> {
        let prefix = if dir_path.is_empty() {
            String::new()
        } else {
            format!("{dir_path}\\")
        };

        let start = self
            .sorted_index
            .partition_point(|&idx| self.entries[idx as usize].name.as_str() < prefix.as_str());

        let mut result = Vec::new();
        let mut seen_dirs = rustc_hash::FxHashSet::default();

        for &idx in &self.sorted_index[start..] {
            let name = &self.entries[idx as usize].name;
            if !name.starts_with(&prefix) {
                break;
            }
            let rest = &name[prefix.len()..];
            if let Some(slash_pos) = rest.find('\\') {
                let subdir = &rest[..slash_pos];
                if seen_dirs.insert(subdir.to_string()) {
                    result.push(DirEntry::Directory(subdir.to_string()));
                }
            } else {
                result.push(DirEntry::File(&self.entries[idx as usize]));
            }
        }

        result
    }

    /// List only immediate subdirectory names (fast — skips over file entries).
    pub fn list_subdirs(&self, dir_path: &str) -> Vec<String> {
        let prefix = if dir_path.is_empty() {
            String::new()
        } else {
            format!("{dir_path}\\")
        };

        let sorted = &self.sorted_index;
        let entries = &self.entries;

        let mut pos = sorted
            .partition_point(|&idx| entries[idx as usize].name.as_str() < prefix.as_str());

        let mut result = Vec::new();

        while pos < sorted.len() {
            let name = &entries[sorted[pos] as usize].name;
            if !name.starts_with(&prefix) {
                break;
            }
            let rest = &name[prefix.len()..];
            if let Some(slash_pos) = rest.find('\\') {
                let subdir = &rest[..slash_pos];
                result.push(subdir.to_string());

                // Skip ahead past all entries with this subdir prefix
                let skip_prefix = format!("{prefix}{subdir}\\");
                // Find the next character after the subdir range.
                // Incrementing the last char gives us the exclusive upper bound.
                let mut skip_end = skip_prefix.clone();
                // Replace trailing backslash with one-higher char to get past the range
                skip_end.pop();
                skip_end.push(']'); // ']' is one past '\\' in ASCII
                pos = sorted.partition_point(|&idx| {
                    entries[idx as usize].name.as_str() < skip_end.as_str()
                });
            } else {
                // It's a file — just skip it
                pos += 1;
            }
        }

        result
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
