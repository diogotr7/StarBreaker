//! Vehicle XML parsing, landing gear queries, and invisible-port extraction.
//!
//! `query_landing_gear` reads landing gear geometry paths and bone names from a
//! `VehicleLandingGearSystem` DataCore record. `load_invisible_ports` reads the
//! vehicle XML override to find ports that should be hidden in the scene.
//! `load_vehicle_xml_parts` parses the vehicle XML into a flat list of `VehicleXmlPart`
//! structs, each describing a named geometry attachment with transform and flags.

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use super::datacore_path_to_p4k;


/// Query landing gear geometry paths + bone names from VehicleLandingGearSystem.
///
/// `VehicleComponentParams.landingSystem` is a Reference to a `VehicleLandingGearSystem` record.
/// That record has `gears[]`, each with a geometry path (.skin) and a bone name.
pub(crate) fn query_landing_gear(db: &Database, record: &Record) -> Vec<(String, String)> {
    // Step 1: Get the landingSystem reference (a Record pointer).
    use starbreaker_datacore::query::value::Value;
    let Ok(compiled) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[VehicleComponentParams].landingSystem",
    ) else {
        return Vec::new();
    };
    let Ok(values) = db.query::<Value>(&compiled, record) else {
        return Vec::new();
    };
    if values.is_empty() {
        return Vec::new();
    }
    // landingSystem is a Reference that auto-resolves to an Object containing gears[].
    let Value::Object { fields, .. } = &values[0] else {
        log::debug!("landing gear: landingSystem not an Object");
        return Vec::new();
    };
    let Some(Value::Array(gears_arr)) = fields.iter().find(|(k, _)| *k == "gears").map(|(_, v)| v) else {
        log::debug!("landing gear: no gears array");
        return Vec::new();
    };
    log::info!("landing gear: {} gears found", gears_arr.len());

    let mut parts = Vec::new();
    for gear in gears_arr {
        let Value::Object { fields, .. } = gear else { continue };
        let fields_map: std::collections::HashMap<&str, &Value> = fields.iter().map(|(k, v)| (*k, v)).collect();
        let bone = match fields_map.get("bone") {
            Some(Value::String(s)) => s.to_string(),
            _ => continue,
        };
        // geometry is GlobalResourceGeometry { path: "...cdf" } or
        // SGeometryResourceParams { Geometry: { Geometry: { path: "..." } } }
        let geom_path = (|| {
            let Value::Object { fields: gf, .. } = fields_map.get("geometry")? else { return None };
            let gf_map: std::collections::HashMap<&str, &Value> = gf.iter().map(|(k, v)| (*k, v)).collect();
            // Try direct path first (GlobalResourceGeometry)
            if let Some(Value::String(s)) = gf_map.get("path") {
                return Some(s.to_string());
            }
            // Try nested Geometry.Geometry.path (SGeometryResourceParams)
            let Value::Object { fields: g2, .. } = gf_map.get("Geometry")? else { return None };
            let g2_map: std::collections::HashMap<&str, &Value> = g2.iter().map(|(k, v)| (*k, v)).collect();
            let Value::Object { fields: g3, .. } = g2_map.get("Geometry")? else { return None };
            let g3_map: std::collections::HashMap<&str, &Value> = g3.iter().map(|(k, v)| (*k, v)).collect();
            match g3_map.get("path") {
                Some(Value::String(s)) => Some(s.to_string()),
                _ => None,
            }
        })();
        if let Some(path) = geom_path {
            if !path.is_empty() {
                parts.push((path, bone));
            }
        }
    }
    parts
}

/// Resolve and parse the .mtl material file for a mesh.
// ── Vehicle XML invisible-port extraction ────────────────────────────────────

/// Load the vehicle implementation XML(s) and return the set of port names
/// whose `<ItemPort>` has `flags` containing "invisible".
///
/// Returns an empty set for non-vehicle entities (no `VehicleComponentParams`).
pub fn load_invisible_ports(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
) -> std::collections::HashSet<String> {
    let mut invisible = std::collections::HashSet::new();

    super::vehicle_xml::visit_vehicle_xml(db, p4k, record, |xml, modification| {
        if modification {
            collect_invisible_ports_from_xml_override(xml, xml.root(), &mut invisible);
        } else {
            collect_invisible_ports_from_xml(xml, xml.root(), &mut invisible);
        }
    });

    if !invisible.is_empty() {
        log::info!("Vehicle XML: {} invisible ports", invisible.len());
        for p in &invisible {
            log::debug!("  invisible port: {p}");
        }
    }

    invisible
}

/// Walk a vehicle XML recursively collecting port names with `invisible` in flags.
fn collect_invisible_ports_from_xml(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    invisible: &mut std::collections::HashSet<String>,
) {
    let tag = xml.node_tag(node);

    if tag == "Part" {
        let part_name = xml
            .node_attributes(node)
            .find(|(k, _)| *k == "name")
            .map(|(_, v)| v)
            .unwrap_or("");

        // Check if this Part has an <ItemPort> child with invisible flags.
        for child in xml.node_children(node) {
            if xml.node_tag(child) == "ItemPort" {
                let flags = xml
                    .node_attributes(child)
                    .find(|(k, _)| *k == "flags")
                    .map(|(_, v)| v)
                    .unwrap_or("");
                if flags.split_whitespace().any(|f| f == "invisible") && !part_name.is_empty() {
                    invisible.insert(part_name.to_string());
                }
            }
        }
    }

    // Recurse into children.
    for child in xml.node_children(node) {
        collect_invisible_ports_from_xml(xml, child, invisible);
    }
}

