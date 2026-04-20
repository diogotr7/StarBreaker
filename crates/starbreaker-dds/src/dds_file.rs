use std::path::Path;

use starbreaker_common::SpanReader;

use crate::decode::{BlockFormat, decode_block_compressed};
use crate::error::DdsError;
use crate::sibling::ReadSibling;
use crate::types::{DDS_MAGIC, DdsHeader, DdsHeaderDxt10, DdsPixelFormat, DxgiFormat};

/// A parsed DDS texture with separated mip levels.
pub struct DdsFile {
    pub header: DdsHeader,
    pub dxt10_header: Option<DdsHeaderDxt10>,
    /// Mip levels, index 0 = largest (full resolution).
    pub mip_data: Vec<Vec<u8>>,
    /// Alpha channel mip levels (empty if none). Same ordering as mip_data.
    pub alpha_mip_data: Vec<Vec<u8>>,
}

impl DdsFile {
    /// Parse only the DDS headers, ignoring pixel data.
    /// Works for any format, even unsupported ones.
    pub fn headers_only(data: &[u8]) -> Result<Self, DdsError> {
        let mut reader = SpanReader::new(data);
        let magic = reader.read_bytes(4)?;
        if magic != DDS_MAGIC {
            return Err(DdsError::InvalidMagic);
        }
        let header = *reader.read_type::<DdsHeader>()?;
        let dxt10_header = if header.pixel_format.four_cc == *b"DX10" {
            Some(*reader.read_type::<DdsHeaderDxt10>()?)
        } else {
            None
        };
        Ok(Self {
            header,
            dxt10_header,
            mip_data: Vec::new(),
            alpha_mip_data: Vec::new(),
        })
    }

    /// Parse a complete (already merged) DDS file from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, DdsError> {
        let mut reader = SpanReader::new(data);

        // Read and validate magic
        let magic = reader.read_bytes(4)?;
        if magic != DDS_MAGIC {
            return Err(DdsError::InvalidMagic);
        }

        // Read main header
        let header = *reader.read_type::<DdsHeader>()?;

        // Read optional DX10 header
        let dxt10_header = if header.pixel_format.four_cc == *b"DX10" {
            Some(*reader.read_type::<DdsHeaderDxt10>()?)
        } else {
            None
        };

        let mip_count = std::cmp::max(1, header.mipmap_count) as usize;

        // Determine number of faces (cubemaps have 6 faces)
        let faces = face_count(&header, dxt10_header.as_ref());

        // Calculate sizes for each mip level (per face)
        let mip_sizes: Vec<usize> = (0..mip_count)
            .map(|level| {
                compute_mip_size(&header.pixel_format, dxt10_header.as_ref(), header.width, header.height, level)
            })
            .collect();

        let remaining = reader.read_bytes(reader.remaining())?;

        let mut mip_data = Vec::with_capacity(mip_count);
        let mut offset = 0;

        if faces == 1 {
            for &size in &mip_sizes {
                if offset + size > remaining.len() {
                    break;
                }
                mip_data.push(remaining[offset..offset + size].to_vec());
                offset += size;
            }
        } else {
            // Cubemap: store per-mip with all faces concatenated.
            for &size in &mip_sizes {
                let total = size * faces;
                if offset + total > remaining.len() {
                    break;
                }
                mip_data.push(remaining[offset..offset + total].to_vec());
                offset += total;
            }
        }

