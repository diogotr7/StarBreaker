use criterion::{Criterion, criterion_group, criterion_main};
use std::env;

fn benchmarks(c: &mut Criterion) {
    let path = match env::var("DCB_PATH") {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "Set DCB_PATH env var to run benchmarks (e.g., DCB_PATH=Game.dcb cargo bench)"
            );
            return;
        }
    };

    let data = std::fs::read(&path).expect("failed to read DCB");

    c.bench_function("parse", |b| {
        b.iter(|| starbreaker_datacore::Database::from_bytes(criterion::black_box(&data)).unwrap())
    });

    let db = starbreaker_datacore::Database::from_bytes(&data).unwrap();

    // Bench all export formats on the largest record
    let target_name = "ContractGenerator.HeadHunters_Mercenary_FPS";
    let big_record = db.records().iter().find(|r| {
        db.is_main_record(r) && db.resolve_string2(r.name_offset) == target_name
    });

    if let Some(record) = big_record {
        let json_bytes = starbreaker_datacore::export::to_json(&db, record).unwrap();
        eprintln!(
            "Benchmarking record: {target_name} ({:.1} MB JSON)",
            json_bytes.len() as f64 / 1_000_000.0
        );

        c.bench_function("export_json", |b| {
            b.iter(|| {
                starbreaker_datacore::export::to_json(
                    criterion::black_box(&db),
                    criterion::black_box(record),
                )
                .unwrap()
            })
        });

        c.bench_function("export_xml", |b| {
            b.iter(|| {
                starbreaker_datacore::export::to_xml(
                    criterion::black_box(&db),
                    criterion::black_box(record),
                )
                .unwrap()
            })
        });

        c.bench_function("export_unp4k_xml", |b| {
            b.iter(|| {
                starbreaker_datacore::export::to_unp4k_xml(
                    criterion::black_box(&db),
                    criterion::black_box(record),
                )
                .unwrap()
            })
        });
    } else {
        eprintln!("Record '{target_name}' not found, skipping export benchmarks.");
    }

    // Find an EntityClassDefinition record with geometry (ship/vehicle)
    let entity_record = db.records().iter().find(|r| {
        let struct_name = db.struct_name(r.struct_id());
        if struct_name != "EntityClassDefinition" {
            return false;
        }
        // Try to compile the geometry path for this struct
        db.compile_path::<String>(
            r.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
        )
        .is_ok()
    });

    if let Some(entity) = entity_record {
        let si = entity.struct_id();
        let entity_name = db.resolve_string2(entity.name_offset);
        eprintln!("Benchmarking queries against: {entity_name}");

        // Benchmark: compile_path (includes type filter resolution)
        c.bench_function("compile_path_geometry", |b| {
            b.iter(|| {
                db.compile_path::<String>(
                    criterion::black_box(si),
                    criterion::black_box(
                        "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
                    ),
                )
                .unwrap()
            })
        });

        let geom_path = db
            .compile_path::<String>(
                si,
                "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
            )
            .unwrap();

        // Benchmark: query_one with deep nested path (exercises skip_properties)
        c.bench_function("query_geometry_path", |b| {
            b.iter(|| {
                db.query::<String>(
                    criterion::black_box(&geom_path),
                    criterion::black_box(entity),
                )
                .unwrap()
            })
        });

        // Benchmark: Value materialization (exercises skip + materialize)
        let value_path = db
            .compile_path::<starbreaker_datacore::query::value::Value>(
                si,
                "Components[SGeometryResourceParams]",
            )
            .unwrap();

        c.bench_function("query_value_subtree", |b| {
            b.iter(|| {
                db.query::<starbreaker_datacore::query::value::Value>(
                    criterion::black_box(&value_path),
                    criterion::black_box(entity),
                )
                .unwrap()
            })
        });

        // Benchmark: loadout resolution (exercises the full pipeline)
        c.bench_function("resolve_loadout", |b| {
            b.iter(|| {
                starbreaker_datacore::loadout::resolve_loadout(
                    criterion::black_box(&db),
                    criterion::black_box(entity),
                )
            })
        });
    }
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
