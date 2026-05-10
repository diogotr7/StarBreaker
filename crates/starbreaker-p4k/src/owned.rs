use hashbrown::HashTable;
use parking_lot::Mutex;
use std::fs::File;
use std::path::{Path, PathBuf};

use crate::archive::{DirEntry, P4kArchive, P4kEntry, cmp_lower_against, hash_path, parse_central_directory_from_file};
use crate::error::P4kError;

/// A P4k archive backed by a pool of file handles.
///
/// Entries are parsed once at construction. Individual reads use positional
/// I/O (`pread`/`seek_read`) on a handle popped from the pool — each rayon
/// worker effectively gets its own handle once the pool stabilises, which
/// avoids the per-file-object lock that Windows takes inside `ReadFile`
/// when many threads share a single handle. The mutex covers only the
/// pop/push around each read; the read itself runs without the lock.
pub struct MappedP4k {
    path: PathBuf,
    file_pool: Mutex<Vec<File>>,
    entries: Vec<P4kEntry>,
    path_index: HashTable<u32>,
    sorted_index: Vec<u32>,
    lowercase_names: Vec<String>,
    sorted_lower_index: Vec<u32>,
}

impl MappedP4k {
    /// Open a P4k file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, P4kError> {
        Self::open_with_progress(path, None)
    }

    /// Open a P4k file with progress reporting.
    pub fn open_with_progress(
        path: impl AsRef<Path>,
        progress: Option<&starbreaker_common::Progress>,
    ) -> Result<Self, P4kError> {
        let path_buf = path.as_ref().to_path_buf();
        let mut file = File::open(&path_buf)?;

        let (entries, path_index, sorted_index, lowercase_names, sorted_lower_index) =
            parse_central_directory_from_file(&mut file, progress)?;

        Ok(MappedP4k {
            path: path_buf,
            file_pool: Mutex::new(vec![file]),
            entries,
            path_index,
            sorted_index,
            lowercase_names,
            sorted_lower_index,
        })
    }

    /// Get the filesystem path to the underlying P4k file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read and decompress an entry's data.
    ///
    /// Pops a `File` handle from the pool (or opens a new one if empty),
    /// reads positionally, then returns the handle. The pop/push is the
    /// only contended path; the read itself runs unlocked. Each concurrent
    /// caller ends up with its own handle once the pool stabilises.
    pub fn read(&self, entry: &P4kEntry) -> Result<Vec<u8>, P4kError> {
        let file = self
            .file_pool
            .lock()
            .pop()
            .map(Ok)
            .unwrap_or_else(|| File::open(&self.path))?;

        let result = P4kArchive::read_from_file_at(&file, entry);

        self.file_pool.lock().push(file);

        result
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
    /// Allocates nothing per call.
    pub fn entry_case_insensitive(&self, path: &str) -> Option<&P4kEntry> {
        use std::cmp::Ordering;
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
