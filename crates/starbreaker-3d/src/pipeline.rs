use std::collections::HashSet;
use std::str::FromStr;

use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_datacore::database::Database;
use starbreaker_datacore::error::QueryError;
use starbreaker_datacore::types::Record;
use starbreaker_dds::{DdsFile, ReadSibling};
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::mtl;
use crate::nmc;
use crate::types::{MaterialTextures, TextureTransformInfo};

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

type InteriorMeshAsset = (
    crate::Mesh,
    Option<mtl::MtlFile>,
    Option<nmc::NodeMeshCombo>,
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PreloadedTextureKey {
    material_source: String,
    palette_hash: u64,
}

/// How materials are represented in the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialMode {
    /// No material data. Plain white surfaces.
    None,
    /// Tint colors from palette/layers, NoDraw hidden, glass marked as transmissive.
    /// Material names and full MTL properties preserved in glTF extras.
    /// Deterministic — only acts on unambiguous shader signals.
    Colors,
    /// Colors + diffuse/normal/roughness textures for materials with direct texture slots.
    /// Tangents included automatically. Deterministic.
    Textures,
    /// Everything we can extract, correctness not guaranteed.
    /// Includes layer textures, alpha mode inference, decal classification, roughness defaults.
    All,
}

/// Top-level export kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    /// Bundled scene export using a single-file artifact such as GLB.
    Bundled,
    /// Decomposed scene export using reusable assets and sidecar metadata.
    Decomposed,
}

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Standard render export (GLB with materials).
    Glb,
    /// 3D print export (STL, no materials, no decals, glass solid, no interior).
    Stl,
}

/// Options for controlling the export pipeline.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Top-level export kind.
    pub kind: ExportKind,
    /// Output format.
    pub format: ExportFormat,
    /// Material detail level.
    pub material_mode: MaterialMode,
    /// Include attached items (weapons, thrusters, landing gear, seats, etc.)
    pub include_attachments: bool,
    /// Include interior rooms from socpak object containers.
    pub include_interior: bool,
    /// Include lights from interior object containers (KHR_lights_punctual).
    pub include_lights: bool,
    /// Include NoDraw submeshes and sidecar entries in decomposed exports.
    pub include_nodraw: bool,
    /// LOD level (0 = highest detail, 1+ = lower).
    pub lod_level: u32,
    /// Texture mip level (0 = full resolution, 2 = 1/4 res, 4 = 1/16 res).
    pub texture_mip: u32,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            kind: ExportKind::Bundled,
            format: ExportFormat::Glb,
            material_mode: MaterialMode::Textures,
            include_attachments: true,
            include_interior: true,
            include_lights: true,
            include_nodraw: false,
            lod_level: 1,
            texture_mip: 2,
        }
    }
}

impl MaterialMode {
    pub fn include_materials(&self) -> bool {
        !matches!(self, MaterialMode::None)
    }
    pub fn include_textures(&self) -> bool {
        matches!(self, MaterialMode::Textures | MaterialMode::All)
    }
    pub fn include_tangents(&self) -> bool {
        matches!(self, MaterialMode::Textures | MaterialMode::All)
    }
    pub fn include_normals(&self) -> bool {
        matches!(self, MaterialMode::Textures | MaterialMode::All)
    }
    pub fn experimental(&self) -> bool {
        matches!(self, MaterialMode::All)
    }
}

impl ExportFormat {
    pub fn is_stl(&self) -> bool {
        matches!(self, ExportFormat::Stl)
    }
}

/// Placeholder for a future decomposed export package.
#[derive(Debug, Clone, Default)]
pub struct DecomposedExport {
    pub files: Vec<ExportedFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportedFileKind {
    PackageManifest,
    MaterialSidecar,
    MeshAsset,
    TextureAsset,
}

impl ExportedFileKind {
    pub fn is_mesh_or_texture_asset(self) -> bool {
        matches!(self, Self::MeshAsset | Self::TextureAsset)
    }
}

#[derive(Debug, Clone)]
pub struct ExportedFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
    pub kind: ExportedFileKind,
}

/// Result of exporting an entity record.
pub struct ExportResult {
    /// The top-level export kind used for this result.
    pub kind: ExportKind,
    /// The requested bundled output format when `kind` is `Bundled`.
    pub format: ExportFormat,
    /// The bundled artifact bytes for the current export path.
    ///
    /// This remains named `glb` for compatibility with the existing bundled API.
    pub glb: Vec<u8>,
    /// Placeholder for future decomposed results.
    pub decomposed: Option<DecomposedExport>,
    /// The geometry file path from DataCore (e.g., "objects/ships/aegs/aegs_gladius.skin").
    pub geometry_path: String,
    /// The material file path from DataCore (e.g., "objects/ships/aegs/aegs_gladius.mtl").
    pub material_path: String,
}

impl ExportResult {
    pub fn bundled_bytes(&self) -> Option<&[u8]> {
        if self.kind == ExportKind::Bundled && !self.glb.is_empty() {
            Some(self.glb.as_slice())
        } else {
            None
        }
    }
}

fn ensure_supported_export_kind(opts: &ExportOptions) -> Result<(), Error> {
    match opts.kind {
        ExportKind::Bundled | ExportKind::Decomposed => Ok(()),
    }
}

fn ensure_supported_export_format(opts: &ExportOptions) -> Result<(), Error> {
    if opts.format == ExportFormat::Glb {
        Ok(())
    } else {
        Err(Error::UnsupportedExportFormat(format!("{:?}", opts.format)))
    }
}

