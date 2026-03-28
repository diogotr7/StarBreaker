//! Dump parsed MTL materials from a P4k file.
fn main() {
    let search = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: dump_mtl <p4k_path_substring>");
        std::process::exit(1);
    }).to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let entry = p4k.entries().iter().find(|e| {
        e.name.to_lowercase().contains(&search) && e.name.to_lowercase().ends_with(".mtl")
    }).unwrap_or_else(|| {
        eprintln!("No .mtl matching '{search}'");
        std::process::exit(1);
    });

    eprintln!("File: {}", entry.name);
    let data = p4k.read_file(&entry.name).expect("failed to read");
    let mtl = starbreaker_gltf::mtl::parse_mtl(&data).expect("failed to parse");

    println!("Materials: {}", mtl.materials.len());
    for (i, m) in mtl.materials.iter().enumerate() {
        println!("\n[{i}] name={}", m.name);
        println!("  shader={}", m.shader);
        println!("  diffuse=[{:.3},{:.3},{:.3}]", m.diffuse[0], m.diffuse[1], m.diffuse[2]);
        println!("  specular=[{:.3},{:.3},{:.3}]", m.specular[0], m.specular[1], m.specular[2]);
        println!("  shininess={:.1} opacity={:.2} alpha_test={:.2}", m.shininess, m.opacity, m.alpha_test);
        println!("  is_nodraw={} palette_tint={}", m.is_nodraw, m.palette_tint);
        println!("  surface_type={}", m.surface_type);
        println!("  string_gen_mask={}", m.string_gen_mask);
        if let Some(ref t) = m.diffuse_tex { println!("  diffuse_tex={t}"); }
        if let Some(ref t) = m.normal_tex { println!("  normal_tex={t}"); }
        for (li, layer) in m.layers.iter().enumerate() {
            println!("  layer[{li}] path={} tint=[{:.3},{:.3},{:.3}] palette={} uv_tiling={:.1}",
                layer.path, layer.tint_color[0], layer.tint_color[1], layer.tint_color[2],
                layer.palette_tint, layer.uv_tiling);
        }
    }
}
