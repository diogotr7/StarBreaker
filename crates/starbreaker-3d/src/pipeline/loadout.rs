//! Loadout resolution and attachment-tree flattening.
//!
//! `resolve_loadout_meshes` is the public entry point: given a DataCore entity record
//! and its loadout tree, it resolves all child attachment geometries into a flat
//! `ResolvedLoadout`. `resolve_children` and `flatten_resolved_tree` walk the
//! loadout tree recursively. `expand_loadout_into_placements` converts the resolved
//! tree into interior-style CGF placements for decomposed export.

use std::collections::{HashMap, HashSet};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;

use super::*;

fn vehicle_subpart_anchor_offset(
    p4k: &MappedP4k,
    geometry_path: &str,
    geometry_name: Option<&str>,
) -> [f32; 3] {
    let Some(geometry_name) = geometry_name else {
        return [0.0; 3];
    };
    if geometry_name.is_empty() {
        return [0.0; 3];
    }

    let Some(nmc) = load_nmc_for_cgf(p4k, geometry_path) else {
        return [0.0; 3];
    };
    let world_transforms = compute_nmc_world_transforms(&nmc);
    let target = geometry_name.to_ascii_lowercase();
    let node_index = nmc.nodes.iter().position(|node| {
        let name = node.name.to_ascii_lowercase();
        name == target || name.ends_with(&format!("_{target}"))
    });
    let Some(node_index) = node_index else {
        return [0.0; 3];
    };

    let (_, _, translation) = world_transforms[node_index].to_scale_rotation_translation();
    [-translation.x, -translation.y, -translation.z]
}

pub(crate) fn flatten_resolved_tree(
    children: &[crate::types::ResolvedNode],
    parent_entity_name: &str,
    override_attachment: Option<(&str, bool)>,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
    out: &mut Vec<crate::types::EntityPayload>,
) {
    let mut specs = Vec::new();
    collect_child_payload_specs(children, parent_entity_name, override_attachment, &mut specs);
    out.extend(load_child_payloads(
        specs,
        db,
        p4k,
        mesh_opts,
        final_material_mode,
        existing_asset_paths,
    ));
}

// ── Shared loadout resolution ────────────────────────────────────────────────

