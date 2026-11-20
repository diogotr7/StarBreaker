//! Root geometry/material selection from an entity's `SubGeometry[]`
//! variants: by an explicit geometry tag (`select_paths`), or, untagged,
//! the one geometry an empty-root entity's variants share (`select_untagged`).
//! `resolve` is the entry point used by entity and loadout export.

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

/// Root paths when no geometry tag was requested. An entity whose root
/// `Geometry.path` is empty carries its mesh only in `SubGeometry[]`; when
/// every variant names the same file (they differ only by material) that
/// file is the entity's geometry. Distinct variant geometries need a tag.
fn select_untagged(
    variants: &[SubGeometryVariant],
    geometry: String,
    material: String,
) -> Result<(String, String), Error> {
    if !geometry.is_empty() {
        return Ok((geometry, material));
    }
    let Some(first) = variants.first() else {
        return Err(Error::Other(
            "Root geometry path is empty and the entity has no SubGeometry variants".into(),
        ));
    };
    if variants
        .iter()
        .any(|v| !v.geometry_path.eq_ignore_ascii_case(&first.geometry_path))
    {
        let tags: Vec<&str> = variants.iter().map(|v| v.tag.as_str()).collect();
        return Err(Error::Other(format!(
            "Root geometry path is empty and SubGeometry variants name different geometry; \
             pick one with a geometry tag ({})",
            tags.join(", ")
        )));
    }
    Ok((first.geometry_path.clone(), material))
}

pub(super) fn resolve(
    db: &Database,
    record: &Record,
    tag: Option<&str>,
    geometry: String,
    material: String,
) -> Result<(String, String), Error> {
    match tag {
        Some(tag) => {
            let variants = EntityIndex::new(db).query_sub_geometry(record);
            select_paths(&variants, tag, material)
        }
        None if geometry.is_empty() => {
            let variants = EntityIndex::new(db).query_sub_geometry(record);
            select_untagged(&variants, geometry, material)
        }
        None => Ok((geometry, material)),
    }
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
                deformer_type: String::new(),
            },
            SubGeometryVariant {
                tag: "male".into(),
                geometry_path: "body.skin".into(),
                material_path: String::new(),
                deformer_type: String::new(),
            },
            SubGeometryVariant {
                tag: "female".into(),
                geometry_path: "body2.skin".into(),
                material_path: "variant.mtl".into(),
                deformer_type: String::new(),
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

    fn variant(tag: &str, geometry: &str, material: &str) -> SubGeometryVariant {
        SubGeometryVariant {
            tag: tag.into(),
            geometry_path: geometry.into(),
            material_path: material.into(),
            deformer_type: String::new(),
        }
    }

    #[test]
    fn untagged_root_keeps_authored_geometry() {
        let variants = vec![variant("a", "other.cdf", "")];
        assert_eq!(
            select_untagged(&variants, "root.cdf".into(), "root.mtl".into()).unwrap(),
            ("root.cdf".into(), "root.mtl".into())
        );
    }

    #[test]
    fn empty_root_takes_the_one_geometry_its_material_only_variants_share() {
        // Variants that differ only by material leave the geometry unambiguous;
        // the root's own (empty) material is kept, as no variant was selected.
        let variants = vec![
            variant("blue", "Objects/gun.cdf", ""),
            variant("mr01", "objects/GUN.cdf", "gun_mr.mtl"),
        ];
        assert_eq!(
            select_untagged(&variants, String::new(), String::new()).unwrap(),
            ("Objects/gun.cdf".into(), String::new())
        );
    }

    #[test]
    fn empty_root_with_distinct_variant_geometries_needs_a_tag() {
        let variants = vec![variant("left", "l.cga", ""), variant("right", "r.cga", "")];
        let err = select_untagged(&variants, String::new(), String::new()).unwrap_err();
        assert!(err.to_string().contains("geometry tag"), "{err}");
        assert!(select_untagged(&[], String::new(), String::new()).is_err());
    }
}
