//! Brute-force CRC32C hash cracker for unknown CHF NameHash values.
//!
//! Collects unknown hashes from a corpus of .chf files, then tries every string
//! from DataCore, P4K paths, .mtl files, and common naming patterns to find matches.
//!
//! This is a research tool — it lives here as an example because it depends on
//! multiple workspace crates (chf, p4k, datacore, cryxml, common) and the CLI
//! crate already has all of them as dependencies.
//!
//! Usage:
//!   cargo run -p starbreaker --example crack_chf_hashes -- <chf_dir> [--p4k <path>]
//!
//! Arguments:
//!   <chf_dir>   Directory containing .chf files to scan for unknown hashes
//!   --p4k       Path to Data.p4k (or set SC_DATA_P4K env var)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use starbreaker_common::NameHash;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <chf_dir> [--p4k <path>]", args[0]);
        eprintln!();
        eprintln!("  <chf_dir>   Directory containing .chf files");
        eprintln!("  --p4k       Path to Data.p4k (or set SC_DATA_P4K)");
        std::process::exit(2);
    }

    let chf_dir = PathBuf::from(&args[1]);
    if !chf_dir.is_dir() {
        bail!("{} is not a directory", chf_dir.display());
    }

    let p4k_path: Option<PathBuf> = args
        .windows(2)
        .find(|w| w[0] == "--p4k")
        .map(|w| PathBuf::from(&w[1]));

    // 1. Collect unknown hashes from CHF corpus
    let mut chf_files = Vec::new();
    collect_chf_recursive(&chf_dir, &mut chf_files);

    eprintln!("Parsing {} CHF files for unknown hashes...", chf_files.len());

    let mut all_hashes: HashSet<u32> = HashSet::new();
    for data in &chf_files {
        let Ok(file) = starbreaker_chf::ChfFile::from_chf(data) else { continue };
        let Ok(parsed) = starbreaker_chf::parse_chf(&file.data) else { continue };

        collect_itemport_hashes(&parsed.itemport, &mut all_hashes);

        for mat in &parsed.materials {
            all_hashes.insert(mat.name.value());
            for sub in &mat.sub_materials {
                all_hashes.insert(sub.name.value());
                for p in &sub.material_params {
                    all_hashes.insert(p.name.value());
                }
                for c in &sub.material_colors {
                    all_hashes.insert(c.name.value());
                }
            }
        }

        all_hashes.insert(parsed.dna.gender_hash.value());
        all_hashes.insert(parsed.dna.variant_hash.value());
    }

    let known_count = all_hashes.iter().filter(|h| NameHash(**h).name().is_some()).count();
    let unknown: HashSet<u32> = all_hashes
        .iter()
        .filter(|h| NameHash(**h).name().is_none())
        .copied()
        .collect();

    eprintln!(
        "Found {} unique hashes ({} known, {} unknown)",
        all_hashes.len(), known_count, unknown.len()
    );

    if unknown.is_empty() {
        println!("All hashes already cracked!");
        return Ok(());
    }

    // 2. Load P4K and DataCore
    let p4k = match p4k_path {
        Some(path) => starbreaker_p4k::MappedP4k::open(&path)
            .with_context(|| format!("opening {}", path.display()))?,
        None => starbreaker_p4k::open_p4k()
            .context("auto-discovering P4K")?,
    };
    eprintln!("Loading DataCore...");

    let dcb_entry = p4k.entries().iter()
        .find(|e| e.name.to_lowercase().ends_with(".dcb"))
        .context("no .dcb file in P4K")?;
    let dcb_data = p4k.read(dcb_entry)?;
    let db = starbreaker_datacore::Database::from_bytes(&dcb_data)?;

    let st1_strings = extract_strings(db.string_table1());
    let st2_strings = extract_strings(db.string_table2());
    eprintln!(
        "DataCore strings: {} from table1, {} from table2",
        st1_strings.len(), st2_strings.len()
    );

    // 3. Hash all DataCore strings and match
    let mut cracked: Vec<(u32, String)> = Vec::new();

    for s in st1_strings.iter().chain(st2_strings.iter()) {
        try_candidate(s, &unknown, &mut cracked);

        let lower = s.to_lowercase();
        if lower != *s { try_candidate(&lower, &unknown, &mut cracked); }
        let upper = s.to_uppercase();
        if upper != *s { try_candidate(&upper, &unknown, &mut cracked); }

        if s.contains('_') {
            let pascal: String = s.split('_')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().chain(c).collect(),
                    }
                })
                .collect();
            try_candidate(&pascal, &unknown, &mut cracked);
        }

        for suffix in &["_m", "_mat", "_material"] {
            try_candidate(&format!("{s}{suffix}"), &unknown, &mut cracked);
        }
    }

    // 4. P4K file paths
    eprintln!("Trying P4K paths...");
    for entry in p4k.entries() {
        let name = &entry.name;
        try_candidate(name, &unknown, &mut cracked);
        if let Some(fname) = name.rsplit(['/', '\\']).next() {
            try_candidate(fname, &unknown, &mut cracked);
            if let Some(stem) = fname.rsplit_once('.').map(|(s, _)| s) {
                try_candidate(stem, &unknown, &mut cracked);
            }
        }
    }

    // 5. Material parameter name patterns
    eprintln!("Trying material parameter patterns...");
    let prefixes = &[
        "Makeup1", "Makeup2", "Makeup3", "Tattoo", "Dye", "Base", "Sun", "Hair",
        "Eye", "Cheek", "Lip", "Body", "Freckles", "Head", "Skin", "Complexion",
        "Beard", "Stubble", "Brow", "Ear", "Nose", "Mouth", "Jaw", "Crown", "Neck",
        "Scalp", "Face", "Lash", "Iris", "Pupil", "Sclera", "Wrinkle", "Pore",
        "Mole", "Scar", "Tattoo1", "Tattoo2", "Tattoo3", "Decal",
    ];
    let suffixes = &[
        "Amount", "Opacity", "OpacityR", "OpacityG", "OpacityB",
        "Shift", "Variation", "Fadeout", "Age", "Redness", "Hue", "HueRotation",
        "Saturation", "Brightness", "Contrast", "Intensity", "Power", "Scale",
        "NumTilesU", "NumTilesV", "OffsetU", "OffsetV", "Rotation",
        "MetalnessR", "MetalnessG", "MetalnessB", "SmoothnessR", "SmoothnessG", "SmoothnessB",
        "Color", "Color1", "Color2", "Color3", "Tint", "Weight", "Blend",
        "Roughness", "Specular", "Glossiness", "Emissive", "Normal", "Albedo",
        "Depth", "Width", "Height", "Radius", "Angle", "Diameter", "Alpha",
    ];
    for prefix in prefixes {
        for suffix in suffixes {
            try_candidate(&format!("{prefix}{suffix}"), &unknown, &mut cracked);
        }
    }

    // 6. Itemport/attachment naming patterns
    eprintln!("Trying itemport patterns...");
    let port_parts = &[
        "head", "body", "hair", "beard", "eyes", "eyebrow", "eyelashes", "stubble",
        "piercings", "tattoo", "decal", "makeup", "complexion", "scalp",
        "piercings_eyebrows", "piercings_l_ear", "piercings_r_ear",
        "piercings_nose", "piercings_mouth",
    ];
    let port_suffixes = &[
        "_itemport", "_skinitemport", "_itemport_skinitemport", "",
        "_m", "_material", "_mat", "_mtl",
    ];
    for part in port_parts {
        for suffix in port_suffixes {
            try_candidate(&format!("{part}{suffix}"), &unknown, &mut cracked);
        }
    }

    // 7. Material names from .mtl paths
    eprintln!("Trying material name patterns...");
    for entry in p4k.entries() {
        let name = &entry.name;
        if name.to_lowercase().ends_with(".mtl") {
            if let Some(fname) = name.rsplit(['/', '\\']).next() {
                if let Some(stem) = fname.rsplit_once('.').map(|(s, _)| s) {
                    for suffix in &["", "_m", "_mat", "_head", "_body", "_skin", "_hair", "_eye"] {
                        try_candidate(&format!("{stem}{suffix}"), &unknown, &mut cracked);
                    }
                }
            }
        }
    }

    // 8. Hair style patterns
    eprintln!("Trying hair/beard patterns...");
    let hair_prefixes = &[
        "f_hair_", "m_hair_", "hair_", "facial_hair_", "f_facial_hair_",
        "m_facial_hair_", "hair_shaved_", "hair_short_", "hair_long_",
        "straight_short_hair_", "straight_long_hair_", "wavy_short_hair_",
        "wavy_long_hair_", "curly_short_hair_", "curly_long_hair_",
        "mohawk_short_hair_", "shaved_short_hair_", "balding_short_hair_",
        "bun_long_hair_", "ponytail_long_hair_", "afro_short_hair_",
        "dreadlocks_long_hair_", "braided_long_hair_",
    ];
    let hair_suffixes = &[
        "_m", "_casual", "_military", "", "_casual_m", "_military_m",
        "_hathair", "_hathair_m", "_cap", "_cap_m", "_helmet", "_helmet_m",
    ];
    for prefix in hair_prefixes {
        for i in 0..200 {
            for suffix in hair_suffixes {
                try_candidate(&format!("{prefix}{i:02}{suffix}"), &unknown, &mut cracked);
                try_candidate(&format!("{prefix}{i}{suffix}"), &unknown, &mut cracked);
            }
        }
    }

    // 9. Head variant patterns
    eprintln!("Trying head variant patterns...");
    for gender in &["male", "female"] {
        for t in 0..5 {
            for ctx in &["pu", "s42", "sq42", "npc", "generic", "base"] {
                try_candidate(
                    &format!("protos_human_{gender}_face_t{t}_{ctx}"),
                    &unknown,
                    &mut cracked,
                );
            }
        }
    }

    // 10. DNA variant patterns
    eprintln!("Trying DNA variant patterns...");
    for prefix in &["male", "female", "Male", "Female", "m", "f",
                     "head", "Head", "protos", "variant"] {
        for i in 0..60 {
            for sep in &["", "_", "0"] {
                for suffix in &["", "_t0", "_t1", "_t2", "_t3", "_t0_pu", "_t1_pu", "_t2_pu",
                                "_pu", "_s42", "_npc", "_base", "_head", "_face"] {
                    try_candidate(&format!("{prefix}{sep}{i:02}{suffix}"), &unknown, &mut cracked);
                }
            }
        }
    }
    for body in &["male_v7", "female_v2", "male_v8", "female_v3"] {
        for part in &["head", "face", "body", ""] {
            for i in 0..60 {
                let name = if part.is_empty() {
                    format!("{body}_{i:02}")
                } else {
                    format!("{body}_{part}_{i:02}")
                };
                try_candidate(&name, &unknown, &mut cracked);
            }
        }
    }

    // 11. Beard sub-material patterns
    eprintln!("Trying beard sub-material patterns...");
    for prefix in &["beard_", "facial_hair_", "stubble_", "goatee_", "mustache_",
                     "sideburns_", "chinstrap_", "vandyke_", "full_beard_",
                     "beard_style_", "beard_dye_", "facial_",
                     "f_facial_hair_", "m_facial_hair_"] {
        for i in 0..200 {
            for suffix in &["", "_m", "_dye", "_material", "_mat", "_color", "_01", "_02"] {
                try_candidate(&format!("{prefix}{i:03}{suffix}"), &unknown, &mut cracked);
                try_candidate(&format!("{prefix}{i:02}{suffix}"), &unknown, &mut cracked);
                try_candidate(&format!("{prefix}{i}{suffix}"), &unknown, &mut cracked);
            }
        }
    }

    // 12. Sub-material names from .mtl files in P4K
    eprintln!("Extracting sub-material names from .mtl files...");
    let mtl_entries: Vec<_> = p4k.entries().iter()
        .filter(|e| e.name.to_lowercase().ends_with(".mtl"))
        .collect();
    eprintln!("  {} .mtl files to scan", mtl_entries.len());
    let mut mtl_scanned = 0usize;
    for entry in &mtl_entries {
        let Ok(data) = p4k.read(entry) else { continue };
        let Ok(xml) = starbreaker_cryxml::from_bytes(&data) else { continue };
        extract_mtl_names(&xml, xml.root(), &unknown, &mut cracked);
        mtl_scanned += 1;
        if mtl_scanned % 5000 == 0 {
            eprint!("\r  {mtl_scanned}/{} .mtl files...", mtl_entries.len());
        }
    }
    eprintln!("\r  Scanned {mtl_scanned} .mtl files");

    // 13. DataCore record names
    eprintln!("Hashing all DataCore record names ({} records)...", db.records().len());
    for record in db.records() {
        let name = db.resolve_string2(record.name_offset);
        try_candidate(name, &unknown, &mut cracked);
        for part in name.split(&['/', '\\', '.', '-', '_'][..]) {
            if part.len() >= 3 {
                try_candidate(part, &unknown, &mut cracked);
            }
        }
        let fname = db.resolve_string(record.file_name_offset);
        if !fname.is_empty() {
            try_candidate(fname, &unknown, &mut cracked);
        }
    }

    // 14. DataCore head pool records for DNA variant hashes
    let dna_unknowns: Vec<u32> = unknown.iter()
        .filter(|h| !cracked.iter().any(|(ch, _)| ch == *h))
        .filter(|h| **h >= 0x64000000 && **h <= 0x69000000)
        .copied()
        .collect();
    if !dna_unknowns.is_empty() {
        eprintln!("Searching DataCore for head pool records...");
        let mut head_pool_records = Vec::new();
        for type_name in &["SCharacterCustomizerDNAHeadPool", "SCharacterCustomizerDNAHeadParams",
                            "CharacterCustomizerDNAHeadPool", "DNAHeadPool", "HeadPool"] {
            let recs: Vec<_> = db.records_by_type_name(type_name).collect();
            if !recs.is_empty() {
                eprintln!("  Found {} records of type {}", recs.len(), type_name);
                head_pool_records.extend(recs);
            }
        }
        if head_pool_records.is_empty() {
            eprintln!("  No head pool records found, searching by name...");
            for record in db.records() {
                let sname = db.resolve_string2(record.name_offset);
                if sname.to_lowercase().contains("head") && sname.to_lowercase().contains("dna") {
                    eprintln!("    Candidate: {sname}");
                    head_pool_records.push(record);
                }
            }
        }
        for record in &head_pool_records {
            let name = db.resolve_string2(record.name_offset);
            let h = crc32c::crc32c(name.as_bytes());
            if dna_unknowns.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                cracked.push((h, format!("{name} (head pool record)")));
            }
        }
    }

    // Deduplicate and report
    cracked.sort_by_key(|(h, _)| *h);
    cracked.dedup_by_key(|(h, _)| *h);

    let cracked_set: HashSet<u32> = cracked.iter().map(|(h, _)| *h).collect();
    let still_unknown = unknown.len() - cracked_set.len();

    println!("\n=== CRACKED HASHES ({} new) ===\n", cracked.len());
    for (hash, name) in &cracked {
        println!("  0x{hash:08X} = \"{name}\"");
    }

    // Categorize remaining unknowns by context
    let mut unknown_contexts: HashMap<u32, Vec<String>> = HashMap::new();
    for data in &chf_files {
        let Ok(file) = starbreaker_chf::ChfFile::from_chf(data) else { continue };
        let Ok(parsed) = starbreaker_chf::parse_chf(&file.data) else { continue };

        let remaining: HashSet<u32> = unknown.iter()
            .filter(|h| !cracked_set.contains(h))
            .copied()
            .collect();

        if remaining.contains(&parsed.dna.gender_hash.value()) {
            unknown_contexts.entry(parsed.dna.gender_hash.value())
                .or_default().push("dna.gender_hash".into());
        }
        if remaining.contains(&parsed.dna.variant_hash.value()) {
            unknown_contexts.entry(parsed.dna.variant_hash.value())
                .or_default().push("dna.variant_hash".into());
        }

        check_ports(&parsed.itemport, &remaining, &mut unknown_contexts, "root");

        for mat in &parsed.materials {
            if remaining.contains(&mat.name.value()) {
                unknown_contexts.entry(mat.name.value())
                    .or_default().push("material.name".into());
            }
            for sub in &mat.sub_materials {
                if remaining.contains(&sub.name.value()) {
                    unknown_contexts.entry(sub.name.value())
                        .or_default().push(format!("submaterial.name (in mat {})", mat.name));
                }
                for p in &sub.material_params {
                    if remaining.contains(&p.name.value()) {
                        unknown_contexts.entry(p.name.value())
                            .or_default().push(format!("float_param (in sub {})", sub.name));
                    }
                }
                for c in &sub.material_colors {
                    if remaining.contains(&c.name.value()) {
                        unknown_contexts.entry(c.name.value())
                            .or_default().push(format!("color_param (in sub {})", sub.name));
                    }
                }
            }
        }
    }

    println!("\n=== STILL UNKNOWN ({still_unknown}) ===\n");
    let mut unknown_sorted: Vec<_> = unknown.iter()
        .filter(|h| !cracked_set.contains(h))
        .collect();
    unknown_sorted.sort();
    for h in unknown_sorted {
        let contexts = unknown_contexts.get(h)
            .map(|v| {
                let mut deduped = v.clone();
                deduped.sort();
                deduped.dedup();
                deduped.join(", ")
            })
            .unwrap_or_default();
        println!("  0x{h:08X}  {contexts}");
    }

    Ok(())
}