/// Resolve an entire loadout tree into a lightweight metadata tree.
/// Loads NMC and probes for geometry existence, but does NOT load mesh vertex data.
/// Consumers (preview/full export) load meshes on demand while walking the tree.
pub fn resolve_loadout_meshes(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
    opts: &ExportOptions,
) -> Result<crate::types::ResolvedNode, Error> {
    // Resolve root geometry path to check it exists.
    let geom_compiled = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
        )
        .map_err(|e| Error::DataCoreQuery(e))?;
    let geometry_path = db
        .query_single::<String>(&geom_compiled, record)?
        .ok_or_else(|| Error::NoGeometryComponent {
            record_name: db.resolve_string2(record.name_offset).to_string(),
        })?;

    let mtl_compiled = db.compile_path::<String>(
        record.struct_id(),
        "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
    ).ok();
    let material_path = mtl_compiled
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten())
        .unwrap_or_default();

    let (geometry_path, material_path) = super::geometry_variant::resolve(
        db, record, opts.geometry_tag.as_deref(), geometry_path, material_path,
    )?;

    // Load NMC + skeleton from .cga/.chr
    let (nmc, _mtl) = load_nmc_and_material(p4k, &geometry_path, &material_path);

    let resolved = resolve_geometry_files(p4k, &geometry_path)?;
    let bones = load_skeleton(p4k, resolved.skeleton_path.as_deref(), &resolved.parts[0].path);

    // Check if mesh companion exists.
    // CDF files don't have companion files — check for the CDF itself.
    let p4k_geom_path = datacore_path_to_p4k(&geometry_path);
    let has_geometry = if geometry_path.to_lowercase().ends_with(".cdf") {
        p4k.entry_case_insensitive(&p4k_geom_path).is_some()
    } else {
        let companion = resolve_companion_path(p4k, &p4k_geom_path, opts.lod_level);
        p4k.entry_case_insensitive(&companion).is_some()
    };

    // Load invisible port flags from vehicle XML (empty for non-vehicles).
    let invisible_ports = load_invisible_ports(db, p4k, record);

    let mut children = resolve_children(db, p4k, &tree.root.children, opts, &invisible_ports);

    // Load Tread/Wheel parts from vehicle definition XML (ground vehicles).
    // These are geometry parts not represented in the DataCore loadout tree.
    let veh_parts = load_vehicle_xml_parts(db, p4k, record);
    if !veh_parts.is_empty() {
        // Skip parts whose names already appear in the loadout children to avoid duplication.
        let existing_names: std::collections::HashSet<String> = children
            .iter()
            .map(|c| c.attachment_name.to_lowercase())
            .collect();

        // Build a lookup from part name → VehicleXmlPart for child wheel attachment.
        let wheel_lookup: HashMap<String, &VehicleXmlPart> = veh_parts
            .iter()
            .filter(|p| p.children.is_empty()) // wheels have no children
            .map(|p| (p.name.to_lowercase(), p))
            .collect();
        let mut wheel_offset_cache: HashMap<(String, String), [f32; 3]> = HashMap::new();

        for part in &veh_parts {
            if existing_names.contains(&part.name.to_lowercase()) {
                log::debug!("  vehicle part '{}' already in loadout, skipping", part.name);
                continue;
            }
            // Only add tread parts here; their wheel children are attached below.
            if part.children.is_empty() && veh_parts.iter().any(|p| p.children.iter().any(|c| c.eq_ignore_ascii_case(&part.name))) {
                continue; // wheel part — will be attached as child of its tread
            }
            let p4k_path = datacore_path_to_p4k(&part.geometry_path);
            // CDF files don't have companion files — check for the CDF itself.
            // For CGA/CGF, check the companion (.cgam/.cgfm).
            let part_has_geom = if part.geometry_path.to_lowercase().ends_with(".cdf") {
                p4k.entry_case_insensitive(&p4k_path).is_some()
            } else {
                let companion = resolve_companion_path(p4k, &p4k_path, opts.lod_level);
                p4k.entry_case_insensitive(&companion).is_some()
            };
            log::debug!("  vehicle part '{}' has_geometry={} path={}", part.name, part_has_geom, p4k_path);
            let part_offset = if let Some(geometry_name) = part.geometry_name.as_deref() {
                let cache_key = (p4k_path.to_ascii_lowercase(), geometry_name.to_ascii_lowercase());
                if let Some(cached) = wheel_offset_cache.get(&cache_key).copied() {
                    cached
                } else {
                    let computed =
                        vehicle_subpart_anchor_offset(p4k, &part.geometry_path, Some(geometry_name));
                    wheel_offset_cache.insert(cache_key, computed);
                    computed
                }
            } else {
                [0.0; 3]
            };

            // Resolve wheel children for treads.
            let mut part_children = Vec::new();
            for wheel_name in &part.children {
                if let Some(wheel) = wheel_lookup.get(&wheel_name.to_lowercase()) {
                    let wheel_p4k = datacore_path_to_p4k(&wheel.geometry_path);
                    let wheel_has_geom = if wheel.geometry_path.to_lowercase().ends_with(".cdf") {
                        p4k.entry_case_insensitive(&wheel_p4k).is_some()
                    } else {
                        let wheel_companion = resolve_companion_path(p4k, &wheel_p4k, opts.lod_level);
                        p4k.entry_case_insensitive(&wheel_companion).is_some()
                    };
                    let wheel_offset = if let Some(geometry_name) = wheel.geometry_name.as_deref() {
                        let cache_key = (wheel_p4k.to_ascii_lowercase(), geometry_name.to_ascii_lowercase());
                        if let Some(cached) = wheel_offset_cache.get(&cache_key).copied() {
                            cached
                        } else {
                            let computed = vehicle_subpart_anchor_offset(
                                p4k,
                                &wheel_p4k,
                                Some(geometry_name),
                            );
                            wheel_offset_cache.insert(cache_key, computed);
                            computed
                        }
                    } else {
                        [0.0; 3]
                    };

                    part_children.push(crate::types::ResolvedNode {
                        entity_name: wheel.name.clone(),
                        attachment_name: wheel.name.clone(),
                        no_rotation: false,
                        offset_position: wheel_offset,
                        offset_rotation: [0.0; 3],
                        detach_direction: [0.0; 3],
                        port_flags: String::new(),
                        nmc: None,
                        bones: Vec::new(),
                        has_geometry: wheel_has_geom,
                        record: *record,
                        geometry_path: Some(wheel.geometry_path.clone()),
                        material_path: if wheel.material_path.is_empty() {
                            None
                        } else {
                            Some(wheel.material_path.clone())
                        },
                        allows_child_object_containers: false,
                        children: Vec::new(),
                    });
                }
            }

            children.push(crate::types::ResolvedNode {
                entity_name: part.name.clone(),
                attachment_name: part.name.clone(),
                no_rotation: false,
                offset_position: part_offset,
                offset_rotation: [0.0; 3],
                detach_direction: [0.0; 3],
                port_flags: String::new(),
                nmc: None,
                bones: Vec::new(),
                has_geometry: part_has_geom,
                record: *record,
                geometry_path: Some(part.geometry_path.clone()),
                material_path: if part.material_path.is_empty() {
                    None
                } else {
                    Some(part.material_path.clone())
                },
                allows_child_object_containers: false,
                children: part_children,
            });
        }
    }

    Ok(crate::types::ResolvedNode {
        entity_name: tree.root.entity_name.clone(),
        attachment_name: String::new(),
        no_rotation: false,
        offset_position: [0.0; 3],
        offset_rotation: [0.0; 3],
        detach_direction: [0.0; 3],
        port_flags: String::new(),
        nmc,
        bones,
        has_geometry,
        record: *record,
        geometry_path: Some(geometry_path),
        material_path: Some(material_path),
        allows_child_object_containers: true,
        children,
    })
}

