//! Stand-in cores: a rigid prop (a backpack) worn with no core under it.
//!
//! In the game a backpack hangs on its core armour's `backpack` bone port
//! ([`super::props`]), and the port's bone (`backpack_attach_1_override`) is
//! not in the body skeleton: each core's `.skin` carries it, under `Spine3`,
//! where that armour's back is. A backpack picked with no core has no port
//! and no bone to go on. A stand-in core supplies both: a core that would be
//! worn there (a free port accepts it) and whose bone port accepts the prop
//! is worn for its port and the mount bone of its `.skin` only — its mesh is
//! not written, its clothing layers do not apply, and the package records
//! which core stood in ([`super::package`]).
//!
//! Which core ([`rank`]): one of the prop's own line (the same name before
//! the piece word: `kap_combat_heavy_backpack_…` → `kap_combat_heavy_core_…`)
//! when there is one, else one whose port is the prop's size (`MaxSize` 1 on
//! light cores, 2 medium, 3 heavy; the prop's `AttachDef.Size`), else a
//! larger one; in each, the mount most cores of that group share (where
//! their `.skin`s bind the bone, to the millimetre: the line's or the weight
//! class's usual mount — many skins share one), then by skin and name. A
//! core whose `.skin` lacks the bone is passed over.
//!
//! - [`place_with_stand_ins`]: placement ([`super::wear::place_items`]) that
//!   adds a stand-in for each rigid item no free port took (pure);
//!   [`dress_with_stand_ins`]: the same on a CHF character with DataCore and
//!   P4k ([`candidates`], [`skin_skeleton`]).
//! - [`StandIn`] (placement), [`StandInCore`] (resolved,
//!   [`super::loadout::Garments::stand_ins`]), [`StandInMount`] (its bones,
//!   [`super::outfit::Outfit::mounts`]); [`mount_skeleton`]: the mount bones
//!   and their ancestors out of a `.skin`'s skeleton.

use starbreaker_chf::ChfData;
use starbreaker_datacore::Database;
use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_datacore::query::value::Value;
use starbreaker_p4k::MappedP4k;

use super::CharacterError;
use super::head_skin::BindPoseSkeleton;
use super::loadout::{LoadoutEntry, worn_geometry};
use super::wear::{PortDef, Placement, Seed, Wearable, dress, place_items, text, wearable};

/// A picked item worn on a stand-in core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandIn {
    pub item: String,
    pub core: String,
}

/// A core that could stand in for a prop: its `.skin` for the wearer's
/// gender, the `MaxSize` and bone of its port that accepts the prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub core: String,
    pub skin: String,
    pub max_size: Option<i32>,
    pub bone: String,
    /// Where the `.skin` binds the bone ([`mount_key`]); `None` when not
    /// read.
    pub mount: Option<[i32; 7]>,
}

/// A stand-in core of a loadout resolved for a gender: the port it is on,
/// its `.skin`, the bones of its bone ports and the props worn on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandInCore {
    pub item: String,
    pub port: String,
    pub geometry: String,
    pub bones: Vec<String>,
    pub items: Vec<String>,
}

/// A [`StandInCore`] with its mount bones loaded ([`mount_skeleton`]).
#[derive(Debug, Clone)]
pub struct StandInMount {
    pub core: StandInCore,
    pub skeleton: BindPoseSkeleton,
}

/// An item's line: its name's `_` words before the first numeric word,
/// without the last (the piece: `backpack`, `core`). `None` with fewer than
/// two such words.
pub fn line(name: &str) -> Option<String> {
    let words: Vec<&str> = name.split('_').take_while(|w| !w.chars().all(|c| c.is_ascii_digit())).collect();
    (words.len() >= 2).then(|| words[..words.len() - 1].join("_").to_ascii_lowercase())
}