fn ensure_supported_export_options(opts: &ExportOptions) -> Result<(), Error> {
    ensure_supported_export_kind(opts)?;
    ensure_supported_export_format(opts)
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

    if !opts.material_mode.include_materials() {
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

    if !opts.material_mode.include_materials() {
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
    let no_tex_opts = ExportOptions { material_mode: MaterialMode::Colors, ..opts.clone() };
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
    assemble_glb_with_loadout_with_progress(db, p4k, record, tree, opts, None, None)
}

pub fn assemble_glb_with_loadout_with_progress(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<ExportResult, Error> {
    ensure_supported_export_options(opts)?;

    use crate::types::EntityPayload;

    report_progress(progress, 0.02, "Resolving loadout");
    let payload_material_mode = if opts.kind == ExportKind::Decomposed {
        MaterialMode::Colors
    } else {
        opts.material_mode
    };
    let payload_opts = ExportOptions {
        material_mode: payload_material_mode,
        ..opts.clone()
    };

    log::info!("[mem-pipeline] resolving loadout meshes...");
    let resolved = resolve_loadout_meshes(db, p4k, record, tree, &payload_opts)?;
    log::info!("[mem-pipeline] resolved: {} children", resolved.children.len());
    report_progress(progress, 0.14, "Exporting root mesh");

    // Export root entity (mesh + textures).
    let (root_mesh, root_mtl, root_tex, _, root_palette, geometry_path, material_path, root_bones) =
        export_entity_payload(db, p4k, record, &payload_opts)?;
    log::info!("[mem-pipeline] root exported: {} verts", root_mesh.positions.len());

    // Check for equipped paint item and resolve its SubGeometry palette/material override.
    let (root_palette, root_mtl) = resolve_paint_override(
        db, p4k, record, &tree.root, root_palette, root_mtl,
    );

    // Load landing gear as separate child entities attached to NMC nodes.
    // Landing gear CDF geometry attaches to NMC helper bones (e.g. hardpoint_landing_gear_front).
    // Adding them as EntityPayloads lets the existing scene graph handle positioning.
    // Children skip textures to save memory, but never exceed the user's material mode.
    let child_payload_material_mode = if opts.kind == ExportKind::Decomposed {
        MaterialMode::Colors
    } else {
        opts.material_mode
    };
    let child_material_mode = match opts.material_mode {
        _ if opts.kind == ExportKind::Decomposed => MaterialMode::Colors,
        MaterialMode::None => MaterialMode::None,
        _ => MaterialMode::Colors,
    };
    let child_opts = ExportOptions {
        material_mode: child_material_mode,
        ..payload_opts.clone()
    };
    let gear_parts = query_landing_gear(db, record);
    let mut child_payloads: Vec<EntityPayload> = Vec::new();
    report_progress(progress, 0.28, "Flattening attachments");
    if opts.include_attachments {
        for (gear_path, bone_name) in &gear_parts {
            match export_entity_from_paths(p4k, gear_path, "", &child_opts) {
                Ok((gear_mesh, gear_mtl, _, gear_nmc, _, gear_geometry_path, gear_material_path, _)) => {
                    let verts = gear_mesh.positions.len();
                    let textures = if child_payload_material_mode.include_textures() {
                        gear_mtl.as_ref().map(|materials| {
                            let mut png_cache = PngCache::new();
                            load_material_textures(
                                p4k,
                                materials,
                                root_palette.as_ref(),
                                opts.texture_mip,
                                &mut png_cache,
                                child_payload_material_mode.include_normals(),
                                child_payload_material_mode.experimental(),
                            )
                        })
                    } else {
                        None
                    };
                    child_payloads.push(EntityPayload {
                        mesh: gear_mesh,
                        materials: gear_mtl,
                        textures,
                        nmc: gear_nmc,
                        palette: root_palette.clone(),
                        geometry_path: gear_geometry_path,
                        material_path: gear_material_path,
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
            &child_opts,
            child_payload_material_mode,
            existing_asset_paths,
            &mut child_payloads,
        );
    }
    let total_child_verts: usize = child_payloads.iter().map(|c| c.mesh.positions.len()).sum();
    log::info!("[mem-pipeline] flattened: {} payloads, {} total verts", child_payloads.len(), total_child_verts);
    report_progress(progress, 0.42, "Discovering interiors");

    // Interior discovery (no mesh loading — JIT during GLB packing).
    let loaded_interiors = if opts.include_interior && !opts.format.is_stl() {
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

    let mut preloaded_interior_meshes = if opts.kind == ExportKind::Bundled {
        preload_interior_meshes(&loaded_interiors, p4k, &child_opts)
    } else {
        Vec::new()
    };
    if !preloaded_interior_meshes.is_empty() {
        log::info!(
            "[mem-pipeline] preloaded interior meshes: {}/{}",
            preloaded_interior_meshes
                .iter()
                .filter(|asset| asset.is_some())
                .count(),
            preloaded_interior_meshes.len()
        );
    }
    let preloaded_interior_textures = if opts.kind == ExportKind::Bundled {
        preload_interior_textures(
            &loaded_interiors,
            &preloaded_interior_meshes,
            root_palette.as_ref(),
            p4k,
            opts,
        )
    } else {
        std::collections::HashMap::new()
    };
    if !preloaded_interior_textures.is_empty() {
        log::info!(
            "[mem-pipeline] preloaded interior texture sets: {}",
            preloaded_interior_textures.len()
        );
    }
    report_progress(progress, 0.60, if opts.kind == ExportKind::Decomposed {
        "Building structured package"
    } else {
        "Packing GLB"
    });
    let preloaded_interior_mesh_indices: std::collections::HashMap<String, usize> =
        loaded_interiors
            .unique_cgfs
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.cgf_path.clone(), index))
            .collect();

    // Texture loading callback: root/child entities fall back to the normal JIT
    // path, while bundled interior meshes can consume preloaded texture sets.
    let mut png_cache = PngCache::new();
    let mut tex_loader: Box<
        dyn FnMut(
            Option<&crate::mtl::MtlFile>,
            Option<&crate::mtl::TintPalette>,
        ) -> Option<MaterialTextures>,
    > =
        if !opts.material_mode.include_textures() {
            Box::new(|_, _| None)
        } else {
            let mip = opts.texture_mip;
            let include_normals = opts.material_mode.include_normals();
            let experimental_textures = opts.material_mode.experimental();
            Box::new(move |mtl: Option<&crate::mtl::MtlFile>, palette: Option<&crate::mtl::TintPalette>| {
                if let Some(material_source) = mtl.and_then(|materials| materials.source_path.as_ref()) {
                    let cache_key = PreloadedTextureKey {
                        material_source: material_source.to_ascii_lowercase(),
                        palette_hash: tint_palette_hash(palette),
                    };
                    if let Some(textures) = preloaded_interior_textures.get(&cache_key).cloned() {
                        return Some(textures);
                    }
                }

                mtl.map(|m| {
                    load_material_textures(
                        p4k,
                        m,
                        palette,
                        mip,
                        &mut png_cache,
                        include_normals,
                        experimental_textures,
                    )
                })
            })
        };

    let mut interior_png_cache = PngCache::new();
    // Bundled GLB exports preload unique interior CGFs in parallel, then fall
    // back to on-demand loading for any cache misses and for decomposed exports.
    let mut interior_mesh_loader =
        |entry: &crate::pipeline::InteriorCgfEntry| -> Option<(crate::Mesh, Option<mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
            if let Some(&index) = preloaded_interior_mesh_indices.get(&entry.cgf_path) {
                if let Some(asset) = preloaded_interior_meshes
                    .get_mut(index)
                    .and_then(Option::take)
                {
                    return Some(asset);
                }
            }

            load_interior_mesh_asset(p4k, entry, &child_opts, &mut interior_png_cache)
        };

    if opts.kind == ExportKind::Decomposed {
        let decomposed_progress = progress.map(|progress| progress.sub(0.60, 0.90));
        let decomposed = crate::decomposed::write_decomposed_export(
            p4k,
            crate::decomposed::DecomposedInput {
                entity_name: resolved.entity_name.clone(),
                geometry_path: geometry_path.clone(),
                material_path: material_path.clone(),
                root_mesh,
                root_materials: root_mtl,
                root_nmc: resolved.nmc,
                root_palette: root_palette.clone(),
                root_bones,
                children: child_payloads,
                interiors: loaded_interiors,
            },
            opts,
            decomposed_progress.as_ref(),
            existing_asset_paths,
            &mut interior_mesh_loader,
        )?;

        report_progress(progress, 0.90, "Writing structured package");

        return Ok(ExportResult {
            kind: opts.kind,
            format: opts.format,
            glb: Vec::new(),
            decomposed: Some(decomposed),
            geometry_path,
            material_path,
        });
    }

    let glb_progress = progress.map(|progress| progress.sub(0.60, 0.90));
    let glb = crate::gltf::write_glb_with_progress(
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
            material_mode: opts.material_mode,
            preserve_textureless_decal_primitives: false,
            metadata: crate::gltf::GlbMetadata {
                entity_name: Some(resolved.entity_name.clone()),
                geometry_path: Some(geometry_path.clone()),
                material_path: Some(material_path.clone()),
                export_options: crate::gltf::ExportOptionsMetadata {
                    kind: format!("{:?}", opts.kind),
                    material_mode: format!("{:?}", opts.material_mode),
                    format: format!("{:?}", opts.format),
                    lod_level: opts.lod_level,
                    texture_mip: opts.texture_mip,
                    include_attachments: opts.include_attachments,
                    include_interior: opts.include_interior,
                },
            },
            fallback_palette: root_palette,
        },
        glb_progress.as_ref(),
    )?;

    report_progress(progress, 0.90, "Writing bundled file");

    Ok(ExportResult {
        kind: opts.kind,
        format: opts.format,
        glb,
        decomposed: None,
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
) -> Option<(
    crate::types::Mesh,
    Option<mtl::MtlFile>,
    Option<nmc::NodeMeshCombo>,
    Option<mtl::TintPalette>,
    Vec<crate::skeleton::Bone>,
    String,
    String,
)> {
    let result = if child.geometry_path.is_some() {
        let gp = child.geometry_path.as_deref().unwrap_or("");
        let mp = child.material_path.as_deref().unwrap_or("");
        export_entity_from_paths(p4k, gp, mp, opts)
            .map_err(|e| {
                log::warn!("  {} -> load from paths failed: {e}", child.entity_name);
                e
            })
            .or_else(|_| export_entity_payload(db, p4k, &child.record, opts))
    } else {
        export_entity_payload(db, p4k, &child.record, opts)
    };

    result.ok().map(|(mesh, mtl, _tex, nmc, palette, geometry_path, material_path, bones)| {
        (mesh, mtl, nmc, palette, bones, geometry_path, material_path)
    })
}

struct ChildPayloadSpec {
    child: crate::types::ResolvedNode,
    parent_entity_name: String,
    parent_node_name: String,
    no_rotation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChildPayloadCacheKey {
    record_id: starbreaker_datacore::types::CigGuid,
    geometry_path: Option<String>,
    material_path: Option<String>,
}

#[derive(Clone)]
struct LoadedChildPayload {
    mesh: crate::types::Mesh,
    materials: Option<mtl::MtlFile>,
    textures: Option<MaterialTextures>,
    nmc: Option<nmc::NodeMeshCombo>,
    palette: Option<mtl::TintPalette>,
    bones: Vec<crate::skeleton::Bone>,
    geometry_path: String,
    material_path: String,
}

fn collect_child_payload_specs(
    children: &[crate::types::ResolvedNode],
    parent_entity_name: &str,
    override_attachment: Option<(&str, bool)>,
    out: &mut Vec<ChildPayloadSpec>,
) {
    for child in children {
        let (attach_name, no_rotation) = match override_attachment {
            Some((name, parent_no_rot)) => (name.to_string(), child.no_rotation || parent_no_rot),
            None => (child.attachment_name.clone(), child.no_rotation),
        };

        let child_creates_nodes = child.has_geometry || child.nmc.is_some();
        if child_creates_nodes {
            out.push(ChildPayloadSpec {
                child: child.clone_payload_source(),
                parent_entity_name: parent_entity_name.to_string(),
                parent_node_name: attach_name,
                no_rotation,
            });
            collect_child_payload_specs(&child.children, &child.entity_name, None, out);
        } else {
            collect_child_payload_specs(
                &child.children,
                parent_entity_name,
                Some((&child.attachment_name, child.no_rotation)),
                out,
            );
        }
    }
}

fn empty_child_mesh() -> crate::types::Mesh {
    crate::types::Mesh {
        positions: Vec::new(),
        indices: Vec::new(),
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: Vec::new(),
        model_min: [0.0; 3],
        model_max: [0.0; 3],
        scaling_min: [0.0; 3],
        scaling_max: [0.0; 3],
    }
}

fn normalize_decomposed_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| p4k_path.replace('\\', "/"))
}

fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
}

fn decomposed_mesh_asset_path(p4k: &MappedP4k, geometry_path: &str) -> Option<String> {
    if geometry_path.is_empty() {
        None
    } else {
        Some(replace_extension(&normalize_decomposed_source_path(p4k, geometry_path), ".glb"))
    }
}

fn build_child_payload_cache_key(child: &crate::types::ResolvedNode) -> ChildPayloadCacheKey {
    ChildPayloadCacheKey {
        record_id: child.record.id,
        geometry_path: child.geometry_path.clone(),
        material_path: child.material_path.clone(),
    }
}

fn load_child_payload_asset(
    child: &crate::types::ResolvedNode,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<LoadedChildPayload> {
    if mesh_opts.kind == ExportKind::Decomposed {
        if let Some(geometry_path) = child.geometry_path.as_deref() {
            if let Some(mesh_asset_path) = decomposed_mesh_asset_path(p4k, geometry_path) {
                if existing_asset_paths
                    .is_some_and(|paths| paths.contains(&mesh_asset_path.to_ascii_lowercase()))
                {
                    let material_path = child.material_path.as_deref().unwrap_or("");
                    let (_, materials) = load_nmc_and_material(p4k, geometry_path, material_path);
                    return Some(LoadedChildPayload {
                        mesh: empty_child_mesh(),
                        materials,
                        textures: None,
                        nmc: None,
                        palette: None,
                        bones: Vec::new(),
                        geometry_path: geometry_path.to_string(),
                        material_path: material_path.to_string(),
                    });
                }
            }
        }
    }

    let (mesh, mtl, nmc, palette, bones, geometry_path, material_path) =
        load_child_mesh(child, db, p4k, mesh_opts)?;
    let textures = if final_material_mode.include_textures() {
        mtl.as_ref().map(|materials| {
            let mut png_cache = PngCache::new();
            load_material_textures(
                p4k,
                materials,
                palette.as_ref(),
                mesh_opts.texture_mip,
                &mut png_cache,
                final_material_mode.include_normals(),
                final_material_mode.experimental(),
            )
        })
    } else {
        None
    };

    Some(LoadedChildPayload {
        mesh,
        materials: mtl,
        textures,
        nmc,
        palette,
        bones,
        geometry_path,
        material_path,
    })
}

fn load_child_payloads(
    specs: Vec<ChildPayloadSpec>,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Vec<crate::types::EntityPayload> {
    use rayon::prelude::*;

    let mut unique_children = Vec::new();
    let mut unique_child_indices = std::collections::HashMap::new();
    let mut spec_asset_indices = Vec::with_capacity(specs.len());

    for spec in &specs {
        let child = &spec.child;
        if !child.has_geometry {
            spec_asset_indices.push(None);
            continue;
        }

        let cache_key = build_child_payload_cache_key(child);
        let unique_index = if let Some(&index) = unique_child_indices.get(&cache_key) {
            index
        } else {
            let index = unique_children.len();
            unique_children.push(child.clone_payload_source());
            unique_child_indices.insert(cache_key, index);
            index
        };
        spec_asset_indices.push(Some(unique_index));
    }

    let loaded_assets: Vec<Option<LoadedChildPayload>> = unique_children
        .into_par_iter()
        .map(|child| {
            load_child_payload_asset(
                &child,
                db,
                p4k,
                mesh_opts,
                final_material_mode,
                existing_asset_paths,
            )
        })
        .collect();

    specs
        .into_iter()
        .enumerate()
        .filter_map(|(spec_index, spec)| {
            let child = &spec.child;
            if child.has_geometry {
                let asset_index = spec_asset_indices[spec_index]?;
                let loaded = loaded_assets.get(asset_index)?.as_ref()?;
                Some(crate::types::EntityPayload {
                    mesh: loaded.mesh.clone(),
                    materials: loaded.materials.clone(),
                    textures: loaded.textures.clone(),
                    nmc: loaded.nmc.clone(),
                    palette: loaded.palette.clone(),
                    geometry_path: loaded.geometry_path.clone(),
                    material_path: loaded.material_path.clone(),
                    bones: loaded.bones.clone(),
                    entity_name: child.entity_name.clone(),
                    parent_node_name: spec.parent_node_name.clone(),
                    parent_entity_name: spec.parent_entity_name.clone(),
                    no_rotation: spec.no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                })
            } else if child.nmc.is_some() {
                Some(crate::types::EntityPayload {
                    mesh: empty_child_mesh(),
                    materials: None,
                    textures: None,
                    nmc: child.nmc.clone(),
                    palette: None,
                    geometry_path: child.geometry_path.clone().unwrap_or_default(),
                    material_path: child.material_path.clone().unwrap_or_default(),
                    bones: Vec::new(),
                    entity_name: child.entity_name.clone(),
                    parent_node_name: spec.parent_node_name.clone(),
                    parent_entity_name: spec.parent_entity_name.clone(),
                    no_rotation: spec.no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                })
            } else {
                None
            }
        })
        .collect()
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

    // Load NMC + skeleton from .cga/.chr
    let (nmc, _mtl) = load_nmc_and_material(p4k, &geometry_path, &material_path);

    let resolved = resolve_geometry_files(p4k, &geometry_path)?;
    let bones = load_skeleton(p4k, resolved.skeleton_path.as_deref());

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
        let wheel_lookup: std::collections::HashMap<String, &VehicleXmlPart> = veh_parts
            .iter()
            .filter(|p| p.children.is_empty()) // wheels have no children
            .map(|p| (p.name.to_lowercase(), p))
            .collect();

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

                    part_children.push(crate::types::ResolvedNode {
                        entity_name: wheel.name.clone(),
                        attachment_name: wheel.name.clone(),
                        no_rotation: false,
                        offset_position: [0.0; 3],
                        offset_rotation: [0.0; 3],
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
                        children: Vec::new(),
                    });
                }
            }

            children.push(crate::types::ResolvedNode {
                entity_name: part.name.clone(),
                attachment_name: part.name.clone(),
                no_rotation: false,
                offset_position: [0.0; 3],
                offset_rotation: [0.0; 3],
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
            // CDF files don't have companion files — check for the CDF itself.
            let p4k_geom_path = datacore_path_to_p4k(geom_path);
            let has_geometry = if geom_path.to_lowercase().ends_with(".cdf") {
                p4k.entry_case_insensitive(&p4k_geom_path).is_some()
            } else {
                let companion = resolve_companion_path(p4k, &p4k_geom_path, opts.lod_level);
                p4k.entry_case_insensitive(&companion).is_some()
            };

            if !has_geometry {
                log::warn!("  {} -> mesh not found: {}", node.entity_name, p4k_geom_path);
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
            query_tint_from_record(
                db,
                record,
                Some(short_name),
            )
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

    world.into_iter().flatten().collect()
}

/// Bake NMC node transforms into mesh vertex positions.
///
/// When `use_root_inv` is true, transforms are made root-relative by factoring
/// out the root node's world transform (used for instanced geometry where scaling
/// bbox = model bbox). When false, absolute world transforms are used (for
/// interior CGFs where scaling bbox ≠ model bbox).
fn bake_nmc_into_mesh(
    mut mesh: crate::types::Mesh,
    nmc: Option<&crate::nmc::NodeMeshCombo>,
    use_root_inv: bool,
) -> crate::types::Mesh {
    let nmc = match nmc {
        Some(n) if !n.nodes.is_empty() => n,
        _ => return mesh,
    };

    let world_transforms = compute_nmc_world_transforms(nmc);

    let root_inv = if use_root_inv {
        let root_idx = nmc.nodes.iter().position(|n| n.parent_index.is_none());
        root_idx
            .map(|i| world_transforms[i].inverse())
            .unwrap_or(glam::Mat4::IDENTITY)
    } else {
        glam::Mat4::IDENTITY
    };

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
/// When `use_model_bbox` is true, dequantizes positions using the model bounding
/// box instead of the scaling bbox (needed for interior CGF placement).
fn export_cgf_from_path(
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

fn load_interior_mesh_asset(
    p4k: &MappedP4k,
    entry: &InteriorCgfEntry,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
) -> Option<InteriorMeshAsset> {
    match export_cgf_from_path(
        p4k,
        &entry.cgf_path,
        entry.material_path.as_deref(),
        opts,
        png_cache,
        false,
    ) {
        Ok((mesh, mtl, _tex, nmc, _palette, _, _, _bones)) => {
            let needs_bake = mesh
                .scaling_min
                .iter()
                .zip(&mesh.model_min)
                .chain(mesh.scaling_max.iter().zip(&mesh.model_max))
                .any(|(s, m)| (s - m).abs() > 0.01);
            let mesh = if needs_bake {
                bake_nmc_into_mesh(mesh, nmc.as_ref(), false)
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
}

fn preload_interior_meshes(
    interiors: &LoadedInteriors,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> Vec<Option<InteriorMeshAsset>> {
    use rayon::prelude::*;

    interiors
        .unique_cgfs
        .par_iter()
        .map(|entry| {
            let mut png_cache = PngCache::new();
            load_interior_mesh_asset(p4k, entry, opts, &mut png_cache)
        })
        .collect()
}

fn tint_palette_hash(palette: Option<&mtl::TintPalette>) -> u64 {
    use std::hash::{Hash, Hasher};

    let Some(palette) = palette else {
        return 0;
    };

    let mut hasher = std::hash::DefaultHasher::new();
    for color in [palette.primary, palette.secondary, palette.tertiary, palette.glass] {
        color[0].to_bits().hash(&mut hasher);
        color[1].to_bits().hash(&mut hasher);
        color[2].to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn collect_interior_palettes(
    interiors: &LoadedInteriors,
    fallback_palette: Option<&mtl::TintPalette>,
) -> Vec<(u64, Option<mtl::TintPalette>)> {
    let mut seen = std::collections::HashSet::new();
    let mut palettes = Vec::new();

    for container in &interiors.containers {
        let palette = container.palette.as_ref().or(fallback_palette).cloned();
        let palette_hash = tint_palette_hash(palette.as_ref());
        if seen.insert(palette_hash) {
            palettes.push((palette_hash, palette));
        }
    }

    palettes
}

fn preload_interior_textures(
    interiors: &LoadedInteriors,
    preloaded_meshes: &[Option<InteriorMeshAsset>],
    fallback_palette: Option<&mtl::TintPalette>,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> std::collections::HashMap<PreloadedTextureKey, MaterialTextures> {
    use rayon::prelude::*;

    if !opts.material_mode.include_textures() {
        return std::collections::HashMap::new();
    }

    let palettes = collect_interior_palettes(interiors, fallback_palette);
    if palettes.is_empty() {
        return std::collections::HashMap::new();
    }

    let mut unique_materials = std::collections::HashMap::<String, mtl::MtlFile>::new();
    for asset in preloaded_meshes {
        let Some((_, Some(materials), _)) = asset else {
            continue;
        };
        let Some(source_path) = materials.source_path.as_ref() else {
            continue;
        };
        unique_materials
            .entry(source_path.to_ascii_lowercase())
            .or_insert_with(|| materials.clone());
    }

    if unique_materials.is_empty() {
        return std::collections::HashMap::new();
    }

    let jobs: Vec<(String, mtl::MtlFile, u64, Option<mtl::TintPalette>)> = unique_materials
        .into_iter()
        .flat_map(|(material_source, materials)| {
            palettes.iter().map(move |(palette_hash, palette)| {
                (
                    material_source.clone(),
                    materials.clone(),
                    *palette_hash,
                    palette.clone(),
                )
            })
        })
        .collect();

    jobs
        .into_par_iter()
        .map(|(material_source, materials, palette_hash, palette)| {
            let mut png_cache = PngCache::new();
            let textures = load_material_textures(
                p4k,
                &materials,
                palette.as_ref(),
                opts.texture_mip,
                &mut png_cache,
                opts.material_mode.include_normals(),
                opts.material_mode.experimental(),
            );
            (
                PreloadedTextureKey {
                    material_source,
                    palette_hash,
                },
                textures,
            )
        })
        .collect()
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

fn empty_material_textures(len: usize) -> MaterialTextures {
    MaterialTextures {
        diffuse: Vec::with_capacity(len),
        normal: Vec::with_capacity(len),
        roughness: Vec::with_capacity(len),
        emissive: Vec::with_capacity(len),
        occlusion: Vec::with_capacity(len),
        diffuse_transform: Vec::with_capacity(len),
        normal_transform: Vec::with_capacity(len),
        roughness_transform: Vec::with_capacity(len),
        emissive_transform: Vec::with_capacity(len),
        occlusion_transform: Vec::with_capacity(len),
        bundled_fallbacks: Vec::with_capacity(len),
    }
}

fn push_fallback_tag(tags: &mut Vec<String>, tag: &str) {
    if !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_string());
    }
}

fn make_texture_transform(scale: [f32; 2], tex_coord: u32) -> Option<TextureTransformInfo> {
    if tex_coord == 0
        && (scale[0] - 1.0).abs() <= 1e-4
        && (scale[1] - 1.0).abs() <= 1e-4
    {
        None
    } else {
        Some(TextureTransformInfo { scale, tex_coord })
    }
}

fn material_uses_secondary_uv(material: &mtl::SubMaterial) -> bool {
    material.public_param_f32(&["UseUV2ForStencil"]).is_some_and(|value| value > 0.0)
        || material.string_gen_mask.contains("SECOND_UVS")
        || material.string_gen_mask.contains("EMISSIVE_SECOND_UVS")
}

fn uniform_scale_transform(material: &mtl::SubMaterial, names: &[&str]) -> Option<[f32; 2]> {
    material
        .public_param_f32(names)
        .map(|value| value.abs())
        .filter(|value| *value > f32::EPSILON)
        .map(|value| [value, value])
}

fn simple_texture_transform(
    material: &mtl::SubMaterial,
    role: Option<mtl::TextureSemanticRole>,
) -> Option<TextureTransformInfo> {
    use mtl::TextureSemanticRole;

    let tex_coord = if material_uses_secondary_uv(material) { 1 } else { 0 };
    let scale = match role {
        Some(TextureSemanticRole::ScreenPixelLayout) => {
            let sx = material
                .public_param_f32(&["PixelGridTilingX"])
                .unwrap_or(1.0)
                .abs();
            let sy = material
                .public_param_f32(&["PixelGridTilingY"])
                .unwrap_or(1.0)
                .abs();
            [sx.max(1.0), sy.max(1.0)]
        }
        Some(TextureSemanticRole::Breakup) => uniform_scale_transform(
            material,
            &["StencilBreakupTiling", "BreakupTiling", "Tiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        Some(TextureSemanticRole::BlendMask) => uniform_scale_transform(
            material,
            &["BlendMaskTiling", "Tiling", "LayerTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        Some(TextureSemanticRole::ScreenMask)
        | Some(TextureSemanticRole::WearGloss)
        | Some(TextureSemanticRole::Dirt)
        | Some(TextureSemanticRole::PatternMask) => uniform_scale_transform(
            material,
            &["GlassTiling", "Tiling", "LayerTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        Some(TextureSemanticRole::WearMask)
        | Some(TextureSemanticRole::HalControl) => uniform_scale_transform(
            material,
            &["Tiling", "LayerTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
        _ => uniform_scale_transform(
            material,
            &["StencilTiling", "GlassTiling", "Tiling", "LayerTiling", "MacroTiling"],
        )
        .or_else(|| material.primary_uv_tiling().map(|value| [value, value]))
        .unwrap_or([1.0, 1.0]),
    };

    make_texture_transform(scale, tex_coord)
}

fn decode_png(bytes: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()
        .map(|image| image.to_rgba8())
}

fn encode_png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut png_buf = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut png_buf),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(png_buf)
}

fn make_solid_image(width: u32, height: u32, color: [f32; 3], alpha: u8) -> image::RgbaImage {
    let red = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    image::RgbaImage::from_pixel(width.max(1), height.max(1), image::Rgba([red, green, blue, alpha]))
}

fn sample_pixel(
    image: &image::RgbaImage,
    x: u32,
    y: u32,
    target_width: u32,
    target_height: u32,
) -> image::Rgba<u8> {
    let src_x = if target_width <= 1 || image.width() <= 1 {
        0
    } else {
        x.saturating_mul(image.width().saturating_sub(1))
            / target_width.saturating_sub(1)
    };
    let src_y = if target_height <= 1 || image.height() <= 1 {
        0
    } else {
        y.saturating_mul(image.height().saturating_sub(1))
            / target_height.saturating_sub(1)
    };
    *image.get_pixel(
        src_x.min(image.width().saturating_sub(1)),
        src_y.min(image.height().saturating_sub(1)),
    )
}

fn sample_luma(
    image: &image::RgbaImage,
    x: u32,
    y: u32,
    target_width: u32,
    target_height: u32,
) -> f32 {
    let pixel = sample_pixel(image, x, y, target_width, target_height);
    (f32::from(pixel[0]) + f32::from(pixel[1]) + f32::from(pixel[2])) / (255.0 * 3.0)
}

fn tint_image(image: &image::RgbaImage, color: [f32; 3]) -> image::RgbaImage {
    let mut tinted = image.clone();
    for pixel in tinted.pixels_mut() {
        pixel[0] = (f32::from(pixel[0]) * color[0].clamp(0.0, 1.0)).round() as u8;
        pixel[1] = (f32::from(pixel[1]) * color[1].clamp(0.0, 1.0)).round() as u8;
        pixel[2] = (f32::from(pixel[2]) * color[2].clamp(0.0, 1.0)).round() as u8;
    }
    tinted
}

fn first_role_path(
    material: &mtl::SubMaterial,
    roles: &[mtl::TextureSemanticRole],
) -> Option<String> {
    roles
        .iter()
        .find_map(|role| material.first_texture_path_for_role(*role))
}

fn load_texture_png(
    p4k: &MappedP4k,
    path: &str,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    cached_load(p4k, path, mip, png_cache, load_diffuse_texture)
}

fn load_semantic_texture_png(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    roles: &[mtl::TextureSemanticRole],
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let path = first_role_path(material, roles)?;
    load_texture_png(p4k, &path, mip, png_cache)
}

fn load_layer_diffuse_png(
    p4k: &MappedP4k,
    layer: &mtl::MatLayer,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    if layer.path.is_empty() {
        return None;
    }

    let p4k_path = datacore_path_to_p4k(&layer.path);
    let layer_mtl = try_load_mtl(p4k, &p4k_path)?;
    let layer_material = layer_mtl.materials.first()?;
    let texture_path = layer_material.diffuse_tex.as_ref()?;
    load_texture_png(p4k, texture_path, mip, png_cache)
}

fn build_layer_source_image(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    layer: &mtl::MatLayer,
    palette: Option<&mtl::TintPalette>,
    mip: u32,
    png_cache: &mut PngCache,
    canvas_size: Option<(u32, u32)>,
) -> Option<image::RgbaImage> {
    let color = material.resolved_layer_color(layer, palette);
    if let Some(layer_png) = load_layer_diffuse_png(p4k, layer, mip, png_cache) {
        return decode_png(&layer_png).map(|image| tint_image(&image, color));
    }

    let (width, height) = canvas_size.unwrap_or((64, 64));
    Some(make_solid_image(width, height, color, 255))
}

fn build_layered_base_color_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    palette: Option<&mtl::TintPalette>,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let base_layer = material.layers.first()?;
    let base_image = build_layer_source_image(p4k, material, base_layer, palette, mip, png_cache, None)?;
    let mut output = base_image.clone();

    if let Some(overlay_layer) = material.layers.get(1) {
        let overlay_image = build_layer_source_image(
            p4k,
            material,
            overlay_layer,
            palette,
            mip,
            png_cache,
            Some((output.width(), output.height())),
        )?;
        let blend_mask = load_semantic_texture_png(
            p4k,
            material,
            &[
                mtl::TextureSemanticRole::BlendMask,
                mtl::TextureSemanticRole::WearMask,
                mtl::TextureSemanticRole::Breakup,
                mtl::TextureSemanticRole::Dirt,
            ],
            mip,
            png_cache,
        )
        .and_then(|png| decode_png(&png));
        let blend_factor = material
            .public_param_f32(&["BlendFactor", "WearBlendBase"])
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);

        for y in 0..output.height() {
            for x in 0..output.width() {
                let base_pixel = *output.get_pixel(x, y);
                let overlay_pixel = sample_pixel(&overlay_image, x, y, output.width(), output.height());
                let mask = blend_mask
                    .as_ref()
                    .map(|image| sample_luma(image, x, y, output.width(), output.height()))
                    .unwrap_or(blend_factor)
                    .clamp(0.0, 1.0);
                let inv = 1.0 - mask;
                output.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        (f32::from(base_pixel[0]) * inv + f32::from(overlay_pixel[0]) * mask)
                            .round() as u8,
                        (f32::from(base_pixel[1]) * inv + f32::from(overlay_pixel[1]) * mask)
                            .round() as u8,
                        (f32::from(base_pixel[2]) * inv + f32::from(overlay_pixel[2]) * mask)
                            .round() as u8,
                        255,
                    ]),
                );
            }
        }
    }

    encode_png(&output)
}

fn build_illum_blend_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    base_color_png: Option<&Vec<u8>>,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let base_image = base_color_png.and_then(|png| decode_png(png))?;
    let alternate_path = first_role_path(
        material,
        &[
            mtl::TextureSemanticRole::AlternateBaseColor,
            mtl::TextureSemanticRole::DecalSheet,
        ],
    )?;
    let alternate_png = load_texture_png(p4k, &alternate_path, mip, png_cache)?;
    let alternate_image = decode_png(&alternate_png)?;
    let blend_mask = load_semantic_texture_png(
        p4k,
        material,
        &[mtl::TextureSemanticRole::BlendMask],
        mip,
        png_cache,
    )
    .and_then(|png| decode_png(&png));
    let blend_factor = material
        .public_param_f32(&["BlendFactor"])
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);

    let mut output = base_image.clone();
    for y in 0..output.height() {
        for x in 0..output.width() {
            let base_pixel = sample_pixel(&base_image, x, y, output.width(), output.height());
            let alternate_pixel = sample_pixel(&alternate_image, x, y, output.width(), output.height());
            let mask = blend_mask
                .as_ref()
                .map(|image| sample_luma(image, x, y, output.width(), output.height()))
                .unwrap_or(blend_factor)
                .clamp(0.0, 1.0);
            let inv = 1.0 - mask;
            output.put_pixel(
                x,
                y,
                image::Rgba([
                    (f32::from(base_pixel[0]) * inv + f32::from(alternate_pixel[0]) * mask)
                        .round() as u8,
                    (f32::from(base_pixel[1]) * inv + f32::from(alternate_pixel[1]) * mask)
                        .round() as u8,
                    (f32::from(base_pixel[2]) * inv + f32::from(alternate_pixel[2]) * mask)
                        .round() as u8,
                    255,
                ]),
            );
        }
    }

    encode_png(&output)
}

fn build_stencil_fallback_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    palette: Option<&mtl::TintPalette>,
    base_color_png: Option<&Vec<u8>>,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<Vec<u8>> {
    let decoded = material.decoded_string_gen_mask();
    if !decoded.has_stencil_map && !material.has_virtual_input("$TintPaletteDecal") {
        return None;
    }

    let base_image = base_color_png.and_then(|png| decode_png(png));
    let stencil_image = load_semantic_texture_png(
        p4k,
        material,
        &[
            mtl::TextureSemanticRole::Stencil,
            mtl::TextureSemanticRole::PatternMask,
        ],
        mip,
        png_cache,
    )
    .and_then(|png| decode_png(&png));
    let breakup_image = load_semantic_texture_png(
        p4k,
        material,
        &[
            mtl::TextureSemanticRole::Breakup,
            mtl::TextureSemanticRole::Dirt,
        ],
        mip,
        png_cache,
    )
    .and_then(|png| decode_png(&png));

    let (width, height) = base_image
        .as_ref()
        .map(|image| (image.width(), image.height()))
        .or_else(|| stencil_image.as_ref().map(|image| (image.width(), image.height())))
        .or_else(|| breakup_image.as_ref().map(|image| (image.width(), image.height())))
        .unwrap_or((64, 64));

    let mut output = base_image.unwrap_or_else(|| make_solid_image(width, height, [0.0, 0.0, 0.0], 0));
    let stencil_color = material
        .public_param_rgb(&[
            "StencilDiffuseColor1",
            "StencilDiffuse1",
            "StencilTintColor",
            "TintColor",
        ])
        .or_else(|| material.resolved_palette_color(palette))
        .unwrap_or(material.diffuse);
    let opacity = material
        .public_param_f32(&["StencilOpacity", "DecalDiffuseOpacity", "DecalAlphaMult"])
        .unwrap_or(if material.is_decal() { 0.85 } else { 0.5 })
        .clamp(0.0, 1.0);
    let color = [
        (stencil_color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (stencil_color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (stencil_color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    ];

    for y in 0..height {
        for x in 0..width {
            let mask = stencil_image
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let breakup = breakup_image
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let blend = (opacity * mask * (0.35 + 0.65 * breakup)).clamp(0.0, 1.0);
            let mut base_pixel = *output.get_pixel(x, y);

            if material.is_decal() && base_color_png.is_none() {
                base_pixel = image::Rgba([
                    color[0],
                    color[1],
                    color[2],
                    (blend * 255.0).round() as u8,
                ]);
            } else {
                let inv = 1.0 - blend;
                base_pixel = image::Rgba([
                    (f32::from(base_pixel[0]) * inv + f32::from(color[0]) * blend).round() as u8,
                    (f32::from(base_pixel[1]) * inv + f32::from(color[1]) * blend).round() as u8,
                    (f32::from(base_pixel[2]) * inv + f32::from(color[2]) * blend).round() as u8,
                    if material.is_decal() {
                        (blend * 255.0).round() as u8
                    } else {
                        255
                    },
                ]);
            }

            output.put_pixel(x, y, base_pixel);
        }
    }

    encode_png(&output)
}

fn build_screen_placeholder_textures(
    material: &mtl::SubMaterial,
    support_mask_png: Option<Vec<u8>>,
    pixel_layout_png: Option<Vec<u8>>,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let family = material.shader_family();
    if !matches!(family, mtl::ShaderFamily::DisplayScreen | mtl::ShaderFamily::UiPlane)
        && !material.has_virtual_input("$RenderToTexture")
    {
        return None;
    }

    let support_mask = support_mask_png.as_deref().and_then(decode_png);
    let pixel_layout = pixel_layout_png.as_deref().and_then(decode_png);
    let (width, height) = support_mask
        .as_ref()
        .map(|image| (image.width(), image.height()))
        .or_else(|| pixel_layout.as_ref().map(|image| (image.width(), image.height())))
        .unwrap_or((96, 64));

    let back_color = material
        .public_param_rgb(&["BackColour"])
        .or_else(|| {
            let emissive = material.emissive_factor();
            if emissive == [0.0, 0.0, 0.0] {
                None
            } else {
                Some(emissive)
            }
        })
        .unwrap_or([0.08, 0.22, 0.35]);
    let accent_color = [
        (back_color[0] * 1.6 + 0.15).clamp(0.0, 1.0),
        (back_color[1] * 1.5 + 0.15).clamp(0.0, 1.0),
        (back_color[2] * 1.8 + 0.20).clamp(0.0, 1.0),
    ];
    let grid_x = material
        .public_param_f32(&["PixelGridTilingX"])
        .unwrap_or(8.0)
        .abs()
        .max(1.0);
    let grid_y = material
        .public_param_f32(&["PixelGridTilingY"])
        .unwrap_or(6.0)
        .abs()
        .max(1.0);

    let mut diffuse = image::RgbaImage::new(width, height);
    let mut emissive = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let u = if width <= 1 {
                0.0
            } else {
                x as f32 / (width - 1) as f32
            };
            let v = if height <= 1 {
                0.0
            } else {
                y as f32 / (height - 1) as f32
            };

            let stripe = if (u * grid_x * 4.0).fract() < 0.08 || (v * grid_y * 4.0).fract() < 0.08 {
                1.0
            } else {
                0.0
            };
            let scanline = if ((v * height.max(2) as f32 / 2.0).fract()) < 0.5 {
                0.92
            } else {
                0.74
            };
            let support = support_mask
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let pixel_grid = pixel_layout
                .as_ref()
                .map(|image| sample_luma(image, x, y, width, height))
                .unwrap_or(stripe)
                .clamp(0.0, 1.0);

            let base_mix = (0.12 + 0.24 * support + 0.10 * pixel_grid).clamp(0.0, 1.0);
            let emissive_mix = (0.35 + 0.65 * pixel_grid.max(stripe)) * support * scanline;
            diffuse.put_pixel(
                x,
                y,
                image::Rgba([
                    (back_color[0] * base_mix * 255.0 + accent_color[0] * stripe * 28.0)
                        .clamp(0.0, 255.0)
                        .round() as u8,
                    (back_color[1] * base_mix * 255.0 + accent_color[1] * stripe * 28.0)
                        .clamp(0.0, 255.0)
                        .round() as u8,
                    (back_color[2] * base_mix * 255.0 + accent_color[2] * stripe * 28.0)
                        .clamp(0.0, 255.0)
                        .round() as u8,
                    255,
                ]),
            );
            emissive.put_pixel(
                x,
                y,
                image::Rgba([
                    (accent_color[0] * emissive_mix * 255.0).clamp(0.0, 255.0).round() as u8,
                    (accent_color[1] * emissive_mix * 255.0).clamp(0.0, 255.0).round() as u8,
                    (accent_color[2] * emissive_mix * 255.0).clamp(0.0, 255.0).round() as u8,
                    255,
                ]),
            );
        }
    }

    Some((encode_png(&diffuse)?, encode_png(&emissive)?))
}

fn build_emissive_texture(
    material: &mtl::SubMaterial,
    base_color_png: Option<&Vec<u8>>,
    screen_emissive_png: Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if let Some(emissive_png) = screen_emissive_png {
        return Some(emissive_png);
    }

    let emissive = material.emissive_factor();
    if emissive == [0.0, 0.0, 0.0] {
        return None;
    }

    let mut image = base_color_png
        .and_then(|png| decode_png(png))
        .unwrap_or_else(|| make_solid_image(64, 64, material.diffuse, 255));
    let scale = [
        emissive[0].clamp(0.0, 1.0),
        emissive[1].clamp(0.0, 1.0),
        emissive[2].clamp(0.0, 1.0),
    ];
    for pixel in image.pixels_mut() {
        pixel[0] = (f32::from(pixel[0]) * scale[0]).round() as u8;
        pixel[1] = (f32::from(pixel[1]) * scale[1]).round() as u8;
        pixel[2] = (f32::from(pixel[2]) * scale[2]).round() as u8;
    }
    encode_png(&image)
}

fn convert_png_to_occlusion(png_bytes: &[u8], invert: bool) -> Option<Vec<u8>> {
    let source = decode_png(png_bytes)?;
    let mut image = image::RgbaImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let luminance = ((u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2])) / 3) as u8;
        let occlusion = if invert { 255u8.saturating_sub(luminance) } else { luminance };
        image.put_pixel(x, y, image::Rgba([occlusion, occlusion, occlusion, 255]));
    }
    encode_png(&image)
}

fn build_occlusion_texture(
    p4k: &MappedP4k,
    material: &mtl::SubMaterial,
    mip: u32,
    png_cache: &mut PngCache,
) -> Option<(Vec<u8>, &'static str)> {
    let height_path = first_role_path(material, &[mtl::TextureSemanticRole::Height]);
    if let Some(path) = height_path {
        let png = load_texture_png(p4k, &path, mip, png_cache)?;
        return convert_png_to_occlusion(&png, true).map(|bytes| (bytes, "height"));
    }

    let mask_path = first_role_path(
        material,
        &[
            mtl::TextureSemanticRole::Dirt,
            mtl::TextureSemanticRole::WearMask,
            mtl::TextureSemanticRole::BlendMask,
            mtl::TextureSemanticRole::Breakup,
            mtl::TextureSemanticRole::PatternMask,
            mtl::TextureSemanticRole::ScreenMask,
            mtl::TextureSemanticRole::WearGloss,
        ],
    );
    let path = mask_path?;
    let png = load_texture_png(p4k, &path, mip, png_cache)?;
    convert_png_to_occlusion(&png, false).map(|bytes| (bytes, "mask"))
}

fn load_material_textures(
    p4k: &MappedP4k,
    mtl: &mtl::MtlFile,
    palette: Option<&mtl::TintPalette>,
    mip: u32,
    png_cache: &mut PngCache,
    include_normals: bool,
    experimental_textures: bool,
) -> MaterialTextures {
    let mut textures = empty_material_textures(mtl.materials.len());

    for material in &mtl.materials {
        let mut fallback_tags = Vec::new();

        let screen_mask = load_semantic_texture_png(
            p4k,
            material,
            &[
                mtl::TextureSemanticRole::ScreenMask,
                mtl::TextureSemanticRole::PatternMask,
            ],
            mip,
            png_cache,
        );
        let pixel_layout = load_semantic_texture_png(
            p4k,
            material,
            &[mtl::TextureSemanticRole::ScreenPixelLayout],
            mip,
            png_cache,
        );
        let screen_placeholder = build_screen_placeholder_textures(material, screen_mask, pixel_layout);

        let direct_diffuse = material
            .diffuse_tex
            .as_ref()
            .and_then(|path| load_texture_png(p4k, path, mip, png_cache));
        let prefer_layered_base = !material.layers.is_empty()
            && matches!(
                material.shader_family(),
                mtl::ShaderFamily::HardSurface
                    | mtl::ShaderFamily::LayerBlendV2
                    | mtl::ShaderFamily::Layer
                    | mtl::ShaderFamily::Illum
            );
        let layered_base = if prefer_layered_base {
            build_layered_base_color_texture(p4k, material, palette, mip, png_cache)
        } else {
            None
        };
        let used_layered_base = layered_base.is_some();
        let mut diffuse = if used_layered_base {
            layered_base.clone().or(direct_diffuse.clone())
        } else {
            direct_diffuse.clone().or(layered_base.clone())
        };
        if used_layered_base {
            push_fallback_tag(&mut fallback_tags, "layered_base_color");
        }

        if matches!(material.shader_family(), mtl::ShaderFamily::Illum) {
            if let Some(blended) = build_illum_blend_texture(p4k, material, diffuse.as_ref(), mip, png_cache) {
                diffuse = Some(blended);
                push_fallback_tag(&mut fallback_tags, "illum_blend_fallback");
            }
        }

        if let Some(stencil) = build_stencil_fallback_texture(p4k, material, palette, diffuse.as_ref(), mip, png_cache) {
            diffuse = Some(stencil);
            push_fallback_tag(&mut fallback_tags, "stencil_fallback");
        }

        if diffuse.is_none() {
            if let Some((placeholder_diffuse, _)) = screen_placeholder.as_ref() {
                diffuse = Some(placeholder_diffuse.clone());
                push_fallback_tag(&mut fallback_tags, "rtt_placeholder");
            }
        }

        let normal_path = if let Some(path) = &material.normal_tex {
            Some(path.clone())
        } else {
            material.layers.first().and_then(|layer| {
                let p4k_path = datacore_path_to_p4k(&layer.path);
                try_load_mtl(p4k, &p4k_path).and_then(|layer_mtl| {
                    layer_mtl
                        .materials
                        .first()
                        .and_then(|layer_material| layer_material.normal_tex.clone())
                })
            })
        };

        let normal = if !include_normals {
            None
        } else if let Some(path) = normal_path.as_ref() {
            if !experimental_textures {
                if let Some(diffuse_path) = material.diffuse_tex.as_ref() {
                    if !textures_share_uv_space(diffuse_path, path) {
                        log::debug!("  skipping mismatched normal: diffuse={diffuse_path}, normal={path}");
                        None
                    } else {
                        cached_load(p4k, path, mip, png_cache, load_normal_texture)
                    }
                } else {
                    cached_load(p4k, path, mip, png_cache, load_normal_texture)
                }
            } else {
                cached_load(p4k, path, mip, png_cache, load_normal_texture)
            }
        } else {
            None
        };

        let roughness = if !include_normals {
            None
        } else if let Some(path) = normal_path.as_ref() {
            if !path.contains("_ddna") {
                None
            } else {
                if !experimental_textures {
                    if let Some(diffuse_path) = material.diffuse_tex.as_ref() {
                        if !textures_share_uv_space(diffuse_path, path) {
                            None
                        } else {
                            let cache_key = format!("{path}@roughness_mip{mip}");
                            if let Some(cached) = png_cache.get(&cache_key) {
                                cached.clone()
                            } else {
                                let result = load_roughness_texture(p4k, path, mip);
                                png_cache.insert(cache_key, result.clone());
                                result
                            }
                        }
                    } else {
                        let cache_key = format!("{path}@roughness_mip{mip}");
                        if let Some(cached) = png_cache.get(&cache_key) {
                            cached.clone()
                        } else {
                            let result = load_roughness_texture(p4k, path, mip);
                            png_cache.insert(cache_key, result.clone());
                            result
                        }
                    }
                } else {
                    let cache_key = format!("{path}@roughness_mip{mip}");
                    if let Some(cached) = png_cache.get(&cache_key) {
                        cached.clone()
                    } else {
                        let result = load_roughness_texture(p4k, path, mip);
                        png_cache.insert(cache_key, result.clone());
                        result
                    }
                }
            }
        } else {
            None
        };

        let emissive = build_emissive_texture(
            material,
            diffuse.as_ref(),
            screen_placeholder.as_ref().map(|(_, emissive)| emissive.clone()),
        );
        if emissive.is_some() {
            push_fallback_tag(&mut fallback_tags, if material.has_virtual_input("$RenderToTexture") {
                "screen_emissive_placeholder"
            } else {
                "emissive_texture"
            });
        }

        let occlusion = build_occlusion_texture(p4k, material, mip, png_cache).map(|(bytes, source)| {
            push_fallback_tag(
                &mut fallback_tags,
                if source == "height" {
                    "occlusion_from_height"
                } else {
                    "occlusion_from_mask"
                },
            );
            bytes
        });

        textures.diffuse.push(diffuse.clone());
        textures.normal.push(normal.clone());
        textures.roughness.push(roughness.clone());
        textures.emissive.push(emissive.clone());
        textures.occlusion.push(occlusion.clone());
        textures.diffuse_transform.push(
            diffuse
                .as_ref()
                .and_then(|_| simple_texture_transform(material, Some(mtl::TextureSemanticRole::BaseColor))),
        );
        textures.normal_transform.push(
            normal
                .as_ref()
                .and_then(|_| simple_texture_transform(material, Some(mtl::TextureSemanticRole::NormalGloss))),
        );
        textures.roughness_transform.push(
            roughness
                .as_ref()
                .and_then(|_| simple_texture_transform(material, Some(mtl::TextureSemanticRole::Height))),
        );
        textures.emissive_transform.push(
            emissive
                .as_ref()
                .and_then(|_| {
                    if matches!(material.shader_family(), mtl::ShaderFamily::DisplayScreen | mtl::ShaderFamily::UiPlane)
                        || material.has_virtual_input("$RenderToTexture")
                    {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::ScreenPixelLayout))
                            .or_else(|| simple_texture_transform(material, Some(mtl::TextureSemanticRole::ScreenMask)))
                    } else {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::BaseColor))
                    }
                }),
        );
        textures.occlusion_transform.push(
            occlusion
                .as_ref()
                .and_then(|_| {
                    if material.decoded_string_gen_mask().has_parallax_occlusion_mapping {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::Height))
                    } else {
                        simple_texture_transform(material, Some(mtl::TextureSemanticRole::BlendMask))
                            .or_else(|| simple_texture_transform(material, Some(mtl::TextureSemanticRole::Dirt)))
                    }
                }),
        );
        textures.bundled_fallbacks.push(fallback_tags);
    }

    textures
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
pub(crate) fn cached_load(
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

pub(crate) fn load_diffuse_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
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
pub(crate) fn load_normal_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
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
pub(crate) fn load_roughness_texture(p4k: &MappedP4k, tif_path: &str, mip_level: u32) -> Option<Vec<u8>> {
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

/// Resolve paint-based palette and material overrides from an equipped paint item.
///
/// Finds the paint item in the loadout (hardpoint_paint), extracts the `@Tag` from its
/// AttachDef.Tags, matches it against the entity's SubGeometry entries, and returns the
/// overridden palette and material. Falls through to the originals if no paint is found.
fn resolve_paint_override(
    db: &Database,
    p4k: &MappedP4k,
    entity_record: &Record,
    root_node: &starbreaker_datacore::loadout::LoadoutNode,
    default_palette: Option<mtl::TintPalette>,
    default_mtl: Option<mtl::MtlFile>,
) -> (Option<mtl::TintPalette>, Option<mtl::MtlFile>) {
    // Find paint item in loadout children.
    let paint_node = root_node.children.iter()
        .find(|c| c.item_port_name.to_lowercase().contains("paint"));
    let Some(paint_node) = paint_node else {
        return (default_palette, default_mtl);
    };

    // Query the paint item's AttachDef.Tags to find the @SubGeometry selector.
    let tags = db
        .compile_path::<String>(
            paint_node.record.struct_id(),
            "Components[SAttachableComponentParams].AttachDef.Tags",
        )
        .ok()
        .and_then(|c| db.query_single::<String>(&c, &paint_node.record).ok().flatten())
        .unwrap_or_default();

    // Extract @Tag from the tags string (e.g., "Paint_Gladius @GladiusPirate" → "GladiusPirate").
    let subgeo_tag = tags.split_whitespace()
        .find_map(|t| t.strip_prefix('@'));
    let Some(subgeo_tag) = subgeo_tag else {
        log::info!("  paint '{}' has no @Tag in '{tags}', using default palette", paint_node.entity_name);
        return (default_palette, default_mtl);
    };
    log::info!("  paint '{}' selects SubGeometry tag '{subgeo_tag}'", paint_node.entity_name);

    // Query SubGeometry entries via the full SGeometryResourceParams component Value tree
    // (same approach as loadout.rs query_sub_geometry — querying SubGeometry directly
    // can truncate the array).
    use starbreaker_datacore::query::value::Value;
    let compiled = match db.compile_path::<Value>(
        entity_record.struct_id(),
        "Components[SGeometryResourceParams]",
    ) {
        Ok(c) => c,
        Err(_) => return (default_palette, default_mtl),
    };
    let components = db.query::<Value>(&compiled, entity_record).unwrap_or_default();

    for component in &components {
        let geom_node = match get_value_field(component, "Geometry") {
            Some(g) => g,
            None => continue,
        };
        let sub_arr = match get_value_array(geom_node, "SubGeometry") {
            Some(a) => a,
            None => continue,
        };

        for (idx, sub) in sub_arr.iter().enumerate() {
            let tag = get_value_string(sub, "Tags").unwrap_or("");
            if !tag.eq_ignore_ascii_case(subgeo_tag) {
                continue;
            }
            log::info!("  matched SubGeometry[{idx}] tag='{tag}'");

            // Extract palette from this SubGeometry's Geometry.Palette.RootRecord.root
            let palette = (|| -> Option<mtl::TintPalette> {
                let geom_data = get_value_field(sub, "Geometry")?;
                let palette_ref = get_value_field(geom_data, "Palette")?;
                let root_record = get_value_field(palette_ref, "RootRecord")?;
                let root = get_value_field(root_record, "root")?;

                let read_entry = |entry_name: &str| -> [f32; 3] {
                    let entry = get_value_field(root, entry_name);
                    let tint = entry.and_then(|e| get_value_field(e, "tintColor"));
                    let r = tint.and_then(|t| get_value_u8(t, "r")).unwrap_or(128);
                    let g = tint.and_then(|t| get_value_u8(t, "g")).unwrap_or(128);
                    let b = tint.and_then(|t| get_value_u8(t, "b")).unwrap_or(128);
                    [
                        srgb_to_linear(r as f32 / 255.0),
                        srgb_to_linear(g as f32 / 255.0),
                        srgb_to_linear(b as f32 / 255.0),
                    ]
                };
                Some(mtl::TintPalette {
                    source_name: None,
                    primary: read_entry("entryA"),
                    secondary: read_entry("entryB"),
                    tertiary: read_entry("entryC"),
                    glass: read_entry("glassColor"),
                })
            })();

            if let Some(ref p) = palette {
                log::info!(
                    "  paint palette: primary=[{:.2},{:.2},{:.2}] secondary=[{:.2},{:.2},{:.2}]",
                    p.primary[0], p.primary[1], p.primary[2],
                    p.secondary[0], p.secondary[1], p.secondary[2],
                );
            }

            // Extract material override.
            let mtl_path = get_value_field(sub, "Geometry")
                .and_then(|g| get_value_field(g, "Material"))
                .and_then(|m| get_value_string(m, "path"))
                .filter(|p| !p.is_empty());

            let mtl = if let Some(mtl_path) = mtl_path {
                log::info!("  paint material override: {mtl_path}");
                let p4k_path = datacore_path_to_p4k(mtl_path);
                try_load_mtl(p4k, &p4k_path).or(default_mtl)
            } else {
                default_mtl
            };

            return (palette.or(default_palette), mtl);
        }
    }

    log::warn!("  paint tag '{subgeo_tag}' not found in SubGeometry entries");
    (default_palette, default_mtl)
}

/// Helper: get an object field from a DataCore Value.
fn get_value_field<'v, 'a>(val: &'v starbreaker_datacore::query::value::Value<'a>, name: &str) -> Option<&'v starbreaker_datacore::query::value::Value<'a>> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
    } else {
        None
    }
}

/// Helper: get a string field from a DataCore Value.
fn get_value_string<'a>(val: &starbreaker_datacore::query::value::Value<'a>, name: &str) -> Option<&'a str> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let starbreaker_datacore::query::value::Value::String(s) = v {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Helper: get an array field from a DataCore Value.
fn get_value_array<'v, 'a>(val: &'v starbreaker_datacore::query::value::Value<'a>, name: &str) -> Option<&'v Vec<starbreaker_datacore::query::value::Value<'a>>> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let starbreaker_datacore::query::value::Value::Array(arr) = v {
                    return Some(arr);
                }
            }
        }
    }
    None
}

/// Helper: get a u8 field from a DataCore Value.
fn get_value_u8(val: &starbreaker_datacore::query::value::Value, name: &str) -> Option<u8> {
    if let starbreaker_datacore::query::value::Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let starbreaker_datacore::query::value::Value::UInt8(n) = v {
                    return Some(*n);
                }
            }
        }
    }
    None
}

