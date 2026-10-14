//! Entity export entry points and geometry/skeleton loading helpers.
//!
//! `export_entity_payload` and `export_entity_from_paths` are the primary entity
//! export functions; `export_cgf_from_path` handles individual CGF files.
//! Geometry loading helpers: `resolve_geometry_files`, `load_geometry_parts`,
//! `skeleton_source_paths`, `load_skeleton`, `apply_default_animation_pose_for_skel`.
//! Mesh helpers: `load_nmc_and_material`, `load_single_mesh`, `rebase_mesh_submeshes_to_bone_space`.
//! CDF exports synthesize a skeleton-backed NMC when merged skin attachments
//! reference bone indices that the primary CGF's NMC does not cover.
//! Public types: `GeometryPart`, `ResolvedGeometry`.

use starbreaker_datacore::error::QueryError;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::mtl;
use crate::nmc;
use crate::types::MaterialTextures;

use super::{
    ExportFormat, ExportKind, ExportOptions, MaterialMode, PngCache, bone_world_transform,
    datacore_path_to_p4k, load_material_textures, query_tint_palette, resolve_material,
    synthesize_nmc_from_bones,
};

/// Bundled result of extracting an entity's mesh data from the P4k archive.
pub(crate) type EntityPayload = (
    crate::Mesh,
    Option<mtl::MtlFile>,
    Option<MaterialTextures>,
    Option<nmc::NodeMeshCombo>,
    Option<mtl::TintPalette>,
    String,
    String,
    Vec<crate::skeleton::Bone>,
    Option<String>,
);

fn ensure_supported_export_kind(opts: &ExportOptions) -> Result<(), Error> {
    match opts.kind {
        ExportKind::Bundled | ExportKind::Decomposed => Ok(()),
    }
}

fn ensure_supported_export_format(opts: &ExportOptions) -> Result<(), Error> {
    match opts.format {
        ExportFormat::Glb | ExportFormat::Blend => Ok(()),
        _ => Err(Error::UnsupportedExportFormat(format!("{:?}", opts.format))),
    }
}

pub(crate) fn ensure_supported_export_options(opts: &ExportOptions) -> Result<(), Error> {
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

    let (geometry_path, material_path) = super::geometry_variant::resolve(
        db, record, opts.geometry_tag.as_deref(), geometry_path, material_path,
    )?;

    let (mesh, mtl_file, textures, nmc, skeleton_bones, primary_path, skeleton_source_path) =
        load_geometry_parts(p4k, &geometry_path, &material_path, opts, png_cache, false)?;

    if !opts.material_mode.include_materials() {
        return Ok((
            mesh,
            None,
            None,
            nmc,
            None,
            primary_path,
            material_path,
            skeleton_bones,
            skeleton_source_path,
        ));
    }

    let palette = opts.geometry_tag.as_deref()
        .and_then(|tag| super::palette::query_geometry_variant_palette(db, record, tag))
        .or_else(|| query_tint_palette(db, record));
    Ok((
        mesh,
        mtl_file,
        textures,
        nmc,
        palette,
        primary_path,
        material_path,
        skeleton_bones,
        skeleton_source_path,
    ))
}

/// Export an entity payload using pre-resolved geometry/material paths (no DataCore lookup).
pub(crate) fn export_entity_from_paths(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
) -> Result<EntityPayload, Error> {
    export_entity_from_paths_cached(
        p4k,
        geometry_path,
        material_path,
        opts,
        &mut PngCache::new(),
        false,
    )
}

