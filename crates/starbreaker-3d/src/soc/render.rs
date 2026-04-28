//! Mesh resolution + glTF emission for SOC scenes.
//!
//! This module turns a [`super::scene::ComposedScene`] into a self-contained
//! `.glb` byte vector that the existing GLB loader (Three.js GLTFLoader, the
//! viewer's mesh decoder) can ingest. The output is organised so the next
//! iteration can wire it into a Tauri command and render it in the Maps
//! tab without further format work.
//!
//! # Pipeline
//!
//! 1. **Resolve meshes.** Every brush placement in the scene references a
//!    StatObj path (a `.cgf` / `.cgfm` pair under `Data\Objects`). We walk
//!    the unique-mesh set, look up the geometry bytes through the
//!    [`MappedP4k`] handle, and parse them with the existing
//!    [`crate::parse_skin_with_options`] entry point. Failed loads are
//!    logged and the offending instance is dropped from the output rather
//!    than aborting the whole scene.
//! 2. **Dedupe.** The dedupe key is the canonical mesh path
//!    (lower-cased, backslash-normalised). The reuse rate is high — Exec
//!    Hangar collapses tens of thousands of brush placements down to a few
//!    thousand unique CGFs, which keeps the emitted glTF small enough for
//!    the frontend to load through `URL.createObjectURL`.
//! 3. **Resolve materials.** For each unique mesh we follow the same
//!    "MtlName chunk first, sibling `.mtl` second" priority the existing
//!    pipeline uses (via [`crate::mtl::extract_mtl_name`] and a sibling
//!    fallback). Failures degrade to a neutral default material so the
//!    emitter never aborts.
//! 4. **Emit a single GLB.** One glTF mesh per unique CGF, one node per
//!    placement carrying the world-space 4x4, lights as
//!    `KHR_lights_punctual` entries, materials with a `submat_index`
//!    extra so a downstream loader can bind by index without re-parsing
//!    names.
//!
//! All file paths exchanged with [`MappedP4k`] use Windows-style
//! backslashes and the `Data\` prefix when needed, matching the
//! convention the in-tree p4k library expects.

use std::collections::HashMap;

use glam::{DQuat, DVec3, Mat4};
use gltf_json as json;
use json::validation::Checked;
use starbreaker_p4k::MappedP4k;

use crate::Mesh;
use crate::mtl;
use crate::parse_skin_with_options;
use crate::soc::entities::{EntityKind, EntityPlacement};
use crate::soc::scene::ComposedScene;

// ── Public types ────────────────────────────────────────────────────────────

/// One unique mesh used by the renderable scene, resolved against the p4k.
#[derive(Debug, Clone)]
pub struct ResolvedMesh {
    /// Canonical path used as the dedupe key. Lower-cased, backslash form.
    pub canonical_path: String,
    /// Geometry decoded from the CGF / CGFM pair.
    pub mesh: Mesh,
    /// Material file resolved for this mesh, when one was found.
    pub material: Option<mtl::MtlFile>,
}

/// One placement of a [`ResolvedMesh`] at a world-space transform.
#[derive(Debug, Clone)]
pub struct MeshPlacement {
    /// Index into [`RenderableScene::meshes`].
    pub mesh_index: usize,
    /// World-space transform stored row-major as 3x4 (the SOC parser
    /// stores `[[f32; 4]; 3]`); we promote it to a 4x4 column-major
    /// matrix at glTF emission time.
    pub world_transform: [[f32; 4]; 3],
    /// Stable identifier — the global brush index in the source
    /// composed scene. Useful for downstream debugging and for matching
    /// node names back to source data.
    pub instance_id: u32,
}

/// Light source variants the emitter can encode through
/// `KHR_lights_punctual`.
#[derive(Debug, Clone)]
pub enum LightKind {
    Point,
    Spot {
        /// Inner cone angle in radians.
        inner_cone: f32,
        /// Outer cone angle in radians.
        outer_cone: f32,
    },
    Directional,
}

/// Per-light descriptor, neutral enough that the frontend can map it onto
/// `THREE.PointLight` / `SpotLight` / `DirectionalLight` without further
/// translation.
#[derive(Debug, Clone)]
pub struct LightDescriptor {
    /// Original entity name (may be empty).
    pub name: String,
    pub kind: LightKind,
    /// Linear RGB color, `[0,1]` per channel.
    pub color: [f32; 3],
    /// Candela for point / spot, lux for directional.
    pub intensity: f32,
    /// Range cutoff in metres, `0.0` for "engine default".
    pub range: f32,
}

/// One light placed at a world-space transform.
#[derive(Debug, Clone)]
pub struct LightInstance {
    pub world_transform: [[f32; 4]; 3],
    pub descriptor: LightDescriptor,
}

/// A scene whose meshes have been resolved and whose placements + lights
/// are ready for glTF emission.
#[derive(Debug, Clone, Default)]
pub struct RenderableScene {
    pub meshes: Vec<ResolvedMesh>,
    pub placements: Vec<MeshPlacement>,
    pub lights: Vec<LightInstance>,
    /// World-space AABB across every brush placement, or `None` if the
    /// scene has no placements. Stored in the parser's coordinate frame
    /// (CryEngine Z-up).
    pub aabb: Option<([f32; 3], [f32; 3])>,
    /// How many brush placements were dropped because their mesh failed
    /// to resolve. Reported back to the supervisor so the smoke test can
    /// surface mesh-load regressions.
    pub dropped_placements: u32,
    /// How many unique mesh paths failed to resolve. A unique-mesh failure
    /// can drop many placements — the two counters are tracked separately.
    pub failed_mesh_paths: u32,
    /// How many resolved meshes successfully loaded an MTL file (either via
    /// the MtlName chunk or the sibling fallback). Surfaced so the
    /// frontend can show a coverage indicator: when this drops far below
    /// `meshes.len()`, materials are mostly the neutral default.
    pub materials_resolved: u32,
    /// How many resolved meshes have no MTL bound and are using the
    /// neutral default material (everything in `meshes` minus
    /// `materials_resolved`).
    pub materials_default: u32,
}

/// Result returned by [`emit_glb`]: the GLB byte vector plus a summary
/// for the caller's manifest / progress UI. Plain fields, easy to copy
/// into a `serde_json::json!` blob at the Tauri layer.
#[derive(Debug, Clone)]
pub struct GlbEmitSummary {
    pub mesh_count: u32,
    pub placement_count: u32,
    /// Lights actually emitted into the GLB. Equal to
    /// `min(scene.lights.len(), max_lights)`.
    pub light_count: u32,
    /// Lights present in the input scene but skipped because the
    /// importance-sorted top-N cap was reached. Surface this on the UI
    /// so the user knows lighting is approximate.
    pub lights_dropped: u32,
    pub material_count: u32,
    pub texture_count: u32,
    pub aabb_min: Option<[f32; 3]>,
    pub aabb_max: Option<[f32; 3]>,
}

