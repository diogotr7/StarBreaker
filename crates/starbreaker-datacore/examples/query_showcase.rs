use starbreaker_datacore::Database;
use starbreaker_datacore::query::compile::CompiledPath;
use starbreaker_datacore::types::Record;
/// Showcase of DataCore queries across different game systems.
///
/// Each section shows what the future XPath-style syntax would look like,
/// then implements it with the current API.
///
/// Usage: cargo run --release --example query_showcase [path.dcb]
use std::{env, fs, time::Instant};

fn main() {
    let data = if let Some(dcb_path) = env::args().nth(1) {
        fs::read(&dcb_path).expect("failed to read DCB file")
    } else {
        let p4k = starbreaker_p4k::open_p4k().expect("failed to find Data.p4k");
        p4k.read_file("Data\\Game2.dcb")
            .expect("failed to read Game2.dcb")
    };
    let db = Database::from_bytes(&data).expect("failed to parse");

    shields(&db);
    quantum_drives(&db);
    power_plants(&db);
    commodities(&db);
    missions(&db);
    weapon_fire_rates(&db);
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn q_f32(db: &Database, p: &CompiledPath, r: &Record) -> Option<f32> {
    db.query_single::<f32>(p, r).ok().flatten()
}

fn q_str<'a>(db: &'a Database<'a>, p: &CompiledPath, r: &Record) -> Option<String> {
    db.query_single::<String>(p, r).ok().flatten()
}

fn q_i32(db: &Database, p: &CompiledPath, r: &Record) -> Option<i32> {
    db.query_single::<i32>(p, r).ok().flatten()
}

fn short_name<'a>(db: &'a Database<'a>, r: &Record) -> &'a str {
    let full = db.resolve_string2(r.name_offset);
    full.rsplit('.').next().unwrap_or(full)
}

// ─── 1. Shield Generators ────────────────────────────────────────────────────

fn shields(db: &Database) {
    // Future syntax:
    //   EntityClassDefinition
    //     .Components[SCItemShieldGeneratorParams]
    //     .{MaxShieldHealth, MaxShieldRegen, DownedRegenDelay, DamagedRegenDelay}

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Shield Generators                                             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let shield = db
        .compile_multi_rooted::<f32>(
            "EntityClassDefinition.Components[SCItemShieldGeneratorParams]",
            &[
                "MaxShieldHealth",
                "MaxShieldRegen",
                "DownedRegenDelay",
                "DamagedRegenDelay",
            ],
        )
        .unwrap();

    let t = Instant::now();

    let mut rows: Vec<_> = db
        .records_of_type(shield.root_struct_id())
        .filter_map(|r| {
            let vals = db.query_multi::<f32>(&shield, r).ok()?;
            let h = vals[0]?;
            if h <= 0.0 {
                return None;
            }
            Some((
                short_name(db, r).to_string(),
                h,
                vals[1].unwrap_or(0.0),
                vals[2].unwrap_or(0.0),
                vals[3].unwrap_or(0.0),
            ))
        })
        .collect();

    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    println!(
        "{:<45} {:>10} {:>10} {:>12} {:>12}",
        "Shield", "HP", "Regen/s", "Down Delay", "Dmg Delay"
    );
    println!("{}", "-".repeat(91));
    for (name, h, rg, dd, dmd) in &rows {
        println!(
            "{:<45} {:>10.0} {:>10.1} {:>12.1} {:>12.1}",
            name, h, rg, dd, dmd
        );
    }
    println!("{} shields in {:?}\n", rows.len(), t.elapsed());
}

// ─── 2. Quantum Drives ──────────────────────────────────────────────────────

