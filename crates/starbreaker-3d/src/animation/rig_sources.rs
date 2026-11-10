//! Ordered rig sources and shared tolerant clip collection for export and catalogue.
//!
//! `ClipCollection::append` merges same-named clips of different rigs into one clip. Each rig
//! plays its own animation of that name, so the merged clip lists every contributing
//! animation in `sources` (rig, frame rate, length, start pose) and each channel names the
//! one it came from by index (`source`).
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

/// The animation one rig contributes to a clip: its rig, own frame rate, own length and own
/// start pose. Two rigs sharing a skeleton and animation give equal records, and so one source.
fn source_record(source: &RigSource, clip: &Value) -> Value {
    let mut record = serde_json::json!({ "skeleton_path": source.skeleton_path });
    for key in ["fps", "frame_count", "start_rotation", "start_position"] {
        if let Some(value) = clip.get(key) {
            record[key] = value.clone();
        }
    }
    record
}

/// Name `index` (into the clip's `sources`) on every channel, and every variant of a channel.
fn stamp_channel_source(clip: &mut Value, index: usize) {
    let Some(bones) = clip.get_mut("bones").and_then(Value::as_object_mut) else {
        return;
    };
    for entry in bones.values_mut() {
        let channels = match entry {
            Value::Array(variants) => variants.iter_mut().collect(),
            channel => vec![channel],
        };
        for channel in channels {
            if let Some(channel) = channel.as_object_mut() {
                channel.insert("source".into(), serde_json::json!(index));
            }
        }
    }
}