/// Query the default tint palette colors from a DataCore entity.
///
/// Strategy:
/// 1. Try querying through the entity's Reference path (follows the Reference to the
///    correct TintPaletteTree record, works when RootRecord is populated).
/// 2. Fallback: search for a TintPaletteTree record matching the entity name.
fn query_tint_palette(db: &Database, record: &Record) -> Option<mtl::TintPalette> {
    let entity_name = db.resolve_string2(record.name_offset);
    let short_name = entity_name.rsplit('.').next().unwrap_or(entity_name).to_lowercase();

    // Strategy 1: Query through the entity's Reference path.
    // This follows Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord
    // through the Reference to the TintPaletteTree and reads colors directly.
    let base = "Components[SGeometryResourceParams].Geometry.Geometry.Palette.RootRecord.root";
    if let Some(palette) = query_tint_from_path(db, record, base, Some(short_name.clone())) {
        return Some(palette);
    }

    // Strategy 2: Find TintPaletteTree record by entity name convention.
    let tpt_si = db.struct_id("TintPaletteTree")?;
    // Find an exact match first (e.g., "rsi_zeus_cl"), not a substring match
    // that could pick up a paint variant like "aegs_gladius_black_grey_grey_geometric".
    let palette_record = db.records_of_type(tpt_si)
        .find(|r| {
            let name = db.resolve_string2(r.name_offset).to_lowercase();
            let rec_short = name.rsplit('.').next().unwrap_or(&name);
            rec_short == short_name
        })?;

    query_tint_from_record(
        db,
        palette_record,
        Some(short_name),
    )
}