fn try_candidate(s: &str, unknown: &HashSet<u32>, cracked: &mut Vec<(u32, String)>) {
    let h = crc32c::crc32c(s.as_bytes());
    if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
        cracked.push((h, s.to_string()));
    }
}

fn extract_strings(table: &[u8]) -> Vec<&str> {
    table
        .split(|&b| b == 0)
        .filter_map(|s| std::str::from_utf8(s).ok())
        .filter(|s| !s.is_empty())
        .collect()
}

fn collect_itemport_hashes(port: &starbreaker_chf::ItemPort, hashes: &mut HashSet<u32>) {
    hashes.insert(port.name.value());
    for child in &port.children {
        collect_itemport_hashes(child, hashes);
    }
}

fn check_ports(
    port: &starbreaker_chf::ItemPort,
    unknown: &HashSet<u32>,
    ctx: &mut HashMap<u32, Vec<String>>,
    path: &str,
) {
    let h = port.name.value();
    if unknown.contains(&h) {
        ctx.entry(h).or_default().push(format!("itemport:{path}"));
    }
    for child in &port.children {
        let child_path = format!("{path}/{}", child.name);
        check_ports(child, unknown, ctx, &child_path);
    }
}

fn extract_mtl_names(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    unknown: &HashSet<u32>,
    cracked: &mut Vec<(u32, String)>,
) {
    for (key, val) in xml.node_attributes(node) {
        if key == "Name" && !val.is_empty() {
            try_candidate(val, unknown, cracked);
            let lower = val.to_lowercase();
            try_candidate(&lower, unknown, cracked);
        }
    }
    for child in xml.node_children(node) {
        extract_mtl_names(xml, child, unknown, cracked);
    }
}

fn collect_chf_recursive(dir: &Path, out: &mut Vec<Vec<u8>>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_chf_recursive(&p, out);
        } else if p.extension().is_some_and(|e| e == "chf") {
            if let Ok(data) = std::fs::read(&p) {
                out.push(data);
            }
        }
    }
}
