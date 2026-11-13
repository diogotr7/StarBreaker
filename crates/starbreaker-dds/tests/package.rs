//! Synthetic coverage of package DDS headers, format selection, and mip validation.
use starbreaker_dds::package::write_package_dds;
use starbreaker_dds::{AlphaMipFormat, DdsFile};

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn source(format: u32) -> DdsFile {
    let mut bytes = vec![0; 148];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124),
        (8, 0xA1007),
        (12, 8),
        (16, 8),
        (28, 3),
        (76, 32),
        (80, 4),
        (128, format),
        (132, 3),
        (140, 1),
    ] {
        put(&mut bytes, offset, value);
    }
    bytes[84..88].copy_from_slice(b"DX10");
    let mut dds = DdsFile::headers_only(&bytes).unwrap();
    let unit = match format {
        61 => 1,
        28 | 29 | 87 | 91 => 4,
        71 | 72 | 80 | 81 => 8,
        _ => 16,
    };
    dds.mip_data = [8_u32, 4, 2]
        .map(|w| {
            let n = if [61, 28, 29, 87, 91].contains(&format) {
                w * w * unit
            } else {
                w.div_ceil(4).pow(2) * unit
            };
            vec![w as u8; n as usize]
        })
        .to_vec();
    dds
}
#[test]
fn package_header_slices_and_clamps_mips() {
    let dds = source(83);
    let bytes = write_package_dds(&dds, 1, false).unwrap();
    for (offset, value) in [
        (4, 124),
        (8, 0xA1007),
        (12, 4),
        (16, 4),
        (20, 16),
        (28, 2),
        (76, 32),
        (80, 4),
        (128, 83),
        (132, 3),
        (136, 0),
        (140, 1),
    ] {
        assert_eq!(word(&bytes, offset), value, "offset {offset}");
    }
    assert_eq!(&bytes[84..88], b"DX10");
    assert_eq!(&bytes[148..], dds.mip_data[1..].concat());
    let clamped = write_package_dds(&source(61), 99, false).unwrap();
    assert_eq!(word(&clamped, 16), 2);
    assert_eq!(word(&clamped, 28), 1);
}
#[test]
fn package_dxgi_fourcc_masks_and_srgb() {
    for format in [
        61, 71, 72, 74, 75, 77, 78, 80, 81, 83, 84, 98, 99, 87, 91, 28, 29,
    ] {
        assert_eq!(
            word(&write_package_dds(&source(format), 0, false).unwrap(), 128),
            format
        );
    }
    for (fourcc, format) in [
        (b"DXT1", 71),
        (b"DXT3", 74),
        (b"DXT5", 77),
        (b"ATI1", 80),
        (b"BC4U", 80),
        (b"BC4S", 81),
        (b"ATI2", 83),
        (b"BC5U", 83),
        (b"BC5S", 84),
    ] {
        let mut dds = source(format);
        dds.dxt10_header = None;
        dds.header.pixel_format.four_cc = *fourcc;
        assert_eq!(
            word(&write_package_dds(&dds, 0, false).unwrap(), 128),
            format
        );
    }
    for (bits, r, g, b, a, format) in [
        (8, 255, 0, 0, 0, 61),
        (32, 0xff0000, 0xff00, 255, 0xff000000, 87),
        (32, 255, 0xff00, 0xff0000, 0xff000000, 28),
    ] {
        let mut dds = source(format);
        dds.dxt10_header = None;
        let pf = &mut dds.header.pixel_format;
        pf.four_cc = [0; 4];
        pf.rgb_bit_count = bits;
        pf.r_bit_mask = r;
        pf.g_bit_mask = g;
        pf.b_bit_mask = b;
        pf.a_bit_mask = a;
        assert_eq!(
            word(&write_package_dds(&dds, 0, false).unwrap(), 128),
            format
        );
    }
    for (linear, srgb) in [(71, 72), (74, 75), (77, 78), (98, 99), (87, 91), (28, 29)] {
        let mut dds = source(linear);
        dds.header.reserved1[1] = 0x800;
        dds.header.reserved2[2] = u32::from_le_bytes(*b"FYRC");
        assert_eq!(word(&write_package_dds(&dds, 0, false).unwrap(), 128), srgb);
    }
}
#[test]
fn rejects_unsupported_shapes_encodings_and_wrong_sizes() {
    assert!(write_package_dds(&source(95), 0, false).is_none());
    for variant in 0..7 {
        let mut dds = source(83);
        match variant {
            0 => dds.mip_data[1].push(0),
            1 => dds.header.cubemap_flags = 0x200,
            2 => dds.header.depth = 2,
            3 => dds.dxt10_header.as_mut().unwrap().array_size = 2,
            4 => dds.dxt10_header.as_mut().unwrap().resource_dimension = 4,
            5 => dds.dxt10_header.as_mut().unwrap().misc_flag = 4,
            _ => {
                dds.header.reserved1[1] = 0x800;
                dds.header.reserved2[2] = u32::from_le_bytes(*b"FYRC");
            }
        }
        assert!(
            write_package_dds(&dds, 0, false).is_none(),
            "variant {variant}"
        );
    }
}
#[test]
fn alpha_mips_require_uniform_formats_and_exact_sizes() {
    for (alpha, dxgi) in [
        (AlphaMipFormat::Bc4Unorm, 80),
        (AlphaMipFormat::Bc4Snorm, 81),
        (AlphaMipFormat::R8Unorm, 61),
    ] {
        let mut dds = source(84);
        dds.alpha_mip_data = source(dxgi).mip_data;
        dds.alpha_mip_formats = vec![Some(alpha); 3];
        let bytes = write_package_dds(&dds, 1, true).unwrap();
        assert_eq!(word(&bytes, 128), dxgi);
        assert_eq!(&bytes[148..], dds.alpha_mip_data[1..].concat());
        dds.alpha_mip_data[2].push(0);
        assert!(write_package_dds(&dds, 1, true).is_none());
    }
    let mut dds = source(84);
    dds.alpha_mip_data = source(80).mip_data;
    dds.alpha_mip_formats = vec![
        Some(AlphaMipFormat::Bc4Unorm),
        Some(AlphaMipFormat::Bc4Snorm),
        Some(AlphaMipFormat::Bc4Snorm),
    ];
    assert!(write_package_dds(&dds, 0, true).is_none());
    assert!(write_package_dds(&dds, 1, true).is_some());
}

