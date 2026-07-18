//! Public pipeline module: type definitions, utility helpers, and module re-exports.
//!
//! Declares sub-modules and re-exports their public surfaces. Defines the primary
//! export types (`ExportOptions`, `ExportResult`, `ExportKind`, `ExportFormat`,
//! `MaterialMode`, `DecomposedExport`) and low-level utilities that are called from
//! many sub-modules: `datacore_path_to_p4k`, `socpaks_to_glb`,
//! `dump_nmc_nodes`, `dump_hierarchy`.

use std::collections::HashSet;
use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_dds::ReadSibling;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::mtl;
use crate::nmc;
use crate::types::MaterialTextures;

mod textures;
use self::textures::*;
pub(crate) use self::textures::{
    PngCache, RoughnessCache, cached_load_keyed, load_diffuse_texture, load_normal_texture,
    load_roughness_texture, load_roughness_texture_result, png_cache_key,
};
mod interiors;
pub(crate) use self::interiors::*;
pub use crate::socpak::SocpakHierarchyNode;
mod nmc_bridge;
pub(crate) use self::nmc_bridge::*;
mod child_payload;
pub(crate) use self::child_payload::*;
mod loadout;
pub use self::loadout::resolve_loadout_meshes;
pub(crate) use self::loadout::*;
mod entity_export;
pub(crate) use self::entity_export::*;
mod weapon_assembly;
pub(crate) use self::weapon_assembly::*;

mod palette;
pub use self::palette::*;
mod interior_animation;
pub(crate) use self::interior_animation::*;
mod vehicle;
pub use self::vehicle::*;
mod glb_assembly;
pub(crate) use self::glb_assembly::path_is_shield_related;
pub use self::glb_assembly::{assemble_glb_with_loadout, assemble_glb_with_loadout_with_progress};
mod blend_assembly;
pub use self::blend_assembly::write_decomposed_export_blend;

pub fn inspect_socpak_hierarchy(
    p4k: &MappedP4k,
    socpak_path: &str,
) -> Result<crate::socpak::SocpakHierarchyNode, Error> {
    crate::socpak::inspect_interior_tree_from_socpak(p4k, socpak_path)
}

