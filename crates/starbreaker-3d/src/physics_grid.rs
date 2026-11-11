//! The vehicle's physics-grid volume for decomposed exports (`ExportOptions::include_physics`).
//! DataCore `VehicleComponentParams.physicsGrid.physicsProxy` names a `.cgf` whose `StatObjPhysics` collider is a
//! closed volume around the hull: the region of the vehicle's own physics grid, inside which the game's actors
//! belong to the grid and the vehicle's own (exterior) colliders no longer block them (the files are commonly
//! named `*_no_collide` / `*_local_grid`). [`query_physics_grid`] reads it and [`grid_json`] emits its shapes,
//! moved into the vehicle entity's frame, as `scene.json` `root_entity.physics_grid`.
//! Contract: docs/decomposed-export-contract.md.
use crate::error::Error;
use crate::ivo::physics::{PhysicsEntry, PhysicsGeometry, read_physics};
use crate::nmc::NmcNode;
use serde_json::{Value, json};
use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

/// `root_entity.physics_grid` for a vehicle record; `None` when the record names no grid volume.
pub(crate) fn query_physics_grid(db: &Database, p4k: &MappedP4k, record: &Record) -> Result<Option<Value>, Error> {
    let path = db
        .compile_path::<String>(record.struct_id(), "Components[VehicleComponentParams].physicsGrid.physicsProxy.path")
        .ok()
        .and_then(|path| db.query_single::<String>(&path, record).ok().flatten())
        .filter(|path| !path.is_empty());
    let Some(path) = path else { return Ok(None) };
    let entry = p4k
        .entry_case_insensitive(&crate::pipeline::datacore_path_to_p4k(&path))
        .ok_or_else(|| Error::Other(format!("physics grid: {path} is not in the P4k")))?;
    let bytes = p4k.read(entry).map_err(|e| Error::Other(format!("physics grid {path}: {e}")))?;
    let entries = read_physics(&bytes).map_err(|e| Error::Other(format!("physics grid {path}: {e}")))?.unwrap_or_default();
    let nodes = crate::nmc::parse_nmc_full(&bytes).map(|(nodes, _)| nodes).unwrap_or_default();
    grid_json(&entry.name.replace('\\', "/"), &entries, &nodes)
}

/// The volume's shapes in the entity frame: a per-node entry through its node's `bone_to_world`, whole-object
/// physics through the single geometry node when there is exactly one (as in [`crate::decomposed_physics`]),
/// else as stored. Damage meshes and SDFs are not part of the volume.
pub(crate) fn grid_json(source_path: &str, entries: &[PhysicsEntry], nodes: &[NmcNode]) -> Result<Option<Value>, Error> {
    let geometry_nodes: Vec<&NmcNode> = nodes.iter().filter(|n| n.geometry_type == 0).collect();
    let mut shapes = Vec::new();
    for e in entries {
        let m = match e.node {
            Some(i) => Some(
                nodes.get(i as usize).ok_or_else(|| Error::Other(format!("physics grid: node {i} outside the NMC")))?.bone_to_world,
            ),
            None if geometry_nodes.len() == 1 => Some(geometry_nodes[0].bone_to_world),
            None => None,
        };
        let point = |p: &[f32; 3]| m.map_or(*p, |m| std::array::from_fn(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3]));
        let dir = |d: &[f32; 3]| m.map_or(*d, |m| std::array::from_fn(|r| m[r][0] * d[0] + m[r][1] * d[1] + m[r][2] * d[2]));
        let mut shape = match &e.geometry {
            PhysicsGeometry::Box { axes, center, half_size } => json!({
                "kind": "box", "center": point(center), "half_size": half_size, "axes": axes.iter().map(dir).collect::<Vec<_>>()}),
            PhysicsGeometry::Sphere { center, radius } => json!({"kind": "sphere", "center": point(center), "radius": radius}),
            PhysicsGeometry::Cylinder { center, axis, radius, half_height } => json!({
                "kind": "cylinder", "center": point(center), "axis": dir(axis), "radius": radius, "half_height": half_height}),
            PhysicsGeometry::Capsule { center, axis, radius, half_height } => json!({
                "kind": "capsule", "center": point(center), "axis": dir(axis), "radius": radius, "half_height": half_height}),
            PhysicsGeometry::Trimesh { vertices, triangles, .. } => json!({
                "kind": "trimesh", "vertices": vertices.iter().map(point).collect::<Vec<_>>(), "triangles": triangles}),
            PhysicsGeometry::DamageMesh { .. } | PhysicsGeometry::Sdf { .. } => continue,
        };
        shape["part"] = json!(e.part);
        shapes.push(shape);
    }
    Ok((!shapes.is_empty()).then(|| json!({"source_path": source_path, "shapes": shapes})))
}

#[cfg(test)]
#[path = "physics_grid_tests.rs"]
mod tests;
