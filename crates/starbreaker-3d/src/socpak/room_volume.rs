//! Room volumes authored in interior object containers (parsing and types;
//! point containment lives in `room_containment.rs`).
//!
//! An object container registers its rooms in the socpak sidecar
//! `<name>/metadata/entitycomponentroom.xml` (one `<Entity CryGUID="…"/>` per
//! room). Inside the `.soc`, an entity carrying an `AreaComponent` whose
//! `<Entities>` list links one of those GUIDs describes the extent of that
//! room as one or more segments:
//!
//! * `Type="Box"` — `BoxMin` / `BoxMax` in segment space,
//! * `Type="Shape"` — a `<Points>` polygon in the segment XY plane, extruded
//!   along +Z by `Height` from the lowest point (the CryEngine area-shape
//!   convention; the extrusion base is an inference from that convention),
//! * `Type="Trimesh"` — a closed triangle soup, three `<Point pos>` per face.
//!
//! Each segment has its own `transform_ls` (`Q : w,x,y,z | T : x,y,z`) below
//! the entity's `Pos` / `Rotate` / `Scale`. Volumes are exported in the same
//! container-local space as the container's lights so a consumer can decide
//! which room a light or a surface belongs to — the renderer confines lights
//! flagged `affectsThisAreaOnly` to the area they sit in.
//!
//! Rooms are recognised by the registry GUID only: no entity class or name is
//! consulted, so area shapes that link audio or navigation entities are left
//! out by construction.

use std::collections::{HashMap, HashSet};

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use starbreaker_cryxml::{CryXml, CryXmlNode};

/// Geometry of one room-volume segment, in segment space.
#[derive(Debug, Clone, PartialEq)]
pub enum RoomVolumeShape {
    Box { min: [f32; 3], max: [f32; 3] },
    /// Polygon in the XY plane extruded along +Z by `height` from the lowest point.
    Prism { points: Vec<[f32; 3]>, height: f32 },
    /// Closed triangle soup: three vertices per triangle.
    Trimesh { vertices: Vec<[f32; 3]> },
}

/// One segment of a room's extent.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomVolume {
    /// `CryGUID` of the room entity (lower case), as listed in the registry.
    pub room_guid: String,
    /// Authored name of the room entity when it lives in the same `.soc`. Label only.
    pub room_name: Option<String>,
    /// Authored name of the entity carrying the `AreaComponent`. Label only.
    pub source_entity: String,
    /// 4×4 column-major transform: segment space → container-local space.
    pub transform: [[f32; 4]; 4],
    pub shape: RoomVolumeShape,
}

impl RoomVolume {
    /// Apply the container's non-item-port delta, exactly as container lights receive it.
    pub(crate) fn transformed(mut self, delta: [[f32; 4]; 4]) -> Self {
        self.transform = (glam::Mat4::from_cols_array_2d(&delta)
            * glam::Mat4::from_cols_array_2d(&self.transform))
        .to_cols_array_2d();
        self
    }

    /// `scene.json` representation (`interiors[].room_volumes[]`).
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "room": self.room_guid,
            "room_name": self.room_name,
            "source": self.source_entity,
            "transform": self.transform,
        });
        let object = value.as_object_mut().expect("json object");
        match &self.shape {
            RoomVolumeShape::Box { min, max } => {
                object.insert("kind".into(), "box".into());
                object.insert("box_min".into(), serde_json::json!(min));
                object.insert("box_max".into(), serde_json::json!(max));
            }
            RoomVolumeShape::Prism { points, height } => {
                object.insert("kind".into(), "prism".into());
                object.insert("points".into(), serde_json::json!(points));
                object.insert("height".into(), serde_json::json!(height));
            }
            RoomVolumeShape::Trimesh { vertices } => {
                object.insert("kind".into(), "trimesh".into());
                object.insert("vertices".into(), serde_json::json!(vertices));
            }
        }
        value
    }
}

