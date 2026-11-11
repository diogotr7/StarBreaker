//! Tiny synthetic DDS-in-ZIP fixture for decomposed container tests.
use starbreaker_p4k::MappedP4k;

fn put32(bytes: &mut [u8], offset: usize, v: u32) {
    bytes[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
}
fn put16(bytes: &mut [u8], offset: usize, v: u16) {
    bytes[offset..offset + 2].copy_from_slice(&v.to_le_bytes());
}
fn synthetic_dds(format: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 148];
    bytes[..4].copy_from_slice(b"DDS ");
    for (o, v) in [
        (4, 124),
        (8, 0xA1007),
        (12, 4),
        (16, 4),
        (20, payload.len() as u32),
        (28, 1),
        (76, 32),
        (80, 4),
        (128, format),
        (132, 3),
        (140, 1),
    ] {
        put32(&mut bytes, o, v);
    }
    bytes[84..88].copy_from_slice(b"DX10");
    bytes.extend_from_slice(payload);
    bytes
}
pub(super) struct SyntheticArchive {
    path: std::path::PathBuf,
    pub(super) p4k: MappedP4k,
}
impl Drop for SyntheticArchive {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
pub(super) fn archive() -> SyntheticArchive {
    archive_with_extra(Vec::new())
}
pub(super) fn archive_with_extra(extra: Vec<(&str, Vec<u8>)>) -> SyntheticArchive {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "wp-t1-{}-{}.p4k",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let color = synthetic_dds(
        84,
        &[
            0x81, 0x80, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x40, 0x10, 0, 0, 0, 0, 0, 0,
        ],
    );
    let alpha = synthetic_dds(80, &[200, 100, 0, 0, 0, 0, 0, 0]);
    let mut fallback = color.clone();
    put32(&mut fallback, 36, 0x800);
    put32(&mut fallback, 124, u32::from_le_bytes(*b"FYRC"));
    let mut entries = vec![
        ("Data\\Test\\synthetic_ddna.dds", color),
        ("Data\\Test\\synthetic_ddna.dds.a", alpha),
        ("Data\\Test\\fallback_ddna.dds", fallback),
        ("Data\\Test\\fallback_ddna.dds.a", vec![0; 9]),
    ];
    for (name, width, height) in [
        ("Data\\Test\\small1_ddna.dds", 1, 1),
        ("Data\\Test\\small2_ddna.dds", 2, 2),
        ("Data\\Test\\odd_ddna.dds", 5, 4),
    ] {
        let mut data = synthetic_dds(
            83,
            &vec![0; starbreaker_dds::package::mip_byte_size(83, width, height).unwrap()],
        );
        put32(&mut data, 12, height);
        put32(&mut data, 16, width);
        entries.push((name, data));
    }
    entries.extend(extra);
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for (name, data) in &entries {
        let offset = bytes.len() as u32;
        let crc = crc32fast::hash(data);
        let mut local = vec![0; 30];
        put32(&mut local, 0, 0x04034b50);
        put16(&mut local, 4, 20);
        put32(&mut local, 14, crc);
        put32(&mut local, 18, data.len() as u32);
        put32(&mut local, 22, data.len() as u32);
        put16(&mut local, 26, name.len() as u16);
        bytes.extend(local);
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend(data);
        let mut dir = vec![0; 46];
        put32(&mut dir, 0, 0x02014b50);
        put16(&mut dir, 4, 20);
        put16(&mut dir, 6, 20);
        put32(&mut dir, 16, crc);
        put32(&mut dir, 20, data.len() as u32);
        put32(&mut dir, 24, data.len() as u32);
        put16(&mut dir, 28, name.len() as u16);
        put32(&mut dir, 42, offset);
        central.extend(dir);
        central.extend_from_slice(name.as_bytes());
    }
    let mut end = vec![0; 22];
    put32(&mut end, 0, 0x06054b50);
    put16(&mut end, 8, entries.len() as u16);
    put16(&mut end, 10, entries.len() as u16);
    put32(&mut end, 12, central.len() as u32);
    put32(&mut end, 16, bytes.len() as u32);
    bytes.extend(central);
    bytes.extend(end);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    drop(file);
    let p4k = MappedP4k::open(&path).unwrap();
    SyntheticArchive { path, p4k }
}
