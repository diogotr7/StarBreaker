//! Dump skeleton bone names from a CGA file inside P4k.
fn main() {
    let search = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            eprintln!("Usage: dump_bones <p4k_path_substring>");
            std::process::exit(1);
        })
        .to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");

    let entry = p4k
        .entries()
        .iter()
        .find(|e| {
            let name = e.name.to_lowercase();
            name.ends_with(".cga") && !name.contains("_lod") && {
                let fname = name.rsplit(['/', '\\']).next().unwrap_or(&name);
                fname.contains(&search)
            }
        })
        .unwrap_or_else(|| {
            eprintln!("No .cga matching '{search}'");
            std::process::exit(1);
        });

    eprintln!("File: {}", entry.name);
    let data = p4k.read_file(&entry.name).expect("failed to read");

    let bones = starbreaker_gltf::skeleton::parse_skeleton(&data).unwrap_or_default();
    eprintln!("Bones: {}", bones.len());
    for (i, bone) in bones.iter().enumerate() {
        println!(
            "{i:4} pos=[{:.2},{:.2},{:.2}] {}",
            bone.world_position[0], bone.world_position[1], bone.world_position[2],
            bone.name
        );
    }
}
