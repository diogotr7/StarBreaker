//! Export one or more socpak files directly to GLB (no DataCore entity needed).

use std::env;

use starbreaker_datacore::database::Database;

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: socpak_to_glb <search_pattern> [output.glb] [--no-textures] [--mip N] [--lod N]"
        );
        eprintln!();
        eprintln!(
            "Finds all .socpak files matching the pattern in the P4k and exports them as a single GLB."
        );
        eprintln!("Example: socpak_to_glb grimhex grimhex.glb --no-textures --lod 1");
        std::process::exit(1);
    }

    let mut positional = Vec::new();
    let mut opts = starbreaker_gltf::ExportOptions {
        include_textures: false,
        ..Default::default()
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--no-textures" => opts.include_textures = false,
            "--mip" => {
                i += 1;
                opts.texture_mip = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(2);
            }
            "--lod" => {
                i += 1;
                opts.lod_level = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    let search = positional.first().expect("Missing search pattern");
    let output = positional
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("{search}.glb"));
    let search_lower = search.to_lowercase();

    // Load P4k (auto-discovers from SC_DATA_P4K env var or default install locations)
    eprintln!("Opening P4k...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    // Load DCB for GUID resolution
    eprintln!("Loading Game2.dcb from P4k...");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).expect("failed to parse DCB");

    // Find matching socpaks
    let socpak_entries: Vec<_> = p4k
        .entries()
        .iter()
        .filter(|e| {
            let name_lower = e.name.to_lowercase();
            name_lower.contains(&search_lower) && name_lower.ends_with(".socpak")
        })
        .collect();

    if socpak_entries.is_empty() {
        eprintln!("No socpak files matching '{search}'");
        std::process::exit(1);
    }

    eprintln!(
        "Found {} socpak files matching '{search}':",
        socpak_entries.len()
    );
    for e in &socpak_entries {
        eprintln!("  {} ({} bytes)", e.name, e.uncompressed_size);
    }

    // Export using the public API
    let socpak_paths: Vec<String> = socpak_entries
        .iter()
        .map(|e| {
            // Strip "Data\" prefix to match the format expected by load_interior_from_socpak
            e.name.strip_prefix("Data\\").unwrap_or(&e.name).to_string()
        })
        .collect();

    match starbreaker_gltf::socpaks_to_glb(&db, &p4k, &socpak_paths, &opts) {
        Ok(glb) => {
            eprintln!("GLB size: {} bytes", glb.len());
            std::fs::write(&output, &glb).expect("failed to write output");
            eprintln!("Written to {output}");
        }
        Err(e) => {
            eprintln!("Export failed: {e}");
            std::process::exit(1);
        }
    }
}
