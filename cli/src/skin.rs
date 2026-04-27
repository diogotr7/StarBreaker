use std::path::PathBuf;
use std::collections::HashMap;
use parking_lot::Mutex;

use clap::Subcommand;
use rayon::prelude::*;

use crate::common::load_p4k;
use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum SkinCommand {
    /// Export a .skin/.cgf file to GLB
    Export {
        /// P4k path substring (case-insensitive)
        path: String,
        /// Output .glb path
        output: Option<PathBuf>,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Inspect a .skinm/.skin/.cgfm/.chr file and print parsed metadata
    Inspect {
        /// P4k path substring (case-insensitive)
        path: String,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
        /// For .skinm files: dump per-vertex bone weight statistics
        /// (max influence count, weight distribution, vertices weighted to
        /// multiple bones).
        #[arg(long)]
        bone_weights: bool,
    },
    /// Scan all mesh files and report stream/chunk type statistics
    ScanStreams {
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
    /// Find all files containing a specific stream type ID
    FindStream {
        /// Stream type ID in hex (e.g. 9D51C5EE)
        stream_id: String,
        /// Path to Data.p4k
        #[arg(long, env = "SC_DATA_P4K")]
        p4k: Option<PathBuf>,
    },
}

impl SkinCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Export { path, output, p4k } => export(path, output, p4k),
            Self::Inspect { path, p4k, bone_weights } => inspect(path, p4k, bone_weights),
            Self::ScanStreams { p4k } => scan_streams(p4k),
            Self::FindStream { stream_id, p4k } => find_stream(stream_id, p4k),
        }
    }
}

fn stream_name(id: u32) -> &'static str {
    match id {
        0xEECDC168 => "IVOINDICES",
        0x91329AE9 => "IVOVERTSUVS",
        0xB3A70D5E => "IVOVERTSUVS2",
        0x9CF3F615 => "IVONORMALS",
        0x38A581FE => "IVONORMALS2",
        0xEE057252 => "IVOQTANGENTS",
        0xB95E9A1B => "IVOTANGENTS",
        0x677C7B23 => "IVOBONEMAP",
        0x6ECA3708 => "IVOBONEMAP32",
        0xD9EED421 => "IVOCOLORS2",
        0x9D51C5EE => "IVO_9D51C5EE",
        0xA596C0E6 => "IVO_A596C0E6",
        0x7E216CAB => "IVO_7E216CAB",
        0x69655CD8 => "IVO_69655CD8",
        0xDA86BE9C => "IVO_DA86BE9C",
        _ => "?",
    }
}

fn chunk_name(id: u32) -> &'static str {
    match id {
        0xBE5E493E => "RC_VERSION",
        0xC2011111 => "SKELETON",
        0x90C62222 => "PHYSICS",
        0x83353333 => "MATERIALINFO",
        0xA7169999 => "RBF",
        0x92914444 => "MESHINFO",
        0x0A2485B6 => "RIGLOGIC",
        0x7E035555 => "RIGINFO",
        0x2B8A2C29 => "SIMTOPOLOGY",
        0xF7086666 => "PROTOSINFO",
        0x9351756F => "LODDISTANCE",
        0x58DE1772 => "STATOBJ_PHYSICS",
        0x2B7ECF9F => "POSITIONBONEMAP",
        0x1E14A062 => "VFXHELPERS",
        0xC1C36AFE => "VISIBILITYGROUPS",
        0xC201973C => "SKELETON_v901",
        0x70697FDA => "SUBOBJECTS",
        0xB32459D2 => "MESH_CONTAINER",
        0xB8757777 => "IVO_SKIN2",
        0xE0181074 => "RAYTRACE_BVH",
        0x57A38888 => "BSHAPESGPU",
        0xFF277A9A => "SIMDEFORMATION",
        _ => "?",
    }
}

