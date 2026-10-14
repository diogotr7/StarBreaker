//! CHF head items (hair, brows, lashes, beard, scalp) → per-gender meshes.
//!
//! A CHF's item-port tree names DataCore `EntityClassDefinition` records by
//! GUID. Head items carry per-gender sub-geometry (`Tags` = `Male`/`Female`)
//! whose `DeformerType` is a `WD_*` wrap deformer — the engine fits them to the
//! per-character head surface. [`resolve_head_items`] selects exactly those
//! (structural: by deformer, not by port or item name), [`load_head_items`]
//! reads their meshes (the head parser's shape and skeleton, and the entity
//! pipeline's mesh and `.mtl` — [`super::part_source::PartSource`]), and the
//! identity render wraps them with [`super::wrap::HeadDeltaField`].
//!
//! Item materials follow the engine's tag selection: the tag context is the
//! gender plus every tag the CHF's items publish (`$tag+`, e.g. a hairstyle's
//! `$scalpVHair_36+`), and an item's `materialVariants` tree resolved against
//! it overrides the sub-geometry's default material (the universal scalp
//! takes the hairstyle's `f_hair_NN.mtl` this way).

use starbreaker_chf::ItemPort;
use starbreaker_common::CigGuid;
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_datacore::material_variants::{item_tags, material_node, published_tags};
use starbreaker_p4k::MappedP4k;

use super::head_skin::{self, BindPoseSkeleton, HeadMesh};
use super::part_source::PartSource;
use super::CharacterError;
use std::sync::Arc;

/// One wrap-deformed head item resolved for a gender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadItem {
    /// The item record (`EntityClassDefinition`) GUID from the CHF.
    pub record: CigGuid,
    /// Attachment port the CHF puts the item on (`hair_itemport`, …).
    pub port: String,
    /// `.skin` path from the gender's sub-geometry.
    pub geometry: String,
    /// `.mtl` path (may be empty).
    pub material: String,
    /// `DeformerType` (`WD_Elastic`, `WD_ElasticNUScaling`, …).
    pub deformer: String,
}

/// A [`HeadItem`] with its mesh loaded.
#[derive(Clone)]
pub struct HeadItemMesh {
    pub item: HeadItem,
    pub mesh: HeadMesh,
    pub skeleton: BindPoseSkeleton,
    /// The item as the entity pipeline loads it, with `item.material`; the
    /// character exports reshape it to the wrapped `mesh`.
    pub(crate) source: Arc<PartSource>,
}

/// Whether a `DeformerType` is one of the head-surface wrap deformers.
pub fn is_wrap_deformer(deformer: &str) -> bool {
    deformer.starts_with("WD_")
}

/// Walk the CHF item-port tree and resolve every item whose `gender`-tagged
/// (`"Male"` / `"Female"`) sub-geometry is wrap-deformed. Ports without a
/// record, records without such a variant, and duplicates are skipped.
pub fn resolve_head_items(db: &Database, root: &ItemPort, gender: &str) -> Vec<HeadItem> {
    let index = EntityIndex::new(db);
    let mut ports = Vec::new();
    let mut stack = vec![root];
    while let Some(port) = stack.pop() {
        stack.extend(port.children.iter().rev());
        if !port.id.is_empty()
            && let Some(record) = db.record_by_id(&port.id)
        {
            ports.push((port, record));
        }
    }
    let mut context = vec![gender.to_owned()];
    for (_, record) in &ports {
        context.extend(published_tags(&item_tags(db, record)));
    }

    let mut out: Vec<HeadItem> = Vec::new();
    for (port, record) in ports {
        let variant = index
            .query_sub_geometry(record)
            .into_iter()
            .find(|v| v.tag.eq_ignore_ascii_case(gender) && is_wrap_deformer(&v.deformer_type));
        if let Some(v) = variant
            && !out.iter().any(|i| i.geometry.eq_ignore_ascii_case(&v.geometry_path))
        {
            let material = material_node(db, record)
                .and_then(|node| node.resolve(&context).map(str::to_owned))
                .unwrap_or(v.material_path);
            out.push(HeadItem {
                record: port.id,
                port: port.name.to_string(),
                geometry: v.geometry_path,
                material,
                deformer: v.deformer_type,
            });
        }
    }
    out
}

/// Load each item's `.skin` + `.skinm` (head parser and entity pipeline).
/// Items whose files are missing or fail to load are reported on stderr and
/// skipped (as head sub-meshes are).
pub fn load_head_items(p4k: &MappedP4k, items: &[HeadItem]) -> Vec<HeadItemMesh> {
    items
        .iter()
        .filter_map(|item| match load_skin_part(p4k, &item.geometry, &item.material) {
            Ok((mesh, skeleton, source)) => Some(HeadItemMesh { item: item.clone(), mesh, skeleton, source: Arc::new(source) }),
            Err(e) => {
                eprintln!("head item {}: skipped ({e})", item.geometry);
                None
            }
        })
        .collect()
}

/// A skinned part's `.skin` + `.skinm` (DataCore path `geometry`): its shape
/// and bone table (head parser) and its pipeline mesh with `material`.
pub(crate) fn load_skin_part(
    p4k: &MappedP4k,
    geometry: &str,
    material: &str,
) -> Result<(HeadMesh, BindPoseSkeleton, PartSource), CharacterError> {
    let skin_path = format!(r"Data\{}", geometry.replace('/', r"\"));
    let skin = super::p4k_read(p4k, &skin_path)?;
    let skinm = super::p4k_read(p4k, &format!("{skin_path}m"))?;
    let (mesh, skeleton) = head_skin::parse_skin_mesh(&skin, &skinm)?;
    let source = PartSource::load(p4k, &skin_path, material)?;
    Ok((mesh, skeleton, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_deformers_are_the_wd_family() {
        for d in ["WD_Elastic", "WD_ElasticNUScaling", "WD_ElasticDQSkinning", "WD_BShapeExclusion"] {
            assert!(is_wrap_deformer(d), "{d}");
        }
        for d in ["Standard", "OriginalSkin", "Protos", "ProtosMisc", ""] {
            assert!(!is_wrap_deformer(d), "{d}");
        }
    }
}
