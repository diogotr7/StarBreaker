use crate::{nmc::NodeMeshCombo, types::Mesh};
use starbreaker_datacore::{Database, query::value::Value};
use std::collections::HashSet;

/// Resolve authored material-property effects to their exact NMC submesh and
/// source material IDs. This is a runtime control binding, not static emission.
pub(crate) fn glow_effect_materials(
    db: &Database,
    entity: &str,
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
) -> HashSet<u32> {
    let Some(record) = db
        .records_by_type_name("EntityClassDefinition")
        .find(|record| {
            let name = db.resolve_string2(record.name_offset);
            name.strip_prefix("EntityClassDefinition.")
                .unwrap_or(name)
                .eq_ignore_ascii_case(entity)
        })
    else {
        return HashSet::new();
    };
    let Ok(path) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[SEntityComponentEffects].entityEffects.materialEffects",
    ) else {
        return HashSet::new();
    };
    let Ok(values) = db.query::<Value>(&path, record) else {
        return HashSet::new();
    };
    let names: HashSet<_> = values.iter().flat_map(glow_submeshes).collect();
    bound_materials(mesh, nmc, &names)
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

fn glow_submeshes<'a>(category: &Value<'a>) -> Vec<&'a str> {
    if field(category, "enabled") != Some(&Value::Bool(true)) {
        return Vec::new();
    }
    let mut names = Vec::new();
    for kind in ["tagEffects", "triggerEffects"] {
        let Some(Value::Array(effects)) = field(category, kind) else {
            continue;
        };
        for effect in effects {
            if field(effect, "enabled") != Some(&Value::Bool(true)) {
                continue;
            }
            let Some(Value::Array(links)) = field(effect, "propertyLinks") else {
                continue;
            };
            for link in links {
                if !matches!(
                    field(link, "materialProperty"),
                    Some(Value::String("Glow") | Value::Enum("Glow"))
                ) {
                    continue;
                }
                if let Some(Value::String(name)) = field(link, "subMeshName") {
                    if !name.is_empty() {
                        names.push(*name);
                    }
                }
            }
        }
    }
    names
}

fn bound_materials(
    mesh: &Mesh,
    nmc: Option<&NodeMeshCombo>,
    names: &HashSet<&str>,
) -> HashSet<u32> {
    mesh.submeshes
        .iter()
        .filter(|submesh| {
            nmc.and_then(|nmc| nmc.nodes.get(submesh.node_parent_index as usize))
                .is_some_and(|node| {
                    names
                        .iter()
                        .any(|name| node.name.eq_ignore_ascii_case(name))
                })
        })
        .map(|submesh| submesh.source_material_id.unwrap_or(submesh.material_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object(fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        Value::Object {
            type_name: "Synthetic",
            fields,
            record_id: None,
        }
    }
    #[test]
    fn only_enabled_glow_property_links_select_authored_submeshes() {
        let effect = |enabled, property, name| {
            object(vec![
                ("enabled", Value::Bool(enabled)),
                (
                    "propertyLinks",
                    Value::Array(vec![object(vec![
                        ("materialProperty", Value::String(property)),
                        ("subMeshName", Value::String(name)),
                    ])]),
                ),
            ])
        };
        let category = object(vec![
            ("enabled", Value::Bool(true)),
            (
                "tagEffects",
                Value::Array(vec![
                    effect(true, "Glow", "arbitrary"),
                    effect(false, "Glow", "disabled"),
                    effect(true, "Opacity", "other"),
                ]),
            ),
        ]);
        assert_eq!(glow_submeshes(&category), vec!["arbitrary"]);
        assert!(glow_submeshes(&object(vec![("enabled", Value::Bool(false))])).is_empty());
    }
    #[test]
    fn submesh_binding_is_exact_and_retains_source_material_ids() {
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
            nodes: vec![node("part"), node("other_part")],
            material_indices: vec![],
        };
        let submesh = |node_parent_index, material_id, source_material_id| crate::types::SubMesh {
            material_name: None,
            material_id,
            source_material_id,
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
                submesh(0, 0, Some(7)),
                submesh(0, 2, None),
                submesh(1, 3, Some(9)),
            ],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: vec![],
        };
        assert_eq!(
            bound_materials(&mesh, Some(&rig), &HashSet::from(["PART"])),
            HashSet::from([7, 2])
        );
        assert!(bound_materials(&mesh, None, &HashSet::from(["part"])).is_empty());
        assert!(bound_materials(&mesh, Some(&rig), &HashSet::from(["missing"])).is_empty());
    }
}
