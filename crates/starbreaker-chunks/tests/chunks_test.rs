use starbreaker_chunks::{ChunkFile, known_types};
use std::path::Path;

/// Helper: skip test gracefully when a file does not exist on disk.
fn read_if_exists(path: &str) -> Option<Vec<u8>> {
    if Path::new(path).exists() {
        Some(std::fs::read(path).expect("failed to read file"))
    } else {
        eprintln!("SKIP: file not found: {path}");
        None
    }
}

// These tests require extracted game data on disk and are ignored by default.
// Run with: cargo test -- --ignored

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_ivo_skin_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/male_v7/body/male_v7_body.skin",
    ) else {
        return;
    };

    let file = ChunkFile::from_bytes(&data).expect("failed to parse chunk file");

    match &file {
        ChunkFile::Ivo(ivo) => {
            assert!(!ivo.chunks().is_empty(), "expected at least one chunk");
            println!("IVO file: {} chunks", ivo.chunks().len());
            for chunk in ivo.chunks() {
                let name = known_types::ivo::name(chunk.chunk_type).unwrap_or("?");
                println!(
                    "  type=0x{:08X} ({}) ver={} offset={} size={}",
                    chunk.chunk_type, name, chunk.version, chunk.offset, chunk.size
                );
                // Verify chunk_data returns a slice of the expected size
                let slice = ivo.chunk_data(chunk);
                assert_eq!(slice.len(), chunk.size);
            }
        }
        _ => panic!("expected IVO format for .skin file"),
    }
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_soc_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/ObjectContainers/Frontend/CharacterCustomizer/charactercustomizer_pu/charactercustomizer_pu.soc",
    ) else {
        return;
    };

    let file = ChunkFile::from_bytes(&data).expect("failed to parse chunk file");

    match &file {
        ChunkFile::CrCh(crch) => {
            assert!(!crch.chunks().is_empty(), "expected at least one chunk");
            println!("CrCh file: {} chunks", crch.chunks().len());
            for chunk in crch.chunks() {
                let name = known_types::crch::name(chunk.chunk_type).unwrap_or("?");
                println!(
                    "  type=0x{:04X} ({}) ver={} id={} offset={} size={}",
                    chunk.chunk_type, name, chunk.version, chunk.id, chunk.offset, chunk.size
                );
                let slice = crch.chunk_data(chunk);
                assert_eq!(slice.len(), chunk.size as usize);
            }
        }
        ChunkFile::Ivo(ivo) => {
            // SOC might be IVO format too — just verify it parses
            println!("IVO file: {} chunks", ivo.chunks().len());
            for chunk in ivo.chunks() {
                let name = known_types::ivo::name(chunk.chunk_type).unwrap_or("?");
                println!(
                    "  type=0x{:08X} ({}) ver={} offset={} size={}",
                    chunk.chunk_type, name, chunk.version, chunk.offset, chunk.size
                );
            }
        }
    }
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_cgf_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/ObjectContainers/Frontend/CharacterCustomizer/charactercustomizer_pu/charactercustomizer_pu/brush/designer_0.cgf",
    ) else {
        return;
    };

    let file = ChunkFile::from_bytes(&data).expect("failed to parse chunk file");

    let format = match &file {
        ChunkFile::Ivo(_) => "IVO",
        ChunkFile::CrCh(_) => "CrCh",
    };
    println!("Format: {format}");
}

#[test]
fn reject_unknown_magic() {
    let data = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00];
    let err = ChunkFile::from_bytes(&data).unwrap_err();
    assert!(
        matches!(
            err,
            starbreaker_chunks::ChunkFileError::UnrecognizedMagic(0xEFBEADDE)
        ),
        "expected UnrecognizedMagic, got: {err:?}"
    );
}

#[test]
fn reject_truncated_input() {
    // Too short to even read a u32 magic
    let data = [0x23, 0x69];
    let err = ChunkFile::from_bytes(&data).unwrap_err();
    assert!(
        matches!(err, starbreaker_chunks::ChunkFileError::Parse(_)),
        "expected Parse error, got: {err:?}"
    );
}

