use starbreaker_datacore::Database;
use starbreaker_datacore::query::compile::CompiledMultiPath;
/// Dump alpha damage for all ship weapons in the DataCore.
///
/// Demonstrates the rooted query API — no struct indices needed:
///
/// ```text
/// db.compile_multi_rooted::<f32>(
///     "EntityClassDefinition.Components[SAmmoContainerComponentParams]\
///      .ammoParamsRecord.projectileParams[BulletProjectileParams].damage[DamageInfo]",
///     &["DamagePhysical", "DamageEnergy", "DamageDistortion",
///       "DamageThermal", "DamageBiochemical", "DamageStun"]
/// )
/// ```
///
/// Usage: cargo run --release --example weapon_alpha [path.dcb]
use std::{env, fs, time::Instant};

fn main() {
    let t0 = Instant::now();
    let data = if let Some(dcb_path) = env::args().nth(1) {
        fs::read(&dcb_path).expect("failed to read DCB file")
    } else {
        let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
        p4k.read_file("Data\\Game2.dcb")
            .expect("failed to read Game2.dcb")
    };
    let t_read = t0.elapsed();

    let t1 = Instant::now();
    let db = Database::from_bytes(&data).expect("failed to parse");
    let t_parse = t1.elapsed();

    println!("Read: {t_read:?}, Parse: {t_parse:?}");
    println!(
        "Records: {}, Structs: {}",
        db.records().len(),
        db.struct_defs().len()
    );

    let t2 = Instant::now();
    let weapons = collect_weapon_damage(&db);
    let t_query = t2.elapsed();

    let mut weapons = weapons;
    weapons.sort_by(|a, b| b.alpha.partial_cmp(&a.alpha).unwrap());

    println!(
        "\n{:<55} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}  {}",
        "Weapon", "Alpha", "Phys", "Energy", "Distort", "Thermal", "Biochem", "Type"
    );
    println!("{}", "-".repeat(125));

    for w in &weapons {
        println!(
            "{:<55} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}  {}",
            w.name, w.alpha, w.dmg[0], w.dmg[1], w.dmg[2], w.dmg[3], w.dmg[4], w.proj_type
        );
    }

    println!("\n{} weapons found in {t_query:?}", weapons.len());
}

struct Weapon {
    name: String,
    proj_type: &'static str,
    alpha: f64,
    dmg: [f64; 6],
}

const DAMAGE_FIELDS: &[&str] = &[
    "DamagePhysical",
    "DamageEnergy",
    "DamageDistortion",
    "DamageThermal",
    "DamageBiochemical",
    "DamageStun",
];

fn collect_weapon_damage(db: &Database) -> Vec<Weapon> {
    struct Strategy {
        label: &'static str,
        proj_label: &'static str,
        path: CompiledMultiPath,
    }

    // Rooted multi-paths — type name is the first segment, no struct indices anywhere.
    let configs: &[(&str, &str, &str)] = &[
        (
            "direct",
            "Bullet",
            "EntityClassDefinition.Components[SAmmoContainerComponentParams].ammoParamsRecord.projectileParams[BulletProjectileParams].damage[DamageInfo]",
        ),
        (
            "direct",
            "Tachyon",
            "EntityClassDefinition.Components[SAmmoContainerComponentParams].ammoParamsRecord.projectileParams[TachyonProjectileParams].damage[DamageInfo]",
        ),
        (
            "indirect",
            "Bullet",
            "EntityClassDefinition.Components[SCItemWeaponComponentParams].ammoContainerRecord.Components[SAmmoContainerComponentParams].ammoParamsRecord.projectileParams[BulletProjectileParams].damage[DamageInfo]",
        ),
        (
            "indirect",
            "Tachyon",
            "EntityClassDefinition.Components[SCItemWeaponComponentParams].ammoContainerRecord.Components[SAmmoContainerComponentParams].ammoParamsRecord.projectileParams[TachyonProjectileParams].damage[DamageInfo]",
        ),
    ];

    let mut strategies = Vec::new();
    for &(label, proj_label, prefix) in configs {
        match db.compile_multi_rooted::<f32>(prefix, DAMAGE_FIELDS) {
            Ok(path) => {
                println!("  {label}/{proj_label}: compiled OK");
                strategies.push(Strategy {
                    label,
                    proj_label,
                    path,
                });
            }
            Err(e) => eprintln!("  {label}/{proj_label}: FAILED: {e}"),
        }
    }

    // All strategies share the same root type, so use the first one's root_struct_index.
    let entity_si = match strategies.first() {
        Some(s) => s.path.root_struct_id(),
        None => {
            eprintln!("No strategies compiled");
            return Vec::new();
        }
    };

    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut entity_count = 0;

    for record in db.records_of_type(entity_si) {
        entity_count += 1;

        for s in &strategies {
            let values = match db.query_multi::<f32>(&s.path, record) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let mut dmg = [0.0f64; 6];
            let mut any = false;
            for (i, val) in values.iter().enumerate() {
                if let Some(v) = val {
                    dmg[i] = *v as f64;
                    if *v > 0.001 {
                        any = true;
                    }
                }
            }
            if !any {
                continue;
            }

            let alpha: f64 = dmg[..5].iter().sum();
            let record_name = db.resolve_string2(record.name_offset);
            let short_name = record_name.rsplit('.').next().unwrap_or(record_name);
            let key = (short_name.to_string(), s.proj_label);
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);

            results.push(Weapon {
                name: format!("{short_name} [{}]", s.label),
                proj_type: s.proj_label,
                alpha,
                dmg,
            });
        }
    }

    println!("  Scanned {entity_count} entities");
    results
}
