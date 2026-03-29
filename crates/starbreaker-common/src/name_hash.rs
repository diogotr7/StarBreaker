use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// A CRC32C hash of a name string, used as a compact identifier in CHF files.
#[derive(Clone, Copy, PartialEq, Eq, Hash, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct NameHash(pub u32);

impl NameHash {
    /// Compute the CRC32C hash of a string and return a `NameHash`.
    pub fn from_string(s: &str) -> Self {
        NameHash(crc32c::crc32c(s.as_bytes()))
    }

    /// Return the raw `u32` hash value.
    pub fn value(self) -> u32 {
        self.0
    }

    /// Look up the original name string, if it is in the known-names table.
    pub fn name(&self) -> Option<&'static str> {
        let v = self.value();
        HASH_TO_NAME.get(&v).copied()
    }
}

impl fmt::Display for NameHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => {
                let v = self.value();
                write!(f, "0x{v:08X}")
            }
        }
    }
}

impl fmt::Debug for NameHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "NameHash({name})"),
            None => {
                let v = self.value();
                write!(f, "NameHash(0x{v:08X})")
            }
        }
    }
}

// ─── Serde ──────────────────────────────────────────────────────────────────

impl serde::Serialize for NameHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for NameHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <String>::deserialize(deserializer)?;
        // Try "0x..." hex literal first.
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            let value = u32::from_str_radix(hex, 16).map_err(serde::de::Error::custom)?;
            return Ok(NameHash(value));
        }
        // Check manual mappings first (these names don't equal crc32c(name)).
        if let Some(&hash) = NAME_TO_HASH.get(s.as_str()) {
            return Ok(NameHash(hash));
        }
        // Otherwise compute CRC32C of the string.
        Ok(NameHash::from_string(&s))
    }
}

// ─── Lookup table ───────────────────────────────────────────────────────────

/// Names whose CRC32C hashes are known.
const KNOWN_NAMES: &[&str] = &[
    "BaseMelanin",
    "BaseMelaninRedness",
    "BaseMelaninVariation",
    "DyeAmount",
    "DyeFadeout",
    "DyePigmentVariation",
    "DyeShift",
    "FrecklesAmount",
    "FrecklesOpacity",
    "Makeup1MetalnessB",
    "Makeup1MetalnessG",
    "Makeup1MetalnessR",
    "Makeup1NumTilesU",
    "Makeup1NumTilesV",
    "Makeup1OffsetU",
    "Makeup1OffsetV",
    "Makeup1Opacity",
    "Makeup1SmoothnessB",
    "Makeup1SmoothnessG",
    "Makeup1SmoothnessR",
    "Makeup2MetalnessB",
    "Makeup2MetalnessG",
    "Makeup2MetalnessR",
    "Makeup2NumTilesU",
    "Makeup2NumTilesV",
    "Makeup2OffsetU",
    "Makeup2OffsetV",
    "Makeup2Opacity",
    "Makeup2SmoothnessB",
    "Makeup2SmoothnessG",
    "Makeup2SmoothnessR",
    "Makeup3MetalnessB",
    "Makeup3MetalnessG",
    "Makeup3MetalnessR",
    "Makeup3NumTilesU",
    "Makeup3NumTilesV",
    "Makeup3OffsetU",
    "Makeup3OffsetV",
    "Makeup3Opacity",
    "Makeup3SmoothnessB",
    "Makeup3SmoothnessG",
    "Makeup3SmoothnessR",
    "Makeup1OpacityB",
    "Makeup1OpacityG",
    "Makeup1OpacityR",
    "Makeup2OpacityB",
    "Makeup2OpacityG",
    "Makeup2OpacityR",
    "Makeup3OpacityB",
    "Makeup3OpacityG",
    "Makeup3OpacityR",
    "SunSpotsAmount",
    "SunSpotsOpacity",
    "TattooAge",
    "TattooHueRotation",
    "TattooNumTilesU",
    "TattooNumTilesV",
    "TattooOffsetU",
    "TattooOffsetV",
    "beard_itemport",
    "body_itemport",
    "body_m",
    "dna matrix 1.0",
    "eyebrow_itemport",
    "eyelashes_itemport",
    "eyes_itemport",
    "f_limbs_m",
    "f_torso_m",
    "female21",
    "female22",
    "female23",
    "female24",
    "female26",
    "female27",
    "female28",
    "female29",
    "female30",
    "hair_itemport",
    "head",
    "head_itemport",
    "limbs_m",
    "material_variant",
    "piercings_eyebrows_itemport",
    "piercings_l_ear_itemport",
    "piercings_mouth_itemport",
    "piercings_nose_itemport",
    "piercings_r_ear_itemport",
    "protos_human_female_face_t1_pu",
    "protos_human_male_face_t1_pu",
    "shader_Head",
    "shader_eyeInner",
    "shader_eyeinner",
    "shader_head",
    "stubble_itemport",
    "universal_scalp_itemport",
    // Hair submaterial names (cracked from P4k .mtl files)
    "balding_short_hair_02_m",
    "brows_001_m",
    "brows_002_m",
    "brows_003_m",
    "brows_004_m",
    "brows_005_m",
    "brows_006_m",
    "bun_long_hair_01_m",
    "f_hair_25_m",
    "f_hair_27_m",
    "f_hair_27_scalp",
    "f_hair_57_scalp_m",
    "f_hair_31_m",
    "f_hair_57_m",
    "facial_hair_001",
    "facial_hair_002",
    "facial_hair_003",
    "facial_hair_004",
    "facial_hair",
    "facial_hair_005",
    "facial_hair_020",
    "facial_hair_006",
    "facial_hair_007",
    "facial_hair_008",
    "facial_hair_009",
    "facial_hair_010",
    "facial_hair_011",
    "facial_hair_012",
    "facial_hair_0120",
    "facial_hair_013",
    "facial_hair_014",
    "facial_hair_015",
    "facial_hair_016",
    "facial_hair_017",
    "facial_hair_018",
    "facial_hair_019",
    "facial_hair_021",
    "facial_hair_022",
    "facial_hair_023",
    "facial_hair_024",
    "facial_hair_025",
    "facial_hair_026",
    "facial_hair_027",
    "facial_hair_028",
    "facial_hair_029",
    "facial_hair_030",
    "facial_hair_031",
    "facial_hair_032",
    "facial_hair_035",
    "facial_hair_037",
    "facial_hair_038",
    "facial_hair_040",
    "facial_hair_041",
    "facial_hair_042",
    "facial_hair_043",
    "facial_hair_044",
    "facial_hair_045",
    "facial_hair_048",
    "facial_hair_53_m",
    "facial_hair_m",
    "facial_hair_shaved",
    "hair_21_m",
    "hair_28_m",
    "hair_31_m",
    "hair_33_m",
    "hair_34_m",
    "hair_38_m",
    "hair_36_m",
    "hair_42_m",
    "hair_48_m",
    "hair_49_m",
    "hair_75_m",
    "hair_m",
    "hair_scalp_m",
    "hair_scalp_shadow_m",
    "hair_scalp_shaved_m",
    "hair_shaved_m",
    "hair_shaved_scalp_m",
    "hair_volume__scalp_m",
    "m_hair_25_m",
    "mohawk_short_hair_01_m",
    "scalp_hair_m",
    "shaved_hair_m",
    "shaved_short_hair_01_m",
    "short_hair_03_m",
    "straight_long_hair_05_m",
    "straight_long_hair_07_m",
    "straight_short_hair_01_m",
    "straight_short_hair_12_m",
    "straight_short_hair_14_m",
    "straight_short_hair_16_m",
    "straight_short_hair_20_m",
    "straight_short_hair_31_m",
    "wavy_long_hair_02_m",
    "wavy_short_hair_02_m",
];

