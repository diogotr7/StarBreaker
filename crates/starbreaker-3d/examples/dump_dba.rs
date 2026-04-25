use starbreaker_3d::animation::parse_dba;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/sb_dba/Data/Animations/Spaceships/Ships/RSI/Scorpius.dba".into());
    let data = std::fs::read(&path).unwrap();
    let db = parse_dba(&data).unwrap();
    println!("clips: {}", db.clips.len());
    let needle = std::env::args().nth(2);
    for (i, c) in db.clips.iter().enumerate() {
        let want_hash: Option<u32> = needle
            .as_ref()
            .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok());
        let mut hash_match = false;
        if let Some(h) = want_hash {
            hash_match = c.channels.iter().any(|ch| ch.bone_hash == h);
        }
        let name_match = needle
            .as_ref()
            .map(|n| c.name.contains(n.as_str()))
            .unwrap_or(true);
        let show = name_match || hash_match;
        if !show {
            continue;
        }
        println!(
            "[{i:3}] channels={:3} fps={:5.1} {}",
            c.channels.len(),
            c.fps,
            c.name
        );
        if needle.is_some() {
            for ch in &c.channels {
                println!(
                    "    bone=0x{:08X}  rot={:3}  pos={:3}",
                    ch.bone_hash,
                    ch.rotations.len(),
                    ch.positions.len()
                );
            }
        }
    }
}