type InteriorMeshAsset = (
    crate::Mesh,
    Option<mtl::MtlFile>,
    Option<nmc::NodeMeshCombo>,
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreloadedTextureKey {
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
    /// Blender decomposed export (separate .blend files with scene.blend linking).
    Blend,
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
    /// Include shield helper meshes and shield attachments.
    pub include_shields: bool,
    /// LOD level (0 = highest detail, 1+ = lower).
    pub lod_level: u32,
    /// Texture mip level (0 = full resolution, 2 = 1/4 res, 4 = 1/16 res).
    pub texture_mip: u32,
    /// Worker threads for parallel export phases. 0 = auto/all available cores, 1 = sequential.
    pub threads: usize,
    /// Export animation clips into decomposed scene sidecars.
    pub include_animations: bool,
    /// Apply default-state animation poses (e.g. landing-gear-deployed) to
    /// skeletons that ship a `.chrparams` file. Affects the rest pose written
    /// into the GLB / decomposed skeleton data.
    pub apply_default_animation_pose: bool,
    /// Animation event tags (chrparams `<Animation name="…"/>`) to look up
    /// when `apply_default_animation_pose` is enabled. The first match wins
    /// per skeleton. Default: `landing_gear_extend`.
    pub default_animation_tags: Vec<String>,
    /// Optional subdirectory under `Packages/` for decomposed outputs.
    /// Example: `ship` writes manifests to `Packages/ship/<package>/...` while
    /// still keeping shared assets under `Data/...`.
    pub decomposed_package_subdir: Option<String>,
    /// Decomposed fast path: emit only UI-generated images and required scene manifests.
    pub ui_only_files: bool,
    /// Optional normalized lowercase socpak paths to include during inherited
    /// interior discovery. `None` exports every discovered container.
    pub socpak_path_filter: Option<HashSet<String>>,
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
            include_shields: false,
            lod_level: 1,
            texture_mip: 2,
            threads: 0,
            include_animations: false,
            apply_default_animation_pose: true,
            default_animation_tags: vec!["landing_gear_extend".to_string()],
            decomposed_package_subdir: None,
            ui_only_files: false,
            socpak_path_filter: None,
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

    pub fn is_reusable_asset(self) -> bool {
        matches!(
            self,
            Self::MeshAsset | Self::MaterialSidecar | Self::TextureAsset
        )
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

/// Export an entity with its loadout tree as a single GLB.
///
/// This is the main export entry point. Handles loadout children, interiors,
/// invisible port filtering, textures, and lights. Works for any DataCore
/// entity — ships get full loadout assembly, simpler entities just export
/// their root geometry.
// ── Interior loading ────────────────────────────────────────────────────────

/// Interior layout data: unique CGF paths + placement transforms.
/// Mesh data is NOT loaded here — it's loaded JIT during GLB packing.

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
/// Resolve the MTL file path in P4k format from a MtlName string.
///
/// DataCore: `objects/ships/aegs/file.skin` (forward slashes, no prefix)
/// P4k:      `Data\Objects\Ships\AEGS\file.skin` (backslashes, `Data\` prefix)
///
/// Case mismatch is handled by `entry_case_insensitive` on the P4k side.
/// Transform a mesh's vertices by a skeleton bone's world transform (rotation + translation).
/// Used for CA_BONE CDF attachments that are authored in bone-local space.
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
        if opts
            .socpak_path_filter
            .as_ref()
            .is_some_and(|filter| !filter.contains(&socpak::normalize_socpak_filter_key(socpak_path)))
        {
            continue;
        }
        match socpak::load_interior_tree_from_socpak_filtered_with_progress(
            p4k,
            socpak_path,
            identity,
            identity,
            std::slice::from_ref(&identity),
            opts.socpak_path_filter.as_ref(),
            &mut |_| {},
        ) {
            Ok(mut tree_payloads) => payloads.append(&mut tree_payloads),
            Err(e) => log::warn!("failed to load {socpak_path}: {e}"),
        }
    }

    let interiors =
        build_interiors_from_payloads(db, p4k, &payloads, opts.include_lights, opts.lod_level);

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
    let mut interior_mesh_loader = |entry: &InteriorCgfEntry| -> Option<(
        crate::Mesh,
        Option<mtl::MtlFile>,
        Option<crate::nmc::NodeMeshCombo>,
    )> {
        match export_cgf_from_path(
            p4k,
            &entry.cgf_path,
            entry.material_path.as_deref(),
            &no_tex_opts,
            &mut interior_png_cache,
            false,
        ) {
            Ok((mesh, mtl, _tex, nmc, _palette, _, _, _bones, _skeleton_source_path)) => {
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

/// Export explicit socpak roots as a decomposed native Blender package.
///
/// Socpaks do not have a ship/entity root mesh, so the package uses an empty
/// generated root asset and carries the real visual content through `interiors`.
pub fn socpaks_to_decomposed_blend(
    db: &Database,
    p4k: &MappedP4k,
    socpak_paths: &[String],
    export_name: &str,
    opts: &ExportOptions,
) -> Result<DecomposedExport, Error> {
    let mut ignore_progress = |_progress: SocpakExportProgress| {};
    socpaks_to_decomposed_blend_with_progress(
        db,
        p4k,
        socpak_paths,
        export_name,
        opts,
        &mut ignore_progress,
    )
}

#[derive(Debug, Clone)]
pub struct SocpakExportProgress {
    pub socpak_path: String,
    pub depth: usize,
    pub containers_loaded: usize,
    pub child_count: usize,
    pub mesh_count: usize,
    pub light_count: usize,
    pub stage: String,
}

/// Export explicit socpak roots as a decomposed native Blender package, reporting
/// cheap progress from the recursive socpak tree walk.
pub fn socpaks_to_decomposed_blend_with_progress<F>(
    db: &Database,
    p4k: &MappedP4k,
    socpak_paths: &[String],
    export_name: &str,
    opts: &ExportOptions,
    progress: &mut F,
) -> Result<DecomposedExport, Error>
where
    F: FnMut(SocpakExportProgress),
{
    use crate::decomposed::DecomposedInput;
    use crate::socpak;

    ensure_supported_export_options(opts)?;
    if opts.kind != ExportKind::Decomposed || opts.format != ExportFormat::Blend {
        return Err(Error::UnsupportedExportFormat(
            "socpak decomposed blend export requires kind=Decomposed and format=Blend".to_string(),
        ));
    }

    let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
    let mut payloads = Vec::new();
    for socpak_path in socpak_paths {
        if opts
            .socpak_path_filter
            .as_ref()
            .is_some_and(|filter| !filter.contains(&socpak::normalize_socpak_filter_key(socpak_path)))
        {
            continue;
        }
        match socpak::load_interior_tree_from_socpak_filtered_with_progress(
            p4k,
            socpak_path,
            identity,
            identity,
            std::slice::from_ref(&identity),
            opts.socpak_path_filter.as_ref(),
            &mut |tree_progress| {
                progress(SocpakExportProgress {
                    socpak_path: tree_progress.socpak_path,
                    depth: tree_progress.depth,
                    containers_loaded: tree_progress.containers_loaded,
                    child_count: tree_progress.child_count,
                    mesh_count: tree_progress.mesh_count,
                    light_count: tree_progress.light_count,
                    stage: tree_progress.stage,
                });
            },
        ) {
            Ok(mut tree_payloads) => payloads.append(&mut tree_payloads),
            Err(e) => log::warn!("failed to load {socpak_path}: {e}"),
        }
    }

    let mut interiors =
        build_interiors_from_payloads(db, p4k, &payloads, opts.include_lights, opts.lod_level);
    // Animated interior entities (`.cga` doors/elevators with a sibling
    // `.chrparams`) become decomposed children so their skeletal `.caf` clips are
    // extracted by the existing animation pipeline; this also removes them from
    // the static interior placements to avoid duplicate geometry.
    let anim_children_start = std::time::Instant::now();
    let children = if opts.include_animations {
        extract_animated_interior_children(db, p4k, &mut interiors, opts)
    } else {
        Vec::new()
    };
    log::info!(
        "[timing][socpak] animated_interior_children ({} children): {:.2}s",
        children.len(),
        anim_children_start.elapsed().as_secs_f32()
    );
    let input = DecomposedInput {
        entity_name: export_name.to_string(),
        geometry_path: String::new(),
        material_path: String::new(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: empty_socpak_scene_root_mesh(),
        root_materials: None,
        root_nmc: None,
        root_palette: None,
        available_palettes: Vec::new(),
        root_bones: Vec::new(),
        root_skeleton_source_path: None,
        root_animation_controller: None,
        children,
        interiors,
        paint_variants: Vec::new(),
    };

    write_decomposed_export_blend(db, p4k, input, opts, None, None, None, None)
}

fn empty_socpak_scene_root_mesh() -> crate::types::Mesh {
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

/// Write NMC node properties for a geometry path into the JSON output.
fn dump_nmc_nodes(out: &mut String, key: &str, p4k: &MappedP4k, geom: &str, mtl: &str) {
    use std::fmt::Write;
    if geom.is_empty() {
        return;
    }
    let (nmc, _) = load_nmc_and_material(p4k, geom, mtl);
    if let Some(ref nmc) = nmc {
        let _ = write!(out, "  \"{key}\": [\n");
        for node in &nmc.nodes {
            let _ = write!(
                out,
                "    {{\"node\": {:?}, \"type\": {}",
                node.name, node.geometry_type
            );
            // Include bone_to_world for non-identity transforms (helpers, attachment points)
            let b = &node.bone_to_world;
            let is_identity = (b[0][0] - 1.0).abs() < 0.001
                && (b[1][1] - 1.0).abs() < 0.001
                && (b[2][2] - 1.0).abs() < 0.001
                && b[0][3].abs() < 0.001
                && b[1][3].abs() < 0.001
                && b[2][3].abs() < 0.001
                && b[0][1].abs() < 0.001
                && b[0][2].abs() < 0.001
                && b[1][0].abs() < 0.001
                && b[1][2].abs() < 0.001
                && b[2][0].abs() < 0.001
                && b[2][1].abs() < 0.001;
            if !is_identity {
                let _ = write!(
                    out,
                    ", \"bone_to_world\": [[{:.4},{:.4},{:.4},{:.4}],[{:.4},{:.4},{:.4},{:.4}],[{:.4},{:.4},{:.4},{:.4}]]",
                    b[0][0],
                    b[0][1],
                    b[0][2],
                    b[0][3],
                    b[1][0],
                    b[1][1],
                    b[1][2],
                    b[1][3],
                    b[2][0],
                    b[2][1],
                    b[2][2],
                    b[2][3]
                );
            }
            if !node.properties.is_empty() {
                let _ = write!(out, ", \"props\": {{");
                for (i, (k, v)) in node.properties.iter().enumerate() {
                    if i > 0 {
                        let _ = write!(out, ", ");
                    }
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
    let mtl_compiled = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
        )
        .ok();
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
                    let nodes_with_props: Vec<_> = nmc
                        .nodes
                        .iter()
                        .filter(|n| !n.properties.is_empty())
                        .collect();
                    if !nodes_with_props.is_empty() {
                        let _ = write!(out, ",\n{indent}  \"nmc_properties\": [\n");
                        for n in &nodes_with_props {
                            let _ =
                                write!(out, "{indent}    {{\"node\": {:?}, \"props\": {{", n.name);
                            for (i, (k, v)) in n.properties.iter().enumerate() {
                                if i > 0 {
                                    let _ = write!(out, ", ");
                                }
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
                dump_loadout_nodes(
                    out,
                    p4k,
                    &node.children,
                    &node.entity_name,
                    depth + 2,
                    invisible_ports,
                );
                let _ = write!(out, "{indent}  ]\n");
            } else {
                let _ = write!(out, "\n");
            }
            let _ = write!(out, "{indent}}},\n");
        }
    }
    dump_loadout_nodes(
        &mut out,
        p4k,
        &tree.root.children,
        &tree.root.entity_name,
        0,
        &invisible_ports,
    );
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
        for placement in &container.placements {
            let entry = &interiors.unique_cgfs[placement.mesh_index];
            let tx = placement.transform[3][0];
            let ty = placement.transform[3][1];
            let tz = placement.transform[3][2];
            // Extract scale from rotation columns
            let sx = (placement.transform[0][0] * placement.transform[0][0]
                + placement.transform[0][1] * placement.transform[0][1]
                + placement.transform[0][2] * placement.transform[0][2])
                .sqrt();
            let sy = (placement.transform[1][0] * placement.transform[1][0]
                + placement.transform[1][1] * placement.transform[1][1]
                + placement.transform[1][2] * placement.transform[1][2])
                .sqrt();
            let sz = (placement.transform[2][0] * placement.transform[2][0]
                + placement.transform[2][1] * placement.transform[2][1]
                + placement.transform[2][2] * placement.transform[2][2])
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

    use starbreaker_datacore::types::{CigGuid, StringId, StringId2};

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
            detach_direction: [0.0; 3],
            port_flags: String::new(),
            nmc: with_nmc.then_some(crate::nmc::NodeMeshCombo {
                nodes: Vec::new(),
                material_indices: Vec::new(),
            }),
            bones: Vec::new(),
            has_geometry,
            record: dummy_record(),
            geometry_path: has_geometry.then(|| format!("Data/Objects/{entity_name}.skin")),
            material_path: has_geometry.then(|| format!("Data/Objects/{entity_name}.mtl")),
            allows_child_object_containers: true,
            children,
        }
    }

    fn sample_mesh(node_parent_indices: &[u16]) -> crate::types::Mesh {
        let submeshes = node_parent_indices
            .iter()
            .enumerate()
            .map(|(index, node_parent_index)| crate::types::SubMesh {
                material_name: None,
                material_id: index as u32,
                source_material_id: None,
                first_index: (index as u32) * 3,
                num_indices: 3,
                first_vertex: (index as u32) * 3,
                num_vertices: 3,
                node_parent_index: *node_parent_index,
            })
            .collect();

        crate::types::Mesh {
            positions: vec![[0.0, 0.0, 0.0]; node_parent_indices.len() * 3],
            indices: (0..(node_parent_indices.len() as u32 * 3)).collect(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes,
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
        }
    }

    fn sample_bone(
        name: &str,
        parent_index: Option<u16>,
        local_position: [f32; 3],
        world_position: [f32; 3],
    ) -> crate::skeleton::Bone {
        crate::skeleton::Bone {
            name: name.to_string(),
            parent_index,
            object_node_index: None,
            local_position,
            local_rotation: [1.0, 0.0, 0.0, 0.0],
            world_position,
            world_rotation: [1.0, 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn export_options_default_to_bundled_kind() {
        assert_eq!(ExportOptions::default().kind, ExportKind::Bundled);
    }

    #[test]
    fn synthetic_skin_nmc_uses_root_relative_bone_transforms() {
        let mesh = sample_mesh(&[0, 1]);
        let bones = vec![
            sample_bone("root", None, [2.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            sample_bone("foot", Some(0), [1.5, 2.5, 0.0], [5.0, 4.0, 0.0]),
        ];

        let nmc =
            synthesize_nmc_from_bones(&mesh, &bones).expect("expected a synthetic node hierarchy");

        assert_eq!(nmc.nodes.len(), 2);
        assert_eq!(nmc.nodes[0].name, "root");
        assert_eq!(nmc.nodes[1].name, "foot");
        assert_eq!(nmc.nodes[1].parent_index, Some(0));
        assert_eq!(nmc.nodes[0].bone_to_world[0][3], 0.0);
        assert_eq!(nmc.nodes[0].bone_to_world[1][3], 0.0);
        assert_eq!(nmc.nodes[0].bone_to_world[2][3], 0.0);
        assert_eq!(nmc.nodes[1].bone_to_world[0][3], 1.5);
        assert_eq!(nmc.nodes[1].bone_to_world[1][3], 2.5);
        assert_eq!(nmc.nodes[1].bone_to_world[2][3], 0.0);
    }

    #[test]
    fn synthetic_skin_rebases_rigid_submesh_vertices_to_bone_space() {
        let mut mesh = crate::types::Mesh {
            positions: vec![
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
                [5.0, 4.0, 0.0],
                [6.0, 4.0, 0.0],
                [5.0, 5.0, 0.0],
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![
                crate::types::SubMesh {
                    material_name: None,
                    material_id: 0,
                    source_material_id: None,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 0,
                },
                crate::types::SubMesh {
                    material_name: None,
                    material_id: 1,
                    source_material_id: None,
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 1,
                },
            ],
            model_min: [2.0, 0.0, 0.0],
            model_max: [6.0, 5.0, 0.0],
            scaling_min: [2.0, 0.0, 0.0],
            scaling_max: [6.0, 5.0, 0.0],
        };
        let bones = vec![
            sample_bone("root", None, [2.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            sample_bone("foot", Some(0), [3.0, 4.0, 0.0], [5.0, 4.0, 0.0]),
        ];

        assert!(rebase_mesh_submeshes_to_bone_space(&mut mesh, &bones));
        assert_eq!(mesh.positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(mesh.positions[1], [1.0, 0.0, 0.0]);
        assert_eq!(mesh.positions[2], [0.0, 1.0, 0.0]);
        assert_eq!(mesh.positions[3], [0.0, 0.0, 0.0]);
        assert_eq!(mesh.positions[4], [1.0, 0.0, 0.0]);
        assert_eq!(mesh.positions[5], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn synthetic_skin_nmc_skips_single_node_meshes() {
        let mesh = sample_mesh(&[0, 0]);
        let bones = vec![sample_bone("root", None, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0])];

        assert!(synthesize_nmc_from_bones(&mesh, &bones).is_none());
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
            vec![resolved_node(
                "weapon",
                "hardpoint_weapon",
                true,
                false,
                Vec::new(),
            )],
        );
        let rack = resolved_node(
            "rack",
            "hardpoint_rack",
            false,
            true,
            vec![resolved_node(
                "missile",
                "hardpoint_missile",
                true,
                false,
                Vec::new(),
            )],
        );

        let mut specs = Vec::new();
        collect_child_payload_specs(&[proxy, rack], "root_ship", None, &mut specs);

        assert_eq!(specs.len(), 4);
        assert_eq!(specs[0].child.entity_name, "proxy");
        assert_eq!(specs[0].parent_entity_name, "root_ship");
        assert_eq!(specs[0].parent_node_name, "hardpoint_proxy");

        assert_eq!(specs[1].child.entity_name, "weapon");
        assert_eq!(specs[1].parent_entity_name, "root_ship");
        assert_eq!(specs[1].parent_node_name, "hardpoint_proxy");

        assert_eq!(specs[2].child.entity_name, "rack");
        assert_eq!(specs[2].parent_entity_name, "root_ship");
        assert_eq!(specs[2].parent_node_name, "hardpoint_rack");

        assert_eq!(specs[3].child.entity_name, "missile");
        assert_eq!(specs[3].parent_entity_name, "rack");
        assert_eq!(specs[3].parent_node_name, "hardpoint_missile");
    }

    #[test]
    fn tint_palette_family_keys_include_short_name_and_family_suffix() {
        let keys = tint_palette_family_keys("EntityClassDefinition.rsi_aurora_mk2");

        assert_eq!(
            keys,
            vec!["aurora_mk2".to_string(), "rsi_aurora_mk2".to_string()]
        );
    }

    #[test]
    fn tint_palette_family_matching_accepts_family_variants_only() {
        let keys = tint_palette_family_keys("rsi_aurora_mk2");

        assert!(tint_palette_matches_family("rsi_aurora_mk2", &keys));
        assert!(tint_palette_matches_family(
            "aurora_mk2_pink_green_purple",
            &keys
        ));
        assert!(!tint_palette_matches_family(
            "rsi_interior_aurora_mk2_base",
            &keys
        ));
        assert!(!tint_palette_matches_family(
            "misc_freelancer_black_red",
            &keys
        ));
    }

    #[test]
    fn export_kind_dispatch_accepts_decomposed_backend() {
        let opts = ExportOptions {
            kind: ExportKind::Decomposed,
            ..ExportOptions::default()
        };

        ensure_supported_export_options(&opts).expect("decomposed export kind should be accepted");
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

    #[test]
    fn skeleton_source_paths_include_direct_skin_geometry() {
        assert_eq!(
            skeleton_source_paths(None, "Data/Objects/Ships/Test/gear.skin"),
            vec!["Data/Objects/Ships/Test/gear.skin"]
        );
    }

    #[test]
    fn skeleton_source_paths_prefer_explicit_skeleton_before_geometry() {
        assert_eq!(
            skeleton_source_paths(
                Some("Data/Objects/Ships/Test/gear.chr"),
                "Data/Objects/Ships/Test/gear.skin"
            ),
            vec![
                "Data/Objects/Ships/Test/gear.chr",
                "Data/Objects/Ships/Test/gear.skin",
            ]
        );
    }
}