/// `frames` of an animation running at `own_fps`, in frames of a clip running at `clip_fps`
/// (rounded up, so the clip never ends before the animation). `None` when a rate is unusable.
fn frames_at_clip_rate(frames: u64, own_fps: Option<u64>, clip_fps: Option<u64>) -> Option<u64> {
    match (own_fps, clip_fps) {
        (own, clip) if own == clip => Some(frames),
        (Some(own), Some(clip)) if own > 0 => Some((frames * clip).div_ceil(own)),
        _ => None,
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
        let record = source_record(source, &clip);
        // Empty names are left alone and never merged, exactly as in scene export.
        if let Some(&index) = self.indices.get(&name).filter(|_| !name.is_empty()) {
            let Some(existing) = self.clips.get_mut(index) else {
                return;
            };
            if !clip["bones"].is_object() {
                return;
            }
            // Each rig plays its own animation of this name; the merged clip must run until
            // the longest of them has finished, measured in the clip's own frame rate.
            if let Some(frames) = clip["frame_count"].as_u64() {
                let (own_fps, clip_fps) = (clip["fps"].as_u64(), existing.value["fps"].as_u64());
                match frames_at_clip_rate(frames, own_fps, clip_fps) {
                    Some(frames) if Some(frames) > existing.value["frame_count"].as_u64() => {
                        existing.value["frame_count"] = serde_json::json!(frames);
                    }
                    Some(_) => {}
                    None => log::warn!(
                        "[anim] clip '{name}' of {}: frame rate {own_fps:?} cannot be related to \
                         the merged clip's {clip_fps:?}; its length does not extend the clip",
                        source.skeleton_path
                    ),
                }
            }
            if contributes {
                let Some(sources) = existing.value["sources"].as_array_mut() else {
                    return;
                };
                let known = sources.iter().position(|known| *known == record);
                let source_index = known.unwrap_or_else(|| {
                    sources.push(record);
                    sources.len() - 1
                });
                stamp_channel_source(&mut clip, source_index);
            }
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
            clip["sources"] = Value::Array(contributes.then_some(record).into_iter().collect());
            stamp_channel_source(&mut clip, 0);
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

    fn timed(frames: u64, fps: u64, start: [f64; 4], bone: &str) -> Value {
        serde_json::json!({"name":"open", "fps":fps, "frame_count":frames, "start_rotation":start,
            "bones":{bone:{"rotation":[[1.0, 0.0, 0.0, 0.0]]}}})
    }
    #[test]
    fn merged_clip_records_each_contributing_animation() {
        let sources = rig_sources(
            Some(("ship".into(), "root.chr".into())),
            [("port_a".into(), "a.chr".into()), ("port_b".into(), "b.chr".into())],
        );
        let mut collection = ClipCollection::default();
        collection.append(&sources[1], timed(40, 30, [1.0, 0.0, 0.0, 0.0], "0x1"));
        collection.append(&sources[2], timed(70, 30, [0.5, 0.5, 0.5, 0.5], "0x2"));
        let clip = &collection.clips[0].value;
        assert_eq!(
            clip["sources"],
            serde_json::json!([
                {"skeleton_path":"a.chr", "fps":30, "frame_count":40, "start_rotation":[1.0, 0.0, 0.0, 0.0]},
                {"skeleton_path":"b.chr", "fps":30, "frame_count":70, "start_rotation":[0.5, 0.5, 0.5, 0.5]},
            ])
        );
        assert_eq!(clip["bones"]["0x1"]["source"], 0);
        assert_eq!(clip["bones"]["0x2"]["source"], 1);
        // Clip-level values stay what they were: the first contributor's, and the longest length.
        assert_eq!(clip["frame_count"], 70);
        assert_eq!(clip["start_rotation"], serde_json::json!([1.0, 0.0, 0.0, 0.0]));
    }
    #[test]
    fn one_animation_on_two_entities_is_one_source() {
        let sources = rig_sources(
            Some(("ship".into(), "root.chr".into())),
            [("door_left".into(), "door.chr".into()), ("door_right".into(), "door.chr".into())],
        );
        let mut collection = ClipCollection::default();
        for source in &sources[1..] {
            collection.append(source, timed(40, 30, [1.0, 0.0, 0.0, 0.0], "0x1"));
        }
        let clip = &collection.clips[0].value;
        assert_eq!(clip["sources"].as_array().unwrap().len(), 1);
        assert!(clip["bones"]["0x1"].is_object(), "identical channels stay one channel");
        assert_eq!(collection.clips[0].contributors, ["door_left", "door_right"]);
    }
    #[test]
    fn merged_length_is_the_longest_duration_at_the_clip_rate() {
        let sources = rig_sources(
            Some(("ship".into(), "root.chr".into())),
            [("port_a".into(), "a.chr".into()), ("port_b".into(), "b.chr".into())],
        );
        let mut collection = ClipCollection::default();
        // 2 s at 30 fps, then 2.5 s at 60 fps: 75 frames of the clip's 30 fps, not 150.
        collection.append(&sources[1], timed(60, 30, [1.0, 0.0, 0.0, 0.0], "0x1"));
        collection.append(&sources[2], timed(150, 60, [1.0, 0.0, 0.0, 0.0], "0x2"));
        let clip = &collection.clips[0].value;
        assert_eq!((&clip["fps"], &clip["frame_count"]), (&serde_json::json!(30), &serde_json::json!(75)));
        assert_eq!(clip["sources"][1]["fps"], 60);
        assert_eq!(clip["sources"][1]["frame_count"], 150);
    }
    #[test]
    fn every_channel_variant_of_a_single_rig_names_its_source() {
        let source = rig_sources(Some(("ship".into(), "root.chr".into())), []).remove(0);
        let mut collection = ClipCollection::default();
        collection.append(
            &source,
            serde_json::json!({"name":"open", "fps":30, "frame_count":10,
                "bones":{"0x1":[{"position":[[0.0, 0.0, 0.0]]}, {"position":[[1.0, 0.0, 0.0]]}]}}),
        );
        let clip = &collection.clips[0].value;
        assert_eq!(clip["sources"], serde_json::json!([{"skeleton_path":"root.chr", "fps":30, "frame_count":10}]));
        assert_eq!(clip["bones"]["0x1"][0]["source"], 0);
        assert_eq!(clip["bones"]["0x1"][1]["source"], 0);
    }
}
