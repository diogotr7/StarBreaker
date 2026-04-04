//! Dump a mesh as Wavefront OBJ for debugging.
use std::env;
use std::io::Write;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: dump_obj <p4k_path_substring> [output.obj]");
        std::process::exit(1);
    }

    let search = args[1].to_lowercase();
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    let entry = p4k
        .entries()
        .iter()
        .find(|e| {
            let name = e.name.to_lowercase();
            name.contains(&search) && (name.ends_with(".skinm") || name.ends_with(".cgfm"))
        })
        .expect("no match");

    eprintln!("File: {}", entry.name);
    let data = p4k.read(entry).expect("extract failed");
    let mesh = starbreaker_3d::parse_skin(&data).expect("parse failed");
    eprintln!("model_min: {:?}", mesh.model_min);
    eprintln!("model_max: {:?}", mesh.model_max);
    eprintln!("scaling_min: {:?}", mesh.scaling_min);
    eprintln!("scaling_max: {:?}", mesh.scaling_max);
    eprintln!("submeshes: {}", mesh.submeshes.len());
    for (i, s) in mesh.submeshes.iter().enumerate() {
        eprintln!("  sub[{i}]: node={} first_idx={} num_idx={} mat={}", s.node_parent_index, s.first_index, s.num_indices, s.material_id);
    }

    let output = if args.len() >= 3 {
        args[2].clone()
    } else {
        "debug.obj".into()
    };
    let mut f = std::fs::File::create(&output).expect("create failed");

    writeln!(
        f,
        "# {} vertices, {} indices",
        mesh.positions.len(),
        mesh.indices.len()
    )
    .unwrap();
    for p in &mesh.positions {
        writeln!(f, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]).unwrap();
    }
    for tri in mesh.indices.chunks(3) {
        // OBJ indices are 1-based
        writeln!(f, "f {} {} {}", tri[0] + 1, tri[1] + 1, tri[2] + 1).unwrap();
    }

    eprintln!("Written to {output}");
}