pub(crate) fn export_entity_from_paths_cached(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<EntityPayload, Error> {
    let (mesh, mtl_file, textures, nmc, skeleton_bones, primary_path, skeleton_source_path) =
        load_geometry_parts(
            p4k,
            geometry_path,
            material_path,
            opts,
            png_cache,
            use_model_bbox,
        )?;

    if !opts.material_mode.include_materials() {
        return Ok((
            mesh,
            None,
            None,
            nmc,
            None,
            primary_path,
            material_path.to_string(),
            skeleton_bones,
            skeleton_source_path,
        ));
    }

    Ok((
        mesh,
        mtl_file,
        textures,
        nmc,
        None,
        primary_path,
        material_path.to_string(),
        skeleton_bones,
        skeleton_source_path,
    ))
}

/// Shared geometry loading: resolve parts, load skeleton, load + merge meshes.
/// Returns (mesh, mtl, textures, nmc, skeleton_bones, primary_path, skeleton_source_path).
fn load_geometry_parts(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
    opts: &ExportOptions,
    png_cache: &mut PngCache,
    use_model_bbox: bool,
) -> Result<
    (
        crate::types::Mesh,
        Option<mtl::MtlFile>,
        Option<MaterialTextures>,
        Option<nmc::NodeMeshCombo>,
        Vec<crate::skeleton::Bone>,
        String,
        Option<String>,
    ),
    Error,
> {
    let resolved = resolve_geometry_files(p4k, geometry_path)?;
    let primary_path = resolved.parts[0].path.clone();
    let skeleton_source_path =
        skeleton_source_paths(resolved.skeleton_path.as_deref(), &primary_path)
            .first()
            .map(|path| (*path).to_string());

    let mut skeleton_bones = load_skeleton(p4k, resolved.skeleton_path.as_deref(), &primary_path);

    if opts.apply_default_animation_pose && !skeleton_bones.is_empty() {
        for path in skeleton_source_paths(resolved.skeleton_path.as_deref(), &primary_path) {
            let updated =
                apply_default_animation_pose_for_skel(p4k, path, &mut skeleton_bones, opts);
            if updated > 0 {
                log::info!("[anim] applied default pose to {updated} bone(s) for skeleton {path}");
                break;
            }
        }
    }

    let effective_material = resolved.parts[0]
        .material_override
        .as_deref()
        .unwrap_or(material_path);
    let (mut mesh, mtl_file, textures, mut nmc) = load_single_mesh(
        p4k,
        &primary_path,
        effective_material,
        opts,
        png_cache,
        use_model_bbox,
    )?;

    if nmc.is_none() {
        nmc = synthesize_nmc_from_bones(&mesh, &skeleton_bones);
        if nmc.is_some() {
            rebase_mesh_submeshes_to_bone_space(&mut mesh, &skeleton_bones);
        }
    }

    // Merge additional parts (CA_BONE/CA_SKIN attachments from CDF).
    let no_tex_opts = ExportOptions {
        material_mode: MaterialMode::Colors,
        ..opts.clone()
    };
    for part in &resolved.parts[1..] {
        match load_single_mesh(
            p4k,
            &part.path,
            material_path,
            &no_tex_opts,
            png_cache,
            use_model_bbox,
        ) {
            Ok((mut extra_mesh, _, _, _)) => {
                if let Some(ref bone_name) = part.bone_name {
                    if let Some(bone) = skeleton_bones
                        .iter()
                        .find(|b| b.name.eq_ignore_ascii_case(bone_name))
                    {
                        transform_mesh_by_bone(&mut extra_mesh, bone);
                    }
                }
                mesh.merge_from(extra_mesh);
            }
            Err(e) => log::warn!("  CDF part '{}' failed: {e}", part.path),
        }
    }

    if !nmc_covers_mesh_submesh_nodes(nmc.as_ref(), &mesh) {
        if let Some(skeleton_nmc) = synthesize_nmc_from_bones(&mesh, &skeleton_bones) {
            if rebase_mesh_submeshes_to_bone_space(&mut mesh, &skeleton_bones) {
                nmc = Some(skeleton_nmc);
            }
        }
    }

    Ok((
        mesh,
        mtl_file,
        textures,
        nmc,
        skeleton_bones,
        primary_path,
        skeleton_source_path,
    ))
}

/// Load skeleton bones from a .chr path. Returns empty vec if path is None or load fails.
pub(crate) fn skeleton_source_paths<'a>(
    skel_path: Option<&'a str>,
    geometry_path: &'a str,
) -> Vec<&'a str> {
    let mut paths = Vec::new();
    if let Some(path) = skel_path.filter(|path| !path.is_empty()) {
        paths.push(path);
    }
    if !geometry_path.is_empty()
        && !paths
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(geometry_path))
    {
        paths.push(geometry_path);
    }
    paths
}

