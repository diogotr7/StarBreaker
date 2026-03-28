use starbreaker_dds::{DdsFile, FsSiblingReader};

// Tests that require extracted game data on disk are ignored by default.
// Run with: cargo test -- --ignored

const TEST_DIR: &str = "D:/StarCitizen/P4k-470/Data/Materials/Cinematic/male7_textures/head";

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_base_dds_header() {
    let path = format!("{TEST_DIR}/male07_t2_head_base_ddna.dds");
    let data = std::fs::read(&path).unwrap();
    let reader = FsSiblingReader::new(&path);
    let dds = DdsFile::from_split(&data, &reader).unwrap();
    let w = { dds.header.width };
    let h = { dds.header.height };
    let mc = { dds.header.mipmap_count };
    assert_eq!(w, 2048);
    assert_eq!(h, 2048);
    assert!(mc >= 8);
    println!(
        "DDNA: {w}x{h}, {mc} mips declared, {} mips loaded",
        dds.mip_data.len()
    );
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn merge_split_dds() {
    let base_path = format!("{TEST_DIR}/male07_t2_head_base_ddna.dds");
    let base = std::fs::read(&base_path).unwrap();
    let reader = FsSiblingReader::new(&base_path);
    let dds = DdsFile::from_split(&base, &reader).unwrap();
    assert!(
        dds.mip_data.len() >= 8,
        "expected multiple mip levels, got {}",
        dds.mip_data.len()
    );
    assert!(
        !dds.alpha_mip_data.is_empty(),
        "DDNA should have alpha data"
    );
    println!(
        "Merged: {} mips, {} alpha mips",
        dds.mip_data.len(),
        dds.alpha_mip_data.len()
    );
    for (i, mip) in dds.mip_data.iter().enumerate() {
        let (w, h) = dds.dimensions(i);
        println!("  mip {i}: {w}x{h}, {} bytes", mip.len());
    }
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn merge_and_save_png() {
    let base_path = format!("{TEST_DIR}/male07_t2_head_base_diff.dds");
    let base = std::fs::read(&base_path).unwrap();
    let reader = FsSiblingReader::new(&base_path);
    let dds = DdsFile::from_split(&base, &reader).unwrap();
    let w = { dds.header.width };
    let h = { dds.header.height };
    println!("Diff: {w}x{h}, {} mips", dds.mip_data.len());

    // Save mip 0 (full res) as PNG to temp
    let tmp = std::env::temp_dir().join("starbreaker_test_diff.png");
    dds.save_png(&tmp, 0).unwrap();
    println!("Saved to {}", tmp.display());
    assert!(tmp.exists());
    let metadata = std::fs::metadata(&tmp).unwrap();
    assert!(
        metadata.len() > 1000,
        "PNG seems too small: {} bytes",
        metadata.len()
    );
    println!("PNG size: {} bytes", metadata.len());
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn to_dds_round_trip() {
    let base_path = format!("{TEST_DIR}/male07_t2_head_base_diff.dds");
    let base = std::fs::read(&base_path).unwrap();
    let reader = FsSiblingReader::new(&base_path);
    let dds = DdsFile::from_split(&base, &reader).unwrap();
    let merged = dds.to_dds();
    let reparsed = DdsFile::from_bytes(&merged).unwrap();

    let orig_w = { dds.header.width };
    let orig_h = { dds.header.height };
    let re_w = { reparsed.header.width };
    let re_h = { reparsed.header.height };
    assert_eq!(orig_w, re_w);
    assert_eq!(orig_h, re_h);
    assert_eq!(dds.mip_data.len(), reparsed.mip_data.len());

    // Verify mip data matches
    for (i, (orig, re)) in dds
        .mip_data
        .iter()
        .zip(reparsed.mip_data.iter())
        .enumerate()
    {
        assert_eq!(
            orig.len(),
            re.len(),
            "mip {i} size mismatch: {} vs {}",
            orig.len(),
            re.len()
        );
        assert_eq!(orig, re, "mip {i} data mismatch");
    }
    println!(
        "Round-trip OK: {} mips, {} total bytes",
        reparsed.mip_data.len(),
        merged.len()
    );
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn decode_ddna_normal_map() {
    let base_path = format!("{TEST_DIR}/male07_t2_head_base_ddna.dds");
    let base = std::fs::read(&base_path).unwrap();
    let reader = FsSiblingReader::new(&base_path);
    let dds = DdsFile::from_split(&base, &reader).unwrap();

    // Decode a smaller mip for speed
    let mip = dds.mip_data.len() - 1;
    let (w, h) = dds.dimensions(mip);
    let rgba = dds.decode_rgba(mip).unwrap();
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    println!("Decoded DDNA mip {mip}: {w}x{h}, {} bytes", rgba.len());

    // Save the full-res normal map as PNG
    let tmp = std::env::temp_dir().join("starbreaker_test_ddna.png");
    dds.save_png(&tmp, 0).unwrap();
    println!("Saved DDNA to {}", tmp.display());
    let metadata = std::fs::metadata(&tmp).unwrap();
    assert!(metadata.len() > 1000);
    println!("PNG size: {} bytes", metadata.len());
}
