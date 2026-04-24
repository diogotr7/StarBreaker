//! Socpak reader: opens ship interior containers from P4k, extracts geometry and lights.
//!
//! Flow: P4k → socpak (ZIP) → main .soc (CrCh) → IncludedObjects + CryXMLB → InteriorPayload

use std::collections::HashMap;

use crate::included_objects::IncludedObjects;
use starbreaker_chunks::ChunkFile;
use starbreaker_chunks::known_types::crch;
use starbreaker_cryxml::CryXml;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;
use starbreaker_p4k::{MappedP4k, P4kArchive};

use crate::error::Error;
use crate::types::{InteriorMesh, InteriorPayload, LightInfo, LightStateInfo};

// ── DataCore query ──────────────────────────────────────────────────────────

/// Container reference from DataCore VehicleComponentParams.objectContainers[].
#[derive(Debug, Clone)]
pub struct ObjectContainerRef {
    pub file_name: String,
    pub offset_position: [f32; 3],
    pub offset_rotation: [f32; 3], // Ang3 (degrees)
}

/// Query VehicleComponentParams.objectContainers from a ship entity record.
pub fn query_object_containers(
    db: &Database,
    record: &starbreaker_datacore::types::Record,
) -> Vec<ObjectContainerRef> {
    let Ok(path) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[VehicleComponentParams].objectContainers",
    ) else {
        return Vec::new();
    };

    let Ok(containers) = db.query::<Value>(&path, record) else {
        return Vec::new();
    };

    containers.iter().filter_map(parse_container_ref).collect()
}

fn parse_container_ref(val: &Value) -> Option<ObjectContainerRef> {
    let Value::Object { fields, .. } = val else {
        return None;
    };
    // Dump all fields for debugging
    for (k, v) in fields.iter() {
        log::debug!("  container field: {k} = {v:?}");
    }

    let fields: HashMap<&str, &Value> = fields.iter().map(|(k, v)| (*k, v)).collect();

    let file_name = match fields.get("fileName") {
        Some(Value::String(s)) => (*s).to_owned(),
        _ => return None,
    };
    let (offset_position, offset_rotation) = extract_offset(fields.get("Offset"));

    Some(ObjectContainerRef {
        file_name,
        offset_position,
        offset_rotation,
    })
}

fn extract_offset(offset_val: Option<&&Value>) -> ([f32; 3], [f32; 3]) {
    let mut pos = [0.0f32; 3];
    let mut rot = [0.0f32; 3];

    if let Some(Value::Object { fields, .. }) = offset_val {
        let fields: HashMap<&str, &Value> = fields.iter().map(|(k, v)| (*k, v)).collect();

        if let Some(Value::Object { fields: pf, .. }) = fields.get("Position") {
            let pf: HashMap<&str, &Value> = pf.iter().map(|(k, v)| (*k, v)).collect();
            if let Some(Value::Float(x)) = pf.get("x") {
                pos[0] = *x;
            }
            if let Some(Value::Float(y)) = pf.get("y") {
                pos[1] = *y;
            }
            if let Some(Value::Float(z)) = pf.get("z") {
                pos[2] = *z;
            }
        }
        if let Some(Value::Object { fields: rf, .. }) = fields.get("Rotation") {
            let rf: HashMap<&str, &Value> = rf.iter().map(|(k, v)| (*k, v)).collect();
            if let Some(Value::Float(x)) = rf.get("x") {
                rot[0] = *x;
            }
            if let Some(Value::Float(y)) = rf.get("y") {
                rot[1] = *y;
            }
            if let Some(Value::Float(z)) = rf.get("z") {
                rot[2] = *z;
            }
        }
    }
    (pos, rot)
}

// ── Socpak loading ──────────────────────────────────────────────────────────

