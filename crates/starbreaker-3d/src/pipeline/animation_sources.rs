//! Shared metadata-only rig discovery for entity export and catalogue, including geometry fallback.
use super::*;
use crate::animation::rig_sources::{RigSource, rig_sources};
use starbreaker_datacore::{Database, types::Record};
use starbreaker_p4k::MappedP4k;

pub(crate) fn discover_animation_sources(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    root_geometry: Option<&str>,
    children: &[crate::types::ResolvedNode],
    opts: &ExportOptions,
) -> Vec<RigSource> {
    let entity = db
        .resolve_string2(record.name_offset)
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_owned();
    let root = resolve_rig(p4k, &entity, root_geometry, || {
        entity_geometry_paths(db, record, opts).map(|(path, _)| path)
    })
    .map(|path| (entity, path));
    let mut child_sources = Vec::new();
    if opts.include_attachments {
        for (path, _) in query_landing_gear(db, record) {
            let owner = path.rsplit('/').next().unwrap_or(&path).to_owned();
            if let Some(rig) = resolve_rig(p4k, &owner, Some(&path), || Ok(path.clone())) {
                child_sources.push((owner, rig));
            }
        }
        fn visit(
            db: &Database,
            p4k: &MappedP4k,
            nodes: &[crate::types::ResolvedNode],
            opts: &ExportOptions,
            out: &mut Vec<(String, String)>,
        ) {
            for node in nodes {
                if node.has_geometry {
                    if let Some(path) = resolve_rig(
                        p4k,
                        &node.entity_name,
                        node.geometry_path.as_deref(),
                        || entity_geometry_paths(db, &node.record, opts).map(|(path, _)| path),
                    ) {
                        out.push((node.entity_name.clone(), path));
                    }
                }
                visit(db, p4k, &node.children, opts, out);
            }
        }
        visit(db, p4k, children, opts, &mut child_sources);
    }
    let mut sources = rig_sources(root, child_sources);
    fn controllers(
        db: &Database,
        nodes: &[crate::types::ResolvedNode],
        out: &mut std::collections::HashMap<String, crate::animation::AnimationControllerSource>,
    ) {
        for node in nodes {
            if let Some(controller) = query_animation_controller_source(db, &node.record) {
                if !controller.animation_database.is_empty() {
                    out.insert(node.entity_name.clone(), controller);
                }
            }
            controllers(db, &node.children, out);
        }
    }
    let mut by_owner = std::collections::HashMap::new();
    controllers(db, children, &mut by_owner);
    for source in &mut sources {
        if !source.is_root {
            if let Some(controller) = by_owner.get(&source.owner_entity) {
                source.controller = Some(controller.clone());
                // Repeated entity classes may use different geometry (e.g. mirrored ports).
                // Identity includes the source path; no asset-name classification is involved.
                source.namespace = Some(controller_namespace(
                    &source.owner_entity,
                    &source.skeleton_path,
                ));
            }
        }
    }
    if opts.include_interior {
        let mut lights = crate::socpak::light_component::LightResolver::new(db);
        let interiors = load_interiors(db, p4k, record, opts, &mut lights);
        let by_geometry = interior_controllers_by_geometry(db, record, &interiors);
        for entry in interiors.unique_cgfs {
            let controller = entry.animation_controller.or_else(|| {
                by_geometry
                    .get(&geometry_key(&entry.cgf_path))
                    .cloned()
                    .flatten()
            });
            if let Some(controller) = controller {
                if let Some(path) = resolve_rig(p4k, &entry.name, Some(&entry.cgf_path), || {
                    Ok(entry.cgf_path.clone())
                }) {
                    sources.push(RigSource {
                        owner_entity: entry.name.clone(),
                        namespace: Some(controller_namespace(&entry.name, &path)),
                        skeleton_path: path,
                        is_root: false,
                        controller: Some(controller),
                    });
                }
            }
        }
    }
    sources
}