pub(crate) fn resolve_children(
    db: &Database,
    p4k: &MappedP4k,
    nodes: &[starbreaker_datacore::loadout::LoadoutNode],
    opts: &ExportOptions,
    invisible_ports: &std::collections::HashSet<String>,
) -> Vec<crate::types::ResolvedNode> {
    use rayon::prelude::*;

    nodes
        .par_iter()
        .map(|node| {
            let attachment_name = node
                .helper_bone_name
                .clone()
                .unwrap_or_else(|| node.item_port_name.clone());

            if !opts.include_shields
                && is_shield_related_name(&node.entity_name)
                    || !opts.include_shields
                        && is_shield_related_name(&attachment_name)
                    || !opts.include_shields
                        && path_is_shield_related(node.geometry_path.as_deref())
                    || !opts.include_shields
                        && path_is_shield_related(node.material_path.as_deref())
            {
                log::info!(
                    "  {} -> shield export disabled, skipping geometry and children",
                    node.entity_name
                );
                return crate::types::ResolvedNode {
                    entity_name: node.entity_name.clone(),
                    attachment_name,
                    no_rotation: node.no_rotation,
                    offset_position: node.offset_position,
                    offset_rotation: node.offset_rotation,
                    detach_direction: node.detach_direction,
                    port_flags: node.port_flags.clone(),
                    nmc: None,
                    bones: Vec::new(),
                    has_geometry: false,
                    record: node.record,
                    geometry_path: None,
                    material_path: node.material_path.clone(),
                    allows_child_object_containers: true,
                    children: Vec::new(),
                };
            }

            // Skip geometry (and entire subtree) for ports marked invisible in the vehicle XML.
            let port_invisible = invisible_ports.contains(&node.item_port_name);

            if port_invisible {
                log::info!("  {} -> invisible port '{}', skipping geometry and children", node.entity_name, node.item_port_name);
                return crate::types::ResolvedNode {
                    entity_name: node.entity_name.clone(),
                    attachment_name,
                    no_rotation: node.no_rotation,
                    offset_position: node.offset_position,
                    offset_rotation: node.offset_rotation,
                    detach_direction: node.detach_direction,
                    port_flags: node.port_flags.clone(),
                    nmc: None,
                    bones: Vec::new(),
                    has_geometry: false,
                    record: node.record,
                    geometry_path: None,
                    material_path: node.material_path.clone(),
                    allows_child_object_containers: true,
                    children: Vec::new(),
                };
            }

            let children = resolve_children(db, p4k, &node.children, opts, invisible_ports);

            let Some(geom_path) = &node.geometry_path else {
                return crate::types::ResolvedNode {
                    entity_name: node.entity_name.clone(),
                    attachment_name,
                    no_rotation: node.no_rotation,
                    offset_position: node.offset_position,
                    offset_rotation: node.offset_rotation,
                    detach_direction: node.detach_direction,
                    port_flags: node.port_flags.clone(),
                    nmc: None,
                    bones: Vec::new(),
                    has_geometry: false,
                    record: node.record,
                    geometry_path: None,
                    material_path: node.material_path.clone(),
                    allows_child_object_containers: true,
                    children,
                };
            };

            // Load NMC from .cga (always, even if .cgam is missing).
            let mat_path = node.material_path.as_deref().unwrap_or("");
            let (nmc, _mtl) = load_nmc_and_material(p4k, geom_path, mat_path);

            // Probe whether mesh companion exists.
            // CDF files don't have companion files — check for the CDF itself.
            let p4k_geom_path = datacore_path_to_p4k(geom_path);
            let (has_geometry, metadata_only_geometry) = if geom_path.to_lowercase().ends_with(".cdf") {
                (p4k.entry_case_insensitive(&p4k_geom_path).is_some(), false)
            } else {
                let companion = resolve_companion_path(p4k, &p4k_geom_path, opts.lod_level);
                let companion_exists = p4k.entry_case_insensitive(&companion).is_some();
                let metadata_only = !companion_exists
                    && p4k.entry_case_insensitive(&p4k_geom_path).is_some();
                (companion_exists, metadata_only)
            };

            if !has_geometry {
                if metadata_only_geometry {
                    log::debug!(
                        "  {} -> metadata-only helper geometry (no render mesh): {}",
                        node.entity_name,
                        p4k_geom_path
                    );
                } else {
                    log::warn!("  {} -> mesh not found: {}", node.entity_name, p4k_geom_path);
                }
            }

            if node.offset_position != [0.0; 3] || node.offset_rotation != [0.0; 3] {
                log::info!(
                    "  resolve_children '{}': offset pos=[{:.2},{:.2},{:.2}] rot=[{:.1},{:.1},{:.1}]",
                    node.entity_name,
                    node.offset_position[0], node.offset_position[1], node.offset_position[2],
                    node.offset_rotation[0], node.offset_rotation[1], node.offset_rotation[2],
                );
            }

            crate::types::ResolvedNode {
                entity_name: node.entity_name.clone(),
                attachment_name,
                no_rotation: node.no_rotation,
                offset_position: node.offset_position,
                offset_rotation: node.offset_rotation,
                detach_direction: node.detach_direction,
                port_flags: node.port_flags.clone(),
                nmc,
                bones: Vec::new(),
                has_geometry,
                record: node.record,
                geometry_path: Some(geom_path.clone()),
                material_path: node.material_path.clone(),
                allows_child_object_containers: true,
                children,
            }
        })
        .collect()
}

pub(crate) fn is_shield_related_name(value: &str) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .any(|segment| {
            let lowered = segment.to_ascii_lowercase();
            lowered == "shield"
                || lowered == "shld"
                || lowered == "sheild"
                || lowered.starts_with("shield")
                || lowered.starts_with("sheild")
        })
}

pub(crate) fn port_flags_mark_invisible(flags: &str) -> bool {
    flags.split_whitespace().any(|flag| flag.eq_ignore_ascii_case("invisible"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_flags_mark_invisible_ports() {
        assert!(port_flags_mark_invisible("invisible uneditable"));
        assert!(port_flags_mark_invisible("uneditable invisible"));
        assert!(!port_flags_mark_invisible("dockingport1"));
        assert!(!port_flags_mark_invisible("not_invisible"));
    }
}
