//! Metadata-only entity rig discovery and clip catalogue assembly; no meshes or textures decoded.
use super::*;
use crate::{
    animation::catalogue::{Catalogue, SkeletonInfo},
    error::Error,
};
use starbreaker_datacore::{database::Database, loadout::LoadoutTree, types::Record};
use starbreaker_p4k::MappedP4k;
use std::collections::{HashMap, HashSet};

/// Catalogue using the default decomposed export's root, landing-gear and loadout rigs.
/// Shared animation matching decodes only first/final values, never intermediate keys.
pub fn entity_clip_catalogue(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &LoadoutTree,
) -> Result<Catalogue, Error> {
    let entity = db
        .resolve_string2(record.name_offset)
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_owned();
    let mut catalogue = Catalogue {
        entity: entity.clone(),
        skeletons: Vec::new(),
        clips: Vec::new(),
    };
    let opts = ExportOptions {
        apply_default_animation_pose: false,
        ..Default::default()
    };
    let root = match resolve_loadout_meshes(db, p4k, record, tree, &opts) {
        Ok(root) => root,
        Err(Error::NoGeometryComponent { .. }) => return Ok(catalogue),
        Err(error) => return Err(error),
    };
    let root_path = root
        .geometry_path
        .as_deref()
        .map(|p| rig_path(p4k, p))
        .transpose()?
        .flatten();
    let namespace_children = root_path.is_none();
    let mut sources = Vec::new();
    if let Some(path) = root_path {
        sources.push((entity.clone(), path, true));
    }
    for (path, _) in query_landing_gear(db, record) {
        if let Some(rig) = rig_path(p4k, &path)? {
            let owner = path
                .replace('\\', "/")
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .split('.')
                .next()
                .unwrap_or(&path)
                .to_owned();
            sources.push((owner, rig, false));
        }
    }
    fn visit(
        p4k: &MappedP4k,
        nodes: &[crate::types::ResolvedNode],
        sources: &mut Vec<(String, String, bool)>,
    ) -> Result<(), Error> {
        for node in nodes {
            if node.has_geometry {
                if let Some(path) = &node.geometry_path {
                    if let Some(rig) = rig_path(p4k, path)? {
                        sources.push((node.entity_name.clone(), rig, false));
                    }
                }
            }
            visit(p4k, &node.children, sources)?;
        }
        Ok(())
    }
    visit(p4k, &root.children, &mut sources)?;
    let mut seen = HashSet::new();
    let mut clips: Vec<serde_json::Value> = Vec::new();
    let mut indices = HashMap::<String, usize>::new();
    let mut owners = Vec::new();
    for (owner, path, is_root) in sources {
        if !seen.insert((owner.clone(), path.to_ascii_lowercase(), is_root)) {
            continue;
        }
        let names = p4k
            .read_file(&datacore_path_to_p4k(&path))
            .ok()
            .and_then(|data| crate::skeleton::parse_rig_node_names(&data))
            .unwrap_or_default();
        let values =
            crate::animation::matching::extract_skeleton_json(p4k, &path, is_root, !is_root, true)?;
        let values = values.as_ref().and_then(|v| v.as_array());
        if names.is_empty() && values.is_none() {
            continue;
        }
        let first = values.and_then(|v| v.first());
        catalogue.skeletons.push(SkeletonInfo {
            owner_entity: owner.clone(),
            chr_path: path,
            chrparams_path: first
                .and_then(|v| v["chrparams_path"].as_str())
                .map(str::to_owned),
            dba_path: first
                .and_then(|v| v["dba_path"].as_str())
                .map(str::to_owned),
            bone_count: names.len(),
        });
        for clip in values.into_iter().flatten() {
            let mut clip = clip.clone();
            let original = clip["name"]
                .as_str()
                .ok_or_else(|| Error::Other("Clip without name".into()))?
                .to_owned();
            let name = if namespace_children && !is_root {
                let key = crate::decomposed::interior_clip_key(&owner, &original);
                clip["name"] = serde_json::json!(key);
                clip["display_name"] = serde_json::json!(
                    crate::decomposed::humanize_animation_label(&owner, &original)
                );
                key
            } else {
                original
            };
            if let Some(&index) = indices.get(&name) {
                let incoming = clip["bones"]
                    .as_object()
                    .ok_or_else(|| Error::Other("Clip without bones".into()))?;
                let existing = clips[index]["bones"].as_object_mut().unwrap();
                for (key, value) in incoming {
                    if let Some(old) = existing.get_mut(key) {
                        crate::decomposed::merge_animation_channel_values(
                            old,
                            value.clone(),
                            &name,
                            key,
                        );
                    } else {
                        existing.insert(key.clone(), value.clone());
                    }
                }
            } else {
                indices.insert(name, clips.len());
                owners.push(owner.clone());
                clips.push(clip);
            }
        }
    }
    if !clips.is_empty() {
        if let Some(source) = query_animation_controller_source(db, record) {
            if let Err(error) =
                crate::animation::annotate_animation_fragments_json(p4k, &mut clips, &source)
            {
                // Same optional annotation policy as decomposed export.
                log::warn!("[anim] failed to annotate Mannequin fragments: {error}");
            }
        }
    }
    catalogue.clips = clips
        .iter()
        .zip(owners)
        .map(|(clip, owner)| crate::animation::catalogue::summarize(clip, &owner))
        .collect::<Result<_, _>>()?;
    Ok(catalogue)
}

fn rig_path(p4k: &MappedP4k, geometry: &str) -> Result<Option<String>, Error> {
    if geometry.is_empty() {
        return Ok(None);
    }
    let resolved = resolve_geometry_files(p4k, geometry)?;
    Ok(
        skeleton_source_paths(resolved.skeleton_path.as_deref(), &resolved.parts[0].path)
            .first()
            .map(|p| (*p).to_owned()),
    )
}