/// `candidates` for the prop `item` of `size`, best first (see the module
/// docs); cores whose port is smaller than the prop are left out.
pub fn rank<'c>(candidates: &'c [Candidate], item: &str, size: Option<i32>) -> Vec<&'c Candidate> {
    let own = line(item);
    let tier = |c: &Candidate| {
        if own.is_some() && line(&c.core) == own {
            0
        } else if size.is_some() && c.max_size == size {
            1
        } else {
            2
        }
    };
    let fits = |c: &&Candidate| match (c.max_size, size) {
        (Some(max), Some(size)) => max >= size,
        _ => true,
    };
    let fitting: Vec<&Candidate> = candidates.iter().filter(fits).collect();
    // One mount: the same bind of the bone (else, unread, the same skin).
    let same = |a: &Candidate, b: &Candidate| match (a.mount, b.mount) {
        (Some(a), Some(b)) => a == b,
        _ => a.skin.eq_ignore_ascii_case(&b.skin),
    };
    let shared = |c: &Candidate| fitting.iter().filter(|o| tier(o) == tier(c) && same(o, c)).count();
    let key = |c: &Candidate| (tier(c), std::cmp::Reverse(shared(c)), c.mount, c.skin.to_ascii_lowercase(), c.core.clone());
    let mut ranked = fitting.clone();
    ranked.sort_by_key(|c| key(c));
    ranked
}

/// The bones named `bones` (any case) out of `skeleton`, with their
/// ancestors, parents remapped; `None` when it has none of them.
pub fn mount_skeleton(skeleton: &BindPoseSkeleton, bones: &[String]) -> Option<BindPoseSkeleton> {
    let mut keep = vec![false; skeleton.bones.len()];
    for (i, bone) in skeleton.bones.iter().enumerate() {
        if bones.iter().any(|b| b.eq_ignore_ascii_case(&bone.name)) {
            let mut at = Some(i);
            while let Some(j) = at.filter(|&j| !keep[j]) {
                keep[j] = true;
                at = skeleton.bones[j].parent.map(|p| p as usize).filter(|&p| p < skeleton.bones.len());
            }
        }
    }
    if !keep.contains(&true) {
        return None;
    }
    let mut new_index = vec![None; skeleton.bones.len()];
    let mut kept = Vec::new();
    for (i, bone) in skeleton.bones.iter().enumerate().filter(|(i, _)| keep[*i]) {
        let mut bone = bone.clone();
        bone.parent = bone.parent.and_then(|p| new_index.get(p as usize).copied().flatten());
        new_index[i] = Some(kept.len() as u32);
        kept.push(bone);
    }
    Some(BindPoseSkeleton { bones: kept })
}

/// [`place_items`], then for each rigid item still unplaced the stand-in
/// core `stand_in` offers (given the free ports and the item) is worn too,
/// when the item then goes on it and nothing else loses its place. The
/// stand-in's entry is flagged ([`LoadoutEntry::stand_in`]) and listed in
/// [`Placement::stand_ins`].
pub fn place_with_stand_ins(
    worn: &Seed,
    items: &[Wearable],
    mut stand_in: impl FnMut(&[PortDef], &Wearable) -> Option<Wearable>,
) -> Placement {
    let mut placement = place_items(worn, items);
    let mut wearing = items.to_vec();
    let mut stand_ins = Vec::new();
    for item in items.iter().filter(|w| w.rigid) {
        if find(&placement.entries, &item.item).is_some() {
            continue;
        }
        let Some(core) = stand_in(&placement.free_ports, item) else { continue };
        let mut trial = wearing.clone();
        trial.push(core.clone());
        let tried = place_items(worn, &trial);
        let on_core = find(&tried.entries, &core.item).is_some_and(|c| c.children.iter().any(|e| e.item == item.item));
        if on_core && tried.unplaced.len() + 1 == placement.unplaced.len() {
            wearing = trial;
            placement = tried;
            stand_ins.push(StandIn { item: item.item.clone(), core: core.item });
        }
    }
    for s in &stand_ins {
        flag(&mut placement.entries, &s.core);
    }
    placement.stand_ins = stand_ins;
    placement
}

/// The entry wearing `item`, depth first.
fn find<'e>(entries: &'e [LoadoutEntry], item: &str) -> Option<&'e LoadoutEntry> {
    entries.iter().find_map(|e| if e.item == item { Some(e) } else { find(&e.children, item) })
}

/// Flag the entry wearing `core` as a stand-in.
fn flag(entries: &mut [LoadoutEntry], core: &str) -> bool {
    entries.iter_mut().any(|e| {
        if e.item == core {
            e.stand_in = true;
            true
        } else {
            flag(&mut e.children, core)
        }
    })
}

