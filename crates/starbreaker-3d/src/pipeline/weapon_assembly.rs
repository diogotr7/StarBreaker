//! FPS weapon assembly classification, anchor matching, and scene metadata.
//!
//! This module keeps weapon-specific DataCore/CDF/NMC assembly rules out of the
//! ship and vehicle loadout resolver. It owns FPS weapon classification,
//! attachment-anchor alias matching, and the weapon assembly manifest emitted
//! alongside decomposed `scene.json` packages.

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;

use crate::nmc::NodeMeshCombo;
use crate::skeleton::Bone;
use crate::types::{EntityPayload, Mesh};

use super::{compute_nmc_world_transforms, mat4_to_array, resolve_geometry_files};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntityAssemblyKind {
    Generic,
    FpsWeapon,
}

#[derive(Debug, Clone)]
pub(crate) struct WeaponAssemblyExport {
    pub(crate) manifest: serde_json::Value,
    pub(crate) diagnostics: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WeaponAnchorResolution {
    pub(crate) requested_anchor: String,
    pub(crate) resolved_anchor: String,
    pub(crate) strategy: String,
    pub(crate) warning: Option<String>,
}

pub(crate) fn classify_entity_kind_from_evidence(
    record_name: &str,
    category: Option<&str>,
    attach_def_type: Option<&str>,
    geometry_path: Option<&str>,
) -> EntityAssemblyKind {
    let evidence = [Some(record_name), category, attach_def_type, geometry_path];
    let has_fps_weapon = evidence.into_iter().flatten().any(is_fps_weapon_evidence);
    if has_fps_weapon {
        EntityAssemblyKind::FpsWeapon
    } else {
        EntityAssemblyKind::Generic
    }
}

pub(crate) fn classify_entity_kind(db: &Database, record: &Record) -> EntityAssemblyKind {
    let record_name = db.resolve_string2(record.name_offset);
    let category = query_string_path(db, record, "Category");
    let attach_def_type = query_string_path(
        db,
        record,
        "Components[SAttachableComponentParams].AttachDef.Type",
    );
    let geometry_path = query_string_path(
        db,
        record,
        "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    );

    classify_entity_kind_from_evidence(
        record_name,
        category.as_deref(),
        attach_def_type.as_deref(),
        geometry_path.as_deref(),
    )
}

pub(crate) fn resolve_weapon_anchor(
    requested_anchor: &str,
    available_anchors: &[String],
    root_anchor: &str,
) -> WeaponAnchorResolution {
    if let Some(anchor) = available_anchors
        .iter()
        .find(|anchor| anchor.eq_ignore_ascii_case(requested_anchor))
    {
        return WeaponAnchorResolution {
            requested_anchor: requested_anchor.to_string(),
            resolved_anchor: anchor.clone(),
            strategy: "exact".to_string(),
            warning: None,
        };
    }

    for alias in weapon_anchor_aliases(requested_anchor) {
        if let Some(anchor) = available_anchors
            .iter()
            .find(|anchor| anchor.eq_ignore_ascii_case(alias))
        {
            return WeaponAnchorResolution {
                requested_anchor: requested_anchor.to_string(),
                resolved_anchor: anchor.clone(),
                strategy: "alias".to_string(),
                warning: None,
            };
        }
    }

    let requested_normalized = normalize_anchor_name(requested_anchor);
    if let Some(anchor) = available_anchors
        .iter()
        .find(|anchor| normalize_anchor_name(anchor) == requested_normalized)
    {
        return WeaponAnchorResolution {
            requested_anchor: requested_anchor.to_string(),
            resolved_anchor: anchor.clone(),
            strategy: "normalized".to_string(),
            warning: None,
        };
    }

    for alias in weapon_anchor_aliases(requested_anchor) {
        let alias_normalized = normalize_anchor_name(alias);
        if let Some(anchor) = available_anchors
            .iter()
            .find(|anchor| normalize_anchor_name(anchor) == alias_normalized)
        {
            return WeaponAnchorResolution {
                requested_anchor: requested_anchor.to_string(),
                resolved_anchor: anchor.clone(),
                strategy: "alias".to_string(),
                warning: None,
            };
        }
    }

    WeaponAnchorResolution {
        requested_anchor: requested_anchor.to_string(),
        resolved_anchor: root_anchor.to_string(),
        strategy: "root_fallback".to_string(),
        warning: Some(format!(
            "weapon anchor '{requested_anchor}' was not resolved; attached to root"
        )),
    }
}

pub(crate) fn resolve_weapon_assembly(
    p4k: &starbreaker_p4k::MappedP4k,
    root_entity_name: &str,
    root_assembly_geometry_path: &str,
    root_geometry_path: &str,
    root_material_path: &str,
    root_mesh: &Mesh,
    root_nmc: Option<&NodeMeshCombo>,
    root_bones: &[Bone],
    root_skeleton_source_path: Option<&str>,
    children: &mut [EntityPayload],
) -> WeaponAssemblyExport {
    let anchors = collect_weapon_anchors(root_entity_name, root_nmc, root_bones);
    let available_anchor_names = anchors
        .iter()
        .filter_map(|anchor| anchor.get("name").and_then(|name| name.as_str()))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let root_anchor = available_anchor_names
        .iter()
        .find(|name| name.eq_ignore_ascii_case("root"))
        .or_else(|| available_anchor_names.first())
        .map(String::as_str)
        .unwrap_or(root_entity_name);

    let root_parts =
        collect_root_weapon_parts(root_entity_name, root_geometry_path, root_mesh, root_nmc);
    let cdf_parts = collect_cdf_weapon_parts(p4k, root_assembly_geometry_path);
    let mut parts = root_parts.clone();
    let mut loadout_parts = Vec::new();
    let mut unresolved_slots = Vec::new();

    for child in children {
        let requested_anchor = child.parent_node_name.clone();
        let resolution =
            resolve_weapon_anchor(&requested_anchor, &available_anchor_names, root_anchor);
        child.parent_node_name = resolution.resolved_anchor.clone();

        let role =
            infer_weapon_part_role(&requested_anchor, &child.entity_name, &child.geometry_path);
        if let Some(warning) = resolution.warning.as_ref() {
            unresolved_slots.push(serde_json::json!({
                "slot": role,
                "requested_anchor": resolution.requested_anchor,
                "resolved_anchor": resolution.resolved_anchor,
                "strategy": resolution.strategy,
                "reason": warning,
            }));
        }

        let part = serde_json::json!({
            "role": role,
            "entity_name": child.entity_name,
            "geometry_path": child.geometry_path,
            "material_path": if child.material_path.is_empty() { None } else { Some(child.material_path.clone()) },
            "source": "datacore_default_loadout",
            "attach_slot": requested_anchor,
            "attach_bone": resolution.resolved_anchor,
            "transform_source": resolution.strategy,
            "material_mode": "preserve_export_options",
        });
        loadout_parts.push(part.clone());
        parts.push(part);
    }

    let manifest = serde_json::json!({
        "root": {
            "entity_name": root_entity_name,
            "geometry_path": root_assembly_geometry_path,
            "primary_geometry_path": root_geometry_path,
            "material_path": root_material_path,
            "skeleton_source_path": root_skeleton_source_path,
        },
        "anchors": anchors,
        "parts": parts,
        "root_parts": root_parts,
        "loadout_parts": loadout_parts,
        "cdf_parts": cdf_parts,
        "unresolved_slots": unresolved_slots,
        "skins": [],
        "diagnostics": {
            "anchor_count": available_anchor_names.len(),
            "root_part_count": root_parts.len(),
            "loadout_part_count": loadout_parts.len(),
            "cdf_part_count": cdf_parts.len(),
        },
    });

    let diagnostics = serde_json::json!({
        "version": 1,
        "assembly_kind": "fps_weapon",
        "root_entity_name": root_entity_name,
        "root_geometry_path": root_assembly_geometry_path,
        "primary_geometry_path": root_geometry_path,
        "anchor_names": available_anchor_names,
        "root_parts": root_parts,
        "cdf_parts": cdf_parts,
        "loadout_parts": loadout_parts,
        "unresolved_slots": unresolved_slots,
    });

    WeaponAssemblyExport {
        manifest,
        diagnostics,
    }
}

fn query_string_path(db: &Database, record: &Record, path: &str) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|value| !value.is_empty())
}