/// Walk a modification XML, overriding the base invisible set.
/// If a port in the modification has `invisible`, add it. If it doesn't, remove it.
fn collect_invisible_ports_from_xml_override(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    invisible: &mut std::collections::HashSet<String>,
) {
    let tag = xml.node_tag(node);

    if tag == "Part" {
        let part_name = xml
            .node_attributes(node)
            .find(|(k, _)| *k == "name")
            .map(|(_, v)| v)
            .unwrap_or("");

        for child in xml.node_children(node) {
            if xml.node_tag(child) == "ItemPort" {
                let flags = xml
                    .node_attributes(child)
                    .find(|(k, _)| *k == "flags")
                    .map(|(_, v)| v)
                    .unwrap_or("");
                if !part_name.is_empty() {
                    if flags.split_whitespace().any(|f| f == "invisible") {
                        invisible.insert(part_name.to_string());
                    } else {
                        // Modification overrides base: port is NOT invisible.
                        invisible.remove(part_name);
                    }
                }
            }
        }
    }

    for child in xml.node_children(node) {
        collect_invisible_ports_from_xml_override(xml, child, invisible);
    }
}

// ── Vehicle XML Tread / Wheel part extraction ─────────────────────────────────

/// A geometry part extracted from a vehicle definition XML (treads, wheels).
/// These are ground-vehicle parts not represented in the DataCore loadout tree.
pub(crate) struct VehicleXmlPart {
    /// Part name (used as attachment point / bone name).
    pub name: String,
    /// Geometry file path (relative, DataCore-style).
    pub geometry_path: String,
    /// Material path from XML (may be empty).
    pub material_path: String,
    /// Optional XML sub-part geometry name (e.g. SubPartWheel geometryname="tyre").
    /// When present, this identifies which authored sub-part should align with
    /// the attachment helper.
    pub geometry_name: Option<String>,
    /// Child part names (for treads: the wheel part names attached to this tread).
    pub children: Vec<String>,
}

/// Extract Tread and SubPartWheel parts from a vehicle definition XML.
/// Returns a flat list of parts with parent-child relationships encoded in `children`.
pub fn load_vehicle_xml_parts(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
) -> Vec<VehicleXmlPart> {
    let veh_def = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[VehicleComponentParams].vehicleDefinition",
        )
        .ok()
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten());

    let veh_def = match veh_def {
        Some(ref s) if !s.is_empty() => s,
        _ => return Vec::new(),
    };

    let base_p4k = datacore_path_to_p4k(veh_def);
    let data = match p4k
        .entry_case_insensitive(&base_p4k)
        .and_then(|e| p4k.read(e).ok())
    {
        Some(d) => d,
        None => return Vec::new(),
    };

    let xml = match starbreaker_cryxml::from_bytes(&data) {
        Ok(x) => x,
        Err(_) => return Vec::new(),
    };

    let mut parts = Vec::new();
    collect_vehicle_parts(&xml, xml.root(), &mut parts);

    if !parts.is_empty() {
        log::info!("Vehicle XML: {} tread/wheel parts", parts.len());
        for p in &parts {
            log::debug!("  vehicle part: {} -> {} (children: {:?})", p.name, p.geometry_path, p.children);
        }
    }

    parts
}

/// Recursively walk vehicle XML collecting Tread and SubPartWheel parts.
fn collect_vehicle_parts(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    parts: &mut Vec<VehicleXmlPart>,
) {
    let tag = xml.node_tag(node);

    if tag == "Part" {
        let attrs: std::collections::HashMap<&str, &str> = xml.node_attributes(node).collect();
        let part_class = attrs.get("class").copied().unwrap_or("");
        let part_name = attrs.get("name").copied().unwrap_or("");

        if part_class == "Tread" {
            // Look for <Tread> child element
            for child in xml.node_children(node) {
                if xml.node_tag(child) != "Tread" {
                    continue;
                }
                let tread_attrs: std::collections::HashMap<&str, &str> =
                    xml.node_attributes(child).collect();
                let filename = tread_attrs.get("filename").copied().unwrap_or("");
                let material = tread_attrs.get("materialName").copied().unwrap_or("");

                // Collect wheel part names from <Wheels><Wheel partName="..."/></Wheels>
                let mut wheel_names = Vec::new();
                for tread_child in xml.node_children(child) {
                    if xml.node_tag(tread_child) == "Wheels" {
                        for wheel in xml.node_children(tread_child) {
                            if xml.node_tag(wheel) == "Wheel" {
                                if let Some((_, pn)) =
                                    xml.node_attributes(wheel).find(|(k, _)| *k == "partName")
                                {
                                    wheel_names.push(pn.to_string());
                                }
                            }
                        }
                    }
                }

                if !filename.is_empty() && !part_name.is_empty() {
                    parts.push(VehicleXmlPart {
                        name: part_name.to_string(),
                        geometry_path: filename.to_string(),
                        material_path: material.to_string(),
                        geometry_name: None,
                        children: wheel_names,
                    });
                }
            }
        } else if part_class == "SubPartWheel" {
            // Look for <SubPart> child element
            for child in xml.node_children(node) {
                if xml.node_tag(child) != "SubPart" {
                    continue;
                }
                if let Some((_, filename)) =
                    xml.node_attributes(child).find(|(k, _)| *k == "filename")
                {
                    if !filename.is_empty() && !part_name.is_empty() {
                        let geometry_name = xml
                            .node_attributes(child)
                            .find(|(k, _)| *k == "geometryname")
                            .map(|(_, value)| value.to_string())
                            .filter(|value| !value.is_empty());
                        parts.push(VehicleXmlPart {
                            name: part_name.to_string(),
                            geometry_path: filename.to_string(),
                            material_path: String::new(),
                            geometry_name,
                            children: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    for child in xml.node_children(node) {
        collect_vehicle_parts(xml, child, parts);
    }
}
