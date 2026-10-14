//! Stage 2: identity resolution.
//!
//! Supports the two t1_pu rigs (male and female). The hardcoded mappings
//! below tie CHF identity hashes to in-P4k asset paths. Future work will
//! replace this with a DataCore lookup so additional tiers/variants
//! resolve automatically.

use starbreaker_common::NameHash;

use super::CharacterError;

/// In-P4k asset paths for one rig identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetPaths {
    pub riglogic: &'static str,
    pub cdf: &'static str,
    pub head_skin: &'static str,
    /// Master `.dna` file path. Carries per-(joint × face-part) mixing
    /// matrix (slot 21) and variant delta payloads (slot 31) required by
    /// the DNA-driven morph pipeline.
    pub dna: &'static str,
}

/// The male t1_pu paths, verified manually from the P4k file listing in
/// the prior RigLogic milestone (Tasks 0-9). If these strings ever fail to
/// resolve, re-list the P4k via `starbreaker p4k list` and update.
pub const MALE_T1_PU: AssetPaths = AssetPaths {
    riglogic: r"Data\Objects\Characters\RigLogic\protos_human_male_face_t1_pu.RigLogic",
    cdf: r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\dna_riglogicspu.cdf",
    head_skin: r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\protos_human_male_face_t1_pu_head.skin",
    dna: r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\protos_human_male_face_t1_pu_head.dna",
};

/// The female t1_pu paths, verified from the P4k listing (Phase 18 follow-up).
/// Structurally identical to [`MALE_T1_PU`] with `male` → `female` substitution.
pub const FEMALE_T1_PU: AssetPaths = AssetPaths {
    riglogic: r"Data\Objects\Characters\RigLogic\protos_human_female_face_t1_pu.RigLogic",
    cdf: r"Data\Objects\Characters\Human\heads\female\pu\protos_human_female_face_t1_pu\dna_riglogicspu.cdf",
    head_skin: r"Data\Objects\Characters\Human\heads\female\pu\protos_human_female_face_t1_pu\protos_human_female_face_t1_pu_head.skin",
    dna: r"Data\Objects\Characters\Human\heads\female\pu\protos_human_female_face_t1_pu\protos_human_female_face_t1_pu_head.dna",
};

/// `gender_hash` for the male t1_pu rig.
///
/// Verified empirically against `masculinedefault.chf` from `Data.p4k`
/// (see `print_real_chf_hashes` test). Equals `crc32c("protos_human_male_face_t1_pu")`.
///
/// **Cross-link:** this value is identical to `RigLogic.identity_crc_lo`
/// for the same rig — see `riglogic.rs` (Task 6 of the prior milestone
/// established the equality). If they ever diverge, one of the two parsers
/// is reading the wrong bytes.
pub const MALE_T1_PU_GENDER_HASH: NameHash = NameHash(0xDD6C67F6);

/// `variant_hash` of the canonical `masculinedefault.chf` fixture.
///
/// Pinned for cross-checking against the live file. The CHF
/// `variant_hash` is per-customization (every saved character has its
/// own — confirmed empirically against the
/// `research/websiteCharacters/` corpus where >1300 CHFs share
/// `gender_hash = MALE_T1_PU_GENDER_HASH` but expose many distinct
/// `variant_hash` values: `0x65E740D3`, `0x65D75204`, `0x67448F99`,
/// `0x68F90E3F`, …). [`resolve_head_rig`] therefore matches on
/// `gender_hash` alone.
pub const MALE_T1_PU_VARIANT_HASH: NameHash = NameHash(0x674986D1);

/// `gender_hash` for the female t1_pu rig =
/// `crc32c("protos_human_female_face_t1_pu")`. Pinned in
/// [`female_gender_hash_matches_crc32c_of_rig_name`] against the
/// `femininedefault.chf` fixture.
pub const FEMALE_T1_PU_GENDER_HASH: NameHash = NameHash(0x9EF4EB54);

/// `variant_hash` of the canonical `femininedefault.chf` fixture.
/// Pinned for reference only — see [`MALE_T1_PU_VARIANT_HASH`] for
/// why `variant_hash` is not part of the rig-resolution key.
pub const FEMALE_T1_PU_VARIANT_HASH: NameHash = NameHash(0x67448F99);