/// Configuration knobs for [`emit_glb`]. Defaults match the values most
/// callers want; pass an explicit value when iterating on the cap.
#[derive(Debug, Clone, Copy)]
pub struct EmitOptions {
    /// Maximum number of lights to encode into the GLB. The emitter
    /// sorts the input lights by descending intensity and keeps the top
    /// N; the rest are skipped. Forward-rendered three.js shaders
    /// allocate ~10 fragment uniforms per light, so going above ~64 on
    /// a 1024-uniform GPU exceeds `MAX_FRAGMENT_UNIFORM_VECTORS` and
    /// silently fails to compile every lit material in the scene.
    pub max_lights: usize,
}

/// Default cap on the number of lights emitted. Conservative: a handful
/// of GPUs report `MAX_FRAGMENT_UNIFORM_VECTORS = 1024`; at ~10 uniforms
/// per point light plus the per-material baseline, 64 fits comfortably
/// inside the cap on all of them. Drop to 32 if a user reports the
/// shader-compile error on this default.
pub const DEFAULT_MAX_EMITTED_LIGHTS: usize = 64;

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            max_lights: DEFAULT_MAX_EMITTED_LIGHTS,
        }
    }
}

// ── Mesh resolution ─────────────────────────────────────────────────────────

/// Resolve every brush placement in `scene` against the p4k, producing a
/// [`RenderableScene`] ready for emission. Per-mesh failures are logged
/// at warn level and surface in `RenderableScene::failed_mesh_paths`;
/// per-placement failures are counted in `dropped_placements` so callers
/// can spot a sudden regression.
pub fn resolve_scene(p4k: &MappedP4k, scene: &ComposedScene) -> RenderableScene {
    resolve_scene_with_progress(p4k, scene, &mut |_, _| {})
}

/// Like [`resolve_scene`] but reports per-mesh resolution progress
/// through `on_progress(current, total)`. Callers (e.g. the Tauri
/// command surface) can throttle and forward these updates as
/// `scene-load-progress` events.
pub fn resolve_scene_with_progress<F>(
    p4k: &MappedP4k,
    scene: &ComposedScene,
    on_progress: &mut F,
) -> RenderableScene
where
    F: FnMut(usize, usize),
{
    let mut out = RenderableScene::default();

    // First pass: collect unique mesh paths from every zone's brush set.
    let mut path_to_index: HashMap<String, usize> = HashMap::new();
    let mut canonical_paths: Vec<String> = Vec::new();
    let mut original_paths: Vec<String> = Vec::new();

    for zone in &scene.zones {
        for brush in &zone.brushes {
            let mesh_path = match zone.mesh_paths.get(brush.mesh_index as usize) {
                Some(p) => p,
                None => continue,
            };
            let canonical = canonicalize_mesh_path(mesh_path);
            if !path_to_index.contains_key(&canonical) {
                path_to_index.insert(canonical.clone(), canonical_paths.len());
                canonical_paths.push(canonical);
                original_paths.push(mesh_path.clone());
            }
        }
    }

    // Second pass: try to load each unique mesh. Track which canonical
    // paths fell through so we can fold their placement-side losses into
    // `dropped_placements`.
    let total_unique = canonical_paths.len();
    let mut canonical_to_resolved: HashMap<String, usize> = HashMap::new();
    for (i, (canonical, original)) in canonical_paths
        .iter()
        .zip(original_paths.iter())
        .enumerate()
    {
        on_progress(i, total_unique);
        match resolve_one_mesh(p4k, original) {
            Some(resolved) => {
                let resolved_index = out.meshes.len();
                canonical_to_resolved.insert(canonical.clone(), resolved_index);
                let has_material = resolved.1.is_some();
                out.meshes.push(ResolvedMesh {
                    canonical_path: canonical.clone(),
                    mesh: resolved.0,
                    material: resolved.1,
                });
                if has_material {
                    out.materials_resolved += 1;
                } else {
                    out.materials_default += 1;
                }
            }
            None => {
                out.failed_mesh_paths += 1;
            }
        }
    }
    on_progress(total_unique, total_unique);

    // Third pass: emit one placement per brush. Drop placements whose
    // mesh failed to resolve.
    let mut global_brush_index: u32 = 0;
    for zone in &scene.zones {
        for brush in &zone.brushes {
            let mesh_path = match zone.mesh_paths.get(brush.mesh_index as usize) {
                Some(p) => p,
                None => {
                    out.dropped_placements += 1;
                    global_brush_index = global_brush_index.wrapping_add(1);
                    continue;
                }
            };
            let canonical = canonicalize_mesh_path(mesh_path);
            let Some(&mesh_index) = canonical_to_resolved.get(&canonical) else {
                out.dropped_placements += 1;
                global_brush_index = global_brush_index.wrapping_add(1);
                continue;
            };
            out.placements.push(MeshPlacement {
                mesh_index,
                world_transform: brush.world_transform,
                instance_id: global_brush_index,
            });
            update_aabb(&mut out.aabb, &brush.translation);
            global_brush_index = global_brush_index.wrapping_add(1);
        }
    }

    // Fourth pass: lights. We keep every light entity, even ones with no
    // explicit colour / intensity, and rely on a sane default downstream.
    for zone in &scene.zones {
        for entity in &zone.entities {
            if entity.kind != EntityKind::Light {
                continue;
            }
            out.lights.push(LightInstance {
                world_transform: entity_world_transform(entity, zone.parent_translation,
                    zone.parent_rotation),
                descriptor: light_from_entity(entity),
            });
        }
    }

    out
}

/// Lower-case + backslash-normalise a mesh path, then strip any leading
/// `data\` so reuse works across the few different conventions the SOC
/// data uses.
fn canonicalize_mesh_path(path: &str) -> String {
    let lc = path.to_ascii_lowercase().replace('/', "\\");
    lc.strip_prefix("data\\").map(|s| s.to_string()).unwrap_or(lc)
}

fn resolve_one_mesh(
    p4k: &MappedP4k,
    original_path: &str,
) -> Option<(Mesh, Option<mtl::MtlFile>)> {
    let geom_bytes = read_geom_bytes(p4k, original_path)?;
    let mesh = match parse_skin_with_options(&geom_bytes, false) {
        Ok(m) => m,
        Err(err) => {
            log::warn!("[soc-render] mesh parse failed for {original_path}: {err:?}");
            return None;
        }
    };
    let metadata = read_metadata_bytes(p4k, original_path);
    let material = resolve_material_for_mesh(p4k, original_path, metadata.as_deref());
    Some((mesh, material))
}

/// Read the geometry payload for a mesh. Most SOC brushes ship as a
/// `.cgf` + `.cgfm` pair where the `.cgfm` carries the IVO chunks the
/// parser wants. We try the sibling `.cgfm` first; if absent, the parent
/// `.cgf` itself is the IVO container.
fn read_geom_bytes(p4k: &MappedP4k, original_path: &str) -> Option<Vec<u8>> {
    let primary = p4k_path_for(original_path);
    let companion = format!("{primary}m");
    if let Some(entry) = p4k.entry_case_insensitive(&companion)
        && let Ok(bytes) = p4k.read(entry)
    {
        return Some(bytes);
    }
    if let Some(entry) = p4k.entry_case_insensitive(&primary)
        && let Ok(bytes) = p4k.read(entry)
    {
        return Some(bytes);
    }
    None
}