fn quantum_drives(db: &Database) {
    // Future syntax:
    //   EntityClassDefinition
    //     .Components[SCItemQuantumDriveParams]
    //     .{quantumFuelRequirement, jumpRange, params.spoolUpTime}

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Quantum Drives                                                ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let fuel = db
        .compile_rooted::<f32>(
            "EntityClassDefinition.Components[SCItemQuantumDriveParams].quantumFuelRequirement",
        )
        .unwrap();
    let range = db
        .compile_rooted::<f32>(
            "EntityClassDefinition.Components[SCItemQuantumDriveParams].jumpRange",
        )
        .unwrap();
    let spool = db
        .compile_rooted::<f32>(
            "EntityClassDefinition.Components[SCItemQuantumDriveParams].params.spoolUpTime",
        )
        .unwrap();

    let t = Instant::now();

    let mut rows: Vec<_> = db
        .records_of_type(fuel.root_struct_id())
        .filter_map(|r| {
            let f = q_f32(db, &fuel, r)?;
            if f <= 0.0 {
                return None;
            }
            Some((
                short_name(db, r).to_string(),
                f,
                q_f32(db, &range, r).unwrap_or(0.0),
                q_f32(db, &spool, r).unwrap_or(0.0),
            ))
        })
        .collect();

    rows.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    println!(
        "{:<45} {:>14} {:>14} {:>12}",
        "Quantum Drive", "Fuel Req", "Range", "Spool Time"
    );
    println!("{}", "-".repeat(87));
    for (name, f, rng, sp) in &rows {
        println!("{:<45} {:>14.4} {:>14.0} {:>12.1}", name, f, rng, sp);
    }
    println!("{} quantum drives in {:?}\n", rows.len(), t.elapsed());
}

// ─── 3. Power Plants ────────────────────────────────────────────────────────

fn power_plants(db: &Database) {
    // Future syntax:
    //   EntityClassDefinition
    //     .Components[SCItemPowerPlantComponentParams]
    //     .MaxPowerGeneration

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Power Plants                                                  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let power = match db.compile_rooted::<f32>(
        "EntityClassDefinition.Components[SCItemPowerPlantComponentParams].MaxPowerGeneration",
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("  Power plant path failed: {e}");
            return;
        }
    };

    let t = Instant::now();

    let mut rows: Vec<_> = db
        .records_of_type(power.root_struct_id())
        .filter_map(|r| {
            let p = q_f32(db, &power, r)?;
            if p <= 0.0 {
                return None;
            }
            Some((short_name(db, r).to_string(), p))
        })
        .collect();

    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    println!("{:<55} {:>14}", "Power Plant", "Max Power");
    println!("{}", "-".repeat(70));
    for (name, p) in &rows {
        println!("{:<55} {:>14.0}", name, p);
    }
    println!("{} power plants in {:?}\n", rows.len(), t.elapsed());
}

// ─── 4. Commodities ─────────────────────────────────────────────────────────

fn commodities(db: &Database) {
    // Future syntax:
    //   CommoditySubtype.{name, symbol, volatility, commodity.type.typeName}

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Commodities                                                   ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let name_path = db
        .compile_rooted::<String>("CommoditySubtype.name")
        .unwrap();
    let symbol_path = db
        .compile_rooted::<String>("CommoditySubtype.symbol")
        .unwrap();
    let vol_path = db
        .compile_rooted::<f32>("CommoditySubtype.volatility")
        .unwrap();
    // Cross-reference: CommoditySubtype → commodity (Ref) → type (Ref) → typeName
    let type_path = db
        .compile_rooted::<String>("CommoditySubtype.commodity.type.typeName")
        .unwrap();

    let si = name_path.root_struct_id();
    let t = Instant::now();

    let mut rows: Vec<_> = db
        .records_of_type(si)
        .filter_map(|r| {
            let name = q_str(db, &name_path, r)?;
            if name.is_empty() {
                return None;
            }
            Some((
                name,
                q_str(db, &symbol_path, r).unwrap_or_default(),
                q_f32(db, &vol_path, r).unwrap_or(0.0),
                q_str(db, &type_path, r).unwrap_or_default(),
            ))
        })
        .collect();

    rows.sort_by(|a, b| a.3.cmp(&b.3).then(a.0.cmp(&b.0)));

    println!(
        "{:<35} {:>8} {:>10} {:<20}",
        "Commodity", "Symbol", "Volatility", "Type"
    );
    println!("{}", "-".repeat(76));
    for (name, sym, vol, typ) in &rows {
        println!("{:<35} {:>8} {:>10.2} {:<20}", name, sym, vol, typ);
    }
    println!("{} commodities in {:?}\n", rows.len(), t.elapsed());
}

// ─── 5. Missions ────────────────────────────────────────────────────────────

