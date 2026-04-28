// Diagnostic dumper for the time-stream bytes of a DBA clip's rotation
// channels. Phase 47 was opened after in-game footage of the Scorpius
// wings_deploy showed perfectly symmetric wing motion, but our parser
// produces asymmetric stagger because it stretches each channel's keys
// uniformly across [start..end] regardless of key count. We need to
// inspect the actual time-format bytes to understand what `0x42` really
// encodes.
//
// Usage:
//     dump_dba_time_stream <dba_path> <clip_index> [hash1 hash2 ...]
//
// If no hashes are given, all channels are dumped.

use std::env;

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: <dba_path> <clip_index> [hash1 ...]");
    let clip_index: usize = args.next().expect("clip_index").parse().unwrap();
    let want_hashes: Vec<u32> = args
        .map(|s| {
            let s = s.trim();
            if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u32::from_str_radix(stripped, 16).expect("hex hash")
            } else {
                s.parse().expect("decimal hash")
            }
        })
        .collect();

    let bytes = std::fs::read(&path).expect("read dba");
    let db = starbreaker_3d::animation::parse_dba(&bytes).expect("parse_dba");
    let clip = &db.clips[clip_index];
    println!(
        "clip[{clip_index}] = {} ({} channels, fps={})",
        clip.name, clip.channels.len(), clip.fps
    );

    use starbreaker_chunks::ChunkFile;
    const DBA_DATA: u32 = 0x194FBC50;
    let chunk_file = ChunkFile::from_bytes(&bytes).expect("ChunkFile");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        _ => panic!("expected IVO"),
    };
    let db_chunk = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == DBA_DATA)
        .expect("DBA_DATA chunk");
    let data: &[u8] = &ivo.file_data()[db_chunk.offset as usize..];

    // Walk blocks until the wanted clip.
    let mut offset = 0usize;
    if data.len() >= 4 {
        let total = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if total > 0 && total <= data.len() {
            offset = 4;
        }
    }
    let mut block_idx = 0usize;
    while offset + 12 <= data.len() {
        let sig = &data[offset..offset + 4];
        if sig != b"#caf" && sig != b"#dba" {
            break;
        }
        let bone_count = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        let block_start = offset + 12;
        let hash_table = block_start;
        let ctrl_table = hash_table + bone_count * 4;
        let headers_end = ctrl_table + bone_count * 24;
        if block_idx == clip_index {
            for i in 0..bone_count {
                let h_off = hash_table + i * 4;
                let bone_hash = u32::from_le_bytes([
                    data[h_off], data[h_off + 1], data[h_off + 2], data[h_off + 3],
                ]);
                if !want_hashes.is_empty() && !want_hashes.contains(&bone_hash) {
                    continue;
                }
                let c_off = ctrl_table + i * 24;
                let num_rot = u16::from_le_bytes([data[c_off], data[c_off + 1]]);
                let rot_flags = u16::from_le_bytes([data[c_off + 2], data[c_off + 3]]);
                let rot_t_off = u32::from_le_bytes([
                    data[c_off + 4], data[c_off + 5], data[c_off + 6], data[c_off + 7],
                ]);
                let abs_t = c_off + rot_t_off as usize;
                let time_format = (rot_flags & 0x0F) as u8;
                println!(
                    "\n=== hash=0x{:08X} num_rot={} rot_flags=0x{:04X} time_format=0x{:02X} ctrl_base=0x{:x} rot_t_off=0x{:x} abs_t=0x{:x} ===",
                    bone_hash, num_rot, rot_flags, time_format, c_off, rot_t_off, abs_t,
                );
                if rot_t_off == 0 {
                    println!("  (no time stream — synthesized as 0..N)");
                    continue;
                }
                // Print start/end/marker if the format suggests an 8-byte header.
                if time_format == 0x02 || time_format == 0x42 {
                    let start = u16::from_le_bytes([data[abs_t], data[abs_t + 1]]);
                    let end = u16::from_le_bytes([data[abs_t + 2], data[abs_t + 3]]);
                    let m_lo = u16::from_le_bytes([data[abs_t + 4], data[abs_t + 5]]);
                    let m_hi = u16::from_le_bytes([data[abs_t + 6], data[abs_t + 7]]);
                    let marker = u32::from_le_bytes([
                        data[abs_t + 4], data[abs_t + 5], data[abs_t + 6], data[abs_t + 7],
                    ]);
                    println!(
                        "  header: start_u16={start} end_u16={end} marker_u32=0x{marker:08x} (lo={m_lo} hi={m_hi})"
                    );
                    // Dump the next few bytes after the header in case there's
                    // a per-key sub-table.
                    let dump_len = 64.min(data.len().saturating_sub(abs_t + 8));
                    print!("  bytes after header (hex): ");
                    for k in 0..dump_len {
                        print!("{:02x}{}", data[abs_t + 8 + k], if k % 2 == 1 { " " } else { "" });
                    }
                    println!();
                    // If `marker` looks like an offset (small + < data.len()),
                    // try following it from the controller base.
                    if marker > 0 && (marker as usize) < data.len() {
                        let abs_m = c_off + marker as usize;
                        if abs_m < data.len() {
                            let dump_len2 = 64.min(data.len() - abs_m);
                            print!("  bytes at ctrl_base+marker (0x{abs_m:x}): ");
                            for k in 0..dump_len2 {
                                print!("{:02x}{}", data[abs_m + k], if k % 2 == 1 { " " } else { "" });
                            }
                            println!();
                        }
                    }
                }
                // Format 0x00 (1 byte/key) and 0x01 (2 bytes/key) — dump the key bytes.
                if time_format == 0x00 {
                    let dump_len = (num_rot as usize).min(64);
                    print!("  keys (u8): ");
                    for k in 0..dump_len {
                        print!("{} ", data[abs_t + k]);
                    }
                    println!();
                }
                if time_format == 0x01 {
                    let dump_len = (num_rot as usize).min(32);
                    print!("  keys (u16): ");
                    for k in 0..dump_len {
                        let t = u16::from_le_bytes([data[abs_t + k * 2], data[abs_t + k * 2 + 1]]);
                        print!("{} ", t);
                    }
                    println!();
                }
            }
            return;
        }
        block_idx += 1;
        offset = headers_end;
    }
    eprintln!("clip_index {clip_index} not found");
}
