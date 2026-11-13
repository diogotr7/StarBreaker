//! Interior entity semantics resolved from DataCore: proximity sensors, entity
//! class animation controllers, and inline loadouts.
//!
//! - [`ProximitySensor`] / [`SensorResolver`]: an interior entity whose class
//!   carries `SCItemProximitySensorParams` (a door's walk-up trigger) becomes a
//!   container-level sensor record: its container-local transform, its trigger
//!   volume (`SensorShape`) and the ids its `EntityLinks` target (the door).
//!   Sensors are recognised by that component only, never by class name.
//! - [`mesh_animation_controller`]: the Mannequin controller of the entity
//!   class an interior entity was spawned from.
//! - [`inline_loadout_nodes`]: an entity's own manual loadout, resolved against
//!   its class's item ports (it replaces the class loadout).
//! - [`placement_identity`]: the id/class/links an interior placement carries.

use std::collections::HashMap;
use std::str::FromStr;

use starbreaker_common::CigGuid;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::loadout::{EntityIndex, LoadoutNode, ManualLoadoutEntry};
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::Record;

use crate::types::{InlineLoadoutEntry, InteriorMesh, PlacementIdentity};

use super::query_animation_controller_source;

/// The trigger volume of a proximity sensor, in the sensor entity's frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SensorVolume {
    /// `SCItemProximitySensorBoxParams`: `extent` and `offset` as authored.
    Box { extent: [f32; 3], offset: [f32; 3] },
}

/// A proximity sensor entity of an interior container.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProximitySensor {
    pub entity_id: Option<String>,
    pub entity_class: String,
    pub entity_class_guid: String,
    /// Container-local, the frame of the container's placements.
    pub transform: [[f32; 4]; 4],
    pub volume: SensorVolume,
    /// `exitDelay`: seconds after the last occupant leaves before the sensor
    /// reports the exit.
    pub exit_delay: Option<f32>,
    /// Ids the sensor's `EntityLinks` target (the entities it drives).
    pub links: Vec<String>,
}

impl ProximitySensor {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let volume = match &self.volume {
            SensorVolume::Box { extent, offset } => {
                serde_json::json!({ "kind": "box", "extent": extent, "offset": offset })
            }
        };
        serde_json::json!({
            "entity_id": self.entity_id,
            "entity_class": self.entity_class,
            "entity_class_guid": self.entity_class_guid,
            "transform": self.transform,
            "volume": volume,
            "exit_delay": self.exit_delay,
            "links": self.links,
        })
    }
}

fn field<'v, 'a>(value: &'v Value<'a>, name: &str) -> Option<&'v Value<'a>> {
    match value {
        Value::Object { fields, .. } => fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v),
        _ => None,
    }
}

fn number(value: &Value) -> Option<f32> {
    match value {
        Value::Float(v) => Some(*v),
        Value::Double(v) => Some(*v as f32),
        _ => None,
    }
}

fn vec3(value: &Value) -> Option<[f32; 3]> {
    Some([
        number(field(value, "x")?)?,
        number(field(value, "y")?)?,
        number(field(value, "z")?)?,
    ])
}

/// Volume and exit delay of an `SCItemProximitySensorParams` component.
/// `None` for a shape type this exporter does not know (logged by the caller).
pub(crate) fn sensor_volume(component: &Value) -> Option<(SensorVolume, Option<f32>)> {
    let shape = field(component, "SensorShape")?;
    let volume = match shape {
        Value::Object { type_name: "SCItemProximitySensorBoxParams", .. } => SensorVolume::Box {
            extent: vec3(field(shape, "Extent")?)?,
            offset: vec3(field(shape, "Offset")?)?,
        },
        _ => return None,
    };
    Some((volume, field(component, "exitDelay").and_then(number)))
}

