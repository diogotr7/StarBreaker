//! Extract a .skinm/.cgfm file from Data.p4k by path and convert to GLB.
//! Usage: p4k_skin_to_glb <p4k_path_substring> [output.glb]

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: p4k_skin_to_glb <path_substring> [output.glb]");
        eprintln!();
        eprintln!("Searches P4k for .skinm/.cgfm files matching the substring.");
        eprintln!("Example: p4k_skin_to_glb gladius");
        std::process::exit(1);
    }

    let search = args[1].to_lowercase();

    eprintln!("Opening P4k...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    // Find matching .skinm or .cgfm files
    let matches: Vec<_> = p4k
        .entries()
        .iter()
        .filter(|e| {
            let name = e.name.to_lowercase();
            name.contains(&search) && (name.ends_with(".skinm") || name.ends_with(".cgfm"))
        })
        .collect();

    if matches.is_empty() {
        eprintln!("No .skinm/.cgfm files matching '{}'", args[1]);
        std::process::exit(1);
    }

    eprintln!("Found {} matching files:", matches.len());
    for (i, entry) in matches.iter().enumerate().take(20) {
        eprintln!("  [{i}] {} ({} bytes)", entry.name, entry.uncompressed_size);
    }
    if matches.len() > 20 {
        eprintln!("  ... and {} more", matches.len() - 20);
    }

    // Use the first match (or a specific one if there's a better heuristic)
    let entry = matches[0];
    let output = if args.len() >= 3 {
        args[2].clone()
    } else {
        // Derive output name from the P4k entry name
        let base = entry.name.rsplit('\\').next().unwrap_or(&entry.name);
        base.replace(".skinm", ".glb").replace(".cgfm", ".glb")
    };

    eprintln!("Extracting: {} ...", entry.name);
    let data = p4k.read(entry).expect("failed to extract from P4k");
    eprintln!("Extracted {} bytes, converting to GLB...", data.len());

    match starbreaker_3d::skin_to_glb(&data) {
        Ok(glb) => {
            std::fs::write(&output, &glb).expect("failed to write output");
            eprintln!("Written {} bytes to {output}", glb.len());
        }
        Err(e) => {
            eprintln!("Error converting: {e}");
            std::process::exit(1);
        }
    }
}