/// Parse `metadata/entitycomponentroom.xml` (binary CryXMLB or plain text)
/// into the lower-cased `CryGUID`s of the container's rooms.
pub(crate) fn parse_room_registry(data: &[u8]) -> HashSet<String> {
    let mut rooms = HashSet::new();
    if let Ok(xml) = starbreaker_cryxml::from_bytes(data) {
        fn walk(xml: &CryXml, node: &CryXmlNode, rooms: &mut HashSet<String>) {
            if xml.node_tag(node) == "Entity" {
                if let Some((_, guid)) = xml.node_attributes(node).find(|(key, _)| *key == "CryGUID") {
                    rooms.insert(guid.to_ascii_lowercase());
                }
            }
            for child in xml.node_children(node) {
                walk(xml, child, rooms);
            }
        }
        walk(&xml, xml.root(), &mut rooms);
        return rooms;
    }

    let Ok(text) = std::str::from_utf8(data) else {
        return rooms;
    };
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == "Entity" => {
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == "CryGUID" {
                        if let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) {
                            rooms.insert(value.to_ascii_lowercase());
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                log::warn!("failed to parse room registry xml: {e}");
                break;
            }
        }
        buf.clear();
    }
    rooms
}

/// Collect the room volumes of one `.soc` entity chunk.
///
/// `rooms` is the container's registry ([`parse_room_registry`]); an
/// `AreaComponent` that links none of those GUIDs is not a room and is skipped.
pub(crate) fn extract_room_volumes(xml: &CryXml, rooms: &HashSet<String>) -> Vec<RoomVolume> {
    if rooms.is_empty() {
        return Vec::new();
    }
    let root = xml.root();
    let entities: Vec<&CryXmlNode> = match xml
        .node_children(root)
        .find(|child| matches!(xml.node_tag(child), "Entities" | "SCOC_Entities"))
    {
        Some(container) => xml.node_children(container).collect(),
        None if matches!(xml.node_tag(root), "Entities" | "SCOC_Entities") => {
            xml.node_children(root).collect()
        }
        None => Vec::new(),
    };

    let mut room_names: HashMap<String, &str> = HashMap::new();
    for entity in &entities {
        let attrs: HashMap<&str, &str> = xml.node_attributes(entity).collect();
        if let (Some(guid), Some(name)) = (attrs.get("EntityCryGUID"), attrs.get("Name")) {
            let guid = guid.to_ascii_lowercase();
            if rooms.contains(&guid) {
                room_names.insert(guid, name);
            }
        }
    }

    let mut volumes = Vec::new();
    for entity in entities {
        if xml.node_tag(entity) != "Entity" {
            continue;
        }
        // The component is serialised twice on some entities; the copies are identical.
        let Some(area) = xml
            .node_children(entity)
            .find(|child| xml.node_tag(child) == "AreaComponent")
        else {
            continue;
        };
        let Some(room_guid) = xml
            .node_children(area)
            .filter(|child| xml.node_tag(child) == "Entities")
            .flat_map(|links| xml.node_children(links))
            .filter_map(|link| {
                xml.node_attributes(link)
                    .find(|(key, _)| *key == "EntityCryGUID")
                    .map(|(_, guid)| guid.to_ascii_lowercase())
            })
            .find(|guid| rooms.contains(guid))
        else {
            continue;
        };

        let attrs: HashMap<&str, &str> = xml.node_attributes(entity).collect();
        let source_entity = attrs.get("Name").copied().unwrap_or_default().to_string();
        let entity_transform = glam::Mat4::from_scale_rotation_translation(
            parse_vec3(attrs.get("Scale").copied()).unwrap_or(glam::Vec3::ONE),
            parse_quat_wxyz(attrs.get("Rotate").copied()).unwrap_or(glam::Quat::IDENTITY),
            parse_vec3(attrs.get("Pos").copied()).unwrap_or(glam::Vec3::ZERO),
        );

        for segment in xml
            .node_children(area)
            .filter(|child| xml.node_tag(child) == "Segment")
        {
            let segment_attrs: HashMap<&str, &str> = xml.node_attributes(segment).collect();
            let Some(shape) = parse_segment_shape(xml, segment, &segment_attrs) else {
                log::debug!(
                    "room volume '{source_entity}': unsupported or malformed segment type {:?}",
                    segment_attrs.get("Type")
                );
                continue;
            };
            let segment_transform =
                parse_segment_transform(segment_attrs.get("transform_ls").copied().unwrap_or(""));
            volumes.push(RoomVolume {
                room_name: room_names.get(&room_guid).map(|name| name.to_string()),
                room_guid: room_guid.clone(),
                source_entity: source_entity.clone(),
                transform: (entity_transform * segment_transform).to_cols_array_2d(),
                shape,
            });
        }
    }
    volumes
}