/// Convert an sRGB 0.0-1.0 component to linear.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Read tint palette colors from a path through an entity record.
fn query_tint_from_path(
    db: &Database,
    record: &Record,
    base: &str,
    source_name: Option<String>,
) -> Option<mtl::TintPalette> {
    let query_rgb = |entry: &str| -> [f32; 3] {
        let mut rgb = [0.5f32; 3];
        for (i, ch) in ["r", "g", "b"].iter().enumerate() {
            let path = format!("{base}.{entry}.tintColor.{ch}");
            if let Ok(compiled) = db.compile_path::<u8>(record.struct_id(), &path)
                && let Ok(Some(v)) = db.query_single::<u8>(&compiled, record)
            {
                // DataCore stores palette colors as SRGB8 — convert to linear for glTF PBR.
                rgb[i] = srgb_to_linear(v as f32 / 255.0);
            }
        }
        rgb
    };

    // Quick check: can we even query this path?
    let test_path = format!("{base}.entryA.tintColor.r");
    let compiled = db.compile_path::<u8>(record.struct_id(), &test_path).ok()?;
    let _val = db.query_single::<u8>(&compiled, record).ok().flatten()?;

    Some(mtl::TintPalette {
        source_name,
        primary: query_rgb("entryA"),
        secondary: query_rgb("entryB"),
        tertiary: query_rgb("entryC"),
        glass: query_rgb("glassColor"),
    })
}

