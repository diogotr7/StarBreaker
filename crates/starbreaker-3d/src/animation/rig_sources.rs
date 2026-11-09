//! Ordered rig sources and shared tolerant clip collection for export and catalogue.
use serde_json::Value;
use starbreaker_p4k::MappedP4k;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct RigSource {
    pub owner_entity: String,
    pub skeleton_path: String,
    pub is_root: bool,
    pub namespace: Option<String>,
}

/// Preserve exporter order and rootless-interior namespace rules.
pub(crate) fn rig_sources(
    root: Option<(String, String)>,
    children: impl IntoIterator<Item = (String, String)>,
) -> Vec<RigSource> {
    let namespace_children = root.is_none();
    let mut sources = Vec::new();
    if let Some((owner_entity, skeleton_path)) = root {
        sources.push(RigSource {
            owner_entity,
            skeleton_path,
            is_root: true,
            namespace: None,
        });
    }
    sources.extend(
        children
            .into_iter()
            .map(|(owner_entity, skeleton_path)| RigSource {
                namespace: namespace_children.then(|| owner_entity.clone()),
                owner_entity,
                skeleton_path,
                is_root: false,
            }),
    );
    sources
}

/// Malformed/missing rig animation data must not hide other rigs' clips.
pub(crate) fn read_source_clips(
    p4k: &MappedP4k,
    source: &RigSource,
    endpoints: bool,
) -> Option<Vec<Value>> {
    match super::extraction::extract_skeleton_json(
        p4k,
        &source.skeleton_path,
        source.is_root,
        !source.is_root,
        endpoints,
    ) {
        Ok(Some(Value::Array(clips))) => Some(clips),
        Ok(_) => None,
        Err(error) => {
            log::warn!(
                "[anim] failed to extract animations for skeleton '{}': {error}",
                source.skeleton_path
            );
            None
        }
    }
}

pub(crate) struct CollectedClip {
    pub value: Value,
    pub owner_entity: String,
    pub contributors: Vec<String>,
}
#[derive(Default)]
pub(crate) struct ClipCollection {
    pub clips: Vec<CollectedClip>,
    indices: HashMap<String, usize>,
}
impl ClipCollection {
    pub fn append(&mut self, source: &RigSource, mut clip: Value) {
        let original = clip["name"].as_str().unwrap_or("").to_owned();
        let name = match &source.namespace {
            Some(owner) if !original.is_empty() => {
                let key = crate::decomposed::interior_clip_key(owner, &original);
                clip["name"] = serde_json::json!(key);
                clip["display_name"] = serde_json::json!(
                    crate::decomposed::humanize_animation_label(owner, &original)
                );
                key
            }
            _ => original,
        };
        let contributes = clip["bones"]
            .as_object()
            .is_some_and(|bones| !bones.is_empty());
        let mut contributors = Vec::new();
        if contributes {
            contributors.push(source.owner_entity.clone());
        }
        // Empty names are left alone and never merged, exactly as in scene export.
        if let Some(&index) = self.indices.get(&name).filter(|_| !name.is_empty()) {
            let Some(existing) = self.clips.get_mut(index) else {
                return;
            };
            let Some(incoming_bones) = clip["bones"].as_object() else {
                return;
            };
            let Some(existing_bones) = existing.value["bones"].as_object_mut() else {
                return;
            };
            for (key, value) in incoming_bones {
                if let Some(old) = existing_bones.get_mut(key) {
                    crate::decomposed::merge_animation_channel_values(
                        old,
                        value.clone(),
                        &name,
                        key,
                    );
                } else {
                    existing_bones.insert(key.clone(), value.clone());
                }
            }
            for contributor in contributors {
                if !existing.contributors.contains(&contributor) {
                    existing.contributors.push(contributor);
                }
            }
        } else {
            if !name.is_empty() {
                self.indices.insert(name, self.clips.len());
            }
            self.clips.push(CollectedClip {
                value: clip,
                owner_entity: source.owner_entity.clone(),
                contributors,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rootless_names_and_empty_names_preserve_export_rules() {
        let sources = rig_sources(
            None,
            [
                ("door_a".into(), "a.chr".into()),
                ("door_b".into(), "b.chr".into()),
            ],
        );
        let mut collection = ClipCollection::default();
        for source in &sources {
            collection.append(source, serde_json::json!({"name":"open", "bones":{}}));
            collection.append(source, serde_json::json!({"name":"", "bones":{}}));
        }
        assert_eq!(
            collection
                .clips
                .iter()
                .map(|c| c.value["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["door_a__open", "", "door_b__open", ""]
        );
    }
    #[test]
    fn merged_doors_keep_all_contributors_without_changing_ids() {
        let sources = rig_sources(
            Some(("ship".into(), "root.chr".into())),
            [
                ("door_a".into(), "a.chr".into()),
                ("door_b".into(), "b.chr".into()),
            ],
        );
        let mut collection = ClipCollection::default();
        for source in &sources[1..] {
            collection.append(source, serde_json::json!({"name":"door_open", "bones":{"0x1":{"source_node_name":source.owner_entity}}}));
        }
        assert_eq!(collection.clips.len(), 1);
        assert_eq!(collection.clips[0].value["name"], "door_open");
        assert_eq!(collection.clips[0].contributors, ["door_a", "door_b"]);
        assert_eq!(
            collection.clips[0].value["bones"]["0x1"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
