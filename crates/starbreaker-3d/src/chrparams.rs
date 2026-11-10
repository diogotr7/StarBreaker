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
//! - `#filepath` is a running directive: it sets the directory that the
//!   `path` of every entry declared after it is relative to, until the next
//!   `#filepath`. A file may carry several (typically a second one scoping a
//!   trailing wildcard rule), so the prefix is applied while reading — see
//!   [`ChrParams::from_entries`] — and never kept as one per-file value.
//! - `$TracksDatabase` / `$AnimEventDatabase` carry full engine paths to the
//!   `.dba` holding the keyframe data and to the `.animevents` file.
//! - All other entries map an event name (the key the engine binds to game
//!   logic) to a `.caf` path, which names one clip of the tracks database.
//! - Directive names compare case-insensitively (`#filePath`, `#Filepath`
//!   occur in shipped files).
//!
//! The rules were checked against every `.chrparams` / `.dba` pair of a
//! shipped archive: the path built this way names a clip of the declared
//! tracks database wherever any reading of the entry does.

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
    /// Animation event name → full engine `.caf` path (the declared `path`
    /// under the `#filepath` in effect at its declaration).
    pub animations: BTreeMap<String, String>,
    /// Animation event name → all declared `.caf` paths (resolved like
    /// `animations`) in declaration order.
    ///
    /// Some chrparams files may define the same event name multiple times for
    /// variant clips or fallback entries. `animations` preserves the first
    /// declaration for backwards compatibility; this map keeps the full list.
    pub animation_paths: BTreeMap<String, Vec<String>>,
}

impl ChrParams {
    /// Parse a CryXmlB `.chrparams` blob.
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        let xml = starbreaker_cryxml::from_bytes(data)
            .map_err(|e| Error::Other(format!("CryXml parse: {e:?}")))?;

        let mut entries: Vec<(&str, &str)> = Vec::new();
        Self::collect_entries(&xml, xml.root(), &mut entries);
        Ok(Self::from_entries(entries))
    }

    /// Build the document from its `<Animation name path>` entries in
    /// declaration order. Order matters: each event path is resolved against
    /// the `#filepath` declared most recently before it.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut out = ChrParams::default();
        let mut filepath: Option<&str> = None;
        for (name, path) in entries {
            if name.eq_ignore_ascii_case("#filepath") {
                filepath = Some(path);
            } else if name.eq_ignore_ascii_case("$TracksDatabase") {
                out.tracks_database = Some(path.to_string());
            } else if name.eq_ignore_ascii_case("$AnimEventDatabase") {
                out.anim_event_database = Some(path.to_string());
            } else {
                out.record_animation_path(name, &resolve_under(filepath, path));
            }
        }
        out
    }

    fn collect_entries<'a>(
        xml: &'a CryXml,
        node: &'a starbreaker_cryxml::CryXmlNode,
        out: &mut Vec<(&'a str, &'a str)>,
    ) {
        if xml.node_tag(node).eq_ignore_ascii_case("Animation") {
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
                out.push((name, path));
            }
        }
        for child in xml.node_children(node) {
            Self::collect_entries(xml, child, out);
        }
    }

    fn record_animation_path(&mut self, name: &str, path: &str) {
        // Wildcard names declare mapping rules, not playable event identifiers.
        // DBA-only clips remain discoverable through the metadata catalogue.
        if name.contains(['*', '?']) {
            return;
        }
        let key = name.to_string();
        let value = path.to_string();
        self.animation_paths
            .entry(key.clone())
            .or_default()
            .push(value.clone());
        self.animations.entry(key).or_insert(value);
    }
}