        Ok(Self {
            header,
            dxt10_header,
            mip_data,
            alpha_mip_data: Vec::new(),
        })
    }

    /// Parse a split CryEngine DDS file and merge sibling chunks.
    ///
    /// `base_data` is the content of the `.dds` file (header + smallest mip data).
    /// `reader` provides access to sibling files (`.1`, `.2`, ..., `.7`, `.a`, `.1a`, etc.).
    pub fn from_split(base_data: &[u8], reader: &dyn ReadSibling) -> Result<Self, DdsError> {
        let mut r = SpanReader::new(base_data);

        // Read and validate magic
        let magic = r.read_bytes(4)?;
        if magic != DDS_MAGIC {
            return Err(DdsError::InvalidMagic);
        }

        // Read main header
        let header = *r.read_type::<DdsHeader>()?;

        // Read optional DX10 header
        let dxt10_header = if header.pixel_format.four_cc == *b"DX10" {
            Some(*r.read_type::<DdsHeaderDxt10>()?)
        } else {
            None
        };

        let mip_count = std::cmp::max(1, header.mipmap_count) as usize;

        // Remaining bytes in the base file are the smallest mip level(s)
        let small_mip_bytes = r.read_bytes(r.remaining())?;

        // Calculate sizes for each mip level
        let mip_sizes: Vec<usize> = (0..mip_count)
            .map(|level| {
                compute_mip_size(&header.pixel_format, dxt10_header.as_ref(), header.width, header.height, level)
            })
            .collect();

        // Probe for sibling split files: .8, .7, .6, ..., .1
        // These are ordered from largest mip to smallest. The highest-numbered
        // existing file corresponds to mip 0 (full resolution).
        let mut sibling_data: Vec<Vec<u8>> = Vec::new();
        for n in (1..=8).rev() {
            let suffix = format!(".{n}");
            if let Some(data) = reader.read_sibling(&suffix) {
                sibling_data.push(data);
            }
        }

        // Determine face count from the largest split file
        let faces = if !sibling_data.is_empty() && !mip_sizes.is_empty() {
            let largest_mip_size = mip_sizes[0];
            if largest_mip_size > 0 && sibling_data[0].len().is_multiple_of(largest_mip_size) {
                sibling_data[0].len() / largest_mip_size
            } else {
                1
            }
        } else {
            face_count(&header, dxt10_header.as_ref())
        };

        // Assemble mip data: sibling files first (largest to smallest), then base file mips
        let num_sibling_mips = sibling_data.len();
        let mut mip_data = Vec::with_capacity(mip_count);

        if faces <= 1 {
            // Simple case: no cubemap
            // Sibling files each contain one mip level
            for sib in &sibling_data {
                mip_data.push(sib.clone());
            }

            // Remaining mips from the base file
            let mut offset = 0;
            for &size in mip_sizes.iter().skip(num_sibling_mips) {
                if offset + size > small_mip_bytes.len() {
                    break;
                }
                mip_data.push(small_mip_bytes[offset..offset + size].to_vec());
                offset += size;
            }
        } else {
            // Cubemap case: each sibling file contains data for ALL faces at that mip level.
            for sib in &sibling_data {
                mip_data.push(sib.clone());
            }

            let mut offset = 0;
            for &per_face_size in mip_sizes.iter().skip(num_sibling_mips) {
                let total_size = per_face_size * faces;
                if offset + total_size > small_mip_bytes.len() {
                    // Try without face multiplier as fallback
                    if offset + per_face_size <= small_mip_bytes.len() {
                        mip_data.push(small_mip_bytes[offset..offset + per_face_size].to_vec());
                        offset += per_face_size;
                    } else {
                        break;
                    }
                } else {
                    mip_data.push(small_mip_bytes[offset..offset + total_size].to_vec());
                    offset += total_size;
                }
            }
        }

        // Probe for alpha sibling files: .7a, .6a, ..., .1a, then .a
        let mut alpha_mip_data: Vec<Vec<u8>> = Vec::new();
        for n in (1..=8).rev() {
            let suffix = format!(".{n}a");
            if let Some(data) = reader.read_sibling(&suffix) {
                alpha_mip_data.push(data);
            }
        }
        if let Some(data) = reader.read_sibling(".a") {
            alpha_mip_data.push(data);
        }

        Ok(Self {
            header,
            dxt10_header,
            mip_data,
            alpha_mip_data,
        })
    }

    /// Serialize back to a complete DDS file (magic + header + optional DX10 + mip data).
    pub fn to_dds(&self) -> Vec<u8> {
        let total_mip_bytes: usize = self.mip_data.iter().map(|m| m.len()).sum();
        let header_bytes = 4 + 124 + if self.dxt10_header.is_some() { 20 } else { 0 };
        let mut out = Vec::with_capacity(header_bytes + total_mip_bytes);

        out.extend_from_slice(&DDS_MAGIC);
        out.extend_from_slice(zerocopy::IntoBytes::as_bytes(&self.header));
        if let Some(ref dx10) = self.dxt10_header {
            out.extend_from_slice(zerocopy::IntoBytes::as_bytes(dx10));
        }

        for mip in &self.mip_data {
            out.extend_from_slice(mip);
        }

        out
    }

    /// Decode a specific mip level to RGBA8 pixels.
    ///
    /// Returns a `Vec<u8>` with 4 bytes per pixel (R, G, B, A).
    pub fn decode_rgba(&self, mip_level: usize) -> Result<Vec<u8>, DdsError> {
        if mip_level >= self.mip_data.len() {
            return Err(DdsError::MipOutOfRange {
                level: mip_level,
                max: self.mip_data.len().saturating_sub(1),
            });
        }

        let pf = &self.header.pixel_format;
        let (w, h) = self.dimensions(mip_level);
        let data = &self.mip_data[mip_level];

        // Uncompressed: FourCC is zero and rgb_bit_count is set
        if pf.four_cc == [0; 4] && self.dxt10_header.is_none() && pf.rgb_bit_count > 0 {
            return decode_uncompressed(data, w, h, pf);
        }

        // DX10 header with an uncompressed DXGI format (e.g. R8_UNORM)
        if let Some(ref dx10) = self.dxt10_header {
            if let Some(rgba) = decode_dxgi_uncompressed(data, w, h, dx10.dxgi_format)? {
                return Ok(rgba);
            }
        }

        let format = resolve_format(pf, self.dxt10_header.as_ref())?;
        let is_snorm = matches!(format, DxgiFormat::BC4Snorm | DxgiFormat::BC5Snorm);
        let block_format = dxgi_to_block_format(format)?;

        decode_block_compressed(data, w, h, block_format, is_snorm)
    }

    /// Decode an alpha sibling mip to a single-channel grayscale buffer.
    ///
    /// Alpha mips are stored in separate `.7a`, `.6a`, ..., `.a` sibling files
    /// and contain per-pixel smoothness data (EFTT_SMOOTHNESS) as BC4 compressed.
    /// Returns a `Vec<u8>` with 1 byte per pixel (smoothness value).
    pub fn decode_alpha_mip(&self, mip_level: usize) -> Result<Vec<u8>, DdsError> {
        if mip_level >= self.alpha_mip_data.len() {
            return Err(DdsError::MipOutOfRange {
                level: mip_level,
                max: self.alpha_mip_data.len().saturating_sub(1),
            });
        }
        let (w, h) = self.dimensions(mip_level);
        let data = &self.alpha_mip_data[mip_level];
        // Alpha mips are BC4 compressed (single channel).
        // Use the main decode path (which returns RGBA with R=G=B=value),
        // then extract just the R channel.
        let rgba = crate::decode::decode_block_compressed(data, w, h, crate::decode::BlockFormat::BC4, false)?;
        Ok(rgba.iter().step_by(4).copied().collect())
    }

    /// Check if this DDS has alpha/smoothness mip data.
    pub fn has_alpha_mips(&self) -> bool {
        !self.alpha_mip_data.is_empty()
    }

    /// Save a specific mip level as a PNG file.
    pub fn save_png(&self, path: &Path, mip_level: usize) -> Result<(), DdsError> {
        let rgba = self.decode_rgba(mip_level)?;
        let (w, h) = self.dimensions(mip_level);

        let img = image::RgbaImage::from_raw(w, h, rgba)
            .ok_or_else(|| DdsError::Image("failed to create image from raw pixels".to_string()))?;

        img.save(path).map_err(|e| DdsError::Image(e.to_string()))?;

        Ok(())
    }

    /// Get the dimensions (width, height) of a specific mip level.
    pub fn dimensions(&self, mip_level: usize) -> (u32, u32) {
        let w = std::cmp::max(1, self.header.width >> mip_level);
        let h = std::cmp::max(1, self.header.height >> mip_level);
        (w, h)
    }

    /// Number of mip levels present.
    pub fn mip_count(&self) -> usize {
        self.mip_data.len()
    }

    /// Returns `true` if this is a cubemap texture.
    pub fn is_cubemap(&self) -> bool {
        // Check DX10 misc_flag bit 2 (D3D11_RESOURCE_MISC_TEXTURECUBE = 0x4)
        if let Some(ref dx10) = self.dxt10_header
            && dx10.misc_flag & 0x4 != 0
        {
            return true;
        }
        // Check cubemap_flags in base header (DDSCAPS2_CUBEMAP = 0x200)
        self.header.cubemap_flags & 0x200 != 0
    }
}

