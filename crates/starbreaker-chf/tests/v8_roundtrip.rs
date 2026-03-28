//! v8 round-trip testing: decompress CHF → parse → write → compare
//! cargo test -p starbreaker-chf --test v8_roundtrip -- --nocapture

use starbreaker_chf::{ChfFile, parse_chf, write_chf};
use std::fs;
use std::path::Path;

fn collect_chf_files() -> Vec<(String, Vec<u8>)> {
    let base = Path::new("C:/Development/StarCitizen/StarBreaker/research");
    let mut files = Vec::new();
    for dir in &["localCharacters", "websiteCharacters"] {
        let d = base.join(dir);
        if d.exists() {
            collect_recursive(&d, &mut files);
        }
    }
    files
}

fn collect_recursive(dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_recursive(&p, out);
        } else if p.extension().is_some_and(|e| e == "chf")
            && let Ok(data) = fs::read(&p)
        {
            out.push((p.display().to_string(), data));
        }
    }
}

#[test]
fn round_trip_all_v8_files() {
    let chf_files = collect_chf_files();
    println!("\n=== V8 ROUND-TRIP TEST ===\n");

    let mut v8_count = 0;
    let mut v7_count = 0;
    let mut v8_pass = 0;
    let mut v8_fail = Vec::new();

    for (name, chf_bytes) in &chf_files {
        let Ok(file) = ChfFile::from_chf(chf_bytes) else {
            continue;
        };
        let bin = &file.data;

        let Ok(data) = parse_chf(bin) else { continue };

        if data.male_version == 8 {
            v8_count += 1;
            let written = write_chf(&data);

            if *bin == written {
                v8_pass += 1;
            } else {
                // Find first difference
                let first_diff = bin
                    .iter()
                    .zip(written.iter())
                    .enumerate()
                    .find(|(_, (a, b))| a != b)
                    .map(|(i, _)| i);

                let short_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
                let msg = match first_diff {
                    Some(i) => format!(
                        "{short_name}: first diff at byte {i} (orig=0x{:02x}, written=0x{:02x}), orig_len={}, written_len={}",
                        bin[i],
                        written[i],
                        bin.len(),
                        written.len()
                    ),
                    None => format!(
                        "{short_name}: length mismatch orig={} written={}",
                        bin.len(),
                        written.len()
                    ),
                };
                v8_fail.push(msg);
            }
        } else {
            v7_count += 1;
        }
    }

    println!("v7 files: {v7_count}");
    println!("v8 files: {v8_count}");
    println!("v8 round-trip pass: {v8_pass}");
    println!("v8 round-trip fail: {}", v8_fail.len());

    if !v8_fail.is_empty() {
        println!("\nFailures (first 10):");
        for msg in v8_fail.iter().take(10) {
            println!("  {msg}");
        }
    }

    // Also collect stats on what differs
    if !v8_fail.is_empty() {
        // Analyze one failure in detail
        println!("\n=== DETAILED DIFF FOR FIRST V8 FAILURE ===");
        for (_name, chf_bytes) in &chf_files {
            let Ok(file) = ChfFile::from_chf(chf_bytes) else {
                continue;
            };
            let bin = &file.data;
            let Ok(data) = parse_chf(bin) else { continue };
            if data.male_version != 8 {
                continue;
            }

            let written = write_chf(&data);
            if *bin == written {
                continue;
            }

            println!("\nOriginal: {} bytes", bin.len());
            println!("Written:  {} bytes", written.len());

            let min_len = bin.len().min(written.len());
            let mut diffs = 0;
            for i in 0..min_len {
                if bin[i] != written[i] {
                    if diffs < 5 {
                        // Show context
                        let start = i.saturating_sub(4);
                        let end = (i + 8).min(min_len);
                        println!(
                            "  Diff at {i}: orig[{start}..{end}]={:02x?} written[{start}..{end}]={:02x?}",
                            &bin[start..end],
                            &written[start..end]
                        );
                    }
                    diffs += 1;
                }
            }
            if bin.len() != written.len() {
                println!(
                    "  Tail of longer: {:02x?}",
                    if bin.len() > written.len() {
                        &bin[min_len..]
                    } else {
                        &written[min_len..]
                    }
                );
            }
            println!("  Total differing bytes: {diffs}");
            break;
        }
    }

    assert_eq!(v8_fail.len(), 0, "v8 round-trip failures");
}
