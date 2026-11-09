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
    rig_sources(root, child_sources)
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
