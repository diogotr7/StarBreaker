//! Lossless package DDS serialization and exact per-mip byte sizes.
//! Colour and smoothness chains use a normalized DX10 header; unsupported inputs
//! return `None` so callers can retain their existing decoded export path.
use crate::{AlphaMipFormat, DdsFile, DdsHeaderDxt10, DdsPixelFormat};
use zerocopy::IntoBytes;

/// Exact payload size for a supported package format (including partial blocks).
pub fn mip_byte_size(format: u32, width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    let (w, h, unit) = match format {
        61 => (width, height, 1),
        28 | 29 | 87 | 91 => (width, height, 4),
        71 | 72 | 80 | 81 => (width.div_ceil(4), height.div_ceil(4), 8),
        74 | 75 | 77 | 78 | 83 | 84 | 98 | 99 => (width.div_ceil(4), height.div_ceil(4), 16),
        _ => return None,
    };
    (w as usize).checked_mul(h as usize)?.checked_mul(unit)
}

/// Maximum legal mip-chain length for these dimensions (zero for an empty texture).
pub fn max_mip_count(width: u32, height: u32) -> u32 {
    u32::BITS - width.max(height).leading_zeros()
}

/// Package compressed top levels must satisfy the consumer's block alignment.
/// Tail levels may be smaller than a block; uncompressed tops have no such limit.
pub fn top_mip_is_aligned(format: u32, width: u32, height: u32) -> bool {
    !matches!(
        format,
        71 | 72 | 74 | 75 | 77 | 78 | 80 | 81 | 83 | 84 | 98 | 99
    ) || (width.is_multiple_of(4) && height.is_multiple_of(4))
}

fn colour_format(dds: &DdsFile) -> Option<u32> {
    let pf = dds.header.pixel_format;
    let format = if let Some(dx10) = dds.dxt10_header {
        dx10.dxgi_format
    } else {
        match &pf.four_cc {
            b"DXT1" => 71,
            // The decoder's legacy DXT3 approximation is not a storage format.
            b"DXT3" => 74,
            b"DXT5" => 77,
            b"ATI1" | b"BC4U" => 80,
            b"BC4S" => 81,
            b"ATI2" | b"BC5U" => 83,
            b"BC5S" => 84,
            [0, 0, 0, 0] => match (
                pf.rgb_bit_count,
                pf.r_bit_mask,
                pf.g_bit_mask,
                pf.b_bit_mask,
                pf.a_bit_mask,
            ) {
                (8, 0xff, 0, 0, 0) => 61,
                (32, 0xff0000, 0xff00, 0xff, 0xff000000) => 87,
                (32, 0xff, 0xff00, 0xff0000, 0xff000000) => 28,
                _ => return None,
            },
            _ => return None,
        }
    };
    if dds.is_srgb() {
        Some(match format {
            71 | 72 => 72,
            74 | 75 => 75,
            77 | 78 => 78,
            98 | 99 => 99,
            87 | 91 => 91,
            28 | 29 => 29,
            _ => return None,
        })
    } else {
        Some(format)
    }
}

fn alpha_format(dds: &DdsFile, mip: usize) -> Option<u32> {
    Some(match dds.alpha_mip_format_for_mip(mip)? {
        AlphaMipFormat::Bc4Unorm => 80,
        AlphaMipFormat::Bc4Snorm => 81,
        AlphaMipFormat::R8Unorm => 61,
    })
}

/// Write colour (`alpha=false`) or linear smoothness (`alpha=true`) mips.
/// Requested mip is clamped to the selected chain; all retained mips must have
/// exactly the format's byte size. No decoding or pixel transformations occur.
pub fn write_package_dds(dds: &DdsFile, mip: u32, alpha: bool) -> Option<Vec<u8>> {
    let h = dds.header;
    if h.width == 0
        || h.height == 0
        || h.depth > 1
        || h.cubemap_flags & (0xfe00 | 0x200000) != 0
        || h.flags & 0x800000 != 0
        || dds
            .dxt10_header
            .is_some_and(|dx| dx.resource_dimension != 3 || dx.misc_flag != 0 || dx.array_size != 1)
    {
        return None;
    }
    let mips = if alpha {
        &dds.alpha_mip_data
    } else {
        &dds.mip_data
    };
    if mips.is_empty() || mips.len() > max_mip_count(h.width, h.height) as usize {
        return None;
    }
    let first = (mip as usize).min(mips.len() - 1);
    let format = if alpha {
        alpha_format(dds, first)?
    } else {
        colour_format(dds)?
    };
    let (width, height) = dds.dimensions(first);
    if !top_mip_is_aligned(format, width, height) {
        return None;
    }
    for (level, bytes) in mips.iter().enumerate().skip(first) {
        if alpha && alpha_format(dds, level)? != format {
            return None;
        }
        let (w, h) = dds.dimensions(level);
        if bytes.len() != mip_byte_size(format, w, h)? {
            return None;
        }
    }
    let mut header = h;
    header.size = 124;
    header.flags = 0xA1007;
    header.width = width;
    header.height = height;
    header.pitch_or_linear_size = u32::try_from(mips[first].len()).ok()?;
    header.depth = 0;
    header.mipmap_count = u32::try_from(mips.len() - first).ok()?;
    header.reserved1 = [0; 11];
    header.pixel_format = DdsPixelFormat {
        size: 32,
        flags: 4,
        four_cc: *b"DX10",
        rgb_bit_count: 0,
        r_bit_mask: 0,
        g_bit_mask: 0,
        b_bit_mask: 0,
        a_bit_mask: 0,
    };
    header.surface_flags = if mips.len() - first > 1 {
        0x401008
    } else {
        0x1000
    };
    header.cubemap_flags = 0;
    header.reserved2 = [0; 3];
    let dx10 = DdsHeaderDxt10 {
        dxgi_format: format,
        resource_dimension: 3,
        misc_flag: 0,
        array_size: 1,
        misc_flags2: 0,
    };
    let mut bytes = b"DDS ".to_vec();
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(dx10.as_bytes());
    for data in &mips[first..] {
        bytes.extend_from_slice(data);
    }
    Some(bytes)
}
