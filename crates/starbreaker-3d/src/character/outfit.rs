//! What a character wears: its body and the garments of a loadout, with
//! the clothing layers resolved.
//!
//! - [`Outfit::load`]: the CHF's body ([`super::body`]), the loadout's
//!   garments ([`super::loadout::resolve_garments`], each loaded like a head
//!   item: head-parser shape + skeleton, pipeline mesh + `.mtl`) and, from
//!   every worn item's `SCItemClothingParams`, the zones each one has
//!   covered ([`super::clothing::hidden_zones`]: an undersuit covers the
//!   body) and the CHF ports hidden with everything under them (a full
//!   helmet hides `Head_ItemPort`: hair, brows, …).
//! - [`Outfit::with_body`]: just a body, nothing covered.
//! - [`Outfit::wears`]: whether a CHF head item is worn (not on a hidden
//!   port); `Outfit::parts`: the body and garments as rig parts with
//!   their covered zones (`CharacterRig::dress`).
//!
//! Gap: the DNA head itself is the rig's root and stays when its port is
//! hidden (only the CHF items under it go).

use std::collections::HashSet;
use std::sync::Arc;

use starbreaker_chf::{ChfData, ItemPort};
use starbreaker_datacore::Database;
use starbreaker_p4k::MappedP4k;

use super::body::{Body, load_chf_body};
use super::clothing::{Clothing, clothing_params, hidden_zones};
use super::head_skin::{BindPoseSkeleton, HeadMesh};
use super::items::{HeadItemMesh, load_skin_part};
use super::loadout::{Garment, LoadoutEntry, resolve_garments};
use super::part_source::PartSource;
use super::shared::{CharacterPart, stem};
use super::gender_tag;

/// A [`Garment`] with its mesh loaded.
#[derive(Clone)]
pub struct GarmentMesh {
    pub garment: Garment,
    pub mesh: HeadMesh,
    pub skeleton: BindPoseSkeleton,
    pub(crate) source: Arc<PartSource>,
    /// Zone hashes other worn items cover.
    pub hidden_zones: HashSet<u32>,
}

/// A character's body and garments (see the module docs).
#[derive(Clone, Default)]
pub struct Outfit {
    pub body: Option<Body>,
    /// Zone hashes of the body the garments cover.
    pub body_hidden_zones: HashSet<u32>,
    pub garments: Vec<GarmentMesh>,
    /// CHF ports (lowercase) hidden by a garment, with every port under them.
    hidden_chf_ports: HashSet<String>,
}

impl Outfit {
    /// Just `body` (if any), nothing worn over it.
    pub fn with_body(body: Option<Body>) -> Self {
        Self { body, ..Self::default() }
    }

    /// The CHF character's body dressed in the `loadout` (empty: just the
    /// body). Garments that fail to load are reported on stderr and skipped.
    pub fn load(p4k: &MappedP4k, db: &Database, chf: &ChfData, loadout: &[LoadoutEntry]) -> Self {
        let body = load_chf_body(p4k, db, chf);
        if loadout.is_empty() {
            return Self::with_body(body);
        }
        let worn = resolve_garments(db, loadout, gender_tag(chf.dna.gender_hash), &chf.itemport.name.to_string());
        let garments: Vec<(Garment, HeadMesh, BindPoseSkeleton, PartSource)> = worn
            .garments
            .into_iter()
            .filter_map(|g| match load_skin_part(p4k, &g.geometry, &g.material) {
                Ok((mesh, skeleton, source)) => Some((g, mesh, skeleton, source)),
                Err(e) => {
                    eprintln!("garment {}: skipped ({e})", g.item);
                    None
                }
            })
            .collect();

        let body_clothing = body
            .as_ref()
            .and_then(|b| db.record_by_id(&b.item.record))
            .map(|record| clothing_params(db, record))
            .unwrap_or_default();
        let mut layers: Vec<&Clothing> = vec![&body_clothing];
        layers.extend(garments.iter().map(|(g, ..)| &g.clothing));
        let mut hidden = hidden_zones(&layers).into_iter();
        let body_hidden_zones = hidden.next().unwrap_or_default();
        let garments = garments
            .into_iter()
            .zip(hidden)
            .map(|((garment, mesh, skeleton, source), hidden_zones)| GarmentMesh {
                garment,
                mesh,
                skeleton,
                source: Arc::new(source),
                hidden_zones,
            })
            .collect();
        let hidden_chf_ports = hidden_ports_under(&chf.itemport, &worn.hidden_ports);
        Self { body, body_hidden_zones, garments, hidden_chf_ports }
    }

    /// Whether the CHF item `item` is worn (its port is not hidden).
    pub fn wears(&self, item: &HeadItemMesh) -> bool {
        !self.hidden_chf_ports.contains(&item.item.port.to_lowercase())
    }

    /// The body and the garments as rig parts, each with its covered zones.
    pub(crate) fn parts(&self) -> Vec<CharacterPart> {
        let body = self.body.iter().map(|b| CharacterPart {
            name: stem(&b.item.geometry),
            mesh: b.mesh.clone(),
            skeleton: b.skeleton.clone(),
            source: b.source.clone(),
            hidden_zones: self.body_hidden_zones.clone(),
        });
        let garments = self.garments.iter().map(|g| CharacterPart {
            name: stem(&g.garment.geometry),
            mesh: g.mesh.clone(),
            skeleton: g.skeleton.clone(),
            source: g.source.clone(),
            hidden_zones: g.hidden_zones.clone(),
        });
        body.chain(garments).collect()
    }
}

/// The ports (lowercase) of the CHF tree under `root` that are in `hidden`
/// or below one that is.
fn hidden_ports_under(root: &ItemPort, hidden: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack = vec![(root, false)];
    while let Some((port, parent_hidden)) = stack.pop() {
        let name = port.name.to_string().to_lowercase();
        let is_hidden = parent_hidden || hidden.iter().any(|h| h.eq_ignore_ascii_case(&name));
        if is_hidden {
            out.insert(name);
        }
        stack.extend(port.children.iter().map(|c| (c, is_hidden)));
    }
    out
}
