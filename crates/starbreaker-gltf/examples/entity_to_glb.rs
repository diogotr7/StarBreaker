use std::env;

use starbreaker_datacore::database::Database;

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: entity_to_glb <entity_name> [output.glb] [options]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --no-textures     Skip texture embedding");
        eprintln!("  --mip N           Use texture mip level N (0=full, 2=1/4 res, 4=1/16 res)");
        eprintln!("  --lod N           Use LOD level N (0=highest detail, 1+=lower)");
        eprintln!("  --no-interior     Skip interior geometry from socpak containers");
        eprintln!();
        eprintln!("Example: entity_to_glb AEGS_Gladius gladius.glb --lod 1 --mip 2");
        std::process::exit(1);
    }

    // Parse args: positional (entity_name, output) and flags
    let mut positional = Vec::new();
    let mut opts = starbreaker_gltf::ExportOptions::default();
    let mut dump_hierarchy = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--no-textures" => opts.include_textures = false,
            "--no-normals" => opts.include_normals = false,
            "--no-interior" => opts.include_interior = false,
            "--no-materials" => opts.include_materials = false,
            "--dump-hierarchy" => dump_hierarchy = true,
            "--mip" => {
                i += 1;
                opts.texture_mip = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--lod" => {
                i += 1;
                opts.lod_level = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    let entity_name = positional.first().unwrap_or_else(|| {
        eprintln!("Missing entity name");
        std::process::exit(1);
    });
    let output = positional
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("{entity_name}.glb"));
    eprintln!(
        "Options: include_textures={}, mip={}, lod={}, interior={}",
        opts.include_textures, opts.texture_mip, opts.lod_level, opts.include_interior
    );

    // Load P4k (auto-discovers from SC_DATA_P4K env var or default install locations)
    eprintln!("Opening P4k...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    // Load DCB from inside the P4k
    eprintln!("Loading Game2.dcb from P4k...");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb from P4k");
    let db = Database::from_bytes(&dcb_data).expect("failed to parse DCB");

    // Find EntityClassDefinition records matching the name, try each until one has geometry
    let search = entity_name.to_lowercase();
    let entity_si = db
        .struct_id("EntityClassDefinition")
        .expect("EntityClassDefinition not found");
    let mut candidates: Vec<_> = db
        .records_of_type(entity_si)
        .filter(|record| {
            let name = db.resolve_string2(record.name_offset);
            name.to_lowercase().contains(&search)
        })
        .collect();

    if candidates.is_empty() {
        eprintln!("No EntityClassDefinition records matching '{entity_name}'");
        std::process::exit(1);
    }

    // Sort: prefer shorter names (more likely to be the main entity)
    candidates.sort_by_key(|r| db.resolve_string2(r.name_offset).len());

    eprintln!("Found {} candidates, trying each...", candidates.len());

    // Build entity index once (O(n) scan), reuse for all lookups
    let idx = starbreaker_datacore::loadout::EntityIndex::new(&db);

    let mut found = None;
    for record in &candidates {
        let name = db.resolve_string2(record.name_offset);
        let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&idx, record);

        // Dump loadout tree
        eprintln!("\nLoadout tree for {}:", tree.root.entity_name);
        for child in &tree.root.children {
            let geom_marker = if child.geometry_path.is_some() {
                "G"
            } else {
                "."
            };
            eprintln!(
                "  {geom_marker} {} -> {}",
                child.item_port_name, child.entity_name
            );
        }
        eprintln!();

        if dump_hierarchy {
            let json = starbreaker_gltf::dump_hierarchy(&db, &p4k, record, &tree);
            let json_output = output.replace(".glb", ".json");
            std::fs::write(&json_output, &json).expect("failed to write hierarchy JSON");
            eprintln!("Hierarchy written to {json_output}");
            return;
        }

        let export_result =
            starbreaker_gltf::assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts);
        match export_result {
            Ok(result) => {
                // Skip .cdf files — character definition, not direct geometry
                if result.geometry_path.ends_with(".cdf") {
                    eprintln!("  {name} -> skipped (.cdf character definition)");
                    continue;
                }
                eprintln!("  {name} -> OK ({} bytes)", result.glb.len());
                found = Some((*record, result));
                break;
            }
            Err(e) => {
                eprintln!("  {name} -> {e}");
            }
        }
    }

    if dump_hierarchy {
        eprintln!("No candidates with geometry found for hierarchy dump");
        std::process::exit(1);
    }

    let (record, result) = found.unwrap_or_else(|| {
        eprintln!("None of the candidates could be exported");
        std::process::exit(1);
    });
    let _ = record; // used for the search, result has everything

    // Write output
    eprintln!("Geometry: {}", result.geometry_path);
    eprintln!("Material: {}", result.material_path);
    eprintln!("GLB size: {} bytes", result.glb.len());
    std::fs::write(&output, &result.glb).expect("failed to write output");
    eprintln!("Written to {output}");
}
