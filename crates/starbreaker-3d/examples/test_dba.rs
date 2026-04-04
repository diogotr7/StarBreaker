//! Quick test: parse a .dba animation database from P4k.
//! Usage: test_dba [path_substring]
//! Default: Gladius.dba

fn main() {
    env_logger::init();
    let search = std::env::args().nth(1).unwrap_or_else(|| "AEGS\\Gladius.dba".into());
    let search_lower = search.to_lowercase();

    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    let entry = p4k.entries().iter()
        .find(|e| e.name.to_lowercase().contains(&search_lower) && e.name.to_lowercase().ends_with(".dba"))
        .expect("No .dba file found matching search");

    eprintln!("Reading: {} ({} bytes)", entry.name, entry.uncompressed_size);
    let data = p4k.read(entry).unwrap();

    match starbreaker_3d::animation::dba::parse_dba(&data) {
        Ok(db) => {
            println!("{} animation clips:\n", db.clips.len());
            for clip in &db.clips {
                let total_rot: usize = clip.channels.iter().map(|c| c.rotations.len()).sum();
                let total_pos: usize = clip.channels.iter().map(|c| c.positions.len()).sum();
                println!("  '{}': {} bones, {} rot keys, {} pos keys, {:.0} fps",
                    clip.name, clip.channels.len(), total_rot, total_pos, clip.fps);
                // Show first bone's first keyframe
                if let Some(ch) = clip.channels.first() {
                    if let Some(kf) = ch.rotations.first() {
                        println!("    bone 0x{:08X}: rot[0] t={:.1} q=[{:.3},{:.3},{:.3},{:.3}]",
                            ch.bone_hash, kf.time, kf.value[0], kf.value[1], kf.value[2], kf.value[3]);
                    }
                    if let Some(kf) = ch.positions.first() {
                        println!("    bone 0x{:08X}: pos[0] t={:.1} p=[{:.3},{:.3},{:.3}]",
                            ch.bone_hash, kf.time, kf.value[0], kf.value[1], kf.value[2]);
                    }
                }
            }
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}