/// Calculate the byte size for a single mip level.
/// Works for both block-compressed and uncompressed formats.
fn compute_mip_size(
    pf: &DdsPixelFormat,
    dxt10: Option<&DdsHeaderDxt10>,
    width: u32,
    height: u32,
    mip_level: usize,
) -> usize {
    let w = std::cmp::max(1, width >> mip_level);
    let h = std::cmp::max(1, height >> mip_level);

    // Try block-compressed first
    if let Ok(format) = resolve_format(pf, dxt10) {
        let block_size = format.block_size();
        let blocks_w = w.div_ceil(4) as usize;
        let blocks_h = h.div_ceil(4) as usize;
        return blocks_w * blocks_h * block_size;
    }

    // DX10 uncompressed formats — bytes per pixel from DXGI format
    if let Some(dx10) = dxt10 {
        if let Some(bpp) = dxgi_uncompressed_bpp(dx10.dxgi_format) {
            return (w as usize) * (h as usize) * bpp;
        }
    }

    // Legacy uncompressed: use rgb_bit_count
    let bpp = { pf.rgb_bit_count } as usize;
    let byte_pp = if bpp > 0 { bpp / 8 } else { 4 }; // default to 32-bit
    (w as usize) * (h as usize) * byte_pp
}

/// Determine number of faces (1 for regular textures, 6 for cubemaps).
fn face_count(header: &DdsHeader, dxt10: Option<&DdsHeaderDxt10>) -> usize {
    if let Some(dx10) = dxt10
        && dx10.misc_flag & 0x4 != 0
    {
        return 6;
    }
    // DDSCAPS2_CUBEMAP = 0x200
    if header.cubemap_flags & 0x200 != 0 {
        // Count set face bits
        let mut faces = 0;
        for bit in [0x400, 0x800, 0x1000, 0x2000, 0x4000, 0x8000] {
            if header.cubemap_flags & bit != 0 {
                faces += 1;
            }
        }
        if faces == 0 { 6 } else { faces }
    } else {
        1
    }
}

