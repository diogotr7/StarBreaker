//! Search P4k for .mtl files with non-white Diffuse colors.
//! Prints the most colorful materials found.
use starbreaker_gltf::mtl;

fn main() {
    eprintln!("Opening P4k...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    let mtl_entries: Vec<_> = p4k
        .entries()
        .iter()
        .filter(|e| e.name.to_lowercase().ends_with(".mtl"))
        .take(5000) // sample first 5000
        .collect();

    eprintln!("Checking {} .mtl files...", mtl_entries.len());

    let mut colorful = Vec::new();

    for entry in &mtl_entries {
        let data = match p4k.read(entry) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mtl_file = match mtl::parse_mtl(&data) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for mat in &mtl_file.materials {
            if mat.is_nodraw {
                continue;
            }
            let [r, g, b] = mat.diffuse;
            // Look for materials that are NOT white/grey (have color variation)
            let is_grey = (r - g).abs() < 0.05 && (g - b).abs() < 0.05;
            let is_near_white = r > 0.9 && g > 0.9 && b > 0.9;
            if !is_grey && !is_near_white {
                let saturation = {
                    let max = r.max(g).max(b);
                    let min = r.min(g).min(b);
                    if max > 0.0 { (max - min) / max } else { 0.0 }
                };
                colorful.push((
                    saturation,
                    entry.name.clone(),
                    mat.name.clone(),
                    mat.diffuse,
                ));
            }
        }
    }

    colorful.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    println!("Top 20 most colorful materials:");
    for (sat, path, name, [r, g, b]) in colorful.iter().take(20) {
        println!("  sat={sat:.2} color=({r:.3},{g:.3},{b:.3}) name={name} file={path}");
    }
}