/// Read the primary mesh file (the `.cgf` itself) — used as the source of
/// the `MtlName` chunk for material resolution.
fn read_metadata_bytes(p4k: &MappedP4k, original_path: &str) -> Option<Vec<u8>> {
    let primary = p4k_path_for(original_path);
    let entry = p4k.entry_case_insensitive(&primary)?;
    p4k.read(entry).ok()
}

/// Convert a SOC-table mesh path into the canonical `Data\...` form the
/// p4k library expects.
fn p4k_path_for(original_path: &str) -> String {
    let normalised = original_path.replace('/', "\\");
    if normalised
        .to_ascii_lowercase()
        .starts_with("data\\")
    {
        normalised
    } else {
        format!("Data\\{normalised}")
    }
}

/// Resolve a material for a mesh by trying the on-disk `MtlName` chunk
/// first and falling back to the same-name sibling `.mtl`.
fn resolve_material_for_mesh(
    p4k: &MappedP4k,
    original_path: &str,
    metadata_bytes: Option<&[u8]>,
) -> Option<mtl::MtlFile> {
    let primary = p4k_path_for(original_path);

    if let Some(metadata) = metadata_bytes
        && let Some(name) = mtl::extract_mtl_name(metadata)
    {
        let mtl_path = mtl_path_for(&name, &primary);
        if let Some(material) = read_mtl(p4k, &mtl_path) {
            return Some(material);
        }
    }

    // Sibling `.mtl` fallback — same stem next to the geometry.
    let sibling = sibling_mtl_for(&primary);
    if let Some(material) = read_mtl(p4k, &sibling) {
        return Some(material);
    }

    None
}

fn read_mtl(p4k: &MappedP4k, p4k_path: &str) -> Option<mtl::MtlFile> {
    let entry = p4k.entry_case_insensitive(p4k_path)?;
    let data = p4k.read(entry).ok()?;
    let mut mtl = mtl::parse_mtl(&data).ok()?;
    mtl.source_path = Some(p4k_path.to_string());
    Some(mtl)
}

