use starbreaker_chunks::ChunkFile;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let input = args.get(1).expect("Usage: inspect <file>");
    let data = std::fs::read(input).expect("failed to read");
    println!("file: {} ({} bytes)", input, data.len());

    match ChunkFile::from_bytes(&data) {
        Ok(ChunkFile::Ivo(ivo)) => {
            println!("format: IVO ({} chunks)", ivo.chunks().len());
            for entry in ivo.chunks() {
                println!(
                    "  type=0x{:08X} ver=0x{:X} offset={} size={}",
                    entry.chunk_type, entry.version, entry.offset, entry.size
                );
            }
        }
        Ok(ChunkFile::CrCh(crch)) => {
            println!("format: CrCh ({} chunks)", crch.chunks().len());
            for entry in crch.chunks() {
                println!(
                    "  type=0x{:04X} ver={} id={} offset={} size={}",
                    entry.chunk_type, entry.version, entry.id, entry.offset, entry.size
                );
            }
        }
        Err(e) => eprintln!("error: {e}"),
    }
}
