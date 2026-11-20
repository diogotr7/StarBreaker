//! Finishes drawn from their own geometry file.
//!
//! A tagged `SubGeometry` node can name a geometry file of its own instead of
//! only a palette or material: a weapon finish's `.cdf` that binds the base's
//! parts with the finish's material and adds parts of its own (a collector
//! shell, glow cables, a charm).
//!
//! - [`own_geometry_path`]: the node's geometry file, when it is not the
//!   component's own ([`component_geometry_path`]).
//! - [`variant_look`]: what that file changes against the base's parts — the
//!   material it gives the parts both share, and the parts only it binds.
//! - [`resolve_finish_geometry`]: applies that to the paint variants at export.
//! - [`finish_part_payloads`]: the extra parts as child payloads on their bones.

use starbreaker_datacore::query::value::Value;
use starbreaker_p4k::MappedP4k;

use crate::mtl;
use crate::types::EntityPayload as ChildPayload;

use super::datacore_path_to_p4k;
use super::entity_export::{GeometryPart, export_entity_from_paths, resolve_geometry_files};
use super::palette::{get_value_field, get_value_string, try_load_mtl};
use super::textures::load_material_textures;
use super::{ExportOptions, PngCache};

/// `Geometry.Geometry.path` of an `SGeometryDataParams`-holding node.
fn geometry_path_of<'a>(data: &Value<'a>) -> Option<&'a str> {
    get_value_string(get_value_field(data, "Geometry")?, "path")
}

fn same_path(a: &str, b: &str) -> bool {
    a.replace('\\', "/").eq_ignore_ascii_case(&b.replace('\\', "/"))
}

/// The component's own geometry file (`Geometry.Geometry.Geometry.path`), if set.
pub(crate) fn component_geometry_path<'a>(component: &Value<'a>) -> Option<&'a str> {
    get_value_field(component, "Geometry")
        .and_then(|g| get_value_field(g, "Geometry"))
        .and_then(geometry_path_of)
        .filter(|p| !p.is_empty())
}

/// The geometry file a tagged node's data (`SGeometryDataParams`) names, when
/// it is set and differs from the component's own (`Geometry.Geometry`).
pub(crate) fn own_geometry_path(component: &Value, node_data: &Value) -> Option<String> {
    let own = geometry_path_of(node_data).filter(|p| !p.is_empty())?;
    let base = component_geometry_path(component).unwrap_or("");
    (!same_path(own, base)).then(|| own.to_string())
}

/// What a finish's own geometry changes against the base's parts.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct VariantLook {
    /// The material the finish binds to a part the base also binds (the
    /// first such part in the finish's order), when it differs from the base's.
    pub material: Option<String>,
    /// The parts only the finish binds, in its order.
    pub extras: Vec<GeometryPart>,
}

pub(crate) fn variant_look(base: &[GeometryPart], variant: &[GeometryPart]) -> VariantLook {
    let mut look = VariantLook::default();
    for part in variant {
        match base.iter().find(|b| same_path(&b.path, &part.path)) {
            Some(shared) => {
                if look.material.is_none()
                    && part.material_override.is_some()
                    && part.material_override != shared.material_override
                {
                    look.material = part.material_override.clone();
                }
            }
            None => look.extras.push(part.clone()),
        }
    }
    look
}

/// The child entity name a variant's extra part is exported under: unique
/// to the variant, so `paints.json` can name the children it shows.
fn part_entity_name(tag: &str, path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    format!("{tag}:{stem}")
}

/// Fill in each variant drawn from its own geometry file: the material it
/// binds to the base's parts (when the node names none) and the parts only it
/// binds. `base_geometry` is the entity's own geometry file.
pub(crate) fn resolve_finish_geometry(p4k: &MappedP4k, base_geometry: &str, variants: &mut [mtl::PaintVariant]) {
    let Ok(base) = resolve_geometry_files(p4k, base_geometry) else {
        return;
    };
    for variant in variants.iter_mut() {
        let Some(own) = variant.geometry_path.as_deref() else { continue };
        let finish = match resolve_geometry_files(p4k, own) {
            Ok(finish) => finish,
            Err(e) => {
                log::warn!("  paint variant {:?}: geometry '{own}' failed: {e}", variant.subgeometry_tag);
                continue;
            }
        };
        let look = variant_look(&base.parts, &finish.parts);
        if variant.material_path.is_none() {
            if let Some(material) = look.material {
                let path = datacore_path_to_p4k(&material).replace('\\', "/");
                variant.materials = try_load_mtl(p4k, &path.replace('/', "\\"));
                variant.material_path = Some(path);
            }
        }
        variant.extra_parts = look
            .extras
            .into_iter()
            .map(|part| mtl::VariantPart {
                entity_name: part_entity_name(&variant.subgeometry_tag, &part.path),
                geometry_path: part.path,
                material_path: part.material_override,
                bone_name: part.bone_name,
            })
            .collect();
        log::info!(
            "  paint variant {:?}: own geometry '{own}', material={:?}, extra parts={:?}",
            variant.subgeometry_tag,
            variant.material_path,
            variant.extra_parts.iter().map(|p| p.entity_name.as_str()).collect::<Vec<_>>(),
        );
    }
}

