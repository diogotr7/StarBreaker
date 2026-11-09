//! Bind authored node-level Glow effects only to glow-capable material sections.
//! Preserve effect identity and ordered modifier chains; derive control bounds in
//! material-property units rather than inventing an emission-strength range.
use crate::{
    error::Error,
    mtl::{MtlFile, ShaderFamily, SubMaterial},
    nmc::NodeMeshCombo,
    types::Mesh,
};
use serde::Serialize;
use starbreaker_datacore::{Database, query::value::Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum GlowModifier {
    Multiplier { multiplier: f32 },
    MinMax { minimum: f32, maximum: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct GlowPropertyLink {
    pub effect_kind: String,
    pub effect_index: usize,
    pub property_link_index: usize,
    pub submesh_name: String,
    pub parameter_record_id: Option<String>,
    pub tag_record_id: Option<String>,
    pub modifiers: Vec<GlowModifier>,
    /// Product of authored multipliers, null when no multiplier is authored.
    pub multiplier: Option<f32>,
    /// Output bounds after the ordered chain; null means unbounded/unauthored.
    pub minimum: Option<f32>,
    pub maximum: Option<f32>,
}

pub(crate) type GlowMaterialBindings = HashMap<u32, Vec<GlowPropertyLink>>;

/// The flag identifies glow cards even when their static Emissive/Glow is zero.
/// Emittance permutations are also explicit emission sources. Names never qualify.
fn glow_capable(material: &SubMaterial) -> bool {
    (matches!(
        material.shader_family(),
        ShaderFamily::Illum | ShaderFamily::HardSurface
    ) && material.is_forward_rendered())
        || material.has_emittance_map()
}

pub(crate) fn glow_effect_materials(
    db: &Database,
    entity: &str,
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
    materials: Option<&MtlFile>,
) -> Result<GlowMaterialBindings, Error> {
    let Some(record) = db
        .records_by_type_name("EntityClassDefinition")
        .find(|record| {
            let name = db.resolve_string2(record.name_offset);
            name.strip_prefix("EntityClassDefinition.")
                .unwrap_or(name)
                .eq_ignore_ascii_case(
                    entity
                        .strip_prefix("EntityClassDefinition.")
                        .unwrap_or(entity),
                )
        })
    else {
        return Ok(HashMap::new());
    };
    let path = db.compile_path::<Value>(
        record.struct_id(),
        "Components[SEntityComponentEffects].entityEffects.materialEffects",
    )?;
    let mut links = Vec::new();
    for value in db.query::<Value>(&path, record)? {
        links.extend(glow_links(&value)?);
    }
    Ok(bound_materials(mesh, nmc, materials, &links))
}

fn field<'a, 'b>(value: &'b Value<'a>, name: &str) -> Option<&'b Value<'a>> {
    let Value::Object { fields, .. } = value else {
        return None;
    };
    fields
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

fn record_id(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Object {
            record_id: Some(id),
            ..
        })
        | Some(Value::Guid(id)) => Some(id.to_string()),
        _ => None,
    }
}

fn number(value: &Value, name: &str) -> Result<f32, Error> {
    match field(value, name) {
        Some(Value::Float(n)) if n.is_finite() => Ok(*n),
        _ => Err(Error::Other(format!(
            "Glow modifier lacks finite authored {name}: {value:?}"
        ))),
    }
}

fn modifiers(link: &Value) -> Result<Vec<GlowModifier>, Error> {
    let Some(Value::Array(modifiers)) = field(link, "modifiers") else {
        return Err(Error::Other(
            "Glow property link lacks its modifier array".into(),
        ));
    };
    modifiers
        .iter()
        .map(|modifier| match modifier {
            Value::Object {
                type_name: "SEntityEffectSystem_MultiplierModifier",
                ..
            } => Ok(GlowModifier::Multiplier {
                multiplier: number(modifier, "multiplier")?,
            }),
            Value::Object {
                type_name: "SEntityEffectSystem_MinMaxModifier",
                ..
            } => {
                let minimum = number(modifier, "minimum")?;
                let maximum = number(modifier, "maximum")?;
                if minimum > maximum {
                    return Err(Error::Other("Glow modifier minimum exceeds maximum".into()));
                }
                Ok(GlowModifier::MinMax { minimum, maximum })
            }
            _ => Err(Error::Other(format!(
                "unsupported Glow modifier: {modifier:?}"
            ))),
        })
        .collect()
}

fn summarize(modifiers: &[GlowModifier]) -> (Option<f32>, Option<f32>, Option<f32>) {
    let (mut multiplier, mut minimum, mut maximum) = (None, None, None);
    for modifier in modifiers {
        match *modifier {
            GlowModifier::Multiplier { multiplier: factor } => {
                multiplier = Some(multiplier.unwrap_or(1.0) * factor);
                if factor == 0.0 {
                    minimum = Some(0.0);
                    maximum = Some(0.0);
                } else {
                    minimum = minimum.map(|v| v * factor);
                    maximum = maximum.map(|v| v * factor);
                    if factor < 0.0 {
                        std::mem::swap(&mut minimum, &mut maximum);
                    }
                }
            }
            GlowModifier::MinMax {
                minimum: lo,
                maximum: hi,
            } => {
                minimum = Some(minimum.map_or(lo, |v: f32| v.clamp(lo, hi)));
                maximum = Some(maximum.map_or(hi, |v: f32| v.clamp(lo, hi)));
            }
        }
    }
    (multiplier, minimum, maximum)
}

fn glow_links(category: &Value) -> Result<Vec<GlowPropertyLink>, Error> {
    if field(category, "enabled") != Some(&Value::Bool(true)) {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for kind in ["tagEffects", "triggerEffects"] {
        let Some(Value::Array(effects)) = field(category, kind) else {
            continue;
        };
        for (effect_index, effect) in effects.iter().enumerate() {
            if field(effect, "enabled") != Some(&Value::Bool(true)) {
                continue;
            }
            let Some(Value::Array(links)) = field(effect, "propertyLinks") else {
                continue;
            };
            for (property_link_index, link) in links.iter().enumerate() {
                if !matches!(
                    field(link, "materialProperty"),
                    Some(Value::String("Glow") | Value::Enum("Glow"))
                ) {
                    continue;
                }
                let Some(Value::String(name)) = field(link, "subMeshName") else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let modifiers = modifiers(link)?;
                let (multiplier, minimum, maximum) = summarize(&modifiers);
                result.push(GlowPropertyLink {
                    effect_kind: kind.into(),
                    effect_index,
                    property_link_index,
                    submesh_name: (*name).into(),
                    parameter_record_id: record_id(field(link, "parameterName")),
                    tag_record_id: record_id(field(effect, "tag")),
                    modifiers,
                    multiplier,
                    minimum,
                    maximum,
                });
            }
        }
    }
    Ok(result)
}

fn bound_materials(
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
    materials: Option<&MtlFile>,
    links: &[GlowPropertyLink],
) -> GlowMaterialBindings {
    let mut result: GlowMaterialBindings = HashMap::new();
    for submesh in &mesh.submeshes {
        let source_id = submesh.source_material_id.unwrap_or(submesh.material_id);
        if !materials
            .and_then(|m| m.materials.get(source_id as usize))
            .is_some_and(glow_capable)
        {
            continue;
        }
        let Some(node) = nmc.and_then(|n| n.nodes.get(submesh.node_parent_index as usize)) else {
            continue;
        };
        for link in links
            .iter()
            .filter(|link| node.name.eq_ignore_ascii_case(&link.submesh_name))
        {
            let bindings = result.entry(source_id).or_default();
            if !bindings.contains(link) {
                bindings.push(link.clone());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object(
        type_name: &'static str,
        fields: Vec<(&'static str, Value<'static>)>,
    ) -> Value<'static> {
        Value::Object {
            type_name,
            fields,
            record_id: None,
        }
    }
    fn effect(
        enabled: bool,
        property: &'static str,
        factor: f32,
        lo: f32,
        hi: f32,
    ) -> Value<'static> {
        object(
            "Synthetic",
            vec![
                ("enabled", Value::Bool(enabled)),
                (
                    "propertyLinks",
                    Value::Array(vec![object(
                        "Synthetic",
                        vec![
                            ("materialProperty", Value::Enum(property)),
                            ("subMeshName", Value::String("PART")),
                            (
                                "modifiers",
                                Value::Array(vec![
                                    object(
                                        "SEntityEffectSystem_MultiplierModifier",
                                        vec![("multiplier", Value::Float(factor))],
                                    ),
                                    object(
                                        "SEntityEffectSystem_MinMaxModifier",
                                        vec![
                                            ("minimum", Value::Float(lo)),
                                            ("maximum", Value::Float(hi)),
                                        ],
                                    ),
                                ]),
                            ),
                        ],
                    )]),
                ),
            ],
        )
    }
    fn links() -> Vec<GlowPropertyLink> {
        // Synthetic values intentionally unrelated to any game's thruster settings.
        glow_links(&object(
            "Synthetic",
            vec![
                ("enabled", Value::Bool(true)),
                (
                    "tagEffects",
                    Value::Array(vec![
                        effect(true, "Glow", 3.0, 2.0, 17.0),
                        effect(true, "Glow", 7.0, 4.0, 29.0),
                        effect(false, "Glow", 9.0, 1.0, 2.0),
                        effect(true, "Opacity", 9.0, 1.0, 2.0),
                    ]),
                ),
            ],
        ))
        .unwrap()
    }
    #[test]
    fn node_glow_selects_forward_card_not_deferred_materials() {
        let bytes = crate::test_xml::encode(
            r#"<Material><SubMaterials>
            <Material Shader="Illum" MtlFlags="32768"/>
            <Material Shader="Illum" Emissive="0,0,0"/>
            <Material Shader="HardSurface" Emissive="1,1,1"/>
            <Material Shader="MeshDecal"/>
            <Material Shader="Illum" StringGenMask="%DECAL%DECAL_OPACITY_MAP"/>
            <Material Shader="Illum" MtlFlags="0x8000" Emissive="0,0,0"/>
        </SubMaterials></Material>"#,
        );
        let materials = crate::mtl::parse_mtl(&bytes).unwrap();
        let node = |name: &str| crate::nmc::NmcNode {
            name: name.into(),
            parent_index: None,
            world_to_bone: [[0.0; 4]; 3],
            bone_to_world: [[0.0; 4]; 3],
            scale: [1.0; 3],
            geometry_type: 0,
            properties: Default::default(),
        };
        let rig = NodeMeshCombo {
            nodes: vec![node("part"), node("part_other")],
            material_indices: vec![],
        };
        let submesh = |node_parent_index, source_material_id| crate::types::SubMesh {
            material_name: None,
            material_id: 0,
            source_material_id: Some(source_material_id),
            first_index: 0,
            num_indices: 0,
            first_vertex: 0,
            num_vertices: 0,
            node_parent_index,
        };
        let mesh = Mesh {
            positions: vec![],
            indices: vec![],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![
                submesh(1, 0),
                submesh(0, 1),
                submesh(0, 2),
                submesh(0, 3),
                submesh(0, 4),
                submesh(0, 5),
                submesh(0, 5),
            ],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: vec![],
        };
        let links = links();
        let bound = bound_materials(&mesh, Some(&rig), Some(&materials), &links);
        assert_eq!(bound.keys().copied().collect::<Vec<_>>(), vec![5]);
        assert_eq!(bound[&5], links); // Both effects survive; repeated primitives do not duplicate them.
        assert!(bound_materials(&mesh, None, Some(&materials), &links).is_empty());
        assert!(bound_materials(&mesh, Some(&rig), None, &links).is_empty());
    }
    #[test]
    fn enabled_links_retain_both_modifier_chains_and_bounds() {
        let links = links();
        assert_eq!(links.len(), 2);
        assert_eq!(
            (links[0].multiplier, links[0].minimum, links[0].maximum),
            (Some(3.0), Some(2.0), Some(17.0))
        );
        assert_eq!(
            (links[1].multiplier, links[1].minimum, links[1].maximum),
            (Some(7.0), Some(4.0), Some(29.0))
        );
        assert_eq!((links[0].effect_index, links[1].effect_index), (0, 1));
        assert!(
            glow_links(&object("Synthetic", vec![("enabled", Value::Bool(false))]))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn modifier_order_and_absent_bounds_are_preserved() {
        assert_eq!(summarize(&[]), (None, None, None));
        assert_eq!(
            summarize(&[GlowModifier::Multiplier { multiplier: 3.0 }]),
            (Some(3.0), None, None)
        );
        assert_eq!(
            summarize(&[
                GlowModifier::MinMax {
                    minimum: 2.0,
                    maximum: 17.0
                },
                GlowModifier::Multiplier { multiplier: -3.0 }
            ]),
            (Some(-3.0), Some(-51.0), Some(-6.0))
        );
    }
    #[test]
    fn emittance_token_enables_deferred_source_but_glow_does_not_change_card_class() {
        for (shader, flags, tokens, expected) in [
            ("HardSurface", "0", "%MACRO_EMISSIVE_MAP", true),
            ("MeshDecal", "0", "%EMITTANCE_MAP", true),
            ("Illum", "0", "", false),
            ("LayerBlend_V2", "32768", "", false),
        ] {
            let bytes = crate::test_xml::encode(&format!(
                r#"<Material Shader="{shader}" MtlFlags="{flags}" StringGenMask="{tokens}" Glow="0.125"/>"#
            ));
            let material = crate::mtl::parse_mtl(&bytes).unwrap();
            assert_eq!(glow_capable(&material.materials[0]), expected);
        }
    }
    #[test]
    fn unknown_or_malformed_modifiers_fail_instead_of_inventing_a_range() {
        let link = object(
            "Synthetic",
            vec![("modifiers", Value::Array(vec![object("Unknown", vec![])]))],
        );
        assert!(modifiers(&link).is_err());
        assert!(
            number(
                &object("Synthetic", vec![("minimum", Value::Float(f32::NAN))]),
                "minimum"
            )
            .is_err()
        );
    }
}
