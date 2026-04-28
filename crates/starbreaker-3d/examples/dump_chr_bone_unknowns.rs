//! Phase 40 research probe: dump per-bone _unknown1/_unknown2/num_children/limb_id
//! from a CHR CompiledBones v901 chunk. Read-only; not part of the production parser.

use starbreaker_chunks::ChunkFile;
use starbreaker_chunks::known_types::ivo;
use starbreaker_common::reader::SpanReader;

fn main() {
    let p = std::env::args().nth(1).expect("path to .chr");
    let data = std::fs::read(&p).unwrap();
    let cf = ChunkFile::from_bytes(&data).expect("chunk file");
    let ivo_file = match &cf {
        ChunkFile::Ivo(i) => i,
        ChunkFile::CrCh(_) => panic!("CrCh not supported"),
    };
    for entry in ivo_file.chunks() {
        let chunk = ivo_file.chunk_data(entry);
        if entry.chunk_type == ivo::COMPILED_BONES {
            println!("# version=v901");
            dump_v901(chunk);
            return;
        }
        if entry.chunk_type == ivo::COMPILED_BONES_IVO320 {
            println!("# version=v900");
            dump_v900(chunk);
            return;
        }
    }
    eprintln!("no CompiledBones chunk found; chunk types present:");
    for entry in ivo_file.chunks() {
        eprintln!("  0x{:08x}", entry.chunk_type);
    }
}

fn dump_v900(data: &[u8]) {
    let mut r = SpanReader::new(data);
    let num_bones = r.read_u32().unwrap() as usize;
    println!("# v900 num_bones={num_bones}");
    let mut entries: Vec<(u32, u32, i32)> = Vec::with_capacity(num_bones);
    for _ in 0..num_bones {
        let controller_id = r.read_u32().unwrap();
        let limb_id = r.read_u32().unwrap();
        let parent_index = r.read_i32().unwrap();
        // skip relative+world transforms (28+28 = 56 bytes)
        r.advance(56).unwrap();
        entries.push((controller_id, limb_id, parent_index));
    }
    let names: Vec<String> = r.remaining_bytes()
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    println!("idx parent ctrlid       limb       name");
    for (i, (ctrl, limb, parent)) in entries.iter().enumerate() {
        let name = names.get(i).map(|s| s.as_str()).unwrap_or("<missing>");
        println!("{:3} {:5} 0x{:08x} 0x{:08x} {}", i, parent, ctrl, limb, name);
    }
}

fn dump_v901(data: &[u8]) {
    let mut r = SpanReader::new(data);
    let num_bones = r.read_u32().unwrap() as usize;
    let string_table_size = r.read_u32().unwrap() as usize;
    let flags1 = r.read_u32().unwrap();
    let flags2 = r.read_u32().unwrap();
    r.advance(32).unwrap();
    println!("# num_bones={num_bones} string_table_size={string_table_size} flags1=0x{flags1:08x} flags2=0x{flags2:08x}");

    // Read entries by hand so we can print every field.
    let mut entries: Vec<(u32, u16, u16, i16, i16, i16, i16)> = Vec::with_capacity(num_bones);
    for _ in 0..num_bones {
        let controller_id = r.read_u32().unwrap();
        let limb_id = r.read_u16().unwrap();
        let num_children = r.read_u16().unwrap();
        let parent_index = r.read_i16().unwrap();
        let unk1 = r.read_i16().unwrap();
        let unk2 = r.read_i16().unwrap();
        let object_node_index = r.read_i16().unwrap();
        entries.push((controller_id, limb_id, num_children, parent_index, unk1, unk2, object_node_index));
    }

    // String table.
    let string_bytes = r.read_bytes(string_table_size).unwrap();
    let names: Vec<String> = string_bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();

    println!("idx parent ctrlid       limb children unk1   unk2   nodeidx  name");
    for (i, (ctrl, limb, nc, parent, u1, u2, node)) in entries.iter().enumerate() {
        let name = names.get(i).map(|s| s.as_str()).unwrap_or("<missing>");
        println!(
            "{:3} {:5} 0x{:08x} {:4} {:4}     {:6} {:6} {:6}    {}",
            i, parent, ctrl, limb, nc, u1, u2, node, name
        );
    }
}
