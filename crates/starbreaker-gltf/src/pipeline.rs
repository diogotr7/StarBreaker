use std::str::FromStr;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::error::QueryError;
use starbreaker_datacore::types::Record;
use starbreaker_dds::{DdsFile, ReadSibling};
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::mtl;
use crate::nmc;
use crate::types::MaterialTextures;

/// Bundled result of extracting an entity's mesh data from the P4k archive.
type EntityPayload = (
    crate::Mesh,
    Option<mtl::MtlFile>,
    Option<MaterialTextures>,
    Option<nmc::NodeMeshCombo>,
    Option<mtl::TintPalette>,
    String,
    String,
    Vec<crate::skeleton::Bone>,
);

/// Options for controlling the export pipeline.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Include texture loading and embedding (disable for much smaller GLB files).
    pub include_textures: bool,
    /// Texture mip level to use (0 = full resolution, 2 = 1/4 res, 4 = 1/16 res).
    /// Higher values = smaller textures = smaller GLB.
    pub texture_mip: u32,
    /// Use a lower LOD level (0 = highest detail, 1+ = lower).
    /// Looks for `_lod{N}` variants of geometry files in the P4k.
    pub lod_level: u32,
    /// Include interior geometry from socpak object containers.
    pub include_interior: bool,
    /// Include normal map textures.
    pub include_normals: bool,
    /// Include lights from interior socpak containers.
    pub include_lights: bool,
    /// Include tangent data (disable to reduce GLB size when tangents are not needed).
    pub include_tangents: bool,
    /// Include material data (.mtl parsing, palette colors, material extras).
    /// When disabled, all surfaces are plain white — useful for debugging geometry.
    pub include_materials: bool,
    /// Use aggressive texture matching: apply normal/roughness textures even when
    /// they come from a different texture set than the diffuse (different UV space).
    /// This can produce specular noise on some materials but preserves more detail
    /// on materials where the UV mapping happens to work.
    pub experimental_textures: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_textures: true,
            texture_mip: 2,
            lod_level: 1,
            include_interior: true,
            include_normals: true,
            include_lights: true,
            include_tangents: true,
            include_materials: true,
            experimental_textures: false,
        }
    }
}

/// Result of exporting an entity record to GLB.
pub struct ExportResult {
    /// The GLB binary data.
    pub glb: Vec<u8>,
    /// The geometry file path from DataCore (e.g., "objects/ships/aegs/aegs_gladius.skin").
    pub geometry_path: String,
    /// The material file path from DataCore (e.g., "objects/ships/aegs/aegs_gladius.mtl").
    pub material_path: String,
}

/// Export a single entity's mesh, materials, textures, NMC, and palette from DataCore + P4k.
/// Export an entity payload by resolving geometry/material paths from DataCore.
pub(crate) fn export_entity_payload(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    opts: &ExportOptions,
) -> Result<EntityPayload, Error> {
    export_entity_payload_cached(db, p4k, record, opts, &mut PngCache::new())
}

fn export_entity_payload_cached(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
) -> Result<EntityPayload, Error> {
    let record_name = || db.resolve_string2(record.name_offset).to_string();

    let geom_compiled = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
        )
        .map_err(|e| match e {
            QueryError::PropertyNotFound { .. }
            | QueryError::TypeFilterMismatch { .. }
            | QueryError::TypeFilterRequired { .. } => Error::NoGeometryComponent {
                record_name: record_name(),
            },
            other => Error::DataCoreQuery(other),
        })?;
    let mtl_compiled = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
        )
        .map_err(|e| match e {
            QueryError::PropertyNotFound { .. }
            | QueryError::TypeFilterMismatch { .. }
            | QueryError::TypeFilterRequired { .. } => Error::NoGeometryComponent {
                record_name: record_name(),
            },
            other => Error::DataCoreQuery(other),
        })?;

    let geometry_path = db
        .query_single::<String>(&geom_compiled, record)?
        .ok_or_else(|| Error::NoGeometryComponent {
            record_name: record_name(),
        })?;

    let material_path = db
        .query_single::<String>(&mtl_compiled, record)?
        .unwrap_or_default();

    let (mesh, mtl_file, textures, nmc, skeleton_bones, primary_path) =
        load_geometry_parts(p4k, &geometry_path, &material_path, opts, png_cache, false)?;

    if !opts.include_materials {
        return Ok((mesh, None, None, nmc, None, primary_path, material_path, skeleton_bones));
    }

    let palette = query_tint_palette(db, record);

    Ok((
        mesh,
        mtl_file,
        textures,
        nmc,
        palette,
        primary_path,
        material_path,
        skeleton_bones,
    ))
}

/// Export an entity payload using pre-resolved geometry/material paths (no DataCore lookup).
fn export_entity_from_paths(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
) -> Result<EntityPayload, Error> {
    export_entity_from_paths_cached(p4k, geometry_path, material_path, opts, &mut PngCache::new(), false)
}

