//! Split alpha/smoothness mip parsing for CryEngine DDS siblings.
//!
//! This module owns the DDNA-facing alpha mip format/layout metadata, alpha tail
//! parsing, raw-tail splitting, and raw R8 alpha decoding helpers used by
//! `DdsFile::from_split` and `DdsFile::decode_alpha_mip`.

use starbreaker_common::SpanReader;

use crate::error::DdsError;
use crate::types::{DDS_MAGIC, DdsHeader, DdsHeaderDxt10, DdsPixelFormat};

/// Encoding used by split alpha/smoothness mip payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMipFormat {
    Bc4Unorm,
    Bc4Snorm,
    R8Unorm,
}

/// How a split alpha/smoothness mip was found in CryEngine sibling data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMipLayout {
    NumberedSibling,
    HeaderedTail,
    RawTailSplit,
    RawSinglePayload,
}

pub(crate) struct ParsedAlphaTail {
    pub(crate) mips: Vec<Vec<u8>>,
    pub(crate) format: Option<AlphaMipFormat>,
}

pub(crate) fn parse_alpha_tail_mips(
    data: &[u8],
    numbered_alpha_mips: usize,
    base_mip_count: usize,
    base_width: u32,
    base_height: u32,
) -> ParsedAlphaTail {
    let mut reader = SpanReader::new(data);
    if data.starts_with(&DDS_MAGIC) && reader.read_bytes(4).is_err() {
        return empty_alpha_tail();
    }

    let Ok(header_ref) = reader.read_type::<DdsHeader>() else {
        return empty_alpha_tail();
    };
    let header = *header_ref;
    let dxt10_header = if header.pixel_format.four_cc == *b"DX10" {
        match reader.read_type::<DdsHeaderDxt10>() {
            Ok(header) => Some(*header),
            Err(_) => return empty_alpha_tail(),
        }
    } else {
        None
    };
    let remaining = match reader.read_bytes(reader.remaining()) {
        Ok(bytes) => bytes,
        Err(_) => return empty_alpha_tail(),
    };

    let mip_count = std::cmp::max(1, header.mipmap_count) as usize;
    if mip_count != base_mip_count || header.width != base_width || header.height != base_height {
        return empty_alpha_tail();
    }
    if numbered_alpha_mips >= mip_count {
        return empty_alpha_tail();
    }

    let format = alpha_mip_format_from_header(&header.pixel_format, dxt10_header.as_ref());
    if format.is_none() || header.width == 0 || header.height == 0 {
        return empty_alpha_tail();
    }
    let faces = alpha_face_count(&header, dxt10_header.as_ref());
    let mip_sizes = alpha_tail_mip_sizes(
        0,
        mip_count,
        header.width,
        header.height,
        format.expect("format checked"),
    );

    let mut offset = 0;
    let mut mips = Vec::new();
    for &per_face_size in mip_sizes.iter().skip(numbered_alpha_mips) {
        let size = per_face_size.saturating_mul(faces);
        if size == 0 || offset + size > remaining.len() {
            break;
        }
        mips.push(remaining[offset..offset + size].to_vec());
        offset += size;
    }
    if offset != remaining.len() {
        return empty_alpha_tail();
    }
    ParsedAlphaTail { mips, format }
}

pub(crate) fn parse_raw_alpha_tail_mips(
    data: &[u8],
    numbered_alpha_mips: usize,
    mip_count: usize,
    width: u32,
    height: u32,
) -> ParsedAlphaTail {
    if numbered_alpha_mips >= mip_count {
        return empty_alpha_tail();
    }
    let r8_sizes = alpha_tail_mip_sizes(
        numbered_alpha_mips,
        mip_count,
        width,
        height,
        AlphaMipFormat::R8Unorm,
    );
    if data.len() == r8_sizes.iter().sum::<usize>() {
        return split_raw_alpha_tail(data, &r8_sizes, AlphaMipFormat::R8Unorm);
    }
    let bc4_sizes = alpha_tail_mip_sizes(
        numbered_alpha_mips,
        mip_count,
        width,
        height,
        AlphaMipFormat::Bc4Unorm,
    );
    if data.len() == bc4_sizes.iter().sum::<usize>() {
        return split_raw_alpha_tail(data, &bc4_sizes, AlphaMipFormat::Bc4Unorm);
    }
    empty_alpha_tail()
}

