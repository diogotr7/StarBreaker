use std::fmt;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// A 4-byte packed RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct ColorRgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorRgba {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        ColorRgba { r, g, b, a }
    }
}

/// Display as `#rrggbbaa`.
impl fmt::Display for ColorRgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:02x}{:02x}{:02x}{:02x}",
            self.r, self.g, self.b, self.a
        )
    }
}

/// Error returned when parsing a color string fails.
#[derive(Debug, Clone)]
pub struct ColorParseError(String);

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid color: {}", self.0)
    }
}

impl std::error::Error for ColorParseError {}

impl std::str::FromStr for ColorRgba {
    type Err = ColorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix('#')
            .ok_or_else(|| ColorParseError("missing '#' prefix".to_string()))?;

        if hex.len() != 8 {
            return Err(ColorParseError(format!(
                "expected 8 hex digits after '#', got {}",
                hex.len()
            )));
        }

        let parse_byte = |i: usize| -> Result<u8, ColorParseError> {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| ColorParseError(format!("invalid hex at position {i}")))
        };

        Ok(ColorRgba {
            r: parse_byte(0)?,
            g: parse_byte(2)?,
            b: parse_byte(4)?,
            a: parse_byte(6)?,
        })
    }
}

// ─── Serde ──────────────────────────────────────────────────────────────────

impl serde::Serialize for ColorRgba {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for ColorRgba {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ─── Size assertion ─────────────────────────────────────────────────────────

const _: () = assert!(size_of::<ColorRgba>() == 4);
