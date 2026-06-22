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

use starbreaker_datacore::database::Database;
use starbreaker_datacore::error::QueryError;
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

pub(crate) fn rebase_mesh_submeshes_to_bone_space(
    mesh: &mut crate::Mesh,
    bones: &[crate::skeleton::Bone],
) -> bool {
    if mesh.submeshes.is_empty() || bones.is_empty() {
        return false;
    }

    let source_positions = mesh.positions.clone();
    let source_uvs = mesh.uvs.clone();
    let source_secondary_uvs = mesh.secondary_uvs.clone();
    let source_normals = mesh.normals.clone();
    let source_tangents = mesh.tangents.clone();
    let source_colors = mesh.colors.clone();
    let source_indices = mesh.indices.clone();
    let source_submeshes = mesh.submeshes.clone();

    let mut rebuilt_positions: Vec<[f32; 3]> = Vec::new();
    let mut rebuilt_uvs = source_uvs.as_ref().map(|_| Vec::new());
    let mut rebuilt_secondary_uvs = source_secondary_uvs.as_ref().map(|_| Vec::new());
    let mut rebuilt_normals = source_normals.as_ref().map(|_| Vec::new());
    let mut rebuilt_tangents = source_tangents.as_ref().map(|_| Vec::new());
    let mut rebuilt_colors = source_colors.as_ref().map(|_| Vec::new());
    let mut rebuilt_indices = Vec::new();
    let mut rebuilt_submeshes = Vec::with_capacity(source_submeshes.len());

    for submesh in &source_submeshes {
        let bone_index = submesh.node_parent_index as usize;
        if bone_index >= bones.len() {
            return false;
        }

        let bone = &bones[bone_index];
        let bone_inverse = bone_world_transform(bone).inverse();
        let [qw, qx, qy, qz] = bone.world_rotation;
        let inv_rot = glam::Quat::from_xyzw(qx, qy, qz, qw).inverse();

        let start = submesh.first_index as usize;
        let end = (start + submesh.num_indices as usize).min(source_indices.len());
        let mut remap = std::collections::BTreeMap::<u32, u32>::new();
        let first_vertex = rebuilt_positions.len() as u32;
        let first_index = rebuilt_indices.len() as u32;

        for source_index in &source_indices[start..end] {
            let rebuilt_index = if let Some(existing) = remap.get(source_index) {
                *existing
            } else {
                let source_vertex = *source_index as usize;
                if source_vertex >= source_positions.len() {
                    return false;
                }
                let transformed = bone_inverse
                    .transform_point3(glam::Vec3::from(source_positions[source_vertex]));
                let new_index = rebuilt_positions.len() as u32;
                rebuilt_positions.push(transformed.into());

                if let (Some(source), Some(target)) = (&source_uvs, rebuilt_uvs.as_mut()) {
                    target.push(source[source_vertex]);
                }
                if let (Some(source), Some(target)) =
                    (&source_secondary_uvs, rebuilt_secondary_uvs.as_mut())
                {
                    target.push(source[source_vertex]);
                }
                if let (Some(source), Some(target)) = (&source_normals, rebuilt_normals.as_mut()) {
                    target.push((inv_rot * glam::Vec3::from(source[source_vertex])).into());
                }
                if let (Some(source), Some(target)) = (&source_tangents, rebuilt_tangents.as_mut())
                {
                    let tangent = source[source_vertex];
                    let rotated = inv_rot * glam::Vec3::new(tangent[0], tangent[1], tangent[2]);
                    target.push([rotated.x, rotated.y, rotated.z, tangent[3]]);
                }
                if let (Some(source), Some(target)) = (&source_colors, rebuilt_colors.as_mut()) {
                    target.push(source[source_vertex]);
                }

                remap.insert(*source_index, new_index);
                new_index
            };
            rebuilt_indices.push(rebuilt_index);
        }

        let mut rebuilt_submesh = submesh.clone();
        rebuilt_submesh.first_vertex = first_vertex;
        rebuilt_submesh.num_vertices = rebuilt_positions.len() as u32 - first_vertex;
        rebuilt_submesh.first_index = first_index;
        rebuilt_submesh.num_indices = rebuilt_indices.len() as u32 - first_index;
        rebuilt_submeshes.push(rebuilt_submesh);
    }

    let mut new_min = [f32::MAX; 3];
    let mut new_max = [f32::MIN; 3];
    for position in &rebuilt_positions {
        for axis in 0..3 {
            new_min[axis] = new_min[axis].min(position[axis]);
            new_max[axis] = new_max[axis].max(position[axis]);
        }
    }

    mesh.positions = rebuilt_positions;
    mesh.uvs = rebuilt_uvs;
    mesh.secondary_uvs = rebuilt_secondary_uvs;
    mesh.normals = rebuilt_normals;
    mesh.tangents = rebuilt_tangents;
    mesh.colors = rebuilt_colors;
    mesh.indices = rebuilt_indices;
    mesh.submeshes = rebuilt_submeshes;
    if !mesh.positions.is_empty() {
        mesh.model_min = new_min;
        mesh.model_max = new_max;
    }
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

    let mesh_bytes = crate::socpak::read_geometry_file(p4k, &companion_path).ok_or_else(|| {
        Error::FileNotFoundInP4k {
            path: companion_path.clone(),
        }
    })?;

    let (nmc, mut mtl_file) = load_nmc_and_material(p4k, geometry_path, material_path);
    let mesh = crate::parse_skin_with_options(&mesh_bytes, use_model_bbox)?;
    if material_path_is_incompatible_with_mesh(&mesh, mtl_file.as_ref()) {
        let metadata_bytes = crate::socpak::read_geometry_file(p4k, &p4k_geom_path);
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