/// Scan stream headers from IVO_SKIN2 chunk data (already extracted from chunk wrapper).
fn scan_stream_types(chunk_data: &[u8]) -> Vec<(u32, u32)> {
    if chunk_data.len() < 168 { return vec![]; }

    // Header: 4B flags + 76B MeshInfo + 88B padding = 168
    let num_verts = u32::from_le_bytes(chunk_data[8..12].try_into().unwrap_or([0;4])) as usize;
    let num_idx = u32::from_le_bytes(chunk_data[16..20].try_into().unwrap_or([0;4])) as usize;
    let num_submeshes = u32::from_le_bytes(chunk_data[12..16].try_into().unwrap_or([0;4])) as usize;
    let extra_count = u32::from_le_bytes(chunk_data[76..80].try_into().unwrap_or([0;4])) as usize;

    // Sanity checks
    if num_submeshes > 1000 || num_verts > 10_000_000 || extra_count > 10000 { return vec![]; }

    let _sub_size = num_submeshes * 48; // SubMeshDescriptor is 48 bytes (36 data + 12 padding? let's try both)
    let start = 168 + num_submeshes * 36 + extra_count * 4;
    if start >= chunk_data.len() { return vec![]; }

    let mut streams = vec![];
    let mut pos = start;
    while pos + 8 <= chunk_data.len() {
        let tag = u32::from_le_bytes(chunk_data[pos..pos+4].try_into().unwrap_or([0;4]));
        if tag == 0 { pos += 4; continue; }
        let elem_size = u32::from_le_bytes(chunk_data[pos+4..pos+8].try_into().unwrap_or([0;4]));
        if elem_size == 0 || elem_size > 1024 { break; }

        streams.push((tag, elem_size));

        let count = if tag == 0xEECDC168 { num_idx } else { num_verts };
        let stream_bytes = elem_size as usize * count;
        pos += 8 + stream_bytes;
        // Align to 8 bytes
        let rem = stream_bytes % 8;
        if rem != 0 { pos += 8 - rem; }
    }
    streams
}

/// Extract chunk types and IVO stream types from a .cgfm/.skinm file using proper chunk parsing.
fn scan_file(data: &[u8]) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
    use starbreaker_chunks::ChunkFile;

    let mut chunks = vec![];
    let mut streams = vec![];

    let Ok(chunk_file) = ChunkFile::from_bytes(data) else { return (chunks, streams) };
    let ChunkFile::Ivo(ivo) = &chunk_file else { return (chunks, streams) };

    for entry in ivo.chunks() {
        chunks.push((entry.chunk_type, entry.version));

        // If this is IVO_SKIN2, scan its stream types
        if entry.chunk_type == 0xB8757777 {
            let chunk_data = ivo.chunk_data(entry);
            streams.extend(scan_stream_types(chunk_data));
        }
    }
    (chunks, streams)
}

fn scan_streams(p4k_path: Option<PathBuf>) -> Result<()> {
    let p4k = load_p4k(p4k_path.as_deref())?;

    let mesh_entries: Vec<_> = p4k.entries().iter()
        .filter(|e| {
            let name = e.name.to_lowercase();
            name.ends_with(".skinm") || name.ends_with(".cgfm")
        })
        .collect();

    eprintln!("Scanning {} mesh files...", mesh_entries.len());

    // Collect: stream_id -> (count, elem_sizes_seen, example_files)
    type Stats = HashMap<u32, (usize, Vec<u32>, Vec<String>)>;
    let stream_stats: Mutex<Stats> = Mutex::new(HashMap::new());
    let chunk_stats: Mutex<Stats> = Mutex::new(HashMap::new());
    let errors = Mutex::new(0usize);
    let scanned = Mutex::new(0usize);

    mesh_entries.par_iter().for_each(|entry| {
        let Ok(data) = p4k.read(entry) else {
            *errors.lock() += 1;
            return;
        };
        let (chunks, streams) = scan_file(&data);

        {
            let mut s = chunk_stats.lock();
            for (tag, version) in chunks {
                let e = s.entry(tag).or_insert_with(|| (0, vec![], vec![]));
                e.0 += 1;
                if !e.1.contains(&version) { e.1.push(version); }
                if e.2.len() < 3 {
                    e.2.push(entry.name.rsplit(['/', '\\']).next().unwrap_or("").to_string());
                }
            }
        }
        {
            let mut s = stream_stats.lock();
            for (tag, elem_size) in streams {
                let e = s.entry(tag).or_insert_with(|| (0, vec![], vec![]));
                e.0 += 1;
                if !e.1.contains(&elem_size) { e.1.push(elem_size); }
                if e.2.len() < 3 {
                    e.2.push(entry.name.rsplit(['/', '\\']).next().unwrap_or("").to_string());
                }
            }
        }

        let mut n = scanned.lock();
        *n += 1;
        if *n % 5000 == 0 { eprint!("\r  {}/{} files...", *n, mesh_entries.len()); }
    });

    let stream_stats = stream_stats.into_inner();
    let chunk_stats = chunk_stats.into_inner();
    let errors = errors.into_inner();
    eprintln!("\r  Done. {} files scanned, {} read errors.", scanned.into_inner(), errors);

    // Chunk census
    let mut sorted: Vec<_> = chunk_stats.into_iter().collect();
    sorted.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    println!("\n=== CHUNK TYPE CENSUS ===\n");
    println!("{:<14} {:<22} {:>7}  {:<12}  Example", "Chunk ID", "Name", "Count", "Versions");
    println!("{}", "-".repeat(90));
    for (tag, (count, versions, examples)) in &sorted {
        let name = chunk_name(*tag);
        let ver: Vec<String> = versions.iter().map(|v| format!("0x{v:X}")).collect();
        let ex = examples.first().map(|s| s.as_str()).unwrap_or("");
        println!("0x{:08X}  {:<22} {:>7}  {:<12}  {}", tag, name, count, ver.join(","), ex);
    }

    // Stream census
    let mut sorted: Vec<_> = stream_stats.into_iter().collect();
    sorted.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    println!("\n=== STREAM TYPE CENSUS ===\n");
    println!("{:<14} {:<20} {:>7}  {:<15}  Example", "Stream ID", "Name", "Count", "Elem Sizes");
    println!("{}", "-".repeat(90));
    for (tag, (count, sizes, examples)) in &sorted {
        let name = stream_name(*tag);
        let sizes_str: Vec<String> = sizes.iter().map(|s| s.to_string()).collect();
        let ex = examples.first().map(|s| s.as_str()).unwrap_or("");
        println!("0x{:08X}  {:<20} {:>7}  {:<15}  {}", tag, name, count, sizes_str.join(","), ex);
    }

    Ok(())
}

