use crate::error::Error;
use starbreaker_datacore::loadout::{EntityIndex, SubGeometryVariant};
use starbreaker_datacore::{database::Database, types::Record};

fn select_paths(
    variants: &[SubGeometryVariant],
    tag: &str,
    material: String,
) -> Result<(String, String), Error> {
    let variant = variants
        .iter()
        .find(|variant| variant.tag.eq_ignore_ascii_case(tag))
        .ok_or_else(|| Error::Other(format!("No root geometry variant tagged '{tag}'")))?;
    Ok((
        variant.geometry_path.clone(),
        if variant.material_path.is_empty() {
            material
        } else {
            variant.material_path.clone()
        },
    ))
}

pub(super) fn resolve(
    db: &Database,
    record: &Record,
    tag: Option<&str>,
    geometry: String,
    material: String,
) -> Result<(String, String), Error> {
    let Some(tag) = tag else {
        return Ok((geometry, material));
    };
    let variants = EntityIndex::new(db).query_sub_geometry(record);
    select_paths(&variants, tag, material)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_selection_preserves_inherited_material_and_ignores_nested_view_tags() {
        let variants = vec![
            SubGeometryVariant {
                tag: "male fp".into(),
                geometry_path: "view.skin".into(),
                material_path: "view.mtl".into(),
            },
            SubGeometryVariant {
                tag: "male".into(),
                geometry_path: "body.skin".into(),
                material_path: String::new(),
            },
            SubGeometryVariant {
                tag: "female".into(),
                geometry_path: "body2.skin".into(),
                material_path: "variant.mtl".into(),
            },
        ];
        assert_eq!(
            select_paths(&variants, "Male", "base.mtl".into()).unwrap(),
            ("body.skin".into(), "base.mtl".into())
        );
        assert_eq!(
            select_paths(&variants, "Female", "base.mtl".into()).unwrap(),
            ("body2.skin".into(), "variant.mtl".into())
        );
        assert!(select_paths(&variants, "absent", "base.mtl".into()).is_err());
    }
}