/// Load skeleton bones from an explicit `.chr` path, falling back to the primary geometry.
/// Direct `.skin` assets can carry inline CompiledBones chunks without a companion skeleton.
pub(crate) fn load_skeleton(
    p4k: &MappedP4k,
    skel_path: Option<&str>,
    geometry_path: &str,
) -> Vec<crate::skeleton::Bone> {
    for path in skeleton_source_paths(skel_path, geometry_path) {
        let p4k_path = datacore_path_to_p4k(path);
        if let Some(bones) = p4k
            .entry_case_insensitive(&p4k_path)
            .and_then(|entry| p4k.read(entry).ok())
            .and_then(|data| crate::skeleton::parse_skeleton(&data))
        {
            return bones;
        }
    }
    Vec::new()
}

/// Look up a `.chrparams` for the given skeleton path, find an animation
/// matching one of `opts.default_animation_tags`, and bake its final-frame
/// pose into `bones`. Returns the number of bones updated (0 = nothing
/// applied / chrparams missing / animation missing).
fn apply_default_animation_pose_for_skel(
    p4k: &MappedP4k,
    skel_path: &str,
    bones: &mut [crate::skeleton::Bone],
    opts: &ExportOptions,
) -> usize {
    // Derive the .chrparams path: replace .chr/.skin extension with .chrparams.
    let chrparams_path = match swap_extension(skel_path, "chrparams") {
        Some(p) => p,
        None => return 0,
    };
    let p4k_path = datacore_path_to_p4k(&chrparams_path);
    let bytes = match p4k
        .entry_case_insensitive(&p4k_path)
        .and_then(|entry| p4k.read(entry).ok())
    {
        Some(b) => b,
        None => return 0,
    };
    let cp = match crate::chrparams::ChrParams::from_bytes(&bytes) {
        Ok(cp) => cp,
        Err(e) => {
            log::warn!("[anim] failed to parse {chrparams_path}: {e}");
            return 0;
        }
    };
    // Pick the first matching animation tag.
    let mut tag_match: Option<(&str, String)> = None;
    for tag in &opts.default_animation_tags {
        if let Some(p) = cp.animations.get(tag) {
            tag_match = Some((tag.as_str(), cp.resolved_caf_path(p)));
            break;
        }
    }
    let (_tag, _caf_path) = match tag_match {
        Some(t) => t,
        None => return 0,
    };
    // We need the .dba ($TracksDatabase). The .caf is a hint that the right
    // bone subset will live in some DBA block; we don't open the .caf.
    let dba_path = match cp.tracks_database.as_deref() {
        Some(p) => p,
        None => return 0,
    };
    let dba_p4k = datacore_path_to_p4k(dba_path);
    let dba_bytes = match p4k
        .entry_case_insensitive(&dba_p4k)
        .and_then(|entry| p4k.read(entry).ok())
    {
        Some(b) => b,
        None => {
            log::warn!("[anim] tracks database not found: {dba_path}");
            return 0;
        }
    };
    let db = match crate::animation::parse_dba(&dba_bytes) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("[anim] failed to parse {dba_path}: {e}");
            return 0;
        }
    };
    // Skeleton bone-hash set for signature matching.
    let skel_hashes: std::collections::HashSet<u32> = bones
        .iter()
        .map(|b| crate::animation::bone_name_hash(&b.name))
        .collect();
    let clip = match crate::animation::find_block_for_skeleton(&db, &skel_hashes, true) {
        Some(c) => c,
        None => return 0,
    };
    let pose = crate::animation::clip_final_pose(clip);
    crate::animation::apply_pose_to_skeleton(bones, &pose)
}

