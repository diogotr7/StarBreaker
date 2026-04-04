//! Parser for `.chrparams` (Character Parameters) files.
//!
//! These are CryXML files that map animation names to `.caf`/`.dba` file paths.
//! Structure:
//! ```xml
//! <Params>
//!   <AnimationList>
//!     <Animation name="#filepath" path="Animations/Ships/AEGS/Gladius" />
//!     <Animation name="$TracksDatabase" path="Animations/Ships/AEGS/Gladius.dba" />
//!     <Animation name="canopy_open" path="canopy_open.caf" />
//!     ...
//!   </AnimationList>
//! </Params>
//! ```
//!
//! Special names:
//! - `#filepath` — sets the base directory for subsequent relative paths
//! - `$TracksDatabase` — path to the `.dba` animation database
//! - `$AnimEventDatabase` — path to `.animevents` file (optional)
//! - `*` — wildcard catch-all pattern (ignored by us)

/// Parsed chrparams: animation database path + named animation clips.
#[derive(Debug, Clone)]
pub struct ChrParams {
    /// Path to the `.dba` animation database (from `$TracksDatabase`).
    pub tracks_database: Option<String>,
    /// Path to the `.animevents` file (from `$AnimEventDatabase`).
    pub anim_event_database: Option<String>,
    /// Named animation clips: (name, resolved_path).
    /// Paths are resolved to absolute P4k paths using the `#filepath` base directory.
    pub animations: Vec<AnimationEntry>,
}

/// A single named animation clip from chrparams.
#[derive(Debug, Clone)]
pub struct AnimationEntry {
    pub name: String,
    pub path: String,
}

/// Parse a `.chrparams` file from raw bytes (CryXML binary or XML text).
pub fn parse_chrparams(data: &[u8]) -> Result<ChrParams, crate::error::Error> {
    let xml = starbreaker_cryxml::from_bytes(data)?;
    let root = xml.root();

    let mut tracks_database = None;
    let mut anim_event_database = None;
    let mut animations = Vec::new();
    let mut base_path = String::new();

    // Find <AnimationList> inside <Params>
    let anim_list = xml.node_children(root)
        .find(|c| xml.node_tag(c) == "AnimationList");

    let Some(anim_list) = anim_list else {
        return Ok(ChrParams { tracks_database, anim_event_database, animations });
    };

    for child in xml.node_children(anim_list) {
        if xml.node_tag(child) != "Animation" {
            continue;
        }

        let attrs: std::collections::HashMap<&str, &str> =
            xml.node_attributes(child).collect();
        let name = attrs.get("name").copied().unwrap_or("");
        let path = attrs.get("path").copied().unwrap_or("");

        if name.is_empty() || path.is_empty() {
            continue;
        }

        match name {
            "#filepath" => {
                // Set base directory for subsequent relative paths.
                // Normalize to forward slashes, strip trailing slash.
                base_path = path.replace('\\', "/");
                if base_path.ends_with('/') {
                    base_path.pop();
                }
            }
            "$TracksDatabase" => {
                tracks_database = Some(resolve_path(&base_path, path));
            }
            "$AnimEventDatabase" => {
                anim_event_database = Some(resolve_path(&base_path, path));
            }
            "*" => {
                // Wildcard catch-all — skip
            }
            _ => {
                animations.push(AnimationEntry {
                    name: name.to_string(),
                    path: resolve_path(&base_path, path),
                });
            }
        }
    }

    Ok(ChrParams { tracks_database, anim_event_database, animations })
}

/// Resolve a potentially relative path against the current base directory.
fn resolve_path(base: &str, path: &str) -> String {
    let path = path.replace('\\', "/");
    if path.contains('/') && !path.starts_with('.') {
        // Already an absolute-ish path (contains directory separator)
        path
    } else if base.is_empty() {
        path
    } else {
        format!("{base}/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_path_relative() {
        assert_eq!(
            resolve_path("Animations/Ships/AEGS/Gladius", "canopy_open.caf"),
            "Animations/Ships/AEGS/Gladius/canopy_open.caf"
        );
    }

    #[test]
    fn test_resolve_path_absolute() {
        assert_eq!(
            resolve_path("Animations/Ships/AEGS/Gladius", "Animations/Ships/AEGS/Gladius.dba"),
            "Animations/Ships/AEGS/Gladius.dba"
        );
    }

    #[test]
    fn test_resolve_path_empty_base() {
        assert_eq!(
            resolve_path("", "some_file.caf"),
            "some_file.caf"
        );
    }
}
