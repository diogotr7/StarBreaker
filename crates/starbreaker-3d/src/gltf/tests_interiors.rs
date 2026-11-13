//! Bundled-GLB placement of interior containers: a container authored on a
//! helper of its owner entity hangs under that helper's node, so its
//! helper-relative `container_transform` (and the lights and meshes below it)
//! resolve to the same owner-frame positions the decomposed export reports.

use super::glb_builder::GlbBuilder;
use crate::pipeline::{InteriorContainerData, LoadedInteriors, MaterialMode};
use gltf_json as json;

// Every name and number below is synthetic.
const LIGHT_SOC: &str = r#"<Entities>
    <Entity EntityClass="LightGroup" Name="synthetic_group" Pos="-7.5,0.5,0.25">
        <EntityComponentLightGroup><BakedInLights>
            <Light><RelativeXForm translation="0.5,0,0"/><EntityComponentLight lightType="Omni"><defaultState intensity="0.125"/><sizeParams lightRadius="7"/></EntityComponentLight></Light>
        </BakedInLights></EntityComponentLightGroup>
    </Entity>
</Entities>"#;

fn builder_with_helper() -> (GlbBuilder, u32) {
    let mut builder = GlbBuilder::new();
    builder.nodes_json.push(json::Node { name: Some("synthetic_owner".into()), ..Default::default() });
    builder.nodes_json.push(json::Node {
        name: Some("Helper_Synthetic".into()),
        translation: Some([10.0, 0.0, 0.0]),
        ..Default::default()
    });
    builder.append_child_to_node(0, 1);
    builder.node_name_to_idx.insert("synthetic_owner".into(), 0);
    builder.node_name_to_idx.insert("helper_synthetic".into(), 1);
    (builder, 1)
}

fn container(parent_node_name: Option<&str>) -> LoadedInteriors {
    LoadedInteriors {
        unique_cgfs: Vec::new(),
        containers: vec![InteriorContainerData {
            name: "synthetic_container".into(),
            parent_entity_name: parent_node_name.map(|_| "synthetic_owner".into()),
            parent_node_name: parent_node_name.map(str::to_owned),
            container_transform: glam::Mat4::IDENTITY.to_cols_array_2d(),
            placements: Vec::new(),
            lights: crate::socpak::lights_from_soc_xml(LIGHT_SOC),
            room_volumes: Vec::new(),
            palette: None,
            proximity_sensors: Vec::new(),
        }],
    }
}

fn attach(builder: &mut GlbBuilder, interiors: &LoadedInteriors) -> Vec<u32> {
    let (scene_nodes, lights) = builder.attach_interiors(
        interiors,
        MaterialMode::None,
        None,
        &mut |_, _| None,
        &mut |_| None,
    );
    assert_eq!(lights.len(), 1);
    scene_nodes.iter().map(|node| node.value() as u32).collect()
}

fn node_named(builder: &GlbBuilder, name: &str) -> usize {
    builder.nodes_json.iter().position(|node| node.name.as_deref() == Some(name)).unwrap()
}

fn children(builder: &GlbBuilder, node: usize) -> Vec<usize> {
    builder.nodes_json[node].children.iter().flatten().map(|child| child.value()).collect()
}

/// World translation of a node through both `matrix` and TRS node transforms.
fn world_translation(builder: &GlbBuilder, node: usize) -> glam::Vec3 {
    let local = |index: usize| {
        let node = &builder.nodes_json[index];
        node.matrix.map_or_else(
            || glam::Mat4::from_translation(glam::Vec3::from_array(node.translation.unwrap_or_default())),
            |matrix| glam::Mat4::from_cols_array(&matrix),
        )
    };
    let mut world = local(node);
    let mut current = node;
    while let Some(parent) = (0..builder.nodes_json.len()).find(|&index| children(builder, index).contains(&current)) {
        world = local(parent) * world;
        current = parent;
    }
    world.w_axis.truncate()
}

#[test]
fn helper_parented_container_hangs_under_its_helper_node() {
    let (mut builder, helper) = builder_with_helper();
    let scene_nodes = attach(&mut builder, &container(Some("Helper_Synthetic")));

    let container_node = node_named(&builder, "interior_synthetic_container");
    assert!(scene_nodes.is_empty(), "a parented container is not a scene root");
    assert_eq!(children(&builder, helper as usize), vec![container_node]);
    let light = children(&builder, container_node)[0];
    assert_eq!(world_translation(&builder, light), glam::Vec3::new(3.0, 0.5, 0.25));
}

#[test]
fn unparented_container_stays_a_scene_root() {
    let (mut builder, helper) = builder_with_helper();
    let scene_nodes = attach(&mut builder, &container(None));

    let container_node = node_named(&builder, "interior_synthetic_container");
    assert_eq!(scene_nodes, vec![container_node as u32]);
    assert!(children(&builder, helper as usize).is_empty());
}

#[test]
fn prefixed_helper_node_is_found_like_the_nmc_helper_lookup() {
    let (mut builder, helper) = builder_with_helper();
    builder.nodes_json[helper as usize].name = Some("body_7_Helper_Synthetic".into());
    builder.node_name_to_idx.remove("helper_synthetic");
    builder.node_name_to_idx.insert("body_7_helper_synthetic".into(), helper);
    let scene_nodes = attach(&mut builder, &container(Some("Helper_Synthetic")));

    let container_node = node_named(&builder, "interior_synthetic_container");
    assert!(scene_nodes.is_empty());
    assert_eq!(children(&builder, helper as usize), vec![container_node]);
}

#[test]
fn container_whose_helper_has_no_node_hangs_under_its_owner_entity() {
    let (mut builder, helper) = builder_with_helper();
    let scene_nodes = attach(&mut builder, &container(Some("helper_absent")));

    let container_node = node_named(&builder, "interior_synthetic_container");
    assert!(scene_nodes.is_empty());
    assert_eq!(children(&builder, 0), vec![helper as usize, container_node]);
}

#[test]
fn container_whose_parents_have_no_node_stays_a_scene_root() {
    let (mut builder, helper) = builder_with_helper();
    builder.node_name_to_idx.remove("synthetic_owner");
    let scene_nodes = attach(&mut builder, &container(Some("helper_absent")));

    let container_node = node_named(&builder, "interior_synthetic_container");
    assert_eq!(scene_nodes, vec![container_node as u32]);
    assert_eq!(children(&builder, 0), vec![helper as usize]);
}