/// Load a single socpak and extract its interior geometry + lights.
pub fn load_interior_from_socpak(
    p4k: &MappedP4k,
    socpak_path: &str,
    container_transform: [[f32; 4]; 4],
) -> Result<InteriorPayload, Error> {
    let p4k_path = normalize_socpak_path(socpak_path);

    let entry = p4k
        .entry_case_insensitive(&p4k_path)
        .ok_or_else(|| Error::MissingSocpak(p4k_path.clone()))?;

    let socpak_data = p4k
        .read(entry)
        .map_err(|e| Error::P4kRead(format!("{p4k_path}: {e}")))?;

    let inner = P4kArchive::from_bytes(&socpak_data)
        .map_err(|e| Error::P4kRead(format!("ZIP parse {p4k_path}: {e}")))?;

    let name = socpak_path
        .rsplit(&['/', '\\'])
        .next()
        .unwrap_or(socpak_path)
        .strip_suffix(".socpak")
        .unwrap_or(socpak_path)
        .to_string();

    // Parse ALL .soc files in the socpak (main + children).
    // The main .soc has IncludedObjects geometry; child .socs have lights and VFX entities.
    let soc_entries: Vec<_> = inner
        .entries()
        .iter()
        .filter(|e| e.name.to_lowercase().ends_with(".soc"))
        .collect();

    if soc_entries.is_empty() {
        return Err(Error::MissingSocpak(format!("No .soc in {p4k_path}")));
    }

    let mut meshes = Vec::new();
    let mut lights = Vec::new();
    let mut tint_palette_names = Vec::new();

    for soc_entry in &soc_entries {
        let soc_data = match inner.read(soc_entry) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("failed to read {}: {e}", soc_entry.name);
                continue;
            }
        };

        match parse_soc(&soc_data, &soc_entry.name, container_transform) {
            Ok((payload, palette_names)) => {
                log::debug!(
                    "  .soc '{}' → {} meshes, {} lights",
                    soc_entry.name,
                    payload.meshes.len(),
                    payload.lights.len()
                );
                meshes.extend(payload.meshes);
                lights.extend(payload.lights);
                if tint_palette_names.is_empty() {
                    tint_palette_names = palette_names;
                }
            }
            Err(e) => {
                log::warn!("failed to parse {}: {e}", soc_entry.name);
            }
        }
    }

    Ok(InteriorPayload {
        name,
        meshes,
        lights,
        container_transform,
        tint_palette_names,
    })
}

fn normalize_socpak_path(path: &str) -> String {
    let normalized = path.replace('/', "\\");
    if normalized.to_lowercase().starts_with("data\\") {
        normalized
    } else {
        format!("Data\\{normalized}")
    }
}

/// Parse a .soc file's CrCh chunks. Returns meshes/lights + tint palette names.
fn parse_soc(
    data: &[u8],
    name: &str,
    container_transform: [[f32; 4]; 4],
) -> Result<(InteriorPayload, Vec<String>), Error> {
    let cf =
        ChunkFile::from_bytes(data).map_err(|e| Error::ChunkParse(format!("{name}.soc: {e}")))?;

    let ChunkFile::CrCh(crch_file) = &cf else {
        return Err(Error::ChunkParse(format!(
            "{name}.soc: expected CrCh, got IVO"
        )));
    };

    let mut meshes = Vec::new();
    let mut lights = Vec::new();
    let mut palette_names = Vec::new();

    for chunk in crch_file.chunks() {
        let chunk_data = crch_file.chunk_data(chunk);

        match chunk.chunk_type {
            crch::INCLUDED_OBJECTS => match IncludedObjects::from_bytes(chunk_data) {
                Ok(io) => {
                    meshes.extend(included_objects_to_meshes(&io));
                    if palette_names.is_empty() {
                        palette_names = io.tint_palette_paths.clone();
                    }
                }
                Err(e) => log::warn!("failed to parse IncludedObjects in {name}: {e}"),
            },
            crch::CRYXMLB => match starbreaker_cryxml::from_bytes(chunk_data) {
                Ok(xml) => {
                    let (entity_meshes, entity_lights) = extract_cryxml_entities(&xml);
                    meshes.extend(entity_meshes);
                    lights.extend(entity_lights);
                }
                Err(e) => log::warn!("failed to parse CryXMLB in {name}: {e}"),
            },
            _ => {}
        }
    }

    Ok((InteriorPayload {
        name: name.to_string(),
        meshes,
        lights,
        container_transform,
        tint_palette_names: Vec::new(), // Set by caller from palette_names
    }, palette_names))
}

// ── IncludedObjects → InteriorMesh ──────────────────────────────────────────

