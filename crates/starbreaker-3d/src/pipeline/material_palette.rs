//! The tint palette a material file names as its default.
//!
//! A root `<Material>` can carry a `DefaultPalette` attribute: the DataCore
//! path of a `TintPaletteTree` record (for example the Javelin's exterior
//! `.mtl`). An entity whose DataCore definition names no palette of its own
//! is drawn with that one.
//!
//! - [`default_palette_file`]: the named record's DataCore file, without the
//!   extension (DataCore stores record files as `.xml`).
//! - [`query_material_default_palette`]: the palette read from that record.

use starbreaker_datacore::database::Database;

use crate::mtl::{MaterialSetAuthoredData, MtlFile, TintPalette};

use super::palette::query_tint_from_record;

/// `path` with forward slashes, lower case, and no extension on its last segment.
fn normalized_record_file(path: &str) -> String {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let name_start = path.rfind('/').map_or(0, |i| i + 1);
    match path[name_start..].rfind('.') {
        Some(dot) => path[..name_start + dot].to_string(),
        None => path,
    }
}

/// The DataCore file (forward slashes, lower case, no extension) of the
/// palette record the material set's `DefaultPalette` attribute names.
pub(crate) fn default_palette_file(material_set: &MaterialSetAuthoredData) -> Option<String> {
    let value = &material_set
        .attributes
        .iter()
        .find(|attribute| attribute.name.eq_ignore_ascii_case("DefaultPalette"))?
        .value;
    (!value.is_empty()).then(|| normalized_record_file(value))
}

/// The palette a material file names as its default, read from DataCore;
/// its `source_name` is the palette record's name.
pub(crate) fn query_material_default_palette(db: &Database, mtl: &MtlFile) -> Option<TintPalette> {
    let file = default_palette_file(&mtl.material_set)?;
    let struct_id = db.struct_id("TintPaletteTree")?;
    let record = db
        .records_of_type(struct_id)
        .find(|r| normalized_record_file(db.resolve_string(r.file_name_offset)) == file)?;
    let name = db.resolve_string2(record.name_offset);
    let name = name.rsplit('.').next().unwrap_or(name).to_ascii_lowercase();
    query_tint_from_record(db, record, Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtl::AuthoredAttribute;

    fn material_set(attributes: &[(&str, &str)]) -> MaterialSetAuthoredData {
        MaterialSetAuthoredData {
            attributes: attributes
                .iter()
                .map(|(name, value)| AuthoredAttribute { name: name.to_string(), value: value.to_string() })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_palette_attribute_names_a_datacore_record_file() {
        let set = material_set(&[
            ("MtlFlags", "256"),
            ("DefaultPalette", r"Libs/Foundry/Records/TintPalettes/Brand/Synthetic\synthetic_default"),
        ]);
        assert_eq!(
            default_palette_file(&set).as_deref(),
            Some("libs/foundry/records/tintpalettes/brand/synthetic/synthetic_default"),
        );
        // DataCore stores the record's file as `.xml`; the extension does not take part.
        assert_eq!(
            normalized_record_file("libs/foundry/records/tintpalettes/brand/synthetic/synthetic_default.xml"),
            default_palette_file(&set).unwrap(),
        );
    }

    #[test]
    fn a_material_without_a_default_palette_names_none() {
        assert_eq!(default_palette_file(&material_set(&[("MtlFlags", "256")])), None);
        assert_eq!(default_palette_file(&material_set(&[("DefaultPalette", "")])), None);
    }
}
