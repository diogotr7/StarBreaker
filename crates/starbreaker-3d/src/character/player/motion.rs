//! Ordered Mannequin fragment resolution and player motion queries.
use super::ports::subset;
use serde_json::Value;
pub(super) fn resolve<'a>(
    fragments: &'a [Value],
    name: &str,
    tags: &[String],
    frag_tags: &[String],
) -> Option<&'a Value> {
    let strings = |v: &Value| {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    fragments.iter().find(|f| {
        f["fragment"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(name))
            && subset(&strings(&f["tags"]), tags)
            && subset(&strings(&f["frag_tags"]), frag_tags)
    })
}

use super::{
    blend_space, clips,
    loadouts::read,
    ports::{text, words},
    sources::Sources,
    xml::{self, Node, Result, error},
};
use serde_json::json;
use starbreaker_p4k::MappedP4k;
use std::collections::{BTreeMap, BTreeSet};

fn tag_def(p4k: &MappedP4k, path: &str, seen: &mut BTreeSet<String>) -> Result<Node> {
    if !seen.insert(path.to_ascii_lowercase()) {
        return Ok(Node::default());
    }
    let archive_path = format!("Data/{path}").replace('/', "\\");
    if p4k.entry_case_insensitive(&archive_path).is_none() {
        return Ok(Node {
            children: vec![Node {
                tag: "Unavailable".into(),
                attrs: BTreeMap::from([("path".into(), path.into())]),
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    let mut root = xml::parse(&read(p4k, path)?)?;
    let imports = root.child("Imports").map(|n| n.children.clone()).unwrap_or_default();
    for i in imports {
        let other = tag_def(p4k, i.attr("filename"), seen)?;
        root.children.extend(other.children);
    }
    Ok(root)
}
fn groups(root: &Node) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for list in root.children.iter().filter(|n| n.tag == "Tags") {
        for g in list.children.iter().filter(|n| n.tag == "Group") {
            out.insert(
                g.attr("name").to_owned(),
                g.children.iter().map(|n| n.attr("name").to_owned()).collect(),
            );
        }
    }
    out
}
fn adb(
    p4k: &MappedP4k,
    path: &str,
    allowed: &BTreeSet<String>,
    tags: &[String],
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Value>,
    docs: &mut BTreeMap<String, std::sync::Arc<Node>>,
) -> Result<()> {
    if !seen.insert(path.to_ascii_lowercase()) {
        return Ok(());
    }
    let root = if let Some(root) = docs.get(path) {
        root.clone()
    } else {
        let root = std::sync::Arc::new(xml::parse(&read(p4k, path)?)?);
        docs.insert(path.into(), root.clone());
        root
    };
    for sub in root.child("SubADBs").into_iter().flat_map(|n| &n.children) {
        if !subset(&words(sub.attr("Tags")), tags) {
            continue;
        }
        let names = sub
            .children
            .iter()
            .filter(|n| n.tag == "FragmentID")
            .map(|n| n.attr("Name").to_owned())
            .collect::<BTreeSet<_>>();
        let eligible = if names.is_empty() {
            allowed.clone()
        } else {
            allowed.intersection(&names).cloned().collect()
        };
        if !eligible.is_empty() {
            adb(p4k, sub.attr("File"), &eligible, tags, seen, out, docs)?;
        }
    }
    for group in root
        .child("FragmentList")
        .into_iter()
        .flat_map(|n| &n.children)
        .filter(|n| allowed.contains(&n.tag))
    {
        for f in &group.children {
            if !subset(&words(f.attr("Tags")), tags) {
                continue;
            }
            let mut layers = Vec::new();
            for (index, layer) in f.children.iter().filter(|n| n.tag == "AnimLayer").enumerate() {
                let mut blend = Node::default();
                for node in &layer.children {
                    if node.tag == "Blend" {
                        blend = node.clone();
                    } else if node.tag == "Animation" && !node.attr("name").is_empty() {
                        layers.push(json!({"animation":node.attr("name").to_ascii_lowercase(),"layer":index,
                        "loop":words(node.attr("flags")).iter().any(|s|s=="Loop"),"speed":node.number("speed",1.0)?,
                        "blend_in":blend.number("Duration",0.0)?,"blend_out":f.number("BlendOutDuration",0.0)?,
                        "exit_time":blend.number("ExitTime",0.0)?,"start_time":blend.number("StartTime",0.0)?,
                        "weight":node.number("weight",1.0)?,"joint_mask":node.attrs.get("jointMask")}));
                    }
                }
            }
            out.push(json!({"fragment":group.tag,"guid":f.attr("GUID"),"tags":words(f.attr("Tags")),"frag_tags":words(f.attr("FragTags")),"layers":layers,"source":path}));
        }
    }
    Ok(())
}

/// Resolve every authored variant whose fragment-tag schema describes ground
/// locomotion or a stow-port selection. Names come from FragmentDefs/TagDefs.
pub(super) fn build(
    p4k: &MappedP4k,
    database: &str,
    controller_path: &str,
    weapons: &[Value],
    gender: &str,
    skeleton: &str,
    bones: &[crate::skeleton::Bone],
) -> Result<(Value, Vec<Value>, BTreeMap<String, Value>, Vec<Value>)> {
    let root = xml::parse(&read(p4k, database)?)?;
    let controller = xml::parse(&read(p4k, controller_path)?)?;
    let defs = tag_def(p4k, root.attr("FragDef"), &mut BTreeSet::new())?;
    let global = tag_def(p4k, root.attr("TagDef"), &mut BTreeSet::new())?;
    let global_groups = groups(&global);
    let mut schemas = BTreeMap::new();
    let mut unavailable = defs
        .children
        .iter()
        .chain(&global.children)
        .filter(|n| n.tag == "Unavailable")
        .map(|n| n.attr("path").to_owned())
        .collect::<Vec<_>>();
    for list in defs.children.iter().filter(|n| n.tag == "Tags") {
        for f in &list.children {
            if f.attr("subTagDef").is_empty() {
                continue;
            }
            let path = format!("Data/{}", f.attr("subTagDef")).replace('/', "\\");
            if p4k.entry_case_insensitive(&path).is_none() {
                unavailable.push(f.attr("subTagDef").to_owned());
                continue;
            }
            if !ground_schema(&xml::parse(&read(p4k, f.attr("subTagDef"))?)?) {
                continue;
            }
            let g = groups(&tag_def(p4k, f.attr("subTagDef"), &mut BTreeSet::new())?);
            if (g.contains_key("MotionHemisphere") && body_motion_scope(&controller, f.attr("name")))
                || g.contains_key("selectTag")
            {
                schemas.insert(f.attr("name").to_owned(), g);
            }
        }
    }
    let allowed = schemas.keys().cloned().collect();
    let mut states = Vec::new();
    let mut references = BTreeSet::new();
    let mut sources = Sources::new(p4k, skeleton)?;
    let mut spaces = BTreeMap::new();
    let mut docs = BTreeMap::new();
    for weapon in std::iter::once(None).chain(weapons.iter().map(Some)) {
        let weapon_tags = weapon.map(|w| strings(&w["mannequin_tags"])).unwrap_or_default();
        for stance in global_groups
            .get("stance")
            .into_iter()
            .flatten()
            .filter(|s| matches!(s.as_str(), "stand" | "crouch"))
        {
            let speeds = super::queries::speeds(&global_groups);
            for speed in speeds {
                for fp in [false, true] {
                    let mut tags = weapon_tags.clone();
                    tags.push(stance.clone());
                    if let Some(speed) = speed {
                        tags.push(speed.clone());
                    }
                    if fp {
                        tags.push("FP".into());
                    }
                    tags.extend(global_groups.get("ActorControl").into_iter().flatten().cloned());
                    tags.extend(
                        global_groups
                            .get("skeleton")
                            .into_iter()
                            .flatten()
                            .filter(|s| super::ports::gender_variant(s, gender))
                            .cloned(),
                    );
                    let mut fragments = Vec::new();
                    adb(
                        p4k,
                        database,
                        &allowed,
                        &tags,
                        &mut BTreeSet::new(),
                        &mut fragments,
                        &mut docs,
                    )?;
                    let mut bases = super::queries::bases(&fragments, &schemas, &tags, &weapon_tags);
                    if weapon.is_some() && bases.values().all(Option::is_none) {
                        // Some weapon/base associations are authored only for one stance.
                        // Carry that actor base into other stances without broadening the
                        // tags used for actual fragment selection.
                        let context = super::queries::base_context(&tags, &global_groups["stance"]);
                        let mut associations = Vec::new();
                        adb(
                            p4k,
                            database,
                            &allowed,
                            &context,
                            &mut BTreeSet::new(),
                            &mut associations,
                            &mut docs,
                        )?;
                        bases = super::queries::bases(&associations, &schemas, &context, &weapon_tags);
                    }
                    let mut resolved = Vec::new();
                    for (name, schema) in &schemas {
                        let base = bases[name].clone();
                        let mut permitted = BTreeSet::new();
                        for key in [
                            "MotionHemisphere",
                            "MotionHipDir",
                            "MotionTurnAngle",
                            "MotionTurnDir",
                            "MotionAction",
                        ] {
                            permitted.extend(schema.get(key).into_iter().flatten().cloned());
                        }
                        if let Some(base) = &base {
                            permitted.insert(base.clone());
                        }
                        if let Some(w) = weapon {
                            if let Some(t) = w["stow_port"]["select_tag"].as_str() {
                                permitted.insert(t.to_owned());
                            }
                        }
                        let mut queries = BTreeSet::new();
                        let mut basic = base.into_iter().collect::<Vec<_>>();
                        if schema.contains_key("selectTag") {
                            if let Some(w) = weapon {
                                if let Some(t) = w["stow_port"]["select_tag"].as_str() {
                                    basic.push(t.into());
                                }
                            } else {
                                continue;
                            }
                        }
                        queries.insert(basic.clone());
                        for f in &fragments {
                            if f["fragment"] != *name || !subset(&strings(&f["tags"]), &tags) {
                                continue;
                            }
                            let ts = strings(&f["frag_tags"]);
                            if ts.iter().all(|t| permitted.contains(t)) {
                                let mut q = basic.clone();
                                q.extend(ts);
                                q.sort();
                                q.dedup();
                                queries.insert(q);
                            }
                        }
                        for query in queries {
                            if let Some(f) = resolve(&fragments, name, &tags, &query) {
                                if f["layers"].as_array().is_none_or(Vec::is_empty) {
                                    continue;
                                }
                                let mut f = f.clone();
                                f["query_frag_tags"] = json!(query);
                                if let Some(def) = controller.child("FragmentDefs").and_then(|d| d.child(name)) {
                                    let matched = def
                                        .children
                                        .iter()
                                        .find(|d| {
                                            subset(&words(d.attr("tags")), &tags)
                                                && subset(&words(d.attr("fragTags")), &query)
                                        })
                                        .unwrap_or(def);
                                    f["scopes"] = json!(words(matched.attr("scopes")));
                                }
                                for layer in f["layers"].as_array_mut().unwrap() {
                                    let id = text(&layer["animation"]).to_owned();
                                    layer.as_object_mut().unwrap().remove("animation");
                                    expand(&id, &mut sources, &mut spaces, &mut references)?;
                                    if spaces.contains_key(&id) {
                                        layer["blend_space"] = json!(id);
                                    } else {
                                        layer["clip"] = json!(id);
                                    }
                                }
                                if !resolved.iter().any(|x: &Value| {
                                    x["guid"] == f["guid"] && x["query_frag_tags"] == f["query_frag_tags"]
                                }) {
                                    resolved.push(f);
                                }
                            }
                        }
                    }
                    states.push(json!({"weapon_child_id":weapon.map(|w|w["child_id"].clone()),"stance":stance,"move_speed_tag":speed,"first_person":fp,"tags":tags,"fragments":resolved}));
                }
            }
        }
    }
    let (clips, missing) = sources.export(&references, skeleton, bones).map_err(|e| {
        let message = e.to_string();
        let failed = references.iter().find(|name| message.contains(name.as_str()));
        let owners = failed
            .map(|name| {
                let mut owners = spaces
                    .iter()
                    .filter(|(_, b)| {
                        b["examples"]
                            .as_array()
                            .is_some_and(|es| es.iter().any(|e| e["clip"] == *name))
                    })
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                for state in &states {
                    for f in state["fragments"].as_array().unwrap() {
                        if f["layers"].as_array().unwrap().iter().any(|l| l["clip"] == *name) {
                            owners.push(format!("{} {}", text(&f["source"]), text(&f["fragment"])));
                        }
                    }
                }
                owners.sort();
                owners.dedup();
                owners.truncate(5);
                owners
            })
            .unwrap_or_default();
        error(format!("{message}; referenced by {owners:?}"))
    })?;
    let diagnostics =
        super::diagnostics::mark_missing(&mut states, &mut spaces, &missing, &gender.to_ascii_lowercase());
    for state in &mut states {
        for f in state["fragments"].as_array_mut().unwrap() {
            for l in f["layers"].as_array_mut().unwrap() {
                if l["missing"] == true {
                    l["additive"] = Value::Null;
                    continue;
                }
                l["additive"] = l["clip"]
                    .as_str()
                    .and_then(|s| clips.get(s))
                    .map(|v| v["additive"].clone())
                    .unwrap_or(json!(false));
            }
        }
    }
    for space in spaces.values_mut() {
        let dims = space["dimensions"].as_array().unwrap().clone();
        for e in space["examples"].as_array_mut().unwrap() {
            let Some(c) = clips.get(text(&e["clip"])) else {
                continue;
            };
            for (i, d) in dims.iter().enumerate() {
                if e["params"][i].is_null() {
                    e["params"][i] = json!(clips::parameter(
                        text(&d["name"]),
                        &c["root_motion"],
                        c["duration"].as_f64().unwrap(),
                        e["playback_scale"].as_f64().unwrap()
                    )?);
                }
            }
        }
        // Authored sample positions can lie beyond the authored sampling grid.
        // Keep both ranges; never clamp or change sample coordinates.
        let examples = space["examples"].as_array().unwrap().clone();
        for (i, d) in space["dimensions"].as_array_mut().unwrap().iter_mut().enumerate() {
            d["authored_min"] = d["min"].clone();
            d["authored_max"] = d["max"].clone();
            let values = examples
                .iter()
                .filter_map(|e| e["params"][i].as_f64())
                .collect::<Vec<_>>();
            d["min"] = json!(values.iter().copied().fold(d["min"].as_f64().unwrap(), f64::min));
            d["max"] = json!(values.iter().copied().fold(d["max"].as_f64().unwrap(), f64::max));
        }
    }
    Ok((
        json!({"states":states,"unavailable_tag_definitions":unavailable}),
        spaces.into_values().collect(),
        clips,
        diagnostics,
    ))
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}
fn expand(
    id: &str,
    sources: &mut Sources,
    spaces: &mut BTreeMap<String, Value>,
    clips: &mut BTreeSet<String>,
) -> Result<()> {
    if spaces.contains_key(id) || clips.contains(id) {
        return Ok(());
    }
    let path = sources.files.get(id).cloned();
    if let Some(path) = path.clone().filter(|p| p.to_ascii_lowercase().ends_with(".bspace")) {
        let doc = xml::parse(&read(sources.p4k, &path)?)?;
        let mut b = blend_space::parse(&doc).map_err(|e| error(format!("{path}: {e}")))?;
        b["id"] = json!(id);
        b["source"] = json!(path);
        for e in b["examples"].as_array().unwrap() {
            clips.insert(text(&e["clip"]).to_owned());
        }
        spaces.insert(id.into(), b);
    } else if let Some(path) = path.filter(|p| p.to_ascii_lowercase().ends_with(".comb")) {
        let doc = xml::parse(&read(sources.p4k, &path)?)?;
        if doc.tag != "CombinedBlendSpace" {
            return Err(error("expected CombinedBlendSpace"));
        }
        spaces.insert(id.into(), Value::Null);
        let mut components = Vec::new();
        for n in doc.child("BlendSpaces").into_iter().flat_map(|n| &n.children) {
            let child = super::sources::stem(n.attr("AName"));
            sources
                .files
                .entry(child.clone())
                .or_insert_with(|| n.attr("AName").into());
            expand(&child, sources, spaces, clips)?;
            let component = spaces
                .get(&child)
                .filter(|v| !v.is_null())
                .ok_or_else(|| error("combined-space cycle or missing child"))?;
            components.push(component.clone());
        }
        let mut b = blend_space::combine(&doc, &components)?;
        b["id"] = json!(id);
        b["source"] = json!(path);
        spaces.insert(id.into(), b);
    } else {
        clips.insert(id.into());
    }
    Ok(())
}
pub(super) fn ground_schema(root: &Node) -> bool {
    let g = groups(root);
    g.contains_key("MotionHemisphere") || g.contains_key("selectTag")
}
pub(super) fn query_base(stow: bool, base: Option<String>) -> Option<String> {
    if stow { None } else { base }
}
pub(super) fn body_motion_scope(controller: &Node, fragment: &str) -> bool {
    let Some(scopes) = controller.child("ScopeDefs") else {
        return false;
    };
    let Some(root) = scopes.children.iter().find(|s| s.attr("alwaysRootScope") == "1") else {
        return false;
    };
    let Some(def) = controller.child("FragmentDefs").and_then(|d| d.child(fragment)) else {
        return false;
    };
    let names = words(def.attr("scopes"));
    names.iter().any(|n| n == &root.tag)
        && names.iter().all(|n| {
            scopes
                .child(n)
                .is_some_and(|s| s.attr("context") == root.attr("context") || s.attr("numLayers") == "0")
        })
}