/// Read tint palette colors from a TintPaletteTree record directly.
fn query_tint_from_record(
    db: &Database,
    record: &Record,
    source_name: Option<String>,
) -> Option<mtl::TintPalette> {
    query_tint_from_path(db, record, "root", source_name)
}

pub(crate) fn try_load_mtl(p4k: &MappedP4k, p4k_path: &str) -> Option<mtl::MtlFile> {
    let entry = p4k.entry_case_insensitive(p4k_path)?;
    let data = p4k.read(entry).ok()?;
    let mut mtl = mtl::parse_mtl(&data).ok()?;
    mtl.source_path = Some(p4k_path.to_string());
    Some(mtl)
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

pub(crate) fn datacore_path_to_p4k(path: &str) -> String {
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
                        parts.push(VehicleXmlPart {
                            name: part_name.to_string(),
                            geometry_path: filename.to_string(),
                            material_path: String::new(),
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

    let textures = if !opts.material_mode.include_textures() {
        None
    } else {
        mtl_file
            .as_ref()
            .map(|mtl| {
                load_material_textures(
                    p4k,
                    mtl,
                    None,
                    opts.texture_mip,
                    png_cache,
                    opts.material_mode.include_normals(),
                    opts.material_mode.experimental(),
                )
            })
    };

    let mesh = crate::parse_skin_with_options(&mesh_bytes, use_model_bbox)?;

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

    ensure_supported_export_options(opts)?;

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
        material_mode: MaterialMode::Colors,
        ..opts.clone()
    };
    let mut no_tex: Box<
        dyn FnMut(
            Option<&crate::mtl::MtlFile>,
            Option<&crate::mtl::TintPalette>,
        ) -> Option<MaterialTextures>,
    > = Box::new(|_, _| None);
    let mut interior_png_cache = PngCache::new();
    let mut interior_mesh_loader =
        |entry: &InteriorCgfEntry| -> Option<(crate::Mesh, Option<mtl::MtlFile>, Option<crate::nmc::NodeMeshCombo>)> {
            match export_cgf_from_path(
                p4k,
                &entry.cgf_path,
                entry.material_path.as_deref(),
                &no_tex_opts,
                &mut interior_png_cache,
                false,
            ) {
                Ok((mesh, mtl, _tex, nmc, _palette, _, _, _bones)) => {
                    let needs_bake = mesh.scaling_min.iter().zip(&mesh.model_min)
                        .chain(mesh.scaling_max.iter().zip(&mesh.model_max))
                        .any(|(s, m)| (s - m).abs() > 0.01);
                    let mesh = if needs_bake {
                        bake_nmc_into_mesh(mesh, nmc.as_ref(), false)
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
            material_mode: opts.material_mode,
            preserve_textureless_decal_primitives: false,
            metadata: crate::gltf::GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: crate::gltf::ExportOptionsMetadata {
                    kind: format!("{:?}", opts.kind),
                    material_mode: format!("{:?}", opts.material_mode),
                    format: format!("{:?}", opts.format),
                    lod_level: opts.lod_level,
                    texture_mip: opts.texture_mip,
                    include_attachments: opts.include_attachments,
                    include_interior: opts.include_interior,
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
    use std::path::{Path, PathBuf};

    use starbreaker_datacore::database::Database;
    use starbreaker_datacore::types::{CigGuid, StringId, StringId2};
    use starbreaker_p4k::MappedP4k;

    fn sample_export_result(kind: ExportKind, glb: Vec<u8>) -> ExportResult {
        ExportResult {
            kind,
            format: ExportFormat::Glb,
            glb,
            decomposed: None,
            geometry_path: "objects/test.skin".to_string(),
            material_path: "objects/test.mtl".to_string(),
        }
    }

    fn dummy_record() -> starbreaker_datacore::types::Record {
        starbreaker_datacore::types::Record {
            name_offset: StringId2(-1),
            file_name_offset: StringId(0),
            tag_offset: StringId2(-1),
            struct_index: 0,
            id: CigGuid::EMPTY,
            instance_index: 0,
            struct_size: 0,
        }
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates dir")
            .parent()
            .expect("StarBreaker dir")
            .parent()
            .expect("workspace root")
            .to_path_buf()
    }

    fn integration_p4k_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("STARBREAKER_TEST_P4K") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
            eprintln!("STARBREAKER_TEST_P4K not found at {}, skipping", path.display());
            return None;
        }

        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let candidates = [
            home.join("Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k"),
            home.join("Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/PTU/Data.p4k"),
        ];
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }

        eprintln!("Data.p4k not found. Set STARBREAKER_TEST_P4K to run this manual export harness.");
        None
    }

    fn integration_dcb_bytes(p4k: &MappedP4k) -> Option<Vec<u8>> {
        if let Ok(path) = std::env::var("STARBREAKER_TEST_DCB") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(std::fs::read(&path).expect("failed to read STARBREAKER_TEST_DCB"));
            }
            eprintln!("STARBREAKER_TEST_DCB not found at {}, falling back to Data.p4k", path.display());
        }

        p4k.read_file("Data\\Game2.dcb")
            .or_else(|_| p4k.read_file("Data\\Game.dcb"))
            .ok()
    }

    fn resolved_node(
        entity_name: &str,
        attachment_name: &str,
        has_geometry: bool,
        with_nmc: bool,
        children: Vec<crate::types::ResolvedNode>,
    ) -> crate::types::ResolvedNode {
        crate::types::ResolvedNode {
            entity_name: entity_name.to_string(),
            attachment_name: attachment_name.to_string(),
            no_rotation: false,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            nmc: with_nmc.then_some(crate::nmc::NodeMeshCombo {
                nodes: Vec::new(),
                material_indices: Vec::new(),
            }),
            bones: Vec::new(),
            has_geometry,
            record: dummy_record(),
            geometry_path: has_geometry.then(|| format!("Data/Objects/{entity_name}.skin")),
            material_path: has_geometry.then(|| format!("Data/Objects/{entity_name}.mtl")),
            children,
        }
    }

    #[test]
    fn export_options_default_to_bundled_kind() {
        assert_eq!(ExportOptions::default().kind, ExportKind::Bundled);
    }

    #[test]
    fn bundled_bytes_are_only_available_for_bundled_results() {
        let bundled = sample_export_result(ExportKind::Bundled, vec![1, 2, 3]);
        assert_eq!(bundled.bundled_bytes(), Some(&[1, 2, 3][..]));

        let decomposed = sample_export_result(ExportKind::Decomposed, vec![1, 2, 3]);
        assert_eq!(decomposed.bundled_bytes(), None);

        let empty = sample_export_result(ExportKind::Bundled, Vec::new());
        assert_eq!(empty.bundled_bytes(), None);
    }

    #[test]
    fn collect_child_payload_specs_preserves_reparenting_and_helper_nodes() {
        let proxy = resolved_node(
            "proxy",
            "hardpoint_proxy",
            false,
            false,
            vec![resolved_node("weapon", "hardpoint_weapon", true, false, Vec::new())],
        );
        let rack = resolved_node(
            "rack",
            "hardpoint_rack",
            false,
            true,
            vec![resolved_node("missile", "hardpoint_missile", true, false, Vec::new())],
        );

        let mut specs = Vec::new();
        collect_child_payload_specs(&[proxy, rack], "root_ship", None, &mut specs);

        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].child.entity_name, "weapon");
        assert_eq!(specs[0].parent_entity_name, "root_ship");
        assert_eq!(specs[0].parent_node_name, "hardpoint_proxy");

        assert_eq!(specs[1].child.entity_name, "rack");
        assert_eq!(specs[1].parent_entity_name, "root_ship");
        assert_eq!(specs[1].parent_node_name, "hardpoint_rack");

        assert_eq!(specs[2].child.entity_name, "missile");
        assert_eq!(specs[2].parent_entity_name, "rack");
        assert_eq!(specs[2].parent_node_name, "hardpoint_missile");
    }

    #[test]
    fn export_kind_dispatch_accepts_decomposed_backend() {
        let opts = ExportOptions {
            kind: ExportKind::Decomposed,
            ..ExportOptions::default()
        };

        ensure_supported_export_options(&opts)
            .expect("decomposed export kind should be accepted");
    }

    #[test]
    fn export_format_dispatch_rejects_stl_until_backend_exists() {
        let opts = ExportOptions {
            format: ExportFormat::Stl,
            ..ExportOptions::default()
        };

        let err = ensure_supported_export_options(&opts)
            .expect_err("stl export format should be rejected");
        assert!(matches!(err, Error::UnsupportedExportFormat(format) if format == "Stl"));
    }

    #[test]
    #[ignore = "manual export harness for refreshing the workspace MOLE package"]
    fn regenerate_mole_decomposed_export_to_workspace() {
        let Some(p4k_path) = integration_p4k_path() else {
            return;
        };

        let p4k = MappedP4k::open(&p4k_path).expect("failed to open Data.p4k");
        let dcb_bytes = integration_dcb_bytes(&p4k).expect("failed to load Game2.dcb/Game.dcb");
        let db = Database::from_bytes(&dcb_bytes).expect("failed to parse DataCore database");
        let index = starbreaker_datacore::loadout::EntityIndex::new(&db);
        let record = index.find_record("ARGO_MOLE").expect("ARGO_MOLE not found");
        let tree = starbreaker_datacore::loadout::resolve_loadout_indexed(&index, record);

        let opts = ExportOptions {
            kind: ExportKind::Decomposed,
            material_mode: MaterialMode::All,
            include_attachments: true,
            include_interior: true,
            include_lights: true,
            include_nodraw: false,
            lod_level: 0,
            texture_mip: 0,
            ..ExportOptions::default()
        };

        let result = assemble_glb_with_loadout(&db, &p4k, record, &tree, &opts)
            .expect("ARGO_MOLE decomposed export failed");
        let decomposed = result
            .decomposed
            .as_ref()
            .expect("expected decomposed files in export result");

        let output_root = std::env::var("STARBREAKER_TEST_EXPORT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| workspace_root().join("ships"));
        let package_root = output_root.join("Packages/Argo MOLE");
        if package_root.exists() {
            std::fs::remove_dir_all(&package_root).expect("failed to clear existing Argo MOLE package");
        }

        for file in &decomposed.files {
            let output_path = output_root.join(&file.relative_path);
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).expect("failed to create output directory");
            }
            std::fs::write(&output_path, &file.bytes).expect("failed to write decomposed export file");
        }

        eprintln!(
            "Wrote {} decomposed files for ARGO_MOLE to {} using {}",
            decomposed.files.len(),
            output_root.display(),
            p4k_path.display(),
        );
    }

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
