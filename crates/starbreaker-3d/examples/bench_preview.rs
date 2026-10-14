//! Times the character-builder preview compute: context load (once per gender)
//! and `HeadIdentityContext::build_glb` (head-only and full), reporting GLB
//! byte sizes. Isolates the Rust render cost from the Tauri IPC + frontend so
//! we can see "where the time goes" in the live preview.
//!
//! Usage: cargo run --release -p starbreaker-3d --example bench_preview

use std::time::Instant;

use starbreaker_3d::character::HeadIdentityContext;
use starbreaker_chf::ChfFile;
use starbreaker_p4k::MappedP4k;

const DEFAULT_P4K: &str =
    r"C:\Program Files\Roberts Space Industries\StarCitizen\LIVE\Data.p4k";
const FEMININE: &str = r"Data\Libs\CharacterCustomizer\femininedefault.chf";

fn main() {
    let p4k = MappedP4k::open(
        &std::env::var("SC_DATA_P4K").unwrap_or_else(|_| DEFAULT_P4K.to_string()),
    )
    .expect("open p4k");

    let chf_bytes = p4k.read_file(FEMININE).expect("femininedefault.chf");
    let chf = ChfFile::from_chf(&chf_bytes).expect("chf");
    let dna = chf.parse().expect("parse").dna;

    // Context load (paid once per gender, then cached in the app).
    let t = Instant::now();
    let ctx = HeadIdentityContext::load(&p4k, dna.gender_hash).expect("ctx");
    println!("context load (once/gender): {:>7.1} ms", t.elapsed().as_secs_f64() * 1000.0);

    // Warm + repeated build to get a stable per-preview figure.
    // mt=false is the live PREVIEW path (no morph targets); mt=true is EXPORT.
    for full in [false, true] {
        for mt in [false, true] {
            let label = format!(
                "{:9} mt={}",
                if full { "full" } else { "head-only" },
                if mt { "on " } else { "off" }
            );
            let warm = ctx.build_glb(&dna, full, mt).expect("build");
            let mut best = f64::MAX;
            let mut total = 0.0;
            const N: u32 = 8;
            for _ in 0..N {
                let t = Instant::now();
                let glb = ctx.build_glb(&dna, full, mt).expect("build");
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                best = best.min(ms);
                total += ms;
                std::hint::black_box(&glb);
            }
            println!(
                "build_glb {label}: best {best:>6.1} ms  avg {:>6.1} ms  | GLB {:>7.1} KB (= {} bytes)",
                total / N as f64,
                warm.len() as f64 / 1024.0,
                warm.len(),
            );
        }
    }
}
