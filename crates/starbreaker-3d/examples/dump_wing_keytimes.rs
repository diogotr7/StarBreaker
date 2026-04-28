// Phase 47: dump decoded keyframe times for the four Scorpius wing
// rotation channels in `wings_deploy.caf`. Lets us verify that the new
// per-frame keyframe-bitmap decoder produces synchronised key times for
// the diagonally-paired wings (BR↔TL, TR↔BL).

fn main() {
    let path = std::env::args().nth(1).expect("dba path");
    let clip_index: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(53);

    let bytes = std::fs::read(&path).expect("read dba");
    let db = starbreaker_3d::animation::parse_dba(&bytes).expect("parse_dba");
    let clip = &db.clips[clip_index];
    println!("clip[{clip_index}] = {} ({} channels)", clip.name, clip.channels.len());

    let wings = [
        "Wing_Mechanism_Bottom_Right",
        "Wing_Mechanism_Top_Right",
        "Wing_Mechanism_Bottom_Left",
        "Wing_Mechanism_Top_Left",
    ];
    for name in wings {
        let h = starbreaker_3d::animation::bone_name_hash(name);
        for ch in &clip.channels {
            if ch.bone_hash == h {
                let times: Vec<f32> = ch.rotations.iter().map(|k| k.time).collect();
                println!("\n{name} (hash 0x{h:08X}): {} keys", times.len());
                println!("  times: {:?}", times);
            }
        }
    }
}