fn included_objects_to_meshes(io: &IncludedObjects) -> Vec<InteriorMesh> {
    let material_path = io.material_paths.first().cloned();
    log::debug!(
        "  IncludedObjects: {} CGFs, {} objects, {} materials, {} palettes",
        io.cgf_paths.len(),
        io.objects.len(),
        io.material_paths.len(),
        io.tint_palette_paths.len()
    );
    for (i, path) in io.cgf_paths.iter().enumerate() {
        log::debug!("    CGF[{i}]: {path}");
    }
    for (i, path) in io.tint_palette_paths.iter().enumerate() {
        log::debug!("    Palette[{i}]: {path}");
    }
    for obj in &io.objects {
        let path = io
            .cgf_paths
            .get(obj.cgf_index as usize)
            .map(|s| s.as_str())
            .unwrap_or("??");
        let filename = path.rsplit('/').next().unwrap_or(path);
        log::debug!(
            "    obj: cgf_index={}, unknown2={:#x}, v1=[{:.1},{:.1},{:.1}], v2=[{:.1},{:.1},{:.1}] → {}",
            obj.cgf_index,
            obj.unknown2,
            obj.vector1[0],
            obj.vector1[1],
            obj.vector1[2],
            obj.vector2[0],
            obj.vector2[1],
            obj.vector2[2],
            filename
        );
    }

    io.objects
        .iter()
        .filter_map(|obj| {
            let cgf_path = io.cgf_paths.get(obj.cgf_index as usize)?.clone();
            let transform = f64_3x4_to_f32_4x4(&obj.transform);
            Some(InteriorMesh {
                cgf_path,
                material_path: material_path.clone(),
                transform,
                entity_class_guid: None,
            })
        })
        .collect()
}

fn f64_3x4_to_f32_4x4(m: &[[f64; 3]; 4]) -> [[f32; 4]; 4] {
    [
        [m[0][0] as f32, m[0][1] as f32, m[0][2] as f32, 0.0],
        [m[1][0] as f32, m[1][1] as f32, m[1][2] as f32, 0.0],
        [m[2][0] as f32, m[2][1] as f32, m[2][2] as f32, 0.0],
        [m[3][0] as f32, m[3][1] as f32, m[3][2] as f32, 1.0],
    ]
}

// ── CryXMLB entity extraction ───────────────────────────────────────────────

/// Entity classes to skip (non-visual game logic).
const SKIP_ENTITY_CLASSES: &[&str] = &[
    "ActionArea",
    "AudioAreaAmbience",
    "AudioEnvironmentFeedbackPoint",
    "AudioTriggerSpot",
    "AreaShape",
    "CameraSource",
    "ColorGradient",
    "CommentEntity",
    "EditorCamera",
    "EnvironmentLight",
    "FlographEntity",
    "FogVolume",
    "GravityArea",
    "GravityBox",
    "GreenZone",
    "Hazard",
    "Hint",
    "Ladder",
    "LandingArea",
    "LedgeObject",
    "LocationManager",
    "MusicArea",
    "NavigationArea",
    "ParticleEffect",
    "ParticleField",
    "PlanetAreaEntity",
    "ProceduralPointOfInterestProxy",
    "Room",
    "RoomConnector",
    "SafeTeleportPoint",
    "SCShop",
    "SequenceObjectItem",
    "ShadowRegionEntity",
    "SurfaceRaindropsTarget",
    "TagPoint",
    "TransitDestination",
    "TransitGateway",
    "TransitManager",
    "TransitNavSpline",
    "VibrationAudioPoint",
    "VehicleAudioPoint",
];

fn extract_cryxml_entities(xml: &CryXml) -> (Vec<InteriorMesh>, Vec<LightInfo>) {
    let mut meshes = Vec::new();
    let mut lights = Vec::new();

    let root = xml.root();
    let root_tag = xml.node_tag(root);

    // Find <Entities> or <SCOC_Entities> container
    let entities_node = xml.node_children(root).find(|child| {
        let tag = xml.node_tag(child);
        tag == "Entities" || tag == "SCOC_Entities"
    });

    if let Some(container) = entities_node {
        process_entity_children(xml, container, &mut meshes, &mut lights);
    } else if root_tag == "Entities" || root_tag == "SCOC_Entities" {
        process_entity_children(xml, root, &mut meshes, &mut lights);
    }

    (meshes, lights)
}