/// Resolve the DXGI format from the DDS headers.
pub fn resolve_format(
    pf: &DdsPixelFormat,
    dxt10: Option<&DdsHeaderDxt10>,
) -> Result<DxgiFormat, DdsError> {
    if let Some(dx10) = dxt10 {
        let dxgi_fmt = { dx10.dxgi_format };
        DxgiFormat::from_dxgi(dxgi_fmt)
            .ok_or_else(|| DdsError::UnsupportedFormat(format!("DXGI format {dxgi_fmt}")))
    } else {
        let four_cc = { pf.four_cc };
        DxgiFormat::from_four_cc(&four_cc).ok_or_else(|| {
            let cc = String::from_utf8_lossy(&four_cc);
            let flags = { pf.flags };
            let bpp = { pf.rgb_bit_count };
            let rm = { pf.r_bit_mask };
            let gm = { pf.g_bit_mask };
            let bm = { pf.b_bit_mask };
            let am = { pf.a_bit_mask };
            DdsError::UnsupportedFormat(format!(
                "FourCC '{cc}' (flags=0x{flags:08X}, bpp={bpp}, rmask=0x{rm:08X}, gmask=0x{gm:08X}, bmask=0x{bm:08X}, amask=0x{am:08X})"
            ))
        })
    }
}

/// Map a DxgiFormat to the decode module's BlockFormat.
fn dxgi_to_block_format(format: DxgiFormat) -> Result<BlockFormat, DdsError> {
    match format {
        DxgiFormat::BC1Unorm | DxgiFormat::BC1UnormSrgb => Ok(BlockFormat::BC1),
        DxgiFormat::BC3Unorm | DxgiFormat::BC3UnormSrgb => Ok(BlockFormat::BC3),
        DxgiFormat::BC4Unorm | DxgiFormat::BC4Snorm => Ok(BlockFormat::BC4),
        DxgiFormat::BC5Unorm | DxgiFormat::BC5Snorm => Ok(BlockFormat::BC5),
        DxgiFormat::BC6hUf16 => Ok(BlockFormat::BC6H),
        DxgiFormat::BC7Unorm | DxgiFormat::BC7UnormSrgb => Ok(BlockFormat::BC7),
    }
}

