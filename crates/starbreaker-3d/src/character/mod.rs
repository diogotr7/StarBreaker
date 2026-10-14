//! Character customization pipeline: `.chf` → rigged character.
//!
//! [`chf_to_character_glb`] is the entry point: CHF parse → identity
//! ([`identity`]) → head rig assets ([`HeadAssets`]: CDF, head `.skin` /
//! `.skinm`, master `.dna`, RigLogic) → the per-character DNA identity
//! ([`dna_matrix`], [`morphing`]) → [`HeadAssets::character_rig`] (head items,
//! body and garments from [`outfit`], face / body clips from [`clips`]) →
//! the shared glTF writer. [`package::export_character_package`] writes the
//! same rig as a decomposed `.blend` package. See `docs/riglogic/PIPELINE.md`.

use starbreaker_common::NameHash;
use starbreaker_p4k::MappedP4k;
use thiserror::Error;

pub mod animation_export;
pub mod body;
pub mod clips;
pub mod clothing;
pub mod loadout;
pub mod outfit;
pub mod wear;
pub mod apply;
pub mod blendshapes;
pub mod cdf;
pub mod controls;
pub mod customization;
pub mod dna_matrix;
pub mod expression;
pub mod submesh;
pub mod wrap;
pub mod wrinkles;
pub mod face_anim;
pub mod head_skin;
pub mod identity;
pub mod items;
mod identity_context;
pub(crate) mod part_source;
pub mod morphing;
pub mod package;
pub(crate) mod shared;
pub mod normals;

pub use animation_export::{ExpressionAnim, ExpressionBoneTrack};
pub use identity_context::{HeadAssets, HeadIdentityContext};

#[derive(Debug, Error)]
pub enum CharacterError {
    #[error("CHF parse failed: {0}")]
    Chf(#[from] starbreaker_chf::ChfError),

    #[error(
        "unsupported character identity: gender_hash={gender_hash:?}, variant_hash={variant_hash:?} \
         (no human head rig for it)"
    )]
    UnsupportedIdentity {
        gender_hash: NameHash,
        variant_hash: NameHash,
    },

    #[error("P4k read failed for {path}: {message}")]
    P4kRead { path: String, message: String },

    #[error("CDF parse failed at {context}: {message}")]
    CdfParse { context: String, message: String },

    #[error("head .skin parse failed at {context}: {message}")]
    HeadSkinParse { context: String, message: String },

    #[error("RigLogic parse failed: {0}")]
    RigLogic(#[from] starbreaker_common::ParseError),

    #[error("RBF evaluation failed: {0}")]
    Rbf(String),

    #[error(
        "joint mapping failed: RigLogic joint index {rig_joint_idx} (CRC {crc:?}) \
         has no matching bone in .skin"
    )]
    JointMappingFailed { rig_joint_idx: u32, crc: NameHash },

    #[error("glTF emit failed: {0}")]
    GlbEmit(String),

    #[error("DNA parse failed: {0}")]
    Dna(#[from] starbreaker_dna::DnaError),

    #[error("morph failed: {0}")]
    Morph(#[from] morphing::MorphError),

    #[error("chf weights failed: {0}")]
    ChfWeights(#[from] dna_matrix::chf_weights::ChfWeightsError),

    #[error("top-k prune failed: {0}")]
    TopK(#[from] dna_matrix::top_k::TopKError),

    #[error("dna matrix build failed: {0}")]
    DnaMatrix(#[from] dna_matrix::accumulator::DnaMatrixError),

    #[error("control input failed: {0}")]
    Control(#[from] controls::ControlError),

    #[error("face anim load/parse failed: {0}")]
    FaceAnim(String),

    #[error("character customization: {0}")]
    Customization(String),

    #[error("animation: {0}")]
    Animation(String),

    #[error("loadout: {0}")]
    Loadout(String),
}

/// The whole CHF character as a GLB — head items, body on the actor
/// skeleton, the head's blendshapes — wearing `loadout` ([`loadout`],
/// [`outfit`]; empty: just the body) and playing `animation` (a face and/or
/// body clip, as one clip). The same rig the package writes
/// ([`package::export_character_package`]).
pub fn chf_to_character_glb(
    chf_bytes: &[u8],
    p4k: &MappedP4k,
    db: &starbreaker_datacore::Database,
    animation: &clips::CharacterAnimation<'_>,
    loadout: &[loadout::LoadoutEntry],
) -> Result<Vec<u8>, CharacterError> {
    let chf_file = starbreaker_chf::ChfFile::from_chf(chf_bytes)?;
    let chf = chf_file.parse().map_err(|e| CharacterError::Chf(e.into()))?;
    let items = items::load_head_items(
        p4k,
        &items::resolve_head_items(db, &chf.itemport, gender_tag(chf.dna.gender_hash)),
    );
    let assets = identity_context::HeadAssets::load(p4k, chf.dna.gender_hash, chf.dna.variant_hash)?;
    let outfit = outfit::Outfit::load(p4k, db, &chf, loadout);
    assets.character_rig(&chf.dna, &items, &outfit, animation)?.to_glb()
}

/// A character's gender: picks gendered meshes (DataCore sub-geometry tags
/// `Male` / `Female`) and the actor skeleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gender {
    #[default]
    Male,
    Female,
}

impl Gender {
    /// The DataCore sub-geometry tag.
    pub fn tag(self) -> &'static str {
        match self {
            Gender::Male => "Male",
            Gender::Female => "Female",
        }
    }

    /// The gender of a DNA rig (`Dna::gender_hash`).
    pub fn from_hash(gender_hash: NameHash) -> Self {
        if gender_hash == identity::FEMALE_T1_PU_GENDER_HASH { Gender::Female } else { Gender::Male }
    }
}

impl std::str::FromStr for Gender {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "male" => Ok(Gender::Male),
            "female" => Ok(Gender::Female),
            other => Err(format!("unknown gender {other:?} (male or female)")),
        }
    }
}

/// The game's default character of `gender` (the character customizer's
/// `masculinedefault` / `femininedefault` CHF), to wear exported armour.
pub fn default_wearer_chf(p4k: &MappedP4k, gender: Gender) -> Result<Vec<u8>, CharacterError> {
    let name = match gender {
        Gender::Male => "masculinedefault",
        Gender::Female => "femininedefault",
    };
    p4k_read(p4k, &format!(r"Data\Libs\CharacterCustomizer\{name}.chf"))
}

/// DataCore sub-geometry gender tag (`"Male"` / `"Female"`) for a rig.
pub fn gender_tag(gender_hash: NameHash) -> &'static str {
    Gender::from_hash(gender_hash).tag()
}

