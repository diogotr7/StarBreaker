//! Utilities for inspecting Blender 5.x `.blend` files at the binary level.
//!
//! Only Blender 5.x format is supported: `BLENDER17-01v0501` magic (17-byte header),
//! LargeBHead8 block headers, little-endian, 8-byte pointers, zstd compression.
//!
//! Provides:
//! - `.blend` decompression (zstd or uncompressed)
//! - Block-header parsing (LargeBHead8)
//! - DNA1/SDNA struct-layout extraction
//! - Headless Blender binary discovery

use std::path::PathBuf;

// ─── Decompression ───────────────────────────────────────────────────────────

/// Decompress a Blender 5.x `.blend` file's raw bytes.
///
/// Handles:
/// - Zstd-compressed (magic `0x28 0xB5 0x2F 0xFD`): standard format used by Blender 5.x
///   and our Rust writer.
/// - Uncompressed (magic `BLENDER`): for test files that skip compression.
///
/// Returns the decompressed bytes, or an error string.
pub fn decompress_blend(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 7 {
        return Err("file too small to be a .blend".into());
    }

    // Standard zstd magic: 0xFD2FB528 in little-endian = bytes [0x28, 0xB5, 0x2F, 0xFD]
    if data.len() >= 4 && data[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        let mut decoder = structured_zstd::decoding::StreamingDecoder::new(data);
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut out)
            .map_err(|e| format!("zstd decompression failed: {e}"))?;
        return Ok(out);
    }

    // Uncompressed .blend starts with 'BLENDER'
    if &data[..7] == b"BLENDER" {
        return Ok(data.to_vec());
    }

    Err(format!(
        "unrecognised .blend magic: {:?} — expected zstd (0x28 0xB5 0x2F 0xFD) or BLENDER",
        &data[..data.len().min(8)]
    ))
}

// ─── Block parsing ────────────────────────────────────────────────────────────

/// A parsed Blender file block header.
#[derive(Debug, Clone)]
pub struct BlendBlock {
    /// 4-byte block code, e.g. `DATA`, `IM\0\0`, `DNA1`, `ENDB`.
    pub code: [u8; 4],
    /// SDNA struct index (0 if not applicable).
    pub sdna_index: u32,
    /// Old (file) pointer from the header.
    pub old_ptr: u64,
    /// Byte offset where the block data starts in the decompressed file.
    pub data_offset: usize,
    /// Length of the block data in bytes.
    pub data_len: usize,
    /// Element count.
    pub count: u32,
}

impl BlendBlock {
    /// 4-character code as a lossy UTF-8 string (for display).
    pub fn code_str(&self) -> String {
        String::from_utf8_lossy(&self.code).replace('\0', "\\0")
    }
}

/// Parse all block headers from decompressed Blender 5.x `.blend` data.
///
/// Only supports `BLENDER17-..` format (17-byte file header, little-endian, 8-byte pointers).
///
/// Block header layout — **LargeBHead8** (32 bytes):
/// ```text
/// code[4]    — block type (e.g. b"OB\0\0", b"DNA1", b"ENDB")
/// SDNAnr[4]  — SDNA struct index (u32 LE)
/// old[8]     — original memory pointer (u64 LE)
/// len[8]     — data length in bytes (i64 LE; fits in u32 for all practical blocks)
/// nr[8]      — element count (i64 LE; fits in u32)
/// ```
///
/// Our Rust writer produces the same layout using `u32 + 0u32` padding for len/nr, which
/// is byte-identical to LargeBHead8 when values fit in 32 bits.
///
/// Iterates until `ENDB` or end of data. Returns an error if the header magic is wrong
/// or a block claims more bytes than remain in the buffer.
pub fn parse_blend_blocks(data: &[u8]) -> Result<Vec<BlendBlock>, String> {
    if data.len() < 17 {
        return Err(format!(
            "file too small ({} bytes) for Blender 5.x 17-byte header",
            data.len()
        ));
    }
    if &data[..7] != b"BLENDER" {
        return Err(format!(
            "expected BLENDER magic, got {:?}",
            &data[..7.min(data.len())]
        ));
    }
    if data[7] != b'1' {
        return Err(format!(
            "only Blender 5.x (BLENDER17-...) is supported; got byte[7]={:?}",
            data[7] as char
        ));
    }

    let mut pos = 17usize; // skip 17-byte file header
    let mut blocks = Vec::new();

    loop {
        if pos + 4 > data.len() {
            break;
        }
        let code: [u8; 4] = data[pos..pos + 4].try_into().unwrap();

        if &code == b"ENDB" {
            blocks.push(BlendBlock {
                code,
                sdna_index: 0,
                old_ptr: 0,
                data_offset: pos + 32,
                data_len: 0,
                count: 0,
            });
            break;
        }

        // LargeBHead8: 32-byte header total
        if pos + 32 > data.len() {
            break;
        }

        let sdna_index = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
        let old_ptr    = u64::from_le_bytes(data[pos + 8..pos + 16].try_into().unwrap());
        let data_len   = i64::from_le_bytes(data[pos + 16..pos + 24].try_into().unwrap());
        let count      = i64::from_le_bytes(data[pos + 24..pos + 32].try_into().unwrap());

        if data_len < 0 {
            return Err(format!(
                "block {:?} at offset {pos} has negative length {data_len}",
                String::from_utf8_lossy(&code)
            ));
        }
        let data_len = data_len as usize;
        let data_offset = pos + 32;

        if data_offset + data_len > data.len() {
            return Err(format!(
                "block {:?} at offset {pos} claims {data_len} bytes but only {} remain",
                String::from_utf8_lossy(&code),
                data.len() - data_offset
            ));
        }

        blocks.push(BlendBlock {
            code,
            sdna_index,
            old_ptr,
            data_offset,
            data_len,
            count: count as u32,
        });

        pos = data_offset + data_len;
    }

    Ok(blocks)
}