/// Compose an `MtlName` chunk's value with the mesh path to get the
/// material's p4k location. Names that contain a path separator are taken
/// verbatim (rooted under `Data\`); plain names are looked up as a sibling
/// of the mesh.
fn mtl_path_for(mtl_name: &str, p4k_geom_path: &str) -> String {
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

fn sibling_mtl_for(p4k_geom_path: &str) -> String {
    if let Some(dot) = p4k_geom_path.rfind('.') {
        format!("{}.mtl", &p4k_geom_path[..dot])
    } else {
        format!("{p4k_geom_path}.mtl")
    }
}

// ── AABB tracking ───────────────────────────────────────────────────────────

fn update_aabb(aabb: &mut Option<([f32; 3], [f32; 3])>, point: &[f32; 3]) {
    match aabb {
        None => *aabb = Some((*point, *point)),
        Some((mn, mx)) => {
            for i in 0..3 {
                if point[i] < mn[i] {
                    mn[i] = point[i];
                }
                if point[i] > mx[i] {
                    mx[i] = point[i];
                }
            }
        }
    }
}

// ── Lights ──────────────────────────────────────────────────────────────────

/// Default colour when the entity does not carry a tint. Slightly warm
/// white to match the engine's neutral lamp setup.
const LIGHT_DEFAULT_COLOR: [f32; 3] = [1.0, 0.95, 0.88];
/// Default candela value when the entity does not advertise an intensity.
const LIGHT_DEFAULT_INTENSITY: f32 = 1.0;
/// Default range cutoff in metres. Zero tells `KHR_lights_punctual`
/// consumers to use their own engine default.
const LIGHT_DEFAULT_RANGE: f32 = 0.0;

/// Build a `LightDescriptor` from one entity. We only have placement +
/// class on the SOC side; colour / intensity / radius come from
/// `PropertiesDataCore` records that the renderer or DataCore layer
/// resolves later. The descriptor encodes sane defaults so the glTF
/// emitter has a complete record to write.
fn light_from_entity(entity: &EntityPlacement) -> LightDescriptor {
    let kind = match entity.entity_class.as_str() {
        // The SOC stream does not yet split spot vs point — every light
        // placement is treated as a point light at this layer. Iteration
        // C can split spotlights once the DataCore-side classifier lands.
        _ => LightKind::Point,
    };
    LightDescriptor {
        name: if entity.name.is_empty() {
            entity.entity_class.clone()
        } else {
            entity.name.clone()
        },
        kind,
        color: LIGHT_DEFAULT_COLOR,
        intensity: LIGHT_DEFAULT_INTENSITY,
        range: LIGHT_DEFAULT_RANGE,
    }
}

/// Recover a 3x4 row-major world transform for an entity by composing
/// its parent QuatTS with its placement. `EntityPlacement` already
/// stores world-space translation + rotation; we just re-pack into the
/// emitter's expected layout (column rotation followed by translation).
fn entity_world_transform(
    entity: &EntityPlacement,
    _parent_t: [f64; 3],
    _parent_r: [f64; 4],
) -> [[f32; 4]; 3] {
    let q = DQuat::from_xyzw(
        entity.rotation[0] as f64,
        entity.rotation[1] as f64,
        entity.rotation[2] as f64,
        entity.rotation[3] as f64,
    )
    .normalize();
    let t = DVec3::new(
        entity.translation[0] as f64,
        entity.translation[1] as f64,
        entity.translation[2] as f64,
    );
    let cols = [
        q * DVec3::X,
        q * DVec3::Y,
        q * DVec3::Z,
    ];
    [
        [cols[0].x as f32, cols[1].x as f32, cols[2].x as f32, t.x as f32],
        [cols[0].y as f32, cols[1].y as f32, cols[2].y as f32, t.y as f32],
        [cols[0].z as f32, cols[1].z as f32, cols[2].z as f32, t.z as f32],
    ]
}

// ── glTF emission ───────────────────────────────────────────────────────────

/// Convert a 3x4 row-major matrix into glTF's flat 16-element column-major
/// layout (matrices in glTF are column-major).
fn mat3x4_to_gltf_matrix(m: &[[f32; 4]; 3]) -> [f32; 16] {
    [
        m[0][0], m[1][0], m[2][0], 0.0,
        m[0][1], m[1][1], m[2][1], 0.0,
        m[0][2], m[1][2], m[2][2], 0.0,
        m[0][3], m[1][3], m[2][3], 1.0,
    ]
}

/// Emit a self-contained `.glb` byte vector for the given
/// [`RenderableScene`]. The emitted file follows glTF 2.0 spec and uses
/// `KHR_lights_punctual` for every light entity. Uses
/// [`EmitOptions::default`].
pub fn emit_glb(scene: &RenderableScene) -> Result<(Vec<u8>, GlbEmitSummary), String> {
    emit_glb_with_options(scene, EmitOptions::default())
}

/// Like [`emit_glb`] but takes an explicit [`EmitOptions`] so callers
/// can override the light cap.
pub fn emit_glb_with_options(
    scene: &RenderableScene,
    options: EmitOptions,
) -> Result<(Vec<u8>, GlbEmitSummary), String> {
    let mut bin: Vec<u8> = Vec::new();
    let mut buffer_views: Vec<json::buffer::View> = Vec::new();
    let mut accessors: Vec<json::Accessor> = Vec::new();
    let images: Vec<json::Image> = Vec::new();
    let textures: Vec<json::Texture> = Vec::new();
    let samplers: Vec<json::texture::Sampler> = Vec::new();
    let mut materials: Vec<json::Material> = Vec::new();
    let mut meshes: Vec<json::Mesh> = Vec::new();
    let mut nodes: Vec<json::Node> = Vec::new();

    // Default neutral material returned for meshes that did not resolve a
    // material. Stays at index 0; per-mesh materials append after it.
    let default_material_index = materials.len() as u32;
    materials.push(neutral_material());

    // Per-resolved-mesh: list of glTF material indices, one per submesh.
    let mut mesh_material_indices: Vec<Vec<u32>> = Vec::with_capacity(scene.meshes.len());

    // ── Pack per-mesh materials ─────────────────────────────────────────
    for resolved in &scene.meshes {
        let mut indices = Vec::with_capacity(resolved.mesh.submeshes.len());
        for submesh in &resolved.mesh.submeshes {
            let mat_idx = match resolved.material.as_ref() {
                Some(mtl) => build_glb_material(
                    mtl,
                    submesh.material_id as usize,
                    &mut materials,
                ),
                None => default_material_index,
            };
            indices.push(mat_idx);
        }
        mesh_material_indices.push(indices);
    }

    // ── Pack per-mesh geometry ──────────────────────────────────────────
    let mut mesh_indices_by_resolved: Vec<u32> = Vec::with_capacity(scene.meshes.len());
    for (resolved_idx, resolved) in scene.meshes.iter().enumerate() {
        let mat_indices = &mesh_material_indices[resolved_idx];
        let mesh_idx = pack_mesh(
            &resolved.mesh,
            &resolved.canonical_path,
            mat_indices,
            &mut buffer_views,
            &mut accessors,
            &mut meshes,
            &mut bin,
        )?;
        mesh_indices_by_resolved.push(mesh_idx);
    }

    // ── Emit one node per placement ─────────────────────────────────────
    let mut scene_nodes: Vec<json::Index<json::Node>> = Vec::new();

    for placement in &scene.placements {
        let mesh_idx = mesh_indices_by_resolved
            .get(placement.mesh_index)
            .copied()
            .ok_or_else(|| format!("placement references missing mesh {}", placement.mesh_index))?;
        let matrix = mat3x4_to_gltf_matrix(&placement.world_transform);
        let identity = is_identity_matrix(&matrix);

        let mut extras_map = serde_json::Map::new();
        extras_map.insert(
            "instance_id".into(),
            serde_json::json!(placement.instance_id),
        );
        let extras_string = serde_json::to_string(&serde_json::Value::Object(extras_map))
            .map_err(|e| format!("instance extras serialize: {e}"))?;
        let extras = serde_json::value::RawValue::from_string(extras_string)
            .map_err(|e| format!("instance extras raw value: {e}"))?
            .into();

        let node_idx = nodes.len() as u32;
        nodes.push(json::Node {
            mesh: Some(json::Index::new(mesh_idx)),
            matrix: if identity { None } else { Some(matrix) },
            name: Some(format!("brush_{}", placement.instance_id)),
            extras: Some(extras),
            ..Default::default()
        });
        scene_nodes.push(json::Index::new(node_idx));
    }

    // ── Cap the light list ─────────────────────────────────────────────
    // Forward-rendered fragments allocate uniforms per active light, so
    // an unbounded scene blows past `MAX_FRAGMENT_UNIFORM_VECTORS` and
    // every lit material in the WebGL pipeline silently fails to
    // compile. Sort by descending intensity, take the top-N, count the
    // rest as `lights_dropped`.
    let total_light_input = scene.lights.len() as u32;
    let kept_lights = pick_top_lights(&scene.lights, options.max_lights);
    let lights_dropped = total_light_input.saturating_sub(kept_lights.len() as u32);

    // ── Emit one node per kept light, each carrying a KHR_lights_punctual ref
    let lights_root = build_lights_root(&kept_lights, &mut nodes, &mut scene_nodes);

    // ── Finalise the glTF root ──────────────────────────────────────────
    let extensions_used = if kept_lights.is_empty() {
        Vec::new()
    } else {
        vec!["KHR_lights_punctual".to_string()]
    };

    let scene_root = json::Scene {
        nodes: scene_nodes,
        name: Some("soc_scene".to_string()),
        extensions: None,
        extras: Default::default(),
    };

    let aabb_pair = scene.aabb;
    let summary = GlbEmitSummary {
        mesh_count: scene.meshes.len() as u32,
        placement_count: scene.placements.len() as u32,
        light_count: kept_lights.len() as u32,
        lights_dropped,
        material_count: materials.len() as u32,
        texture_count: textures.len() as u32,
        aabb_min: aabb_pair.map(|p| p.0),
        aabb_max: aabb_pair.map(|p| p.1),
    };

    let asset_extras = build_asset_extras(&summary)?;

    let root = json::Root {
        asset: json::Asset {
            generator: Some("starbreaker-soc-scene".into()),
            version: "2.0".into(),
            extras: asset_extras,
            ..Default::default()
        },
        buffers: vec![json::Buffer {
            byte_length: json::validation::USize64(bin.len() as u64),
            uri: None,
            name: None,
            extensions: None,
            extras: Default::default(),
        }],
        buffer_views,
        accessors,
        meshes,
        materials,
        images,
        textures,
        samplers,
        nodes,
        scenes: vec![scene_root],
        scene: Some(json::Index::new(0)),
        extensions_used,
        extensions: lights_root,
        ..Default::default()
    };

    let glb = serialize_glb(&root, &bin)?;
    Ok((glb, summary))
}

fn build_asset_extras(
    summary: &GlbEmitSummary,
) -> Result<Option<Box<serde_json::value::RawValue>>, String> {
    let mut map = serde_json::Map::new();
    map.insert(
        "soc_emitter_version".into(),
        serde_json::json!(env!("CARGO_PKG_VERSION")),
    );
    map.insert("mesh_count".into(), serde_json::json!(summary.mesh_count));
    map.insert(
        "placement_count".into(),
        serde_json::json!(summary.placement_count),
    );
    map.insert("light_count".into(), serde_json::json!(summary.light_count));
    map.insert(
        "lights_dropped".into(),
        serde_json::json!(summary.lights_dropped),
    );
    if let (Some(mn), Some(mx)) = (summary.aabb_min, summary.aabb_max) {
        map.insert("aabb_min".into(), serde_json::json!(mn));
        map.insert("aabb_max".into(), serde_json::json!(mx));
    }
    let s = serde_json::to_string(&serde_json::Value::Object(map))
        .map_err(|e| format!("asset extras serialize: {e}"))?;
    Ok(Some(
        serde_json::value::RawValue::from_string(s)
            .map_err(|e| format!("asset extras raw value: {e}"))?
            .into(),
    ))
}

fn is_identity_matrix(m: &[f32; 16]) -> bool {
    let identity = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    for i in 0..16 {
        if (m[i] - identity[i]).abs() > 1e-6 {
            return false;
        }
    }
    true
}

/// Build a glTF material from one submaterial of an `MtlFile`.
///
/// We deliberately do NOT bind a `baseColorTexture` here, even when the
/// MTL points at a diffuse texture. Earlier iterations embedded a 1x1
/// placeholder PNG and bound every textured material to it; on a scene
/// with thousands of materials, GLTFLoader's per-image blob URLs (one
/// per embedded image) and the placeholder texture's GPU upload
/// lifecycle interacted badly with the texture-resolver substitution
/// step and produced "Couldn't load texture blob:..." errors at render
/// time. Skipping the placeholder removes the failure surface entirely
/// and shrinks the GLB. The downstream loader detects "this material
/// wants a diffuse texture" by reading the `diffuse_texture_path` extra
/// and creates a real `THREE.Texture` on the JS side once the DDS
/// resolves.
fn build_glb_material(
    mtl: &mtl::MtlFile,
    submaterial_id: usize,
    materials: &mut Vec<json::Material>,
) -> u32 {
    let sub = mtl.materials.get(submaterial_id);
    let (base_rgb, opacity, name, diffuse_tex_path, normal_tex_path) = match sub {
        Some(s) => (
            s.diffuse,
            s.opacity,
            Some(format!(
                "{}_{}",
                mtl.source_path
                    .as_ref()
                    .and_then(|p| p.rsplit(['\\', '/']).next())
                    .map(|f| f.strip_suffix(".mtl").unwrap_or(f))
                    .unwrap_or("mtl"),
                if s.name.is_empty() {
                    format!("submat_{submaterial_id}")
                } else {
                    s.name.clone()
                },
            )),
            s.diffuse_tex.clone(),
            s.normal_tex.clone(),
        ),
        None => ([0.8, 0.8, 0.8], 1.0, None, None, None),
    };

    let mut extras_map = serde_json::Map::new();
    extras_map.insert(
        "submat_index".into(),
        serde_json::json!(submaterial_id),
    );
    if let Some(path) = diffuse_tex_path {
        extras_map.insert("diffuse_texture_path".into(), serde_json::json!(path));
    }
    if let Some(path) = normal_tex_path {
        extras_map.insert("normal_texture_path".into(), serde_json::json!(path));
    }
    if let Some(src) = mtl.source_path.as_ref() {
        extras_map.insert("mtl_source_path".into(), serde_json::json!(src));
    }
    let extras_string = serde_json::to_string(&serde_json::Value::Object(extras_map))
        .ok()
        .unwrap_or_default();
    let extras = serde_json::value::RawValue::from_string(extras_string)
        .ok()
        .map(|raw| raw.into());

    let material = json::Material {
        name,
        pbr_metallic_roughness: json::material::PbrMetallicRoughness {
            base_color_factor: json::material::PbrBaseColorFactor([
                base_rgb[0],
                base_rgb[1],
                base_rgb[2],
                opacity,
            ]),
            base_color_texture: None,
            metallic_factor: json::material::StrengthFactor(0.0),
            roughness_factor: json::material::StrengthFactor(0.8),
            metallic_roughness_texture: None,
            extensions: Default::default(),
            extras: Default::default(),
        },
        alpha_mode: Checked::Valid(if opacity < 0.999 {
            json::material::AlphaMode::Blend
        } else {
            json::material::AlphaMode::Opaque
        }),
        double_sided: false,
        extras,
        ..Default::default()
    };
    let idx = materials.len() as u32;
    materials.push(material);
    idx
}

/// Sort the input lights by descending intensity and return at most
/// `max_lights` of them, preserving the `LightInstance` payloads. When
/// `max_lights == 0` returns an empty slice. When the input already fits
/// under the cap, returns it as-is (no clone of the descriptor data
/// payloads beyond the wrapping vector).
fn pick_top_lights(lights: &[LightInstance], max_lights: usize) -> Vec<LightInstance> {
    if max_lights == 0 || lights.is_empty() {
        return Vec::new();
    }
    if lights.len() <= max_lights {
        return lights.to_vec();
    }
    let mut indexed: Vec<(usize, f32)> = lights
        .iter()
        .enumerate()
        .map(|(i, l)| (i, l.descriptor.intensity))
        .collect();
    // Descending by intensity. Use partial_cmp + reverse so NaN inputs
    // sort to the end rather than panicking.
    indexed.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    indexed.truncate(max_lights);
    // Restore the original placement order so the kept set still tracks
    // the level layout — no functional dependency on order, but it
    // makes diffs stable and debugging easier.
    indexed.sort_by_key(|(i, _)| *i);
    indexed
        .into_iter()
        .map(|(i, _)| lights[i].clone())
        .collect()
}

fn neutral_material() -> json::Material {
    json::Material {
        name: Some("soc_default".to_string()),
        pbr_metallic_roughness: json::material::PbrMetallicRoughness {
            base_color_factor: json::material::PbrBaseColorFactor([0.7, 0.7, 0.7, 1.0]),
            base_color_texture: None,
            metallic_factor: json::material::StrengthFactor(0.0),
            roughness_factor: json::material::StrengthFactor(0.85),
            metallic_roughness_texture: None,
            extensions: Default::default(),
            extras: Default::default(),
        },
        alpha_mode: Checked::Valid(json::material::AlphaMode::Opaque),
        double_sided: false,
        ..Default::default()
    }
}

fn align_bin_to_4(bin: &mut Vec<u8>) {
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
}

/// Pack one mesh's geometry into the BIN buffer, register accessors, and
/// emit one glTF Mesh with one primitive per submesh. Returns the glTF
/// mesh index.
fn pack_mesh(
    mesh: &Mesh,
    canonical_path: &str,
    submesh_material_indices: &[u32],
    buffer_views: &mut Vec<json::buffer::View>,
    accessors: &mut Vec<json::Accessor>,
    meshes: &mut Vec<json::Mesh>,
    bin: &mut Vec<u8>,
) -> Result<u32, String> {
    align_bin_to_4(bin);

    // Positions.
    let pos_offset = bin.len();
    for p in &mesh.positions {
        bin.extend_from_slice(&p[0].to_le_bytes());
        bin.extend_from_slice(&p[1].to_le_bytes());
        bin.extend_from_slice(&p[2].to_le_bytes());
    }
    let pos_len = bin.len() - pos_offset;
    let (pos_min, pos_max) = position_min_max(&mesh.positions);

    let pos_bv = buffer_views.len() as u32;
    buffer_views.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_offset: Some(json::validation::USize64(pos_offset as u64)),
        byte_length: json::validation::USize64(pos_len as u64),
        byte_stride: None,
        target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
        name: None,
        extensions: None,
        extras: Default::default(),
    });
    let pos_acc = accessors.len() as u32;
    accessors.push(json::Accessor {
        buffer_view: Some(json::Index::new(pos_bv)),
        byte_offset: Some(json::validation::USize64(0)),
        count: json::validation::USize64(mesh.positions.len() as u64),
        component_type: Checked::Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::F32,
        )),
        type_: Checked::Valid(json::accessor::Type::Vec3),
        min: Some(serde_json::Value::Array(
            pos_min.iter().map(|&v| serde_json::Value::from(v)).collect(),
        )),
        max: Some(serde_json::Value::Array(
            pos_max.iter().map(|&v| serde_json::Value::from(v)).collect(),
        )),
        name: None,
        normalized: false,
        sparse: None,
        extensions: None,
        extras: Default::default(),
    });

    // Indices, packed once for the whole mesh; each submesh gets its own
    // accessor with byte_offset + count for its slice.
    align_bin_to_4(bin);
    let idx_offset = bin.len();
    for &i in &mesh.indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let idx_len = bin.len() - idx_offset;
    let idx_bv = buffer_views.len() as u32;
    buffer_views.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_offset: Some(json::validation::USize64(idx_offset as u64)),
        byte_length: json::validation::USize64(idx_len as u64),
        byte_stride: None,
        target: Some(Checked::Valid(json::buffer::Target::ElementArrayBuffer)),
        name: None,
        extensions: None,
        extras: Default::default(),
    });

    // Optional UVs.
    let uv_acc = mesh.uvs.as_ref().map(|uvs| {
        align_bin_to_4(bin);
        let uv_offset = bin.len();
        for uv in uvs {
            bin.extend_from_slice(&uv[0].to_le_bytes());
            bin.extend_from_slice(&uv[1].to_le_bytes());
        }
        let uv_len = bin.len() - uv_offset;
        let uv_bv = buffer_views.len() as u32;
        buffer_views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_offset: Some(json::validation::USize64(uv_offset as u64)),
            byte_length: json::validation::USize64(uv_len as u64),
            byte_stride: None,
            target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
            name: None,
            extensions: None,
            extras: Default::default(),
        });
        let acc = accessors.len() as u32;
        accessors.push(json::Accessor {
            buffer_view: Some(json::Index::new(uv_bv)),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64(uvs.len() as u64),
            component_type: Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::F32,
            )),
            type_: Checked::Valid(json::accessor::Type::Vec2),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
            extensions: None,
            extras: Default::default(),
        });
        acc
    });

    // Optional normals.
    let normal_acc = mesh.normals.as_ref().map(|norms| {
        align_bin_to_4(bin);
        let off = bin.len();
        for n in norms {
            bin.extend_from_slice(&n[0].to_le_bytes());
            bin.extend_from_slice(&n[1].to_le_bytes());
            bin.extend_from_slice(&n[2].to_le_bytes());
        }
        let len = bin.len() - off;
        let bv = buffer_views.len() as u32;
        buffer_views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_offset: Some(json::validation::USize64(off as u64)),
            byte_length: json::validation::USize64(len as u64),
            byte_stride: None,
            target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
            name: None,
            extensions: None,
            extras: Default::default(),
        });
        let acc = accessors.len() as u32;
        accessors.push(json::Accessor {
            buffer_view: Some(json::Index::new(bv)),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64(norms.len() as u64),
            component_type: Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::F32,
            )),
            type_: Checked::Valid(json::accessor::Type::Vec3),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
            extensions: None,
            extras: Default::default(),
        });
        acc
    });

    // Build one primitive per submesh. Each primitive shares the same
    // POSITION/UV/NORMAL accessors but gets its own index accessor that
    // points at the submesh's slice of the global index buffer.
    let mut primitives: Vec<json::mesh::Primitive> =
        Vec::with_capacity(mesh.submeshes.len().max(1));

    if mesh.submeshes.is_empty() {
        // Whole-mesh fallback for meshes that ship without explicit submeshes.
        let idx_acc = accessors.len() as u32;
        accessors.push(json::Accessor {
            buffer_view: Some(json::Index::new(idx_bv)),
            byte_offset: Some(json::validation::USize64(0)),
            count: json::validation::USize64(mesh.indices.len() as u64),
            component_type: Checked::Valid(json::accessor::GenericComponentType(
                json::accessor::ComponentType::U32,
            )),
            type_: Checked::Valid(json::accessor::Type::Scalar),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
            extensions: None,
            extras: Default::default(),
        });
        primitives.push(make_primitive(
            pos_acc,
            uv_acc,
            normal_acc,
            idx_acc,
            submesh_material_indices.first().copied().unwrap_or(0),
        ));
    } else {
        for (sm_idx, sub) in mesh.submeshes.iter().enumerate() {
            if sub.num_indices == 0 {
                continue;
            }
            let byte_offset = (sub.first_index as usize) * 4;
            let idx_acc = accessors.len() as u32;
            accessors.push(json::Accessor {
                buffer_view: Some(json::Index::new(idx_bv)),
                byte_offset: Some(json::validation::USize64(byte_offset as u64)),
                count: json::validation::USize64(sub.num_indices as u64),
                component_type: Checked::Valid(json::accessor::GenericComponentType(
                    json::accessor::ComponentType::U32,
                )),
                type_: Checked::Valid(json::accessor::Type::Scalar),
                min: None,
                max: None,
                name: None,
                normalized: false,
                sparse: None,
                extensions: None,
                extras: Default::default(),
            });
            let mat_idx = submesh_material_indices
                .get(sm_idx)
                .copied()
                .unwrap_or(0);
            primitives.push(make_primitive(pos_acc, uv_acc, normal_acc, idx_acc, mat_idx));
        }
    }

    if primitives.is_empty() {
        return Err(format!("mesh {canonical_path} produced no primitives"));
    }

    let mesh_idx = meshes.len() as u32;
    meshes.push(json::Mesh {
        primitives,
        name: Some(canonical_path.to_string()),
        weights: None,
        extensions: None,
        extras: Default::default(),
    });
    Ok(mesh_idx)
}

