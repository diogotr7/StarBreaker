//! Reader for `.chrparams` (Cry character params) — a CryXmlB file that maps
//! animation event names (e.g. `landing_gear_extend`) to clip filenames inside
//! a tracks-database (`.dba`).
//!
//! ## Structure
//! ```xml
//! <Params>
//!   <AnimationList>
//!     <Animation name="#filepath" path="Animations/Spaceships/Ships/RSI/Scorpius"/>
//!     <Animation name="$TracksDatabase" path="Animations/Spaceships/Ships/RSI/Scorpius.dba"/>
//!     <Animation name="landing_gear_extend" path="rsi_Scorpius_lg_deploy_r.caf"/>
//!     <Animation name="landing_gear_compress" path="rsi_Scorpius_lg_compress_r.caf"/>
//!   </AnimationList>
//! </Params>
//! ```
//!
//! - `#filepath` is the directory prefix used to resolve the `path` attributes
//!   of subsequent entries when they are relative.
//! - `$TracksDatabase` points to the `.dba` file containing the keyframe data
//!   for every `.caf` listed below it.
//! - All other entries map an event name (the key the engine binds to game
//!   logic) to a `.caf` filename within the tracks database.

use std::collections::{BTreeMap, HashMap};

use starbreaker_cryxml::CryXml;

use crate::error::Error;

/// Parsed `.chrparams` document.
#[derive(Debug, Default, Clone)]
pub struct ChrParams {
    /// `$TracksDatabase` path (engine-relative, e.g. `Animations/.../Scorpius.dba`),
    /// or `None` if the chrparams uses standalone `.caf` files only.
    pub tracks_database: Option<String>,
    /// `$AnimEventDatabase` path (`.animevents`) when present.
    pub anim_event_database: Option<String>,
    /// Filepath prefix applied to relative `path` attributes (`#filepath`).
    pub filepath_prefix: Option<String>,
    /// Animation event name → `.caf` filename (or full path if absolute).
    pub animations: BTreeMap<String, String>,
}

impl ChrParams {
    /// Parse a CryXmlB `.chrparams` blob.
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        let xml = starbreaker_cryxml::from_bytes(data)
            .map_err(|e| Error::Other(format!("CryXml parse: {e:?}")))?;

        let mut out = ChrParams::default();
        Self::walk(&xml, xml.root(), &mut out);
        Ok(out)
    }

    fn walk(xml: &CryXml, node: &starbreaker_cryxml::CryXmlNode, out: &mut ChrParams) {
        let tag = xml.node_tag(node);
        if tag.eq_ignore_ascii_case("Animation") {
            let mut name: Option<&str> = None;
            let mut path: Option<&str> = None;
            for (k, v) in xml.node_attributes(node) {
                match k {
                    "name" => name = Some(v),
                    "path" => path = Some(v),
                    _ => {}
                }
            }
            if let (Some(name), Some(path)) = (name, path) {
                match name {
                    "#filepath" => out.filepath_prefix = Some(path.to_string()),
                    "$TracksDatabase" => out.tracks_database = Some(path.to_string()),
                    "$AnimEventDatabase" => out.anim_event_database = Some(path.to_string()),
                    _ => {
                        out.animations.insert(name.to_string(), path.to_string());
                    }
                }
            }
        }
        for child in xml.node_children(node) {
            Self::walk(xml, child, out);
        }
    }

    /// Resolve a relative `.caf` path against the `#filepath` prefix.
    /// If the path is already absolute (contains `/` or `\\` mid-string),
    /// returns it as-is. Otherwise prepends the prefix.
    pub fn resolved_caf_path(&self, raw_path: &str) -> String {
        if raw_path.contains('/') || raw_path.contains('\\') {
            return raw_path.to_string();
        }
        match self.filepath_prefix.as_deref() {
            Some(prefix) => {
                let sep = if prefix.ends_with('/') || prefix.ends_with('\\') {
                    ""
                } else {
                    "/"
                };
                format!("{prefix}{sep}{raw_path}")
            }
            None => raw_path.to_string(),
        }
    }
}

/// Parse a `.animevents` file and extract per-animation hint labels.
///
/// Returns a map keyed by lowercase animation path (e.g.
/// `animations/.../canopy_open.caf`) with associated labels from attributes
/// such as `bone="..."` and `parameter="..."`.
pub fn parse_animevents_targets(data: &[u8]) -> Result<HashMap<String, Vec<String>>, Error> {
    let xml = starbreaker_cryxml::from_bytes(data)
        .map_err(|e| Error::Other(format!("CryXml parse: {e:?}")))?;

    let mut out: HashMap<String, Vec<String>> = HashMap::new();

    fn walk(
        xml: &CryXml,
        node: &starbreaker_cryxml::CryXmlNode,
        current_animation: Option<&str>,
        out: &mut HashMap<String, Vec<String>>,
    ) {
        let tag = xml.node_tag(node);
        let mut local_animation = current_animation;

        if tag.eq_ignore_ascii_case("animation") {
            let mut anim_name: Option<&str> = None;
            for (k, v) in xml.node_attributes(node) {
                if (k.eq_ignore_ascii_case("animation") || k.eq_ignore_ascii_case("name"))
                    && v.to_ascii_lowercase().ends_with(".caf")
                {
                    anim_name = Some(v);
                    break;
                }
            }
            if let Some(a) = anim_name {
                local_animation = Some(a);
            }
        }

        if let Some(anim_path) = local_animation {
            let key = anim_path.to_ascii_lowercase();
            for (k, v) in xml.node_attributes(node) {
                if (k.eq_ignore_ascii_case("bone") || k.eq_ignore_ascii_case("parameter"))
                    && !v.trim().is_empty()
                {
                    let entry = out.entry(key.clone()).or_default();
                    if !entry.iter().any(|existing| existing.eq_ignore_ascii_case(v)) {
                        entry.push(v.to_string());
                    }
                }
            }
        }

        for child in xml.node_children(node) {
            walk(xml, child, local_animation, out);
        }
    }

    walk(&xml, xml.root(), None, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scorpius_right_chrparams_if_available() {
        let path = "/tmp/sb_skin/Data/Objects/Spaceships/Ships/RSI/LandingGear/Scorpius/RSI_Scorpius_Landinggear_Right_CHR.chrparams";
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("SKIP: fixture not available: {path}");
            return;
        };
        let cp = ChrParams::from_bytes(&bytes).expect("parse");
        assert_eq!(
            cp.tracks_database.as_deref(),
            Some("Animations/Spaceships/Ships/RSI/Scorpius.dba")
        );
        assert!(cp.animations.contains_key("landing_gear_extend"));
        let resolved = cp.resolved_caf_path(cp.animations.get("landing_gear_extend").unwrap());
        assert!(resolved.ends_with("rsi_Scorpius_lg_deploy_r.caf"));
    }
}
