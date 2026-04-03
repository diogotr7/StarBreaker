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
///
/// For glob patterns, both the pattern and name are normalized to forward
/// slashes before matching — P4k entries use backslashes internally but
/// users shouldn't have to care.
pub fn matches_filter(name: &str, filter: Option<&str>, regex: Option<&regex::Regex>) -> bool {
    if let Some(pattern) = filter {
        let norm_name = name.replace('\\', "/");
        let norm_pattern = pattern.replace('\\', "/");
        return glob_match::glob_match(&norm_pattern, &norm_name);
    }
    if let Some(re) = regex {
        return re.is_match(name);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // Typical P4k entry paths (backslash-separated, rooted at Data\).
    const XML_DEEP: &str = r"Data\Libs\Subsumption\Missions\mission.xml";
    const XML_SHALLOW: &str = r"Data\game.xml";
    const DDS_DEEP: &str = r"Data\Objects\ships\aurora\texture.dds";
    const DDS_SIBLING: &str = r"Data\Objects\ships\aurora\texture.dds.1";
    const CGF_DEEP: &str = r"Data\Objects\ships\aurora\model.cgf";

    // -----------------------------------------------------------------------
    // matches_filter — glob: extension wildcards
    // -----------------------------------------------------------------------

    #[test]
    fn glob_star_xml_only_matches_root_level() {
        // `*` does NOT cross path separators, so `*.xml` only matches names
        // with no directory component. This is correct glob semantics.
        assert!(!matches_filter(XML_DEEP, Some("*.xml"), None));
        assert!(!matches_filter(XML_SHALLOW, Some("*.xml"), None));
    }

    #[test]
    fn glob_star_dds_only_matches_root_level() {
        assert!(!matches_filter(DDS_DEEP, Some("*.dds"), None));
    }

    #[test]
    fn glob_doublestar_xml_matches_all_depths() {
        // `**/*.xml` matches .xml files at any depth.
        assert!(matches_filter(XML_DEEP, Some("**/*.xml"), None));
        assert!(matches_filter(XML_SHALLOW, Some("**/*.xml"), None));
    }

    #[test]
    fn glob_doublestar_dds_matches_all_depths() {
        assert!(matches_filter(DDS_DEEP, Some("**/*.dds"), None));
    }

    #[test]
    fn glob_doublestar_dds_excludes_siblings() {
        // `.dds.1` is NOT a `.dds` file.
        assert!(!matches_filter(DDS_SIBLING, Some("**/*.dds"), None));
    }

    // -----------------------------------------------------------------------
    // matches_filter — glob: backslash patterns work (normalized)
    // -----------------------------------------------------------------------

    #[test]
    fn glob_backslash_doublestar_works() {
        // Users on Windows may type backslashes — should work identically.
        assert!(matches_filter(XML_DEEP, Some(r"**\*.xml"), None));
        assert!(matches_filter(XML_SHALLOW, Some(r"**\*.xml"), None));
        assert!(matches_filter(DDS_DEEP, Some(r"**\*.dds"), None));
    }

    // -----------------------------------------------------------------------
    // matches_filter — glob: exact paths & prefixes
    // -----------------------------------------------------------------------

    #[test]
    fn glob_exact_path_with_backslashes() {
        assert!(matches_filter(
            XML_DEEP,
            Some(r"Data\Libs\Subsumption\Missions\mission.xml"),
            None,
        ));
    }

    #[test]
    fn glob_exact_path_with_forward_slashes() {
        assert!(matches_filter(
            XML_DEEP,
            Some("Data/Libs/Subsumption/Missions/mission.xml"),
            None,
        ));
    }

    #[test]
    fn glob_prefix_doublestar_backslash() {
        assert!(matches_filter(XML_DEEP, Some(r"Data\Libs\**"), None));
        assert!(!matches_filter(DDS_DEEP, Some(r"Data\Libs\**"), None));
    }

    #[test]
    fn glob_prefix_doublestar_forward_slash() {
        assert!(matches_filter(XML_DEEP, Some("Data/Libs/**"), None));
        assert!(!matches_filter(DDS_DEEP, Some("Data/Libs/**"), None));
    }

    #[test]
    fn glob_partial_directory_wildcard() {
        // Match all files under any ships subdirectory.
        assert!(matches_filter(DDS_DEEP, Some("**/ships/**"), None));
        assert!(matches_filter(CGF_DEEP, Some("**/ships/**"), None));
        assert!(!matches_filter(XML_DEEP, Some("**/ships/**"), None));
    }

    // -----------------------------------------------------------------------
    // matches_filter — regex mode
    // -----------------------------------------------------------------------

    #[test]
    fn regex_xml_suffix() {
        let re = regex::Regex::new(r"\.xml$").unwrap();
        assert!(matches_filter(XML_DEEP, None, Some(&re)));
        assert!(matches_filter(XML_SHALLOW, None, Some(&re)));
        assert!(!matches_filter(DDS_DEEP, None, Some(&re)));
    }

    #[test]
    fn regex_dds_suffix_excludes_siblings() {
        let re = regex::Regex::new(r"\.dds$").unwrap();
        assert!(matches_filter(DDS_DEEP, None, Some(&re)));
        assert!(!matches_filter(DDS_SIBLING, None, Some(&re)));
    }

    // -----------------------------------------------------------------------
    // matches_filter — no filter
    // -----------------------------------------------------------------------

    #[test]
    fn no_filter_matches_everything() {
        assert!(matches_filter(XML_DEEP, None, None));
        assert!(matches_filter(DDS_DEEP, None, None));
        assert!(matches_filter(CGF_DEEP, None, None));
    }
}

