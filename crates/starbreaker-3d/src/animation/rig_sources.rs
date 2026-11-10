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
    pub controller: Option<super::AnimationControllerSource>,
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
            controller: None,
        });
    }
    sources.extend(
        children
            .into_iter()
            .map(|(owner_entity, skeleton_path)| RigSource {
                controller: None,
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
        endpoints,
    ) {
        Ok(Some(Value::Array(mut clips))) => {
            if let Some(controller) = &source.controller {
                if let Err(error) =
                    super::annotate_animation_fragments_json(p4k, &mut clips, controller)
                {
                    log::warn!("[anim] child controller {}: {error}", source.owner_entity);
                }
            }
            Some(clips)
        }
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
                // Resolve each controller-local animation ID in the same namespace as its clip.
                if let Some(fragments) = clip.get_mut("fragments").and_then(Value::as_array_mut) {
                    for fragment in fragments {
                        if let Some(animations) = fragment["animations"].as_array_mut() {
                            for animation in animations {
                                if let Some(name) = animation["name"].as_str() {
                                    animation["name"] = serde_json::json!(
                                        crate::decomposed::interior_clip_key(owner, name)
                                    );
                                }
                            }
                        }
                    }
                }
                clip["owner_entity"] = serde_json::json!(source.owner_entity);
                clip["name"] = serde_json::json!(key);
                clip["display_name"] = serde_json::json!(
                    crate::decomposed::humanize_animation_label(&source.owner_entity, &original)
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
            // Each rig plays its own animation of this name; the merged clip must run until
            // the longest of them has finished.
            if clip["frame_count"].as_u64() > existing.value["frame_count"].as_u64() {
                existing.value["frame_count"] = clip["frame_count"].clone();
            }
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
    fn child_fragment_animation_ids_follow_namespaced_clip() {
        let source =
            rig_sources(None, [("synthetic_door".into(), "synthetic.chr".into())]).remove(0);
        let mut collection = ClipCollection::default();
        collection.append(
            &source,
            serde_json::json!({"name":"open", "bones":{},
            "fragments":[{"fragment":"Door", "animations":[{"name":"open"}]}]}),
        );
        let clip = &collection.clips[0].value;
        assert_eq!(clip["fragments"][0]["animations"][0]["name"], clip["name"]);
    }
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
        assert!(
            collection
                .clips
                .iter()
                .all(|c| c.value.get("fragments").is_none())
        );
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
    #[test]
    fn merged_clip_runs_as_long_as_its_longest_contributor() {
        let sources = rig_sources(
            Some(("ship".into(), "root.chr".into())),
            [
                ("port_a".into(), "a.chr".into()),
                ("port_b".into(), "b.chr".into()),
                ("port_c".into(), "c.chr".into()),
            ],
        );
        let mut collection = ClipCollection::default();
        for (source, (bone, frames)) in sources[1..].iter().zip([("0x1", 40), ("0x2", 70), ("0x3", 55)]) {
            collection.append(source, serde_json::json!({"name":"open", "frame_count":frames, "bones":{bone:{}}}));
        }
        assert_eq!(collection.clips.len(), 1);
        assert_eq!(collection.clips[0].value["frame_count"], 70);
    }
}