/// The cores that could stand in for the rigid `item` on a wearer of
/// `gender`: entity classes a free skin port of `free_ports` accepts, that
/// have a `.skin` for the gender and a bone port accepting the item.
pub(crate) fn candidates(
    index: &EntityIndex,
    free_ports: &[PortDef],
    item: &Wearable,
    gender: &str,
) -> Result<Vec<Candidate>, CharacterError> {
    let db = index.db;
    let skin_ports: Vec<&PortDef> = free_ports.iter().filter(|p| p.bone.is_none()).collect();
    let Some(entity) = db.struct_id("EntityClassDefinition") else { return Ok(Vec::new()) };
    if skin_ports.is_empty() {
        return Ok(Vec::new());
    }
    let Ok(type_path) = db.compile_path::<Value>(entity, "Components[SAttachableComponentParams].AttachDef.Type") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for record in db.records_of_type(entity) {
        let Some(item_type) = db.query::<Value>(&type_path, record).ok().and_then(|v| v.iter().find_map(text)) else { continue };
        if !skin_ports.iter().any(|p| p.types.iter().any(|t| t.name.eq_ignore_ascii_case(&item_type))) {
            continue;
        }
        let name = db.resolve_string2(record.name_offset).rsplit('.').next().unwrap_or_default();
        let Some(core) = wearable(db, index, name)? else { continue };
        if !skin_ports.iter().any(|p| p.accepts(&core)) {
            continue;
        }
        let Some(port) = core.ports.iter().find(|p| p.bone.is_some() && p.accepts(item)) else { continue };
        let Some(worn) = worn_geometry(db, record, gender, &[gender.to_owned()]).filter(|w| !w.other_gender) else { continue };
        out.push(Candidate {
            core: name.to_owned(),
            skin: worn.geometry,
            max_size: port.max_size,
            bone: port.bone.as_ref().map(|b| b.bone.clone()).unwrap_or_default(),
            mount: None,
        });
    }
    Ok(out)
}

/// The bind-pose skeleton of the `.skin` at DataCore path `geometry`.
pub fn skin_skeleton(p4k: &MappedP4k, geometry: &str) -> Result<BindPoseSkeleton, CharacterError> {
    let skin = super::p4k_read(p4k, &format!(r"Data\{}", geometry.replace('/', r"\")))?;
    super::head_skin::parse_skin_skeleton(&skin)
}

/// Where a bone sits in its `.skin`: its parent-relative bind, translation
/// in millimetres and rotation in thousandths, so binds equal to the
/// millimetre are one mount.
pub fn mount_key(bone: &super::head_skin::Bone) -> [i32; 7] {
    let t = bone.bind_translation.map(|v| (v * 1000.0).round() as i32);
    let r = bone.bind_rotation.map(|v| (v * 1000.0).round() as i32);
    [t[0], t[1], t[2], r[0], r[1], r[2], r[3]]
}

/// [`super::wear::dress_entries`] where a rigid item no port takes (a
/// backpack with no core) goes on a stand-in core: the best ranked
/// ([`rank`]) of the [`candidates`] whose `.skin` has its port's bone,
/// each with its mount read from the `.skin`.
pub fn dress_with_stand_ins(
    p4k: &MappedP4k,
    db: &Database,
    chf: &ChfData,
    loadout: &[LoadoutEntry],
    items: &[String],
) -> Result<Placement, CharacterError> {
    let gender = super::gender_tag(chf.dna.gender_hash);
    dress(db, chf, loadout, items, |index, worn, wearables| {
        let mut failed = None;
        let placement = place_with_stand_ins(worn, wearables, |free, item| {
            let found = match candidates(index, free, item, gender) {
                Ok(found) => found,
                Err(e) => {
                    failed = Some(e);
                    return None;
                }
            };
            let mut skins: std::collections::HashMap<String, Option<BindPoseSkeleton>> = std::collections::HashMap::new();
            let found: Vec<Candidate> = found
                .into_iter()
                .filter_map(|c| {
                    let skin = skins.entry(c.skin.to_ascii_lowercase()).or_insert_with(|| skin_skeleton(p4k, &c.skin).ok());
                    let bone = skin.as_ref()?.bones.iter().find(|b| b.name.eq_ignore_ascii_case(&c.bone))?;
                    Some(Candidate { mount: Some(mount_key(bone)), ..c })
                })
                .collect();
            let core = rank(&found, &item.item, item.size).into_iter().next()?;
            wearable(index.db, index, &core.core).ok().flatten()
        });
        failed.map_or(Ok(placement), Err)
    })
}

#[cfg(test)]
mod tests;
