//! List all chunks in an IVO file from P4k.
use starbreaker_chunks::ChunkFile;
use std::env;

fn main() {
    let search = env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: list_chunks <p4k_path_substring>");
            std::process::exit(1);
        })
        .to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    let entry = p4k
        .entries()
        .iter()
        .find(|e| e.name.to_lowercase().ends_with(&search))
        .unwrap_or_else(|| {
            // Fallback to contains
            p4k.entries()
                .iter()
                .find(|e| e.name.to_lowercase().contains(&search))
                .expect("no match")
        });

    eprintln!("File: {} ({} bytes)", entry.name, entry.uncompressed_size);
    let data = p4k.read(entry).expect("extract failed");

    let chunk_file = ChunkFile::from_bytes(&data).expect("parse failed");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => {
            println!("CrCh format, not IVO");
            return;
        }
    };

    println!("Chunks ({}):", ivo.chunks().len());
    for entry in ivo.chunks() {
        let known = match entry.chunk_type {
            0xB8757777 => "IvoSkin2",
            0x83353333 => "MtlNameIvo320",
            0xE0181074 => "NodeMeshCombo",
            0xCCCC0002 => "CompiledBones",
            0xACDC0000 => "SourceInfo",
            0x746C6F63 => "Colors",
            _ => "unknown",
        };
        println!(
            "  type=0x{:08X} ({:16}) version={} offset={} size={}",
            entry.chunk_type, known, entry.version, entry.offset, entry.size
        );
    }
}
