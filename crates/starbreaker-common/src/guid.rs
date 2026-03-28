use std::fmt;
use std::hash::{Hash, Hasher};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

// ─── CigGuid ────────────────────────────────────────────────────────────────

/// A 128-bit GUID used by Star Citizen / CIG.
///
/// The on-disk byte layout is NOT the standard Windows GUID layout.
///
/// # Display byte reordering
///
/// The `Display` implementation writes the bytes in CIG's mixed-endian order:
///
/// ```text
/// b[7]b[6]b[5]b[4]-b[3]b[2]-b[1]b[0]-b[15]b[14]-b[13]b[12]b[11]b[10]b[9]b[8]
/// ```
///
/// This means the first four bytes of the on-disk representation appear as
/// the *last* four hex digits of the first group, and the final eight bytes
/// appear reversed in the last two groups.  `FromStr` applies the inverse
/// mapping so that `parse(display(guid)) == guid` always holds.
#[derive(Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct CigGuid {
    pub bytes: [u8; 16],
}

impl CigGuid {
    pub const EMPTY: CigGuid = CigGuid { bytes: [0u8; 16] };

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        CigGuid { bytes }
    }

    pub fn is_empty(&self) -> bool {
        self.bytes == [0u8; 16]
    }
}

impl PartialEq for CigGuid {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Eq for CigGuid {}

impl Hash for CigGuid {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl fmt::Display for CigGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.bytes;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[7],
            b[6],
            b[5],
            b[4],
            b[3],
            b[2],
            b[1],
            b[0],
            b[15],
            b[14],
            b[13],
            b[12],
            b[11],
            b[10],
            b[9],
            b[8]
        )
    }
}

impl fmt::Debug for CigGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CigGuid({})", self)
    }
}

// ─── GuidParseError + FromStr ────────────────────────────────────────────────

/// Error returned when parsing a GUID string fails.
#[derive(Debug, Clone)]
pub struct GuidParseError;

impl fmt::Display for GuidParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid GUID format (expected xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)")
    }
}

impl std::error::Error for GuidParseError {}

impl std::str::FromStr for CigGuid {
    type Err = GuidParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return Err(GuidParseError);
        }

        let parse_byte = |i: usize| -> Result<u8, GuidParseError> {
            u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| GuidParseError)
        };

        // Display format: b[7]b[6]b[5]b[4]-b[3]b[2]-b[1]b[0]-b[15]b[14]-b[13]b[12]b[11]b[10]b[9]b[8]
        let mut bytes = [0u8; 16];
        bytes[7] = parse_byte(0)?;
        bytes[6] = parse_byte(1)?;
        bytes[5] = parse_byte(2)?;
        bytes[4] = parse_byte(3)?;
        bytes[3] = parse_byte(4)?;
        bytes[2] = parse_byte(5)?;
        bytes[1] = parse_byte(6)?;
        bytes[0] = parse_byte(7)?;
        bytes[15] = parse_byte(8)?;
        bytes[14] = parse_byte(9)?;
        bytes[13] = parse_byte(10)?;
        bytes[12] = parse_byte(11)?;
        bytes[11] = parse_byte(12)?;
        bytes[10] = parse_byte(13)?;
        bytes[9] = parse_byte(14)?;
        bytes[8] = parse_byte(15)?;

        Ok(CigGuid { bytes })
    }
}

// ─── Serde ──────────────────────────────────────────────────────────────────

impl serde::Serialize for CigGuid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for CigGuid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ─── Size assertion ─────────────────────────────────────────────────────────

const _: () = assert!(size_of::<CigGuid>() == 16);