pub(crate) fn infer_alpha_mip_format_from_payload(
    data: &[u8],
    width: u32,
    height: u32,
) -> Option<AlphaMipFormat> {
    let pixel_count = (width as usize) * (height as usize);
    let bc4_size = bc4_mip_byte_size(width, height);
    if data.len() == pixel_count && data.len() != bc4_size {
        Some(AlphaMipFormat::R8Unorm)
    } else if data.len() == bc4_size {
        Some(AlphaMipFormat::Bc4Unorm)
    } else {
        None
    }
}

pub(crate) fn decode_r8_alpha_mip(
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, DdsError> {
    let pixel_count = (width as usize) * (height as usize);
    if data.len() < pixel_count {
        return Err(DdsError::Decode(format!(
            "R8 alpha mip data too short: need {pixel_count}, have {}",
            data.len()
        )));
    }
    Ok(data[..pixel_count].to_vec())
}

fn empty_alpha_tail() -> ParsedAlphaTail {
    ParsedAlphaTail {
        mips: Vec::new(),
        format: None,
    }
}

fn alpha_tail_mip_sizes(
    first_mip: usize,
    mip_count: usize,
    width: u32,
    height: u32,
    format: AlphaMipFormat,
) -> Vec<usize> {
    (first_mip..mip_count)
        .map(|level| {
            let w = std::cmp::max(1, width >> level);
            let h = std::cmp::max(1, height >> level);
            match format {
                AlphaMipFormat::R8Unorm => (w as usize) * (h as usize),
                AlphaMipFormat::Bc4Unorm | AlphaMipFormat::Bc4Snorm => bc4_mip_byte_size(w, h),
            }
        })
        .collect()
}

fn split_raw_alpha_tail(data: &[u8], sizes: &[usize], format: AlphaMipFormat) -> ParsedAlphaTail {
    let mut offset = 0;
    let mut mips = Vec::with_capacity(sizes.len());
    for &size in sizes {
        if size == 0 || offset + size > data.len() {
            return empty_alpha_tail();
        }
        mips.push(data[offset..offset + size].to_vec());
        offset += size;
    }
    if offset != data.len() {
        return empty_alpha_tail();
    }
    ParsedAlphaTail {
        mips,
        format: Some(format),
    }
}

fn alpha_mip_format_from_header(
    pf: &DdsPixelFormat,
    dxt10: Option<&DdsHeaderDxt10>,
) -> Option<AlphaMipFormat> {
    if let Some(dx10) = dxt10 {
        return match dx10.dxgi_format {
            61 => Some(AlphaMipFormat::R8Unorm),
            80 => Some(AlphaMipFormat::Bc4Unorm),
            81 => Some(AlphaMipFormat::Bc4Snorm),
            _ => None,
        };
    }
    match &pf.four_cc {
        b"ATI1" | b"BC4U" => Some(AlphaMipFormat::Bc4Unorm),
        b"BC4S" => Some(AlphaMipFormat::Bc4Snorm),
        _ if pf.four_cc == [0; 4] && pf.rgb_bit_count == 8 => Some(AlphaMipFormat::R8Unorm),
        _ => None,
    }
}

fn bc4_mip_byte_size(width: u32, height: u32) -> usize {
    let blocks_w = width.max(1).div_ceil(4) as usize;
    let blocks_h = height.max(1).div_ceil(4) as usize;
    blocks_w * blocks_h * 8
}

fn alpha_face_count(header: &DdsHeader, dxt10: Option<&DdsHeaderDxt10>) -> usize {
    if let Some(dx10) = dxt10
        && dx10.misc_flag & 0x4 != 0
    {
        return 6;
    }
    if header.cubemap_flags & 0x200 != 0 {
        6
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::dds_file::DdsFile;
    use crate::sibling::ReadSibling;

    struct FakeSiblings {
        files: HashMap<&'static str, Vec<u8>>,
    }

    impl ReadSibling for FakeSiblings {
        fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
            self.files.get(suffix).cloned()
        }
    }

    fn dx10_header(width: u32, height: u32, mipmap_count: u32) -> DdsHeader {
        DdsHeader {
            size: 124,
            flags: 0x0002_1007,
            height,
            width,
            pitch_or_linear_size: 0,
            depth: 0,
            mipmap_count,
            reserved1: [0; 11],
            pixel_format: DdsPixelFormat {
                size: 32,
                flags: 0x4,
                four_cc: *b"DX10",
                rgb_bit_count: 0,
                r_bit_mask: 0,
                g_bit_mask: 0,
                b_bit_mask: 0,
                a_bit_mask: 0,
            },
            surface_flags: 0x0040_1008,
            cubemap_flags: 0,
            reserved2: [0; 3],
        }
    }

    fn dx10(format: u32) -> DdsHeaderDxt10 {
        DdsHeaderDxt10 {
            dxgi_format: format,
            resource_dimension: 3,
            misc_flag: 0,
            array_size: 1,
            misc_flags2: 0,
        }
    }

    fn dds_with_magic(header: DdsHeader, dxt10: DdsHeaderDxt10, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DDS_MAGIC);
        bytes.extend_from_slice(zerocopy::IntoBytes::as_bytes(&header));
        bytes.extend_from_slice(zerocopy::IntoBytes::as_bytes(&dxt10));
        bytes.extend_from_slice(payload);
        bytes
    }

    fn dds_without_magic(header: DdsHeader, dxt10: DdsHeaderDxt10, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(zerocopy::IntoBytes::as_bytes(&header));
        bytes.extend_from_slice(zerocopy::IntoBytes::as_bytes(&dxt10));
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn split_alpha_tail_dds_is_split_into_remaining_mips() {
        let color_header = dx10_header(8, 8, 4);
        let alpha_header = dx10_header(8, 8, 4);
        let color_dx10 = dx10(83);
        let alpha_dx10 = dx10(80);

        let color_tail = vec![0xCC; 16 + 16 + 16];
        let alpha_tail = vec![0xAA; 8 + 8 + 8];
        let base = dds_with_magic(color_header, color_dx10, &color_tail);
        let siblings = FakeSiblings {
            files: HashMap::from([
                (".1", vec![0x11; 64]),
                (".1a", vec![0x22; 32]),
                (
                    ".a",
                    dds_without_magic(alpha_header, alpha_dx10, &alpha_tail),
                ),
            ]),
        };

        let dds = DdsFile::from_split(&base, &siblings).expect("split DDS should parse");

        assert_eq!(
            dds.mip_data.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![64, 16, 16, 16]
        );
        assert_eq!(
            dds.alpha_mip_data.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![32, 8, 8, 8]
        );
        assert_eq!(
            dds.alpha_mip_layout_for_mip(0),
            Some(AlphaMipLayout::NumberedSibling)
        );
        assert_eq!(
            dds.alpha_mip_layout_for_mip(1),
            Some(AlphaMipLayout::HeaderedTail)
        );
    }

    #[test]
    fn alpha_tail_header_requires_supported_alpha_format() {
        let alpha_header = dx10_header(8, 8, 4);
        let color_dx10 = dx10(83);
        let alpha_tail = vec![0xAA; 8 + 8 + 8];

        let parsed = parse_alpha_tail_mips(
            &dds_without_magic(alpha_header, color_dx10, &alpha_tail),
            1,
            4,
            8,
            8,
        );

        assert!(parsed.mips.is_empty());
        assert_eq!(parsed.format, None);
    }

    #[test]
    fn alpha_tail_header_rejects_trailing_payload_bytes() {
        let alpha_header = dx10_header(8, 8, 4);
        let alpha_dx10 = dx10(80);
        let mut alpha_tail = vec![0xAA; 8 + 8 + 8];
        alpha_tail.push(0xFF);

        let parsed = parse_alpha_tail_mips(
            &dds_without_magic(alpha_header, alpha_dx10, &alpha_tail),
            1,
            4,
            8,
            8,
        );

        assert!(parsed.mips.is_empty());
        assert_eq!(parsed.format, None);
    }

    #[test]
    fn alpha_tail_header_must_match_base_dimensions_and_mip_count() {
        let alpha_header = dx10_header(16, 8, 4);
        let alpha_dx10 = dx10(80);
        let alpha_tail = vec![0xAA; 8 + 8 + 8];

        let parsed = parse_alpha_tail_mips(
            &dds_without_magic(alpha_header, alpha_dx10, &alpha_tail),
            1,
            4,
            8,
            8,
        );

        assert!(parsed.mips.is_empty());
        assert_eq!(parsed.format, None);
    }

    #[test]
    fn per_mip_alpha_format_keeps_numbered_sibling_independent_from_tail_header() {
        let color_header = dx10_header(8, 8, 4);
        let alpha_header = dx10_header(8, 8, 4);
        let color_dx10 = dx10(83);
        let alpha_dx10 = dx10(61);

        let color_tail = vec![0xCC; 16 + 16 + 16];
        let mut alpha_tail = Vec::new();
        alpha_tail.extend(10..26);
        alpha_tail.extend(30..34);
        alpha_tail.push(40);
        let base = dds_with_magic(color_header, color_dx10, &color_tail);
        let siblings = FakeSiblings {
            files: HashMap::from([
                (".1", vec![0x11; 64]),
                (".1a", vec![0x22; 32]),
                (
                    ".a",
                    dds_without_magic(alpha_header, alpha_dx10, &alpha_tail),
                ),
            ]),
        };

        let dds = DdsFile::from_split(&base, &siblings).expect("split DDS should parse");

        assert_eq!(
            dds.alpha_mip_format_for_mip(0),
            Some(AlphaMipFormat::Bc4Unorm)
        );
        assert_eq!(
            dds.alpha_mip_format_for_mip(1),
            Some(AlphaMipFormat::R8Unorm)
        );
        assert_eq!(
            dds.decode_alpha_mip(1)
                .expect("headered R8 tail mip should decode"),
            (10..26).collect::<Vec<_>>()
        );
    }

    #[test]
    fn split_r8_alpha_tail_decodes_raw_smoothness_mip() {
        let color_header = dx10_header(2, 2, 1);
        let alpha_header = dx10_header(2, 2, 1);
        let color_dx10 = dx10(83);
        let alpha_dx10 = dx10(61);

        let base = dds_with_magic(color_header, color_dx10, &[0xCC; 16]);
        let siblings = FakeSiblings {
            files: HashMap::from([(
                ".a",
                dds_without_magic(alpha_header, alpha_dx10, &[10, 20, 30, 40]),
            )]),
        };

        let dds = DdsFile::from_split(&base, &siblings).expect("split DDS should parse");

        assert_eq!(
            dds.decode_alpha_mip(0).expect("R8 alpha should decode"),
            vec![10, 20, 30, 40]
        );
    }

    #[test]
    fn raw_r8_alpha_sibling_decodes_from_payload_size_without_header() {
        let color_header = dx10_header(2, 2, 1);
        let color_dx10 = dx10(83);

        let base = dds_with_magic(color_header, color_dx10, &[0xCC; 16]);
        let siblings = FakeSiblings {
            files: HashMap::from([(".a", vec![10, 20, 30, 40])]),
        };

        let dds = DdsFile::from_split(&base, &siblings).expect("split DDS should parse");

        assert_eq!(
            dds.alpha_mip_format_for_mip(0),
            Some(AlphaMipFormat::R8Unorm)
        );
        assert_eq!(
            dds.decode_alpha_mip(0).expect("raw R8 alpha should decode"),
            vec![10, 20, 30, 40]
        );
    }

    #[test]
    fn raw_r8_alpha_tail_without_header_splits_remaining_mips() {
        let color_header = dx10_header(4, 4, 3);
        let color_dx10 = dx10(83);
        let mut alpha_tail = Vec::new();
        alpha_tail.extend(10..26);
        alpha_tail.extend(30..34);
        alpha_tail.push(40);

        let base = dds_with_magic(color_header, color_dx10, &[0xCC; 16 + 16 + 16]);
        let siblings = FakeSiblings {
            files: HashMap::from([(".a", alpha_tail)]),
        };

        let dds = DdsFile::from_split(&base, &siblings).expect("split DDS should parse");

        assert_eq!(
            dds.alpha_mip_data.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![16, 4, 1]
        );
        assert_eq!(
            dds.alpha_mip_layout_for_mip(1),
            Some(AlphaMipLayout::RawTailSplit)
        );
        assert_eq!(
            dds.alpha_mip_format_for_mip(1),
            Some(AlphaMipFormat::R8Unorm)
        );
        assert_eq!(
            dds.decode_alpha_mip(1)
                .expect("second raw R8 alpha mip should decode"),
            vec![30, 31, 32, 33]
        );
    }

    #[test]
    fn raw_bc4_alpha_tail_without_header_splits_remaining_mips() {
        let color_header = dx10_header(4, 4, 3);
        let color_dx10 = dx10(83);
        let alpha_tail = vec![0x88; 8 + 8 + 8];

        let base = dds_with_magic(color_header, color_dx10, &[0xCC; 16 + 16 + 16]);
        let siblings = FakeSiblings {
            files: HashMap::from([(".a", alpha_tail)]),
        };

        let dds = DdsFile::from_split(&base, &siblings).expect("split DDS should parse");

        assert_eq!(
            dds.alpha_mip_data.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![8, 8, 8]
        );
        assert_eq!(
            dds.alpha_mip_format_for_mip(2),
            Some(AlphaMipFormat::Bc4Unorm)
        );
        assert_eq!(
            dds.alpha_mip_layout_for_mip(2),
            Some(AlphaMipLayout::RawTailSplit)
        );
    }
}
