use starbreaker_p4k::{MappedP4k, P4kArchive};

fn fixture() -> Vec<u8> {
    let mut bytes = b"armor".to_vec();
    let mut entry = vec![0u8; 204];
    entry[10..18].copy_from_slice(&5u64.to_le_bytes());
    entry[18..26].copy_from_slice(&5u64.to_le_bytes());
    bytes.extend(entry);
    let names = b"Data/Objects/test.skin\0";
    bytes.extend(names);
    let mut footer = vec![0u8; 175];
    footer[0..8].copy_from_slice(&1u64.to_le_bytes());
    footer[16..24].copy_from_slice(&5u64.to_le_bytes());
    footer[24..32].copy_from_slice(&204u64.to_le_bytes());
    footer[40..48].copy_from_slice(&209u64.to_le_bytes());
    footer[48..56].copy_from_slice(&(names.len() as u64).to_le_bytes());
    footer[169..171].copy_from_slice(&2u16.to_le_bytes());
    footer[171..175].copy_from_slice(&0x696A694Au32.to_le_bytes());
    bytes.extend(footer);
    bytes
}

#[test]
fn v2_reads_direct_payload_in_memory_and_from_file() {
    let bytes = fixture();
    let archive = P4kArchive::from_bytes(&bytes).unwrap();
    assert_eq!(archive.entries()[0].name, "Data\\Objects\\test.skin");
    assert_eq!(archive.read(&archive.entries()[0]).unwrap(), b"armor");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("starbreaker-v2-{}-{stamp}.p4k", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let archive = MappedP4k::open(&path).unwrap();
    assert_eq!(archive.read(&archive.entries()[0]).unwrap(), b"armor");
    drop(archive);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn v2_rejects_invalid_names_and_directory_ranges() {
    let valid = fixture();
    for (offset, value) in [
        (5 + 34, u64::MAX),
        (valid.len() - 175 + 16, u64::MAX),
        (valid.len() - 175, u64::MAX),
    ] {
        let mut bytes = valid.clone();
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        assert!(P4kArchive::from_bytes(&bytes).is_err());
    }
    let mut bytes = valid;
    let terminator = bytes.len() - 176;
    bytes[terminator] = b'x';
    assert!(P4kArchive::from_bytes(&bytes).is_err());
}
