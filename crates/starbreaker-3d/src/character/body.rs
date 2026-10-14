//! The CHF character's body: the item on the CHF's root port and the actor
//! skeleton the character is built on.
//!
//! - [`resolve_body`]: the root port's item (`body_itemport →
//!   body_01_noMagicPocket`). Its `SGeometryResourceParams` sub-geometry
//!   branches by gender tag, then by skin-tone tags that only pick a
//!   material; the body is the gender-only node's `.skin`. The material is
//!   the `.mtl` of the CHF slot customizing that port (`BodyMaterial`, see
//!   [`port_material_slot`]), else the node's own.
//! - [`actor_skeleton_path`]: the player actor entity ([`PLAYER_ACTOR`])
//!   branches its geometry by the same gender tag (the CHF's `modelTag`) to
//!   the skeleton `.chr` the engine spawns a player on (`bhm_skeleton_v7.chr`
//!   / `bhf_skeleton_v2.chr`, 220 bones). The body `.skin`'s own bone table
//!   is a subset of it (84 bones), and so is the head rig's body part (15
//!   bones, identical binds at every DNA rest); the facial bones hang under
//!   them.
//! - [`load_body`] / [`load_chf_body`]: the body mesh (the head parser's
//!   shape and skeleton, the entity pipeline's mesh and `.mtl`) and the
//!   actor skeleton; the character rig is built on it (via
//!   [`super::outfit::Outfit`]).

use std::sync::Arc;

use starbreaker_chf::{ChfData, ItemPort};
use starbreaker_common::CigGuid;
use starbreaker_datacore::Database;
use starbreaker_datacore::types::Record;
use starbreaker_datacore::loadout::{EntityIndex, SubGeometryVariant};
use starbreaker_p4k::MappedP4k;

use super::customization::{CustomizedMaterial, MaterialLookup, port_material_slot, resolve_chf_materials};
use super::head_skin::{BindPoseSkeleton, HeadMesh};
use super::part_source::PartSource;
use super::{CharacterError, gender_tag, items, p4k_read};
use crate::skeleton::Bone;

/// The actor entity a player character spawns as; its geometry is the
/// per-gender skeleton.
pub const PLAYER_ACTOR: &str = "Player";

/// The body item resolved for a gender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyItem {
    /// The item record (`EntityClassDefinition`) GUID from the CHF.
    pub record: CigGuid,
    /// The CHF root port (`body_itemport`).
    pub port: String,
    /// `.skin` path of the gender's sub-geometry.
    pub geometry: String,
    /// `.mtl` path: the CHF's customized body material, else the
    /// sub-geometry's.
    pub material: String,
    /// The actor skeleton `.chr` ([`actor_skeleton_path`]).
    pub skeleton: String,
}

/// A [`BodyItem`] with its mesh and skeletons loaded.
#[derive(Clone)]
pub struct Body {
    pub item: BodyItem,
    /// The body's shape at its bind.
    pub mesh: HeadMesh,
    /// The body `.skin`'s own bone table (the joints `mesh` refers to).
    pub skeleton: BindPoseSkeleton,
    /// The actor skeleton at its bind (local and world = bind).
    pub actor: Vec<Bone>,
    /// The body as the entity pipeline loads it, with `item.material`.
    pub(crate) source: Arc<PartSource>,
}

/// The gender-only sub-geometry node of `record` (tag = `gender`, any case);
/// deeper nodes add selector tags (skin tones, voices) to the same branch.
fn gender_variant(index: &EntityIndex, record: &Record, gender: &str) -> Option<SubGeometryVariant> {
    index.query_sub_geometry(record).into_iter().find(|v| v.tag.eq_ignore_ascii_case(gender))
}

/// The player actor's skeleton `.chr` for `gender` (see the module docs).
pub fn actor_skeleton_path(db: &Database, gender: &str) -> Option<String> {
    let index = EntityIndex::new(db);
    gender_variant(&index, index.find_record(PLAYER_ACTOR)?, gender).map(|v| v.geometry_path)
}

/// The body on the CHF's root port `root` for `gender`, with the material of
/// the CHF slot customizing that port (from `customization`) when it selects
/// one. `None` when the root port has no record, the record no gender
/// geometry, or the actor no skeleton for the gender.
pub fn resolve_body(db: &Database, root: &ItemPort, gender: &str, customization: &[CustomizedMaterial]) -> Option<BodyItem> {
    let index = EntityIndex::new(db);
    let record = db.record_by_id(&root.id)?;
    let variant = gender_variant(&index, record, gender)?;
    let port = root.name.to_string();
    let slot = port_material_slot(&port);
    let material = customization
        .iter()
        .find(|m| m.slot == slot)
        .and_then(|m| m.mtl_path.clone())
        .unwrap_or(variant.material_path);
    Some(BodyItem {
        record: root.id,
        port,
        geometry: variant.geometry_path,
        material,
        skeleton: actor_skeleton_path(db, gender)?,
    })
}

/// Load the body's mesh (head parser + entity pipeline) and its actor
/// skeleton.
pub fn load_body(p4k: &MappedP4k, item: &BodyItem) -> Result<Body, CharacterError> {
    let (mesh, skeleton, source) = items::load_skin_part(p4k, &item.geometry, &item.material)?;
    let chr = p4k_read(p4k, &format!(r"Data\{}", item.skeleton.replace('/', r"\")))?;
    let actor = crate::skeleton::parse_skeleton(&chr).filter(|b| !b.is_empty()).ok_or_else(|| CharacterError::HeadSkinParse {
        context: format!("actor skeleton {}", item.skeleton),
        message: "no CompiledBones".into(),
    })?;
    Ok(Body { item: item.clone(), mesh, skeleton, actor, source: Arc::new(source) })
}

/// The body of a parsed CHF (its gender, customization and root port), or
/// `None` — reported on stderr — when it does not resolve or load.
pub fn load_chf_body(p4k: &MappedP4k, db: &Database, chf: &ChfData) -> Option<Body> {
    let customization = match MaterialLookup::load(db) {
        Ok(lookup) => resolve_chf_materials(&lookup, &chf.materials),
        Err(e) => {
            eprintln!("body: customization skipped ({e})");
            Vec::new()
        }
    };
    let Some(item) = resolve_body(db, &chf.itemport, gender_tag(chf.dna.gender_hash), &customization) else {
        eprintln!("body: the CHF's root port resolves to no body, skipped");
        return None;
    };
    load_body(p4k, &item).map_err(|e| eprintln!("body {}: skipped ({e})", item.geometry)).ok()
}
