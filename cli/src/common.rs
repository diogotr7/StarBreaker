use std::path::Path;

use clap::Args;
use starbreaker_p4k::MappedP4k;

use crate::error::Result;

/// Open P4k from explicit path or auto-discover.
pub fn load_p4k(p4k_path: Option<&Path>) -> Result<MappedP4k> {
    match p4k_path {
        Some(path) => Ok(MappedP4k::open(path)?),
        None => Ok(starbreaker_p4k::open_p4k()?),
    }
}

/// Load DCB bytes from explicit file or extract from P4k.
/// When dcb_path is provided, P4k is optional.
pub fn load_dcb_bytes(
    p4k_path: Option<&Path>,
    dcb_path: Option<&Path>,
) -> Result<(Option<MappedP4k>, Vec<u8>)> {
    if let Some(dcb) = dcb_path {
        let bytes = std::fs::read(dcb)
            .map_err(|e| crate::error::CliError::IoPath { source: e, path: dcb.display().to_string() })?;
        let p4k = load_p4k(p4k_path).ok();
        return Ok((p4k, bytes));
    }
    let p4k = load_p4k(p4k_path)?;
    let bytes = p4k
        .read_file("Data\\Game2.dcb")
        .or_else(|_| p4k.read_file("Data\\Game.dcb"))?;
    Ok((Some(p4k), bytes))
}

/// Shared glTF export options.
#[derive(Args, Clone)]
pub struct ExportOpts {
    /// Skip texture embedding
    #[arg(long)]
    pub no_textures: bool,
    /// Skip normal map and roughness textures
    #[arg(long)]
    pub no_normals: bool,
    /// Texture mip level (0=full, 2=1/4 res, 4=1/16 res)
    #[arg(long, default_value = "2")]
    pub mip: u32,
    /// LOD level (0=highest detail, 1+=lower)
    #[arg(long, default_value = "1")]
    pub lod: u32,
    /// Skip interior geometry from socpak containers
    #[arg(long)]
    pub no_interior: bool,
    /// Skip lights from interior socpak containers
    #[arg(long)]
    pub no_lights: bool,
    /// Skip tangent data in the GLB output
    #[arg(long)]
    pub no_tangents: bool,
    /// Skip material data (pure white geometry)
    #[arg(long)]
    pub no_materials: bool,
    /// Enable experimental texture matching (may cause specular noise on some materials)
    #[arg(long)]
    pub experimental_textures: bool,
}

impl From<&ExportOpts> for starbreaker_gltf::ExportOptions {
    fn from(opts: &ExportOpts) -> Self {
        starbreaker_gltf::ExportOptions {
            include_textures: !opts.no_textures,
            include_normals: !opts.no_normals,
            texture_mip: opts.mip,
            lod_level: opts.lod,
            include_interior: !opts.no_interior,
            include_lights: !opts.no_lights,
            include_tangents: !opts.no_tangents,
            include_materials: !opts.no_materials,
            experimental_textures: opts.experimental_textures,
        }
    }
}

/// Filter entries by glob or regex.
pub fn matches_filter(name: &str, filter: Option<&str>, regex: Option<&regex::Regex>) -> bool {
    if let Some(pattern) = filter {
        return glob_match::glob_match(pattern, name);
    }
    if let Some(re) = regex {
        return re.is_match(name);
    }
    true
}