/// Replace the file extension of `path` with `new_ext` (no leading dot).
/// Returns `None` if `path` has no extension.
fn swap_extension(path: &str, new_ext: &str) -> Option<String> {
    let dot = path.rfind('.')?;
    let slash = path.rfind(|c: char| c == '/' || c == '\\').unwrap_or(0);
    if dot < slash {
        return None;
    }
    let mut out = String::with_capacity(dot + 1 + new_ext.len());
    out.push_str(&path[..dot + 1]);
    out.push_str(new_ext);
    Some(out)
}
/// Move a CA_BONE attachment into its bone's frame and bind it rigidly to
/// that bone.
fn transform_mesh_by_bone(mesh: &mut crate::Mesh, bone: &crate::skeleton::Bone) {
    let world = bone_world_transform(bone);
    mesh.transform(glam::Affine3A::from_mat4(world));
    mesh.skin = Some(crate::mesh_rig::Skin::rigid(&bone.name, mesh.positions.len()));
}

pub(crate) fn rebase_mesh_submeshes_to_bone_space(
    mesh: &mut crate::Mesh,
    bones: &[crate::skeleton::Bone],
) -> bool {
    if mesh.submeshes.is_empty() || bones.is_empty() {
        return false;
    }
    // Every submesh must name a bone and reference only real vertices.
    let valid = mesh.submeshes.iter().all(|submesh| {
        let start = (submesh.first_index as usize).min(mesh.indices.len());
        let end = (start + submesh.num_indices as usize).min(mesh.indices.len());
        (submesh.node_parent_index as usize) < bones.len()
            && mesh.indices[start..end].iter().all(|&i| (i as usize) < mesh.positions.len())
    });
    if !valid {
        return false;
    }

    // Each submesh, moved into its bone's space, then concatenated.
    let mut rebuilt: Option<crate::Mesh> = None;
    for (index, submesh) in mesh.submeshes.iter().enumerate() {
        let bone = &bones[submesh.node_parent_index as usize];
        let (mut part, _) = mesh.subset(&[index]);
        part.transform(glam::Affine3A::from_mat4(
            bone_world_transform(bone).inverse(),
        ));
        match rebuilt.as_mut() {
            Some(out) => out.merge_from(part),
            None => rebuilt = Some(part),
        }
    }
    let Some(mut rebuilt) = rebuilt else { return false };
    // Bone-space parts share no frame, so the model bbox is the bounds of the
    // rebased positions (as before); the dequantization bbox is unchanged.
    if let (Some(min), Some(max)) = (
        rebuilt.positions.iter().copied().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]),
        rebuilt.positions.iter().copied().reduce(|a, b| [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]),
    ) {
        rebuilt.model_min = min;
        rebuilt.model_max = max;
    }
    rebuilt.scaling_min = mesh.scaling_min;
    rebuilt.scaling_max = mesh.scaling_max;
    *mesh = rebuilt;
    true
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
pub(crate) struct GeometryPart {
    pub(crate) path: String,
    /// Bone name from CDF attachment (for CA_BONE placement). None for CA_SKIN.
    pub(crate) bone_name: Option<String>,
    /// Material override from CDF attachment. Takes priority over the DataCore material.
    pub(crate) material_override: Option<String>,
}

/// Result of resolving a geometry path — all mesh parts plus optional skeleton.
pub(crate) struct ResolvedGeometry {
    pub(crate) parts: Vec<GeometryPart>,
    /// Path to .chr skeleton (from CDF Model element). None for direct .skin/.cgf.
    pub(crate) skeleton_path: Option<String>,
}

