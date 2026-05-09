//! Search benchmarks. Auto-discovers Data.p4k via starbreaker-common; respects
//! SC_DATA_P4K override. Skips silently if no archive can be found.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use starbreaker_common::discover::find_p4k;
use starbreaker_p4k::MappedP4k;

fn bench_search(c: &mut Criterion) {
    let path = match find_p4k() {
        Ok(d) => {
            eprintln!("Using P4k from {}: {}", d.source, d.path.display());
            d.path
        }
        Err(e) => {
            eprintln!("No Data.p4k found ({e}); skipping search bench");
            return;
        }
    };

    let p4k = MappedP4k::open(&path).expect("open P4k");
    let mut group = c.benchmark_group("p4k_search");
    group.sample_size(20);

    let cases: &[(&str, &str)] = &[
        ("empty", ""),                  // empty (early-out)
        ("single_letter_a", "a"),       // very wide match
        ("ext_mtl", "mtl"),             // common extension fragment
        ("prefix_data", "data"),        // common prefix
        ("hornet", "hornet"),           // medium frequency
        ("multi_hornet_ship", "hornet ship"), // multi-token AND
    ];

    for (name, query) in cases {
        group.bench_function(*name, |b| {
            b.iter(|| {
                let results = p4k.search(black_box(*query));
                black_box(results);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
