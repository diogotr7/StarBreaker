//! `.soc` entity extraction: class identity of inline-geometry entities, entity
//! ids and links, and geometry-less class entities (e.g. proximity sensors)
//! kept for DataCore resolution.
use super::*;

fn meshes(text: &str) -> Vec<InteriorMesh> {
    let bytes = crate::test_xml::encode(text);
    extract_cryxml_entities(&starbreaker_cryxml::from_bytes(&bytes).unwrap()).0
}

const DOOR_CLASS: &str = "11110000-0000-0000-0000-000000000001";
const SENSOR_CLASS: &str = "22220000-0000-0000-0000-000000000002";
const DOOR_ID: &str = "aaaa0000-0000-0000-0000-000000000001";
const SENSOR_ID: &str = "bbbb0000-0000-0000-0000-000000000002";

/// The sensor's class name carries the prefix the extractor used to drop by
/// name; whether an entity is a sensor is a DataCore question.
fn container() -> String {
    format!(
        r#"<Entities>
             <Entity Name="synthetic_door" Pos="1,2,3" EntityClass="Synthetic_Door"
                     EntityClassGUID="{DOOR_CLASS}" EntityCryGUID="{DOOR_ID}">
               <PropertiesDataCore>
                 <EntityGeometryResource>
                   <Geometry><Geometry><Geometry path="objects/synthetic/door.cga"/></Geometry></Geometry>
                 </EntityGeometryResource>
               </PropertiesDataCore>
             </Entity>
             <Entity Name="synthetic_sensor" Pos="4,5,6" Rotate="0.70710677,0,0,0.70710677"
                     EntityClass="Door_Ship_Sensor_Synthetic" EntityClassGUID="{SENSOR_CLASS}" EntityCryGUID="{SENSOR_ID}">
               <EntityLinks><Link TargetCryGUID="{DOOR_ID}" Name="NewLink"/></EntityLinks>
             </Entity>
           </Entities>"#
    )
}

#[test]
fn inline_geometry_entities_keep_their_entity_class_and_id() {
    let parsed = meshes(&container());
    let door = parsed.iter().find(|m| !m.cgf_path.is_empty()).expect("door mesh");
    assert_eq!(door.cgf_path, "objects/synthetic/door.cga");
    // The class carries the door's animation controller (Mannequin fragments)
    // and its loadout; inline geometry only overrides the class geometry.
    assert_eq!(door.entity_class_guid.as_deref(), Some(DOOR_CLASS));
    assert_eq!(door.entity.entity_id.as_deref(), Some(DOOR_ID));
}

#[test]
fn class_entities_without_inline_geometry_keep_transform_id_and_links() {
    let parsed = meshes(&container());
    let sensor = parsed.iter().find(|m| m.cgf_path.is_empty()).expect("sensor entity");
    assert_eq!(sensor.entity_class_guid.as_deref(), Some(SENSOR_CLASS));
    assert_eq!(sensor.entity.entity_id.as_deref(), Some(SENSOR_ID));
    assert_eq!(sensor.entity.links, [DOOR_ID]);
    assert_eq!(sensor.transform[3][..3], [4.0, 5.0, 6.0]);
    // 90 degrees about Z: local X maps to world Y.
    assert!((sensor.transform[0][1] - 1.0).abs() < 1e-5);
}