fn make_primitive(
    pos_acc: u32,
    uv_acc: Option<u32>,
    normal_acc: Option<u32>,
    idx_acc: u32,
    mat_idx: u32,
) -> json::mesh::Primitive {
    use json::mesh::Semantic;
    let mut attributes = std::collections::BTreeMap::new();
    attributes.insert(
        Checked::Valid(Semantic::Positions),
        json::Index::new(pos_acc),
    );
    if let Some(acc) = uv_acc {
        attributes.insert(
            Checked::Valid(Semantic::TexCoords(0)),
            json::Index::new(acc),
        );
    }
    if let Some(acc) = normal_acc {
        attributes.insert(
            Checked::Valid(Semantic::Normals),
            json::Index::new(acc),
        );
    }
    json::mesh::Primitive {
        attributes,
        indices: Some(json::Index::new(idx_acc)),
        material: Some(json::Index::new(mat_idx)),
        mode: Checked::Valid(json::mesh::Mode::Triangles),
        targets: None,
        extensions: None,
        extras: Default::default(),
    }
}

fn position_min_max(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut mn = positions[0];
    let mut mx = positions[0];
    for p in &positions[1..] {
        for i in 0..3 {
            if p[i] < mn[i] {
                mn[i] = p[i];
            }
            if p[i] > mx[i] {
                mx[i] = p[i];
            }
        }
    }
    (mn, mx)
}

