//! Reconstruct a shader family's `.cfxb` descriptor as readable HLSL.
//!
//! Uses the grammar shipped for the bin's cache version; a bin from any other
//! game build is refused with both versions named.
//!
//! ```text
//! cargo run -p starbreaker-shadercache --example reconstruct -- \
//!     "C:\...\Data.p4k" Decal 40
//! ```

use starbreaker_shadercache::{ShaderPak, cfxb, hlsl};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let p4k = args
        .next()
        .expect("usage: reconstruct <Data.p4k> <Shader> [lines]");
    let shader = args.next().unwrap_or_else(|| "Decal".to_owned());
    let max_lines: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(60);

    let pak = ShaderPak::open_in_p4k(&p4k)?;
    let bin = cfxb::parse(&pak.read_shader_bin(&shader)?)?;
    let text = hlsl::decode(&bin)?;

    println!(
        "// {shader}.cfxb  api={} cache_version={:#010x} tokens={} table={}",
        pak.api(),
        bin.version,
        bin.tokens.len(),
        bin.table.len()
    );
    for line in text.lines().take(max_lines) {
        println!("{line}");
    }
    Ok(())
}
