// Diagnostic dumper for SNORM-packed position channels in a DBA clip.
// Walks a single animation block manually so we can inspect:
//   - per-controller `pos_data_offset` (raw and resolved)
//   - the 24-byte SNORM header (scale Vec3 + offset Vec3)
//   - the active-axis bitmap derived from the FLT_MAX sentinel
//   - the raw bytes covered by the keyframe blob
//
// Usage:
//     dump_dba_packed_pos <dba_path> <clip_index> <hash1> [hash2 ...]
//
// Hashes may be hex (0x...) or decimal.

use std::env;

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: <dba_path> <clip_index> <hash1> ...");
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

    // Parse via the public API just to find the clip name and confirm channel
    // shapes; raw byte layout we walk ourselves below.
    let db = starbreaker_3d::animation::parse_dba(&bytes).expect("parse_dba");
    let clip = &db.clips[clip_index];
    println!("clip[{clip_index}] = {} ({} channels)", clip.name, clip.channels.len());

    // Re-derive the IVO chunk so we can index into raw data the same way
    // parse_dba does. We piggyback on the public ChunkFile from common.
    use starbreaker_chunks::ChunkFile;
    const DBA_DATA: u32 = 0x194FBC50;
    let chunk_file = ChunkFile::from_bytes(&bytes).expect("ChunkFile");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        _ => panic!("expected IVO"),
    };
    let db_data_chunk = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == DBA_DATA)
        .expect("DBA_DATA chunk");
    let data: &[u8] = &ivo.file_data()[db_data_chunk.offset as usize..];

    // Walk blocks until we land on clip_index.
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
        let bone_count =
            u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        let block_start = offset + 12;
        let hash_table = block_start;
        let ctrl_table = hash_table + bone_count * 4;
        let headers_end = ctrl_table + bone_count * 24;

        if block_idx == clip_index {
            println!(
                "block[{block_idx}] start=0x{:x} bones={} hash_table=0x{:x} ctrl_table=0x{:x} headers_end=0x{:x}",
                block_start, bone_count, hash_table, ctrl_table, headers_end
            );
            for i in 0..bone_count {
                let h_off = hash_table + i * 4;
                let bone_hash = u32::from_le_bytes([
                    data[h_off],
                    data[h_off + 1],
                    data[h_off + 2],
                    data[h_off + 3],
                ]);
                if !want_hashes.is_empty() && !want_hashes.contains(&bone_hash) {
                    continue;
                }
                let c_off = ctrl_table + i * 24;
                let num_rot = u16::from_le_bytes([data[c_off], data[c_off + 1]]);
                let rot_flags = u16::from_le_bytes([data[c_off + 2], data[c_off + 3]]);
                let rot_t_off = u32::from_le_bytes([data[c_off + 4], data[c_off + 5], data[c_off + 6], data[c_off + 7]]);
                let rot_d_off = u32::from_le_bytes([data[c_off + 8], data[c_off + 9], data[c_off + 10], data[c_off + 11]]);
                let num_pos = u16::from_le_bytes([data[c_off + 12], data[c_off + 13]]);
                let pos_flags = u16::from_le_bytes([data[c_off + 14], data[c_off + 15]]);
                let pos_t_off = u32::from_le_bytes([data[c_off + 16], data[c_off + 17], data[c_off + 18], data[c_off + 19]]);
                let pos_d_off = u32::from_le_bytes([data[c_off + 20], data[c_off + 21], data[c_off + 22], data[c_off + 23]]);

                let abs_pos_data = c_off + pos_d_off as usize;
                println!("\n=== ctrl[{i}] hash=0x{:08X} ===", bone_hash);
                println!("  ctrl_row_offset (base)   = 0x{:x}", c_off);
                println!("  num_rot={} rot_flags=0x{:04X} rot_t_off=0x{:x} rot_d_off=0x{:x}", num_rot, rot_flags, rot_t_off, rot_d_off);
                println!("  num_pos={} pos_flags=0x{:04X} pos_t_off=0x{:x} pos_d_off=0x{:x} -> abs=0x{:x}", num_pos, pos_flags, pos_t_off, pos_d_off, abs_pos_data);

                // Decode SNORM-packed header (24 bytes: scale Vec3 + offset Vec3) if 0xC2.
                if (pos_flags >> 8) == 0xC2 || (pos_flags >> 8) == 0xC1 {
                    if abs_pos_data + 24 > data.len() {
                        println!("  HEADER OOB!");
                        continue;
                    }
                    let read_f32 = |o: usize| -> f32 {
                        f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
                    };
                    let scale = [
                        read_f32(abs_pos_data),
                        read_f32(abs_pos_data + 4),
                        read_f32(abs_pos_data + 8),
                    ];
                    let off = [
                        read_f32(abs_pos_data + 12),
                        read_f32(abs_pos_data + 16),
                        read_f32(abs_pos_data + 20),
                    ];
                    println!("  scale  = [{:>14e}, {:>14e}, {:>14e}]", scale[0], scale[1], scale[2]);
                    println!("  offset = [{:>14e}, {:>14e}, {:>14e}]", off[0], off[1], off[2]);
                    const FLT_MAX_SENTINEL: f32 = 3.0e38;
                    let active = [
                        scale[0].abs() < FLT_MAX_SENTINEL,
                        scale[1].abs() < FLT_MAX_SENTINEL,
                        scale[2].abs() < FLT_MAX_SENTINEL,
                    ];
                    let n_active = active.iter().filter(|a| **a).count();
                    let bytes_per_key = if (pos_flags >> 8) == 0xC2 { n_active * 2 } else { 6 };
                    let blob_start = abs_pos_data + 24;
                    let blob_end = blob_start + bytes_per_key * num_pos as usize;
                    println!("  active = {:?}  bytes_per_key = {}  blob = 0x{:x}..0x{:x} ({} bytes)",
                             active, bytes_per_key, blob_start, blob_end, blob_end - blob_start);
                    // Hex dump first 64 bytes + last 16 bytes
                    print!("  hex first 64: ");
                    for k in 0..(64.min(blob_end - blob_start)) {
                        print!("{:02x}{}", data[blob_start + k], if k % 2 == 1 { " " } else { "" });
                    }
                    println!();
                    if blob_end - blob_start > 64 {
                        print!("  hex last 16:  ");
                        for k in (blob_end - blob_start - 16)..(blob_end - blob_start) {
                            print!("{:02x}{}", data[blob_start + k], if k % 2 == 1 { " " } else { "" });
                        }
                        println!();
                    }
                    // First & last decoded values.
                    let decode_key = |key_idx: usize| -> [f32; 3] {
                        let o = blob_start + key_idx * bytes_per_key;
                        let mut p = off;
                        let mut bo = 0usize;
                        for ax in 0..3 {
                            if active[ax] {
                                let u = u16::from_le_bytes([data[o + bo], data[o + bo + 1]]);
                                p[ax] = u as f32 * scale[ax] + off[ax];
                                bo += 2;
                            }
                        }
                        p
                    };
                    let first = decode_key(0);
                    let last = decode_key(num_pos as usize - 1);
                    println!("  decoded first = [{:>9.4}, {:>9.4}, {:>9.4}]", first[0], first[1], first[2]);
                    println!("  decoded last  = [{:>9.4}, {:>9.4}, {:>9.4}]", last[0], last[1], last[2]);
                }
            }
            return;
        }

        block_idx += 1;
        offset = headers_end;
    }
    eprintln!("clip_index {clip_index} not found");
}