fn find_stream(stream_id_hex: String, p4k_path: Option<PathBuf>) -> Result<()> {
    let target = u32::from_str_radix(&stream_id_hex, 16)?;
    let p4k = load_p4k(p4k_path.as_deref())?;

    let mesh_entries: Vec<_> = p4k.entries().iter()
        .filter(|e| {
            let name = e.name.to_lowercase();
            name.ends_with(".skinm") || name.ends_with(".cgfm")
        })
        .collect();

    eprintln!("Searching {} files for stream 0x{:08X}...", mesh_entries.len(), target);

    let results: Mutex<Vec<(String, u32)>> = Mutex::new(vec![]);

    mesh_entries.par_iter().for_each(|entry| {
        let Ok(data) = p4k.read(entry) else { return };
        let (_, streams) = scan_file(&data);
        for (tag, elem_size) in streams {
            if tag == target {
                results.lock().push((entry.name.clone(), elem_size));
                break;
            }
        }
    });

    let mut results = results.into_inner();
    results.sort();
    println!("\nFound {} files with stream 0x{:08X} ({}):\n",
        results.len(), target, stream_name(target));
    for (name, elem_size) in &results {
        println!("  elem_size={:<4}  {}", elem_size, name);
    }
    Ok(())
}

fn export(search: String, output: Option<PathBuf>, p4k_path: Option<PathBuf>) -> Result<()> {
    let p4k = load_p4k(p4k_path.as_deref())?;
    let search_lower = search.to_lowercase();

    let entry = p4k
        .entries()
        .iter()
        .find(|e| {
            let name = e.name.to_lowercase();
            name.contains(&search_lower) && (name.ends_with(".skinm") || name.ends_with(".cgfm"))
        })
        .ok_or_else(|| CliError::NotFound(format!("no .skinm/.cgfm file matching '{search}' in P4k")))?;

    eprintln!("Found: {}", entry.name);
    let data = p4k.read(entry)?;
    let glb = starbreaker_3d::skin_to_glb(&data, None)?;

    let output = output.unwrap_or_else(|| {
        let stem = entry.name.rsplit(['/', '\\']).next().unwrap_or("output");
        PathBuf::from(format!("{stem}.glb"))
    });

    std::fs::write(&output, &glb)
        .map_err(|e| CliError::IoPath { source: e, path: output.display().to_string() })?;
    eprintln!("Written {} bytes to {}", glb.len(), output.display());
    Ok(())
}