#[test]
fn reject_bad_ivo_version() {
    // Valid IVO magic but wrong version
    let mut data = [0u8; 16];
    data[0..4].copy_from_slice(&IVO_MAGIC_BYTES);
    data[4..8].copy_from_slice(&0x901u32.to_le_bytes()); // bad version
    let err = ChunkFile::from_bytes(&data).unwrap_err();
    assert!(
        matches!(
            err,
            starbreaker_chunks::ChunkFileError::UnsupportedVersion(0x901)
        ),
        "expected UnsupportedVersion, got: {err:?}"
    );
}

const IVO_MAGIC_BYTES: [u8; 4] = 0x6F766923u32.to_le_bytes();

#[test]
fn parse_synthetic_ivo() {
    // Build a minimal valid IVO file in memory:
    // Header (16 bytes) + chunk table (1 entry = 16 bytes) + chunk data (8 bytes)
    let chunk_data: [u8; 8] = [0xCA, 0xFE, 0xBA, 0xBE, 0xDE, 0xAD, 0xC0, 0xDE];
    let chunk_data_offset: u64 = 32; // after header(16) + table(16)

    let mut buf = Vec::new();
    // Header
    buf.extend_from_slice(&0x6F766923u32.to_le_bytes()); // magic
    buf.extend_from_slice(&0x900u32.to_le_bytes()); // version
    buf.extend_from_slice(&1u32.to_le_bytes()); // chunk_count
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk_table_offset (right after header)
    // Chunk table entry
    buf.extend_from_slice(&0x92914444u32.to_le_bytes()); // chunk_type = MESH_IVO320
    buf.extend_from_slice(&320u32.to_le_bytes()); // version
    buf.extend_from_slice(&chunk_data_offset.to_le_bytes()); // offset
    // Chunk data
    buf.extend_from_slice(&chunk_data);

    let file = ChunkFile::from_bytes(&buf).expect("failed to parse synthetic IVO");
    match &file {
        ChunkFile::Ivo(ivo) => {
            assert_eq!(ivo.chunks().len(), 1);
            let c = &ivo.chunks()[0];
            assert_eq!(c.chunk_type, 0x92914444);
            assert_eq!(c.version, 320);
            assert_eq!(c.offset, chunk_data_offset);
            assert_eq!(c.size, 8);
            assert_eq!(ivo.chunk_data(c), &chunk_data);
        }
        _ => panic!("expected IVO format"),
    }
}

#[test]
fn parse_synthetic_crch() {
    // Build a minimal valid CrCh file in memory:
    // Header (16 bytes) + chunk table (1 entry = 16 bytes) + chunk data (4 bytes)
    let chunk_data: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
    let chunk_data_offset: u32 = 32; // after header(16) + table(16)

    let mut buf = Vec::new();
    // Header
    buf.extend_from_slice(&0x68437243u32.to_le_bytes()); // magic
    buf.extend_from_slice(&0x746u32.to_le_bytes()); // version
    buf.extend_from_slice(&1u32.to_le_bytes()); // chunk_count
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk_table_offset
    // Chunk table entry
    buf.extend_from_slice(&0x1000u16.to_le_bytes()); // chunk_type = MESH
    buf.extend_from_slice(&0x0801u16.to_le_bytes()); // version_raw = 0x0801 (version=0x0801, no big-endian flag)
    buf.extend_from_slice(&42i32.to_le_bytes()); // id
    buf.extend_from_slice(&4u32.to_le_bytes()); // size
    buf.extend_from_slice(&chunk_data_offset.to_le_bytes()); // offset
    // Chunk data
    buf.extend_from_slice(&chunk_data);

    let file = ChunkFile::from_bytes(&buf).expect("failed to parse synthetic CrCh");
    match &file {
        ChunkFile::CrCh(crch) => {
            assert_eq!(crch.chunks().len(), 1);
            let c = &crch.chunks()[0];
            assert_eq!(c.chunk_type, 0x1000);
            assert_eq!(c.version, 0x0801);
            assert!(!c.big_endian);
            assert_eq!(c.id, 42);
            assert_eq!(c.size, 4);
            assert_eq!(c.offset, chunk_data_offset);
            assert_eq!(crch.chunk_data(c), &chunk_data);
        }
        _ => panic!("expected CrCh format"),
    }
}