fn is_fps_weapon_evidence(value: &str) -> bool {
    let lower = value.replace('\\', "/").to_ascii_lowercase();
    lower.contains("fps_weapons")
        || lower.contains("fpsweapon")
        || lower.contains("fps_weapon")
        || (lower.contains("fps") && lower.contains("weapon"))
        || lower.contains("weaponpersonal")
}

fn normalize_anchor_name(value: &str) -> String {
    let compact = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>();
    compact
        .replace("hardpoint", "")
        .replace("attachment", "")
        .replace("attach", "")
        .replace("itemport", "")
}

fn weapon_anchor_aliases(requested_anchor: &str) -> &'static [&'static str] {
    let normalized = normalize_anchor_name(requested_anchor);
    if normalized.contains("magazine") || normalized == "mag" {
        &["magAttach", "mag_attach", "magazine_attachment", "magazine"]
    } else if normalized.contains("ammo") || normalized.contains("bullet") {
        &["bullet", "ammo", "ammo_attach", "ammo_attachment"]
    } else if normalized.contains("parts") {
        &["parts", "part", "weapon_parts"]
    } else if normalized.contains("barrel") {
        &["barrel_attachment", "barrelAttach", "barrel", "muzzle"]
    } else if normalized.contains("sight") || normalized.contains("optic") {
        &[
            "sight_attachment",
            "optic_attachment",
            "sightAttach",
            "opticAttach",
            "sight",
            "optic",
        ]
    } else if normalized.contains("underbarrel") {
        &["underbarrel_attachment", "underbarrelAttach", "underbarrel"]
    } else if normalized.contains("muzzle") {
        &["muzzle_flash", "muzzle", "barrel_muzzle"]
    } else {
        &[]
    }
}

