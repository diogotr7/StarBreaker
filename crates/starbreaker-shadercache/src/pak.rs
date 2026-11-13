//! Lazy access to the shipped Vulkan shader cache.
//!
//! Nothing is extracted up front. The central directory of
//! `Engine\ShaderCache_Vulkan.pak` — a plain ZIP that `Data.p4k` stores
//! uncompressed — is parsed once with [`starbreaker_p4k::parse_entries_in_range`],
//! and individual permutations are read positionally, from the p4k itself, only
//! when asked for.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use starbreaker_p4k::{MappedP4k, P4kArchive, P4kEntry, parse_entries_in_range};

use crate::error::ShaderCacheError;
use crate::name::{self, Permutation, Stage};
use crate::reflect::{Reflection, ResourceClass};
use crate::shcb::{self, Shcb};

/// The Vulkan shader cache inside `Data.p4k`.
pub const VULKAN_PAK: &str = r"Engine\ShaderCache_Vulkan.pak";

/// Shader template property definitions inside `Data.p4k`.
pub const EXT_PAK: &str = r"Engine\ShaderCache_EXT.pak";

/// Read every entry of a secondary pak stored inside `Data.p4k`.
///
/// Used for the `.ext` template/property files, which carry the flag→bit table
/// for the shader generation mask.
pub fn read_pak_entries(
    p4k: impl AsRef<Path>,
    entry_name: &str,
) -> Result<Vec<(String, Vec<u8>)>, ShaderCacheError> {
    let p4k = MappedP4k::open(p4k)?;
    let entry = p4k
        .entry_case_insensitive(entry_name)
        .or_else(|| p4k.entry_case_insensitive(&entry_name.replace('\\', "/")))
        .ok_or_else(|| ShaderCacheError::EntryNotFound(entry_name.to_owned()))?;

    let mut file = OpenOptions::new().read(true).open(p4k.path())?;
    let mut header = [0u8; 30];
    file.seek(SeekFrom::Start(entry.offset))?;
    file.read_exact(&mut header)?;
    let name_len = u16::from_le_bytes([header[26], header[27]]) as u64;
    let extra_len = u16::from_le_bytes([header[28], header[29]]) as u64;
    let base = entry.offset + 30 + name_len + extra_len;

    let mut reader = file.try_clone()?;
    let entries = parse_entries_in_range(&mut reader, base, entry.compressed_size, None)?;

    let mut out = Vec::with_capacity(entries.len());
    for e in &entries {
        if !e.name.ends_with('/') {
            out.push((e.name.clone(), P4kArchive::read_from_file_at(&file, e)?));
        }
    }
    Ok(out)
}

/// A shader cache archive with its permutations indexed.
pub struct ShaderPak {
    api: String,
    cfxib: HashMap<String, usize>,
    file: File,
    entries: Vec<P4kEntry>,
    perms: Vec<Permutation>,
    /// exact entry name -> entry index
    by_path: HashMap<String, usize>,
}

impl ShaderPak {
    fn build(file: File, base: u64, size: u64) -> Result<Self, ShaderCacheError> {
        let mut reader = file.try_clone()?;
        let entries = parse_entries_in_range(&mut reader, base, size, None)?;

        let mut perms = Vec::new();
        let mut by_path = HashMap::new();
        let mut cfxib = HashMap::new();
        let mut api = String::new();
        for (i, entry) in entries.iter().enumerate() {
            let name = entry.name.replace('\\', "/");
            if let Some(rest) = name.strip_prefix("Shaders/CFXIB/") {
                if let Some((api_dir, file)) = rest.split_once('/') {
                    if api.is_empty() {
                        api = api_dir.to_owned();
                    }
                    if let Some((stem, _ext)) = file.rsplit_once('.') {
                        cfxib.entry(stem.to_owned()).or_insert(i);
                    }
                }
                continue;
            }
            if let Some(p) = name::parse(&entry.name) {
                by_path.insert(p.path.clone(), i);
                perms.push(p);
            }
        }
        Ok(Self {
            api,
            cfxib,
            file,
            entries,
            perms,
            by_path,
        })
    }

    /// Shader API directory the cache was built for (`Vulkan`).
    pub fn api(&self) -> &str {
        &self.api
    }