fn export_entity_from_paths_cached(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<EntityPayload, Error> {
    let (mesh, mtl_file, textures, nmc, _skeleton_bones, primary_path) =
        load_geometry_parts(p4k, geometry_path, material_path, opts, png_cache, use_model_bbox)?;

    if !opts.include_materials {
        return Ok((mesh, None, None, nmc, None, primary_path, material_path.to_string(), Vec::new()));
    }

    Ok((mesh, mtl_file, textures, nmc, None, primary_path, material_path.to_string(), Vec::new()))
}

/// Shared geometry loading: resolve parts, load skeleton, load + merge meshes.
/// Returns (mesh, mtl, textures, nmc, skeleton_bones, primary_path).
fn load_geometry_parts(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<(
    crate::types::Mesh,
    Option<mtl::MtlFile>,
    Option<MaterialTextures>,
    Option<nmc::NodeMeshCombo>,
    Vec<crate::skeleton::Bone>,
    String,
), Error> {
    let resolved = resolve_geometry_files(p4k, geometry_path)?;
    let primary_path = resolved.parts[0].path.clone();

    let skeleton_bones = load_skeleton(p4k, resolved.skeleton_path.as_deref());

    let effective_material = resolved.parts[0]
        .material_override
        .as_deref()
        .unwrap_or(material_path);
    let (mut mesh, mtl_file, textures, nmc) =
        load_single_mesh(p4k, &primary_path, effective_material, opts, png_cache, use_model_bbox)?;

    // Merge additional parts (CA_BONE/CA_SKIN attachments from CDF).
    let no_tex_opts = ExportOptions { include_textures: false, ..opts.clone() };
    for part in &resolved.parts[1..] {
        match load_single_mesh(p4k, &part.path, material_path, &no_tex_opts, png_cache, use_model_bbox) {
            Ok((mut extra_mesh, _, _, _)) => {
                if let Some(ref bone_name) = part.bone_name {
                    if let Some(bone) = skeleton_bones.iter().find(|b| b.name.eq_ignore_ascii_case(bone_name)) {
                        transform_mesh_by_bone(&mut extra_mesh, bone);
                    }
                }
                mesh.merge_from(extra_mesh);
            }
            Err(e) => log::warn!("  CDF part '{}' failed: {e}", part.path),
        }
    }

    Ok((mesh, mtl_file, textures, nmc, skeleton_bones, primary_path))
}

/// Load skeleton bones from a .chr path. Returns empty vec if path is None or load fails.
fn load_skeleton(p4k: &MappedP4k, skel_path: Option<&str>) -> Vec<crate::skeleton::Bone> {
    let Some(skel_path) = skel_path else { return Vec::new() };
    let p4k_skel = datacore_path_to_p4k(skel_path);
    p4k.entry_case_insensitive(&p4k_skel)
        .and_then(|entry| p4k.read(entry).ok())
        .and_then(|data| crate::skeleton::parse_skeleton(&data))
        .unwrap_or_default()
}

/// Export an entity with its loadout tree as a single GLB.
///
/// This is the main export entry point. Handles loadout children, interiors,
/// invisible port filtering, textures, and lights. Works for any DataCore
/// entity — ships get full loadout assembly, simpler entities just export
/// their root geometry.
pub fn assemble_glb_with_loadout(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
    opts: &ExportOptions,
) -> Result<ExportResult, Error> {
    use crate::types::EntityPayload;

    log::info!("[mem-pipeline] resolving loadout meshes...");
    let resolved = resolve_loadout_meshes(db, p4k, record, tree, opts)?;
    log::info!("[mem-pipeline] resolved: {} children", resolved.children.len());

    // Export root entity (mesh + textures).
    let (root_mesh, root_mtl, root_tex, _, root_palette, geometry_path, material_path, root_bones) =
        export_entity_payload(db, p4k, record, opts)?;
    log::info!("[mem-pipeline] root exported: {} verts", root_mesh.positions.len());

    // Load landing gear as separate child entities attached to NMC nodes.
    // Landing gear CDF geometry attaches to NMC helper bones (e.g. hardpoint_landing_gear_front).
    // Adding them as EntityPayloads lets the existing scene graph handle positioning.
    let no_tex_opts = ExportOptions {
        include_textures: false,
        ..opts.clone()
    };
    let gear_parts = query_landing_gear(db, record);
    let mut child_payloads: Vec<EntityPayload> = Vec::new();
    for (gear_path, bone_name) in &gear_parts {
        match export_entity_from_paths(p4k, gear_path, "", &no_tex_opts) {
            Ok((gear_mesh, gear_mtl, _, gear_nmc, _, _, _, _)) => {
                let verts = gear_mesh.positions.len();
                child_payloads.push(EntityPayload {
                    mesh: gear_mesh,
                    materials: gear_mtl,
                    textures: None,
                    nmc: gear_nmc,
                    palette: root_palette.clone(),
                    bones: Vec::new(),
                    entity_name: gear_path.rsplit('/').next().unwrap_or(gear_path).to_string(),
                    parent_node_name: bone_name.clone(),
                    parent_entity_name: resolved.entity_name.clone(),
                    no_rotation: false,
                    offset_position: [0.0; 3],
                    offset_rotation: [0.0; 3],
                });
                log::info!("  landing gear '{gear_path}' → '{bone_name}', {verts} verts");
            }
            Err(e) => log::warn!("  landing gear '{gear_path}' failed: {e}"),
        }
    }
    flatten_resolved_tree(
        &resolved.children,
        &resolved.entity_name,
        None,
        db,
        p4k,
        &no_tex_opts,
        &mut child_payloads,
    );
    let total_child_verts: usize = child_payloads.iter().map(|c| c.mesh.positions.len()).sum();
    log::info!("[mem-pipeline] flattened: {} payloads, {} total verts", child_payloads.len(), total_child_verts);

    // Interior discovery (no mesh loading — JIT during GLB packing).
    let loaded_interiors = if opts.include_interior {
        load_interiors(db, p4k, record, opts)
    } else {
        LoadedInteriors::default()
    };

    log::info!(
        "Assembling: root + {} child meshes + {} interior meshes ({} unique CGFs)",
        child_payloads.len(),
        loaded_interiors
            .containers
            .iter()
            .map(|c| c.placements.len())
            .sum::<usize>(),
        loaded_interiors.unique_cgfs.len()
    );

    // Texture loading callback: called JIT per entity during GLB packing.
    let mut png_cache = PngCache::new();
    let mut tex_loader: Box<dyn FnMut(Option<&crate::mtl::MtlFile>) -> Option<MaterialTextures>> =
        if !opts.include_textures {
            Box::new(|_| None)
        } else {
            let mip = opts.texture_mip;
            let include_normals = opts.include_normals;
            let experimental_textures = opts.experimental_textures;
            Box::new(move |mtl: Option<&crate::mtl::MtlFile>| {
                mtl.map(|m| load_material_textures(p4k, m, mip, &mut png_cache, include_normals, experimental_textures))
            })
        };

    // Interior mesh loader: called JIT per unique CGF during GLB packing.
    let mut interior_png_cache = PngCache::new();
    // Interior CGFs: bake NMC transforms only when the scaling bbox differs from
    // the model bbox (indicating multi-node vertex quantization). Use absolute
    // world transforms (no root_inv) because root + child rotations cancel in
    // the assembled model. When bboxes match, vertices are already correct.
    let mut interior_mesh_loader =
        |entry: &crate::pipeline::InteriorCgfEntry| -> Option<(crate::Mesh, Option<mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
            match export_cgf_from_path(
                p4k,
                &entry.cgf_path,
                entry.material_path.as_deref(),
                &no_tex_opts,
                &mut interior_png_cache,
            ) {
                Ok((mesh, mtl, _tex, nmc, _palette, _, _, _bones)) => {
                    let needs_bake = mesh.scaling_min.iter().zip(&mesh.model_min)
                        .chain(mesh.scaling_max.iter().zip(&mesh.model_max))
                        .any(|(s, m)| (s - m).abs() > 0.01);
                    let mesh = if needs_bake {
                        bake_nmc_absolute(mesh, nmc.as_ref())
                    } else {
                        mesh
                    };
                    Some((mesh, mtl, nmc))
                }
                Err(e) => {
                    log::warn!("failed to load CGF {}: {e}", entry.cgf_path);
                    None
                }
            }
        };

    let glb = crate::gltf::write_glb(
        crate::gltf::GlbInput {
            root_mesh: Some(root_mesh),
            root_materials: root_mtl,
            root_textures: root_tex,
            root_nmc: resolved.nmc,
            root_palette: root_palette.clone(),
            skeleton_bones: root_bones,
            children: child_payloads,
            interiors: loaded_interiors,
        },
        &mut crate::gltf::GlbLoaders {
            load_textures: &mut tex_loader,
            load_interior_mesh: &mut interior_mesh_loader,
        },
        &crate::gltf::GlbOptions {
            include_tangents: opts.include_tangents,
            experimental_textures: opts.experimental_textures,
            metadata: crate::gltf::GlbMetadata {
                entity_name: Some(resolved.entity_name.clone()),
                geometry_path: Some(geometry_path.clone()),
                material_path: Some(material_path.clone()),
                export_options: crate::gltf::ExportOptionsMetadata {
                    texture_mip: opts.texture_mip,
                    lod_level: opts.lod_level,
                    include_interior: opts.include_interior,
                    include_tangents: opts.include_tangents,
                    include_lights: opts.include_lights,
                    include_textures: opts.include_textures,
                    include_materials: opts.include_materials,
                },
            },
            fallback_palette: root_palette,
        },
    )?;

    Ok(ExportResult {
        glb,
        geometry_path,
        material_path,
    })
}

/// Try loading an entity's mesh data from its resolved geometry/material paths,
/// falling back to DataCore record lookup.
fn load_child_mesh(
    child: &crate::types::ResolvedNode,
    db: &Database,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> Option<(crate::types::Mesh, Option<mtl::MtlFile>, Option<nmc::NodeMeshCombo>, Option<mtl::TintPalette>, Vec<crate::skeleton::Bone>)> {
    let result = if child.geometry_path.is_some() {
        let gp = child.geometry_path.as_deref().unwrap_or("");
        let mp = child.material_path.as_deref().unwrap_or("");
        export_entity_from_paths(p4k, gp, mp, opts)
            .or_else(|_| export_entity_payload(db, p4k, &child.record, opts))
    } else {
        export_entity_payload(db, p4k, &child.record, opts)
    };

    result.ok().map(|(mesh, mtl, _tex, nmc, palette, _, _, bones)| (mesh, mtl, nmc, palette, bones))
}

/// Flatten a resolved tree into EntityPayload list, loading meshes on demand.
///
/// When `override_attachment` is Some, the first level of children uses that
/// attachment name instead of their own (reparenting through a no-geometry parent).
fn flatten_resolved_tree(
    children: &[crate::types::ResolvedNode],
    parent_entity_name: &str,
    override_attachment: Option<(&str, bool)>,
    db: &Database,
    p4k: &MappedP4k,
    opts: &ExportOptions,
    out: &mut Vec<crate::types::EntityPayload>,
) {
    for child in children {
        let (attach_name, no_rotation) = match override_attachment {
            Some((name, parent_no_rot)) => (name.to_string(), child.no_rotation || parent_no_rot),
            None => (child.attachment_name.clone(), child.no_rotation),
        };

        let child_creates_nodes = if child.has_geometry {
            if let Some((mesh, mtl, nmc, palette, bones)) = load_child_mesh(child, db, p4k, opts) {
                out.push(crate::types::EntityPayload {
                    mesh,
                    materials: mtl,
                    textures: None,
                    nmc,
                    palette,
                    bones,
                    entity_name: child.entity_name.clone(),
                    parent_node_name: attach_name,
                    parent_entity_name: parent_entity_name.to_string(),
                    no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                });
                true
            } else {
                false
            }
        } else if child.nmc.is_some() {
            // No mesh geometry but has NMC (helpers/bones) — emit an empty-mesh
            // payload so NMC nodes enter the scene graph and provide correct
            // transforms for children (e.g., missile racks with attachment points).
            let empty_mesh = crate::types::Mesh {
                positions: Vec::new(),
                indices: Vec::new(),
                uvs: None,
                normals: None,
                tangents: None,
                colors: None,
                submeshes: Vec::new(),
                model_min: [0.0; 3],
                model_max: [0.0; 3],
                scaling_min: [0.0; 3],
                scaling_max: [0.0; 3],
            };
            out.push(crate::types::EntityPayload {
                mesh: empty_mesh,
                materials: None,
                textures: None,
                nmc: child.nmc.clone(),
                palette: None,
                bones: Vec::new(),
                entity_name: child.entity_name.clone(),
                parent_node_name: attach_name,
                parent_entity_name: parent_entity_name.to_string(),
                no_rotation,
                offset_position: child.offset_position,
                offset_rotation: child.offset_rotation,
            });
            true
        } else {
            false
        };

        if child_creates_nodes {
            flatten_resolved_tree(
                &child.children,
                &child.entity_name,
                None,
                db,
                p4k,
                opts,
                out,
            );
        } else {
            // No geometry — reparent grandchildren to this child's attachment point.
            flatten_resolved_tree(
                &child.children,
                parent_entity_name,
                Some((&child.attachment_name, child.no_rotation)),
                db,
                p4k,
                opts,
                out,
            );
        }
    }
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

    // Load NMC + skeleton from .cga/.chr
    let (nmc, _mtl) = load_nmc_and_material(p4k, &geometry_path, &material_path);

    let resolved = resolve_geometry_files(p4k, &geometry_path)?;
    let bones = load_skeleton(p4k, resolved.skeleton_path.as_deref());

    // Check if mesh companion exists
    let p4k_geom_path = datacore_path_to_p4k(&geometry_path);
    let companion = resolve_companion_path(p4k, &p4k_geom_path, opts.lod_level);
    let has_geometry = p4k.entry_case_insensitive(&companion).is_some();

    // Load invisible port flags from vehicle XML (empty for non-vehicles).
    let invisible_ports = load_invisible_ports(db, p4k, record);

    let children = resolve_children(db, p4k, &tree.root.children, opts, &invisible_ports);

    Ok(crate::types::ResolvedNode {
        entity_name: tree.root.entity_name.clone(),
        attachment_name: String::new(),
        no_rotation: false,
        offset_position: [0.0; 3],
        offset_rotation: [0.0; 3],
        nmc,
        bones,
        has_geometry,
        record: *record,
        geometry_path: Some(geometry_path),
        material_path: Some(material_path),
        children,
    })
}

fn resolve_children(
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
                    nmc: None,
                    bones: Vec::new(),
                    has_geometry: false,
                    record: node.record,
                    geometry_path: None,
                    material_path: node.material_path.clone(),
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
                    nmc: None,
                    bones: Vec::new(),
                    has_geometry: false,
                    record: node.record,
                    geometry_path: None,
                    material_path: node.material_path.clone(),
                    children,
                };
            };

            // Load NMC from .cga (always, even if .cgam is missing).
            let mat_path = node.material_path.as_deref().unwrap_or("");
            let (nmc, _mtl) = load_nmc_and_material(p4k, geom_path, mat_path);

            // Probe whether mesh companion exists.
            let p4k_geom_path = datacore_path_to_p4k(geom_path);
            let companion = resolve_companion_path(p4k, &p4k_geom_path, opts.lod_level);
            let has_geometry = p4k.entry_case_insensitive(&companion).is_some();

            if !has_geometry {
                log::warn!("  {} -> mesh not found: {}", node.entity_name, companion);
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
                nmc,
                bones: Vec::new(),
                has_geometry,
                record: node.record,
                geometry_path: Some(geom_path.clone()),
                material_path: node.material_path.clone(),
                children,
            }
        })
        .collect()
}