// ─── SDNA / DNA1 parsing ─────────────────────────────────────────────────────

/// A field within an SDNA struct.
#[derive(Debug, Clone)]
pub struct SdnaField {
    pub name: String,
    pub type_name: String,
    /// Size of this field in bytes.
    pub size: usize,
    /// Byte offset of this field from the start of its containing struct.
    pub offset: usize,
    /// SDNA struct index for the field type, if it is itself a struct.
    pub struct_index: Option<usize>,
    /// True if this field is a pointer (name starts with `*` or `**`).
    pub is_pointer: bool,
    /// Array dimensions extracted from the field name, e.g. `[3]` or `[3][3]`.
    pub array_dims: Vec<usize>,
}

/// An SDNA struct definition.
#[derive(Debug, Clone)]
pub struct SdnaStruct {
    pub name: String,
    pub size: usize,
    pub fields: Vec<SdnaField>,
}

/// The SDNA index extracted from a `.blend` file's `DNA1` block.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SdnaIndex {
    pub structs: Vec<SdnaStruct>,
    /// Type names indexed by type index (parallel to the `type_sizes` table).
    pub type_names: Vec<String>,
    /// Type sizes indexed by type index.
    pub type_sizes: Vec<usize>,
    pub ptr_size: usize,
}

impl SdnaIndex {
    /// Find a struct by name (case-sensitive), returning its index and definition.
    pub fn find_struct(&self, name: &str) -> Option<(usize, &SdnaStruct)> {
        self.structs
            .iter()
            .enumerate()
            .find(|(_, s)| s.name == name)
    }
}