    /// Names of the extracted shader-bin descriptors (`.cfxb` / `.cfib` stems).
    pub fn shader_bins(&self) -> impl Iterator<Item = &str> {
        self.cfxib.keys().map(String::as_str)
    }

    /// The extension (`cfxb` effect / `cfib` include) of shader-bin `stem`.
    /// Stems are case-sensitive: `Common` (effect) and `common` (include)
    /// are different files.
    pub fn shader_bin_ext(&self, stem: &str) -> Option<&str> {
        let idx = *self.cfxib.get(stem)?;
        self.entries[idx].name.rsplit_once('.').map(|(_, ext)| ext)
    }

    /// Read a `.cfxb` / `.cfib` descriptor by stem (e.g. `Decal`, `common`).
    pub fn read_shader_bin(&self, stem: &str) -> Result<Vec<u8>, ShaderCacheError> {
        let idx = *self
            .cfxib
            .get(stem)
            .ok_or_else(|| ShaderCacheError::EntryNotFound(stem.to_owned()))?;
        Ok(P4kArchive::read_from_file_at(&self.file, &self.entries[idx])?)
    }

    /// Open a `.pak` that has already been extracted to disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ShaderCacheError> {
        let file = OpenOptions::new().read(true).open(path)?;
        let size = file.metadata()?.len();
        Self::build(file, 0, size)
    }

    /// Open the Vulkan shader cache in place, without extracting it.
    pub fn open_in_p4k(path: impl AsRef<Path>) -> Result<Self, ShaderCacheError> {
        let p4k = MappedP4k::open(path)?;
        let entry = p4k
            .entry_case_insensitive(VULKAN_PAK)
            .or_else(|| p4k.entry_case_insensitive(&VULKAN_PAK.replace('\\', "/")))
            .ok_or_else(|| ShaderCacheError::EntryNotFound(VULKAN_PAK.to_owned()))?;

        // The pak is stored uncompressed, so its ZIP starts right after the
        // entry's local header.
        let mut file = OpenOptions::new().read(true).open(p4k.path())?;
        let mut header = [0u8; 30];
        file.seek(SeekFrom::Start(entry.offset))?;
        file.read_exact(&mut header)?;
        let name_len = u16::from_le_bytes([header[26], header[27]]) as u64;
        let extra_len = u16::from_le_bytes([header[28], header[29]]) as u64;
        let base = entry.offset + 30 + name_len + extra_len;
        Self::build(file, base, entry.compressed_size)
    }

    /// Every indexed permutation.
    pub fn permutations(&self) -> &[Permutation] {
        &self.perms
    }

    pub fn len(&self) -> usize {
        self.perms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.perms.is_empty()
    }

    /// Every permutation of one shader entry point (all mask variants).
    pub fn variants<'a>(
        &'a self,
        shader: &'a str,
        entry: &'a str,
    ) -> impl Iterator<Item = &'a Permutation> + 'a {
        self.perms
            .iter()
            .filter(move |p| p.shader == shader && p.entry == entry)
    }

    /// All permutations of one shader family.
    pub fn family<'a>(&'a self, shader: &'a str) -> impl Iterator<Item = &'a Permutation> + 'a {
        self.perms.iter().filter(move |p| p.shader == shader)
    }

    /// Read the raw `SHCB` block for exactly this permutation.
    pub fn read_block(&self, perm: &Permutation) -> Result<Vec<u8>, ShaderCacheError> {
        let idx = *self
            .by_path
            .get(&perm.path)
            .ok_or_else(|| ShaderCacheError::EntryNotFound(perm.path.clone()))?;
        Ok(P4kArchive::read_from_file_at(&self.file, &self.entries[idx])?)
    }

    /// Read a permutation's SPIR-V module (validated).
    pub fn read_spirv(&self, perm: &Permutation) -> Result<Vec<u8>, ShaderCacheError> {
        let block = self.read_block(perm)?;
        let Shcb { payload, .. } = shcb::spirv_payload(&block)?;
        Ok(payload.to_vec())
    }

    /// Read and reflect one permutation.
    pub fn reflect(
        &self,
        perm: &Permutation,
    ) -> Result<crate::reflect::Reflection, ShaderCacheError> {
        let spirv = self.read_spirv(perm)?;
        crate::reflect::reflect(&spirv)
    }

    /// Pick the permutation of one stage that best matches a wanted mask.
    ///
    /// 1. **Mask fit:** fewest wanted bits missing from the permutation's `GL`
    ///    mask, then fewest extra bits.
    /// 2. **Among equally fitting entry points**, the one that reads the most
    ///    material state (per-material uniform members + per-material
    ///    textures), then the most bindings overall. A family compiles several
    ///    passes under one mask — main, depth/G-buffer, shadow, shared helpers
    ///    — and the main material pass reads the most; helpers read none. This
    ///    reflects one variant per tied entry point.
    /// 3. Entry name, only so the result is deterministic.
    ///
    /// Use it for the stage that anchors a pass (pixel, compute); pick the
    /// stages feeding it with [`Self::select_linked`]. Returns the permutation
    /// with its number of missing and extra bits.
    pub fn select_stage(
        &self,
        family: &str,
        stage: Stage,
        want: u64,
    ) -> Result<Option<(&Permutation, u32, u32)>, ShaderCacheError> {
        self.select(family, stage, want, None)
    }

    /// Pick the permutation of `stage` that feeds `consumer` — the next stage
    /// of the same pass, e.g. the vertex shader for a chosen pixel shader.
    ///
    /// Only candidates whose located outputs supply every located input of
    /// `consumer` (same location and type) qualify; a pipeline cannot link
    /// otherwise. Among those, the ranking of [`Self::select_stage`] applies,
    /// falling back to looser mask fits until one links. `None` when no
    /// permutation of `stage` links.
    pub fn select_linked(
        &self,
        family: &str,
        stage: Stage,
        want: u64,
        consumer: &Permutation,
    ) -> Result<Option<(&Permutation, u32, u32)>, ShaderCacheError> {
        let consumer = self.reflect(consumer)?;
        self.select(family, stage, want, Some(&consumer))
    }

    fn select(
        &self,
        family: &str,
        stage: Stage,
        want: u64,
        consumer: Option<&Reflection>,
    ) -> Result<Option<(&Permutation, u32, u32)>, ShaderCacheError> {
        // Fit level -> one variant per entry point (variants sharing an entry
        // and mask differ only in runtime masks).
        let mut levels: BTreeMap<(u32, u32), Vec<&Permutation>> = BTreeMap::new();
        let mut seen = HashSet::new();
        for perm in self
            .perms
            .iter()
            .filter(|p| p.shader == family && p.stage == stage)
        {
            let gl = perm.masks.gl.unwrap_or(0);
            if seen.insert((perm.entry.as_str(), gl)) {
                let fit = ((want & !gl).count_ones(), (gl & !want).count_ones());
                levels.entry(fit).or_default().push(perm);
            }
        }

        for (fit, candidates) in levels {
            let mut entries = HashSet::new();
            let mut best: Option<(&Permutation, (usize, usize))> = None;
            for perm in candidates {
                if !entries.insert(perm.entry.as_str()) {
                    continue;
                }
                let r = self.reflect(perm)?;
                if consumer.is_some_and(|c| !r.feeds(c)) {
                    continue;
                }
                let material = r
                    .uniform_blocks(ResourceClass::PerMaterial)
                    .map(|b| b.members.len())
                    .sum::<usize>()
                    + r.textures(ResourceClass::PerMaterial).count();
                let score = (material, r.bindings.len());
                let better = best.is_none_or(|(cur, cur_score)| {
                    score > cur_score || (score == cur_score && perm.entry < cur.entry)
                });
                if better {
                    best = Some((perm, score));
                }
            }
            if let Some((perm, _)) = best {
                return Ok(Some((perm, fit.0, fit.1)));
            }
        }
        Ok(None)
    }

    /// Pick the pixel permutation that best matches a wanted generation mask.
    pub fn select_pixel(
        &self,
        family: &str,
        want: u64,
    ) -> Result<Option<(&Permutation, u32, u32)>, ShaderCacheError> {
        self.select_stage(family, Stage::Pixel, want)
    }
}


