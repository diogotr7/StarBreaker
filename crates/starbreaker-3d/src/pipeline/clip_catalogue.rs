//! Metadata-only entity clip catalogue using the exporter's shared rig discovery and collection.
use super::*;
use crate::{
    animation::{
        catalogue::{Catalogue, SkeletonInfo},
        rig_sources::{ClipCollection, RigSource, read_source_clips},
    },
    error::Error,
};
use starbreaker_datacore::{database::Database, loadout::LoadoutTree, types::Record};
use starbreaker_p4k::MappedP4k;
use std::collections::HashSet;

/// Catalogue using the default decomposed export's root, landing-gear and loadout rigs.
/// A broken geometry/animation source is warned about and skipped independently.
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
    let opts = ExportOptions {
        kind: ExportKind::Decomposed,
        format: ExportFormat::Glb,
        apply_default_animation_pose: false,
        ..Default::default()
    };
    let resolved = match resolve_loadout_meshes(db, p4k, record, tree, &opts) {
        Ok(root) => Some(root),
        Err(Error::NoGeometryComponent { .. }) => None,
        Err(error) => {
            log::warn!(
                "[anim] cannot resolve root geometry for '{entity}': {error}; continuing child rigs"
            );
            None
        }
    };
    let fallback_children = if resolved.is_none() {
        resolve_children(
            db,
            p4k,
            &tree.root.children,
            &opts,
            &load_invisible_ports(db, p4k, record),
        )
    } else {
        Vec::new()
    };
    let sources = super::animation_sources::discover_animation_sources(
        db,
        p4k,
        record,
        resolved
            .as_ref()
            .and_then(|root| root.geometry_path.as_deref()),
        resolved
            .as_ref()
            .map_or(fallback_children.as_slice(), |root| {
                root.children.as_slice()
            }),
        &opts,
    );
    catalogue_from_sources(
        p4k,
        Catalogue {
            entity,
            skeletons: Vec::new(),
            clips: Vec::new(),
        },
        sources,
        query_animation_controller_source(db, record),
    )
}

fn catalogue_from_sources(
    p4k: &MappedP4k,
    mut catalogue: Catalogue,
    sources: Vec<RigSource>,
    controller: Option<crate::animation::AnimationControllerSource>,
) -> Result<Catalogue, Error> {
    let mut seen = HashSet::new();
    let mut collection = ClipCollection::default();
    for source in sources {
        if !seen.insert((
            source.owner_entity.clone(),
            source.skeleton_path.to_ascii_lowercase(),
            source.is_root,
        )) {
            continue;
        }
        let names = p4k
            .read_file(&datacore_path_to_p4k(&source.skeleton_path))
            .ok()
            .and_then(|data| crate::skeleton::parse_rig_node_names(&data))
            .unwrap_or_default();
        let values = read_source_clips(p4k, &source, true);
        if names.is_empty() && values.is_none() {
            continue;
        }
        let first = values.as_ref().and_then(|v| v.first());
        catalogue.skeletons.push(SkeletonInfo {
            owner_entity: source.owner_entity.clone(),
            chr_path: source.skeleton_path.clone(),
            chrparams_path: first
                .and_then(|v| v["chrparams_path"].as_str())
                .map(str::to_owned),
            dba_path: first
                .and_then(|v| v["dba_path"].as_str())
                .map(str::to_owned),
            bone_count: names.len(),
        });
        for clip in values.into_iter().flatten() {
            collection.append(&source, clip);
        }
    }
    // Annotate the merged names exactly as decomposed export does.
    let mut values: Vec<_> = collection.clips.iter().map(|c| c.value.clone()).collect();
    if !values.is_empty() {
        if let Some(source) = controller {
            if let Err(error) =
                crate::animation::annotate_animation_fragments_json(p4k, &mut values, &source)
            {
                log::warn!("[anim] failed to annotate Mannequin fragments: {error}");
            }
        }
    }
    for (value, collected) in values.iter().zip(collection.clips) {
        match crate::animation::catalogue::summarize(value, &collected.owner_entity) {
            Ok(mut clip) => {
                for owner in collected.contributors {
                    if !clip.contributors.contains(&owner) {
                        clip.contributors.push(owner);
                    }
                }
                catalogue.clips.push(clip);
            }
            Err(error) => log::warn!(
                "[anim] skipping invalid catalogue clip from '{}': {error}",
                collected.owner_entity
            ),
        }
    }
    Ok(catalogue)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_dba_does_not_hide_other_rigs() {
        let archive = MappedP4k::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/clip_catalogue/rigs.zip"),
        )
        .unwrap();
        let catalogue = catalogue_from_sources(
            &archive,
            Catalogue {
                entity: "synthetic".into(),
                skeletons: vec![],
                clips: vec![],
            },
            vec![
                RigSource {
                    controller: None,
                    owner_entity: "broken".into(),
                    skeleton_path: "Objects/synthetic/broken.chr".into(),
                    is_root: true,
                    namespace: None,
                },
                RigSource {
                    controller: None,
                    owner_entity: "good".into(),
                    skeleton_path: "Objects/synthetic/good.chr".into(),
                    is_root: true,
                    namespace: None,
                },
            ],
            None,
        )
        .unwrap();
        assert_eq!(catalogue.clips.len(), 2);
        assert!(catalogue.clips.iter().any(|c| c.id == "latch_cycle"));
    }
}
