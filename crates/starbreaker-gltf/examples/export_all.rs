//! Reproduce the "export all entities" flow from the Tauri UI.
//!
//! Parallel with half-core threads, tracks working set.
//!
//! Usage:
//!     cargo run --example export_all -- <output_dir>

use std::sync::atomic::{AtomicUsize, Ordering};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::{EntityIndex, resolve_loadout_indexed};
use starbreaker_datacore::types::Record;

fn main() {
    env_logger::init();
    let output_dir = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: export_all <output_dir>");
        std::process::exit(1);
    });
    std::fs::create_dir_all(&output_dir).expect("failed to create output dir");

    let opts = starbreaker_gltf::ExportOptions {
        include_textures: true,
        include_normals: true,
        include_tangents: true,
        include_interior: true,
        include_lights: true,
        include_materials: true,
        experimental_textures: true,
        texture_mip: 1,
        lod_level: 1,
    };

    // Load P4k + DCB
    eprintln!("Opening P4k...");
    let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
    eprintln!("Loading Game2.dcb...");
    let dcb_data = p4k
        .read_file("Data\\Game2.dcb")
        .expect("failed to read Game2.dcb");
    let db = Database::from_bytes(&dcb_data).expect("failed to parse DCB");

    // Filter like the UI: main EntityClassDefinition records in ships/groundvehicles
    let entity_si = db
        .struct_id("EntityClassDefinition")
        .expect("EntityClassDefinition not found");
    let records: Vec<&Record> = db
        .records_of_type(entity_si)
        .filter(|r| {
            if !db.is_main_record(r) {
                return false;
            }
            let path = db.resolve_string(r.file_name_offset).to_lowercase();
            path.contains("entities/spaceships") || path.contains("entities/groundvehicles")
        })
        .collect();
    let total = records.len();
    eprintln!("Found {total} ship/vehicle entities to export");

    // Half the cores, min 2 — same as the Tauri command
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap();
    eprintln!("Using {num_threads} threads");

    let baseline_mb = working_set_mb();
    eprintln!("Baseline working set: {baseline_mb} MB");

    let success = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);

    // MappedP4k now uses Mutex<File> internally — safe for concurrent access.
    // Each thread gets its own EntityIndex (has RefCell cache, not Sync).
    let t0 = std::time::Instant::now();
    std::thread::scope(|s| {
        let chunk_size = (records.len() + num_threads - 1) / num_threads;
        let chunks: Vec<&[&Record]> = records.chunks(chunk_size).collect();

        for chunk in chunks {
            let db = &db;
            let p4k = &p4k;
            let opts = &opts;
            let success = &success;
            let errors = &errors;
            let completed = &completed;
            let output_dir = &output_dir;

            s.spawn(move || {
                let idx = EntityIndex::new(db);

                for record in chunk {
                    let i = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    let name = db.resolve_string2(record.name_offset);

                    let tree = resolve_loadout_indexed(&idx, record);
                    let result =
                        starbreaker_gltf::assemble_glb_with_loadout(db, p4k, record, &tree, opts);

                    match result {
                        Ok(res) => {
                            let filename = sanitize(name);
                            let path = std::path::PathBuf::from(output_dir)
                                .join(format!("{filename}.glb"));
                            if let Err(e) = std::fs::write(&path, &res.glb) {
                                eprintln!("[{i}/{total}] {name}: write error: {e}");
                                errors.fetch_add(1, Ordering::Relaxed);
                            } else {
                                eprintln!(
                                    "[{i}/{total}] {name} -> {} bytes  (ws: {} MB)",
                                    res.glb.len(),
                                    working_set_mb(),
                                );
                                success.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(e) => {
                            eprintln!("[{i}/{total}] {name}: {e}");
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });

    let elapsed = t0.elapsed();
    eprintln!(
        "\nDone: {} success, {} errors out of {total} in {:.1}s",
        success.load(Ordering::Relaxed),
        errors.load(Ordering::Relaxed),
        elapsed.as_secs_f64(),
    );
    eprintln!("Final working set: {} MB", working_set_mb());
}

/// Current process working set in MB (Windows: GetProcessMemoryInfo, else RSS from /proc).
fn working_set_mb() -> u64 {
    #[cfg(windows)]
    {
        use std::mem::{MaybeUninit, size_of};
        #[repr(C)]
        #[allow(non_snake_case)]
        struct ProcessMemoryCounters {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }
        unsafe extern "system" {
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                pmc: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
        }
        unsafe {
            let mut pmc = MaybeUninit::<ProcessMemoryCounters>::zeroed().assume_init();
            pmc.cb = size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
                return pmc.WorkingSetSize as u64 / (1024 * 1024);
            }
        }
        0
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            })
            .unwrap_or(0)
            / 1024
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}