/// A co-located head sub-mesh (eyes, teeth, eye-details) from the CDF.
#[derive(Clone)]
pub(crate) struct HeadSubMesh {
    pub mesh: head_skin::HeadMesh,
    pub skeleton: head_skin::BindPoseSkeleton,
    pub deformer: cdf::DeformerKind,
    /// P4k path of the part's `.skin`.
    pub skin_path: String,
    /// The part's own `.dna` (eyes/teeth carry per-variant vertex deltas for
    /// their face parts); `None` when the part has none.
    pub dna: Option<Vec<u8>>,
    /// The part as the entity pipeline loads it (with the `.skin`'s own
    /// material).
    pub source: std::sync::Arc<part_source::PartSource>,
}

/// Load the head's co-located CDF sub-meshes (eyes, teeth, eye-details) as
/// static bind-pose meshes for inclusion in the GLB.
///
/// Selects attachments whose `.skin` lives in the SAME directory as the head
/// `.skin` (structural co-location — excludes body/clothing attachments
/// without gating on names), skips the head itself, and tolerates sub-meshes
/// our loader can't yet parse (logs + skips). Sub-mesh `.skinm` files are a
/// single interleaved `IvoSkin2` chunk, so [`head_skin::parse_skin_mesh`]
/// recovers positions/indices but not skin weights — the meshes are placed at
/// bind pose.
fn load_head_submeshes(
    p4k: &MappedP4k,
    cdf: &cdf::CharacterDefinition,
    head_skin_path: &str,
) -> Vec<HeadSubMesh> {
    fn dir_of(p: &str) -> String {
        let s = p.replace('\\', "/").to_lowercase();
        s.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
    }
    let head_binding = cdf.head_attachment.binding_skin.replace('\\', "/").to_lowercase();
    let head_dir = dir_of(&cdf.head_attachment.binding_skin);
    let p4k_dir = head_skin_path.rsplit_once('\\').map(|(d, _)| d).unwrap_or("");
    let mut out = Vec::new();
    for att in &cdf.attachments {
        if att.binding_skin.replace('\\', "/").to_lowercase() == head_binding {
            continue; // the head itself
        }
        if dir_of(&att.binding_skin) != head_dir {
            continue; // not co-located with the head
        }
        let fname = att.binding_skin.rsplit(['/', '\\']).next().unwrap_or(&att.binding_skin);
        let skin = format!("{p4k_dir}\\{fname}");
        let skinm = format!("{skin}m");
        match (p4k.read_file(&skin), p4k.read_file(&skinm)) {
            (Ok(s), Ok(sm)) => match head_skin::parse_skin_mesh(&s, &sm) {
                Ok((mesh, skeleton)) => match part_source::PartSource::load(p4k, &skin, "") {
                    Ok(source) => out.push(HeadSubMesh {
                        mesh,
                        skeleton,
                        deformer: att.deformer.clone(),
                        skin_path: skin.clone(),
                        // Eyes/teeth ship their own `.dna` (`<stem>.dna`).
                        dna: skin
                            .strip_suffix(".skin")
                            .and_then(|stem| p4k.read_file(&format!("{stem}.dna")).ok()),
                        source: std::sync::Arc::new(source),
                    }),
                    Err(e) => eprintln!("head sub-mesh {fname}: pipeline load skipped ({e})"),
                },
                Err(e) => eprintln!("head sub-mesh {fname}: parse skipped ({e:?})"),
            },
            _ => eprintln!("head sub-mesh {fname}: .skin/.skinm not found, skipped"),
        }
    }
    out
}