/// Build the root-level `KHR_lights_punctual` extension and append one
/// glTF node per light to the scene graph. Returns the
/// `extensions::root::Root` value to set on the glTF root, or `None`
/// when there are no lights.
fn build_lights_root(
    lights: &[LightInstance],
    nodes: &mut Vec<json::Node>,
    scene_nodes: &mut Vec<json::Index<json::Node>>,
) -> Option<json::extensions::root::Root> {
    if lights.is_empty() {
        return None;
    }
    use json::extensions::scene::khr_lights_punctual as klp;

    let mut gltf_lights = Vec::with_capacity(lights.len());
    for (i, light) in lights.iter().enumerate() {
        let (type_, spot) = match light.descriptor.kind {
            LightKind::Point => (Checked::Valid(klp::Type::Point), None),
            LightKind::Directional => (Checked::Valid(klp::Type::Directional), None),
            LightKind::Spot { inner_cone, outer_cone } => (
                Checked::Valid(klp::Type::Spot),
                Some(klp::Spot {
                    inner_cone_angle: inner_cone,
                    outer_cone_angle: outer_cone,
                }),
            ),
        };
        gltf_lights.push(klp::Light {
            color: light.descriptor.color,
            intensity: light.descriptor.intensity,
            name: Some(if light.descriptor.name.is_empty() {
                format!("light_{i}")
            } else {
                light.descriptor.name.clone()
            }),
            range: if light.descriptor.range > 0.0 {
                Some(light.descriptor.range)
            } else {
                None
            },
            type_,
            spot,
            extensions: None,
            extras: Default::default(),
        });

        // glTF nodes for lights carry the placement matrix and reference
        // the light by index through the node-level extension.
        let matrix = mat3x4_to_gltf_matrix(&light.world_transform);
        let identity = is_identity_matrix(&matrix);
        let node_idx = nodes.len() as u32;
        nodes.push(json::Node {
            name: Some(if light.descriptor.name.is_empty() {
                format!("light_{i}")
            } else {
                format!("light_{}", light.descriptor.name)
            }),
            matrix: if identity { None } else { Some(matrix) },
            extensions: Some(json::extensions::scene::Node {
                khr_lights_punctual: Some(klp::KhrLightsPunctual {
                    light: json::Index::new(i as u32),
                }),
            }),
            ..Default::default()
        });
        scene_nodes.push(json::Index::new(node_idx));
    }

    Some(json::extensions::root::Root {
        khr_lights_punctual: Some(json::extensions::root::KhrLightsPunctual {
            lights: gltf_lights,
        }),
    })
}

