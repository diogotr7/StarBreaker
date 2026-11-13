//! Mannequin ADB (Animation Database) fragment reading and JSON serialization.

use std::collections::HashMap;

use super::matching::{clip_name_lookup_keys, parse_f32_attr, split_tag_list};
use super::{AnimationControllerSource};
use crate::error::Error;

pub fn annotate_animation_fragments_json(
    p4k: &starbreaker_p4k::MappedP4k,
    clips: &mut [serde_json::Value],
    source: &AnimationControllerSource,
) -> Result<(), Error> {
    let scopes = read_controller_fragment_scopes(p4k, &source.animation_controller);
    let fragments_by_clip = read_mannequin_fragments_by_clip(p4k, &source.animation_database, &scopes)?;

    for clip in clips.iter_mut() {
        if clip.get("owner_entity").is_some() { continue; } // already annotated in the child controller namespace
        let Some(name) = clip.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        let keys = clip_name_lookup_keys(name);
        let mut fragments: Vec<serde_json::Value> = Vec::new();
        for key in keys {
            if let Some(values) = fragments_by_clip.get(&key) {
                for fragment in values {
                    if !fragments.iter().any(|existing| existing == fragment) {
                        fragments.push(fragment.clone());
                    }
                }
            }
        }
        if !fragments.is_empty() {
            clip["fragments"] = serde_json::Value::Array(fragments);
        }
    }

    Ok(())
}

fn read_mannequin_fragments_by_clip(
    p4k: &starbreaker_p4k::MappedP4k,
    animation_database: &str,
    scopes: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, Vec<serde_json::Value>>, Error> {
    let path = mannequin_adb_p4k_path(animation_database);
    let data = p4k
        .entry_case_insensitive(&path)
        .and_then(|entry| p4k.read(entry).ok())
        .ok_or_else(|| Error::Other(format!("Cannot load Mannequin ADB: {path}")))?
        .to_vec();
    let xml = starbreaker_cryxml::from_bytes(&data)
        .map_err(|error| Error::Other(format!("Mannequin ADB CryXml parse: {error:?}")))?;

    let mut fragments = Vec::new();
    collect_mannequin_fragments(&xml, xml.root(), None, false, scopes, &mut fragments);

    let mut by_clip: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for fragment in fragments {
        let animation_names = fragment
            .get("animations")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|animation| animation.get("name").and_then(|value| value.as_str()))
            .flat_map(clip_name_lookup_keys)
            .collect::<Vec<_>>();
        for key in animation_names {
            by_clip.entry(key).or_default().push(fragment.clone());
        }
    }

    Ok(by_clip)
}

fn read_controller_fragment_scopes(
    p4k: &starbreaker_p4k::MappedP4k,
    animation_controller: &str,
) -> HashMap<String, Vec<String>> {
    let path = mannequin_adb_p4k_path(animation_controller);
    let Some(data) = p4k
        .entry_case_insensitive(&path)
        .and_then(|entry| p4k.read(entry).ok())
        .map(|bytes| bytes.to_vec())
    else {
        return HashMap::new();
    };
    let Ok(xml) = starbreaker_cryxml::from_bytes(&data) else {
        return HashMap::new();
    };

    let mut scopes = HashMap::new();
    collect_controller_fragment_scopes(&xml, xml.root(), &mut scopes);
    scopes
}

fn collect_controller_fragment_scopes(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    scopes: &mut HashMap<String, Vec<String>>,
) {
    let tag = xml.node_tag(node);
    if tag != "ControllerDef" && tag != "Tags" && tag != "Fragments" && tag != "FragmentDefs" {
        let attrs = xml.node_attributes(node).collect::<HashMap<_, _>>();
        if let Some(raw_scopes) = attrs.get("scopes") {
            scopes.insert(tag.to_string(), split_tag_list(raw_scopes));
        }
    }
    for child in xml.node_children(node) {
        collect_controller_fragment_scopes(xml, child, scopes);
    }
}

