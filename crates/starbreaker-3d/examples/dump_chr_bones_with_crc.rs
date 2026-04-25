fn crc32_zlib(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
        }
        table[i as usize] = c;
    }
    let mut c = 0xFFFFFFFFu32;
    for &b in bytes {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFFFFFF
}

fn main() {
    let p = std::env::args().nth(1).expect("path");
    let data = std::fs::read(&p).unwrap();
    let bones = starbreaker_3d::skeleton::parse_skeleton(&data).unwrap_or_default();
    println!("bones: {}", bones.len());
    for b in &bones {
        let crc = crc32_zlib(b.name.as_bytes());
        let crc_lc = crc32_zlib(b.name.to_lowercase().as_bytes());
        println!("0x{crc:08x} (lc 0x{crc_lc:08x})  {}", b.name);
    }
}