fn parse_segment_shape(
    xml: &CryXml,
    segment: &CryXmlNode,
    attrs: &HashMap<&str, &str>,
) -> Option<RoomVolumeShape> {
    let point_of = |node: &CryXmlNode| {
        xml.node_attributes(node)
            .find(|(key, _)| key.eq_ignore_ascii_case("pos"))
            .and_then(|(_, value)| parse_vec3(Some(value)))
            .map(|v| v.to_array())
    };
    match attrs.get("Type").copied()? {
        "Box" => {
            let min = parse_vec3(attrs.get("BoxMin").copied())?;
            let max = parse_vec3(attrs.get("BoxMax").copied())?;
            Some(RoomVolumeShape::Box {
                min: min.min(max).to_array(),
                max: min.max(max).to_array(),
            })
        }
        "Shape" => {
            let height: f32 = attrs.get("Height")?.trim().parse().ok()?;
            let points: Vec<[f32; 3]> = xml
                .node_children(segment)
                .filter(|child| xml.node_tag(child) == "Points")
                .flat_map(|points| xml.node_children(points))
                .filter(|child| xml.node_tag(child) == "Point")
                .filter_map(point_of)
                .collect();
            (points.len() >= 3 && height.is_finite() && height > 0.0)
                .then_some(RoomVolumeShape::Prism { points, height })
        }
        "Trimesh" => {
            let mut vertices: Vec<[f32; 3]> = xml
                .node_children(segment)
                .filter(|child| xml.node_tag(child) == "Point")
                .filter_map(point_of)
                .collect();
            vertices.truncate(vertices.len() / 3 * 3);
            // A closed surface needs at least a tetrahedron.
            (vertices.len() >= 12).then_some(RoomVolumeShape::Trimesh { vertices })
        }
        _ => None,
    }
}

/// `Q : w,x,y,z | T : x,y,z` → matrix. Missing halves stay identity.
fn parse_segment_transform(text: &str) -> glam::Mat4 {
    let mut rotation = glam::Quat::IDENTITY;
    let mut translation = glam::Vec3::ZERO;
    for part in text.split('|') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Q" => rotation = parse_quat_wxyz(Some(value)).unwrap_or(rotation),
            "T" => translation = parse_vec3(Some(value)).unwrap_or(translation),
            _ => {}
        }
    }
    glam::Mat4::from_rotation_translation(rotation, translation)
}

fn parse_floats(text: Option<&str>) -> Option<Vec<f32>> {
    let values: Vec<f32> = text?
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    values.iter().all(|value| value.is_finite()).then_some(values)
}

fn parse_vec3(text: Option<&str>) -> Option<glam::Vec3> {
    let values = parse_floats(text)?;
    (values.len() == 3).then(|| glam::Vec3::new(values[0], values[1], values[2]))
}