/// Resolve a geometry path into the list of actual mesh files to load.
///
/// - `.skin`/`.cgf` etc -> single part
/// - `.cdf` -> parse CharacterDefinition, return all Attachment bindings
pub(crate) fn resolve_geometry_files(
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
            let attrs: std::collections::HashMap<&str, &str> = xml.node_attributes(child).collect();
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
                            material_override: attrs
                                .get("Material")
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
pub(crate) fn resolve_companion_path(
    p4k: &MappedP4k,
    p4k_geom_path: &str,
    lod_level: u32,
) -> String {
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

/// Load NMC and material from the metadata file (.cga/.cgf/.skin).
/// Never fails — returns None for each if the file is missing.
pub(crate) fn load_nmc_and_material(
    p4k: &MappedP4k,
    geometry_path: &str,
    material_path: &str,
) -> (Option<nmc::NodeMeshCombo>, Option<mtl::MtlFile>) {
    let p4k_geom_path = datacore_path_to_p4k(geometry_path);
    let metadata_bytes = crate::socpak::read_geometry_file(p4k, &p4k_geom_path);

    let mtl_file = resolve_material(
        p4k,
        material_path,
        &p4k_geom_path,
        metadata_bytes.as_deref(),
    );

    let nmc =
        metadata_bytes
            .as_deref()
            .and_then(nmc::parse_nmc_full)
            .map(|(nodes, mat_indices)| nmc::NodeMeshCombo {
                nodes,
                material_indices: mat_indices,
            });

    (nmc, mtl_file)
}

pub(crate) fn load_single_mesh(
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

    let mesh_bytes = crate::socpak::read_geometry_file(p4k, &companion_path).ok_or_else(|| {
        Error::FileNotFoundInP4k {
            path: companion_path.clone(),
        }
    })?;

    let (nmc, mut mtl_file) = load_nmc_and_material(p4k, geometry_path, material_path);
    // The metadata file (.skin / .cgf) holds the bone table naming the skin's joints.
    let metadata_bytes = crate::socpak::read_geometry_file(p4k, &p4k_geom_path);
    let mut mesh = crate::parse_skin_pair_with_options(&mesh_bytes, metadata_bytes.as_deref(), use_model_bbox)?;
    if material_path_is_incompatible_with_mesh(&mesh, mtl_file.as_ref()) {
        if let Some(fallback_mtl) = resolve_material(p4k, "", &p4k_geom_path, metadata_bytes.as_deref()) {
            if !material_path_is_incompatible_with_mesh(&mesh, Some(&fallback_mtl)) {
                log::debug!(
                    "material override '{}' does not cover mesh material ids for {}; using geometry metadata material {}",
                    material_path,
                    geometry_path,
                    fallback_mtl.source_path.as_deref().unwrap_or("<unknown>")
                );
                mtl_file = Some(fallback_mtl);
            }
        }
    }
    // Every submesh draws with the material the engine resolves for its slot.
    if let Some(mtl) = mtl_file.as_mut() {
        crate::material_slots::resolve_material_slots(&mut mesh, mtl, || {
            crate::material_slots::load_default_material(p4k)
        })?;
    }

    let textures = if !opts.material_mode.include_textures() {
        None
    } else {
        mtl_file.as_ref().map(|mtl| {
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

    Ok((mesh, mtl_file, textures, nmc))
}

fn material_path_is_incompatible_with_mesh(
    mesh: &crate::Mesh,
    mtl_file: Option<&mtl::MtlFile>,
) -> bool {
    let Some(mtl_file) = mtl_file else {
        return false;
    };
    !mesh_material_ids_fit_material_count(mesh, mtl_file.materials.len() as u32)
}

fn mesh_material_ids_fit_material_count(mesh: &crate::Mesh, material_count: u32) -> bool {
    mesh.submeshes
        .iter()
        .all(|submesh| submesh.material_id < material_count)
}

fn nmc_covers_mesh_submesh_nodes(nmc: Option<&nmc::NodeMeshCombo>, mesh: &crate::Mesh) -> bool {
    let Some(nmc) = nmc.filter(|nmc| !nmc.nodes.is_empty()) else {
        return false;
    };
    mesh.submeshes
        .iter()
        .all(|submesh| (submesh.node_parent_index as usize) < nmc.nodes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmc::{NmcNode, NodeMeshCombo};
    use crate::types::SubMesh;

    fn rotated_bone(name: &str) -> crate::skeleton::Bone {
        // 90 degrees about +Z, at (10, 0, 0); quaternion stored [w, x, y, z]
        let h = std::f32::consts::FRAC_1_SQRT_2;
        crate::skeleton::Bone {
            name: name.to_string(),
            parent_index: None,
            object_node_index: None,
            local_position: [10.0, 0.0, 0.0],
            local_rotation: [h, 0.0, 0.0, h],
            world_position: [10.0, 0.0, 0.0],
            world_rotation: [h, 0.0, 0.0, h],
            bind_world_position: [10.0, 0.0, 0.0],
            bind_world_rotation: [h, 0.0, 0.0, h],
        }
    }

    fn tangent_triangle() -> crate::Mesh {
        let mut mesh = crate::Mesh::empty();
        mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        mesh.indices = vec![0, 1, 2];
        mesh.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
        mesh.tangents = Some(vec![[1.0, 0.0, 0.0, -1.0]; 3]);
        mesh.colors = Some(vec![[1, 2, 3, 4]; 3]);
        mesh.submeshes = vec![SubMesh {
            material_name: None,
            material_id: 0,
            source_material_id: None,
            first_index: 0,
            num_indices: 3,
            first_vertex: 0,
            num_vertices: 3,
            node_parent_index: 0,
        }];
        mesh.model_max = [1.0, 1.0, 0.0];
        mesh
    }

    /// A CA_BONE part follows its bone: every spatial stream (tangents too)
    /// is moved into the bone's frame and the part is rigidly skinned to it.
    #[test]
    fn transform_mesh_by_bone_moves_tangents_and_binds_the_bone() {
        let mut mesh = tangent_triangle();
        transform_mesh_by_bone(&mut mesh, &rotated_bone("hand"));
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(mesh.positions[1], [10.0, 1.0, 0.0]), "{:?}", mesh.positions[1]);
        let t = mesh.tangents.as_ref().unwrap()[0];
        assert!(close([t[0], t[1], t[2]], [0.0, 1.0, 0.0]) && t[3] == -1.0, "{t:?}");
        let skin = mesh.skin.expect("rigid skin to the bone");
        assert_eq!(skin.joint_names, vec!["hand".to_string()]);
        assert!(skin.weights.iter().all(|w| w[0] == 1.0));
    }

    /// Rebasing into bone space rotates tangents with normals and keeps every
    /// other per-vertex stream.
    #[test]
    fn rebase_into_bone_space_moves_tangents_and_keeps_streams() {
        let mut mesh = tangent_triangle();
        assert!(rebase_mesh_submeshes_to_bone_space(&mut mesh, &[rotated_bone("root")]));
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        let t = mesh.tangents.as_ref().unwrap()[0];
        assert!(close([t[0], t[1], t[2]], [0.0, -1.0, 0.0]), "inverse rotation: {t:?}");
        assert_eq!(mesh.colors, Some(vec![[1, 2, 3, 4]; 3]));
        assert!(close(mesh.positions[0], [0.0, 10.0, 0.0]), "{:?}", mesh.positions[0]);
    }

    #[test]
    fn mesh_material_ids_detect_material_override_that_cannot_cover_mesh() {
        let mesh = crate::Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![
                SubMesh {
                    material_name: None,
                    material_id: 0,
                    source_material_id: None,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 0,
                },
                SubMesh {
                    material_name: None,
                    material_id: 2,
                    source_material_id: None,
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 0,
                },
            ],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: Vec::new(),
        };

        assert!(mesh_material_ids_fit_material_count(&mesh, 3));
        assert!(!mesh_material_ids_fit_material_count(&mesh, 2));
    }

    #[test]
    fn nmc_coverage_detects_merged_skin_submeshes_outside_primary_cgf_nodes() {
        let mesh = crate::Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![
                SubMesh {
                    material_name: None,
                    material_id: 0,
                    source_material_id: None,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 0,
                },
                SubMesh {
                    material_name: None,
                    material_id: 1,
                    source_material_id: None,
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 2,
                },
            ],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: Vec::new(),
        };
        let primary_cgf_nmc = NodeMeshCombo {
            nodes: vec![nmc_node("body", None)],
            material_indices: Vec::new(),
        };

        assert!(!nmc_covers_mesh_submesh_nodes(
            Some(&primary_cgf_nmc),
            &mesh
        ));
    }

    fn nmc_node(name: &str, parent_index: Option<u16>) -> NmcNode {
        NmcNode {
            name: name.to_string(),
            parent_index,
            world_to_bone: [[0.0; 4]; 3],
            bone_to_world: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            scale: [1.0; 3],
            geometry_type: 0,
            properties: std::collections::HashMap::new(),
        }
    }
}