// ── Interior loading ────────────────────────────────────────────────────────

/// Interior layout data: unique CGF paths + placement transforms.
/// Mesh data is NOT loaded here — it's loaded JIT during GLB packing.
pub struct LoadedInteriors {
    /// Unique CGF entries (deduplicated by path).
    pub unique_cgfs: Vec<InteriorCgfEntry>,
    /// Per-container data (one per socpak).
    pub containers: Vec<InteriorContainerData>,
}

impl Default for LoadedInteriors {
    fn default() -> Self {
        Self {
            unique_cgfs: Vec::new(),
            containers: Vec::new(),
        }
    }
}

/// Metadata for one unique interior CGF (no mesh data).
pub struct InteriorCgfEntry {
    pub cgf_path: String,
    pub material_path: Option<String>,
    pub name: String,
}

/// One interior container's placement data.
pub struct InteriorContainerData {
    pub name: String,
    /// 4×4 column-major transform positioning this container relative to the hull.
    pub container_transform: [[f32; 4]; 4],
    /// Each entry: (index into unique_cgfs, per-object local transform).
    pub placements: Vec<(usize, [[f32; 4]; 4])>,
    pub lights: Vec<crate::types::LightInfo>,
    /// Tint palette resolved from the socpak's IncludedObjects tint_palette_paths.
    pub palette: Option<mtl::TintPalette>,
}

/// Discovery pass: parse socpaks to find unique CGF paths and placements.
/// No mesh data is loaded — that happens JIT during GLB packing.
fn load_interiors(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    opts: &ExportOptions,
) -> LoadedInteriors {
    use crate::socpak;

    let containers = socpak::query_object_containers(db, record);
    if containers.is_empty() {
        return LoadedInteriors::default();
    }

    log::info!("Discovering {} interior containers...", containers.len());

    let mut payloads = Vec::new();
    for container in &containers {
        let container_transform =
            socpak::build_container_transform(container.offset_position, container.offset_rotation);
        match socpak::load_interior_from_socpak(p4k, &container.file_name, container_transform) {
            Ok(p) => payloads.push(p),
            Err(e) => log::warn!("failed to load {}: {e}", container.file_name),
        }
    }

    build_interiors_from_payloads(db, &payloads, opts.include_lights)
}

/// Shared interior building: dedup CGFs, resolve GUIDs, collect placements and lights.
/// Used by both `load_interiors` (from DataCore) and `socpaks_to_glb` (from explicit paths).
fn build_interiors_from_payloads(
    db: &Database,
    payloads: &[crate::types::InteriorPayload],
    include_lights: bool,
) -> LoadedInteriors {
    use std::collections::HashMap;

    let guid_geom_compiled = db.compile_rooted::<String>(
        "EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
    ).ok();
    let guid_mtl_compiled = db.compile_rooted::<String>(
        "EntityClassDefinition.Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
    ).ok();

    let mut cgf_cache: HashMap<String, Option<usize>> = HashMap::new();
    let mut unique_cgfs = Vec::new();
    let mut container_data = Vec::new();

    for payload in payloads {
        log::debug!(
            "  {} → {} meshes, {} lights",
            payload.name,
            payload.meshes.len(),
            payload.lights.len()
        );

        let mut placements = Vec::new();

        for im in &payload.meshes {
            let (cgf_path, mtl_path) = if !im.cgf_path.is_empty() {
                (im.cgf_path.clone(), im.material_path.clone())
            } else if let Some(guid_str) = &im.entity_class_guid {
                match resolve_guid_geometry(
                    db,
                    guid_str,
                    guid_geom_compiled.as_ref(),
                    guid_mtl_compiled.as_ref(),
                ) {
                    Some((geom, mtl)) => (geom, Some(mtl).filter(|s| !s.is_empty())),
                    None => {
                        log::debug!("  GUID {guid_str} → no geometry found");
                        continue;
                    }
                }
            } else {
                continue;
            };

            let mesh_idx = *cgf_cache.entry(cgf_path.clone()).or_insert_with(|| {
                let idx = unique_cgfs.len();
                let name = cgf_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&cgf_path)
                    .strip_suffix(".cgf")
                    .unwrap_or(&cgf_path)
                    .to_string();
                unique_cgfs.push(InteriorCgfEntry {
                    cgf_path: cgf_path.clone(),
                    material_path: mtl_path.clone().or_else(|| im.material_path.clone()),
                    name,
                });
                Some(idx)
            });

            if let Some(idx) = mesh_idx {
                placements.push((idx, im.transform));
            }
        }

        // Resolve tint palette from the socpak's IncludedObjects palette names.
        // These are DataCore TintPaletteTree record paths — extract the short name
        // (last path component) and look up the record.
        let palette = payload.tint_palette_names.first().and_then(|path| {
            let short_name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
            let tpt_si = db.struct_id("TintPaletteTree")?;
            let record = db.records_of_type(tpt_si).find(|r| {
                db.resolve_string2(r.name_offset).to_lowercase().ends_with(&short_name)
            })?;
            query_tint_from_record(db, record)
        });

        if let Some(ref p) = palette {
            log::debug!(
                "  {} palette: primary=[{:.2},{:.2},{:.2}] secondary=[{:.2},{:.2},{:.2}]",
                payload.name, p.primary[0], p.primary[1], p.primary[2],
                p.secondary[0], p.secondary[1], p.secondary[2],
            );
        }

        container_data.push(InteriorContainerData {
            name: payload.name.clone(),
            container_transform: payload.container_transform,
            placements,
            lights: if include_lights { payload.lights.clone() } else { Vec::new() },
            palette,
        });
    }

    log::info!(
        "  {} unique CGFs, {} containers",
        unique_cgfs.len(),
        container_data.len()
    );

    LoadedInteriors {
        unique_cgfs,
        containers: container_data,
    }
}

/// Resolve an EntityClassGUID to its geometry + material paths via DataCore.
fn resolve_guid_geometry(
    db: &Database,
    guid_str: &str,
    geom_compiled: Option<&starbreaker_datacore::query::compile::CompiledPath>,
    mtl_compiled: Option<&starbreaker_datacore::query::compile::CompiledPath>,
) -> Option<(String, String)> {
    use starbreaker_common::CigGuid;

    let guid = CigGuid::from_str(guid_str).ok()?;
    let record = db.record_by_id(&guid)?;
    let record_name = db.resolve_string2(record.name_offset);
    let struct_name = db.struct_name(record.struct_id());

    let geom_path = match geom_compiled
        .and_then(|compiled| db.query_single::<String>(compiled, record).ok().flatten())
        .or_else(|| {
            // Fallback: try compiling path for this specific struct type
            let compiled = db
                .compile_path::<String>(
                    record.struct_id(),
                    "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
                )
                .ok()?;
            db.query_single::<String>(&compiled, record).ok().flatten()
        }) {
        Some(p) => p,
        None => {
            log::debug!(
                "  GUID {guid_str} → {struct_name}.{record_name} has no geometry component"
            );
            return None;
        }
    };

    if geom_path.is_empty() {
        log::debug!("  GUID {guid_str} → {struct_name}.{record_name} has empty geometry path");
        return None;
    }

    let mtl_path = mtl_compiled
        .and_then(|compiled| db.query_single::<String>(compiled, record).ok().flatten())
        .or_else(|| {
            let compiled = db
                .compile_path::<String>(
                    record.struct_id(),
                    "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
                )
                .ok()?;
            db.query_single::<String>(&compiled, record).ok().flatten()
        })
        .unwrap_or_default();

    log::debug!("  GUID {guid_str} → {geom_path}");
    Some((geom_path, mtl_path))
}