fn collect_mannequin_fragments(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    current_fragment_group: Option<String>,
    in_fragment_list: bool,
    scopes: &HashMap<String, Vec<String>>,
    out: &mut Vec<serde_json::Value>,
) {
    let tag = xml.node_tag(node);
    let now_in_fragment_list = in_fragment_list || tag == "FragmentList";
    let group = if now_in_fragment_list && tag != "FragmentList" && tag != "Fragment" {
        Some(tag.to_string())
    } else {
        current_fragment_group
    };

    if tag == "Fragment" {
        if let Some(fragment) = mannequin_fragment_json(xml, node, group.as_deref(), scopes) {
            out.push(fragment);
        }
    }

    for child in xml.node_children(node) {
        collect_mannequin_fragments(xml, child, group.clone(), now_in_fragment_list, scopes, out);
    }
}

fn mannequin_fragment_json(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    group: Option<&str>,
    scopes: &HashMap<String, Vec<String>>,
) -> Option<serde_json::Value> {
    let group = group.unwrap_or("");
    let attrs = xml.node_attributes(node).collect::<HashMap<_, _>>();
    let animations = collect_fragment_animations(xml, node);
    if animations.is_empty() {
        return None;
    }
    let procedurals = collect_fragment_procedurals(xml, node);

    let mut fragment = serde_json::json!({
        "fragment": group,
        "guid": attrs.get("GUID").copied().unwrap_or_default(),
        "tags": split_tag_list(attrs.get("Tags").copied().unwrap_or_default()),
        "frag_tags": split_tag_list(attrs.get("FragTags").copied().unwrap_or_default()),
        "blend_out_duration": parse_f32_attr(attrs.get("BlendOutDuration").copied()),
        "option_weight": parse_f32_attr(attrs.get("OptionWeight").copied()),
        "animations": animations,
    });
    if let Some(scope_list) = scopes.get(group) {
        fragment["scopes"] = serde_json::json!(scope_list);
    }
    if !procedurals.is_empty() {
        fragment["procedurals"] = serde_json::Value::Array(procedurals);
    }
    Some(fragment)
}

fn collect_fragment_animations(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    for child in xml.node_children(node) {
        if xml.node_tag(child) == "AnimLayer" {
            let mut blend = serde_json::json!({});
            for layer_child in xml.node_children(child) {
                let child_tag = xml.node_tag(layer_child);
                let attrs = xml.node_attributes(layer_child).collect::<HashMap<_, _>>();
                if child_tag == "Blend" {
                    blend = serde_json::json!({
                        "exit_time": parse_f32_attr(attrs.get("ExitTime").copied()),
                        "start_time": parse_f32_attr(attrs.get("StartTime").copied()),
                        "duration": parse_f32_attr(attrs.get("Duration").copied()),
                    });
                } else if child_tag == "Animation" {
                    let mut animation = serde_json::json!({
                        "name": attrs.get("name").copied().unwrap_or_default(),
                        "blend": blend,
                    });
                    if let Some(flags) = attrs.get("flags") {
                        animation["flags"] = serde_json::json!(flags);
                    }
                    if let Some(speed) = parse_f32_attr(attrs.get("speed").copied()) {
                        animation["speed"] = serde_json::json!(speed);
                    }
                    values.push(animation);
                }
            }
        }
        values.extend(collect_fragment_animations(xml, child));
    }
    values
}

fn collect_fragment_procedurals(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    for child in xml.node_children(node) {
        if xml.node_tag(child) == "Procedural" {
            let attrs = xml.node_attributes(child).collect::<HashMap<_, _>>();
            let mut params = serde_json::json!({});
            for proc_child in xml.node_children(child) {
                if xml.node_tag(proc_child) != "ProceduralParams" {
                    continue;
                }
                for param in xml.node_children(proc_child) {
                    let param_attrs = xml.node_attributes(param).collect::<HashMap<_, _>>();
                    if let Some(value) = param_attrs.get("value") {
                        params[xml.node_tag(param)] = serde_json::json!(value);
                    }
                }
            }
            values.push(serde_json::json!({
                "type": attrs.get("type").copied().unwrap_or_default(),
                "params": params,
            }));
        }
        values.extend(collect_fragment_procedurals(xml, child));
    }
    values
}

