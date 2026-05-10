//! Random-read benchmarks. Auto-discovers Data.p4k via starbreaker-common;
//! respects SC_DATA_P4K override. Skips silently if no archive can be found.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use starbreaker_common::discover::find_p4k;
use starbreaker_p4k::MappedP4k;

const HOT_PATHS: &[&str] = &[
    "Data\\Libs\\CharacterCustomizer\\MasculineDefault.xml",
    "Data\\Game.dcb",
    "Data\\Scripts\\Loadouts\\Vehicles\\Default_Loadout_AEGS_Gladius.xml",
];

fn bench_lookup(c: &mut Criterion) {
    let path = match find_p4k() {
        Ok(d) => d.path,
        Err(_) => return,
    };
    let p4k = MappedP4k::open(&path).expect("open P4k");

    let mut group = c.benchmark_group("p4k_lookup");
    group.sample_size(50);
    group.bench_function("entry_case_insensitive_hit", |b| {
        b.iter(|| {
            for p in HOT_PATHS {
                black_box(p4k.entry_case_insensitive(black_box(p)));
            }
        });
    });
    group.bench_function("entry_case_insensitive_miss", |b| {
        b.iter(|| {
            black_box(p4k.entry_case_insensitive(black_box("Data\\Nope\\not_real_file.xyz")));
        });
    });
    group.finish();
}

fn bench_read(c: &mut Criterion) {
    let path = match find_p4k() {
        Ok(d) => d.path,
        Err(_) => return,
    };
    let p4k = MappedP4k::open(&path).expect("open P4k");

    // Pick a present hot path; skip the bench if none of them exist.
    let entry = HOT_PATHS
        .iter()
        .find_map(|p| p4k.entry_case_insensitive(p))
        .cloned();
    let Some(entry) = entry else {
        eprintln!("none of the bench HOT_PATHS are present in the archive; skipping read bench");
        return;
    };

    let mut group = c.benchmark_group("p4k_read");
    group.sample_size(50);
    group.bench_function("read_hot_entry", |b| {
        b.iter(|| {
            black_box(p4k.read(black_box(&entry)).expect("read"));
        });
    });
    group.finish();
}

criterion_group!(benches, bench_lookup, bench_read);
criterion_main!(benches);