#[test]
fn dx10_bc2_mips_survive_source_parsing_before_pass_through() {
    for format in [74, 75] {
        let original = source(format);
        let bytes = original.to_dds();
        let parsed = DdsFile::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.mip_data, original.mip_data);
        assert_eq!(
            write_package_dds(&parsed, 0, false),
            write_package_dds(&original, 0, false)
        );
    }
}

#[test]
fn package_mip_count_is_bounded_by_dimensions() {
    for alpha in [false, true] {
        let mut dds = source(71);
        dds.header.width = 4;
        dds.header.height = 4;
        for count in [0, 1, 3, 4] {
            let mips = vec![vec![0; 8]; count];
            if alpha {
                dds.alpha_mip_data = mips;
                dds.alpha_mip_formats = vec![Some(AlphaMipFormat::Bc4Unorm); count];
            } else {
                dds.mip_data = mips;
            }
            assert_eq!(
                write_package_dds(&dds, 0, alpha).is_some(),
                matches!(count, 1 | 3),
                "alpha={alpha}, count={count}"
            );
        }
    }
}

#[test]
fn package_rejects_unaligned_compressed_top_mips() {
    for format in [71, 72, 74, 75, 77, 78, 80, 81, 83, 84, 98, 99] {
        for (w, h) in [(1, 1), (2, 2), (3, 4), (4, 5), (5, 7)] {
            let mut dds = source(format);
            dds.header.width = w;
            dds.header.height = h;
            dds.mip_data = vec![vec![
                0;
                starbreaker_dds::package::mip_byte_size(format, w, h)
                    .unwrap()
            ]];
            assert!(
                write_package_dds(&dds, 0, false).is_none(),
                "format={format}, top={w}x{h}"
            );
        }
    }
    let dds = source(83);
    assert!(write_package_dds(&dds, 1, false).is_some());
    assert!(write_package_dds(&dds, 2, false).is_none());
    let mut alpha = source(83);
    alpha.alpha_mip_data = source(80).mip_data;
    alpha.alpha_mip_formats = vec![Some(AlphaMipFormat::Bc4Unorm); 3];
    assert!(write_package_dds(&alpha, 2, true).is_none());
    // Uncompressed small tops remain valid, including a clamped last mip.
    assert!(write_package_dds(&source(61), 99, false).is_some());
}