/// Parse the `BShapesGpu` (`0x57A38888`) chunk out of a `.skinm` file's IVO
/// container and decode it via [`blendshapes::parse_bshapes_gpu_with_slot17`].
///
/// `slot17_bytes`, when supplied, modulates the BShapesGpu output per the
/// Pass 15 hypothesis (per-(vertex, blendshape) byte / 255 multiplier).
fn read_bshapes_gpu_from_skinm(
    skinm_bytes: &[u8],
    slot17_bytes: Option<&[u8]>,
) -> Result<blendshapes::BlendShapes, CharacterError> {
    use starbreaker_chunks::ChunkFile;
    let cf = ChunkFile::from_bytes(skinm_bytes).map_err(|e| CharacterError::HeadSkinParse {
        context: "bshapes_gpu ChunkFile::from_bytes".into(),
        message: format!("{e:?}"),
    })?;
    let ivo = match cf {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => {
            return Err(CharacterError::HeadSkinParse {
                context: "bshapes_gpu ChunkFile".into(),
                message: ".skinm must be IVO, got CrCh".into(),
            });
        }
    };
    let entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::BSHAPES_GPU)
        .ok_or_else(|| CharacterError::HeadSkinParse {
            context: "bshapes_gpu chunk lookup".into(),
            message: format!(
                "missing BSHAPES_GPU chunk (0x{:08X}) in .skinm",
                starbreaker_chunks::known_types::ivo::BSHAPES_GPU
            ),
        })?;
    blendshapes::parse_bshapes_gpu_with_slot17(ivo.chunk_data(entry), slot17_bytes).map_err(|e| {
        CharacterError::HeadSkinParse {
            context: "bshapes_gpu parse".into(),
            message: format!("{e:?}"),
        }
    })
}

/// Read a file from the P4k archive, mapping errors to [`CharacterError::P4kRead`].
fn p4k_read(p4k: &MappedP4k, path: &str) -> Result<Vec<u8>, CharacterError> {
    p4k.read_file(path).map_err(|e| CharacterError::P4kRead {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod gender_tests {
    use super::*;

    #[test]
    fn gender_parses_and_tags() {
        assert_eq!("FEMALE".parse::<Gender>(), Ok(Gender::Female));
        assert_eq!("male".parse::<Gender>(), Ok(Gender::Male));
        assert!("robot".parse::<Gender>().is_err());
        assert_eq!(Gender::default().tag(), "Male");
        assert_eq!(Gender::from_hash(identity::FEMALE_T1_PU_GENDER_HASH), Gender::Female);
    }
}