/// Compute world-space transforms for each NMC node by walking the parent chain.
fn compute_nmc_world_transforms(nmc: &crate::nmc::NodeMeshCombo) -> Vec<glam::Mat4> {
    let local: Vec<glam::Mat4> = nmc
        .nodes
        .iter()
        .map(|n| {
            let m = &n.bone_to_world;
            glam::Mat4::from_cols_array(&[
                m[0][0], m[1][0], m[2][0], 0.0,
                m[0][1], m[1][1], m[2][1], 0.0,
                m[0][2], m[1][2], m[2][2], 0.0,
                m[0][3], m[1][3], m[2][3], 1.0,
            ])
        })
        .collect();

    let n = nmc.nodes.len();
    let mut world = vec![None; n];

    fn resolve(
        i: usize,
        nodes: &[crate::nmc::NmcNode],
        local: &[glam::Mat4],
        world: &mut [Option<glam::Mat4>],
    ) -> glam::Mat4 {
        if let Some(w) = world[i] {
            return w;
        }
        let w = match nodes[i].parent_index {
            Some(pi) if (pi as usize) < nodes.len() && (pi as usize) != i => {
                resolve(pi as usize, nodes, local, world) * local[i]
            }
            _ => local[i],
        };
        world[i] = Some(w);
        w
    }

    for i in 0..n {
        resolve(i, &nmc.nodes, &local, &mut world);
    }

    world.into_iter().map(|w| w.unwrap()).collect()
}

/// Bake NMC with absolute world transforms (no root_inv).
/// Used for interior CGFs where scaling bbox ≠ model bbox.
fn bake_nmc_absolute(
    mut mesh: crate::types::Mesh,
    nmc: Option<&crate::nmc::NodeMeshCombo>,
) -> crate::types::Mesh {
    let nmc = match nmc {
        Some(n) if !n.nodes.is_empty() => n,
        _ => return mesh,
    };
    let world_transforms = compute_nmc_world_transforms(nmc);
    let mut vert_node: Vec<Option<usize>> = vec![None; mesh.positions.len()];
    for sub in &mesh.submeshes {
        let node_idx = sub.node_parent_index as usize;
        if node_idx >= world_transforms.len() { continue; }
        let start = sub.first_index as usize;
        let end = (start + sub.num_indices as usize).min(mesh.indices.len());
        for &idx in &mesh.indices[start..end] {
            let vi = idx as usize;
            if vi < vert_node.len() && vert_node[vi].is_none() {
                vert_node[vi] = Some(node_idx);
            }
        }
    }
    for (vi, node_opt) in vert_node.iter().enumerate() {
        let Some(node_idx) = node_opt else { continue };
        let xform = world_transforms[*node_idx];
        if xform == glam::Mat4::IDENTITY { continue; }
        let v = xform.transform_point3(glam::Vec3::from(mesh.positions[vi]));
        mesh.positions[vi] = v.into();
        if let Some(ref mut normals) = mesh.normals {
            if vi < normals.len() {
                let normal_mat = xform.inverse().transpose();
                let n = normal_mat.transform_vector3(glam::Vec3::from(normals[vi])).normalize();
                normals[vi] = n.into();
            }
        }
    }
    mesh
}

/// Bake NMC node transforms into mesh vertex positions (root-relative).
/// Used for instanced geometry where scaling bbox = model bbox.
fn bake_nmc_into_mesh(
    mut mesh: crate::types::Mesh,
    nmc: Option<&crate::nmc::NodeMeshCombo>,
) -> crate::types::Mesh {
    let nmc = match nmc {
        Some(n) if !n.nodes.is_empty() => n,
        _ => return mesh,
    };

    let world_transforms = compute_nmc_world_transforms(nmc);

    // Find the root node(s) — nodes with no parent.
    // Factor out the root transform so we bake relative to root, not absolute.
    let root_idx = nmc.nodes.iter().position(|n| n.parent_index.is_none());
    let root_inv = root_idx
        .map(|i| world_transforms[i].inverse())
        .unwrap_or(glam::Mat4::IDENTITY);

    let mut vert_node: Vec<Option<usize>> = vec![None; mesh.positions.len()];
    for sub in &mesh.submeshes {
        let node_idx = sub.node_parent_index as usize;
        if node_idx >= world_transforms.len() {
            continue;
        }
        let start = sub.first_index as usize;
        let end = (start + sub.num_indices as usize).min(mesh.indices.len());
        for &idx in &mesh.indices[start..end] {
            let vi = idx as usize;
            if vi < vert_node.len() && vert_node[vi].is_none() {
                vert_node[vi] = Some(node_idx);
            }
        }
    }

    for (vi, node_opt) in vert_node.iter().enumerate() {
        let Some(node_idx) = node_opt else { continue };
        // Transform relative to root: inv(root) × node_world × vertex
        let xform = root_inv * world_transforms[*node_idx];
        if xform == glam::Mat4::IDENTITY {
            continue;
        }
        let v = xform.transform_point3(glam::Vec3::from(mesh.positions[vi]));
        mesh.positions[vi] = v.into();
        if let Some(ref mut normals) = mesh.normals {
            if vi < normals.len() {
                let normal_mat = xform.inverse().transpose();
                let n = normal_mat
                    .transform_vector3(glam::Vec3::from(normals[vi]))
                    .normalize();
                normals[vi] = n.into();
            }
        }
    }

    mesh
}

/// Load a .cgf/.cgfm mesh from P4k by interior path.
fn export_cgf_from_path(
    p4k: &MappedP4k,
    cgf_path: &str,
    material_path: Option<&str>,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
) -> Result<EntityPayload, Error> {
    export_cgf_from_path_inner(p4k, cgf_path, material_path, opts, png_cache, false)
}

fn export_cgf_from_path_model_bbox(
    p4k: &MappedP4k,
    cgf_path: &str,
    material_path: Option<&str>,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
) -> Result<EntityPayload, Error> {
    export_cgf_from_path_inner(p4k, cgf_path, material_path, opts, png_cache, true)
}

fn export_cgf_from_path_inner(
    p4k: &MappedP4k,
    cgf_path: &str,
    material_path: Option<&str>,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<EntityPayload, Error> {
    // Strip "data/" prefix if present (CryXMLB paths sometimes include it)
    let clean_path = cgf_path.replace('\\', "/");
    let geometry_path = clean_path
        .strip_prefix("data/")
        .or_else(|| clean_path.strip_prefix("Data/"))
        .unwrap_or(&clean_path);
    let mtl_path = material_path.unwrap_or("");
    export_entity_from_paths_cached(p4k, geometry_path, mtl_path, opts, png_cache, use_model_bbox)
}

struct P4kSiblingReader<'a> {
    p4k: &'a MappedP4k,
    base_path: String,
}

impl ReadSibling for P4kSiblingReader<'_> {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k
            .entry_case_insensitive(&path)
            .and_then(|entry| self.p4k.read(entry).ok())
    }
}

/// Load all textures (diffuse + normal) for a material file.
/// Cache for loaded+encoded texture PNGs, keyed by resolved DDS path.
/// Prevents redundant DDS decode + PNG encode for the same texture file.
pub(crate) type PngCache = std::collections::HashMap<String, Option<Vec<u8>>>;

