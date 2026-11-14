//! Open the shipped Vulkan shader cache and print one permutation's reflection.
//!
//! ```text
//! cargo run -p starbreaker-shadercache --example reflect -- "C:\...\Data.p4k" HardSurface HardSurfacePS
//! ```

use starbreaker_shadercache::ShaderPak;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let p4k = args
        .next()
        .expect("usage: reflect <Data.p4k> [shader] [entry] [gl mask, hex]");
    let shader = args.next().unwrap_or_else(|| "HardSurface".to_owned());
    let entry = args.next().unwrap_or_else(|| "HardSurfacePS".to_owned());

    let pak = ShaderPak::open_in_p4k(&p4k)?;
    println!("indexed {} permutations", pak.len());

    let gl = args
        .next()
        .map(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16))
        .transpose()?;
    let variants: Vec<_> = pak.variants(&shader, &entry).collect();
    let perm = variants
        .iter()
        .copied()
        .find(|p| gl.is_none_or(|g| p.masks.gl == Some(g)))
        .ok_or_else(|| format!("no permutation for {shader}@{entry} gl={gl:x?}"))?;
    eprintln!("{} variants of {shader}@{entry}; using {}", variants.len(), perm.path);
    println!(
        "{}@{} stage={:?} masks={:?}",
        perm.shader, perm.entry, perm.stage, perm.masks
    );

    let reflection = pak.reflect(perm)?;
    println!(
        "entry points: {:?}",
        reflection
            .entry_points
            .iter()
            .map(|e| &e.name)
            .collect::<Vec<_>>()
    );
    for set in reflection.sets() {
        println!("set {set}:");
        for b in reflection.bindings.iter().filter(|b| b.set == Some(set)) {
            println!(
                "  b{:<4} {:<15} {}{}",
                b.binding.map(|v| v.to_string()).unwrap_or_default(),
                b.storage_class,
                b.name,
                if b.members.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  {{{}}}",
                        b.members
                            .iter()
                            .map(|m| m.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            );
        }
    }
    Ok(())
}