fn missions(db: &Database) {
    // Future syntax:
    //   MissionBrokerEntry
    //     .{title, missionDifficulty, type.typeName, missionGiverRecord.name}

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Missions (by difficulty)                                      ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let title_path = db
        .compile_rooted::<String>("MissionBrokerEntry.title")
        .unwrap();
    let diff_path = db
        .compile_rooted::<i32>("MissionBrokerEntry.missionDifficulty")
        .unwrap();
    let type_path = match db.compile_rooted::<String>("MissionBrokerEntry.type.typeName") {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("  type.typeName failed: {e}");
            None
        }
    };
    let giver_path = match db.compile_rooted::<String>("MissionBrokerEntry.missionGiverRecord.name")
    {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("  missionGiverRecord.name failed: {e}");
            None
        }
    };

    let si = title_path.root_struct_id();
    let t = Instant::now();

    let mut rows: Vec<_> = db
        .records_of_type(si)
        .filter(|r| db.is_main_record(r))
        .filter_map(|r| {
            let title = q_str(db, &title_path, r).unwrap_or_default();
            if title.is_empty() {
                return None;
            }
            let diff = q_i32(db, &diff_path, r).unwrap_or(0);
            let typ = type_path
                .as_ref()
                .and_then(|p| q_str(db, p, r))
                .unwrap_or_default();
            let giver = giver_path
                .as_ref()
                .and_then(|p| q_str(db, p, r))
                .unwrap_or_default();
            Some((title, diff, typ, giver))
        })
        .collect();

    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    println!(
        "{:<50} {:>5} {:<20} {:<20}",
        "Title", "Diff", "Type", "Giver"
    );
    println!("{}", "-".repeat(98));
    for (title, diff, typ, giver) in rows.iter().take(60) {
        let title_short = if title.len() > 48 {
            &title[..48]
        } else {
            title
        };
        let typ_short = if typ.len() > 18 { &typ[..18] } else { typ };
        let giver_short = if giver.len() > 18 {
            &giver[..18]
        } else {
            giver
        };
        println!(
            "{:<50} {:>5} {:<20} {:<20}",
            title_short, diff, typ_short, giver_short
        );
    }
    println!(
        "{} missions total (showing top 60) in {:?}\n",
        rows.len(),
        t.elapsed()
    );
}

// ─── 6. Weapon Fire Rates ───────────────────────────────────────────────────

fn weapon_fire_rates(db: &Database) {
    // Future syntax:
    //   EntityClassDefinition
    //     .Components[SCItemWeaponComponentParams]
    //     .fireActions[SWeaponActionFireSingleParams]
    //     .fireRate

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Weapon Fire Rates                                             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    // fireActions is a polymorphic StrongPointer array — try both fire action types
    let single_path = db.compile_rooted::<i32>(
        "EntityClassDefinition.Components[SCItemWeaponComponentParams].fireActions[SWeaponActionFireSingleParams].fireRate");
    let rapid_path = db.compile_rooted::<i32>(
        "EntityClassDefinition.Components[SCItemWeaponComponentParams].fireActions[SWeaponActionFireRapidParams].fireRate");

    // Get entity_si from whichever path compiled successfully
    let entity_si = single_path
        .as_ref()
        .ok()
        .map(|p| p.root_struct_id())
        .or_else(|| rapid_path.as_ref().ok().map(|p| p.root_struct_id()));

    let t = Instant::now();
    let mut rows = Vec::new();

    if let Some(si) = entity_si {
        for r in db.records_of_type(si) {
            let name = short_name(db, r);

            // Try single-fire
            if let Ok(ref p) = single_path {
                if let Some(rate) = q_i32(db, p, r) {
                    if rate > 0 {
                        rows.push((name.to_string(), rate, "Single"));
                    }
                }
            }
            // Try rapid-fire
            if let Ok(ref p) = rapid_path {
                if let Some(rate) = q_i32(db, p, r) {
                    if rate > 0 {
                        rows.push((name.to_string(), rate, "Rapid"));
                    }
                }
            }
        }
    }

    rows.sort_by(|a, b| b.1.cmp(&a.1));

    println!("{:<50} {:>10} {:<10}", "Weapon", "RPM", "Mode");
    println!("{}", "-".repeat(72));
    for (name, rate, mode) in rows.iter().take(50) {
        println!("{:<50} {:>10} {:<10}", name, rate, mode);
    }
    println!(
        "{} weapons with fire rate (showing top 50) in {:?}\n",
        rows.len(),
        t.elapsed()
    );
}