fn parse_quat_wxyz(text: Option<&str>) -> Option<glam::Quat> {
    let values = parse_floats(text)?;
    if values.len() != 4 {
        return None;
    }
    let quat = glam::Quat::from_xyzw(values[1], values[2], values[3], values[0]);
    (quat.length_squared() > 1e-12).then(|| quat.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: &str = "aaaaaaaa-0000-0000-0000-000000000001";

    fn registry() -> HashSet<String> {
        parse_room_registry(
            br#"<EntityComponentRoom><Entity BoundsCenter="0,0,0" CryGUID="AAAAAAAA-0000-0000-0000-000000000001"/></EntityComponentRoom>"#,
        )
    }

    fn volumes(entities: &str) -> Vec<RoomVolume> {
        let bytes = crate::test_xml::encode(&format!("<Entities>{entities}</Entities>"));
        let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
        extract_room_volumes(&xml, &registry())
    }

    fn area(entity_attrs: &str, segments: &str, link: &str) -> String {
        format!(
            r#"<Entity Name="shape" {entity_attrs}><AreaComponent Id="-1">{segments}<Entities><Entity EntityCryGUID="{link}"/></Entities></AreaComponent></Entity>"#
        )
    }

    #[test]
    fn registry_reads_text_and_binary_and_lower_cases_guids() {
        assert_eq!(registry(), HashSet::from([ROOM.to_string()]));
        let binary = crate::test_xml::encode(
            r#"<EntityComponentRoom><Entity CryGUID="AAAAAAAA-0000-0000-0000-000000000001"/><Entity CryGUID="bbbbbbbb-0000-0000-0000-000000000002"/></EntityComponentRoom>"#,
        );
        assert_eq!(parse_room_registry(&binary).len(), 2);
        assert!(parse_room_registry(&binary).contains(ROOM));
    }

    #[test]
    fn area_linking_an_unregistered_entity_is_not_a_room() {
        let segment = r#"<Segment transform_ls="Q : 1,0,0,0 | T : 0,0,0" Type="Box" BoxMin="-1,-1,-1" BoxMax="1,1,1"/>"#;
        assert!(volumes(&area(r#"Pos="0,0,0""#, segment, "cccccccc-0000-0000-0000-000000000003")).is_empty());
        assert_eq!(volumes(&area(r#"Pos="0,0,0""#, segment, ROOM)).len(), 1);
    }

    #[test]
    fn room_name_comes_from_the_linked_entity_and_duplicate_components_collapse() {
        let segment = r#"<Segment transform_ls="Q : 1,0,0,0 | T : 0,0,0" Type="Box" BoxMin="0,0,0" BoxMax="1,1,1"/>"#;
        let component = format!(
            r#"<AreaComponent>{segment}<Entities><Entity EntityCryGUID="{ROOM}"/></Entities></AreaComponent>"#
        );
        let found = volumes(&format!(
            r#"<Entity Name="synthetic room" EntityCryGUID="{ROOM}"/><Entity Name="shape" Pos="0,0,0">{component}<Tags/>{component}</Entity>"#
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].room_guid, ROOM);
        assert_eq!(found[0].room_name.as_deref(), Some("synthetic room"));
        assert_eq!(found[0].source_entity, "shape");
    }

    #[test]
    fn box_composes_entity_and_segment_transforms() {
        // Entity: +10 on X, quarter turn about Z (w = x-of-(w,x,y,z) = cos 45°).
        // Segment: +2 on its own X. Segment X therefore runs along container Y.
        let found = volumes(&area(
            r#"Pos="10,0,0" Rotate="0.70710678,0,0,0.70710678""#,
            r#"<Segment transform_ls="Q : 1,0,0,0 | T : 2,0,0" Type="Box" BoxMin="0,0,0" BoxMax="3,1,1"/>"#,
            ROOM,
        ));
        let volume = &found[0];
        // Segment point (2.5, 0.5, 0.5) → entity space (4.5, 0.5, 0.5) → rotated (-0.5, 4.5, 0.5) → +10 X.
        assert!(volume.contains([9.5, 4.5, 0.5]));
        assert!(!volume.contains([9.5, 1.5, 0.5])); // before the segment translation
        assert!(!volume.contains([14.5, 0.5, 0.5])); // where it would sit without the rotation
        let moved = volume.clone().transformed(
            glam::Mat4::from_translation(glam::vec3(0.0, 0.0, 100.0)).to_cols_array_2d(),
        );
        assert!(moved.contains([9.5, 4.5, 100.5]));
        assert!(!moved.contains([9.5, 4.5, 0.5]));
    }

    #[test]
    fn prism_extrudes_the_xy_polygon_up_from_its_lowest_point() {
        // L-shaped footprint; base at z = 1, height 2.
        let found = volumes(&area(
            r#"Pos="0,0,0""#,
            r#"<Segment transform_ls="Q : 1,0,0,0 | T : 0,0,0" Type="Shape" Height="2"><Points><Point Pos="0,0,1"/><Point Pos="4,0,1"/><Point Pos="4,1,1"/><Point Pos="1,1,1"/><Point Pos="1,4,1"/><Point Pos="0,4,1"/></Points></Segment>"#,
            ROOM,
        ));
        let volume = &found[0];
        assert!(volume.contains([3.0, 0.5, 2.0]));
        assert!(volume.contains([0.5, 3.0, 2.9]));
        assert!(!volume.contains([3.0, 3.0, 2.0])); // the notch of the L
        assert!(!volume.contains([3.0, 0.5, 0.5])); // below the base
        assert!(!volume.contains([3.0, 0.5, 3.5])); // above base + height
    }

    #[test]
    fn trimesh_parity_handles_a_closed_soup() {
        // Tetrahedron with vertices O, 4X, 4Y, 4Z.
        let found = volumes(&area(
            r#"Pos="0,0,0""#,
            r#"<Segment transform_ls="Q : 1,0,0,0 | T : 0,0,0" Type="Trimesh"><Point pos="0,0,0"/><Point pos="4,0,0"/><Point pos="0,4,0"/><Point pos="0,0,0"/><Point pos="0,4,0"/><Point pos="0,0,4"/><Point pos="0,0,0"/><Point pos="0,0,4"/><Point pos="4,0,0"/><Point pos="4,0,0"/><Point pos="0,0,4"/><Point pos="0,4,0"/></Segment>"#,
            ROOM,
        ));
        let volume = &found[0];
        assert!(volume.contains([0.5, 0.5, 0.5]));
        assert!(volume.contains([1.0, 1.0, 1.0]));
        assert!(!volume.contains([2.0, 2.0, 2.0])); // beyond the slanted face
        assert!(!volume.contains([-0.5, 0.5, 0.5]));
        assert!(!volume.contains([0.5, 0.5, -0.5]));
    }

    #[test]
    fn malformed_segments_are_dropped_not_guessed() {
        let found = volumes(&area(
            r#"Pos="0,0,0""#,
            concat!(
                r#"<Segment Type="Box" BoxMin="0,0" BoxMax="1,1,1"/>"#,
                r#"<Segment Type="Shape" Height="0"><Points><Point Pos="0,0,0"/><Point Pos="1,0,0"/><Point Pos="0,1,0"/></Points></Segment>"#,
                r#"<Segment Type="Trimesh"><Point pos="0,0,0"/><Point pos="1,0,0"/><Point pos="0,1,0"/></Segment>"#,
                r#"<Segment Type="Sphere" Radius="3"/>"#,
            ),
            ROOM,
        ));
        assert!(found.is_empty());
    }

    #[test]
    fn json_carries_kind_specific_fields() {
        let found = volumes(&area(
            r#"Pos="0,0,0""#,
            r#"<Segment transform_ls="Q : 1,0,0,0 | T : 0,0,5" Type="Box" BoxMin="1,0,0" BoxMax="0,1,1"/>"#,
            ROOM,
        ));
        let json = found[0].to_json();
        assert_eq!(json["kind"], "box");
        assert_eq!(json["room"], ROOM);
        assert_eq!(json["box_min"], serde_json::json!([0.0, 0.0, 0.0])); // min/max sorted per axis
        assert_eq!(json["box_max"], serde_json::json!([1.0, 1.0, 1.0]));
        assert_eq!(json["transform"][3][2], 5.0);
        assert!(json.get("vertices").is_none());
    }
}
