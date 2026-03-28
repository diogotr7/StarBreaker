//! Dump strings found in a specific chunk of a CGA file.
use starbreaker_chunks::ChunkFile;

fn main() {
    let search = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: dump_chunk_strings <p4k_path_substring> [chunk_type_hex]");
        std::process::exit(1);
    }).to_lowercase();
    let chunk_type: u32 = std::env::args().nth(2)
        .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x58DE1772);

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let entry = p4k.entries().iter().find(|e| {
        let name = e.name.to_lowercase();
        name.ends_with(".cga") && !name.contains("_lod") && {
            let fname = name.rsplit(['/', '\\']).next().unwrap_or(&name);
            fname.contains(&search)
        }
    }).unwrap_or_else(|| {
        eprintln!("No .cga matching '{search}'");
        std::process::exit(1);
    });

    eprintln!("File: {}", entry.name);
    let data = p4k.read_file(&entry.name).expect("failed to read");

    let chunk_file = ChunkFile::from_bytes(&data).expect("invalid chunk file");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => { eprintln!("CrCh format"); std::process::exit(1); }
    };

    let target = ivo.chunks().iter().find(|c| c.chunk_type == chunk_type).unwrap_or_else(|| {
        eprintln!("No chunk 0x{chunk_type:08X}");
        std::process::exit(1);
    });

    let chunk_data = ivo.chunk_data(target);
    eprintln!("Chunk 0x{:08X}: {} bytes", chunk_type, chunk_data.len());

    // Print first 256 bytes as hex dump
    eprintln!("\nFirst 256 bytes:");
    for (i, chunk) in chunk_data[..chunk_data.len().min(256)].chunks(16).enumerate() {
        eprint!("  {:04x}: ", i * 16);
        for b in chunk { eprint!("{:02x} ", b); }
        for _ in chunk.len()..16 { eprint!("   "); }
        eprint!(" |");
        for b in chunk {
            let c = *b as char;
            if c.is_ascii_graphic() || c == ' ' { eprint!("{c}"); }
            else { eprint!("."); }
        }
        eprintln!("|");
    }

    // Extract ASCII strings (min length 6)
    eprintln!("\nStrings (min 6 chars):");
    let mut start = None;
    let mut count = 0;
    for (i, &b) in chunk_data.iter().enumerate() {
        if b.is_ascii_graphic() || b == b' ' || b == b'_' || b == b'/' || b == b'\\' {
            if start.is_none() { start = Some(i); }
        } else {
            if let Some(s) = start {
                let len = i - s;
                if len >= 6 {
                    let text = std::str::from_utf8(&chunk_data[s..i]).unwrap_or("???");
                    println!("  @{s:>8} ({len:>4} bytes): {text}");
                    count += 1;
                }
            }
            start = None;
        }
    }
    eprintln!("\nTotal strings: {count}");
}
