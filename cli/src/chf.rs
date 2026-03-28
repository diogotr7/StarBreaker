use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use crate::common::load_p4k;

#[derive(Subcommand)]
pub enum ChfCommand {
    /// Crack unknown NameHash CRC32C values against DataCore string tables
    CrackHashes {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
}

impl ChfCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::CrackHashes { p4k } => crack_hashes(p4k),
        }
    }
}

/// Extract all null-terminated strings from a byte slice (string table).
fn extract_strings(table: &[u8]) -> Vec<&str> {
    table
        .split(|&b| b == 0)
        .filter_map(|s| std::str::from_utf8(s).ok())
        .filter(|s| !s.is_empty())
        .collect()
}

fn crack_hashes(p4k_path: Option<PathBuf>) -> Result<()> {
    use starbreaker_common::NameHash;

    // 1. Collect unknown hashes from CHF corpus
    let base = std::path::Path::new("C:/Development/StarCitizen/StarBreaker/research");
    let mut all_hashes: HashSet<u32> = HashSet::new();
    let _hash_contexts: HashMap<u32, Vec<&'static str>> = HashMap::new();

    let dirs = ["localCharacters", "websiteCharacters"];
    let mut chf_files = Vec::new();
    for dir in &dirs {
        let d = base.join(dir);
        if d.exists() {
            collect_chf_recursive(&d, &mut chf_files);
        }
    }

    eprintln!("Parsing {} CHF files for unknown hashes...", chf_files.len());

    // We need to use leaked data since we're collecting hashes across files
    for data in &chf_files {
        let Ok(file) = starbreaker_chf::ChfFile::from_chf(data) else { continue };
        let Ok(parsed) = starbreaker_chf::parse_chf(&file.data) else { continue };

        // Collect hashes from itemport names
        collect_itemport_hashes(&parsed.itemport, &mut all_hashes);

        // Collect hashes from materials
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

        // DNA hashes
        all_hashes.insert(parsed.dna.gender_hash.value());
        all_hashes.insert(parsed.dna.variant_hash.value());
    }

    // Remove already-known hashes
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

    // 2. Load DataCore and extract all strings
    let p4k = load_p4k(p4k_path.as_deref())?;
    eprintln!("Loading DataCore...");

    let dcb_entry = p4k.entries().iter()
        .find(|e| e.name.to_lowercase().ends_with(".dcb"))
        .ok_or_else(|| anyhow::anyhow!("no global.dcb in P4K"))?;
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
    let mut _tried = 0usize;

    for s in st1_strings.iter().chain(st2_strings.iter()) {
        let hash = crc32c::crc32c(s.as_bytes());
        if unknown.contains(&hash) {
            cracked.push((hash, s.to_string()));
        }
        _tried += 1;

        // Try case variations: lowercase, UPPERCASE, PascalCase
        let lower = s.to_lowercase();
        if lower != *s {
            let h = crc32c::crc32c(lower.as_bytes());
            if unknown.contains(&h) { cracked.push((h, lower)); }
        }
        let upper = s.to_uppercase();
        if upper != *s {
            let h = crc32c::crc32c(upper.as_bytes());
            if unknown.contains(&h) { cracked.push((h, upper)); }
        }
        // snake_case → PascalCase (e.g. "facial_hair" → "FacialHair")
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
            let h = crc32c::crc32c(pascal.as_bytes());
            if unknown.contains(&h) { cracked.push((h, pascal)); }
        }
        // Also try with _m suffix on DataCore strings
        for suffix in &["_m", "_mat", "_material"] {
            let with_suffix = format!("{s}{suffix}");
            let h = crc32c::crc32c(with_suffix.as_bytes());
            if unknown.contains(&h) { cracked.push((h, with_suffix)); }
        }
    }

    // 4. Also try P4K file paths (material names often match .mtl paths)
    eprintln!("Trying P4K paths...");
    for entry in p4k.entries() {
        let name = &entry.name;
        let hash = crc32c::crc32c(name.as_bytes());
        if unknown.contains(&hash) {
            cracked.push((hash, name.clone()));
        }
        // Try just the filename
        if let Some(fname) = name.rsplit(['/', '\\']).next() {
            let h = crc32c::crc32c(fname.as_bytes());
            if unknown.contains(&h) {
                cracked.push((h, fname.to_string()));
            }
            // Without extension
            if let Some(stem) = fname.rsplit_once('.').map(|(s, _)| s) {
                let h = crc32c::crc32c(stem.as_bytes());
                if unknown.contains(&h) {
                    cracked.push((h, stem.to_string()));
                }
            }
        }
    }

    // 5. Try common CryEngine material parameter name patterns
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
            let name = format!("{prefix}{suffix}");
            let h = crc32c::crc32c(name.as_bytes());
            if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                cracked.push((h, name));
            }
        }
    }

    // 6. Try known itemport/attachment naming patterns
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
            let name = format!("{part}{suffix}");
            let h = crc32c::crc32c(name.as_bytes());
            if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                cracked.push((h, name));
            }
        }
    }

    // 7. Try material/sub-material naming patterns from P4K .mtl paths
    eprintln!("Trying material name patterns...");
    for entry in p4k.entries() {
        let name = &entry.name;
        if name.to_lowercase().ends_with(".mtl") {
            // Extract material name from path
            if let Some(fname) = name.rsplit(['/', '\\']).next() {
                if let Some(stem) = fname.rsplit_once('.').map(|(s, _)| s) {
                    // Try the stem and common submaterial suffixes
                    for suffix in &["", "_m", "_mat", "_head", "_body", "_skin", "_hair", "_eye"] {
                        let candidate = format!("{stem}{suffix}");
                        let h = crc32c::crc32c(candidate.as_bytes());
                        if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                            cracked.push((h, candidate));
                        }
                    }
                }
            }
        }
    }

    // 8. Try hair style naming patterns — exhaustive
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
    let hair_suffixes = &["_m", "_casual", "_military", "", "_casual_m", "_military_m",
        "_hathair", "_hathair_m", "_cap", "_cap_m", "_helmet", "_helmet_m"];
    for prefix in hair_prefixes {
        for i in 0..200 {
            for suffix in hair_suffixes {
                let name = format!("{prefix}{i:02}{suffix}");
                let h = crc32c::crc32c(name.as_bytes());
                if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                    cracked.push((h, name));
                }
                // Also without zero-padding
                let name2 = format!("{prefix}{i}{suffix}");
                let h2 = crc32c::crc32c(name2.as_bytes());
                if unknown.contains(&h2) && !cracked.iter().any(|(ch, _)| *ch == h2) {
                    cracked.push((h2, name2));
                }
            }
        }
    }

    // 9. Try head variant naming patterns (protos_human_{gender}_face_t{N}_{context})
    eprintln!("Trying head variant patterns...");
    for gender in &["male", "female"] {
        for t in 0..5 {
            for ctx in &["pu", "s42", "sq42", "npc", "generic", "base"] {
                let name = format!("protos_human_{gender}_face_t{t}_{ctx}");
                let h = crc32c::crc32c(name.as_bytes());
                if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                    cracked.push((h, name));
                }
            }
        }
    }

    // 10. Try DNA variant hashes — head IDs like "male01_t1", "head_variant_03" etc.
    eprintln!("Trying DNA variant patterns...");
    for prefix in &["male", "female", "Male", "Female", "m", "f",
                     "head", "Head", "protos", "variant"] {
        for i in 0..60 {
            for sep in &["", "_", "0"] {
                for suffix in &["", "_t0", "_t1", "_t2", "_t3", "_t0_pu", "_t1_pu", "_t2_pu",
                                "_pu", "_s42", "_npc", "_base", "_head", "_face"] {
                    let name = format!("{prefix}{sep}{i:02}{suffix}");
                    let h = crc32c::crc32c(name.as_bytes());
                    if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                        cracked.push((h, name));
                    }
                }
            }
        }
    }
    // Also try "male_v7_head_XX", "female_v2_head_XX" patterns
    for body in &["male_v7", "female_v2", "male_v8", "female_v3"] {
        for part in &["head", "face", "body", ""] {
            for i in 0..60 {
                let name = if part.is_empty() {
                    format!("{body}_{i:02}")
                } else {
                    format!("{body}_{part}_{i:02}")
                };
                let h = crc32c::crc32c(name.as_bytes());
                if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                    cracked.push((h, name));
                }
            }
        }
    }

    // 11. Try beard sub-material name patterns
    eprintln!("Trying beard sub-material patterns...");
    for prefix in &["beard_", "facial_hair_", "stubble_", "goatee_", "mustache_",
                     "sideburns_", "chinstrap_", "vandyke_", "full_beard_",
                     "beard_style_", "beard_dye_", "facial_",
                     "f_facial_hair_", "m_facial_hair_"] {
        for i in 0..200 {
            for suffix in &["", "_m", "_dye", "_material", "_mat", "_color", "_01", "_02"] {
                let name = format!("{prefix}{i:03}{suffix}");
                let h = crc32c::crc32c(name.as_bytes());
                if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                    cracked.push((h, name));
                }
                let name2 = format!("{prefix}{i:02}{suffix}");
                let h2 = crc32c::crc32c(name2.as_bytes());
                if unknown.contains(&h2) && !cracked.iter().any(|(ch, _)| *ch == h2) {
                    cracked.push((h2, name2));
                }
                let name3 = format!("{prefix}{i}{suffix}");
                let h3 = crc32c::crc32c(name3.as_bytes());
                if unknown.contains(&h3) && !cracked.iter().any(|(ch, _)| *ch == h3) {
                    cracked.push((h3, name3));
                }
            }
        }
    }

    // 12. Extract sub-material names from ALL .mtl files in P4K
    eprintln!("Extracting sub-material names from .mtl files...");
    let mtl_entries: Vec<_> = p4k.entries().iter()
        .filter(|e| e.name.to_lowercase().ends_with(".mtl"))
        .collect();
    eprintln!("  {} .mtl files to scan", mtl_entries.len());
    let mut mtl_names_tried = 0usize;
    for entry in &mtl_entries {
        let Ok(data) = p4k.read(entry) else { continue };
        // Parse CryXML mtl and extract Name attributes
        let Ok(xml) = starbreaker_cryxml::from_bytes(&data) else { continue };
        extract_mtl_names(&xml, xml.root(), &unknown, &mut cracked);
        mtl_names_tried += 1;
        if mtl_names_tried % 5000 == 0 {
            eprint!("\r  {}/{} .mtl files...", mtl_names_tried, mtl_entries.len());
        }
    }
    eprintln!("\r  Scanned {} .mtl files", mtl_names_tried);

    // 13. Search all DataCore record names for DNA variant hashes
    eprintln!("Hashing all DataCore record names ({} records)...", db.records().len());
    for record in db.records() {
        let name = db.resolve_string2(record.name_offset);
        let h = crc32c::crc32c(name.as_bytes());
        if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
            cracked.push((h, name.to_string()));
        }
        // Also try sub-parts of the record name
        for part in name.split(&['/', '\\', '.', '-', '_'][..]) {
            if part.len() >= 3 {
                let h2 = crc32c::crc32c(part.as_bytes());
                if unknown.contains(&h2) && !cracked.iter().any(|(ch, _)| *ch == h2) {
                    cracked.push((h2, part.to_string()));
                }
            }
        }
        // Also try the file name field
        let fname = db.resolve_string(record.file_name_offset);
        if !fname.is_empty() {
            let h2 = crc32c::crc32c(fname.as_bytes());
            if unknown.contains(&h2) && !cracked.iter().any(|(ch, _)| *ch == h2) {
                cracked.push((h2, fname.to_string()));
            }
        }
    }

    // 14. Check if DNA variant hashes are raw DataCore headId values (not CRC32C)
    // Search for SCharacterCustomizerDNAHeadPool records and extract headId values
    let dna_unknowns: Vec<u32> = unknown.iter()
        .filter(|h| !cracked.iter().any(|(ch, _)| ch == *h))
        .filter(|h| **h >= 0x64000000 && **h <= 0x69000000) // the cluster range
        .copied()
        .collect();
    if !dna_unknowns.is_empty() {
        eprintln!("Searching DataCore for SCharacterCustomizerDNAHeadPool records...");
        // Try multiple possible struct names
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
            // Try partial name match
            eprintln!("  No head pool records found, searching for similar struct names...");
            for record in db.records() {
                let sname = db.resolve_string2(record.name_offset);
                if sname.to_lowercase().contains("head") && sname.to_lowercase().contains("dna") {
                    eprintln!("    Candidate record: {}", sname);
                    head_pool_records.push(record);
                }
            }
        }
        eprintln!("  Total candidate records: {}", head_pool_records.len());
        for record in &head_pool_records {
            let name = db.resolve_string2(record.name_offset);
            eprintln!("  Record: {}", name);
            // The headId is at a specific offset in the instance data.
            // Try hashing the record name which might be the head variant name.
            let h = crc32c::crc32c(name.as_bytes());
            if dna_unknowns.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                cracked.push((h, format!("{name} (head pool record)")));
            }
        }
    }

    // Deduplicate
    cracked.sort_by_key(|(h, _)| *h);
    cracked.dedup_by_key(|(h, _)| *h);

    let cracked_set: HashSet<u32> = cracked.iter().map(|(h, _)| *h).collect();
    let still_unknown = unknown.len() - cracked_set.len();

    println!("\n=== CRACKED HASHES ({} new) ===\n", cracked.len());
    for (hash, name) in &cracked {
        println!("  0x{:08X} = \"{}\"", hash, name);
    }

    // Categorize remaining unknowns by context
    let mut unknown_contexts: HashMap<u32, Vec<String>> = HashMap::new();
    for data in &chf_files {
        let Ok(file) = starbreaker_chf::ChfFile::from_chf(data) else { continue };
        let Ok(parsed) = starbreaker_chf::parse_chf(&file.data) else { continue };

        let still_unknown: HashSet<u32> = unknown.iter()
            .filter(|h| !cracked_set.contains(h))
            .copied()
            .collect();

        // Check DNA
        if still_unknown.contains(&parsed.dna.gender_hash.value()) {
            unknown_contexts.entry(parsed.dna.gender_hash.value())
                .or_default().push("dna.gender_hash".into());
        }
        if still_unknown.contains(&parsed.dna.variant_hash.value()) {
            unknown_contexts.entry(parsed.dna.variant_hash.value())
                .or_default().push("dna.variant_hash".into());
        }

        // Check itemports
        fn check_ports(port: &starbreaker_chf::ItemPort, unknown: &HashSet<u32>, ctx: &mut HashMap<u32, Vec<String>>, path: &str) {
            let h = port.name.value();
            if unknown.contains(&h) {
                ctx.entry(h).or_default().push(format!("itemport:{path}"));
            }
            for child in &port.children {
                let child_path = format!("{path}/{}", child.name);
                check_ports(child, unknown, ctx, &child_path);
            }
        }
        check_ports(&parsed.itemport, &still_unknown, &mut unknown_contexts, "root");

        // Check materials
        for mat in &parsed.materials {
            if still_unknown.contains(&mat.name.value()) {
                unknown_contexts.entry(mat.name.value())
                    .or_default().push("material.name".into());
            }
            for sub in &mat.sub_materials {
                if still_unknown.contains(&sub.name.value()) {
                    unknown_contexts.entry(sub.name.value())
                        .or_default().push(format!("submaterial.name (in mat {})", mat.name));
                }
                for p in &sub.material_params {
                    if still_unknown.contains(&p.name.value()) {
                        unknown_contexts.entry(p.name.value())
                            .or_default().push(format!("float_param (in sub {})", sub.name));
                    }
                }
                for c in &sub.material_colors {
                    if still_unknown.contains(&c.name.value()) {
                        unknown_contexts.entry(c.name.value())
                            .or_default().push(format!("color_param (in sub {})", sub.name));
                    }
                }
            }
        }
    }

    println!("\n=== STILL UNKNOWN ({}) ===\n", still_unknown);
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
        println!("  0x{:08X}  {}", h, contexts);
    }

    Ok(())
}

fn collect_itemport_hashes(port: &starbreaker_chf::ItemPort, hashes: &mut HashSet<u32>) {
    hashes.insert(port.name.value());
    for child in &port.children {
        collect_itemport_hashes(child, hashes);
    }
}

fn extract_mtl_names(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    unknown: &HashSet<u32>,
    cracked: &mut Vec<(u32, String)>,
) {
    // Check Name attribute on this node
    for (key, val) in xml.node_attributes(node) {
        if key == "Name" && !val.is_empty() {
            let h = crc32c::crc32c(val.as_bytes());
            if unknown.contains(&h) && !cracked.iter().any(|(ch, _)| *ch == h) {
                cracked.push((h, val.to_string()));
            }
            // Also try lowercase
            let lower = val.to_lowercase();
            let h2 = crc32c::crc32c(lower.as_bytes());
            if unknown.contains(&h2) && !cracked.iter().any(|(ch, _)| *ch == h2) {
                cracked.push((h2, lower));
            }
        }
    }
    // Recurse into children
    for child in xml.node_children(node) {
        extract_mtl_names(xml, child, unknown, cracked);
    }
}

fn collect_chf_recursive(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
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