/// Resolve a `(gender_hash, variant_hash)` pair to in-P4k paths.
///
/// `variant_hash` is per-customization (each saved CHF carries its
/// own) and not load-bearing for asset resolution. Only `gender_hash`
/// selects the rig. Supports the male and female t1_pu rigs; errors
/// on anything else.
pub fn resolve_head_rig(
    gender_hash: NameHash,
    variant_hash: NameHash,
) -> Result<AssetPaths, CharacterError> {
    if gender_hash == MALE_T1_PU_GENDER_HASH {
        Ok(MALE_T1_PU)
    } else if gender_hash == FEMALE_T1_PU_GENDER_HASH {
        Ok(FEMALE_T1_PU)
    } else {
        Err(CharacterError::UnsupportedIdentity {
            gender_hash,
            variant_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_male_t1_pu_hashes() {
        let paths = resolve_head_rig(MALE_T1_PU_GENDER_HASH, MALE_T1_PU_VARIANT_HASH)
            .expect("known hashes should resolve");
        assert!(paths.riglogic.ends_with("protos_human_male_face_t1_pu.RigLogic"));
        assert!(paths.cdf.ends_with("dna_riglogicspu.cdf"));
        assert!(paths.head_skin.ends_with("_head.skin"));
    }

    #[test]
    fn male_t1_pu_paths_include_dna() {
        let paths = MALE_T1_PU;
        assert!(paths.dna.ends_with("protos_human_male_face_t1_pu_head.dna"));
        assert!(paths.dna.contains("male"));
        assert!(paths.dna.contains("pu"));
    }

    #[test]
    fn male_gender_hash_matches_crc32c_of_rig_name() {
        assert_eq!(
            MALE_T1_PU_GENDER_HASH,
            NameHash::from_string("protos_human_male_face_t1_pu")
        );
    }

    #[test]
    fn female_gender_hash_matches_crc32c_of_rig_name() {
        let computed = NameHash::from_string("protos_human_female_face_t1_pu");
        assert_eq!(
            FEMALE_T1_PU_GENDER_HASH.value(),
            computed.value(),
            "FEMALE_T1_PU_GENDER_HASH = 0x{:08X}, computed = 0x{:08X}",
            FEMALE_T1_PU_GENDER_HASH.value(),
            computed.value()
        );
    }

    #[test]
    fn resolves_female_t1_pu_hashes() {
        let paths =
            resolve_head_rig(FEMALE_T1_PU_GENDER_HASH, FEMALE_T1_PU_VARIANT_HASH)
                .expect("female hashes should resolve");
        assert!(paths.riglogic.ends_with("protos_human_female_face_t1_pu.RigLogic"));
        assert!(paths.dna.ends_with("protos_human_female_face_t1_pu_head.dna"));
        assert!(paths.dna.contains("female"));
    }

    #[test]
    fn rejects_unknown_identity() {
        let bad = NameHash(0xDEADBEEF);
        let result = resolve_head_rig(bad, bad);
        assert!(matches!(
            result,
            Err(CharacterError::UnsupportedIdentity { .. })
        ));
    }

    /// The default characters' CHFs carry the pinned rig hashes. Skipped
    /// without a Data.p4k.
    #[test]
    fn default_chfs_carry_the_pinned_hashes() {
        for (chf, gender, variant) in [
            ("masculinedefault", MALE_T1_PU_GENDER_HASH, MALE_T1_PU_VARIANT_HASH),
            ("femininedefault", FEMALE_T1_PU_GENDER_HASH, FEMALE_T1_PU_VARIANT_HASH),
        ] {
            let Some(bytes) = crate::test_p4k::read(&format!(r"Data\Libs\CharacterCustomizer\{chf}.chf")) else { return };
            let dna = starbreaker_chf::ChfFile::from_chf(&bytes).unwrap().parse().unwrap().dna;
            assert_eq!((dna.gender_hash, dna.variant_hash), (gender, variant), "{chf}");
        }
    }
}