/// Manually-mapped hashes where the original name is not simply the CRC32C
/// of the display string (or the original source string is unknown).
const MANUAL_MAPPINGS: &[(u32, &str)] = &[
    (0xa98beb34, "Head Material"),
    (0x6c836947, "HairDyeMaterial"),
    (0x078ac8bd, "EyebrowDyeMaterial"),
    (0xa047885e, "EyeMaterial"),
    (0x9b274d93, "BeardDyeMaterial"),
    (0x27424d58, "BodyMaterial"),
    (0xa8770416, "DyeMaterial"),
    (0xbd530797, "BodyColor"),
    (0xb29b1d90, "EyeMakeupColor1"),
    (0xe3230e2f, "EyeMakeupColor2"),
    (0x2ec0e736, "EyeMakeupColor3"),
    (0x1a081a93, "CheekMakeupColor1"),
    (0x4bb0092c, "CheekMakeupColor2"),
    (0x8653e035, "CheekMakeupColor3"),
    (0x7d86e792, "LipMakeupColor1"),
    (0x2c3ef42d, "LipMakeupColor2"),
    (0xe1dd1d34, "LipMakeupColor3"),
    (0x442a34ac, "EyeColor"),
    (0x15e90814, "HairDyeColor1"),
    (0xa2c7c909, "HairDyeColor2"),
    (0x4e865b74, "EyebrowDye"),
    (0x3b73d344, "StubbleDye"),
    (0x8792319b, "BeardDye"),
    (0x2c6279e6, "HairDye"),
    (0x75196d10, "EyebrowDye2"),
    (0x9f37ad63, "EyebrowDye3"),
    (0x1f1fad17, "HairDye2"),
    (0x13170b02, "HairDye3"),
];

static HASH_TO_NAME: LazyLock<HashMap<u32, &'static str>> = LazyLock::new(|| {
    let mut map = HashMap::with_capacity(KNOWN_NAMES.len() + MANUAL_MAPPINGS.len());
    for &name in KNOWN_NAMES {
        map.insert(crc32c::crc32c(name.as_bytes()), name);
    }
    for &(hash, name) in MANUAL_MAPPINGS {
        map.insert(hash, name);
    }
    map
});

/// Reverse lookup: display name -> hash value.
/// Only needed for manual mappings where crc32c(name) != stored hash.
static NAME_TO_HASH: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    let mut map = HashMap::with_capacity(MANUAL_MAPPINGS.len());
    for &(hash, name) in MANUAL_MAPPINGS {
        map.insert(name, hash);
    }
    map
});