/// glTF 2.0 binary container layout (12-byte header + JSON chunk + BIN
/// chunk). Mirrors `crate::gltf::glb_builder::serialize_glb` but stays
/// independent so the SOC scene emitter does not pull in the
/// ship-pipeline builder state.
fn serialize_glb(root: &json::Root, bin: &[u8]) -> Result<Vec<u8>, String> {
    let json_value = serde_json::to_value(root).map_err(|e| format!("json::to_value: {e}"))?;
    let json_bytes = serde_json::to_vec(&json_value).map_err(|e| format!("json::to_vec: {e}"))?;

    let json_pad = (4 - json_bytes.len() % 4) % 4;
    let json_padded_len = json_bytes.len() + json_pad;
    let bin_pad = (4 - bin.len() % 4) % 4;
    let bin_padded_len = bin.len() + bin_pad;
    let total_len = 12 + 8 + json_padded_len + 8 + bin_padded_len;

    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());

    out.extend_from_slice(&(json_padded_len as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // 'JSON'
    out.extend_from_slice(&json_bytes);
    out.extend(std::iter::repeat_n(b' ', json_pad));

    out.extend_from_slice(&(bin_padded_len as u32).to_le_bytes());
    out.extend_from_slice(&0x004E4942u32.to_le_bytes()); // 'BIN\0'
    out.extend_from_slice(bin);
    out.extend(std::iter::repeat_n(0u8, bin_pad));

    Ok(out)
}

// Suppress an "unused" warning for the helper used only by the inner
// glTF builder when a future refactor adds a non-Mat4 path.
#[allow(dead_code)]
fn mat4_for_transform(m: &[[f32; 4]; 3]) -> Mat4 {
    let cols = [
        glam::Vec4::new(m[0][0], m[1][0], m[2][0], 0.0),
        glam::Vec4::new(m[0][1], m[1][1], m[2][1], 0.0),
        glam::Vec4::new(m[0][2], m[1][2], m[2][2], 0.0),
        glam::Vec4::new(m[0][3], m[1][3], m[2][3], 1.0),
    ];
    Mat4::from_cols(cols[0], cols[1], cols[2], cols[3])
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SubMesh;

    fn tri_mesh() -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            secondary_uvs: None,
            normals: Some(vec![[0.0, 0.0, 1.0]; 3]),
            tangents: None,
            colors: None,
            submeshes: vec![SubMesh {
                material_name: Some("test".into()),
                material_id: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            }],
            model_min: [0.0; 3],
            model_max: [1.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [1.0; 3],
        }
    }

    #[test]
    fn canonicalize_strips_data_prefix_and_normalises_slashes() {
        assert_eq!(
            canonicalize_mesh_path("Data/Objects/foo/bar.cgf"),
            "objects\\foo\\bar.cgf"
        );
        assert_eq!(
            canonicalize_mesh_path("DATA\\objects\\Foo.cgf"),
            "objects\\foo.cgf"
        );
        assert_eq!(
            canonicalize_mesh_path("objects/foo/bar.cgf"),
            "objects\\foo\\bar.cgf"
        );
    }

    #[test]
    fn p4k_path_for_adds_data_prefix_when_missing() {
        assert_eq!(
            p4k_path_for("objects/foo.cgf"),
            "Data\\objects\\foo.cgf"
        );
        assert_eq!(
            p4k_path_for("Data\\objects\\foo.cgf"),
            "Data\\objects\\foo.cgf"
        );
    }

    #[test]
    fn mtl_path_for_uses_mtl_name_when_qualified() {
        assert_eq!(
            mtl_path_for(
                "objects/material/foo",
                "Data\\objects\\meshes\\bar.cgf"
            ),
            "Data\\objects\\material\\foo.mtl"
        );
        assert_eq!(
            mtl_path_for("foo", "Data\\objects\\meshes\\bar.cgf"),
            "Data\\objects\\meshes\\foo.mtl"
        );
    }

    #[test]
    fn sibling_mtl_for_replaces_extension() {
        assert_eq!(
            sibling_mtl_for("Data\\objects\\foo.cgf"),
            "Data\\objects\\foo.mtl"
        );
        assert_eq!(
            sibling_mtl_for("Data\\objects\\foo"),
            "Data\\objects\\foo.mtl"
        );
    }

    #[test]
    fn emit_glb_roundtrips_minimal_scene() {
        let scene = RenderableScene {
            meshes: vec![ResolvedMesh {
                canonical_path: "test.cgf".to_string(),
                mesh: tri_mesh(),
                material: None,
            }],
            placements: vec![MeshPlacement {
                mesh_index: 0,
                world_transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                instance_id: 0,
            }],
            lights: Vec::new(),
            aabb: Some(([0.0; 3], [1.0; 3])),
            dropped_placements: 0,
            failed_mesh_paths: 0,
            materials_resolved: 0,
            materials_default: 1,
        };
        let (bytes, summary) = emit_glb(&scene).expect("emit ok");
        assert!(bytes.starts_with(b"glTF"), "should start with glTF magic");
        assert!(summary.mesh_count == 1);
        assert!(summary.placement_count == 1);
    }

    #[test]
    fn emit_glb_with_a_light_uses_extension() {
        let scene = RenderableScene {
            meshes: vec![ResolvedMesh {
                canonical_path: "test.cgf".to_string(),
                mesh: tri_mesh(),
                material: None,
            }],
            placements: vec![MeshPlacement {
                mesh_index: 0,
                world_transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                instance_id: 0,
            }],
            lights: vec![LightInstance {
                world_transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                descriptor: LightDescriptor {
                    name: "test_light".into(),
                    kind: LightKind::Point,
                    color: [1.0, 1.0, 1.0],
                    intensity: 100.0,
                    range: 5.0,
                },
            }],
            aabb: Some(([0.0; 3], [1.0; 3])),
            dropped_placements: 0,
            failed_mesh_paths: 0,
            materials_resolved: 0,
            materials_default: 1,
        };
        let (bytes, summary) = emit_glb(&scene).expect("emit ok");
        assert!(summary.light_count == 1);
        // Find the JSON header magic and verify it mentions the extension.
        let json_chunk_start = 12 + 8;
        let json_len_bytes: [u8; 4] = bytes[12..16].try_into().unwrap();
        let json_len = u32::from_le_bytes(json_len_bytes) as usize;
        let json_text = std::str::from_utf8(&bytes[json_chunk_start..json_chunk_start + json_len])
            .unwrap();
        assert!(
            json_text.contains("KHR_lights_punctual"),
            "glTF JSON should declare the lights extension"
        );
    }

    #[test]
    fn canonicalize_collapses_case_and_separators() {
        // Two zones referring to the same mesh under different casing
        // must canonicalise to the same key — that is the dedupe-pass
        // contract.
        let canonical_a = canonicalize_mesh_path("objects/foo.cgf");
        let canonical_b = canonicalize_mesh_path("OBJECTS\\Foo.cgf");
        assert_eq!(canonical_a, canonical_b);
    }

    fn light_with_intensity(intensity: f32) -> LightInstance {
        LightInstance {
            world_transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            descriptor: LightDescriptor {
                name: format!("light_{intensity}"),
                kind: LightKind::Point,
                color: [1.0, 1.0, 1.0],
                intensity,
                range: 0.0,
            },
        }
    }

    #[test]
    fn pick_top_lights_keeps_highest_intensity() {
        let lights = vec![
            light_with_intensity(1.0),
            light_with_intensity(10.0),
            light_with_intensity(5.0),
            light_with_intensity(20.0),
        ];
        let kept = pick_top_lights(&lights, 2);
        assert_eq!(kept.len(), 2);
        let intensities: Vec<f32> = kept.iter().map(|l| l.descriptor.intensity).collect();
        // Highest two are 20.0 and 10.0, regardless of return order.
        assert!(intensities.contains(&20.0));
        assert!(intensities.contains(&10.0));
        assert!(!intensities.contains(&5.0));
        assert!(!intensities.contains(&1.0));
    }

    #[test]
    fn pick_top_lights_handles_max_zero() {
        let lights = vec![light_with_intensity(1.0)];
        assert!(pick_top_lights(&lights, 0).is_empty());
    }

    #[test]
    fn pick_top_lights_under_cap_returns_input() {
        let lights = vec![
            light_with_intensity(1.0),
            light_with_intensity(2.0),
        ];
        let kept = pick_top_lights(&lights, 64);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn emit_glb_caps_lights_and_reports_dropped_count() {
        // Build 100 lights with descending intensity. Cap at 16. Verify
        // the emitter keeps the top 16 by intensity and reports 84 as
        // dropped.
        let mut lights = Vec::with_capacity(100);
        for i in 0..100 {
            lights.push(light_with_intensity(100.0 - i as f32));
        }
        let scene = RenderableScene {
            meshes: vec![ResolvedMesh {
                canonical_path: "test.cgf".to_string(),
                mesh: tri_mesh(),
                material: None,
            }],
            placements: vec![MeshPlacement {
                mesh_index: 0,
                world_transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                instance_id: 0,
            }],
            lights,
            aabb: Some(([0.0; 3], [1.0; 3])),
            dropped_placements: 0,
            failed_mesh_paths: 0,
            materials_resolved: 0,
            materials_default: 1,
        };
        let opts = EmitOptions { max_lights: 16 };
        let (_bytes, summary) =
            emit_glb_with_options(&scene, opts).expect("emit ok");
        assert_eq!(summary.light_count, 16);
        assert_eq!(summary.lights_dropped, 84);
    }

    #[test]
    fn emit_glb_does_not_embed_placeholder_textures() {
        // The minimal-scene roundtrip uses material=None, so no MTL is
        // resolved. With the placeholder PNG emission removed, even
        // textured-material scenes should report zero textures and zero
        // images; that is verified end-to-end by the Exec Hangar render
        // smoke test. Here we just confirm the no-material baseline
        // emits no textures and no baseColorTexture binding.
        let scene = RenderableScene {
            meshes: vec![ResolvedMesh {
                canonical_path: "test.cgf".to_string(),
                mesh: tri_mesh(),
                material: None,
            }],
            placements: vec![MeshPlacement {
                mesh_index: 0,
                world_transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                instance_id: 0,
            }],
            lights: Vec::new(),
            aabb: Some(([0.0; 3], [1.0; 3])),
            dropped_placements: 0,
            failed_mesh_paths: 0,
            materials_resolved: 0,
            materials_default: 1,
        };
        let (bytes, summary) = emit_glb(&scene).expect("emit ok");
        assert_eq!(summary.texture_count, 0);
        let json_chunk_start = 12 + 8;
        let json_len_bytes: [u8; 4] = bytes[12..16].try_into().unwrap();
        let json_len = u32::from_le_bytes(json_len_bytes) as usize;
        let json_text =
            std::str::from_utf8(&bytes[json_chunk_start..json_chunk_start + json_len]).unwrap();
        assert!(
            !json_text.contains("baseColorTexture"),
            "no material should bind a baseColorTexture (no placeholder)"
        );
    }
}
