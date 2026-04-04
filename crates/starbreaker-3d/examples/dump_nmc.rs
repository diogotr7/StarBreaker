//! Dump NMC node names and types from a CGA file inside P4k.
use starbreaker_chunks::ChunkFile;
use starbreaker_common::reader::SpanReader;

const NMC_FULL: u32 = 0x70697FDA;

fn main() {
    let search = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: dump_nmc <p4k_path_substring>");
            std::process::exit(1);
        })
        .to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    // Find matching .cga file
    let entry = p4k
        .entries()
        .iter()
        .find(|e| {
            let name = e.name.to_lowercase();
            (name.ends_with(".cga") || name.ends_with(".cgf")) && !name.contains("_lod") && {
                let fname = name.rsplit(['/', '\\']).next().unwrap_or(&name);
                fname.to_lowercase().contains(&search)
            }
        })
        .unwrap_or_else(|| {
            eprintln!("No .cga matching '{search}'");
            std::process::exit(1);
        });

    eprintln!("File: {}", entry.name);
    let data = p4k.read_file(&entry.name).expect("failed to read");

    let chunk_file = ChunkFile::from_bytes(&data).expect("invalid chunk file");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => {
            eprintln!("CrCh format — NMC not available");
            std::process::exit(1);
        }
    };

    let nmc_entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == NMC_FULL)
        .expect("no NMC chunk");
    let nmc_data = ivo.chunk_data(nmc_entry);
    let mut r = SpanReader::new(nmc_data);

    // Header
    let _h0 = r.read_i32().unwrap();
    let num_nodes = r.read_i32().unwrap() as usize;
    let _num_geom = r.read_i32().unwrap();
    let num_unk = r.read_i32().unwrap() as usize;
    let _num_mat = r.read_i32().unwrap() as usize;
    let str_size = r.read_i32().unwrap() as usize;
    let _h6 = r.read_i32().unwrap();
    let _h7 = r.read_i32().unwrap();

    eprintln!("Nodes: {num_nodes} (geom: {_num_geom})");

    // Read per-node: parent_index + geometry_type + full BoneToWorld matrix
    let mut node_info: Vec<(u16, u16, [[f32; 4]; 3])> = Vec::new();
    for _ in 0..num_nodes {
        r.advance(32).unwrap(); // pre-matrix
        r.advance(48).unwrap(); // WorldToBone
        // BoneToWorld: 3x4 row-major matrix
        let mut b2w = [[0.0f32; 4]; 3];
        for row in &mut b2w {
            for val in row.iter_mut() {
                *val = r.read_f32().unwrap();
            }
        }

        r.advance(20).unwrap(); // Scale + Id + Unknown
        let parent_index = r.read_u16().unwrap();
        let geometry_type = r.read_u16().unwrap();
        r.advance(56).unwrap();

        node_info.push((parent_index, geometry_type, b2w));
    }

    // Footer + indices
    r.advance(32).unwrap();
    for _ in 0..num_unk {
        r.read_u16().unwrap();
    }
    // Skip mat indices
    // ...we just need the string table
    // Actually we need to skip mat indices too
    let _num_mat = _num_mat;
    for _ in 0.._num_mat {
        r.read_u16().unwrap();
    }

    // String table
    let str_bytes = r.read_bytes(str_size).unwrap();
    let mut names = Vec::new();
    let mut pos = 0;
    for _ in 0..num_nodes {
        if pos >= str_bytes.len() {
            names.push(String::new());
            continue;
        }
        let end = str_bytes[pos..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| pos + p)
            .unwrap_or(str_bytes.len());
        let full = std::str::from_utf8(&str_bytes[pos..end]).unwrap_or("");
        names.push(full.lines().next().unwrap_or("").to_string());
        pos = end + 1;
    }

    // Print
    for (i, name) in names.iter().enumerate() {
        let (parent, geom_type, b2w) = node_info[i];
        let type_str = match geom_type {
            0 => "GEOM",
            2 => "HELP2",
            3 => "HELP3",
            _ => "UNK",
        };
        let parent_str = if parent == 0xFFFF {
            "root".to_string()
        } else {
            format!("{parent}")
        };
        // Check if rotation is identity
        let is_identity_rot = (b2w[0][0] - 1.0).abs() < 0.001
            && (b2w[1][1] - 1.0).abs() < 0.001
            && (b2w[2][2] - 1.0).abs() < 0.001
            && b2w[0][1].abs() < 0.001 && b2w[0][2].abs() < 0.001
            && b2w[1][0].abs() < 0.001 && b2w[1][2].abs() < 0.001
            && b2w[2][0].abs() < 0.001 && b2w[2][1].abs() < 0.001;
        let rot_flag = if is_identity_rot { "" } else { " ROT" };
        println!(
            "{i:4} {type_str:5} parent={parent_str:>5} pos=[{:.2},{:.2},{:.2}]{rot_flag} {name}",
            b2w[0][3], b2w[1][3], b2w[2][3]
        );
        if !is_identity_rot {
            println!("       [{:.3},{:.3},{:.3}]", b2w[0][0], b2w[0][1], b2w[0][2]);
            println!("       [{:.3},{:.3},{:.3}]", b2w[1][0], b2w[1][1], b2w[1][2]);
            println!("       [{:.3},{:.3},{:.3}]", b2w[2][0], b2w[2][1], b2w[2][2]);
        }
    }
}
