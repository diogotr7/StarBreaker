//! Emit the material vocabulary of a shader family as JSON.
//!
//! Reads the shipped Vulkan shader cache from `Data.p4k` at runtime and prints
//! the uniform blocks (with byte offsets and type sizes), texture bindings and
//! samplers for one permutation. This is the data an exporter needs to map
//! `.mtl` parameters and texture slots onto engine uniforms.
//!
//! ```text
//! cargo run -p starbreaker-shadercache --example matspec -- "C:\...\Data.p4k" HardSurface HardSurfacePS
//! ```

use starbreaker_shadercache::{ResourceClass, ShaderPak};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let p4k = args
        .next()
        .expect("usage: matspec <Data.p4k> [shader] [entry] [gl mask, hex]");
    let shader = args.next().unwrap_or_else(|| "HardSurface".to_owned());
    let entry = args.next().unwrap_or_else(|| "HardSurfacePS".to_owned());

    let pak = ShaderPak::open_in_p4k(&p4k)?;
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
    let r = pak.reflect(perm)?;

    let blocks: Vec<_> = [ResourceClass::PerMaterial, ResourceClass::PerView, ResourceClass::PerMesh, ResourceClass::PerObject]
        .into_iter()
        .flat_map(|class| {
            r.uniform_blocks(class).map(move |b| {
                serde_json::json!({
                    "class": format!("{class:?}"),
                    "name": b.name,
                    "binding": b.binding,
                    "block_size": b.block_size,
                    "members": b.members,
                })
            })
        })
        .collect();

    let textures: Vec<_> = [
        ResourceClass::PerMaterial,
        ResourceClass::PerView,
        ResourceClass::PerMesh,
        ResourceClass::PerObject,
    ]
    .into_iter()
    .flat_map(|class| {
        r.textures(class).map(move |b| {
                serde_json::json!({
                    "class": format!("{class:?}"),
                    "name": b.name,
                    "binding": b.binding,
                    "storage": b.storage_class,
                })
            })
        })
        .collect();

    let doc = serde_json::json!({
        "shader": perm.shader,
        "entry": perm.entry,
        "stage": format!("{:?}", perm.stage),
        "masks": perm.masks,
        "entry_points": r.entry_points,
        "uniform_blocks": blocks,
        "textures": textures,
    });
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}