fn class_record<'a>(db: &'a Database, mesh: &InteriorMesh) -> Option<&'a Record> {
    let guid = CigGuid::from_str(mesh.entity_class_guid.as_deref()?).ok()?;
    db.record_by_id(&guid)
}

/// Resolves interior entities to proximity sensors, caching per entity class.
pub(crate) struct SensorResolver<'a> {
    db: &'a Database<'a>,
    by_class: HashMap<String, Option<(String, SensorVolume, Option<f32>)>>,
}

impl<'a> SensorResolver<'a> {
    pub(crate) fn new(db: &'a Database<'a>) -> Self {
        Self { db, by_class: HashMap::new() }
    }

    /// The sensor `mesh` is, when its entity class has a proximity-sensor
    /// component with a known shape.
    pub(crate) fn sensor(&mut self, mesh: &InteriorMesh) -> Option<ProximitySensor> {
        let class_guid = mesh.entity_class_guid.as_ref()?;
        let db = self.db;
        let class = self
            .by_class
            .entry(class_guid.to_ascii_lowercase())
            .or_insert_with(|| {
                let record = class_record(db, mesh)?;
                let compiled = db
                    .compile_path::<Value>(record.struct_id(), "Components[SCItemProximitySensorParams]")
                    .ok()?;
                let component = db.query_single::<Value>(&compiled, record).ok().flatten()?;
                let name = db.resolve_string2(record.name_offset);
                let name = name.rsplit('.').next().unwrap_or(name).to_string();
                let Some((volume, exit_delay)) = sensor_volume(&component) else {
                    log::warn!("[interior] proximity sensor class {name}: unsupported SensorShape");
                    return None;
                };
                Some((name, volume, exit_delay))
            })
            .clone()?;
        let (entity_class, volume, exit_delay) = class;
        Some(ProximitySensor {
            entity_id: mesh.entity.entity_id.clone(),
            entity_class,
            entity_class_guid: class_guid.clone(),
            transform: mesh.transform,
            volume,
            exit_delay,
            links: mesh.entity.links.clone(),
        })
    }
}

/// The Mannequin controller of the class `mesh` was spawned from (by GUID or,
/// for root item-port entities, by class name).
pub(crate) fn mesh_animation_controller(
    db: &Database,
    mesh: &InteriorMesh,
) -> Option<crate::animation::AnimationControllerSource> {
    class_record(db, mesh)
        .or_else(|| {
            mesh.entity_class_name
                .as_deref()
                .and_then(|name| super::resolve_named_entity_record(db, name))
        })
        .and_then(|record| query_animation_controller_source(db, record))
        .filter(|source| !source.animation_database.is_empty())
}

/// The loadout attached to the entity `mesh`: its own inline manual loadout
/// when it authors one (resolved against its class's item ports), else its
/// class's default loadout. Empty when the entity has no class record.
pub(crate) fn entity_loadout_nodes(
    idx: &EntityIndex,
    db: &Database,
    mesh: &InteriorMesh,
) -> Vec<LoadoutNode> {
    let Some(record) = class_record(db, mesh) else {
        return Vec::new();
    };
    match &mesh.entity.inline_loadout {
        Some(entries) => inline_loadout_nodes(idx, record, entries),
        None => starbreaker_datacore::loadout::resolve_loadout_indexed(idx, record).root.children,
    }
}

fn inline_loadout_nodes(idx: &EntityIndex, parent: &Record, entries: &[InlineLoadoutEntry]) -> Vec<LoadoutNode> {
    starbreaker_datacore::loadout::resolve_manual_loadout_indexed(idx, parent, &manual_entries(entries))
}

pub(crate) fn manual_entries(entries: &[InlineLoadoutEntry]) -> Vec<ManualLoadoutEntry> {
    entries
        .iter()
        .map(|entry| ManualLoadoutEntry {
            item_port_name: entry.item_port_name.clone(),
            entity_class_name: entry.entity_class_name.clone(),
            entity_class_guid: entry
                .entity_class_guid
                .as_deref()
                .and_then(|guid| CigGuid::from_str(guid).ok()),
            children: entry.children.as_deref().map(manual_entries),
        })
        .collect()
}

