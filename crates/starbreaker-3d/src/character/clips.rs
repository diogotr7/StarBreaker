//! What a character export plays, and the rig every character writer
//! shares.
//!
//! - [`CharacterAnimation`]: an optional face clip ([`FaceAnimation`]: a
//!   `.caf` that drives the DNA head through RigLogic — joints, blendshapes,
//!   wrinkles) and an optional body clip (an [`AnimationClip`] of bone-local
//!   samples on the actor skeleton). With both they play as one clip, named
//!   after the body clip ([`super::shared::CharacterRig::add_body_clip`]).
//! - [`load_body_clip`] / [`select_clip`]: a body clip from a `.caf`, or one
//!   clip of a `.dba` picked by name.
//! - `HeadAssets::character_rig`: the rest rig (or, with a face clip, the
//!   expression rig) + the outfit (body on its actor skeleton, garments;
//!   CHF items on hidden ports left off, the head too when its port is
//!   hidden) + the body clip. The package and
//!   the character GLB both build from it.

use starbreaker_chf::Dna;

use super::outfit::Outfit;
use super::identity_context::HeadAssets;
use super::items::HeadItemMesh;
use super::shared::CharacterRig;
use super::{CharacterError, face_anim};
use crate::animation::{AnimationClip, AnimationDatabase};

/// A face clip: the `.caf` file (full IVO) and its weight
/// (`animation_export::EXPRESSION_DEFAULT_INTENSITY` is the engine's).
#[derive(Debug, Clone, Copy)]
pub struct FaceAnimation<'a> {
    pub caf: &'a [u8],
    pub intensity: f32,
}

/// The clips a character export plays (none: the rest pose).
#[derive(Debug, Clone, Copy, Default)]
pub struct CharacterAnimation<'a> {
    pub face: Option<FaceAnimation<'a>>,
    pub body: Option<&'a AnimationClip>,
}

/// The clip of `db` named `name` — its file stem, or a unique case-insensitive
/// substring of its path — or, without a name, its only clip.
pub fn select_clip(db: AnimationDatabase, name: Option<&str>) -> Result<AnimationClip, CharacterError> {
    let listing = |clips: &[&AnimationClip]| clips.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
    let all: Vec<&AnimationClip> = db.clips.iter().collect();
    let picked: Vec<&AnimationClip> = match name {
        None => all.clone(),
        Some(name) => {
            let name = name.to_lowercase();
            let exact: Vec<&AnimationClip> =
                all.iter().copied().filter(|c| super::shared::stem(&c.name).to_lowercase() == name).collect();
            if exact.is_empty() { all.iter().copied().filter(|c| c.name.to_lowercase().contains(&name)).collect() } else { exact }
        }
    };
    match picked.as_slice() {
        [clip] => Ok((*clip).clone()),
        [] => Err(CharacterError::Animation(format!("no clip {name:?} among: {}", listing(&all)))),
        many => Err(CharacterError::Animation(format!("pick one clip (by name) of: {}", listing(many)))),
    }
}

/// A body clip from the file at `path` (`.dba` or `.caf`, by extension)
/// with contents `bytes`; `name` picks a `.dba`'s clip ([`select_clip`]).
pub fn load_body_clip(bytes: &[u8], path: &str, name: Option<&str>) -> Result<AnimationClip, CharacterError> {
    let lower = path.to_lowercase();
    let db = if lower.ends_with(".dba") {
        crate::animation::parse_dba(bytes)
    } else if lower.ends_with(".caf") {
        crate::animation::parse_caf(bytes)
    } else {
        return Err(CharacterError::Animation(format!("{path}: not a .dba or .caf")));
    }
    .map_err(|e| CharacterError::Animation(format!("{path}: {e}")))?;
    select_clip(db, name)
}

impl HeadAssets {
    /// The character at rest — or playing `animation.face` — with the CHF
    /// `items` the `outfit` leaves on, dressed in the `outfit` (the rig is
    /// then built on its body's actor skeleton) and `animation.body` playing
    /// over it.
    pub(crate) fn character_rig(
        &self,
        dna: &Dna,
        items: &[HeadItemMesh],
        outfit: &Outfit,
        animation: &CharacterAnimation<'_>,
    ) -> Result<CharacterRig, CharacterError> {
        let items: Vec<HeadItemMesh> = items.iter().filter(|i| outfit.wears(i)).cloned().collect();
        let items = items.as_slice();
        let mut rig = match animation.face {
            Some(face) => {
                let chunk = face_anim::load_caf_chunk_from_ivo(face.caf)
                    .map_err(|e| CharacterError::FaceAnim(format!("{e}")))?;
                self.expression_rig_state(dna, chunk, face.intensity, items)?
            }
            None => self.rig_state(dna, true, true, items)?,
        };
        if outfit.hides_geometry(self.head_skin_path()) {
            rig.hide_head(|part| self.is_head_submesh(part));
        }
        rig.dress(outfit);
        if let Some(clip) = animation.body {
            rig.add_body_clip(clip);
        }
        Ok(rig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(names: &[&str]) -> AnimationDatabase {
        AnimationDatabase {
            clips: names
                .iter()
                .map(|n| AnimationClip { name: (*n).into(), fps: 30.0, channels: Vec::new(), start_rotation: None, start_position: None })
                .collect(),
        }
    }

    const SELECT: [&str; 3] = [
        "animations/characters/human/male_v7/css/charselect_heavy_marine.caf",
        "animations/characters/human/male_v7/css/char_customizer_stand_idle_01.caf",
        "animations/characters/human/male_v7/css/charselect_heavy_outlaw.caf",
    ];

    #[test]
    fn a_clip_is_picked_by_stem_or_unique_substring() {
        let stem = select_clip(db(&SELECT), Some("CHAR_CUSTOMIZER_STAND_IDLE_01")).unwrap();
        assert_eq!(stem.name, SELECT[1]);
        let sub = select_clip(db(&SELECT), Some("outlaw")).unwrap();
        assert_eq!(sub.name, SELECT[2]);
    }

    #[test]
    fn an_exact_stem_beats_longer_matches() {
        let names = ["a/idle.caf", "a/idle_long.caf"];
        assert_eq!(select_clip(db(&names), Some("idle")).unwrap().name, "a/idle.caf");
    }

    #[test]
    fn ambiguous_or_missing_names_list_the_clips() {
        let err = select_clip(db(&SELECT), Some("charselect")).unwrap_err().to_string();
        assert!(err.contains("charselect_heavy_marine") && err.contains("charselect_heavy_outlaw"), "{err}");
        let err = select_clip(db(&SELECT), Some("walk")).unwrap_err().to_string();
        assert!(err.contains("char_customizer_stand_idle_01"), "{err}");
        assert!(select_clip(db(&SELECT), None).is_err(), "a database needs a name");
        assert_eq!(select_clip(db(&SELECT[..1]), None).unwrap().name, SELECT[0], "a single clip needs none");
    }

    #[test]
    fn body_clip_files_go_by_extension() {
        let err = load_body_clip(&[], "x/idle.chr", None).unwrap_err().to_string();
        assert!(err.contains("not a .dba or .caf"), "{err}");
        assert!(load_body_clip(&[], "x/idle.dba", None).is_err(), "unparseable bytes are an error");
    }
}
