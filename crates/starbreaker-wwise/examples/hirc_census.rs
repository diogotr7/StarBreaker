/// Scan all BNK files in a directory, build typed Hierarchy, and count parse success rate.
use std::collections::HashMap;
use std::env;
use std::fs;

use starbreaker_wwise::{BnkFile, HircObject, HircObjectType, Hierarchy};

fn main() -> anyhow::Result<()> {
    let dir = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: hirc_census <directory>");
        std::process::exit(1);
    });

    let mut type_counts: HashMap<u8, usize> = HashMap::new();
    let mut typed_count: u32 = 0;
    let mut unknown_count: u32 = 0;
    let mut bank_count: u32 = 0;
    let mut hirc_count: u32 = 0;
    let mut errors: u32 = 0;

    for entry in walkdir(dir.as_ref()) {
        let data = match fs::read(&entry) {
            Ok(d) => d,
            Err(_) => { errors += 1; continue; }
        };

        let bnk = match BnkFile::parse(&data) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  PARSE ERROR {}: {e}", entry.display());
                errors += 1;
                continue;
            }
        };

        bank_count += 1;

        if let Some(ref hirc) = bnk.hirc {
            hirc_count += hirc.entries.len() as u32;

            // Build typed hierarchy
            let hierarchy = Hierarchy::from_section(hirc);

            for e in &hirc.entries {
                *type_counts.entry(e.type_id).or_insert(0) += 1;
                match hierarchy.get(e.object_id) {
                    Some(HircObject::Unknown { .. }) => unknown_count += 1,
                    Some(_) => typed_count += 1,
                    None => unknown_count += 1,
                }
            }
        }
    }

    eprintln!("\n=== HIRC Census (Typed Parse) ===");
    eprintln!("Banks scanned:  {bank_count}");
    eprintln!("BNK parse errors: {errors}");
    eprintln!("HIRC objects:   {hirc_count}");
    eprintln!("  Typed:        {typed_count} ({:.1}%)", typed_count as f64 / hirc_count as f64 * 100.0);
    eprintln!("  Unknown:      {unknown_count} ({:.1}%)", unknown_count as f64 / hirc_count as f64 * 100.0);
    eprintln!();

    let mut sorted: Vec<_> = type_counts.iter().collect();
    sorted.sort_by_key(|(_, count)| std::cmp::Reverse(**count));

    eprintln!("{:<6} {:<40} {:<10}", "ID", "Type", "Count");
    eprintln!("{}", "-".repeat(56));
    for (type_id, count) in sorted {
        let name = HircObjectType::from_u8(*type_id)
            .map(|t| t.name().to_string())
            .unwrap_or_else(|| format!("UNKNOWN({})", type_id));
        eprintln!("{:<6} {:<40} {:<10}", type_id, name, count);
    }

    Ok(())
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path));
            } else if path.extension().is_some_and(|e| e == "bnk") {
                results.push(path);
            }
        }
    }
    results
}