/// `path` under the `#filepath` directory in effect, or as given when no
/// `#filepath` has been declared yet.
fn resolve_under(filepath: Option<&str>, path: &str) -> String {
    match filepath {
        Some(dir) => format!("{}/{path}", dir.trim_end_matches(['/', '\\'])),
        None => path.to_string(),
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
        let resolved = cp.animations.get("landing_gear_extend").unwrap();
        assert!(resolved.ends_with("rsi_Scorpius_lg_deploy_r.caf"));
    }

    #[test]
    fn preserves_duplicate_animation_event_paths() {
        let parsed = ChrParams::from_entries([
            ("door_open", "synthetic/door_open_a.caf"),
            ("door_open", "synthetic/door_open_b.caf"),
        ]);

        assert_eq!(
            parsed.animations.get("door_open"),
            Some(&"synthetic/door_open_a.caf".to_string())
        );
        assert_eq!(
            parsed.animation_paths.get("door_open"),
            Some(&vec![
                "synthetic/door_open_a.caf".to_string(),
                "synthetic/door_open_b.caf".to_string()
            ])
        );
    }

    /// `#filepath` is a running directive: it applies to the entries declared
    /// after it, until the next `#filepath`. A later directive (here the one
    /// that scopes a wildcard rule) must not re-home the earlier events.
    #[test]
    fn filepath_applies_only_to_the_entries_that_follow_it() {
        let parsed = ChrParams::from_entries([
            ("#filepath", "Animations/Synthetic/ShipA"),
            ("$TracksDatabase", "Animations/Synthetic/ShipA.dba"),
            ("hatch_open", "hatch_open.caf"),
            ("#filepath", "Animations/Synthetic/Shared"),
            ("*", "*/*.caf"),
            ("hatch_close", "hatch_close.caf"),
        ]);

        assert_eq!(
            parsed.animations.get("hatch_open").map(String::as_str),
            Some("Animations/Synthetic/ShipA/hatch_open.caf")
        );
        assert_eq!(
            parsed.animations.get("hatch_close").map(String::as_str),
            Some("Animations/Synthetic/Shared/hatch_close.caf")
        );
        assert_eq!(
            parsed.animation_paths.get("hatch_open"),
            Some(&vec!["Animations/Synthetic/ShipA/hatch_open.caf".to_string()])
        );
    }

    /// Directive names are matched without regard to case (`#filePath`,
    /// `#Filepath` both occur in shipped files) and never become events.
    #[test]
    fn directive_names_are_case_insensitive() {
        let parsed = ChrParams::from_entries([
            ("#filePath", "animations/synthetic/turret"),
            ("$tracksdatabase", "animations/synthetic/turret.dba"),
            ("$ANIMEVENTDATABASE", "animations/synthetic/turret.animevents"),
            ("recoil", "turret_recoil.caf"),
        ]);

        assert_eq!(
            parsed.animations.get("recoil").map(String::as_str),
            Some("animations/synthetic/turret/turret_recoil.caf")
        );
        assert_eq!(parsed.animations.len(), 1, "directives are not events");
        assert_eq!(
            parsed.tracks_database.as_deref(),
            Some("animations/synthetic/turret.dba")
        );
        assert_eq!(
            parsed.anim_event_database.as_deref(),
            Some("animations/synthetic/turret.animevents")
        );
    }

    /// A `path` with sub-directories is still relative to the `#filepath` in
    /// effect; only entries declared before any `#filepath` are used as given.
    #[test]
    fn filepath_prefixes_paths_with_subdirectories() {
        let parsed = ChrParams::from_entries([
            ("panel_open", "Animations/Synthetic/Loose/panel_open.caf"),
            ("#filepath", "Animations/Synthetic/ShipB/"),
            ("gear_extend", "exterior/gear_extend.caf"),
        ]);

        assert_eq!(
            parsed.animations.get("panel_open").map(String::as_str),
            Some("Animations/Synthetic/Loose/panel_open.caf")
        );
        assert_eq!(
            parsed.animations.get("gear_extend").map(String::as_str),
            Some("Animations/Synthetic/ShipB/exterior/gear_extend.caf")
        );
    }
}