fn inspect(search: String, p4k_path: Option<PathBuf>, bone_weights: bool) -> Result<()> {
    use starbreaker_chunks::ChunkFile;

    let p4k = load_p4k(p4k_path.as_deref())?;
    let search_lower = search.to_lowercase();

    let entry = p4k
        .entries()
        .iter()
        .find(|e| {
            let name = e.name.to_lowercase();
            // Prefer exact suffix match if search includes an extension
            if let Some(dot) = search_lower.rfind('.') {
                let search_ext = &search_lower[dot..];
                if !name.ends_with(search_ext) {
                    return false;
                }
            }
            name.contains(&search_lower)
                && (name.ends_with(".skinm")
                    || name.ends_with(".skin")
                    || name.ends_with(".cgfm")
                    || name.ends_with(".chr"))
        })
        .ok_or_else(|| CliError::NotFound(format!(
            "no .skinm/.skin/.cgfm/.chr file matching '{search}' in P4k"
        )))?;

    println!("Found: {}", entry.name);
    let data = p4k.read(entry)?;

    let lower_name = entry.name.to_lowercase();
    let is_skel_file = lower_name.ends_with(".chr") || lower_name.ends_with(".skin");
    if is_skel_file {
        if let Some(bones) = starbreaker_3d::skeleton::parse_skeleton(&data) {
            println!("Skeleton bones: {}", bones.len());
            for (index, bone) in bones.iter().enumerate() {
                println!(
                    "  [{index}] {} parent={:?} object_node_index={:?} local_pos={:?} local_rot={:?} world_pos={:?} world_rot={:?}",
                    bone.name,
                    bone.parent_index,
                    bone.object_node_index,
                    bone.local_position,
                    bone.local_rotation,
                    bone.world_position,
                    bone.world_rotation,
                );
            }
            return Ok(());
        }
        return Err(CliError::InvalidInput(format!(
            "failed to parse skeleton from {}",
            entry.name
        )));
    }

    let chunk_file = ChunkFile::from_bytes(&data)
        .map_err(|error| CliError::InvalidInput(format!("failed to parse chunk file {}: {error}", entry.name)))?;
    let ChunkFile::Ivo(ivo) = chunk_file else {
        return Err(CliError::InvalidInput(format!("unsupported non-IVO mesh file {}", entry.name)));
    };

    let skin_entry = ivo
        .chunks()
        .iter()
        .find(|chunk| chunk.chunk_type == starbreaker_chunks::known_types::ivo::IVO_SKIN2)
        .ok_or_else(|| CliError::InvalidInput(format!("no IVO_SKIN2 chunk in {}", entry.name)))?;

    let skin = starbreaker_3d::ivo::skin::SkinMesh::read(ivo.chunk_data(skin_entry))
        .map_err(|error| CliError::InvalidInput(format!("failed to parse skin mesh {}: {error}", entry.name)))?;

    println!(
        "SkinMesh flags={} verts={} indices={} submeshes={} extra_words={:?}",
        skin.flags,
        skin.info.num_vertices,
        skin.info.num_indices,
        skin.info.num_submeshes,
        skin.extra_words,
    );
    for (index, submesh) in skin.submeshes.iter().enumerate() {
        println!(
            "  submesh[{index}] node_parent={} first_index={} num_indices={} first_vertex={} num_vertices={} center={:?} radius={} unknown0=0x{:08X} unknown1=0x{:08X}",
            submesh.node_parent_index,
            submesh.first_index,
            submesh.num_indices,
            submesh.first_vertex,
            submesh.num_vertices,
            submesh.center,
            submesh.radius,
            submesh.unknown0,
            submesh.unknown1,
        );
    }

    if bone_weights {
        match &skin.streams.bone_maps {
            None => println!("\n[bone-weights] mesh has no bone_maps stream — purely rigid"),
            Some(maps) => {
                println!("\n[bone-weights] {} vertices total", maps.len());

                // Influence-count histogram: how many vertices have 1, 2, 3, 4 non-zero weights?
                let mut influence_hist = [0usize; 5];
                let mut joint_usage: HashMap<u16, usize> = HashMap::new();
                let mut multi_bone_examples: Vec<(usize, [u16; 4], [u8; 4])> = vec![];

                for (vid, bm) in maps.iter().enumerate() {
                    let n = bm.weights.iter().filter(|w| **w > 0).count();
                    influence_hist[n] += 1;
                    for slot in 0..4 {
                        if bm.weights[slot] > 0 {
                            *joint_usage.entry(bm.joint_indices[slot]).or_insert(0) += 1;
                        }
                    }
                    if n >= 2 && multi_bone_examples.len() < 16 {
                        multi_bone_examples.push((vid, bm.joint_indices, bm.weights));
                    }
                }

                println!("\nInfluence-count histogram (verts weighted to N non-zero bones):");
                for n in 0..=4 {
                    let pct = (influence_hist[n] as f64 / maps.len() as f64) * 100.0;
                    println!("  {n}-bone: {:>8} ({:>5.1}%)", influence_hist[n], pct);
                }

                println!("\nJoint usage (vertices weighted to each bone, sorted by count):");
                let mut sorted_joints: Vec<_> = joint_usage.into_iter().collect();
                sorted_joints.sort_by(|a, b| b.1.cmp(&a.1));
                for (joint, count) in sorted_joints.iter().take(20) {
                    println!("  joint[{:>3}]: {:>8} verts", joint, count);
                }

                if !multi_bone_examples.is_empty() {
                    println!("\nFirst {} vertices with multi-bone weights:", multi_bone_examples.len());
                    for (vid, joints, weights) in &multi_bone_examples {
                        println!(
                            "  vert[{:>5}] joints={:?} weights={:?}",
                            vid, joints, weights
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
