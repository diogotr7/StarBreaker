//! Diagnostic: dump raw DBA metadata entries (count, num_controllers,
//! start_rotation) alongside the decoded first-frame rotation of every block.
//! Used to derive a robust metadata→block matching scheme.

use std::collections::HashMap;

use starbreaker_3d::animation::parse_dba;
use starbreaker_chunks::ChunkFile;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/sb_dba/Data/Animations/Spaceships/Ships/RSI/Scorpius.dba".into());
    let bytes = std::fs::read(&path).unwrap();

    // Dump metadata directly from the IVO chunk.
    let chunk_file = ChunkFile::from_bytes(&bytes).unwrap();
    let ivo = match &chunk_file {
        ChunkFile::Ivo(i) => i,
        _ => panic!("not an IVO file"),
    };
    let meta_chunk = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == 0xF7351608)
        .expect("no DBA metadata chunk");
    let m = ivo.chunk_data(meta_chunk);

    let count = u32::from_le_bytes(m[0..4].try_into().unwrap()) as usize;
    println!("metadata entries: {count}");
    let entry_size = 48;
    let entries_end = 4 + count * entry_size;

    let mut entries: Vec<(u16, u16, [f32; 4], u32, u32)> = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * entry_size;
        let flags0 = u32::from_le_bytes(m[o..o + 4].try_into().unwrap());
        let flags1 = u32::from_le_bytes(m[o + 4..o + 8].try_into().unwrap());
        let fps = u16::from_le_bytes(m[o + 8..o + 10].try_into().unwrap());
        let num_controllers = u16::from_le_bytes(m[o + 10..o + 12].try_into().unwrap());
        let sr = [
            f32::from_le_bytes(m[o + 24..o + 28].try_into().unwrap()),
            f32::from_le_bytes(m[o + 28..o + 32].try_into().unwrap()),
            f32::from_le_bytes(m[o + 32..o + 36].try_into().unwrap()),
            f32::from_le_bytes(m[o + 36..o + 40].try_into().unwrap()),
        ];
        entries.push((num_controllers, fps, sr, flags0, flags1));
    }

    let mut names: Vec<String> = Vec::with_capacity(count);
    let mut p = entries_end;
    for _ in 0..count {
        let end = m[p..]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(m.len() - p);
        names.push(String::from_utf8_lossy(&m[p..p + end]).into_owned());
        p += end + 1;
    }

    // Also parse blocks for bone_count + first-frame rotation.
    let db = parse_dba(&bytes).unwrap();
    println!("\n=== METADATA (raw, sorted by file order) ===");
    for (i, ((nc, fps, sr, f0, f1), name)) in entries.iter().zip(names.iter()).enumerate() {
        println!(
            "[{i:3}] f0=0x{f0:08X} f1=0x{f1:08X} nc={nc:3} fps={fps} sr=[{:+.4},{:+.4},{:+.4},{:+.4}] name={name}",
            sr[0], sr[1], sr[2], sr[3]
        );
    }

    println!("\n=== BLOCKS (file order) ===");
    for (i, c) in db.clips.iter().enumerate() {
        let first_rot = c
            .channels
            .iter()
            .find_map(|ch| ch.rotations.first().map(|kf| kf.value));
        println!(
            "[{i:3}] bones={:3}  first_rot={:?}",
            c.channels.len(),
            first_rot
        );
    }

    // Try to match metadata→blocks by num_controllers + nearest start_rotation.
    println!("\n=== MATCHED ===");
    let mut by_count: HashMap<u16, Vec<usize>> = HashMap::new();
    for (i, c) in db.clips.iter().enumerate() {
        by_count.entry(c.channels.len() as u16).or_default().push(i);
    }
    for (mi, ((nc, _fps, sr, _f0, _f1), name)) in entries.iter().zip(names.iter()).enumerate() {
        let candidates = by_count.get(nc).cloned().unwrap_or_default();
        let mut best: Option<(usize, f32)> = None;
        for bi in &candidates {
            let block = &db.clips[*bi];
            // Look at every channel's first quat; find min angular distance.
            let mut min_d = f32::INFINITY;
            for ch in &block.channels {
                if let Some(kf0) = ch.rotations.first() {
                    let q = kf0.value;
                    // Compare as raw 4-vector (sign-invariant).
                    let dot = (q[0] * sr[0] + q[1] * sr[1] + q[2] * sr[2] + q[3] * sr[3]).abs();
                    let d = 1.0 - dot.clamp(0.0, 1.0);
                    if d < min_d {
                        min_d = d;
                    }
                }
            }
            match best {
                None => best = Some((*bi, min_d)),
                Some((_, prev)) if min_d < prev => best = Some((*bi, min_d)),
                _ => {}
            }
        }
        println!(
            "meta[{mi:3}] nc={nc} -> block={:?} (cand={:?}) name={name}",
            best.map(|(b, d)| format!("{b} d={d:.4}")),
            candidates
        );
    }
}
