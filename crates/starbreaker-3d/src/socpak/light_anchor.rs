//! Item-port anchoring of tile-attached light groups.
//!
//! A light group entity in a `.soc` that carries `TileAttachmentParams` is
//! attached to an item port of the object container's descriptor
//! (`<container>.xml`, `TileItemPortEntries/ItemPort`). The port's
//! `interactionOffset` is the group's origin in the frame of the entity that
//! owns the container; the `.soc` `Pos` only agrees with it when the container
//! was authored in the frame its DataCore offset implies.
//!
//! - [`parse_item_port_positions`] reads the descriptor's port table (CryXMLB
//!   or plain-text XML).
//! - [`assign_item_port_positions`] gives each parsed light its port position.
//! - [`anchor_group_lights_to_item_ports`] moves each group, as a rigid body,
//!   so its origin lands on the port.

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use crate::types::LightInfo;

/// `ItemPort@name` → `ItemPort@interactionOffset` (owner-entity frame, metres).
/// Ports without an authored offset are absent rather than placed at the origin,
/// and so is a name that two ports give different offsets: a group naming it has
/// no single origin, whatever order the descriptor encoding lists the ports in.
pub(crate) fn parse_item_port_positions(data: &[u8]) -> HashMap<String, [f64; 3]> {
    let mut seen = HashMap::new();
    collect_item_ports(data, &mut |name, offset| {
        let (Some(name), Some(offset)) = (name, offset) else {
            return;
        };
        let [x, y, z] = super::parse_csv_f64(offset)[..] else {
            return;
        };
        let position = [x, y, z];
        let known = seen.entry(name.to_owned()).or_insert(Some(position));
        if known.is_some_and(|known| known != position) {
            log::warn!("item port '{name}' is listed with different offsets; its groups stay unanchored");
            *known = None;
        }
    });
    seen.into_iter()
        .filter_map(|(name, position)| Some((name, position?)))
        .collect()
}

/// Feed every `ItemPort`'s `name` and `interactionOffset` attributes to `add`.
fn collect_item_ports(data: &[u8], add: &mut dyn FnMut(Option<&str>, Option<&str>)) {
    if let Ok(xml) = starbreaker_cryxml::from_bytes(data) {
        let mut stack = vec![xml.root()];
        while let Some(node) = stack.pop() {
            if xml.node_tag(node) == "ItemPort" {
                let attrs: HashMap<&str, &str> = xml.node_attributes(node).collect();
                add(attrs.get("name").copied(), attrs.get("interactionOffset").copied());
            }
            stack.extend(xml.node_children(node));
        }
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == "ItemPort" => {
                let attr = |key: &str| {
                    e.attributes()
                        .flatten()
                        .find(|attr| attr.key.as_ref() == key)
                        .and_then(|attr| attr.normalized_value(XmlVersion::Implicit1_0).ok())
                };
                add(attr("name").as_deref(), attr("interactionOffset").as_deref());
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                log::warn!("failed to parse container item-port xml: {e}");
                break;
            }
        }
        buf.clear();
    }
}

/// Record on every tile-attached light the position of the port it names.
pub(crate) fn assign_item_port_positions(
    lights: &mut [LightInfo],
    ports: &HashMap<String, [f64; 3]>,
) {
    for attachment in lights.iter_mut().filter_map(|light| light.attachment.as_mut()) {
        attachment.port_position = attachment
            .item_port
            .as_ref()
            .and_then(|name| ports.get(name))
            .copied();
    }
}

