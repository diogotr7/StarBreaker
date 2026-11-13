//! GLB player package assembly with nested item packages and body clip sidecars.
use super::super::{Gender, body, clips::CharacterAnimation, loadout};
use super::{
    loadouts::{self, Item, PlayerRequest, read},
    motion,
    ports::{self, text},
    sources,
    xml::{Result, error},
};
use crate::{DecomposedExport, ExportFormat, ExportKind, ExportOptions, ExportedFile, ExportedFileKind};
use serde_json::{Value, json};
use starbreaker_datacore::{
    Database,
    loadout::{EntityIndex, LoadoutNode, resolve_loadout_indexed},
};
use starbreaker_p4k::MappedP4k;

/// HeadCam is the first-person camera joint literal in StarCitizen.exe. DataCore
/// confirms it as the Player audio-listener bone, but supplies no camera binding.
const PLAYER_CAMERA_BONE: &str = "HeadCam";

pub fn export_player(p4k: &MappedP4k, db: &Database, request: &PlayerRequest) -> Result<DecomposedExport> {
    let gender: Gender = request.gender.parse().map_err(error)?;
    let loadout = match &request.loadout {
        Some(path) => path.clone(),
        None => {
            loadouts::player_loadouts(p4k, db, gender)?
                .into_iter()
                .find(|l| !l.weapons.is_empty())
                .ok_or_else(|| error("no default loadout with a stowed WeaponPersonal item"))?
                .path
        }
    };
    let bytes = super::super::default_wearer_chf(p4k, gender)?;
    let chf = starbreaker_chf::ChfFile::from_chf(&bytes)?
        .parse()
        .map_err(|e| error(e.to_string()))?;
    let entries = loadouts::wearer_equipment(db, &chf, &loadouts::entries(p4k, &loadout)?)?;
    let tree = loadouts::tree(db, &chf, &entries, gender.tag())?;
    let skeleton = body::actor_skeleton_path(db, gender.tag()).ok_or_else(|| error("no player skeleton"))?;
    let bones = crate::skeleton::parse_skeleton(&read(p4k, &skeleton)?)
        .ok_or_else(|| error("player skeleton failed to parse"))?;
    if !bones.iter().any(|b| b.name == PLAYER_CAMERA_BONE) {
        return Err(error("player rig lacks executable camera joint"));
    }
    let opts = ExportOptions {
        kind: ExportKind::Decomposed,
        format: ExportFormat::Glb,
        lod_level: 0,
        texture_mip: 2,
        apply_default_animation_pose: false,
        include_animations: true,
        ..Default::default()
    };
    let name = format!("player_{}", request.gender.to_ascii_lowercase());
    let mut export = super::super::package::export_character_package(
        p4k,
        db,
        &bytes,
        &name,
        &opts,
        &CharacterAnimation::default(),
        &entries,
    )?;
    let scene_index = export
        .files
        .iter()
        .position(|f| f.relative_path.ends_with("/scene.json"))
        .ok_or_else(|| error("character scene missing"))?;
    let mut scene: Value =
        serde_json::from_slice(&export.files[scene_index].bytes).map_err(|e| error(e.to_string()))?;
    let idx = EntityIndex::new(db);
    let mut weapons = Vec::new();
    let mut parts = Vec::new();
    let mut hand_ports = Vec::new();
    collect(&tree, &idx, gender.tag(), &mut parts, &mut hand_ports)?;
    annotate_part(&mut scene["root_entity"], &parts);
    for child in scene["children"]
        .as_array_mut()
        .ok_or_else(|| error("character children missing"))?
    {
        annotate_part(child, &parts);
    }
    let mut serial = 0;
    attach_items(
        &tree,
        &idx,
        p4k,
        &opts,
        &name,
        gender.tag(),
        &mut serial,
        &mut scene,
        &mut export,
        &mut weapons,
    )?;
    let player = idx
        .find_record(body::PLAYER_ACTOR)
        .ok_or_else(|| error("Player record missing"))?;
    let controller = ports::query(db, player, "Components[SAnimatedCharacterComponentParams]")?;
    let (motion, spaces, clips, diagnostics) = motion::build(
        p4k,
        text(&controller["animDatabase3P"]),
        text(&controller["actionController"]),
        &weapons,
        gender.tag(),
        &skeleton,
        &bones,
    )?;
    let package = export.files[scene_index]
        .relative_path
        .rsplit_once('/')
        .unwrap()
        .0
        .to_owned();
    let mut animations = Vec::new();
    let mut filenames = std::collections::HashSet::new();
    for (name, clip) in clips {
        let file = crate::animation::sanitize_clip_filename(&name);
        if !filenames.insert(file.clone()) {
            return Err(error(format!("body clip filename collision: {name}")));
        }
        let sidecar = format!("animations/{file}.json");
        let (index, body) = crate::animation::split_clip_for_sidecar(&clip, &sidecar);
        animations.push(index);
        export.files.push(ExportedFile {
            relative_path: format!("{package}/{sidecar}"),
            bytes: serde_json::to_vec(&body).map_err(|e| error(e.to_string()))?,
            kind: ExportedFileKind::PackageManifest,
        });
    }
    scene["root_entity"]["animations"] = json!(animations);
    scene["player"] = json!({"gender":request.gender.to_ascii_lowercase(),"loadout":loadout,"skeleton":skeleton,"hand_ports":hand_ports,
        "camera_bone":PLAYER_CAMERA_BONE,"root_motion_bone":bones.iter().find(|b|b.parent_index.is_none()).ok_or_else(||error("root joint missing"))?.name,
        "weapons":weapons,"motion":motion,"blend_spaces":spaces,"diagnostics":diagnostics});
    export.files[scene_index].bytes = serde_json::to_vec_pretty(&scene).map_err(|e| error(e.to_string()))?;
    // Shared assets from nested packages must have identical bytes, never silently overwrite.
    let mut seen = std::collections::BTreeMap::new();
    for f in std::mem::take(&mut export.files) {
        if let Some(previous) = seen.get(&f.relative_path) {
            let previous: &ExportedFile = previous;
            if previous.bytes != f.bytes {
                return Err(error(format!("conflicting shared asset {}", f.relative_path)));
            }
        } else {
            seen.insert(f.relative_path.clone(), f);
        }
    }
    export.files = seen.into_values().collect();
    Ok(export)
}
fn collect(
    n: &Item,
    idx: &EntityIndex,
    gender: &str,
    parts: &mut Vec<(String, Value)>,
    hands: &mut Vec<Value>,
) -> Result<()> {
    if let Some(r) = idx.find_record(&n.entity) {
        hands.extend(
            ports::ports(idx.db, r, &n.entity, &[gender.into()])?
                .into_iter()
                .filter(|p| {
                    p["attachment"] == "bone" && p["flags"].as_array().is_some_and(|a| a.iter().any(|v| v == "select"))
                }),
        );
        if let Some(p) = &n.port {
            if ports::attach(idx.db, r)?["Type"] == "Char_Head" {
                parts.push((String::new(), p.clone()));
            }
        }
        if let (Some(p), Some(w)) = (&n.port, loadout::worn_geometry(idx.db, r, gender, &[gender.into()])) {
            parts.push((sources::stem(&w.geometry), p.clone()));
        }
    }
    for c in &n.children {
        collect(c, idx, gender, parts, hands)?;
    }
    Ok(())
}
fn annotate_part(node: &mut Value, parts: &[(String, Value)]) {
    let stem = sources::stem(text(&node["geometry_path"]));
    if let Some((_, port)) = parts
        .iter()
        .find(|(s, _)| s == &stem)
        .or_else(|| parts.iter().find(|(s, _)| s.is_empty()))
    {
        node["port"] = port.clone();
    }
}
fn override_children(node: &mut LoadoutNode, item: &Item, idx: &EntityIndex) -> Result<()> {
    for c in &item.children {
        if c.entity.is_empty() {
            continue;
        }
        let r = idx
            .find_record(&c.entity)
            .ok_or_else(|| error(format!("unknown loadout item {}", c.entity)))?;
        let mut replacement = resolve_loadout_indexed(idx, r).root;
        let port = c.port.as_ref();
        let name = port.map(|p| text(&p["name"])).unwrap_or("");
        if let Some(i) = node
            .children
            .iter()
            .position(|n| n.item_port_name.eq_ignore_ascii_case(name))
        {
            let old = node.children.remove(i);
            replacement.item_port_name = old.item_port_name;
            replacement.helper_bone_name = old.helper_bone_name;
            replacement.offset_position = old.offset_position;
            replacement.offset_rotation = old.offset_rotation;
            replacement.port_flags = old.port_flags;
        } else if let Some(p) = port {
            replacement.item_port_name = name.into();
            replacement.helper_bone_name = p["attach_bone"].as_str().map(str::to_owned);
        }
        override_children(&mut replacement, c, idx)?;
        node.children.push(replacement);
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn attach_items(
    item: &Item,
    idx: &EntityIndex,
    p4k: &MappedP4k,
    opts: &ExportOptions,
    root: &str,
    gender: &str,
    serial: &mut usize,
    scene: &mut Value,
    export: &mut DecomposedExport,
    weapons: &mut Vec<Value>,
) -> Result<()> {
    for child in &item.children {
        let bone = child.port.as_ref().is_some_and(|p| p["attachment"] == "bone");
        let r = idx.find_record(&child.entity);
        let skinned = r
            .and_then(|r| loadout::worn_geometry(idx.db, r, gender, &[gender.into()]))
            .is_some_and(|w| !w.other_gender);
        if bone && !skinned {
            let r = r.ok_or_else(|| error(format!("unknown rigid loadout item {}", child.entity)))?;
            let mut tree = resolve_loadout_indexed(idx, r);
            // A bone-attached character item's children are independently attached to
            // body ports; weapon children belong to the weapon's own rig.
            let attach = ports::attach(idx.db, r)?;
            let weapon = attach["Type"] == "WeaponPersonal";
            if weapon {
                override_children(&mut tree.root, child, idx)?;
            } else {
                tree.root.children.clear();
            }
            if tree.root.geometry_path.is_none() && idx.query_sub_geometry(r).is_empty() {
                *serial += 1;
                scene["children"].as_array_mut().unwrap().push(json!({"id":format!("player_item_{serial}"),"entity_name":child.entity,
                    "parent_entity_name":root,"port":child.port,"item_type":attach["Type"],"mesh_asset":null,"geometry_path":"","children":[]}));
                attach_items(child, idx, p4k, opts, root, gender, serial, scene, export, weapons)?;
                continue;
            }
            let result =
                crate::assemble_glb_with_loadout_with_progress(idx.db, p4k, r, &tree, opts, None, None, None, None)
                    .map_err(|e| error(e.to_string()))?;
            let mut nested = result.decomposed.ok_or_else(|| error("item returned no package"))?;
            let manifest = nested
                .files
                .iter()
                .find(|f| f.relative_path.ends_with("/scene.json"))
                .ok_or_else(|| error("item scene missing"))?;
            let mut sub: Value = serde_json::from_slice(&manifest.bytes).map_err(|e| error(e.to_string()))?;
            let package_dir = manifest.relative_path.rsplit_once('/').unwrap().0.to_owned();
            *serial += 1;
            let id = format!("player_item_{serial}");
            for file in &mut nested.files {
                relocate_provenance(file, &package_dir);
            }
            rebase_assets(&mut sub, &mut nested.files, &id)?;
            let mut node = sub["root_entity"].take();
            node["id"] = json!(id);
            node["entity_name"] = json!(child.entity);
            node["parent_entity_name"] = json!(root);
            node["port"] = child.port.clone().unwrap();
            node["item_type"] = attach["Type"].clone();
            node["parent_node_name"] = node["port"]["attach_bone"].clone();
            node["assembly_kind"] = sub["assembly_kind"].clone();
            node["weapon_assembly"] = sub["weapon_assembly"].clone();
            node["children"] = sub["children"].clone();
            node["package_dir"] = json!(package_dir.replacen("Packages/", &format!("Packages/PlayerItems/{id}/"), 1));
            if weapon {
                let mut grip = ports::query(idx.db, r, "Components[SEntityComponentCarryableParams].gripData")?;
                // The skeleton tag suffix is authored by the character TagDefinition;
                // helpers carry the same variant suffix. Retain all grips and offsets.
                for g in grip["grips"].as_array_mut().into_iter().flatten() {
                    for key in ["handGrip", "offHandGrip"] {
                        if let Some(helper) = g[key]["optionalHelper"].as_str() {
                            let variant = node["weapon_assembly"]["anchors"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|a| a["name"].as_str())
                                .find(|name| {
                                    name.strip_prefix(helper)
                                        .and_then(|s| s.strip_prefix('_'))
                                        .is_some_and(|tag| ports::gender_variant(tag, gender))
                                })
                                .map(str::to_owned);
                            if let Some(variant) = variant {
                                g[key]["optionalHelper"] = json!(variant);
                            }
                        }
                    }
                }
                weapons.push(json!({"entity":child.entity,"child_id":id,"stow_port":child.port,"mannequin_tags":ports::words(text(&attach["Tags"])),"grip_data":grip,"size":attach["Size"],"subtype":attach["SubType"]}));
            }
            let package_dir = text(&node["package_dir"]).to_owned();
            scene["children"].as_array_mut().unwrap().push(node);
            export.files.extend(
                nested
                    .files
                    .into_iter()
                    .filter(|f| !f.relative_path.ends_with("/scene.json"))
                    .map(|mut f| {
                        relocate_provenance(&mut f, &package_dir);
                        f
                    }),
            );
            if !weapon {
                attach_items(child, idx, p4k, opts, root, gender, serial, scene, export, weapons)?;
            }
        } else {
            attach_items(child, idx, p4k, opts, root, gender, serial, scene, export, weapons)?;
        }
    }
    Ok(())
}
/// Move a nested export's UI stamp (in its own package, or at the legacy
/// shared `Data/UI/Generated/` root) into the player package.
pub(super) fn relocate_provenance(file: &mut ExportedFile, package: &str) {
    let path = file.relative_path.as_str();
    let nested_package_stamp = path.starts_with("Packages/") && path.ends_with("/.export_stamp.json");
    if nested_package_stamp || path == "Data/UI/Generated/.export_stamp.json" {
        file.relative_path = format!("{package}/.export_stamp.json");
    }
}
/// Independent item material views can differ at the same source asset path.
/// Namespace packaged assets per instance, preserving original geometry paths.
pub(super) fn rebase_assets(scene: &mut Value, files: &mut [ExportedFile], instance: &str) -> Result<()> {
    let paths: std::collections::BTreeMap<String, String> = files
        .iter()
        .filter_map(|f| {
            ["Data", "Packages"].into_iter().find_map(|root| {
                f.relative_path
                    .strip_prefix(&format!("{root}/"))
                    .map(|p| (f.relative_path.clone(), format!("{root}/PlayerItems/{instance}/{p}")))
            })
        })
        .collect();
    fn rewrite(value: &mut Value, paths: &std::collections::BTreeMap<String, String>) {
        match value {
            Value::String(s) => {
                if let Some(path) = paths.get(s) {
                    *s = path.clone();
                }
            }
            Value::Array(values) => {
                for value in values {
                    rewrite(value, paths);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    rewrite(value, paths);
                }
            }
            _ => {}
        }
    }
    rewrite(scene, &paths);
    for file in files {
        if file.relative_path.ends_with(".json") && !file.relative_path.ends_with("/.export_stamp.json") {
            let mut value: Value = serde_json::from_slice(&file.bytes).map_err(|e| error(e.to_string()))?;
            rewrite(&mut value, &paths);
            file.bytes = serde_json::to_vec_pretty(&value).map_err(|e| error(e.to_string()))?;
        }
        if let Some(path) = paths.get(&file.relative_path) {
            file.relative_path = path.clone();
        }
    }
    Ok(())
}
