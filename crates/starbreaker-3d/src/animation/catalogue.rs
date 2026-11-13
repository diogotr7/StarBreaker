//! Serializable clip catalogue and sparse endpoint summaries, sharing export matching and naming.
use crate::error::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Catalogue {
    pub entity: String,
    pub skeletons: Vec<SkeletonInfo>,
    pub clips: Vec<ClipInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkeletonInfo {
    pub owner_entity: String,
    pub chr_path: String,
    pub chrparams_path: Option<String>,
    pub dba_path: Option<String>,
    pub bone_count: usize,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClipSource {
    Chrparams,
    DbaOnly,
    Caf,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipInfo {
    pub id: String,
    pub display_name: String,
    /// First owner in export discovery order; named clips may merge multiple rigs.
    pub owner_entity: String,
    /// Owners and resolved rig nodes contributing channels; stable first-seen order.
    #[serde(default)]
    pub contributors: Vec<String>,
    pub source: ClipSource,
    pub fps: u16,
    pub frame_count: u32,
    pub duration_s: f32,
    pub has_position: bool,
    pub has_rotation: bool,
    /// Number of distinct source-rig/bone channel variants after merging.
    pub bone_count: usize,
    pub fragments: Vec<Fragment>,
    pub boomerang: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fragment {
    pub fragment: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub frag_tags: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

pub(crate) fn summarize(clip: &Value, owner: &str) -> Result<ClipInfo, Error> {
    let invalid = || Error::Other("Invalid internal clip catalogue record".into());
    let id = clip["name"].as_str().ok_or_else(invalid)?.to_owned();
    let fps = clip["fps"]
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(invalid)?;
    let frame_count = clip["frame_count"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(invalid)?;
    let mut has_position = false;
    let mut has_rotation = false;
    let mut boomerang = true;
    let mut bone_count = 0;
    let mut contributors = Vec::new();
    for bone in clip["bones"].as_object().ok_or_else(invalid)?.values() {
        let variants: Vec<&Value> = match bone.as_array() {
            Some(a) => a.iter().collect(),
            None => vec![bone],
        };
        for channel in variants {
            bone_count += 1;
            if !contributors.iter().any(|name| name == owner) {
                contributors.push(owner.to_owned());
            }
            if let Some(name) = channel["source_node_name"].as_str() {
                if !contributors.iter().any(|existing| existing == name) {
                    contributors.push(name.to_owned());
                }
            }
            has_position |= channel["has_position"] == true;
            has_rotation |= channel["has_rotation"] == true;
            for key in ["position", "rotation"] {
                if let Some(samples) = channel[key].as_array().filter(|s| !s.is_empty()) {
                    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
                        continue;
                    };
                    // q and -q represent the same orientation.
                    let equal = first == last
                        || (key == "rotation"
                            && first.as_array().zip(last.as_array()).is_some_and(|(a, b)| {
                                a.iter().zip(b).all(|(x, y)| {
                                    x.as_f64().zip(y.as_f64()).is_some_and(|(x, y)| x == -y)
                                })
                            }));
                    boomerang &= equal;
                }
            }
        }
    }
    let fragments = clip
        .get("fragments")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| Error::Other(e.to_string()))?
        .unwrap_or_default();
    Ok(ClipInfo {
        display_name: clip["display_name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                crate::decomposed::humanize_animation_label("", &id)
                    .trim_start_matches(" — ")
                    .to_owned()
            }),
        id,
        owner_entity: owner.into(),
        contributors,
        source: serde_json::from_value(clip["catalogue_source"].clone())
            .map_err(|e| Error::Other(e.to_string()))?,
        fps,
        frame_count,
        duration_s: frame_count as f32 / fps as f32,
        has_position,
        has_rotation,
        bone_count,
        fragments,
        boomerang,
    })
}