fn process_entity_children(
    xml: &CryXml,
    parent: &starbreaker_cryxml::CryXmlNode,
    meshes: &mut Vec<InteriorMesh>,
    lights: &mut Vec<LightInfo>,
) {
    for entity in xml.node_children(parent) {
        if xml.node_tag(entity) != "Entity" {
            continue;
        }

        let attrs: HashMap<&str, &str> = xml.node_attributes(entity).collect();
        let entity_class = attrs.get("EntityClass").copied().unwrap_or("");

        // Skip non-visual entities
        if SKIP_ENTITY_CLASSES.contains(&entity_class)
            || entity_class.starts_with("Door_Ship_Sensor")
            || entity_class.starts_with("ChipSet_Light")
        {
            continue;
        }

        // Log all attributes for debugging
        let entity_name = attrs.get("Name").copied().unwrap_or("?");
        log::trace!(
            "  CryXML entity: class={entity_class} name={entity_name} attrs={:?}",
            attrs.keys().collect::<Vec<_>>()
        );

        let pos = parse_csv_f64(attrs.get("Pos").copied().unwrap_or("0,0,0"));
        let rot = parse_csv_f64(attrs.get("Rotate").copied().unwrap_or("1,0,0,0"));
        let scale = parse_csv_f64(attrs.get("Scale").copied().unwrap_or("1,1,1"));

        if entity_class == "Light"
            || entity_class == "LightBox"
            || entity_class == "LightGroup"
            || entity_class == "LightGroupPoweredItem"
        {
            let parsed_lights = parse_light_entities(xml, entity, &attrs, &pos, &rot, entity_class);
            lights.extend(parsed_lights);
            continue;
        }

        // Try to extract geometry path from inline PropertiesDataCore
        let transform = pos_rot_scale_to_4x4(&pos, &rot, &scale);
        let material_path = attrs.get("Material").map(|s| s.to_string());

        if let Some(geom_path) = extract_entity_geometry(xml, entity) {
            meshes.push(InteriorMesh {
                cgf_path: geom_path,
                material_path,
                transform,
                entity_class_guid: None,
            });
        } else if let Some(guid) = attrs.get("EntityClassGUID") {
            // No inline geometry — resolve via DataCore using EntityClassGUID
            meshes.push(InteriorMesh {
                cgf_path: String::new(),
                material_path,
                transform,
                entity_class_guid: Some(guid.to_string()),
            });
        }
    }
}