/// Translate each tile-attached light group so that its origin maps to its item
/// port under `owner_from_container`. Lights keep their offset from the group
/// origin and their orientation; lights without a port are left alone.
pub(crate) fn anchor_group_lights_to_item_ports(
    lights: &mut [LightInfo],
    owner_from_container: glam::Mat4,
) {
    let container_from_owner = owner_from_container.inverse();
    for light in lights {
        let Some(attachment) = light.attachment.as_mut() else {
            continue;
        };
        let (Some(group), Some(port)) = (attachment.group_position, attachment.port_position)
        else {
            continue;
        };
        let anchor = container_from_owner
            .transform_point3(glam::Vec3::from_array(port.map(|v| v as f32)))
            .to_array()
            .map(f64::from);
        let moved: f64 = (0..3).map(|i| (anchor[i] - group[i]).powi(2)).sum::<f64>().sqrt();
        log::debug!(
            "light '{}': group '{}' anchored to its item port, moved {moved:.3} m",
            light.name,
            attachment.group
        );
        for axis in 0..3 {
            light.position[axis] += anchor[axis] - group[axis];
        }
        attachment.group_position = Some(anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every name and number below is synthetic.
    const DESCRIPTOR: &str = r#"<ObjectContainer><TileXmlEntry><TileItemPortEntries>
        <ItemPort name="Port7_synthetic_group[layer_a]" interactionOffset="10.5,-20.25,30.125" rotation="1,0,0,0"/>
        <ItemPort name="Port8_no_offset[layer_a]" rotation="1,0,0,0"/>
    </TileItemPortEntries></TileXmlEntry></ObjectContainer>"#;

    const GROUP_SOC: &str = r#"<Entities>
        <Entity EntityClass="LightGroup" Name="synthetic_group" Pos="1.5,-2.5,3.5">
            <TileAttachmentParams ItemPortName="Port7_synthetic_group[layer_a]" ItemPortIndex="7"/>
            <EntityComponentLightGroup><BakedInLights>
                <Light><RelativeXForm translation="0.25,0.5,0.75"/><EntityComponentLight lightType="Omni"><defaultState intensity="0.125"/><sizeParams lightRadius="7"/></EntityComponentLight></Light>
            </BakedInLights></EntityComponentLightGroup>
        </Entity>
        <Entity EntityClass="LightGroup" Name="loose_group" Pos="4.5,5.5,6.5">
            <EntityComponentLightGroup><BakedInLights>
                <Light><RelativeXForm translation="1,0,0"/><EntityComponentLight lightType="Omni"><defaultState intensity="0.125"/><sizeParams lightRadius="7"/></EntityComponentLight></Light>
            </BakedInLights></EntityComponentLightGroup>
        </Entity>
    </Entities>"#;

    fn group_lights() -> Vec<LightInfo> {
        let bytes = crate::test_xml::encode(GROUP_SOC);
        let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
        super::super::extract_cryxml_entities(&xml).1
    }

    fn close(a: [f64; 3], b: glam::Vec3) -> bool {
        (glam::Vec3::from_array(a.map(|x| x as f32)) - b).length() < 1e-4
    }

    #[test]
    fn port_table_reads_text_and_binary_descriptors() {
        for data in [DESCRIPTOR.as_bytes().to_vec(), crate::test_xml::encode(DESCRIPTOR)] {
            let ports = parse_item_port_positions(&data);
            assert_eq!(
                ports.get("Port7_synthetic_group[layer_a]"),
                Some(&[10.5, -20.25, 30.125])
            );
            assert!(!ports.contains_key("Port8_no_offset[layer_a]"));
        }
    }

    #[test]
    fn conflicting_duplicate_port_names_are_dropped_in_both_encodings() {
        let descriptor = r#"<ObjectContainer><TileXmlEntry><TileItemPortEntries>
            <ItemPort name="Port1_twice" interactionOffset="1.5,2.5,3.5"/>
            <ItemPort name="Port2_same" interactionOffset="4.5,5.5,6.5"/>
            <ItemPort name="Port1_twice" interactionOffset="-1.5,2.5,3.5"/>
            <ItemPort name="Port2_same" interactionOffset="4.5,5.5,6.5"/>
            <ItemPort name="Port1_twice" interactionOffset="1.5,2.5,3.5"/>
        </TileItemPortEntries></TileXmlEntry></ObjectContainer>"#;
        for data in [descriptor.as_bytes().to_vec(), crate::test_xml::encode(descriptor)] {
            let ports = parse_item_port_positions(&data);
            assert!(!ports.contains_key("Port1_twice"), "{ports:?}");
            assert_eq!(ports.get("Port2_same"), Some(&[4.5, 5.5, 6.5]));
        }
    }

    #[test]
    fn light_group_records_its_item_port_and_origin() {
        let lights = group_lights();
        assert_eq!(lights.len(), 2);
        let attached = lights[0].attachment.as_ref().unwrap();
        assert_eq!(attached.item_port.as_deref(), Some("Port7_synthetic_group[layer_a]"));
        assert_eq!(attached.attachment_point.as_deref(), Some("layer_a"));
        assert_eq!(attached.group_position, Some([1.5, -2.5, 3.5]));
        assert_eq!(attached.port_position, None);
        let loose = lights[1].attachment.as_ref().unwrap();
        assert_eq!(loose.item_port, None);
        assert_eq!(loose.group_position, Some([4.5, 5.5, 6.5]));
    }

    #[test]
    fn only_lights_naming_a_known_port_receive_its_position() {
        let mut lights = group_lights();
        assign_item_port_positions(&mut lights, &parse_item_port_positions(DESCRIPTOR.as_bytes()));
        assert_eq!(
            lights[0].attachment.as_ref().unwrap().port_position,
            Some([10.5, -20.25, 30.125])
        );
        assert_eq!(lights[1].attachment.as_ref().unwrap().port_position, None);

        let mut unknown = group_lights();
        assign_item_port_positions(&mut unknown, &HashMap::new());
        assert_eq!(unknown[0].attachment.as_ref().unwrap().port_position, None);
    }

    #[test]
    fn anchoring_moves_the_group_origin_onto_the_port_and_keeps_light_offsets() {
        let mut lights = group_lights();
        assign_item_port_positions(&mut lights, &parse_item_port_positions(DESCRIPTOR.as_bytes()));
        let before = lights.clone();
        // The container frame is rotated and offset against its owner.
        let owner_from_container = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_z(0.5),
            glam::vec3(3.0, 4.0, 5.0),
        );
        anchor_group_lights_to_item_ports(&mut lights, owner_from_container);

        let port = glam::vec3(10.5, -20.25, 30.125);
        let offset = glam::Quat::from_rotation_z(0.5) * glam::vec3(0.25, 0.5, 0.75);
        let owner_position = owner_from_container.transform_point3(glam::Vec3::from_array(
            lights[0].position.map(|x| x as f32),
        ));
        assert!((owner_position - (port + offset)).length() < 1e-4, "{owner_position:?}");
        // Not the container-offset placement the `.soc` position alone implies.
        let unanchored = owner_from_container.transform_point3(glam::vec3(1.75, -2.0, 4.25));
        assert!((owner_position - unanchored).length() > 1.0);
        let group = lights[0].attachment.as_ref().unwrap().group_position.unwrap();
        assert!(close(group, owner_from_container.inverse().transform_point3(port)));
        assert_eq!(lights[0].rotation, before[0].rotation);
        assert_eq!(lights[0].direction_sc, before[0].direction_sc);

        // A group without a port keeps its authored container-local placement.
        assert_eq!(lights[1].position, before[1].position);
        assert_eq!(lights[1].position, [5.5, 5.5, 6.5]);
    }

    #[test]
    fn container_delta_transform_carries_the_group_origin_with_the_light() {
        let light = group_lights().remove(0);
        let delta = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_x(0.25),
            glam::vec3(-1.0, 2.0, 0.5),
        );
        let moved = super::super::transform_non_item_port_light(light, delta.to_cols_array_2d());
        let group = moved.attachment.as_ref().unwrap().group_position.unwrap();
        assert!(close(group, delta.transform_point3(glam::vec3(1.5, -2.5, 3.5))));
        assert!(close(moved.position, delta.transform_point3(glam::vec3(1.75, -2.0, 4.25))));
    }
}