/// The same primary-path then DataCore-record fallback as child payload loading.
fn resolve_rig(
    p4k: &MappedP4k,
    owner: &str,
    primary: Option<&str>,
    fallback: impl FnOnce() -> Result<String, Error>,
) -> Option<String> {
    let resolve = |path: &str| -> Result<String, Error> {
        let geometry = resolve_geometry_files(p4k, path)?;
        let primary = geometry
            .parts
            .first()
            .ok_or_else(|| Error::Other(format!("No geometry parts in {path}")))?;
        let rig = skeleton_source_paths(geometry.skeleton_path.as_deref(), &primary.path)
            .first()
            .map(|p| (*p).to_owned())
            .ok_or_else(|| Error::Other(format!("No rig source in {path}")))?;
        if p4k
            .entry_case_insensitive(&datacore_path_to_p4k(&rig))
            .is_none()
        {
            return Err(Error::Other(format!("Missing rig geometry: {rig}")));
        }
        Ok(rig)
    };
    if let Some(path) = primary {
        match resolve(path) {
            Ok(path) => return Some(path),
            Err(error) => log::warn!(
                "[anim] cannot resolve primary geometry for '{owner}': {error}; trying DataCore"
            ),
        }
    }
    match fallback().and_then(|path| resolve(&path)) {
        Ok(path) => Some(path),
        Err(Error::NoGeometryComponent { .. }) => None,
        Err(error) => {
            log::warn!("[anim] skipping unresolved rig '{owner}': {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_child_geometry_uses_fallback_or_skips() {
        let archive = MappedP4k::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/clip_catalogue/rigs.zip"),
        )
        .unwrap();
        assert_eq!(
            resolve_rig(&archive, "child", Some("missing.cdf"), || Ok(
                "Objects/synthetic/good.chr".into()
            )),
            Some("Objects/synthetic/good.chr".into())
        );
        assert!(
            resolve_rig(&archive, "child", Some("missing.cdf"), || Ok(
                "also_missing.cdf".into()
            ))
            .is_none()
        );
        assert_eq!(
            resolve_rig(&archive, "child", None, || Ok(
                "Objects/synthetic/good.chr".into()
            )),
            Some("Objects/synthetic/good.chr".into())
        );
    }
}

/// Compact, stable IDs leave sidecar filenames within ustar's name limit.
fn controller_namespace(owner: &str, path: &str) -> String {
    use sha2::{Digest, Sha256};
    let identity = format!(
        "{}|{}",
        owner.to_ascii_lowercase(),
        path.replace('\\', "/").to_ascii_lowercase()
    );
    let digest = Sha256::digest(identity.as_bytes());
    format!(
        "item_{}",
        digest[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

// IncludedObjects can carry raw geometry only. Resolve its controller by an exact
// DataCore geometry relationship; conflicting authored controllers remain unresolved.
fn geometry_key(path: &str) -> String {
    path.replace('\\', "/")
        .to_ascii_lowercase()
        .trim_start_matches("data/")
        .to_owned()
}
fn interior_controllers_by_geometry(
    db: &Database,
    root: &Record,
    interiors: &LoadedInteriors,
) -> std::collections::HashMap<String, Option<crate::animation::AnimationControllerSource>> {
    let wanted: std::collections::HashSet<_> = interiors
        .unique_cgfs
        .iter()
        .filter(|e| {
            e.animation_controller.is_none() && e.cgf_path.to_ascii_lowercase().ends_with(".cga")
        })
        .map(|e| geometry_key(&e.cgf_path))
        .collect();
    let mut found: std::collections::HashMap<
        String,
        Option<crate::animation::AnimationControllerSource>,
    > = std::collections::HashMap::new();
    if wanted.is_empty() {
        return found;
    }
    let Ok(path) = db.compile_path::<String>(
        root.struct_id(),
        "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    ) else {
        return found;
    };
    for record in db
        .records()
        .iter()
        .filter(|r| r.struct_id() == root.struct_id())
    {
        let Ok(Some(geometry)) = db.query_single::<String>(&path, record) else {
            continue;
        };
        let key = geometry_key(&geometry);
        if !wanted.contains(&key) {
            continue;
        }
        let Some(controller) = query_animation_controller_source(db, record)
            .filter(|c| !c.animation_database.is_empty())
        else {
            continue;
        };
        found
            .entry(key)
            .and_modify(|existing| {
                if existing.as_ref().is_some_and(|c| {
                    !c.animation_database
                        .eq_ignore_ascii_case(&controller.animation_database)
                        || !c
                            .animation_controller
                            .eq_ignore_ascii_case(&controller.animation_controller)
                }) {
                    *existing = None;
                }
            })
            .or_insert(Some(controller));
    }
    found
}