fn load_material_textures(
    p4k: &MappedP4k,
    mtl: &mtl::MtlFile,
    mip: u32,
    png_cache: &mut PngCache,
    include_normals: bool,
    experimental_textures: bool,
) -> MaterialTextures {
    let diffuse = mtl
        .materials
        .iter()
        .map(|m| {
            // Load direct TexSlot1 diffuse textures. For HardSurface/LayerBlend
            // materials, these are tileable detail patterns (greyscale) that get
            // multiplied by base_color_factor (palette tint) in glTF rendering.
            if let Some(path) = &m.diffuse_tex
                && let Some(png) = cached_load(p4k, path, mip, png_cache, load_diffuse_texture)
            {
                return Some(png);
            }
            // Also try the first MatLayer's external .mtl for the diffuse texture
            if let Some(layer) = m.layers.first() {
                let p4k_path = datacore_path_to_p4k(&layer.path);
                if let Some(layer_mtl) = try_load_mtl(p4k, &p4k_path)
                    && let Some(layer_mat) = layer_mtl.materials.first()
                    && let Some(tex_path) = &layer_mat.diffuse_tex
                {
                    return cached_load(p4k, tex_path, mip, png_cache, load_diffuse_texture);
                }
            }
            None
        })
        .collect();

    // Resolve normal texture path per submaterial and check UV compatibility.
    // Normal/roughness textures that come from a different texture set than the diffuse
    // (e.g., tileable normal on atlas-UV'd mesh) use different UV mappings.
    // glTF only supports one texCoord per texture, so mismatched UVs produce noise.
    let normal_paths: Vec<Option<String>> = mtl.materials.iter().map(|m| {
        // Direct normal_tex from TexSlot2
        if let Some(path) = &m.normal_tex {
            return Some(path.clone());
        }
        // Fallback: layer MTL's normal_tex
        m.layers.first().and_then(|layer| {
            let p4k_path = datacore_path_to_p4k(&layer.path);
            try_load_mtl(p4k, &p4k_path)
                .and_then(|lm| lm.materials.first()
                    .and_then(|lmat| lmat.normal_tex.clone()))
        })
    }).collect();

    let normal = if !include_normals {
        vec![None; mtl.materials.len()]
    } else {
        mtl.materials
            .iter()
            .zip(normal_paths.iter())
            .map(|(m, normal_path)| {
                let normal_mip = mip;
                let Some(path) = normal_path else { return None };
                // Skip if the normal texture uses different UVs than the diffuse.
                // Detect by checking if the texture base names share a common prefix.
                // Atlas diffuse + tileable normal = UV mismatch → skip.
                if !experimental_textures {
                    if let Some(ref dtex) = m.diffuse_tex {
                        if !textures_share_uv_space(dtex, path) {
                            log::debug!("  skipping mismatched normal: diffuse={dtex}, normal={path}");
                            return None;
                        }
                    }
                }
                cached_load(p4k, path, normal_mip, png_cache, load_normal_texture)
            })
            .collect()
    };

    // Extract per-pixel roughness from the alpha mips of _ddna normal maps.
    // Only when the normal texture is UV-compatible (same check as above).
    let roughness = if !include_normals {
        vec![None; mtl.materials.len()]
    } else {
        mtl.materials
            .iter()
            .zip(normal_paths.iter())
            .map(|(m, normal_path)| {
                let Some(tif_path) = normal_path else { return None };
                if !tif_path.contains("_ddna") {
                    return None;
                }
                if !experimental_textures {
                    if let Some(ref dtex) = m.diffuse_tex {
                        if !textures_share_uv_space(dtex, tif_path) {
                            return None;
                        }
                    }
                }

                let cache_key = format!("{tif_path}@roughness_mip{mip}");
                if let Some(cached) = png_cache.get(&cache_key) {
                    return cached.clone();
                }

                let result = load_roughness_texture(p4k, tif_path, mip);
                png_cache.insert(cache_key, result.clone());
                result
            })
            .collect()
    };

    MaterialTextures { diffuse, normal, roughness }
}

/// Check if a diffuse and normal texture are from the same texture set (same UV space).
///
/// CryEngine materials can pair atlas diffuse textures (unique UV layout per mesh) with
/// tileable normal maps (designed to repeat). These use different UV mappings but we only
/// support one texCoord in glTF. When they don't match, the normal/roughness creates noise.
///
/// Heuristic: extract the filename stem (strip path + suffixes like `_diff`, `_ddna`) and
/// check if they share a common base. E.g., `cockpit_diff.tif` + `cockpit_ddna.tif` → match.
/// `leather_atlas_a_diff.tif` + `leather_base_tilable_ddna.dds` → no match.
fn textures_share_uv_space(diffuse_path: &str, normal_path: &str) -> bool {
    fn stem(p: &str) -> &str {
        let filename = p.rsplit(&['/', '\\']).next().unwrap_or(p);
        let base = filename.split('.').next().unwrap_or(filename);
        let base = base.strip_suffix("_diff").unwrap_or(base);
        let base = base.strip_suffix("_ddna").unwrap_or(base);
        let base = base.strip_suffix("_ddn").unwrap_or(base);
        let base = base.strip_suffix("_spec").unwrap_or(base);
        base
    }
    let d = stem(diffuse_path);
    let n = stem(normal_path);
    d == n || d.starts_with(n) || n.starts_with(d)
}

/// Load a texture with caching by path — prevents redundant DDS decode + PNG encode.
fn cached_load(
    p4k: &MappedP4k,
    path: &str,
    mip: u32,
    cache: &mut PngCache,
    loader: fn(&MappedP4k, &str, u32) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let key = format!("{path}@mip{mip}");
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let result = loader(p4k, path, mip);
    cache.insert(key, result.clone());
    result
}

fn load_diffuse_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
    if tif_path.starts_with('$') {
        return None;
    }

    let dds_path = tif_path
        .strip_suffix(".tif")
        .map(|base| format!("{base}.dds"))
        .unwrap_or_else(|| tif_path.to_string());

    let p4k_dds_path = datacore_path_to_p4k(&dds_path);
    let base_entry = p4k.entry_case_insensitive(&p4k_dds_path)?;
    let base_bytes = p4k.read(base_entry).ok()?;

    let sibling_reader = P4kSiblingReader {
        p4k,
        base_path: p4k_dds_path,
    };
    let dds = DdsFile::from_split(&base_bytes, &sibling_reader).ok()?;

    // Use requested mip level, clamped to available levels
    let mip = (mip_level as usize).min(dds.mip_count().saturating_sub(1));
    let (w, h) = dds.dimensions(mip);
    let rgba = dds.decode_rgba(mip).ok()?;

    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut png_buf = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_buf),
        image::ImageFormat::Png,
    )
    .ok()?;

    Some(png_buf)
}

/// Load a normal map texture from a `_ddna.dds` file.
///
/// CryEngine stores normals in DXT5nm (BC3n) encoding:
/// - Alpha channel = X component of the normal
/// - Green channel = Y component of the normal
/// - Z is reconstructed: sqrt(1 - X² - Y²)
///
/// Output: RGB PNG where R=X, G=Y, B=Z in tangent space.
/// Load a normal map texture from a _ddna DDS file.
///
/// The DDS decoder outputs RGBA for BC5 with Z reconstructed, but degenerate
/// pixels (background/padding) can have Z≈0 which breaks Cycles. We re-read
/// R+G, ensure Z>0, and output a clean normal map PNG.
fn load_normal_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
    if tif_path.starts_with('$') {
        return None;
    }

    // Only load actual normal maps (_ddna/_ddn), not specular/other textures
    // that happen to be in TexSlot2.
    let lower = tif_path.to_lowercase();
    if !lower.contains("_ddna") && !lower.contains("_ddn.") && !lower.contains("_ddn_") {
        log::debug!("  skipping non-normal in TexSlot2: {tif_path}");
        return None;
    }

    let dds_path = tif_path
        .strip_suffix(".tif")
        .map(|base| format!("{base}.dds"))
        .unwrap_or_else(|| tif_path.to_string());

    let p4k_dds_path = datacore_path_to_p4k(&dds_path);
    let base_entry = p4k.entry_case_insensitive(&p4k_dds_path)?;
    let base_bytes = p4k.read(base_entry).ok()?;

    let sibling_reader = P4kSiblingReader {
        p4k,
        base_path: p4k_dds_path,
    };
    let dds = DdsFile::from_split(&base_bytes, &sibling_reader).ok()?;

    let format = starbreaker_dds::resolve_format(
        &dds.header.pixel_format,
        dds.dxt10_header.as_ref(),
    );
    let (dw, dh) = ({ dds.header.width }, { dds.header.height });
    log::debug!("  normal: {tif_path} → {format:?}, {dw}x{dh}");

    let mip = (mip_level as usize).min(dds.mip_count().saturating_sub(1));
    let (w, h) = dds.dimensions(mip);
    let rgba = dds.decode_rgba(mip).ok()?;

    // The DDS decoder already handles SNorm→unsigned conversion and Z reconstruction
    // in expand_normal_map. Output is [0..255] where 128 = zero, ready for glTF.
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut png_buf = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_buf),
        image::ImageFormat::Png,
    )
    .ok()?;

    Some(png_buf)
}

/// Extract per-pixel roughness from the alpha mips of a _ddna normal map DDS.
///
/// CryEngine stores smoothness in separate sibling files (.7a, .6a, ...) as BC4 compressed.
/// We convert smoothness → roughness (1-smoothness) and pack into a glTF metallicRoughness
/// texture: R=0, G=roughness, B=metallic(0), A=255.
fn load_roughness_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
    let dds_path = tif_path
        .strip_suffix(".tif")
        .map(|base| format!("{base}.dds"))
        .unwrap_or_else(|| tif_path.to_string());

    let p4k_path = datacore_path_to_p4k(&dds_path);
    let entry = p4k.entry_case_insensitive(&p4k_path)?;
    let base_bytes = p4k.read(entry).ok()?;
    let sibling_reader = P4kSiblingReader {
        p4k,
        base_path: p4k_path,
    };
    let dds = DdsFile::from_split(&base_bytes, &sibling_reader).ok()?;

    if !dds.has_alpha_mips() {
        return None;
    }

    let mip = (mip_level as usize).min(dds.alpha_mip_data.len().saturating_sub(1));
    let (w, h) = dds.dimensions(mip);

    let smoothness = dds.decode_alpha_mip(mip).ok()?;
    let pixel_count = (w * h) as usize;
    if smoothness.len() != pixel_count {
        return None;
    }

    // Pack into glTF metallicRoughness format: R=0, G=roughness, B=metallic(0), A=255
    let mut rgba = vec![0u8; pixel_count * 4];
    for i in 0..pixel_count {
        let roughness = 255 - smoothness[i]; // roughness = 1 - smoothness
        rgba[i * 4] = 0;           // R: unused
        rgba[i * 4 + 1] = roughness; // G: roughness
        rgba[i * 4 + 2] = 0;       // B: metallic = 0
        rgba[i * 4 + 3] = 255;     // A: unused
    }

    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut png_buf = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_buf),
        image::ImageFormat::Png,
    )
    .ok()?;

    Some(png_buf)
}