/// Decode uncompressed pixel data using the DDS pixel format bit masks.
fn decode_uncompressed(
    data: &[u8],
    width: u32,
    height: u32,
    pf: &DdsPixelFormat,
) -> Result<Vec<u8>, DdsError> {
    let bpp = pf.rgb_bit_count as usize;
    let byte_pp = bpp / 8;
    if byte_pp == 0 || bpp % 8 != 0 {
        return Err(DdsError::UnsupportedFormat(format!(
            "uncompressed {bpp}-bit (non-byte-aligned)"
        )));
    }

    let pixel_count = (width as usize) * (height as usize);
    let expected = pixel_count * byte_pp;
    if data.len() < expected {
        return Err(DdsError::Decode(format!(
            "uncompressed data too short: need {expected}, have {}",
            data.len()
        )));
    }

    let r_shift = pf.r_bit_mask.trailing_zeros();
    let g_shift = pf.g_bit_mask.trailing_zeros();
    let b_shift = pf.b_bit_mask.trailing_zeros();
    let has_alpha = pf.a_bit_mask != 0;
    let a_shift = pf.a_bit_mask.trailing_zeros();

    let mut out = vec![255u8; pixel_count * 4];

    for i in 0..pixel_count {
        let off = i * byte_pp;
        let mut raw = 0u32;
        for b in 0..byte_pp {
            raw |= (data[off + b] as u32) << (b * 8);
        }

        out[i * 4] = ((raw >> r_shift) & 0xFF) as u8;
        out[i * 4 + 1] = ((raw >> g_shift) & 0xFF) as u8;
        out[i * 4 + 2] = ((raw >> b_shift) & 0xFF) as u8;
        if has_alpha {
            out[i * 4 + 3] = ((raw >> a_shift) & 0xFF) as u8;
        }
    }

    Ok(out)
}

/// Decode uncompressed pixel data identified by a DXGI format value in a DX10 header.
///
/// Returns `Ok(Some(rgba))` if the format is a known uncompressed type,
/// `Ok(None)` if the format is not uncompressed (caller should try block-compressed),
/// or `Err` if the data is invalid for the format.
fn decode_dxgi_uncompressed(
    data: &[u8],
    width: u32,
    height: u32,
    dxgi_format: u32,
) -> Result<Option<Vec<u8>>, DdsError> {
    let pixel_count = (width as usize) * (height as usize);
    match dxgi_format {
        // DXGI_FORMAT_R8_UNORM (61) — single-channel 8-bit grayscale
        61 => {
            if data.len() < pixel_count {
                return Err(DdsError::Decode(format!(
                    "R8_UNORM data too short: need {pixel_count}, have {}",
                    data.len()
                )));
            }
            let mut out = vec![255u8; pixel_count * 4];
            for i in 0..pixel_count {
                let v = data[i];
                out[i * 4] = v;
                out[i * 4 + 1] = v;
                out[i * 4 + 2] = v;
                // alpha stays 255
            }
            Ok(Some(out))
        }
        // DXGI_FORMAT_R8G8_UNORM (49) — two-channel 8-bit
        49 => {
            let expected = pixel_count * 2;
            if data.len() < expected {
                return Err(DdsError::Decode(format!(
                    "R8G8_UNORM data too short: need {expected}, have {}",
                    data.len()
                )));
            }
            let mut out = vec![255u8; pixel_count * 4];
            for i in 0..pixel_count {
                out[i * 4] = data[i * 2];
                out[i * 4 + 1] = data[i * 2 + 1];
                out[i * 4 + 2] = 0;
                // alpha stays 255
            }
            Ok(Some(out))
        }
        // DXGI_FORMAT_R8G8B8A8_UNORM (28) / R8G8B8A8_UNORM_SRGB (29)
        28 | 29 => {
            let expected = pixel_count * 4;
            if data.len() < expected {
                return Err(DdsError::Decode(format!(
                    "R8G8B8A8_UNORM data too short: need {expected}, have {}",
                    data.len()
                )));
            }
            Ok(Some(data[..expected].to_vec()))
        }
        // DXGI_FORMAT_B8G8R8A8_UNORM (87) / B8G8R8A8_UNORM_SRGB (91)
        87 | 91 => {
            let expected = pixel_count * 4;
            if data.len() < expected {
                return Err(DdsError::Decode(format!(
                    "B8G8R8A8_UNORM data too short: need {expected}, have {}",
                    data.len()
                )));
            }
            let mut out = Vec::with_capacity(expected);
            for i in 0..pixel_count {
                let off = i * 4;
                out.push(data[off + 2]); // R
                out.push(data[off + 1]); // G
                out.push(data[off]);     // B
                out.push(data[off + 3]); // A
            }
            Ok(Some(out))
        }
        _ => Ok(None),
    }
}

/// Bytes per pixel for known uncompressed DXGI formats.
/// Returns `None` for block-compressed or unknown formats.
fn dxgi_uncompressed_bpp(dxgi_format: u32) -> Option<usize> {
    match dxgi_format {
        61 => Some(1),  // R8_UNORM
        49 => Some(2),  // R8G8_UNORM
        28 | 29 => Some(4),  // R8G8B8A8_UNORM / SRGB
        87 | 91 => Some(4),  // B8G8R8A8_UNORM / SRGB
        _ => None,
    }
}