fn infer_weapon_part_role(
    requested_anchor: &str,
    entity_name: &str,
    geometry_path: &str,
) -> &'static str {
    let combined = format!("{requested_anchor} {entity_name} {geometry_path}");
    let normalized = normalize_anchor_name(&combined);
    if normalized.contains("magazine") || normalized.contains("mag") {
        "magazine"
    } else if normalized.contains("ammo") || normalized.contains("bullet") {
        "ammo"
    } else if normalized.contains("trigger") {
        "trigger"
    } else if normalized.contains("barrel") {
        "barrel"
    } else if normalized.contains("sight") || normalized.contains("optic") {
        "sight"
    } else if normalized.contains("underbarrel") {
        "underbarrel"
    } else if normalized.contains("muzzle") {
        "muzzle"
    } else if normalized.contains("parts") {
        "parts"
    } else {
        "attachment"
    }
}

fn collect_root_weapon_parts(
    root_entity_name: &str,
    root_geometry_path: &str,
    root_mesh: &Mesh,
    root_nmc: Option<&NodeMeshCombo>,
) -> Vec<serde_json::Value> {
    let Some(nmc) = root_nmc.filter(|nmc| !nmc.nodes.is_empty()) else {
        return Vec::new();
    };

    let mut parts = Vec::new();
    for (index, submesh) in root_mesh.submeshes.iter().enumerate() {
        let node_index = submesh.node_parent_index as usize;
        let Some(node) = nmc.nodes.get(node_index) else {
            continue;
        };
        if node.name.is_empty() {
            continue;
        }

        let material_name = submesh.material_name.clone();
        let role = infer_weapon_part_role(
            &node.name,
            &material_name.clone().unwrap_or_default(),
            root_geometry_path,
        );
        parts.push(serde_json::json!({
            "role": role,
            "entity_name": root_entity_name,
            "geometry_path": root_geometry_path,
            "material_path": null,
            "source": "root_skin_submesh",
            "attach_slot": node.name,
            "attach_bone": node.name,
            "transform_source": "skin_node_parent_index",
            "submesh_index": index,
            "node_parent_index": submesh.node_parent_index,
            "material_id": submesh.material_id,
            "source_material_id": submesh.source_material_id,
            "material_name": material_name,
        }));
    }

    parts
}

fn collect_weapon_anchors(
    root_entity_name: &str,
    root_nmc: Option<&NodeMeshCombo>,
    root_bones: &[Bone],
) -> Vec<serde_json::Value> {
    let mut anchors = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(nmc) = root_nmc {
        let world = compute_nmc_world_transforms(nmc);
        for (index, node) in nmc.nodes.iter().enumerate() {
            if node.name.is_empty() || !seen.insert(node.name.to_ascii_lowercase()) {
                continue;
            }
            anchors.push(serde_json::json!({
                "name": node.name,
                "source": "nmc",
                "parent_index": node.parent_index,
                "local_transform": world.get(index).map(|matrix| mat4_to_array(*matrix)),
            }));
        }
    }

    for (index, bone) in root_bones.iter().enumerate() {
        if bone.name.is_empty() || !seen.insert(bone.name.to_ascii_lowercase()) {
            continue;
        }
        let matrix = super::bone_world_transform(bone);
        anchors.push(serde_json::json!({
            "name": bone.name,
            "source": "skeleton",
            "parent_index": bone.parent_index,
            "local_transform": mat4_to_array(matrix),
            "skeleton_index": index,
        }));
    }

    if seen.insert(root_entity_name.to_ascii_lowercase()) {
        anchors.push(serde_json::json!({
            "name": root_entity_name,
            "source": "scene_root",
            "parent_index": null,
            "local_transform": mat4_to_array(glam::Mat4::IDENTITY),
        }));
    }

    anchors
}