/// Parse the SDNA index from a `DNA1` block.
///
/// `block_data` is the raw bytes of the `DNA1` block (after the block header).
/// For Blender 5.x, `ptr_size` is always `8`; pass `8` directly.
pub fn parse_sdna(block_data: &[u8], ptr_size: usize) -> Result<SdnaIndex, String> {
    let mut pos = 0usize;

    macro_rules! need {
        ($n:expr) => {
            if pos + $n > block_data.len() {
                return Err(format!(
                    "SDNA truncated at pos={pos} needing {}, len={}",
                    $n,
                    block_data.len()
                ));
            }
        };
    }
    macro_rules! read4 {
        () => {{
            need!(4);
            let v = u32::from_le_bytes(block_data[pos..pos + 4].try_into().unwrap());
            pos += 4;
            v as usize
        }};
    }
    macro_rules! read2 {
        () => {{
            need!(2);
            let v = u16::from_le_bytes(block_data[pos..pos + 2].try_into().unwrap());
            pos += 2;
            v as usize
        }};
    }

    // "SDNA"
    need!(4);
    if &block_data[pos..pos + 4] != b"SDNA" {
        return Err(format!("expected SDNA identifier, got {:?}", &block_data[pos..pos + 4]));
    }
    pos += 4;

    // "NAME"
    need!(4);
    if &block_data[pos..pos + 4] != b"NAME" {
        return Err("expected NAME".into());
    }
    pos += 4;
    let name_count = read4!();
    let mut field_names = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        let end = block_data[pos..].iter().position(|&b| b == 0).unwrap_or(block_data.len() - pos);
        field_names.push(String::from_utf8_lossy(&block_data[pos..pos + end]).to_string());
        pos += end + 1;
    }
    // Align to 4
    while pos % 4 != 0 { pos += 1; }

    // "TYPE"
    need!(4);
    if &block_data[pos..pos + 4] != b"TYPE" {
        return Err("expected TYPE".into());
    }
    pos += 4;
    let type_count = read4!();
    let mut type_names = Vec::with_capacity(type_count);
    for _ in 0..type_count {
        let end = block_data[pos..].iter().position(|&b| b == 0).unwrap_or(block_data.len() - pos);
        type_names.push(String::from_utf8_lossy(&block_data[pos..pos + end]).to_string());
        pos += end + 1;
    }
    while pos % 4 != 0 { pos += 1; }

    // "TLEN"
    need!(4);
    if &block_data[pos..pos + 4] != b"TLEN" {
        return Err("expected TLEN".into());
    }
    pos += 4;
    let mut type_sizes = Vec::with_capacity(type_count);
    for _ in 0..type_count {
        type_sizes.push(read2!());
    }
    while pos % 4 != 0 { pos += 1; }

    // "STRC"
    need!(4);
    if &block_data[pos..pos + 4] != b"STRC" {
        return Err("expected STRC".into());
    }
    pos += 4;
    let struct_count = read4!();

    // First pass: collect struct names and type indices so we can resolve field struct indices.
    let mut struct_type_indices: Vec<usize> = Vec::with_capacity(struct_count);
    let strc_start = pos;
    for _ in 0..struct_count {
        need!(4);
        let type_idx = read2!();
        let field_count = read2!();
        struct_type_indices.push(type_idx);
        pos += field_count * 4; // 2 bytes type_idx + 2 bytes name_idx per field
    }

    // Build a map: type_name → struct_index
    let type_to_struct: std::collections::HashMap<String, usize> = struct_type_indices
        .iter()
        .enumerate()
        .map(|(si, &ti)| (type_names[ti].clone(), si))
        .collect();

    // Second pass: build full struct definitions.
    pos = strc_start;
    let mut structs = Vec::with_capacity(struct_count);

    for _ in 0..struct_count {
        need!(4);
        let type_idx = read2!();
        let field_count = read2!();
        let struct_name = type_names[type_idx].clone();
        let struct_total_size = type_sizes[type_idx];

        let mut fields = Vec::with_capacity(field_count);
        let mut field_offset = 0usize;

        for _ in 0..field_count {
            need!(4);
            let f_type_idx = read2!();
            let f_name_idx = read2!();
            let raw_name = &field_names[f_name_idx];

            // Parse the raw name: may be `*name`, `**name`, `name[3]`, etc.
            let (is_pointer, base_name, array_dims) = parse_field_name(raw_name);

            let element_size = if is_pointer {
                ptr_size
            } else {
                type_sizes[f_type_idx]
            };

            let total_elements: usize = if array_dims.is_empty() {
                1
            } else {
                array_dims.iter().product()
            };

            let field_size = element_size * total_elements;
            let struct_index = if !is_pointer {
                type_to_struct.get(&type_names[f_type_idx]).copied()
            } else {
                None
            };

            fields.push(SdnaField {
                name: base_name,
                type_name: type_names[f_type_idx].clone(),
                size: field_size,
                offset: field_offset,
                struct_index,
                is_pointer,
                array_dims,
            });
            field_offset += field_size;
        }

        structs.push(SdnaStruct {
            name: struct_name,
            size: struct_total_size,
            fields,
        });
    }

    Ok(SdnaIndex { structs, type_names, type_sizes, ptr_size })
}

/// Parse a raw SDNA field name like `*name`, `**name`, `name[3][4]`, `(*func)()`.
/// Returns `(is_pointer, cleaned_name, array_dims)`.
fn parse_field_name(raw: &str) -> (bool, String, Vec<usize>) {
    let is_pointer = raw.contains('*') || raw.starts_with('(');
    // Strip pointer decorators
    let stripped = raw.trim_start_matches('(')
        .trim_start_matches('*')
        .trim_start_matches(')');
    // Extract array dims
    let mut array_dims = Vec::new();
    let mut base = stripped.to_string();
    while let Some(open) = base.rfind('[') {
        if let Some(close) = base[open..].find(']') {
            let dim_str = &base[open + 1..open + close];
            if let Ok(d) = dim_str.parse::<usize>() {
                array_dims.insert(0, d);
            }
            base.truncate(open);
        } else {
            break;
        }
    }
    // Strip trailing function-pointer garbage like `)()`
    let base = base.trim_end_matches(')').trim_end_matches('(').trim_end_matches(')');
    (is_pointer, base.to_string(), array_dims)
}

// ─── Blender binary discovery ─────────────────────────────────────────────────

