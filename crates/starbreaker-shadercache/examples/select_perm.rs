//! Pick the shader permutation that matches a material's generation flags.
//!
//! The material carries a `%FLAG` string (`string_gen_mask`). The shipped `.ext`
//! files map every flag to a bit of the shader generation mask, and every cached
//! permutation name carries the `(GL…)` mask it was built with. So choosing the
//! right `.shcb` is a subset match: the permutation must include every bit the
//! material asks for, and should add as few others as possible.
//!
//! ```text
//! cargo run -p starbreaker-shadercache --example select_perm -- \
//!     "C:\...\Data.p4k" "Data\Objects\Spaceships\Ships\AEGS\Gladius\AEGS_Gladius.mtl" HardSurface
//! ```

use std::error::Error;

use starbreaker_p4k::MappedP4k;
use starbreaker_shadercache::{EXT_PAK, GenMasks, ShaderPak, Stage, read_pak_entries};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let p4k_path = args
        .next()
        .expect("usage: select_perm <Data.p4k> <mtl path inside p4k> [family]");
    let mtl_path = args.next().expect("mtl path");
    let want_family = args.next();

    let p4k = MappedP4k::open(&p4k_path)?;
    let bytes = p4k.read_file(&mtl_path)?;
    let mtl = starbreaker_3d::mtl::parse_mtl(&bytes)?;
    let sub = mtl
        .materials
        .iter()
        .filter(|m| m.shader != "NoDraw")
        .filter(|m| want_family.as_deref().is_none_or(|w| m.shader.contains(w)))
        .max_by_key(|m| m.public_params.len())
        .or_else(|| mtl.materials.first())
        .ok_or("material file has no submaterials")?;

    let decoded = sub.decoded_string_gen_mask();
    let family = sub.shader_family().as_str().to_owned();
    println!("material : {} ({})", sub.name, sub.shader);
    println!("flags    : {:?}", decoded.tokens);

    // Flag -> bit table from the shipped .ext files.
    let ext = read_pak_entries(&p4k_path, EXT_PAK)?;
    let ext_files: Vec<(String, String)> = ext
        .iter()
        .map(|(name, data)| (name.clone(), String::from_utf8_lossy(data).into_owned()))
        .collect();
    let masks = GenMasks::from_ext_files(ext_files.iter().map(|(n, t)| (n.as_str(), t.as_str())));
    let genmask = masks
        .family(&family)
        .ok_or_else(|| format!("no .ext for shader family {}", family))?;
    println!("genmask  : {} flags known from {family}.ext", genmask.len());

    let (want, unknown) = genmask.mask_for(&decoded.tokens);
    println!("expected : {want:#x}");
    if !unknown.is_empty() {
        println!("unmapped flags: {unknown:?}");
    }
    println!();

    let pak = ShaderPak::open_in_p4k(&p4k_path)?;
    let mut candidates: Vec<(&str, u64, u32, u32)> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for perm in pak
        .permutations()
        .iter()
        .filter(|p| p.shader == family && p.stage == Stage::Pixel)
    {
        let gl = perm.masks.gl.unwrap_or(0);
        if !seen.insert((perm.entry.clone(), gl)) {
            continue;
        }
        let missing = (want & !gl).count_ones();
        let extra = (gl & !want).count_ones();
        candidates.push((perm.entry.as_str(), gl, missing, extra));
    }
    candidates.sort_by_key(|(entry, gl, missing, extra)| (*missing, *extra, *gl, *entry));

    println!("{:>4}  {:<8}  {:<18} entry", "miss", "extra", "gl");
    for (entry, gl, missing, extra) in candidates.iter().take(10) {
        println!("{missing:>4}  {extra:<8}  {gl:<18x} {entry}");
    }
    println!("({} distinct entry/mask combinations)", candidates.len());

    let (entry, gl, missing, _) = candidates[0];
    println!();
    println!("chosen   : {entry} gl={gl:#x} missing={missing}");
    let perm = pak
        .permutations()
        .iter()
        .find(|p| p.shader == family && p.entry == entry && p.masks.gl.unwrap_or(0) == gl)
        .expect("candidate came from the list");
    let reflection = pak.reflect(perm)?;
    let members: Vec<&str> = reflection
        .uniform_blocks(starbreaker_shadercache::ResourceClass::PerMaterial)
        .flat_map(|b| b.members.iter().map(|m| m.name.as_str()))
        .collect();
    println!("per-material uniforms ({}): {}", members.len(), members.join(", "));
    let pom = members.iter().any(|m| m.to_ascii_lowercase().contains("pom"))
        || members.iter().any(|m| m.to_ascii_lowercase().contains("parallax"));
    println!("POM uniforms present: {pom}");

    Ok(())
}