fn extract_entity_geometry(
    xml: &CryXml,
    entity: &starbreaker_cryxml::CryXmlNode,
) -> Option<String> {
    // PropertiesDataCore → EntityGeometryResource → Geometry → Geometry → Geometry → @path
    for child in xml.node_children(entity) {
        if xml.node_tag(child) != "PropertiesDataCore" {
            continue;
        }
        for prop in xml.node_children(child) {
            if xml.node_tag(prop) != "EntityGeometryResource" {
                continue;
            }
            for g1 in xml.node_children(prop) {
                if xml.node_tag(g1) != "Geometry" {
                    continue;
                }
                for g2 in xml.node_children(g1) {
                    if xml.node_tag(g2) != "Geometry" {
                        continue;
                    }
                    for g3 in xml.node_children(g2) {
                        if xml.node_tag(g3) != "Geometry" {
                            continue;
                        }
                        let inner_attrs: HashMap<&str, &str> = xml.node_attributes(g3).collect();
                        if let Some(path) = inner_attrs.get("path")
                            && !path.is_empty()
                        {
                            return Some(path.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parse light properties from CryXML entity.
///
/// Handles both `Light` (single light in PropertiesDataCore/EntityComponentLight)
/// and `LightGroup` (multiple baked-in lights in EntityComponentLightGroup).
fn parse_light_entities(
    xml: &CryXml,
    entity: &starbreaker_cryxml::CryXmlNode,
    attrs: &HashMap<&str, &str>,
    pos: &[f64],
    rot: &[f64],
    entity_class: &str,
) -> Vec<LightInfo> {
    let base_name = attrs.get("Name").unwrap_or(&"Light").to_string();
    let base_pos = [
        pos.first().copied().unwrap_or(0.0),
        pos.get(1).copied().unwrap_or(0.0),
        pos.get(2).copied().unwrap_or(0.0),
    ];
    let base_rot = [
        rot.first().copied().unwrap_or(1.0),
        rot.get(1).copied().unwrap_or(0.0),
        rot.get(2).copied().unwrap_or(0.0),
        rot.get(3).copied().unwrap_or(0.0),
    ];

    if entity_class == "LightGroup" {
        // LightGroup: EntityComponentLightGroup > BakedInLights > Light[]
        // Each Light child has its own EntityComponentLight
        return parse_light_group(xml, entity, &base_name, &base_pos, &base_rot);
    }

    // Single Light: PropertiesDataCore > EntityComponentLight
    if let Some(lc) = find_entity_component_light(xml, entity) {
        if let Some(light) =
            build_light_info_from_component(xml, &lc, &base_name, &base_pos, &base_rot)
        {
            return vec![light];
        }
    }

    // Fallback: basic light with entity-level Radius
    let radius = attrs
        .get("Radius")
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(5.0);
    vec![LightInfo {
        name: base_name,
        position: base_pos,
        rotation: base_rot,
        color: [1.0, 0.95, 0.9],
        light_type: "Omni".to_string(),
        intensity: 200.0,
        radius,
        inner_angle: None,
        outer_angle: None,
        projector_texture: None,
        active_state: String::new(),
        states: std::collections::BTreeMap::new(),
    }]
}

/// Find PropertiesDataCore > EntityComponentLight in a CryXML entity.
fn find_entity_component_light<'a>(
    xml: &'a CryXml,
    entity: &'a starbreaker_cryxml::CryXmlNode,
) -> Option<&'a starbreaker_cryxml::CryXmlNode> {
    for child in xml.node_children(entity) {
        if xml.node_tag(child) != "PropertiesDataCore" {
            continue;
        }
        for prop in xml.node_children(child) {
            if xml.node_tag(prop) == "EntityComponentLight" {
                return Some(prop);
            }
        }
    }
    None
}

/// Parse a LightGroup entity with baked-in lights.
fn parse_light_group(
    xml: &CryXml,
    entity: &starbreaker_cryxml::CryXmlNode,
    base_name: &str,
    base_pos: &[f64; 3],
    base_rot: &[f64; 4],
) -> Vec<LightInfo> {
    let mut lights = Vec::new();

    // EntityComponentLightGroup > BakedInLights > Light[]
    // Also check direct children (PropertiesDataCore > EntityComponentLightGroup)
    for child in xml.node_children(entity) {
        let tag = xml.node_tag(child);
        let lg_node = if tag == "EntityComponentLightGroup" {
            child
        } else if tag == "PropertiesDataCore" {
            // Sometimes nested under PropertiesDataCore
            match xml
                .node_children(child)
                .find(|c| xml.node_tag(c) == "EntityComponentLightGroup")
            {
                Some(n) => n,
                None => continue,
            }
        } else {
            continue;
        };

        for baked in xml.node_children(lg_node) {
            if xml.node_tag(baked) != "BakedInLights" {
                continue;
            }
            let mut idx = 0;
            for light_node in xml.node_children(baked) {
                if xml.node_tag(light_node) != "Light" {
                    continue;
                }
                let light_name = format!("{base_name}-{idx:03}");

                // Each baked-in Light node has a RelativeXForm child with
                // per-light translation/rotation offsets relative to the group.
                let (rel_translation, rel_rotation) =
                    extract_relative_xform(xml, light_node);
                let rel_translation_world =
                    quat_rotate_vec(base_rot, &rel_translation);

                // Combine group position with per-light offset
                let light_pos = [
                    base_pos[0] + rel_translation_world[0],
                    base_pos[1] + rel_translation_world[1],
                    base_pos[2] + rel_translation_world[2],
                ];
                let light_rot = quat_mul(base_rot, &rel_rotation);

                // Each Light node has its own EntityComponentLight child
                for lc_child in xml.node_children(light_node) {
                    if xml.node_tag(lc_child) == "EntityComponentLight" {
                        if let Some(light) = build_light_info_from_component(
                            xml,
                            lc_child,
                            &light_name,
                            &light_pos,
                            &light_rot,
                        ) {
                            lights.push(light);
                        }
                        break;
                    }
                }
                idx += 1;
            }
        }
    }

    if lights.is_empty() {
        // Fallback
        lights.push(LightInfo {
            name: base_name.to_string(),
            position: *base_pos,
            rotation: *base_rot,
            color: [1.0, 0.95, 0.9],
            light_type: "Omni".to_string(),
            intensity: 200.0,
            radius: 5.0,
            inner_angle: None,
            outer_angle: None,
            projector_texture: None,
            active_state: String::new(),
            states: std::collections::BTreeMap::new(),
        });
    }
    lights
}

/// Build a LightInfo from an EntityComponentLight CryXML node.
///
/// Expected structure:
/// ```xml
/// <EntityComponentLight lightType="..." intensity="..." radius="..." color="r,g,b" ...>
///   <projectorParams texture="..." fov="..." nearPlane="..." />
///   <colorParams colorTemperature="..." />
///   <optionalParams .../>
/// </EntityComponentLight>
/// ```
fn build_light_info_from_component(
    xml: &CryXml,
    component: &starbreaker_cryxml::CryXmlNode,
    name: &str,
    pos: &[f64; 3],
    rot: &[f64; 4],
) -> Option<LightInfo> {
    // The EntityComponentLight carries top-level fields (lightType,
    // useTemperature, etc.) directly. State-specific values (intensity,
    // temperature, and the <color r g b> child element) live on dedicated
    // state children: offState / defaultState / auxiliaryState /
    // emergencyState / cinematicState. Star Citizen renders the "default"
    // state for baked-in lights, so we read from <defaultState> only.
    let component_attrs: HashMap<&str, &str> = xml
        .node_attributes(component)
        .filter(|(k, _)| *k != "__type")
        .collect();

    let bool_truthy = |s: &str| matches!(s, "1" | "true" | "True" | "TRUE");
    let use_temperature = component_attrs
        .get("useTemperature")
        .map(|s| bool_truthy(s))
        .unwrap_or(false);
    let light_type = component_attrs
        .get("lightType")
        .copied()
        .unwrap_or("Omni")
        .to_string();

    // CryEngine light components expose several runtime states
    // (`offState` / `defaultState` / `auxiliaryState` / `emergencyState` /
    // `cinematicState`). Each carries its own intensity, temperature, and
    // <color r g b> child. Collect every authored state so downstream
    // tools can switch between them; then pick the first with
    // intensity > 0 (in fallback order) as the active state to expose on
    // the flat `color` / `intensity` fields.
    const ALL_STATES: &[&str] = &[
        "offState",
        "defaultState",
        "auxiliaryState",
        "emergencyState",
        "cinematicState",
    ];
    const STATE_PRIORITY: &[&str] = &[
        "defaultState",
        "auxiliaryState",
        "emergencyState",
        "cinematicState",
    ];

    let read_state = |tag: &str| -> Option<LightStateInfo> {
        let node = xml
            .node_children(component)
            .find(|c| xml.node_tag(c) == tag)?;
        let a: HashMap<&str, &str> = xml
            .node_attributes(node)
            .filter(|(k, _)| *k != "__type")
            .collect();
        let intensity_raw = a
            .get("intensity")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        let temperature = a
            .get("temperature")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(6500.0);
        let (cr, cg, cb) = xml
            .node_children(node)
            .find(|c| xml.node_tag(c) == "color")
            .map(|c| {
                let ca: HashMap<&str, &str> = xml
                    .node_attributes(c)
                    .filter(|(k, _)| *k != "__type")
                    .collect();
                let f = |k: &str| {
                    ca.get(k)
                        .and_then(|s| s.parse::<f32>().ok())
                        .unwrap_or(1.0)
                        .clamp(0.0, 1.0)
                };
                (f("r"), f("g"), f("b"))
            })
            .unwrap_or((1.0, 1.0, 1.0));
        Some(LightStateInfo {
            intensity_raw,
            intensity_cd: intensity_raw * 200.0,
            temperature,
            use_temperature,
            color: [cr, cg, cb],
        })
    };

    let mut states: std::collections::BTreeMap<String, LightStateInfo> =
        std::collections::BTreeMap::new();
    for tag in ALL_STATES {
        if let Some(s) = read_state(tag) {
            states.insert((*tag).to_string(), s);
        }
    }

    // Pick the active state via priority order.
    let active_state_name = STATE_PRIORITY
        .iter()
        .find(|tag| {
            states
                .get(**tag)
                .map(|s| s.intensity_raw > 0.0)
                .unwrap_or(false)
        })
        .copied()
        .unwrap_or("");
    let active = states.get(active_state_name);
    let intensity = active.map(|s| s.intensity_raw).unwrap_or(0.0);
    let temperature = active.map(|s| s.temperature).unwrap_or(6500.0);
    let (color_r, color_g, color_b) = active
        .map(|s| (s.color[0], s.color[1], s.color[2]))
        .unwrap_or((1.0, 1.0, 1.0));

    let color = if use_temperature {
        kelvin_to_rgb(temperature.clamp(1000.0, 40000.0))
    } else {
        [color_r, color_g, color_b]
    };

    // sizeParams > lightRadius (attenuation radius).
    let radius = xml
        .node_children(component)
        .find(|c| xml.node_tag(c) == "sizeParams")
        .and_then(|sp| {
            xml.node_attributes(sp)
                .find(|(k, _)| *k == "lightRadius")
                .and_then(|(_, v)| v.parse::<f32>().ok())
        })
        .filter(|r| *r > 0.0)
        .unwrap_or(5.0);

    // projectorParams > texture, FOV
    let (projector_texture, fov) = xml
        .node_children(component)
        .find(|c| xml.node_tag(c) == "projectorParams")
        .map(|pp| {
            let a: HashMap<&str, &str> = xml
                .node_attributes(pp)
                .filter(|(k, _)| *k != "__type")
                .collect();
            let tex = a
                .get("texture")
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            let fov = a
                .get("FOV")
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.0);
            (tex, fov)
        })
        .unwrap_or((None, 0.0));

    // Spot light half-angles
    let (inner_angle, outer_angle) = if light_type == "Projector" && fov > 0.0 {
        let outer = fov * 0.5;
        let inner = outer * 0.8;
        (Some(inner), Some(outer))
    } else {
        (None, None)
    };

    // CryEngine intensity → glTF candela.
    let candela = intensity * 200.0;

    log::debug!(
        "  Light '{name}' type={light_type} useTemp={use_temperature} \
         temperature={temperature} intensity={intensity} radius={radius} color={color:?}"
    );

    Some(LightInfo {
        name: name.to_string(),
        position: *pos,
        rotation: *rot,
        color,
        light_type,
        intensity: candela,
        radius,
        inner_angle,
        outer_angle,
        projector_texture,
        active_state: active_state_name.to_string(),
        states,
    })
}

/// Convert color temperature in Kelvin to linear sRGB [r, g, b] (0-1).
/// Uses Tanner Helland's algorithm (fast approximation).
fn kelvin_to_rgb(kelvin: f32) -> [f32; 3] {
    let temp = kelvin / 100.0;
    let r = if temp <= 66.0 {
        1.0
    } else {
        let x = temp - 60.0;
        (329.698727446 * x.powf(-0.1332047592) / 255.0).clamp(0.0, 1.0)
    };
    let g = if temp <= 66.0 {
        let x = temp;
        (99.4708025861 * x.ln() - 161.1195681661).clamp(0.0, 255.0) / 255.0
    } else {
        let x = temp - 60.0;
        (288.1221695283 * x.powf(-0.0755148492) / 255.0).clamp(0.0, 1.0)
    };
    let b = if temp >= 66.0 {
        1.0
    } else if temp <= 19.0 {
        0.0
    } else {
        let x = temp - 10.0;
        (138.5177312231 * x.ln() - 305.0447927307).clamp(0.0, 255.0) / 255.0
    };
    [r, g, b]
}

/// Extract translation and rotation from a baked-in Light node's RelativeXForm child.
/// Returns `([tx,ty,tz], [qw,qx,qy,qz])` — identity if absent.
fn extract_relative_xform(
    xml: &CryXml,
    light_node: &starbreaker_cryxml::CryXmlNode,
) -> ([f64; 3], [f64; 4]) {
    for child in xml.node_children(light_node) {
        if xml.node_tag(child) != "RelativeXForm" {
            continue;
        }
        let attrs: HashMap<&str, &str> = xml.node_attributes(child).collect();
        let translation = parse_csv_f64(attrs.get("translation").copied().unwrap_or("0,0,0"));
        let rotation = parse_csv_f64(attrs.get("rotation").copied().unwrap_or("1,0,0,0"));
        return (
            [
                translation.first().copied().unwrap_or(0.0),
                translation.get(1).copied().unwrap_or(0.0),
                translation.get(2).copied().unwrap_or(0.0),
            ],
            [
                rotation.first().copied().unwrap_or(1.0),
                rotation.get(1).copied().unwrap_or(0.0),
                rotation.get(2).copied().unwrap_or(0.0),
                rotation.get(3).copied().unwrap_or(0.0),
            ],
        );
    }
    ([0.0; 3], [1.0, 0.0, 0.0, 0.0])
}

// ── Math helpers ────────────────────────────────────────────────────────────

/// Multiply two quaternions [w, x, y, z].
fn quat_mul(a: &[f64; 4], b: &[f64; 4]) -> [f64; 4] {
    let (aw, ax, ay, az) = (a[0], a[1], a[2], a[3]);
    let (bw, bx, by, bz) = (b[0], b[1], b[2], b[3]);
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}

fn quat_rotate_vec(rotation: &[f64; 4], vector: &[f64; 3]) -> [f64; 3] {
    let quat = glam::DQuat::from_xyzw(rotation[1], rotation[2], rotation[3], rotation[0]);
    let rotated = quat * glam::DVec3::new(vector[0], vector[1], vector[2]);
    [rotated.x, rotated.y, rotated.z]
}

fn parse_csv_f64(s: &str) -> Vec<f64> {
    s.split(',')
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect()
}

/// Build a 4×4 column-major transform from position `[x,y,z]`, quaternion `[w,x,y,z]`, and scale `[x,y,z]`.
fn pos_rot_scale_to_4x4(pos: &[f64], rot: &[f64], scale: &[f64]) -> [[f32; 4]; 4] {
    let w = rot.first().copied().unwrap_or(1.0) as f32;
    let x = rot.get(1).copied().unwrap_or(0.0) as f32;
    let y = rot.get(2).copied().unwrap_or(0.0) as f32;
    let z = rot.get(3).copied().unwrap_or(0.0) as f32;
    let tx = pos.first().copied().unwrap_or(0.0) as f32;
    let ty = pos.get(1).copied().unwrap_or(0.0) as f32;
    let tz = pos.get(2).copied().unwrap_or(0.0) as f32;
    let sx = scale.first().copied().unwrap_or(1.0) as f32;
    let sy = scale.get(1).copied().unwrap_or(1.0) as f32;
    let sz = scale.get(2).copied().unwrap_or(1.0) as f32;

    let m = glam::Mat4::from_scale_rotation_translation(
        glam::Vec3::new(sx, sy, sz),
        glam::Quat::from_xyzw(x, y, z, w),
        glam::Vec3::new(tx, ty, tz),
    );
    m.to_cols_array_2d()
}


/// Build a 4×4 container transform from position offset and Ang3 rotation (degrees).
pub fn build_container_transform(pos: [f32; 3], rot_deg: [f32; 3]) -> [[f32; 4]; 4] {
    let px = rot_deg[0].to_radians();
    let py = rot_deg[1].to_radians();
    let pz = rot_deg[2].to_radians();
    let (sx, cx) = px.sin_cos();
    let (sy, cy) = py.sin_cos();
    let (sz, cz) = pz.sin_cos();

    // CryEngine Euler rotation order: Z * Y * X (yaw * pitch * roll)
    [
        [cy * cz, cy * sz, -sy, 0.0],
        [sx * sy * cz - cx * sz, sx * sy * sz + cx * cz, sx * cy, 0.0],
        [cx * sy * cz + sx * sz, cx * sy * sz - sx * cz, cx * cy, 0.0],
        [pos[0], pos[1], pos[2], 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::{quat_mul, quat_rotate_vec};

    fn approx_eq3(left: [f64; 3], right: [f64; 3]) {
        for index in 0..3 {
            assert!(
                (left[index] - right[index]).abs() < 1e-9,
                "component {} mismatch: left={} right={}",
                index,
                left[index],
                right[index]
            );
        }
    }

    #[test]
    fn light_group_relative_translation_respects_group_rotation() {
        let half_turn = std::f64::consts::FRAC_1_SQRT_2;
        let base_rotation = [half_turn, 0.0, 0.0, half_turn];
        let rel_translation = [5.0, 0.0, 0.0];
        let rotated = quat_rotate_vec(&base_rotation, &rel_translation);

        approx_eq3(rotated, [0.0, 5.0, 0.0]);
    }

    #[test]
    fn light_group_relative_rotation_still_composes_after_translation_fix() {
        let half_turn = std::f64::consts::FRAC_1_SQRT_2;
        let base_rotation = [half_turn, 0.0, 0.0, half_turn];
        let rel_rotation = [half_turn, half_turn, 0.0, 0.0];

        let combined = quat_mul(&base_rotation, &rel_rotation);

        approx_eq3([combined[1], combined[2], combined[3]], [0.5, 0.5, 0.5]);
        assert!((combined[0] - 0.5).abs() < 1e-9);
    }
}