/// Find the headless Blender binary.
///
/// Priority:
/// 1. `blender_bin` parameter (if provided and non-empty)
/// 2. `BLENDER_BIN` environment variable
/// 3. `blender` on `PATH` (cross-platform: checks `blender.exe` on Windows)
/// 4. Platform-specific well-known paths
pub fn find_blender_bin(blender_bin: Option<&str>) -> Result<PathBuf, String> {
    // 1. Explicit per-call override
    if let Some(p) = blender_bin.filter(|s| !s.is_empty()) {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(format!("blender_bin={p:?} is not a file"));
    }

    // 2. Environment variable
    if let Ok(env_path) = std::env::var("BLENDER_BIN") {
        if !env_path.is_empty() {
            let pb = PathBuf::from(&env_path);
            if pb.is_file() {
                return Ok(pb);
            }
            return Err(format!("BLENDER_BIN={env_path:?} is not a file"));
        }
    }

    // 3. PATH lookup
    let exe = if cfg!(windows) { "blender.exe" } else { "blender" };
    if let Ok(found) = which_blender(exe) {
        return Ok(found);
    }

    // 4. Platform-specific fallbacks
    #[cfg(target_os = "macos")]
    {
        let mac = PathBuf::from("/Applications/Blender.app/Contents/MacOS/blender");
        if mac.is_file() { return Ok(mac); }
    }

    #[cfg(target_os = "windows")]
    {
        for root in &[r"C:\Program Files\Blender Foundation", r"C:\Program Files (x86)\Blender Foundation"] {
            if let Ok(rd) = std::fs::read_dir(root) {
                let mut versions: Vec<_> = rd
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .collect();
                versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
                for v in versions {
                    let candidate = v.path().join("blender.exe");
                    if candidate.is_file() { return Ok(candidate); }
                }
            }
        }
    }

    Err("Blender binary not found. Set BLENDER_BIN environment variable or pass blender_bin parameter.".into())
}

fn which_blender(exe: &str) -> Result<PathBuf, String> {
    if let Ok(path_var) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path_var.split(sep) {
            let candidate = PathBuf::from(dir).join(exe);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!("{exe} not found on PATH"))
}

// ─── Hex formatting ───────────────────────────────────────────────────────────

/// Format bytes as a hex dump with offset, hex, and ASCII columns.
pub fn hex_dump(data: &[u8], base_offset: usize) -> String {
    let mut out = String::new();
    for (chunk_idx, chunk) in data.chunks(16).enumerate() {
        let offset = base_offset + chunk_idx * 16;
        out.push_str(&format!("{offset:08x}  "));
        for (i, b) in chunk.iter().enumerate() {
            if i == 8 { out.push(' '); }
            out.push_str(&format!("{b:02x} "));
        }
        // Pad incomplete lines
        let missing = 16 - chunk.len();
        out.push_str(&"   ".repeat(missing));
        if missing >= 8 { out.push(' '); }
        out.push_str(" |");
        for &b in chunk {
            out.push(if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' });
        }
        out.push_str("|\n");
    }
    out
}

// ─── Struct layout formatting ─────────────────────────────────────────────────

/// Format a struct's field layout recursively.
///
/// `base_offset` is the cumulative byte offset from the root struct start.
/// `indent` controls nesting depth.
/// `max_depth` limits recursion (0 = top-level fields only).
pub fn format_struct_layout(
    sdna: &SdnaIndex,
    s: &SdnaStruct,
    base_offset: usize,
    indent: usize,
    max_depth: usize,
    out: &mut String,
) {
    let pad = "  ".repeat(indent);
    for field in &s.fields {
        let abs = base_offset + field.offset;
        let ptr_marker = if field.is_pointer { "*" } else { "" };
        let arr_str: String = field.array_dims.iter().map(|d| format!("[{d}]")).collect();
        out.push_str(&format!(
            "{pad}+{:04} ({abs:04})  {ptr_marker}{}{arr_str}  {}  {}b\n",
            field.offset, field.name, field.type_name, field.size
        ));
        // Recurse into nested structs
        if max_depth > 0 && !field.is_pointer && field.array_dims.is_empty() {
            if let Some(si) = field.struct_index {
                format_struct_layout(sdna, &sdna.structs[si], abs, indent + 1, max_depth - 1, out);
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_field_name_pointer() {
        let (is_ptr, name, dims) = parse_field_name("*next");
        assert!(is_ptr);
        assert_eq!(name, "next");
        assert!(dims.is_empty());
    }

    #[test]
    fn parse_field_name_array() {
        let (is_ptr, name, dims) = parse_field_name("data[3][4]");
        assert!(!is_ptr);
        assert_eq!(name, "data");
        assert_eq!(dims, vec![3, 4]);
    }

    #[test]
    fn parse_field_name_double_pointer() {
        let (is_ptr, name, dims) = parse_field_name("**mat");
        assert!(is_ptr);
        assert_eq!(name, "mat");
        assert!(dims.is_empty());
    }

    #[test]
    fn hex_dump_output() {
        let data = b"HelloWorld";
        let out = hex_dump(data, 0);
        assert!(out.contains("48 65 6c 6c 6f"));
        assert!(out.contains("HelloWorld"));
    }
}