fn mannequin_adb_p4k_path(path: &str) -> String {
    let normalized = path.trim_start_matches("Data/").trim_start_matches("Data\\");
    let with_prefix = if normalized.to_ascii_lowercase().starts_with("animations/")
        || normalized.to_ascii_lowercase().starts_with("animations\\")
    {
        normalized.to_string()
    } else {
        format!("Animations/Mannequin/ADB/{normalized}")
    };
    format!("Data/{}", with_prefix).replace('/', "\\")
}

/// Structured dump of a Mannequin ADB plus its companion ControllerDef
/// XML for diagnostic / debug tooling. Returns a JSON value with one
/// entry per Mannequin Fragment containing `fragment` (group name),
/// `guid`, `tags`, `frag_tags`, `blend_out_duration`, `option_weight`,
/// `animations`, `scopes` (resolved from the ControllerDef), and any
/// `procedurals`. Used by the StarBreaker MCP `mannequin_dump` tool.
///
/// Phase 37 conclusion: ADB fragment metadata is captured at
/// fragment scope only — there is no per-bone blend-mode flag.
/// CAF/DBA `Controller` chunks expose `rot_format_flags` and
/// `pos_format_flags` per bone (now visible via `dba_dump`), but
/// these encode keyframe compression format, not additive/override
/// blend mode. Both are surfaced via MCP so empirical inspection can
/// be done from agent sessions; the canonical fallback when neither
/// distinguishes a bone is the geometric convex-hull test (Phase 38).
pub fn dump_mannequin_adb_to_json(
    p4k: &starbreaker_p4k::MappedP4k,
    source: &AnimationControllerSource,
    filter: Option<&str>,
) -> Result<serde_json::Value, Error> {
    let scopes = read_controller_fragment_scopes(p4k, &source.animation_controller);
    let adb_path = mannequin_adb_p4k_path(&source.animation_database);
    let data = p4k
        .entry_case_insensitive(&adb_path)
        .and_then(|entry| p4k.read(entry).ok())
        .ok_or_else(|| Error::Other(format!("Cannot load Mannequin ADB: {adb_path}")))?
        .to_vec();
    let xml = starbreaker_cryxml::from_bytes(&data)
        .map_err(|error| Error::Other(format!("Mannequin ADB CryXml parse: {error:?}")))?;
    let mut fragments = Vec::new();
    collect_mannequin_fragments(&xml, xml.root(), None, false, &scopes, &mut fragments);

    let filter_lc = filter.map(|f| f.to_ascii_lowercase());
    let filtered: Vec<serde_json::Value> = fragments
        .into_iter()
        .filter(|f| {
            let Some(needle) = filter_lc.as_ref() else {
                return true;
            };
            // Match against fragment group name, GUID, or any animation name.
            if let Some(group) = f.get("fragment").and_then(|v| v.as_str()) {
                if group.to_ascii_lowercase().contains(needle) {
                    return true;
                }
            }
            if let Some(guid) = f.get("guid").and_then(|v| v.as_str()) {
                if guid.to_ascii_lowercase().contains(needle) {
                    return true;
                }
            }
            if let Some(anims) = f.get("animations").and_then(|v| v.as_array()) {
                for a in anims {
                    if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                        if n.to_ascii_lowercase().contains(needle) {
                            return true;
                        }
                    }
                }
            }
            false
        })
        .collect();

    Ok(serde_json::json!({
        "animation_database": source.animation_database,
        "animation_controller": source.animation_controller,
        "adb_path": adb_path,
        "fragment_count": filtered.len(),
        "fragments": filtered,
    }))
}