/// Identity of the placement of `.soc` entity `mesh`.
pub(crate) fn placement_identity(mesh: &InteriorMesh) -> PlacementIdentity {
    PlacementIdentity {
        entity_id: mesh.entity.entity_id.clone(),
        entity_class_guid: mesh.entity_class_guid.clone(),
        links: mesh.entity.links.clone(),
        loadout_owner_entity_id: None,
        item_port_name: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(type_name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        Value::Object { type_name, fields, record_id: None }
    }

    fn vec3_value(x: f32, y: f32, z: f32) -> Value<'static> {
        object("Vec3", vec![("x", Value::Float(x)), ("y", Value::Float(y)), ("z", Value::Float(z))])
    }

    #[test]
    fn box_sensor_volume_is_read_as_authored() {
        let component = object(
            "SCItemProximitySensorParams",
            vec![
                ("exitDelay", Value::Float(0.25)),
                (
                    "SensorShape",
                    object(
                        "SCItemProximitySensorBoxParams",
                        vec![("Extent", vec3_value(1.0, 2.0, 3.0)), ("Offset", vec3_value(0.0, 0.5, 0.0))],
                    ),
                ),
            ],
        );
        assert_eq!(
            sensor_volume(&component),
            Some((SensorVolume::Box { extent: [1.0, 2.0, 3.0], offset: [0.0, 0.5, 0.0] }, Some(0.25)))
        );
    }

    #[test]
    fn unknown_or_missing_shape_is_not_a_volume() {
        let unknown = object(
            "SCItemProximitySensorParams",
            vec![("SensorShape", object("SCItemProximitySensorSyntheticParams", vec![]))],
        );
        assert_eq!(sensor_volume(&unknown), None);
        let null = object("SCItemProximitySensorParams", vec![("SensorShape", Value::Null)]);
        assert_eq!(sensor_volume(&null), None);
    }

    #[test]
    fn sensor_record_serialises_volume_and_links() {
        let sensor = ProximitySensor {
            entity_id: Some("sensor-id".into()),
            entity_class: "Synthetic_Sensor".into(),
            entity_class_guid: "class-guid".into(),
            transform: glam::Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0)).to_cols_array_2d(),
            volume: SensorVolume::Box { extent: [1.0, 2.0, 3.0], offset: [0.0; 3] },
            exit_delay: Some(0.5),
            links: vec!["door-id".into()],
        };
        let json = sensor.to_json();
        assert_eq!(json["volume"]["kind"], "box");
        assert_eq!(json["volume"]["extent"], serde_json::json!([1.0, 2.0, 3.0]));
        assert_eq!(json["links"], serde_json::json!(["door-id"]));
        assert_eq!(json["transform"][3], serde_json::json!([1.0, 2.0, 3.0, 1.0]));
        assert_eq!(json["entity_id"], "sensor-id");
    }

    #[test]
    fn inline_entries_convert_with_their_nesting() {
        let entries = vec![InlineLoadoutEntry {
            item_port_name: "port_a".into(),
            entity_class_name: String::new(),
            entity_class_guid: Some("dddd0000-0000-0000-0000-000000000004".into()),
            children: Some(vec![InlineLoadoutEntry {
                item_port_name: "nested".into(),
                entity_class_name: "Synthetic_Nested".into(),
                entity_class_guid: None,
                children: None,
            }]),
        }];
        let manual = manual_entries(&entries);
        assert_eq!(manual[0].item_port_name, "port_a");
        assert!(manual[0].entity_class_guid.is_some());
        let nested = manual[0].children.as_ref().unwrap();
        assert_eq!(nested[0].entity_class_name, "Synthetic_Nested");
        assert!(nested[0].children.is_none());
    }
}