/// Resolve the MTL file path in P4k format from a MtlName string.
fn resolve_mtl_p4k_path(mtl_name: &str, p4k_geom_path: &str) -> String {
    if mtl_name.contains('/') || mtl_name.contains('\\') {
        format!("Data\\{}.mtl", mtl_name.replace('/', "\\"))
    } else {
        let dir = p4k_geom_path
            .rfind('\\')
            .map(|i| &p4k_geom_path[..i])
            .unwrap_or(p4k_geom_path);
        format!("{dir}\\{mtl_name}.mtl")
    }
}

/// Query landing gear geometry paths + bone names from VehicleLandingGearSystem.
///
/// `VehicleComponentParams.landingSystem` is a Reference to a `VehicleLandingGearSystem` record.
/// That record has `gears[]`, each with a geometry path (.skin) and a bone name.
fn query_landing_gear(db: &Database, record: &Record) -> Vec<(String, String)> {
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
fn resolve_material(
    p4k: &MappedP4k,
    datacore_material_path: &str,
    p4k_geom_path: &str,
    metadata_bytes: Option<&[u8]>,
) -> Option<mtl::MtlFile> {
    // 1. Try DataCore material path first
    if !datacore_material_path.is_empty() {
        let p4k_path = datacore_path_to_p4k(datacore_material_path);
        if let Some(mtl) = try_load_mtl(p4k, &p4k_path) {
            return Some(mtl);
        }
    }

    // 2. Use pre-loaded metadata companion for MtlName fallback
    let metadata = metadata_bytes?;
    let mtl_name = mtl::extract_mtl_name(metadata)?;
    let mtl_p4k_path = resolve_mtl_p4k_path(&mtl_name, p4k_geom_path);
    try_load_mtl(p4k, &mtl_p4k_path)
}

/// Query the default tint palette colors from a DataCore entity.
///
/// Strategy:
/// 1. Try querying through the entity's Reference path (follows the Reference to the
///    correct TintPaletteTree record, works when RootRecord is populated).
/// 2. Fallback: search for a TintPaletteTree record matching the entity name.
fn query_tint_palette(db: &Database, record: &Record) -> Option<mtl::TintPalette> {
    // Strategy 1: Query through the entity's Reference path.
    // This follows Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord
    // through the Reference to the TintPaletteTree and reads colors directly.
    let base = "Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord.root";
    if let Some(palette) = query_tint_from_path(db, record, base) {
        return Some(palette);
    }

    // Strategy 2: Find TintPaletteTree record by entity name convention.
    let entity_name = db.resolve_string2(record.name_offset);
    let short_name = entity_name.rsplit('.').next().unwrap_or(entity_name).to_lowercase();
    let tpt_si = db.struct_id("TintPaletteTree")?;
    // Find an exact match first (e.g., "rsi_zeus_cl"), not a substring match
    // that could pick up a paint variant like "aegs_gladius_black_grey_grey_geometric".
    let palette_record = db.records_of_type(tpt_si)
        .find(|r| {
            let name = db.resolve_string2(r.name_offset).to_lowercase();
            let rec_short = name.rsplit('.').next().unwrap_or(&name);
            rec_short == short_name
        })?;

    query_tint_from_record(db, palette_record)
}

/// Read tint palette colors from a path through an entity record.
fn query_tint_from_path(db: &Database, record: &Record, base: &str) -> Option<mtl::TintPalette> {
    let query_rgb = |entry: &str| -> [f32; 3] {
        let mut rgb = [0.5f32; 3];
        for (i, ch) in ["r", "g", "b"].iter().enumerate() {
            let path = format!("{base}.{entry}.tintColor.{ch}");
            if let Ok(compiled) = db.compile_path::<u8>(record.struct_id(), &path)
                && let Ok(Some(v)) = db.query_single::<u8>(&compiled, record)
            {
                rgb[i] = v as f32 / 255.0;
            }
        }
        rgb
    };

    // Quick check: can we even query this path?
    let test_path = format!("{base}.entryA.tintColor.r");
    let compiled = db.compile_path::<u8>(record.struct_id(), &test_path).ok()?;
    let _val = db.query_single::<u8>(&compiled, record).ok().flatten()?;

    Some(mtl::TintPalette {
        primary: query_rgb("entryA"),
        secondary: query_rgb("entryB"),
        tertiary: query_rgb("entryC"),
        glass: query_rgb("glassColor"),
    })
}

/// Read tint palette colors from a TintPaletteTree record directly.
fn query_tint_from_record(db: &Database, record: &Record) -> Option<mtl::TintPalette> {
    query_tint_from_path(db, record, "root")
}

fn try_load_mtl(p4k: &MappedP4k, p4k_path: &str) -> Option<mtl::MtlFile> {
    let entry = p4k.entry_case_insensitive(p4k_path)?;
    let data = p4k.read(entry).ok()?;
    mtl::parse_mtl(&data).ok()
}

/// Convert a DataCore file path to P4k format.
///
/// DataCore: `objects/ships/aegs/file.skin` (forward slashes, no prefix)
/// P4k:      `Data\Objects\Ships\AEGS\file.skin` (backslashes, `Data\` prefix)
///
/// Case mismatch is handled by `entry_case_insensitive` on the P4k side.
/// Transform a mesh's vertices by a skeleton bone's world transform (rotation + translation).
/// Used for CA_BONE CDF attachments that are authored in bone-local space.
fn transform_mesh_by_bone(mesh: &mut crate::Mesh, bone: &crate::skeleton::Bone) {
    let [qw, qx, qy, qz] = bone.world_rotation;
    let rot = glam::Quat::from_xyzw(qx, qy, qz, qw);
    let trans = glam::Vec3::from(bone.world_position);
    let affine = glam::Affine3A::from_rotation_translation(rot, trans);
    let mat3 = glam::Mat3A::from_quat(rot);

    for pos in &mut mesh.positions {
        let p = affine.transform_point3(glam::Vec3::from(*pos));
        *pos = p.into();
    }

    if let Some(ref mut normals) = mesh.normals {
        for n in normals {
            let v = mat3 * glam::Vec3::from(*n);
            *n = v.into();
        }
    }

    // Update bounding box by transforming all 8 corners
    let mn = glam::Vec3::from(mesh.model_min);
    let mx = glam::Vec3::from(mesh.model_max);
    let mut new_min = glam::Vec3::splat(f32::MAX);
    let mut new_max = glam::Vec3::splat(f32::MIN);
    for &x in &[mn.x, mx.x] {
        for &y in &[mn.y, mx.y] {
            for &z in &[mn.z, mx.z] {
                let t = affine.transform_point3(glam::Vec3::new(x, y, z));
                new_min = new_min.min(t);
                new_max = new_max.max(t);
            }
        }
    }
    mesh.model_min = new_min.into();
    mesh.model_max = new_max.into();
}

fn datacore_path_to_p4k(path: &str) -> String {
    // Some DataCore paths already include a "Data/" prefix — strip it to avoid "Data\Data\".
    let clean = path
        .strip_prefix("Data/")
        .or_else(|| path.strip_prefix("data/"))
        .or_else(|| path.strip_prefix("Data\\"))
        .or_else(|| path.strip_prefix("data\\"))
        .unwrap_or(path);
    format!("Data\\{}", clean.replace('/', "\\"))
}

/// Resolve a `.cdf` (CharacterDefinition) geometry path to the actual mesh path.
///
/// CDF files are CryXmlB documents that define a skeleton + skin attachments:
/// ```xml
/// <CharacterDefinition>
///   <Model File="path/to/skeleton.chr" />
///   <AttachmentList>
///     <Attachment Type="CA_SKIN" Binding="path/to/mesh.skin" ... />
///     <Attachment Type="CA_BONE" Binding="path/to/part.cgf" ... />
///   </AttachmentList>
/// </CharacterDefinition>
/// ```
///
/// Returns the `Binding` path of the first `CA_SKIN` attachment (the primary mesh).
/// Falls back to the first attachment with any `Binding` if no `CA_SKIN` is found.
/// A single geometry file to load, with optional bone attachment info.
struct GeometryPart {
    path: String,
    /// Bone name from CDF attachment (for CA_BONE placement). None for CA_SKIN.
    bone_name: Option<String>,
    /// Material override from CDF attachment. Takes priority over the DataCore material.
    material_override: Option<String>,
}

/// Result of resolving a geometry path — all mesh parts plus optional skeleton.
struct ResolvedGeometry {
    parts: Vec<GeometryPart>,
    /// Path to .chr skeleton (from CDF Model element). None for direct .skin/.cgf.
    skeleton_path: Option<String>,
}

/// Resolve a geometry path into the list of actual mesh files to load.
///
/// - `.skin`/`.cgf` etc -> single part
/// - `.cdf` -> parse CharacterDefinition, return all Attachment bindings
fn resolve_geometry_files(
    p4k: &MappedP4k,
    geometry_path: &str,
) -> Result<ResolvedGeometry, Error> {
    if !geometry_path.to_lowercase().ends_with(".cdf") {
        return Ok(ResolvedGeometry {
            parts: vec![GeometryPart {
                path: geometry_path.to_string(),
                bone_name: None,
                material_override: None,
            }],
            skeleton_path: None,
        });
    }

    let p4k_path = datacore_path_to_p4k(geometry_path);
    let entry = p4k
        .entry_case_insensitive(&p4k_path)
        .ok_or_else(|| Error::FileNotFoundInP4k {
            path: p4k_path.clone(),
        })?;
    let data = p4k.read(entry).map_err(Error::P4k)?;

    let xml = starbreaker_cryxml::from_bytes(&data)
        .map_err(|e| Error::Other(format!("Failed to parse CDF {geometry_path}: {e}")))?;

    let root = xml.root();
    let mut parts = Vec::new();
    let mut skeleton_path = None;

    for child in xml.node_children(root) {
        if xml.node_tag(child) == "Model" {
            let attrs: std::collections::HashMap<&str, &str> =
                xml.node_attributes(child).collect();
            if let Some(&file) = attrs.get("File") {
                if !file.is_empty() {
                    skeleton_path = Some(file.to_string());
                }
            }
        }
        if xml.node_tag(child) == "AttachmentList" {
            for attachment in xml.node_children(child) {
                if xml.node_tag(attachment) != "Attachment" {
                    continue;
                }
                let attrs: std::collections::HashMap<&str, &str> =
                    xml.node_attributes(attachment).collect();
                if let Some(&binding) = attrs.get("Binding") {
                    if !binding.is_empty() {
                        // Use BoneName (CA_BONE rigid attachment) for bone transform.
                        // CA_SKIN attachments don't have BoneName — they share
                        // the skeleton's coordinate space and merge at origin.
                        parts.push(GeometryPart {
                            path: binding.to_string(),
                            bone_name: attrs.get("BoneName").map(|s| s.to_string()),
                            material_override: attrs.get("Material")
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
    }

    if parts.is_empty() {
        return Err(Error::Other(format!(
            "CDF {geometry_path} has no attachments"
        )));
    }

    Ok(ResolvedGeometry {
        parts,
        skeleton_path,
    })
}

/// Load a single mesh file from the P4k, with LOD resolution and material/NMC/texture loading.
/// Resolve the companion file path (.cgam/.skinm) for a geometry path, with LOD fallback.
fn resolve_companion_path(p4k: &MappedP4k, p4k_geom_path: &str, lod_level: u32) -> String {
    if lod_level > 0 {
        let lod_geom = if let Some(dot) = p4k_geom_path.rfind('.') {
            format!(
                "{}_lod{}{}",
                &p4k_geom_path[..dot],
                lod_level,
                &p4k_geom_path[dot..]
            )
        } else {
            format!("{}_lod{}", p4k_geom_path, lod_level)
        };
        let lod_companion = format!("{lod_geom}m");
        if p4k.entry_case_insensitive(&lod_companion).is_some() {
            lod_companion
        } else {
            format!("{p4k_geom_path}m")
        }
    } else {
        format!("{p4k_geom_path}m")
    }
}

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

    // Query vehicleDefinition and modification from DataCore.
    let veh_def = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[VehicleComponentParams].vehicleDefinition",
        )
        .ok()
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten());

    let veh_def = match veh_def {
        Some(ref s) if !s.is_empty() => s,
        _ => return invisible,
    };

    let modification = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[VehicleComponentParams].modification",
        )
        .ok()
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten())
        .unwrap_or_default();

    // Load base vehicle XML from p4k.
    let base_p4k = datacore_path_to_p4k(veh_def);
    if let Some(data) = p4k
        .entry_case_insensitive(&base_p4k)
        .and_then(|e| p4k.read(e).ok())
    {
        if let Ok(xml) = starbreaker_cryxml::from_bytes(&data) {
            collect_invisible_ports_from_xml(&xml, xml.root(), &mut invisible);
        }
    }

    // Load modification XML if present.
    if !modification.is_empty() {
        // Base: "scripts/.../foo.xml" → Modification: "scripts/.../Modifications/foo_bar.xml"
        if let Some(slash) = veh_def.rfind('/').or_else(|| veh_def.rfind('\\')) {
            let dir = &veh_def[..slash];
            let stem = &veh_def[slash + 1..].trim_end_matches(".xml").trim_end_matches(".XML");
            let mod_path = format!("{dir}/Modifications/{stem}_{modification}.xml");
            let mod_p4k = datacore_path_to_p4k(&mod_path);
            if let Some(data) = p4k
                .entry_case_insensitive(&mod_p4k)
                .and_then(|e| p4k.read(e).ok())
            {
                if let Ok(xml) = starbreaker_cryxml::from_bytes(&data) {
                    // Modification overrides: re-collect, allowing override of base flags.
                    collect_invisible_ports_from_xml_override(&xml, xml.root(), &mut invisible);
                }
            }
        }
    }

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

/// Load NMC and material from the metadata file (.cga/.cgf/.skin).
/// Never fails — returns None for each if the file is missing.
fn load_nmc_and_material(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
) -> (Option<nmc::NodeMeshCombo>, Option<mtl::MtlFile>) {
    let p4k_geom_path = datacore_path_to_p4k(geometry_path);
    let metadata_bytes = p4k
        .entry_case_insensitive(&p4k_geom_path)
        .and_then(|entry| p4k.read(entry).ok());

    let mtl_file = resolve_material(p4k, material_path, &p4k_geom_path, metadata_bytes.as_deref());

    let nmc = metadata_bytes
        .as_deref()
        .and_then(nmc::parse_nmc_full)
        .map(|(nodes, mat_indices)| nmc::NodeMeshCombo {
            nodes,
            material_indices: mat_indices,
        });

    (nmc, mtl_file)
}

fn load_single_mesh(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<
    (
        crate::Mesh,
        Option<mtl::MtlFile>,
        Option<MaterialTextures>,
        Option<nmc::NodeMeshCombo>,
    ),
    Error,
> {
    let p4k_geom_path = datacore_path_to_p4k(geometry_path);
    let companion_path = resolve_companion_path(p4k, &p4k_geom_path, opts.lod_level);

    let entry = p4k
        .entry_case_insensitive(&companion_path)
        .ok_or_else(|| Error::FileNotFoundInP4k {
            path: companion_path.clone(),
        })?;
    let mesh_bytes = p4k.read(entry).map_err(Error::P4k)?;

    let (nmc, mtl_file) = load_nmc_and_material(p4k, geometry_path, material_path);

    let textures = if !opts.include_textures {
        None
    } else {
        mtl_file
            .as_ref()
            .map(|mtl| load_material_textures(p4k, mtl, opts.texture_mip, png_cache, opts.include_normals, opts.experimental_textures))
    };

    let mesh = if use_model_bbox {
        crate::parse_skin_model_bbox(&mesh_bytes)?
    } else {
        crate::parse_skin(&mesh_bytes)?
    };

    Ok((mesh, mtl_file, textures, nmc))
}

/// Export a set of standalone socpak files to a single GLB.
///
/// This is for locations (space stations, landing zones) that aren't entities
/// with geometry but are composed entirely of socpak containers.
/// Export socpak containers directly as a GLB (no root entity mesh).
pub fn socpaks_to_glb(
    db: &Database,
    p4k: &MappedP4k,
    socpak_paths: &[String],
    opts: &ExportOptions,
) -> Result<Vec<u8>, Error> {
    use crate::socpak;

    let identity: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    let mut payloads = Vec::new();
    for socpak_path in socpak_paths {
        match socpak::load_interior_from_socpak(p4k, socpak_path, identity) {
            Ok(p) => payloads.push(p),
            Err(e) => log::warn!("failed to load {socpak_path}: {e}"),
        }
    }

    let interiors = build_interiors_from_payloads(db, &payloads, opts.include_lights);

    let no_tex_opts = ExportOptions {
        include_textures: false,
        ..opts.clone()
    };
    let mut no_tex: Box<dyn FnMut(Option<&crate::mtl::MtlFile>) -> Option<MaterialTextures>> =
        Box::new(|_| None);
    let mut interior_png_cache = PngCache::new();
    let mut interior_mesh_loader =
        |entry: &InteriorCgfEntry| -> Option<(crate::Mesh, Option<mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
            match export_cgf_from_path(
                p4k,
                &entry.cgf_path,
                entry.material_path.as_deref(),
                &no_tex_opts,
                &mut interior_png_cache,
            ) {
                Ok((mesh, mtl, _tex, nmc, _palette, _, _, _bones)) => {
                    let needs_bake = mesh.scaling_min.iter().zip(&mesh.model_min)
                        .chain(mesh.scaling_max.iter().zip(&mesh.model_max))
                        .any(|(s, m)| (s - m).abs() > 0.01);
                    let mesh = if needs_bake {
                        bake_nmc_absolute(mesh, nmc.as_ref())
                    } else {
                        mesh
                    };
                    Some((mesh, mtl, nmc))
                }
                Err(e) => {
                    log::warn!("failed to load CGF {}: {e}", entry.cgf_path);
                    None
                }
            }
        };

    crate::gltf::write_glb(
        crate::gltf::GlbInput {
            root_mesh: None,
            root_materials: None,
            root_textures: None,
            root_nmc: None,
            root_palette: None,
            skeleton_bones: Vec::new(),
            children: Vec::new(),
            interiors,
        },
        &mut crate::gltf::GlbLoaders {
            load_textures: &mut no_tex,
            load_interior_mesh: &mut interior_mesh_loader,
        },
        &crate::gltf::GlbOptions {
            include_tangents: false,
            experimental_textures: opts.experimental_textures,
            metadata: crate::gltf::GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: crate::gltf::ExportOptionsMetadata {
                    texture_mip: opts.texture_mip,
                    lod_level: opts.lod_level,
                    include_interior: opts.include_interior,
                    include_tangents: false,
                    include_lights: opts.include_lights,
                    include_textures: false,
                    include_materials: opts.include_materials,
                },
            },
            fallback_palette: None,
        },
    )
}

/// Write NMC node properties for a geometry path into the JSON output.
fn dump_nmc_nodes(out: &mut String, key: &str, p4k: &MappedP4k, geom: &str, mtl: &str) {
    use std::fmt::Write;
    if geom.is_empty() { return; }
    let (nmc, _) = load_nmc_and_material(p4k, geom, mtl);
    if let Some(ref nmc) = nmc {
        let _ = write!(out, "  \"{key}\": [\n");
        for node in &nmc.nodes {
            let _ = write!(out, "    {{\"node\": {:?}, \"type\": {}", node.name, node.geometry_type);
            // Include bone_to_world for non-identity transforms (helpers, attachment points)
            let b = &node.bone_to_world;
            let is_identity = (b[0][0] - 1.0).abs() < 0.001 && (b[1][1] - 1.0).abs() < 0.001
                && (b[2][2] - 1.0).abs() < 0.001
                && b[0][3].abs() < 0.001 && b[1][3].abs() < 0.001 && b[2][3].abs() < 0.001
                && b[0][1].abs() < 0.001 && b[0][2].abs() < 0.001
                && b[1][0].abs() < 0.001 && b[1][2].abs() < 0.001
                && b[2][0].abs() < 0.001 && b[2][1].abs() < 0.001;
            if !is_identity {
                let _ = write!(out, ", \"bone_to_world\": [[{:.4},{:.4},{:.4},{:.4}],[{:.4},{:.4},{:.4},{:.4}],[{:.4},{:.4},{:.4},{:.4}]]",
                    b[0][0], b[0][1], b[0][2], b[0][3],
                    b[1][0], b[1][1], b[1][2], b[1][3],
                    b[2][0], b[2][1], b[2][2], b[2][3]);
            }
            if !node.properties.is_empty() {
                let _ = write!(out, ", \"props\": {{");
                for (i, (k, v)) in node.properties.iter().enumerate() {
                    if i > 0 { let _ = write!(out, ", "); }
                    let _ = write!(out, "{:?}: {:?}", k, v);
                }
                let _ = write!(out, "}}");
            }
            let _ = write!(out, "}},\n");
        }
        let _ = write!(out, "  ],\n");
    }
}

/// Dump the full geometry hierarchy (loadout + interiors) as a JSON string.
/// Includes NMC per-node properties for each geometry file.
pub fn dump_hierarchy(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
) -> String {
    use std::fmt::Write;

    let mut out = String::from("{\n");

    // Root entity
    let geom_compiled = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
        )
        .ok();
    let root_geom = geom_compiled
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten())
        .unwrap_or_default();
    let mtl_compiled = db.compile_path::<String>(
        record.struct_id(),
        "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
    ).ok();
    let root_mtl = mtl_compiled
        .and_then(|c| db.query_single::<String>(&c, record).ok().flatten())
        .unwrap_or_default();
    let _ = write!(
        out,
        "  \"root\": {{\n    \"entity\": {:?},\n    \"geometry\": {:?}\n  }},\n",
        tree.root.entity_name, root_geom
    );

    // NMC nodes for root geometry
    dump_nmc_nodes(&mut out, "root_nmc", p4k, &root_geom, &root_mtl);

    // Load invisible port flags from vehicle XML.
    let invisible_ports = load_invisible_ports(db, p4k, record);

    // Loadout children
    let _ = write!(out, "  \"loadout\": [\n");
    fn dump_loadout_nodes(
        out: &mut String,
        p4k: &MappedP4k,
        nodes: &[starbreaker_datacore::loadout::LoadoutNode],
        parent: &str,
        depth: usize,
        invisible_ports: &std::collections::HashSet<String>,
    ) {
        let indent = "    ".repeat(depth + 1);
        for node in nodes {
            let geom = node.geometry_path.as_deref().unwrap_or("");
            let is_invisible = invisible_ports.contains(&node.item_port_name);
            let _ = write!(out, "{indent}{{\n");
            let _ = write!(out, "{indent}  \"entity\": {:?},\n", node.entity_name);
            let _ = write!(out, "{indent}  \"port\": {:?},\n", node.item_port_name);
            let _ = write!(out, "{indent}  \"parent\": {:?},\n", parent);
            if is_invisible {
                let _ = write!(out, "{indent}  \"invisible\": true,\n");
            }
            let _ = write!(out, "{indent}  \"geometry\": {:?}", geom);
            // Include NMC properties for this node's geometry
            if !geom.is_empty() {
                let mat = node.material_path.as_deref().unwrap_or("");
                let (nmc, _) = load_nmc_and_material(p4k, geom, mat);
                if let Some(ref nmc) = nmc {
                    let nodes_with_props: Vec<_> = nmc.nodes.iter()
                        .filter(|n| !n.properties.is_empty())
                        .collect();
                    if !nodes_with_props.is_empty() {
                        let _ = write!(out, ",\n{indent}  \"nmc_properties\": [\n");
                        for n in &nodes_with_props {
                            let _ = write!(out, "{indent}    {{\"node\": {:?}, \"props\": {{", n.name);
                            for (i, (k, v)) in n.properties.iter().enumerate() {
                                if i > 0 { let _ = write!(out, ", "); }
                                let _ = write!(out, "{:?}: {:?}", k, v);
                            }
                            let _ = write!(out, "}}}},\n");
                        }
                        let _ = write!(out, "{indent}  ]");
                    }
                }
            }
            if !node.children.is_empty() {
                let _ = write!(out, ",\n{indent}  \"children\": [\n");
                dump_loadout_nodes(out, p4k, &node.children, &node.entity_name, depth + 2, invisible_ports);
                let _ = write!(out, "{indent}  ]\n");
            } else {
                let _ = write!(out, "\n");
            }
            let _ = write!(out, "{indent}}},\n");
        }
    }
    dump_loadout_nodes(&mut out, p4k, &tree.root.children, &tree.root.entity_name, 0, &invisible_ports);
    let _ = write!(out, "  ],\n");

    // Interior containers
    let interiors = load_interiors(db, p4k, record, &ExportOptions::default());
    let _ = write!(out, "  \"interiors\": [\n");
    for container in &interiors.containers {
        let _ = write!(
            out,
            "    {{\n      \"container\": {:?},\n      \"meshes\": [\n",
            container.name
        );
        for &(cgf_idx, ref transform) in &container.placements {
            let entry = &interiors.unique_cgfs[cgf_idx];
            let tx = transform[3][0];
            let ty = transform[3][1];
            let tz = transform[3][2];
            // Extract scale from rotation columns
            let sx = (transform[0][0] * transform[0][0]
                + transform[0][1] * transform[0][1]
                + transform[0][2] * transform[0][2])
                .sqrt();
            let sy = (transform[1][0] * transform[1][0]
                + transform[1][1] * transform[1][1]
                + transform[1][2] * transform[1][2])
                .sqrt();
            let sz = (transform[2][0] * transform[2][0]
                + transform[2][1] * transform[2][1]
                + transform[2][2] * transform[2][2])
                .sqrt();
            let _ = write!(
                out,
                "        {{\"cgf\": {:?}, \"pos\": [{tx:.2}, {ty:.2}, {tz:.2}], \"scale\": [{sx:.3}, {sy:.3}, {sz:.3}]",
                entry.cgf_path
            );
            if let Some(ref mtl) = entry.material_path {
                let _ = write!(out, ", \"material\": {:?}", mtl);
            }
            let _ = write!(out, "}},\n");
        }
        let _ = write!(
            out,
            "      ],\n      \"lights\": {}\n    }},\n",
            container.lights.len()
        );
    }
    let _ = write!(out, "  ]\n}}\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_mtl_p4k_path_full_path() {
        assert_eq!(
            resolve_mtl_p4k_path(
                "Objects/buildingsets/human/foo/bar",
                "Data\\Objects\\buildingsets\\human\\foo\\bar.cgf"
            ),
            "Data\\Objects\\buildingsets\\human\\foo\\bar.mtl"
        );
    }

    #[test]
    fn resolve_mtl_p4k_path_short_name() {
        assert_eq!(
            resolve_mtl_p4k_path("teapot", "Data\\objects\\default\\teapot.cgf"),
            "Data\\objects\\default\\teapot.mtl"
        );
    }

    #[test]
    fn test_datacore_path_to_p4k_simple() {
        assert_eq!(
            datacore_path_to_p4k("objects/ships/aegs/aegs_gladius.skin"),
            "Data\\objects\\ships\\aegs\\aegs_gladius.skin"
        );
    }

    #[test]
    fn test_datacore_path_to_p4k_no_slashes() {
        assert_eq!(datacore_path_to_p4k("file.skin"), "Data\\file.skin");
    }

    #[test]
    fn test_datacore_path_to_p4k_deep() {
        assert_eq!(
            datacore_path_to_p4k("a/b/c/d/e.cgf"),
            "Data\\a\\b\\c\\d\\e.cgf"
        );
    }
}
