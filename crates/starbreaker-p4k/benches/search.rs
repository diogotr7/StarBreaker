//! Search benchmarks. Skips silently if SC_DATA_P4K is not set so CI can run them.

use std::path::PathBuf;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use starbreaker_p4k::MappedP4k;

fn locate_p4k() -> Option<PathBuf> {
    std::env::var_os("SC_DATA_P4K").map(PathBuf::from)
}

fn bench_search(c: &mut Criterion) {
    let Some(path) = locate_p4k() else {
        eprintln!("SC_DATA_P4K not set; skipping search bench");
        return;
    };

    let p4k = MappedP4k::open(&path).expect("open P4k");
    let mut group = c.benchmark_group("p4k_search");
    group.sample_size(20);

    for query in [
        "",            // empty (early-out)
        "a",           // single letter, very wide match
        "mtl",         // common extension fragment
        "data",        // common prefix
        "hornet",      // medium frequency
        "hornet ship", // multi-token AND
    ] {
        group.bench_function(query, |b| {
            b.iter(|| {
                let results = p4k.search(black_box(query));
                black_box(results);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
