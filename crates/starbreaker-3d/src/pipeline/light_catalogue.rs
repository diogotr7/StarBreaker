//! Metadata-only light discovery and shared entity-space/attachment resolution.
//! Uses the same authored SOC components, container transforms and loadout traversal as export.
use super::*;
use crate::types::{LightInfo, ResolvedNode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityLight {
    pub owner_entity: String,
    pub source_container: String,
    pub source_container_index: usize,
    pub source_parent_node_name: Option<String>,
    pub source_light_index: usize,
    /// Actual NMC node name, when an authored group attachment resolves.
    pub parent_node_name: Option<String>,
    /// Same light in its attachment node's local frame, for animation.
    pub parent_local_position: Option<[f64; 3]>,
    pub parent_local_rotation: Option<[f64; 4]>,
    /// Entity-local CryEngine Z-up metres. Projector texture remains a source path in the API.
    #[serde(flatten)]
    pub light: LightInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightCatalogue {
    pub entity: String,
    pub ports: Vec<VehicleLightPort>,
    pub lights: Vec<EntityLight>,
}

pub fn entity_light_catalogue(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
) -> Result<LightCatalogue, Error> {
    let opts = ExportOptions {
        apply_default_animation_pose: false,
        ..Default::default()
    };
    let root = match resolve_loadout_meshes(db, p4k, record, tree, &opts) {
        Ok(root) => Some(root),
        Err(Error::NoGeometryComponent { .. }) => None,
        Err(error) => return Err(error),
    };
    let fallback = if root.is_none() {
        resolve_children(
            db,
            p4k,
            &tree.root.children,
            &opts,
            &load_invisible_ports(db, p4k, record),
        )
    } else {
        Vec::new()
    };
    let children = root
        .as_ref()
        .map_or(fallback.as_slice(), |r| r.children.as_slice());
    let mut light_resolver = crate::socpak::light_component::LightResolver::new(db);
    let mut interiors = load_interiors(db, p4k, record, &opts, &mut light_resolver);
    let names = interiors
        .containers
        .iter()
        .map(|c| c.name.to_ascii_lowercase())
        .collect();
    let child_interiors = load_child_interiors(
        db,
        p4k,
        &tree.root.entity_name,
        &children,
        &names,
        &opts,
        &mut light_resolver,
    );
    merge_interiors(&mut interiors, child_interiors);
    let mut rigs = HashMap::new();
    if let Some(nmc) = root.as_ref().and_then(|r| r.nmc.as_ref()) {
        rigs.insert(tree.root.entity_name.clone(), nmc);
    }
    fn walk<'a>(nodes: &'a [ResolvedNode], rigs: &mut HashMap<String, &'a nmc::NodeMeshCombo>) {
        for node in nodes {
            if let Some(nmc) = &node.nmc {
                rigs.insert(node.entity_name.clone(), nmc);
            }
            walk(&node.children, rigs);
        }
    }
    walk(&children, &mut rigs);
    let mut owners: Vec<_> = rigs.keys().cloned().collect();
    if !owners.contains(&tree.root.entity_name) {
        owners.push(tree.root.entity_name.clone());
    }
    let mut ports = load_owner_light_ports(db, p4k, &owners)?;
    let lights = collect_entity_lights(&tree.root.entity_name, &rigs, &ports, &interiors);
    Ok(LightCatalogue {
        entity: tree.root.entity_name.clone(),
        ports: ports.remove(&tree.root.entity_name).unwrap_or_default(),
        lights,
    })
}

fn find_helper(nmc: &nmc::NodeMeshCombo, name: &str) -> Option<(String, glam::Mat4)> {
    let index = nmc
        .nodes
        .iter()
        .position(|n| n.name.eq_ignore_ascii_case(name))?;
    Some((
        nmc.nodes[index].name.clone(),
        compute_nmc_world_transforms(nmc)[index],
    ))
}

pub(crate) fn collect_entity_lights(
    root: &str,
    rigs: &HashMap<String, &nmc::NodeMeshCombo>,
    ports: &HashMap<String, Vec<VehicleLightPort>>,
    interiors: &LoadedInteriors,
) -> Vec<EntityLight> {
    let mut out = Vec::new();
    for (ci, container) in interiors.containers.iter().enumerate() {
        let owner = container.parent_entity_name.as_deref().unwrap_or(root);
        let rig = rigs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(owner))
            .map(|(_, rig)| *rig);
        let container_parent = container
            .parent_node_name
            .as_deref()
            .and_then(|name| rig.and_then(|nmc| find_helper(nmc, name)));
        let world = container_parent
            .as_ref()
            .map_or(glam::Mat4::IDENTITY, |(_, m)| *m)
            * glam::Mat4::from_cols_array_2d(&container.container_transform);
        for (li, light) in container.lights.iter().enumerate() {
            let attachment = light.attachment.as_ref();
            let xml_bone = attachment
                .and_then(|a| a.attachment_point.as_ref())
                .and_then(|point| {
                    ports
                        .get(owner)
                        .into_iter()
                        .flatten()
                        .find_map(|port| port.attachment_points.get(point))
                });
            let helper = attachment
                .and_then(|a| a.helper.as_deref())
                .or(xml_bone.map(String::as_str));
            let resolved = helper.and_then(|h| rig.and_then(|nmc| find_helper(nmc, h)));
            if helper.is_some() && resolved.is_none() {
                log::warn!(
                    "Light '{}' authored helper {:?} not found on '{}'",
                    light.name,
                    helper,
                    owner
                );
            }
            let entity_light = crate::socpak::transform_non_item_port_light(
                light.clone(),
                world.to_cols_array_2d(),
            );
            let local = resolved.as_ref().map(|(_, helper)| {
                crate::socpak::transform_non_item_port_light(
                    entity_light.clone(),
                    helper.inverse().to_cols_array_2d(),
                )
            });
            out.push(EntityLight {
                owner_entity: owner.to_owned(),
                source_container: container.name.clone(),
                source_container_index: ci,
                source_light_index: li,
                source_parent_node_name: container.parent_node_name.clone(),
                parent_node_name: resolved.map(|(name, _)| name),
                parent_local_position: local.as_ref().map(|l| l.position),
                parent_local_rotation: local.as_ref().map(|l| l.rotation),
                light: entity_light,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_attachment_preserves_entity_position_and_recovers_animated_local_frame() {
        // All values synthetic. Container and attachment have different rotations/translations.
        let joint = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_z(0.5),
            glam::vec3(3.0, 4.0, 5.0),
        );
        let cols = joint.to_cols_array_2d();
        let rig = nmc::NodeMeshCombo {
            material_indices: vec![],
            nodes: vec![nmc::NmcNode {
                name: "joint".into(),
                parent_index: None,
                bone_to_world: std::array::from_fn(|r| std::array::from_fn(|c| cols[c][r])),
                world_to_bone: [[0.0; 4]; 3],
                scale: [1.0; 3],
                geometry_type: 3,
                properties: HashMap::new(),
            }],
        };
        let light: LightInfo = serde_json::from_value(serde_json::json!({
            "name":"synthetic", "position":[1.0,2.0,3.0],"rotation":[1.0,0.0,0.0,0.0],
            "direction_sc":[1.0,0.0,0.0],"transform_basis":"cryengine_z_up",
            "color":[0.5,0.25,0.75],"light_type":"Omni","semantic_light_kind":"point",
            "intensity_raw":0.5,"intensity_unit":"cryengine_authored_intensity",
            "intensity_candela_proxy":750.0,"intensity":750.0,"radius":9.0,"radius_m":9.0,
            "inner_angle":null,"outer_angle":null,"projector_texture":null,
            "active_state":"","states":{},"light_style":2,
            "attachment":{"group":"arbitrary","helper":"joint","attachment_point":"tip"}
        }))
        .unwrap();
        let mut interiors = LoadedInteriors::default();
        let transform = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_x(0.25),
            glam::vec3(7.0, 8.0, 9.0),
        );
        interiors.containers.push(InteriorContainerData {
            name: "container".into(),
            parent_entity_name: Some("owner".into()),
            parent_node_name: None,
            container_transform: transform.to_cols_array_2d(),
            room_volumes: Vec::new(),
            placements: vec![],
            lights: vec![light],
            palette: None,
        });
        let rigs = HashMap::from([("owner".into(), &rig)]);
        let result = collect_entity_lights("owner", &rigs, &HashMap::new(), &interiors);
        let record = &result[0];
        assert_eq!(record.parent_node_name.as_deref(), Some("joint"));
        let position = glam::Vec3::from_array(record.light.position.map(|x| x as f32));
        assert!((position - transform.transform_point3(glam::vec3(1.0, 2.0, 3.0))).length() < 1e-5);
        let local = glam::Vec3::from_array(record.parent_local_position.unwrap().map(|x| x as f32));
        assert!((position - joint.transform_point3(local)).length() < 1e-5);
        let direction = glam::Vec3::from_array(record.light.direction_sc.map(|x| x as f32));
        assert!((direction.length() - 1.0).abs() < 1e-5);
        assert_eq!(record.light.color, Some([0.5, 0.25, 0.75]));
    }
    #[test]
    fn item_port_anchored_group_lands_on_its_port_in_the_owner_frame() {
        // All values synthetic. The container hangs off a rotated, offset helper, the way a
        // helper-parented object container does; the group's port lies elsewhere in the owner frame.
        let helper = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_z(0.5),
            glam::vec3(3.0, 4.0, 5.0),
        );
        let cols = helper.to_cols_array_2d();
        let rig = nmc::NodeMeshCombo {
            material_indices: vec![],
            nodes: vec![nmc::NmcNode {
                name: "mount".into(),
                parent_index: None,
                bone_to_world: std::array::from_fn(|r| std::array::from_fn(|c| cols[c][r])),
                world_to_bone: [[0.0; 4]; 3],
                scale: [1.0; 3],
                geometry_type: 3,
                properties: HashMap::new(),
            }],
        };
        let mut light: LightInfo = serde_json::from_value(serde_json::json!({
            "name":"synthetic", "position":[1.0,2.0,3.0],"rotation":[1.0,0.0,0.0,0.0],
            "direction_sc":[1.0,0.0,0.0],"transform_basis":"cryengine_z_up",
            "color":[0.5,0.25,0.75],"light_type":"Omni","semantic_light_kind":"point",
            "intensity_raw":0.5,"intensity_unit":"cryengine_authored_intensity",
            "intensity_candela_proxy":750.0,"intensity":750.0,"radius":9.0,"radius_m":9.0,
            "inner_angle":null,"outer_angle":null,"projector_texture":null,
            "active_state":"","states":{},"light_style":2,
            "attachment":{"group":"arbitrary","helper":"mount","attachment_point":"tip"}
        }))
        .unwrap();
        let attachment = light.attachment.as_mut().unwrap();
        attachment.group_position = Some([0.5, 1.5, 2.0]);
        attachment.port_position = Some([10.0, 20.0, 30.0]);
        let mut lights = vec![light];
        crate::socpak::light_anchor::anchor_group_lights_to_item_ports(&mut lights, helper);
        let mut interiors = LoadedInteriors::default();
        interiors.containers.push(InteriorContainerData {
            name: "container".into(),
            parent_entity_name: Some("owner".into()),
            parent_node_name: Some("mount".into()),
            container_transform: glam::Mat4::IDENTITY.to_cols_array_2d(),
            room_volumes: Vec::new(),
            placements: vec![],
            lights,
            palette: None,
        });
        let rigs = HashMap::from([("owner".into(), &rig)]);
        let result = collect_entity_lights("owner", &rigs, &HashMap::new(), &interiors);
        let record = &result[0];
        let position = glam::Vec3::from_array(record.light.position.map(|x| x as f32));
        let expected = glam::vec3(10.0, 20.0, 30.0)
            + glam::Quat::from_rotation_z(0.5) * glam::vec3(0.5, 0.5, 1.0);
        assert!((position - expected).length() < 1e-4, "{position:?}");
        let local = glam::Vec3::from_array(record.parent_local_position.unwrap().map(|x| x as f32));
        assert!((position - helper.transform_point3(local)).length() < 1e-4);
    }
    #[test]
    fn suffix_helpers_do_not_bind_even_when_unique() {
        let node = |name: &str| nmc::NmcNode {
            name: name.into(),
            parent_index: None,
            bone_to_world: [[0.0; 4]; 3],
            world_to_bone: [[0.0; 4]; 3],
            scale: [1.0; 3],
            geometry_type: 3,
            properties: HashMap::new(),
        };
        let nmc = nmc::NodeMeshCombo {
            nodes: vec![node("a_tip")],
            material_indices: vec![],
        };
        assert!(find_helper(&nmc, "tip").is_none());
    }
}