fn collect_cdf_weapon_parts(
    p4k: &starbreaker_p4k::MappedP4k,
    root_geometry_path: &str,
) -> Vec<serde_json::Value> {
    if !root_geometry_path.to_ascii_lowercase().ends_with(".cdf") {
        return Vec::new();
    }

    let Ok(resolved) = resolve_geometry_files(p4k, root_geometry_path) else {
        return Vec::new();
    };

    resolved
        .parts
        .iter()
        .map(|part| {
            serde_json::json!({
                "role": if part.bone_name.is_some() { "rigid_attachment" } else { "skin" },
                "geometry_path": part.path,
                "material_path": part.material_override,
                "source": "cdf_attachment",
                "attach_bone": part.bone_name,
                "transform_source": if part.bone_name.is_some() { "cdf_bonename" } else { "shared_skeleton" },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmc::{NmcNode, NodeMeshCombo};
    use crate::types::{Mesh, SubMesh};

    fn anchors(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn weapon_anchor_alias_resolves_magazine_to_magattach() {
        let resolved = resolve_weapon_anchor("magazine", &anchors(&["root", "magAttach"]), "root");

        assert_eq!(resolved.resolved_anchor, "magAttach");
        assert_eq!(resolved.strategy, "alias");
        assert!(resolved.warning.is_none());
    }

    #[test]
    fn weapon_anchor_alias_resolves_ammo_to_bullet() {
        let resolved = resolve_weapon_anchor("ammo", &anchors(&["root", "bullet"]), "root");

        assert_eq!(resolved.resolved_anchor, "bullet");
        assert_eq!(resolved.strategy, "alias");
        assert!(resolved.warning.is_none());
    }

    #[test]
    fn missing_weapon_anchor_falls_back_to_root_with_warning() {
        let resolved = resolve_weapon_anchor("optic", &anchors(&["root", "magAttach"]), "root");

        assert_eq!(resolved.resolved_anchor, "root");
        assert_eq!(resolved.strategy, "root_fallback");
        assert!(
            resolved
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("optic"))
        );
    }

    #[test]
    fn fps_weapon_classification_uses_structural_evidence() {
        let kind = classify_entity_kind_from_evidence(
            "EntityClassDefinition.behr_p4ar",
            Some("FPSWeapon"),
            Some("WeaponPersonal"),
            Some("Objects/Weapons/FPS_Weapons/BEHR/P4AR/p4ar.cdf"),
        );

        assert_eq!(kind, EntityAssemblyKind::FpsWeapon);
    }

    #[test]
    fn root_skin_submeshes_are_reported_as_weapon_parts() {
        let mesh = Mesh {
            positions: vec![[0.0; 3]; 6],
            indices: vec![0, 1, 2, 3, 4, 5],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![
                SubMesh {
                    material_name: Some("barrel_mat".to_string()),
                    material_id: 0,
                    source_material_id: Some(0),
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 1,
                },
                SubMesh {
                    material_name: Some("trigger_mat".to_string()),
                    material_id: 1,
                    source_material_id: Some(1),
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 2,
                },
            ],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
        };
        let nmc = NodeMeshCombo {
            nodes: vec![
                nmc_node("root", None),
                nmc_node("barrel_attachment", Some(0)),
                nmc_node("trigger01", Some(0)),
            ],
            material_indices: vec![],
        };

        let parts = collect_root_weapon_parts(
            "EntityClassDefinition.test_weapon",
            "objects/fps_weapons/test/weapon_parts.skin",
            &mesh,
            Some(&nmc),
        );

        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["role"], serde_json::json!("barrel"));
        assert_eq!(
            parts[0]["attach_bone"],
            serde_json::json!("barrel_attachment")
        );
        assert_eq!(parts[0]["source"], serde_json::json!("root_skin_submesh"));
        assert_eq!(parts[1]["role"], serde_json::json!("trigger"));
        assert_eq!(parts[1]["attach_bone"], serde_json::json!("trigger01"));
    }

    fn nmc_node(name: &str, parent_index: Option<u16>) -> NmcNode {
        NmcNode {
            name: name.to_string(),
            parent_index,
            world_to_bone: [[0.0; 4]; 3],
            bone_to_world: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            scale: [1.0; 3],
            geometry_type: 0,
            properties: std::collections::HashMap::new(),
        }
    }
}
