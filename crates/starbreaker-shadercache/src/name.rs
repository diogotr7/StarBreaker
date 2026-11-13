//! Parsing of shader-cache entry names.
//!
//! A permutation is named
//! `Shaders/<api>/<stage>Shaders/<Shader>@<Entry>(<masks>).shcb`, e.g.
//! `Shaders/Vulkan/PShaders/HardSurface@HardSurfacePS(GL1)(RT0)(MDV2).shcb`.
//! The masks select a shader variant; they are archived here verbatim, not
//! interpreted.

/// Pipeline stage, derived from the cache directory name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Stage {
    Pixel,
    Vertex,
    Compute,
    Domain,
    Hull,
    Geometry,
    Other,
}

impl Stage {
    fn from_dir(dir: &str) -> Self {
        match dir {
            "PShaders" => Stage::Pixel,
            "VShaders" => Stage::Vertex,
            "CShaders" => Stage::Compute,
            "DShaders" => Stage::Domain,
            "HShaders" => Stage::Hull,
            "GShaders" => Stage::Geometry,
            _ => Stage::Other,
        }
    }
}

/// The permutation selector fields carried in the entry name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize)]
pub struct Masks {
    pub gl: Option<u64>,
    pub rt: Option<u64>,
    pub mdv: Option<u64>,
}

/// A single cached shader permutation.
///
/// `path` is the exact cache entry name and is what identifies a permutation:
/// many permutations share `shader` + `entry` and differ only in their masks
/// (or in name parts not parsed here, such as stage-linkage groups).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Permutation {
    /// Entry name inside the cache archive, as stored.
    pub path: String,
    pub shader: String,
    pub entry: String,
    pub stage: Stage,
    pub api: String,
    pub masks: Masks,
}

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s, 16).ok()
}

/// Parse `Shaders/<api>/<stage>Shaders/<Shader>@<Entry>(...).shcb`.
///
/// Returns `None` for entries that are not `.shcb` files (the cache also holds
/// `.cfxb` / `.cfib` descriptor blobs, which are out of scope here).
pub fn parse(path: &str) -> Option<Permutation> {
    let normalized = path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').collect();
    let file = *segments.last()?;
    let stem = file.strip_suffix(".shcb")?;
    let stage_dir = segments.get(segments.len().wrapping_sub(2))?;
    let api = segments.get(segments.len().wrapping_sub(3))?.to_string();

    let (shader, rest) = stem.split_once('@')?;
    let entry_end = rest.find('(').unwrap_or(rest.len());
    let entry = &rest[..entry_end];

    let mut masks = Masks::default();
    for group in rest[entry_end..].split('(').skip(1) {
        let group = group.strip_suffix(')').unwrap_or(group);
        if let Some(v) = group.strip_prefix("GL").and_then(parse_hex) {
            masks.gl = Some(v);
        } else if let Some(v) = group.strip_prefix("RT").and_then(parse_hex) {
            masks.rt = Some(v);
        } else if let Some(v) = group.strip_prefix("MDV").and_then(parse_hex) {
            masks.mdv = Some(v);
        }
    }

    Some(Permutation {
        path: path.to_string(),
        shader: shader.to_string(),
        entry: entry.to_string(),
        stage: Stage::from_dir(stage_dir),
        api,
        masks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_permutation_name() {
        let p = parse(r"Shaders\Vulkan\PShaders\HardSurface@HardSurfacePS(GL1)(RT0)(MDV2).shcb")
            .expect("valid name");
        assert_eq!(p.shader, "HardSurface");
        assert_eq!(p.entry, "HardSurfacePS");
        assert_eq!(p.stage, Stage::Pixel);
        assert_eq!(p.api, "Vulkan");
        assert_eq!(
            p.masks,
            Masks {
                gl: Some(0x1),
                rt: Some(0x0),
                mdv: Some(0x2)
            }
        );
    }

    #[test]
    fn parses_wide_masks_and_linkage_groups() {
        let p = parse(
            "Shaders/Vulkan/VShaders/MeshDecal@MeshDecalZPassVS(GL20)(RT20040000000)(out(PS)(HS)).shcb",
        )
        .expect("valid name");
        assert_eq!(p.stage, Stage::Vertex);
        assert_eq!(p.masks.gl, Some(0x20));
        assert_eq!(p.masks.rt, Some(0x200_4000_0000));
        assert_eq!(p.masks.mdv, None);
    }

    #[test]
    fn ignores_non_shader_cache_entries() {
        assert!(parse("Shaders/CFXIB/Vulkan/common.cfib").is_none());
        assert!(parse("Shaders/CFXIB/Vulkan/Illum.cfxb").is_none());
        assert!(parse("not-a-shader.txt").is_none());
    }
}
