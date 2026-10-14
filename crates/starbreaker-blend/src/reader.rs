//! A minimal `.blend` reader for files with this crate's layouts (Blender 5.1
//! DNA): blocks indexed by old pointer, ID lookup by code, and little-endian
//! field helpers. Reads exports back for parity tests and diagnostics; it
//! does not interpret DNA, callers use the offsets the writers document.

use std::collections::HashMap;

/// One block: header fields and its payload.
#[derive(Debug, Clone, Copy)]
pub struct Block<'a> {
    pub code: [u8; 4],
    pub sdna: u32,
    pub old_ptr: u64,
    pub count: u32,
    pub data: &'a [u8],
}

impl Block<'_> {
    /// The two-letter ID code (`b"ME"`, `b"OB"`, …).
    pub fn id_code(&self) -> [u8; 2] {
        [self.code[0], self.code[1]]
    }

    /// The ID name without its code (`ID.name` at 40, after 2 code chars).
    pub fn id_name(&self) -> &str {
        cstr(self.data, 42, 256)
    }
}

/// A parsed (decompressed) `.blend` file.
pub struct BlendFile {
    bytes: Vec<u8>,
    /// (code, sdna, old_ptr, count, payload start, payload len)
    heads: Vec<([u8; 4], u32, u64, u32, usize, usize)>,
    by_ptr: HashMap<u64, usize>,
}

impl BlendFile {
    /// Parse `bytes` (zstd-compressed or raw).
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let bytes = crate::decompress_blend_bytes_if_needed(bytes).map_err(|e| e.to_string())?;
        if !bytes.starts_with(b"BLENDER") {
            return Err("not a .blend file".into());
        }
        // `BLENDER17-01v0501`: a 17-byte header with 64-bit block headers.
        let mut off = crate::BLEND_MAGIC.len();
        let mut heads = Vec::new();
        while off + 32 <= bytes.len() {
            let code: [u8; 4] = bytes[off..off + 4].try_into().unwrap();
            let sdna = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let old_ptr = u64::from_le_bytes(bytes[off + 8..off + 16].try_into().unwrap());
            let len = u64::from_le_bytes(bytes[off + 16..off + 24].try_into().unwrap()) as usize;
            let count = u64::from_le_bytes(bytes[off + 24..off + 32].try_into().unwrap()) as u32;
            let start = off + 32;
            if start + len > bytes.len() {
                return Err(format!("block at {off} runs past the end"));
            }
            heads.push((code, sdna, old_ptr, count, start, len));
            off = start + len;
            if &code == b"ENDB" {
                break;
            }
        }
        let by_ptr = heads.iter().enumerate().filter(|(_, h)| h.2 != 0).map(|(i, h)| (h.2, i)).collect();
        Ok(Self { bytes, heads, by_ptr })
    }

    fn at(&self, i: usize) -> Block<'_> {
        let (code, sdna, old_ptr, count, start, len) = self.heads[i];
        Block { code, sdna, old_ptr, count, data: &self.bytes[start..start + len] }
    }

    pub fn blocks(&self) -> impl Iterator<Item = Block<'_>> {
        (0..self.heads.len()).map(|i| self.at(i))
    }

    /// The block a pointer refers to.
    pub fn block(&self, ptr: u64) -> Option<Block<'_>> {
        (ptr != 0).then(|| self.by_ptr.get(&ptr).map(|&i| self.at(i))).flatten()
    }

    /// ID blocks with the two-letter `code`.
    pub fn ids<'a>(&'a self, code: &'a [u8; 2]) -> impl Iterator<Item = Block<'a>> + 'a {
        self.blocks().filter(move |b| &b.id_code() == code && b.code[2] == 0)
    }

    /// The pointers of a pointer-array block (`T **`).
    pub fn ptr_array(&self, ptr: u64) -> Vec<u64> {
        self.block(ptr)
            .map(|b| b.data.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
            .unwrap_or_default()
    }

    /// Walk a `ListBase` (`first` at `off`): each element's `next` is at 0.
    pub fn list(&self, data: &[u8], off: usize) -> Vec<Block<'_>> {
        let mut out = Vec::new();
        let mut next = ptr_at(data, off);
        while let Some(b) = self.block(next) {
            if out.len() > self.heads.len() {
                break;
            }
            next = ptr_at(b.data, 0);
            out.push(b);
        }
        out
    }
}

pub fn ptr_at(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
}

pub fn i32_at(data: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

pub fn i16_at(data: &[u8], off: usize) -> i16 {
    i16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}

pub fn f32_at(data: &[u8], off: usize) -> f32 {
    f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

/// A NUL-terminated string in `data[off..off + max]`.
pub fn cstr(data: &[u8], off: usize, max: usize) -> &str {
    let raw = &data[off..(off + max).min(data.len())];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    std::str::from_utf8(&raw[..end]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BLEND_MAGIC, write_block, write_block_header, write_id_name, write_ptr};

    fn sample() -> Vec<u8> {
        let mut out = BLEND_MAGIC.to_vec();
        let mut me = vec![0u8; 416];
        write_id_name(&mut me, "ME", "Head");
        write_ptr(&mut me, 408, 0x20);
        write_block(&mut out, b"ME\0\0", 322, 0x10, 1, &me);
        write_block(&mut out, b"DATA", 0, 0x20, 1, &[0x30u64, 0x40].iter().flat_map(|p| p.to_le_bytes()).collect::<Vec<_>>());
        let mut a = vec![0u8; 16];
        write_ptr(&mut a, 0, 0x40);
        write_block(&mut out, b"DATA", 0, 0x30, 1, &a);
        write_block(&mut out, b"DATA", 0, 0x40, 1, &[0u8; 16]);
        write_block_header(&mut out, b"ENDB", 0, 0, 0, 0);
        out
    }

    #[test]
    fn reads_blocks_ids_and_pointers() {
        let file = BlendFile::parse(&sample()).unwrap();
        let mesh = file.ids(b"ME").next().expect("mesh");
        assert_eq!(mesh.id_name(), "Head");
        assert_eq!(file.ptr_array(ptr_at(mesh.data, 408)), vec![0x30, 0x40]);
        let chain = file.list(file.block(0x20).unwrap().data, 0);
        assert_eq!(chain.iter().map(|b| b.old_ptr).collect::<Vec<_>>(), vec![0x30, 0x40]);
    }

    #[test]
    fn reads_compressed_files_and_rejects_others() {
        let compressed = crate::compress_blend_bytes(&sample());
        assert_eq!(BlendFile::parse(&compressed).unwrap().ids(b"ME").count(), 1);
        assert!(BlendFile::parse(b"not a blend").is_err());
    }
}
