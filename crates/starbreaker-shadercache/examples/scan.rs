//! Find which cached permutations of a shader family reference a symbol.
//!
//! Reflects nothing: it searches the SPIR-V debug names for a substring, one
//! permutation per distinct `(entry, GL mask)`. Useful to answer "is this
//! material parameter actually compiled into this family, and under which
//! flags?".
//!
//! ```text
//! cargo run -p starbreaker-shadercache --example scan -- \
//!     "C:\...\Data.p4k" HardSurface DamagePerObjectWear
//! ```

use std::collections::BTreeSet;

use starbreaker_shadercache::{ShaderPak, Stage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let p4k = args.next().expect("usage: scan <Data.p4k> <family> <symbol>");
    let family = args.next().expect("family");
    let needle = args.next().expect("symbol");
    let stage = match args.next().as_deref() {
        Some("P") => Stage::Pixel,
        Some("V") => Stage::Vertex,
        Some("C") => Stage::Compute,
        _ => Stage::Other, // "Other" = any stage
    };

    let pak = ShaderPak::open_in_p4k(&p4k)?;
    let mut seen = BTreeSet::new();
    let mut scanned = 0usize;
    let mut found = 0usize;

    for perm in pak
        .permutations()
        .iter()
                .filter(|p| p.shader == family && (stage == Stage::Other || p.stage == stage))
    {
        if !seen.insert((perm.entry.clone(), perm.masks.gl)) {
            continue;
        }
        let Ok(spirv) = pak.read_spirv(perm) else {
            continue;
        };
        scanned += 1;
        if spirv.windows(needle.len()).any(|w| w == needle.as_bytes()) {
            found += 1;
            if found <= 25 {
                println!(
                    "  {:<24} gl={:#x} mdv={:?}",
                    perm.entry,
                    perm.masks.gl.unwrap_or(0),
                    perm.masks.mdv,
                );
            }
        }
    }
    if found > 25 {
        println!("  … ({} more)", found - 25);
    }
    println!("scanned {scanned} distinct (entry, GL mask) permutations, {found} reference {needle:?}");
    Ok(())
}