/// One child payload per variant extra part, hung on its bone under
/// `parent_entity`. A `CA_SKIN` part (no bone) shares the base skeleton and is
/// not placed by this path; it is logged and left out.
pub(crate) fn finish_part_payloads(
    p4k: &MappedP4k,
    variants: &[mtl::PaintVariant],
    parent_entity: &str,
    palette: Option<&mtl::TintPalette>,
    opts: &ExportOptions,
) -> Vec<ChildPayload> {
    let mut payloads = Vec::new();
    for variant in variants {
        for part in &variant.extra_parts {
            let Some(bone) = part.bone_name.clone() else {
                log::warn!("  paint variant {:?}: skin part '{}' is not placed", variant.subgeometry_tag, part.geometry_path);
                continue;
            };
            let material = part.material_path.as_deref().unwrap_or("");
            let (mesh, materials, _, nmc, _, geometry_path, material_path, bones, skeleton_source_path) =
                match export_entity_from_paths(p4k, &part.geometry_path, material, opts) {
                    Ok(loaded) => loaded,
                    Err(e) => {
                        log::warn!("  paint variant {:?}: part '{}' failed: {e}", variant.subgeometry_tag, part.geometry_path);
                        continue;
                    }
                };
            let textures = if opts.material_mode.include_textures() {
                materials.as_ref().map(|m| {
                    load_material_textures(
                        p4k,
                        m,
                        palette,
                        opts.texture_mip,
                        &mut PngCache::new(),
                        opts.material_mode.include_normals(),
                        opts.material_mode.experimental(),
                    )
                })
            } else {
                None
            };
            payloads.push(ChildPayload {
                mesh,
                materials,
                textures,
                nmc,
                palette: palette.cloned(),
                geometry_path,
                material_path,
                bones,
                skeleton_source_path,
                entity_name: part.entity_name.clone(),
                entity_category: None,
                attach_def_type: None,
                parent_node_name: bone,
                parent_entity_name: parent_entity.to_string(),
                no_rotation: false,
                offset_position: [0.0; 3],
                offset_rotation: [0.0; 3],
                detach_direction: [0.0; 3],
                port_flags: String::new(),
                ui_bindings: Vec::new(),
            });
        }
    }
    payloads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(type_name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        Value::Object { type_name, fields, record_id: None }
    }

    /// An `SGeometryDataParams` naming `path`.
    fn data(path: &'static str) -> Value<'static> {
        object("SGeometryDataParams", vec![("Geometry", object("GlobalResourceGeometry", vec![("path", Value::String(path))]))])
    }

    fn component(path: &'static str) -> Value<'static> {
        object("SGeometryResourceParams", vec![("Geometry", object("SGeometryNodeParams", vec![("Geometry", data(path))]))])
    }

    fn part(path: &str, bone: Option<&str>, material: Option<&str>) -> GeometryPart {
        GeometryPart {
            path: path.to_string(),
            bone_name: bone.map(str::to_string),
            material_override: material.map(str::to_string),
        }
    }

    #[test]
    fn a_node_naming_another_geometry_file_has_its_own_geometry() {
        let c = component("Objects/gun/gun.cdf");
        assert_eq!(own_geometry_path(&c, &data("Objects/gun/cosmetics/gun_gold.cdf")).as_deref(), Some("Objects/gun/cosmetics/gun_gold.cdf"));
        assert_eq!(own_geometry_path(&c, &data("objects\\GUN\\gun.cdf")), None, "the base's own file, in another case and separator");
        assert_eq!(own_geometry_path(&c, &data("")), None, "no geometry of its own");
    }

    #[test]
    fn a_finish_geometry_brings_its_material_and_its_extra_parts() {
        let base = [part("gun/parts.skin", None, None), part("gun/body.cgf", Some("root"), None), part("gun/display.cgf", Some("display"), None)];
        let finish = [
            part("gun/parts.skin", None, Some("gun/cosmetics/gold.mtl")),
            part("gun/body.cgf", Some("root"), Some("gun/cosmetics/gold.mtl")),
            part("gun/display.cgf", Some("display"), None),
            part("gun/cosmetics/shell.cgf", Some("root"), Some("gun/cosmetics/shell.mtl")),
            part("charms/fang.cgf", Some("charm_attach"), Some("charms/fang.mtl")),
        ];
        let look = variant_look(&base, &finish);
        assert_eq!(look.material.as_deref(), Some("gun/cosmetics/gold.mtl"));
        assert_eq!(look.extras, vec![finish[3].clone(), finish[4].clone()]);
    }

    #[test]
    fn a_finish_geometry_keeping_the_base_materials_brings_no_material() {
        let base = [part("gun/parts.skin", None, Some("gun/gun.mtl"))];
        let finish = [part("gun/parts.skin", None, Some("gun/gun.mtl")), part("gun/extra.cgf", Some("root"), None)];
        let look = variant_look(&base, &finish);
        assert_eq!(look.material, None);
        assert_eq!(look.extras.len(), 1);
    }
}
