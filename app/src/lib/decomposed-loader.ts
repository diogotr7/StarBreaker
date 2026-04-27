// Pure conversion logic for decomposed-export records into Three.js objects.
//
// This file knows the shape of the `scene.json` / material-sidecar / light
// records emitted by `crates/starbreaker-3d/src/decomposed.rs`. It owns
// matrix conversion, light type mapping, and a generic PBR material builder.
// Keeping this logic out of `scene-viewer.tsx` lets the viewer stay thin.
//
// All paths in the contract are relative to the *export root* (the parent
// of `Packages/`), use forward slashes, and start with `Data/...` for
// shared assets or `Packages/...` for per-package files.

import * as THREE from "three";

// ────────────────────────────────────────────────────────────
// Coordinate-system conversion
// ────────────────────────────────────────────────────────────
//
// Star Citizen is right-handed Z-up; Three.js is right-handed Y-up. The
// exporter writes `local_transform_sc`, bone matrices, and interior
// container/placement transforms in CryEngine basis (Z-up). Every GLB
// emitted by `crates/starbreaker-3d/src/gltf` ALSO wraps its content in
// a top-level `CryEngine_Z_up` node that performs the Z-up -> Y-up
// rotation, so a single GLB opens correctly in any glTF viewer (Blender,
// gltf-viewer, etc.).
//
// In the scene viewer we compose many GLBs into one tree, with
// CryEngine-basis transforms between them (`local_transform_sc`,
// `container_transform`, `placement.transform`). If every GLB does its
// own basis flip at its root, those Z-up inter-mesh transforms get
// applied to already-Y-up subtrees and produce visible artifacts: ground
// vehicles render nose-down, interior containers fly off-axis. To get a
// consistent assembled scene we strip each loaded GLB's CryEngine_Z_up
// wrapper at load time (see `stripCryEngineZUp`) so all geometry stays
// in raw Z-up, then apply the basis change exactly once at the scene
// root via `applySCBasisToObject`.

const SC_TO_THREE = new THREE.Matrix4().makeRotationX(-Math.PI / 2);

/** Wrap a SC-basis Object3D in a Y-up basis change so it renders correctly. */
export function applySCBasisToObject(obj: THREE.Object3D): void {
  obj.matrixAutoUpdate = false;
  obj.matrix.premultiply(SC_TO_THREE);
}

/**
 * Remove the GLB-level `CryEngine_Z_up` wrapper from a loaded glTF
 * scene, leaving its inner content reparented directly to `scene`. This
 * normalises every loaded mesh into raw CryEngine (Z-up) basis so that
 * the inter-mesh transforms recorded in `scene.json` (which are also in
 * CryEngine basis) compose cleanly. The scene viewer applies the single
 * Z-up -> Y-up rotation at the scene root.
 *
 * Idempotent: returns the input unchanged if no `CryEngine_Z_up` node is
 * present (e.g. if a future exporter version drops the wrapper).
 */
export function stripCryEngineZUp(scene: THREE.Object3D): THREE.Object3D {
  // Walk the scene's direct children looking for the wrapper. The
  // exporter places it as the sole top-level node in the gltf scene
  // graph (`glb_builder.rs::finalize`), but defensively scan in case the
  // structure ever gains siblings. Snapshot the children arrays before
  // iterating: `scene.remove` / `scene.add` mutate them in place.
  // oxlint-disable-next-line unicorn/no-useless-spread
  const topLevel = [...scene.children];
  for (const child of topLevel) {
    if (child.name === "CryEngine_Z_up") {
      // Reparent the wrapper's children to its parent. We discard the
      // wrapper's matrix because its sole purpose was to rotate Z-up to
      // Y-up, which we no longer want at this layer.
      // oxlint-disable-next-line unicorn/no-useless-spread
      const inner = [...child.children];
      scene.remove(child);
      for (const node of inner) {
        scene.add(node);
      }
    }
  }
  return scene;
}

/**
 * Convert a 4x4 row-major matrix from `local_transform_sc` (or any
 * exporter-emitted matrix field) into a `THREE.Matrix4`. The exporter
 * emits row-major data, while `Matrix4.set` also takes row-major args,
 * so the transcription is direct.
 */
export function matrixFromRows(rows: number[][]): THREE.Matrix4 {
  // rows is a 4x4 nested array. Defensive: tolerate fewer rows by
  // padding with identity rows. Three.js Matrix4.set takes the elements
  // in row-major order.
  const m = new THREE.Matrix4();
  if (
    !Array.isArray(rows) ||
    rows.length < 4 ||
    rows.some((r) => !Array.isArray(r) || r.length < 4)
  ) {
    return m; // identity
  }
  m.set(
    rows[0][0], rows[0][1], rows[0][2], rows[0][3],
    rows[1][0], rows[1][1], rows[1][2], rows[1][3],
    rows[2][0], rows[2][1], rows[2][2], rows[2][3],
    rows[3][0], rows[3][1], rows[3][2], rows[3][3],
  );
  return m;
}

/**
 * Build a fallback transform from the legacy `offset_position` /
 * `offset_rotation` (Euler degrees) fields when `local_transform_sc` is
 * absent. The exporter prefers the resolved matrix path but old sidecars
 * may not have it.
 */
export function matrixFromOffsetEuler(
  offsetPosition: number[] | undefined,
  offsetRotationDeg: number[] | undefined,
): THREE.Matrix4 {
  const pos = new THREE.Vector3(
    offsetPosition?.[0] ?? 0,
    offsetPosition?.[1] ?? 0,
    offsetPosition?.[2] ?? 0,
  );
  const eulerDeg = offsetRotationDeg ?? [0, 0, 0];
  const euler = new THREE.Euler(
    THREE.MathUtils.degToRad(eulerDeg[0] ?? 0),
    THREE.MathUtils.degToRad(eulerDeg[1] ?? 0),
    THREE.MathUtils.degToRad(eulerDeg[2] ?? 0),
    "XYZ",
  );
  const quat = new THREE.Quaternion().setFromEuler(euler);
  return new THREE.Matrix4().compose(pos, quat, new THREE.Vector3(1, 1, 1));
}

// ────────────────────────────────────────────────────────────
// Manifest types — minimal surface, just what we read.
// ────────────────────────────────────────────────────────────

export interface SceneManifest {
  version?: number;
  /** Decomposed-export contract revision the writer used. Bumped in
   *  lockstep with `starbreaker_3d::DECOMPOSED_CONTRACT_VERSION` whenever
   *  on-disk semantics change (sidecar layout, GLB extras, palette
   *  contract). The loader compares this against its own
   *  `DECOMPOSED_CONTRACT_VERSION` and refuses to load mismatched packages
   *  rather than silently rendering against stale data. Absent on
   *  pre-contract-v2 exports — those are treated as v0 (mismatch). */
  contract_version?: number;
  package_rule?: {
    package_dir?: string;
    shared_asset_root?: string;
  };
  root_entity?: {
    entity_name?: string;
    /** Always 0 for the root entity; present in exports >= the
     *  instance-id contract addition. Children reference this via
     *  `parent_instance_id`. */
    instance_id?: number;
    geometry_path?: string;
    mesh_asset?: string;
    material_sidecar?: string | null;
    palette_id?: string | null;
  };
  children?: SceneInstance[];
  interiors?: InteriorContainer[];
  export_options?: Record<string, unknown>;
}

export interface SceneInstance {
  entity_name?: string;
  /** Unique per-export id for this child placement. Disambiguates
   *  sibling placements that share the same `entity_name` (e.g. paired
   *  weapon mounts on a Nox). Loaders should prefer this over
   *  `entity_name` when keying a parent-attach registry. Optional for
   *  backward compatibility with older exports; absent fields fall back
   *  to `entity_name`-based matching with a console warning. */
  instance_id?: number;
  /** Instance id of the parent placement (root or another child). When
   *  present, takes precedence over `parent_entity_name` for resolving
   *  the attach target. */
  parent_instance_id?: number;
  geometry_path?: string;
  mesh_asset?: string;
  material_sidecar?: string | null;
  palette_id?: string | null;
  parent_node_name?: string | null;
  parent_entity_name?: string | null;
  source_transform_basis?: string | null;
  local_transform_sc?: number[][] | null;
  resolved_no_rotation?: boolean;
  no_rotation?: boolean;
  offset_position?: number[];
  offset_rotation?: number[];
}

export interface InteriorContainer {
  name?: string;
  palette_id?: string | null;
  container_transform?: number[][];
  placements?: InteriorPlacement[];
  lights?: LightRecord[];
}

export interface InteriorPlacement {
  cgf_path?: string;
  mesh_asset?: string;
  material_sidecar?: string | null;
  entity_class_guid?: string | null;
  transform?: number[][];
  palette_id?: string | null;
}

/** Known CryEngine light kinds emitted by the exporter. The wider `string`
 *  fallback keeps the field tolerant of new types (e.g. future area-light
 *  variants) without breaking the build. Match the runtime switch in
 *  `buildLight` against the lower-cased value before relying on the literal. */
export type LightTypeName =
  | "Omni"
  | "SoftOmni"
  | "Projector"
  | "Planar"
  | "Ambient";

export interface LightRecord {
  name?: string;
  position?: number[];
  rotation?: number[];
  direction_sc?: number[] | null;
  color?: number[]; // linear RGB
  light_type?: LightTypeName | string;
  semantic_light_kind?: string | null;
  intensity?: number;
  intensity_raw?: number;
  intensity_candela_proxy?: number | null;
  radius?: number;
  radius_m?: number | null;
  inner_angle?: number | null;
  outer_angle?: number | null;
  projector_texture?: string | null;
  active_state?: string | null;
  states?: Record<string, LightStateRecord>;
}

export interface LightStateRecord {
  intensity_raw?: number;
  intensity_cd?: number | null;
  intensity_candela_proxy?: number | null;
  temperature?: number | null;
  use_temperature?: boolean;
  color?: number[];
}

// Material sidecar — minimal subset. Captures enough to build a
// PBR material from one of the submaterials.
export interface MaterialSidecar {
  version?: number;
  source_material_path?: string;
  submaterials?: SubmaterialRecord[];
}

/** A single paint / livery variant available for the loaded ship. The
 *  exporter writes one of these per `<SubGeometry>` paint slot it found
 *  on the entity (Beta / Delta / Murray Cup / IAE2951 / etc.). The
 *  variant either ships a substitute exterior `*.materials.json`
 *  sidecar or shares the default sidecar and only swaps the palette.
 *  See `liveries.json` / `paints.json` in `_*_export/Packages/<name>/`. */
export interface PaintVariant {
  /** Palette identifier (e.g. `palette/cnou_mustang_alpha`). Stable
   *  per-export and used as the dropdown's selection key. The default
   *  livery is selected when no variant is active. */
  palette_id: string;
  /** Authored CryEngine `<SubGeometry>` tag the variant maps to (e.g.
   *  `Beta`, `Paint_Mustang_IAE2951_Grey_White`). Used as the dropdown
   *  label fallback when `display_name` is null. */
  subgeometry_tag?: string | null;
  /** Human-readable label from DataCore (`PaintVariantParams.displayName`).
   *  Often null for default-baked variants like Beta/Delta which only
   *  exist as authoring tags. */
  display_name?: string | null;
  /** Substitute `*.materials.json` sidecar to use for the entity's
   *  exterior submaterials when this variant is active. Null when the
   *  variant only swaps the palette and reuses the default sidecar's
   *  textures. Path is relative to the export root. */
  exterior_material_sidecar?: string | null;
}

/** `paints.json` schema. Lists every paint variant the exporter
 *  resolved off the entity's `Components[SCItemPaintParams]` table.
 *  Variant set is per-ship — populates the livery dropdown without
 *  any per-ship hardcoding. */
export interface PaintsManifest {
  version?: number;
  paint_variants?: PaintVariant[];
}

/** Known shader_family values emitted by the exporter. The wider `string`
 *  fallback keeps the union forward-compatible: new families added on the
 *  Rust side won't fail the type check, but they fall through to the
 *  generic HardSurface / Illum branch in `buildMaterial`. The literal
 *  members exist to catch typos in switch statements at compile time. */
export type ShaderFamilyName =
  | "DisplayScreen"
  | "GlassPBR"
  | "HardSurface"
  | "Hologram"
  | "HologramCIG"
  | "Illum"
  | "Layer"
  | "LayerBlend_V2"
  | "MeshDecal"
  | "Monitor"
  | "Shield_Holo"
  | "UIMesh"
  | "UIPlane";

export interface SubmaterialRecord {
  index?: number;
  /** Authored material name (e.g. "Damage_Internal_Tile_Alpha"). */
  submaterial_name?: string;
  /**
   * Stable Blender slot name (e.g. "rsi_polaris_ext:Damage_Internal_Tile_Alpha").
   * The Rust GLB writer uses this same string as the glTF material name,
   * so it is the most reliable key for matching exporter materials to
   * sidecar entries.
   */
  blender_material_name?: string;
  /** Raw shader string (e.g. "Illum", "HardSurface"). */
  shader?: string | null;
  /** Normalized shader family classification, when known. */
  shader_family?: ShaderFamilyName | string | null;
  /** Decoded CryEngine StringGenMask flags. The Rust exporter parses tokens
   *  like `DECAL`, `STENCIL_MAP`, `VERTCOLORS`, `PARALLAX_OCCLUSION_MAPPING`
   *  out of the `%TOKEN1%TOKEN2` mask and surfaces them here. The decal
   *  pipeline keys off `has_decal` / `has_stencil_map` rather than
   *  `shader_family` because Illum, HardSurface, and Layer materials can
   *  all be decals depending on their authored mask. */
  decoded_feature_flags?: DecodedFeatureFlags | null;
  texture_slots?: TextureSlotRecord[];
  direct_textures?: DirectTextureRecord[];
  /** Per-submaterial palette tint baked by the exporter (HardSurface only).
   *  Null for non-tinted submaterials. When `assigned_channel` resolves to
   *  one of `entries`, the matching `tint_color` is multiplied with the
   *  diffuse map (`final_albedo = tint_color × diffuse_sample`). */
  tint_palette?: SubmaterialPalette | null;
  /** Palette-channel routing for layered HardSurface submaterials.
   *  `material_channel` is the channel this submaterial as a whole reads
   *  from; `layer_channels` enumerates per-layer channel assignments and
   *  is used to pick the layer that owns the visible diffuse/normal pair. */
  palette_routing?: PaletteRouting | null;
  /** Resolved per-layer texture pointers for layered HardSurface
   *  submaterials. The visible "paint" tiling diffuse / normal live here
   *  rather than in the top-level `texture_slots`, because the authored
   *  layer chain (Primary, Wear, etc.) is preserved for future blending
   *  work. The frontend currently picks the first layer matching the
   *  submaterial's `material_channel`. */
  layer_manifest?: LayerManifestEntry[];
  /** Loader-derived activation state. "active" means the material is
   *  renderable; "inactive" means the mesh should be hidden (e.g. a
   *  stencil-float decal whose base colour texture is absent, or a
   *  nodraw slot). Set by the Rust exporter in `material.extras.semantic`. */
  activation_state?: { state: "active" | "inactive"; reason?: string } | null;
  // Many other fields exist (public_params, paint_override,
  // material_set_identity, etc.). Frontend ignores them for the POC.
}

export interface DecodedFeatureFlags {
  has_decal?: boolean;
  has_iridescence?: boolean;
  has_parallax_occlusion_mapping?: boolean;
  has_stencil_map?: boolean;
  has_vertex_colors?: boolean;
  tokens?: string[];
}

export interface TintPaletteEntry {
  channel: string;
  /** Linear-space RGB tint (sRGB→linear conversion already applied on the
   *  Rust side). Feed directly into `THREE.Color` without further conversion. */
  tint_color: [number, number, number];
  spec_color: [number, number, number] | null;
  glossiness: number | null;
}

export interface SubmaterialPalette {
  palette_id: string;
  palette_source_name: string | null;
  /** Which channel of `entries` this submaterial actually reads. Null when
   *  the routing could not be resolved on the Rust side; the frontend
   *  treats null as "leave material.color at the family default". */
  assigned_channel: string | null;
  entries: TintPaletteEntry[];
}

export interface PaletteChannel {
  index: number;
  name: string;
}

export interface PaletteRouting {
  material_channel: PaletteChannel | null;
  layer_channels: { index: number; channel: PaletteChannel | null }[];
}

export interface LayerManifestEntry {
  index?: number;
  name?: string;
  diffuse_export_path?: string | null;
  normal_export_path?: string | null;
  palette_channel?: PaletteChannel | null;
}

export interface TextureTransformRecord {
  /** [tileU, tileV] from CryEngine TexMod. Maps to THREE.Texture.repeat. */
  scale?: number[];
  /** [offsetU, offsetV] from CryEngine TexMod. Maps to THREE.Texture.offset. */
  offset?: number[];
  /** Rotation in radians (currently unused; CryEngine emits TexMod_RotateType
   *  which we treat as advisory until a concrete mapping is needed). */
  rotation?: number | null;
  attributes?: Record<string, unknown>;
}

export interface TextureSlotRecord {
  role?: string;
  /** TexSlotN identifier (e.g. "TexSlot1", "TexSlot7"). Used to spot
   *  decal stencils when the role enum is `unknown`. */
  slot?: string;
  source_path?: string;
  export_path?: string;
  alpha_semantic?: string | null;
  texture_transform?: TextureTransformRecord | null;
}

export interface DirectTextureRecord {
  role?: string;
  slot?: string;
  source_path?: string;
  export_path?: string;
  alpha_semantic?: string | null;
  texture_transform?: TextureTransformRecord | null;
}

// ────────────────────────────────────────────────────────────
// Light conversion
// ────────────────────────────────────────────────────────────

/**
 * Per-type intensity scalars for converting CryEngine candela values to
 * Three.js-friendly intensities. CryEngine candela values are emitted in
 * physical units (cd) while Three.js non-physical lights treat intensity
 * as a unitless multiplier; the mapping is purely visual. Tuned by eye on
 * Mustang / Nox / Polaris fixtures: omnis dominate cabin lighting and
 * shouldn't blow out, projectors are cone-narrow and need more cd to
 * register, ambient is global fill so it scales lowest.
 */
const LIGHT_INTENSITY_SCALARS: Record<string, number> = {
  omni: 0.008,
  softomni: 0.008,
  point: 0.008,
  projector: 0.02,
  spot: 0.02,
  ambient: 0.05,
  ambient_proxy: 0.05,
};
/** Scalar applied when we fall back to `intensity_raw` (unitless gameplay
 *  value, much hotter than candela). Roughly the ratio observed on
 *  contract samples between intensity_raw and intensity_candela_proxy. */
const INTENSITY_RAW_SCALAR = 0.001;

/**
 * Convert one exporter `LightRecord` into a Three.js light. Returns null
 * when the type is unrecognized (we log so operators can spot it).
 *
 * Intensity is normalized to a Three.js-friendly range using per-type
 * heuristic scalars. Different light types fill different roles in a
 * scene, so a single blanket scalar (the previous 0.01) over-lit cabin
 * point lights and under-lit spotlights. See `LIGHT_INTENSITY_SCALARS`.
 */
export function buildLight(record: LightRecord): THREE.Light | null {
  const color = colorFromLinearRGB(record.color, record.states, record.active_state);
  const kind = (record.semantic_light_kind ?? record.light_type ?? "").toLowerCase();

  // Pick base candela. If `intensity_candela_proxy` is missing or zero,
  // fall back to the raw CryEngine intensity scaled down to the same
  // order of magnitude.
  const candela = pickIntensity(record);
  let baseIntensity = candela;
  if (!Number.isFinite(candela) || candela <= 0) {
    const raw = typeof record.intensity_raw === "number" ? record.intensity_raw : 0;
    baseIntensity = Math.max(0, raw) * INTENSITY_RAW_SCALAR / scalarForKind(kind);
  }
  const scalar = scalarForKind(kind);
  const intensity = Math.max(0, baseIntensity * scalar);

  const radius = record.radius_m ?? record.radius ?? 10.0;
  const distance = Math.max(radius, 0.01);

  let light: THREE.Light | null = null;

  switch (kind) {
    case "ambient":
    case "ambient_proxy":
      light = new THREE.AmbientLight(color, intensity);
      break;
    case "spot":
    case "projector": {
      const outer = degToRadOrDefault(record.outer_angle, 30);
      // If inner_angle is missing on a Projector, default to half the
      // outer angle (matches CryEngine's "soft cone" convention for
      // projector lights when no explicit inner angle is authored).
      const innerDeg =
        typeof record.inner_angle === "number"
          ? record.inner_angle
          : (typeof record.outer_angle === "number" ? record.outer_angle * 0.5 : 15);
      const inner = THREE.MathUtils.degToRad(innerDeg);
      const spot = new THREE.SpotLight(color, intensity, distance, outer);
      spot.penumbra = Math.max(0, 1 - inner / outer);
      light = spot;
      break;
    }
    case "point":
    case "omni":
    case "softomni":
    default:
      light = new THREE.PointLight(color, intensity, distance, 2);
      break;
  }

  if (record.position && light) {
    light.position.set(
      record.position[0] ?? 0,
      record.position[1] ?? 0,
      record.position[2] ?? 0,
    );
  }
  if (light) {
    light.name = record.name ?? "light";
    // Spotlights default-orient down -Z in Three.js. The exporter's
    // `direction_sc` field, when present, is the forward direction in SC
    // basis. We use a target object set to position + direction.
    if (light instanceof THREE.SpotLight && record.direction_sc) {
      const dir = new THREE.Vector3(
        record.direction_sc[0] ?? 0,
        record.direction_sc[1] ?? 0,
        record.direction_sc[2] ?? -1,
      ).normalize();
      const target = new THREE.Object3D();
      target.position.copy(light.position).add(dir);
      light.target = target;
    }
  }
  return light;
}

function scalarForKind(kind: string): number {
  return LIGHT_INTENSITY_SCALARS[kind] ?? 0.01;
}

function pickIntensity(record: LightRecord): number {
  // Priority: explicit intensity_candela_proxy on the record, then
  // intensity, then walk states by priority order.
  if (typeof record.intensity_candela_proxy === "number") {
    return record.intensity_candela_proxy;
  }
  if (typeof record.intensity === "number") return record.intensity;
  if (typeof record.intensity_raw === "number") return record.intensity_raw;
  const states = record.states ?? {};
  for (const key of [
    record.active_state ?? "",
    "defaultState",
    "auxiliaryState",
    "emergencyState",
    "cinematicState",
  ]) {
    const s = states[key];
    if (s && typeof s.intensity_candela_proxy === "number") {
      return s.intensity_candela_proxy;
    }
    if (s && typeof s.intensity_cd === "number") return s.intensity_cd;
    if (s && typeof s.intensity_raw === "number") return s.intensity_raw;
  }
  return 1.0;
}

function colorFromLinearRGB(
  rgb: number[] | undefined,
  states: Record<string, LightStateRecord> | undefined,
  active: string | null | undefined,
): THREE.Color {
  let arr = rgb;
  if (!arr && states) {
    for (const key of [
      active ?? "",
      "defaultState",
      "auxiliaryState",
      "emergencyState",
      "cinematicState",
    ]) {
      const s = states[key];
      if (s?.color && s.color.length >= 3) {
        arr = s.color;
        break;
      }
    }
  }
  return new THREE.Color(
    Math.max(0, arr?.[0] ?? 1),
    Math.max(0, arr?.[1] ?? 1),
    Math.max(0, arr?.[2] ?? 1),
  );
}

function degToRadOrDefault(value: number | null | undefined, fallbackDeg: number): number {
  const deg = typeof value === "number" ? value : fallbackDeg;
  return THREE.MathUtils.degToRad(deg);
}

// ────────────────────────────────────────────────────────────
// Material building
// ────────────────────────────────────────────────────────────

/** User-selectable presentation modes. `Textured` is the default and the
 *  only one that consults sidecar textures; the others are diagnostic /
 *  artistic overlays that ignore textures so they work uniformly across
 *  all entities even when texture loads are incomplete. */
export type RenderStyle =
  | "textured"
  | "opaque"
  | "metallic"
  | "glass"
  | "holographic";

export const RENDER_STYLES: { value: RenderStyle; label: string }[] = [
  { value: "textured", label: "Textured" },
  { value: "opaque", label: "Opaque Lit" },
  { value: "metallic", label: "Metallic" },
  { value: "glass", label: "Glass" },
  { value: "holographic", label: "Holographic" },
];

/** Stable cache key for a (submaterial, style) pair. Two instances of the
 *  same submaterial at the same style share one Three.js Material, which
 *  collapses draw-call overhead and shader-program count. */
export function materialCacheKey(
  submaterial: SubmaterialRecord,
  style: RenderStyle,
): string {
  const name =
    submaterial.blender_material_name ??
    submaterial.submaterial_name ??
    `idx:${submaterial.index ?? "?"}`;
  // Debug-fallback mode is NOT in the cache key — toggling DBG mutates
  // existing materials in place via `applyDebugFallbackToMaterial`,
  // avoiding the rebuild + shader-recompile storm a fresh-material
  // path triggers on big ships (Aurora has ~1000 unique submats; one
  // shader compile per material × 1000 = multi-second main-thread
  // freeze). Color swaps don't recompile shaders.
  return `${style}|${name}`;
}

/** Diagnostic colours for fallback / stand-in material paths. When
 *  `debugFallbackMode` is on, each builder swaps its base colour for the
 *  neon corresponding to the path it took, so the user can visually
 *  identify which fallback is firing on each surface.
 *
 *  Two design rules:
 *    1. Only paths that are *guesses* or *stand-ins* get coloured.
 *       A correctly-resolved palette tint or a properly-authored
 *       decal/glass/skin material renders normally — neoning it would
 *       create false positives.
 *    2. The neon multiplies with whatever diffuse texture the slot
 *       resolution produces, so a green-coloured "no palette but has
 *       diffuse" panel still shows the texture detail (just tinted
 *       green). Pure neon = no diffuse loaded either. */
export const FALLBACK_KINDS = {
  HARDSURFACE_PALETTE_RESOLVED: "hardsurface_palette_resolved",
  HARDSURFACE_PALETTE_HEURISTIC: "hardsurface_palette_heuristic",
  HARDSURFACE_NO_PALETTE: "hardsurface_no_palette",
  FAMILY_UNKNOWN_DEFAULT: "family_unknown_default",
  HOLOGRAM_STUB: "hologram_stub",
  SCREEN_STUB: "screen_stub",
  SKIN_DEFAULT: "skin_default",
} as const;

export type FallbackKind =
  (typeof FALLBACK_KINDS)[keyof typeof FALLBACK_KINDS];

const FALLBACK_NEONS: Partial<Record<FallbackKind, number>> = {
  hardsurface_palette_resolved: 0x0080ff, // bright blue: routed-channel palette resolved correctly
  hardsurface_palette_heuristic: 0x00ffff, // cyan: heuristic entryA fallback fired
  hardsurface_no_palette: 0x00ff00, // green: no tint_palette at all (multiplied with diffuse)
  family_unknown_default: 0xff0000, // red: shader_family didn't match any builder
  hologram_stub: 0xffff00, // yellow: Hologram emissive-stub stand-in
  screen_stub: 0xff00aa, // magenta-violet: DisplayScreen / RTT stub
  skin_default: 0xff80c0, // pink: HumanSkin stand-in (no proper SSS)
};

let debugFallbackMode = false;

export function setDebugFallbackMode(on: boolean): void {
  debugFallbackMode = on;
}

export function isDebugFallbackMode(): boolean {
  return debugFallbackMode;
}

/** Resolve the neon colour for a given fallback kind, or null if the
 *  kind has no entry in `FALLBACK_NEONS`. Does NOT consult
 *  `debugFallbackMode` — callers are responsible for honouring it.
 *  (Build-time callers want the neon eagerly when debug mode is on at
 *  build time; the in-place toggle path consults this helper directly
 *  and mutates `material.color` based on it.) */
function debugColorFor(kind: FallbackKind): THREE.Color | null {
  const hex = FALLBACK_NEONS[kind];
  return hex == null ? null : new THREE.Color(hex);
}

/** Stash a material's original (non-debug) colour state so the DBG
 *  toggle can restore it without a rebuild. Captures `.color` plus
 *  `.emissive` / `.emissiveIntensity` for PBR materials (so the
 *  emissive override the toggle layers in can be reverted). For the
 *  Hologram ShaderMaterial, captures the `baseColor` uniform Vector3.
 *  Idempotent — safe to call multiple times. */
function tagOriginalColor(material: THREE.Material): void {
  if (material.userData.originalColor) return;
  if (material.userData.originalBaseColor) return;
  if ("color" in material && material.color instanceof THREE.Color) {
    material.userData.originalColor = (material.color as THREE.Color).clone();
    if ("emissive" in material && material.emissive instanceof THREE.Color) {
      material.userData.originalEmissive = (
        material.emissive as THREE.Color
      ).clone();
      material.userData.originalEmissiveIntensity = (
        material as THREE.MeshStandardMaterial
      ).emissiveIntensity;
    }
    return;
  }
  if (material instanceof THREE.ShaderMaterial) {
    const u = material.uniforms?.baseColor?.value;
    if (u instanceof THREE.Vector3) {
      material.userData.originalBaseColor = u.clone();
    }
  }
}

/** Swap a material's colour to/from its debug neon based on the global
 *  flag and the material's `userData.fallbackKind`. Mutates in place
 *  — no new material, no shader recompile.
 *
 *  PBR materials get BOTH `.color` AND `.emissive` set to the neon, so
 *  the override is visible even when bright IBL + clearcoat reflection
 *  would otherwise wash out a pure base-colour swap. Initial attempt
 *  set only `.color`, which made exterior hull panels (heavy IBL +
 *  clearcoat = 1.0) appear unchanged — the reflective specular layer
 *  was hiding the colour shift. Emissive is unaffected by lighting,
 *  so it self-illuminates the surface in the diagnostic colour. */
export function applyDebugFallbackToMaterial(material: THREE.Material): void {
  const kind = material.userData.fallbackKind as FallbackKind | undefined;
  if (!kind) return;
  const neon = debugColorFor(kind);
  if (debugFallbackMode && neon) {
    if ("color" in material && material.color instanceof THREE.Color) {
      (material.color as THREE.Color).copy(neon);
      // Also light up via emissive so the neon survives clearcoat /
      // IBL reflections on PBR materials.
      if ("emissive" in material && material.emissive instanceof THREE.Color) {
        (material.emissive as THREE.Color).copy(neon);
        (material as THREE.MeshStandardMaterial).emissiveIntensity = 1.0;
      }
    } else if (
      material instanceof THREE.ShaderMaterial &&
      material.uniforms?.baseColor?.value instanceof THREE.Vector3
    ) {
      (material.uniforms.baseColor.value as THREE.Vector3).set(
        neon.r,
        neon.g,
        neon.b,
      );
    }
  } else {
    // Restore original. tagOriginalColor must have run at build time.
    if (
      "color" in material &&
      material.color instanceof THREE.Color &&
      material.userData.originalColor instanceof THREE.Color
    ) {
      (material.color as THREE.Color).copy(material.userData.originalColor);
      if (
        "emissive" in material &&
        material.emissive instanceof THREE.Color &&
        material.userData.originalEmissive instanceof THREE.Color
      ) {
        (material.emissive as THREE.Color).copy(
          material.userData.originalEmissive,
        );
        (material as THREE.MeshStandardMaterial).emissiveIntensity =
          (material.userData.originalEmissiveIntensity as number | undefined) ??
          1.0;
      }
    } else if (
      material instanceof THREE.ShaderMaterial &&
      material.uniforms?.baseColor?.value instanceof THREE.Vector3 &&
      material.userData.originalBaseColor instanceof THREE.Vector3
    ) {
      (material.uniforms.baseColor.value as THREE.Vector3).copy(
        material.userData.originalBaseColor,
      );
    }
  }
}

// ────────────────────────────────────────────────────────────
// Sidecar matcher
// ────────────────────────────────────────────────────────────
//
// The Rust GLB writer at `crates/starbreaker-3d/src/gltf/glb_builder.rs`
// (~line 2174) names each glTF material as:
//
//   {mtl_stem}_mtl_{material.name}_0{material_id}
//
// where `mtl_stem` is the basename (sans `.mtl`) of the source MTL file
// (e.g. `CNOU_Mustang_Alpha`), `material.name` is the authored
// SubMaterial name (e.g. `metal_a`), and `material_id` is the zero-based
// integer index of the submaterial inside the MTL — written WITHOUT
// padding but WITH a literal leading `0`, so idx 0 → `_00`, idx 11 →
// `_011`, idx 100 → `_0100`.
//
// The decomposed sidecar `*.materials.json` keeps the same material
// list and assigns each entry the same integer in `index`. The sidecar
// also exposes:
//
//   - `submaterial_name` = `material.name` (raw authored name)
//   - `blender_material_name` = `{mtl_stem}:{material.name}` (or
//     `{mtl_stem}:{material.name}_{index}` when `material.name` is not
//     unique within the MTL — see `preferred_blender_material_names`
//     in `crates/starbreaker-3d/src/decomposed.rs`).
//
// Strict equality of `blender_material_name` against the GLB material
// name therefore NEVER succeeds for the current writer — the GLB uses
// underscores and a numeric suffix, the sidecar uses a colon. The
// matcher below first tries strict equality (cheap, future-proof in
// case the writer is ever changed to align), then falls back to
// parsing the GLB suffix and matching on `(submaterial_name, index)`,
// then on `submaterial_name` alone (case-insensitive) when the suffix
// can't be parsed.

interface ParsedGlbMaterialName {
  baseName: string;
  index: number;
}

/** Parse a glTF material name written by the Rust GLB builder.
 *
 *  Primary format (underscore): `{stem}_mtl_{name}_0{digits}$`
 *  Returns the submaterial name and integer index.
 *
 *  Secondary format (colon): `{stem}:{name}` — emitted by older exports
 *  that pre-date the `_mtl_` normalisation (pre contract v2). Returns
 *  `index: -1` as a sentinel; the caller skips index-based strategies
 *  and relies on strategy (1) / (2) name equality instead.
 *
 *  Returns null if neither format matches. */
export function parseGlbMaterialName(
  name: string,
): ParsedGlbMaterialName | null {
  if (!name) return null;

  // Primary: anchored trailing `_0<digits>` AFTER an `_mtl_` separator.
  // Both must be present together — otherwise the trailing `_01` could be
  // a coincidental tail of an MTL submat name like `parkerized_metal_01`,
  // and we'd misparse a colon-format name as having an index. Critically,
  // a partial match here MUST fall through to the colon-format secondary
  // check below rather than returning null.
  const tail = /_0(\d+)$/.exec(name);
  if (tail) {
    const idx = Number.parseInt(tail[1], 10);
    const before = name.slice(0, tail.index);
    const sep = before.indexOf("_mtl_");
    if (Number.isFinite(idx) && sep >= 0) {
      const baseName = before.slice(sep + "_mtl_".length);
      if (baseName) return { baseName, index: idx };
    }
    // Partial match — fall through to colon check.
  }

  // Secondary: colon-separated `{stem}:{name}`. The stem may be a ship
  // stem, an MTL filename stem, or a generic component prefix.
  // index: -1 signals no index is available; index-based strategies
  // (3) and (5) are skipped for these names.
  const colonIdx = name.indexOf(":");
  if (colonIdx >= 0) {
    const baseName = name.slice(colonIdx + 1);
    if (baseName) return { baseName, index: -1 };
  }

  return null;
}

/** Locate the sidecar submaterial that corresponds to a glTF material
 *  loaded from a GLB. Returns the sidecar entry plus the authoritative
 *  index into `submats` (the caller wires materials by index). When no
 *  match is possible, returns null and emits a single `console.warn`
 *  with both the GLB name and the candidate sidecar names so the
 *  mismatch is debuggable from the browser console without re-running
 *  the export.
 *
 *  Match order:
 *    1. Strict equality against `blender_material_name` (cheap; works
 *       if the GLB writer is ever changed to use the same key).
 *    2. Strict equality against `submaterial_name`.
 *    3. Parse `..._mtl_<name>_0<index>$` from the GLB name and match
 *       against `(submaterial_name, index)`.
 *    4. Same as (3) but `submaterial_name` only, case-insensitive — a
 *       last-chance fallback when sidecar `index` and GLB suffix have
 *       drifted but names still align unambiguously.
 *    5. Index-only fallback: parsed GLB index matched against the
 *       sidecar entry whose `index` field equals it. This is the
 *       generic bridge between GLB material names that come from one
 *       MTL's source naming space (`pom_decals`, `graphic_decals`)
 *       and sidecar entries that come from a Blender-semantic naming
 *       space (`Decal_POM`, `Decal_DIFF`) — when the resolved sidecar
 *       was authored against a different MTL than the GLB but the
 *       loader still picks it (e.g. the KLWE laser merged-weapon GLBs
 *       resolving to `component_master_01_TEX2.materials.json`). The
 *       GLB writer at `gltf/glb_builder.rs` and the sidecar writer at
 *       `decomposed.rs` both emit submaterials in
 *       `materials.materials.iter().enumerate()` order, so positional
 *       index is preserved across the two. Logged at warn so operators
 *       can see when the names diverged enough that we resorted to
 *       positional matching.
 *
 *  Generic by category: no per-ship branches, no hardcoded names. */
export function findSubmaterialIndexForGlbName(
  glbMaterialName: string,
  submats: SubmaterialRecord[],
): number {
  if (!glbMaterialName || submats.length === 0) return -1;

  // Name-only fallback. The primary binding path reads the Rust-emitted
  // `material.extras.submat_index` directly (see scene-viewer.tsx); this
  // function only runs for older cached GLBs produced before that field
  // existed (pre contract v2). Those slots are auto-orphaned on the
  // next app mount, so this whole ladder exists mostly to make stale
  // cache hits render *something* during the brief overlap before the
  // re-export completes.

  // (1) strict equality on blender_material_name.
  let idx = submats.findIndex(
    (sm) => sm.blender_material_name === glbMaterialName,
  );
  if (idx >= 0) return idx;

  // (2) strict equality on submaterial_name (covers exporter variants
  //     that emit just the bare name as the glTF material name).
  idx = submats.findIndex((sm) => sm.submaterial_name === glbMaterialName);
  if (idx >= 0) return idx;

  // (3) parse the GLB suffix and match on (name, index).
  //     Skipped when parsed.index === -1 (colon-format, no index available).
  const parsed = parseGlbMaterialName(glbMaterialName);
  if (parsed) {
    if (parsed.index >= 0) {
      idx = submats.findIndex(
        (sm) =>
          sm.submaterial_name === parsed.baseName && sm.index === parsed.index,
      );
      if (idx >= 0) return idx;
    }

    // (4) name-only fallback, case-insensitive. Only commit if the
    //     match is unambiguous — multiple hits means we can't decide.
    const lower = parsed.baseName.toLowerCase();
    const candidates: number[] = [];
    for (let i = 0; i < submats.length; i += 1) {
      const smName = submats[i].submaterial_name;
      if (smName && smName.toLowerCase() === lower) candidates.push(i);
    }
    if (candidates.length === 1) return candidates[0];

    // (5) index-only fallback. Names diverged entirely (different
    //     naming spaces — MTL-source vs Blender-semantic), but both
    //     writers enumerate submaterials in source order, so positional
    //     alignment still matches like-for-like slot. Surface the bind
    //     so operators can see the names diverged.
    //     Skipped when parsed.index === -1 (colon-format, no index available).
    if (parsed.index >= 0) {
      const positional = submats.findIndex((sm) => sm.index === parsed.index);
      if (positional >= 0) {
        const boundName =
          submats[positional].submaterial_name ??
          submats[positional].blender_material_name ??
          `idx:${parsed.index}`;
        console.warn(
          `[decomposed-loader] index-fallback: GLB material ` +
            `'${glbMaterialName}' bound to sidecar entry ` +
            `'${boundName}' by parsed index ${parsed.index} ` +
            `(names diverged across naming spaces).`,
        );
        return positional;
      }
    }
  }

  // No match: leave the GLB's default material in place. Log enough
  // context to debug from the browser console.
  const sidecarNames = submats
    .map((sm) => sm.submaterial_name ?? sm.blender_material_name ?? "?")
    .slice(0, 16)
    .join(", ");
  const more = submats.length > 16 ? ` (+${submats.length - 16} more)` : "";
  console.warn(
    `[decomposed-loader] no sidecar submaterial matches GLB material ` +
      `'${glbMaterialName}'. Sidecar candidates: [${sidecarNames}]${more}`,
  );
  return -1;
}

/**
 * Apply a TexMod-derived UV transform to a Three.js texture. CryEngine
 * `TileU/TileV` map directly to `texture.repeat`; `OffsetU/OffsetV` to
 * `texture.offset`. Rotation pivots around the UV centre to match
 * authoring expectations (Three.js otherwise rotates around (0,0)).
 *
 * The texture cache returns the SAME `THREE.Texture` instance for every
 * call to `loadTexture(path)`. Mutating that shared instance leaks UV
 * transforms across submaterials: the Mustang Beta `decals` submaterial
 * sets `repeat=(1,1)` while a sibling submaterial that reuses the same
 * source paint sets `repeat=(2,2)` — and whichever assigns last wins for
 * every other consumer. The viewer renders the wrong stickers in the
 * wrong tiles. To stay correct, callers MUST clone the source texture
 * via `cloneTextureForSubmaterial` before applying any transform; this
 * helper otherwise treats a missing transform as identity (leaves the
 * texture unchanged).
 */
export function applyTextureTransform(
  texture: THREE.Texture,
  transform: TextureTransformRecord | null | undefined,
): void {
  if (!transform) return;
  if (Array.isArray(transform.scale) && transform.scale.length >= 2) {
    const sx = Number(transform.scale[0]) || 1;
    const sy = Number(transform.scale[1]) || 1;
    texture.repeat.set(sx, sy);
    if (sx !== 1 || sy !== 1) {
      // Tiling implies the texture should wrap; without this the GPU
      // clamps and the second tile reads transparent on the edges.
      texture.wrapS = THREE.RepeatWrapping;
      texture.wrapT = THREE.RepeatWrapping;
    }
  }
  if (Array.isArray(transform.offset) && transform.offset.length >= 2) {
    const tx = Number(transform.offset[0]) || 0;
    const ty = Number(transform.offset[1]) || 0;
    texture.offset.set(tx, ty);
  }
  const rot = typeof transform.rotation === "number" ? transform.rotation : 0;
  if (rot !== 0) {
    texture.rotation = rot;
    texture.center.set(0.5, 0.5);
  }
}

/**
 * Return a per-submaterial copy of a cached source texture so that
 * mutations (`repeat`, `offset`, `wrapS/T`, `colorSpace`, `rotation`,
 * `center`) stay scoped to one material. The clone shares the underlying
 * GPU image (via `texture.image`) — only the sampler state is duplicated,
 * which is cheap. Without this, the texture cache's promise-dedup means
 * every submaterial that loads the same source path receives the same
 * `THREE.Texture` instance, and per-slot state set by one bleed into all
 * the others.
 *
 * Three.js's `Texture.clone()` deep-copies the `Source` reference and
 * sampler/transform state. We additionally reset `needsUpdate` because
 * the parent texture is already uploaded to the GPU; the clone reuses
 * the same `Source` so it never needs an upload.
 */
export function cloneTextureForSubmaterial(texture: THREE.Texture): THREE.Texture {
  const clone = texture.clone();
  // The cloned sampler state defaults to the parent's mutated values.
  // Reset to identity so callers that DON'T have a `texture_transform`
  // get a clean slate, and so consumers don't accidentally inherit an
  // `offset` / `repeat` set by a prior consumer of the source texture.
  clone.repeat.set(1, 1);
  clone.offset.set(0, 0);
  clone.rotation = 0;
  clone.center.set(0, 0);
  clone.wrapS = texture.wrapS;
  clone.wrapT = texture.wrapT;
  // Three.js needs to know the clone is ready to render without a
  // re-upload because the underlying Source is shared with the parent.
  clone.needsUpdate = false;
  return clone;
}

/** Roles that contribute to a colour map. Detection is by the
 *  exporter-emitted role enum; the `unknown` slot fallback is handled
 *  separately by checking the TexSlotN identifier (TexSlot1 = base map). */
function isBaseColorRole(role: string): boolean {
  return (
    role === "base_color" ||
    role === "alternate_base_color" ||
    role === "decal_sheet" ||
    role.includes("diffuse") ||
    role.includes("color") ||
    role.includes("albedo")
  );
}

function isNormalRole(role: string): boolean {
  return role === "normal_gloss" || role.includes("normal");
}

function isSpecularRole(role: string): boolean {
  return (
    role === "specular_support" ||
    role.includes("metal") ||
    role.includes("specular")
  );
}

function isEmissiveRole(role: string): boolean {
  return role.includes("emissive") || role.includes("emit");
}

function isStencilRole(role: string, slotName: string): boolean {
  // `stencil` is the explicit role; `tint_palette_decal` is the role the
  // exporter assigns to MeshDecal `$TintPaletteDecal` virtual TexSlot7;
  // some MeshDecal submaterials carry their stencil in TexSlot7 with
  // role `unknown` for older sidecars.
  return (
    role === "stencil" ||
    role === "tint_palette_decal" ||
    slotName.toUpperCase() === "TEXSLOT7"
  );
}

/** Recognise CryEngine height / displacement texture roles. The exporter
 *  assigns `height` for explicit `HeightMap` slots and may use `displacement`
 *  for the same slot in some material variants. POM (parallax occlusion
 *  mapping) consumes this texture as the per-pixel ray-march source. */
function isHeightRole(role: string, slotName: string): boolean {
  return (
    role === "height" ||
    role === "displacement" ||
    role.includes("height") ||
    slotName.toUpperCase() === "TEXSLOT3"
  );
}

/** Recognise CryEngine detail / breakup / dirt overlay roles. SD wires
 *  these as multiplied surface variation in his `physical_surface`
 *  template (sd_addon_rendering_map.md §E). Three.js doesn't have a
 *  native overlay slot; we inject a multiply onto diffuseColor via
 *  `onBeforeCompile`. The slot is informational — the actual shader
 *  patch lives in `applyDirtOverlayShader`. */
function isDirtOverlayRole(role: string, slotName: string): boolean {
  return (
    role === "dirt" ||
    role === "detail" ||
    role === "breakup" ||
    role.includes("dirt") ||
    role.includes("detail") ||
    role.includes("breakup") ||
    // Common CryEngine slots for surface variation.
    slotName.toUpperCase() === "TEXSLOT4" ||
    slotName.toUpperCase() === "TEXSLOT5"
  );
}

/** Per-material shader extras: composable hooks queued during the
 *  texture-load phase and applied once when promises settle. The patches
 *  are stacked rather than overwriting `onBeforeCompile` directly so
 *  multiple effects (DDNA roughness + POM + dirt overlay + system-B
 *  iridescence + ...) coexist without clobbering each other. */
interface MaterialShaderHooks {
  ddnaRoughness?: boolean;
  pomHeightMap?: THREE.Texture;
  pomScale?: number;
  dirtOverlayMap?: THREE.Texture;
  /** Fresnel-blended secondary/tertiary palette colours used by the
   *  iridescent paint variants. Activated when the submat's source MTL
   *  filename ends in `_i` (gated upstream in
   *  `buildHardSurfaceMaterial`). Mirrors the Fresnel-blend node group
   *  used by the Blender importer. */
  systemBSecondary?: THREE.Color;
  systemBTertiary?: THREE.Color;
}

/** Apply a queued bundle of shader hooks via `onBeforeCompile`. Three.js
 *  invokes `onBeforeCompile` once per material when the program first
 *  compiles; subsequent calls require `material.needsUpdate = true`. */
function applyShaderHooks(
  material: THREE.MeshStandardMaterial | THREE.MeshPhysicalMaterial,
  hooks: MaterialShaderHooks,
): void {
  if (
    !hooks.ddnaRoughness &&
    !hooks.pomHeightMap &&
    !hooks.dirtOverlayMap &&
    !hooks.systemBSecondary
  ) {
    return;
  }
  material.onBeforeCompile = (shader) => {
    // DDNA: re-derive roughness from alpha channel.
    if (hooks.ddnaRoughness) {
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <roughnessmap_fragment>",
        [
          "float roughnessFactor = roughness;",
          "#ifdef USE_ROUGHNESSMAP",
          "  vec4 texelRoughness = texture2D( roughnessMap, vRoughnessMapUv );",
          "  roughnessFactor *= 1.0 - texelRoughness.a;",
          "#endif",
        ].join("\n"),
      );
    }

    // POM: parallax occlusion ray-march replaces the standard map UV
    // lookup with a height-displaced one. 16 sample steps is a
    // reasonable cost/quality balance for typical hull-detail surfaces.
    if (hooks.pomHeightMap) {
      const heightScale = hooks.pomScale ?? 0.015;
      shader.uniforms.pomHeightMap = { value: hooks.pomHeightMap };
      shader.uniforms.pomHeightScale = { value: heightScale };

      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <common>",
        [
          "#include <common>",
          "uniform sampler2D pomHeightMap;",
          "uniform float pomHeightScale;",
        ].join("\n"),
      );
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <map_fragment>",
        [
          "vec3 pomViewDir = normalize(vViewPosition);",
          "vec2 pomUV = vMapUv;",
          "{",
          "  const int POM_STEPS = 16;",
          "  float stepSize = 1.0 / float(POM_STEPS);",
          "  float currentLayerH = 1.0;",
          "  vec2 dtex = pomViewDir.xy * pomHeightScale / (abs(pomViewDir.z) + 0.001) / float(POM_STEPS);",
          "  float texH = texture2D(pomHeightMap, pomUV).r;",
          "  for (int i = 0; i < POM_STEPS; i++) {",
          "    if (currentLayerH <= texH) break;",
          "    pomUV -= dtex;",
          "    texH = texture2D(pomHeightMap, pomUV).r;",
          "    currentLayerH -= stepSize;",
          "  }",
          "}",
          "#ifdef USE_MAP",
          "  vec4 sampledDiffuseColor = texture2D(map, pomUV);",
          "  #ifdef DECODE_VIDEO_TEXTURE",
          "    sampledDiffuseColor = sRGBTransferEOTF(sampledDiffuseColor);",
          "  #endif",
          "  diffuseColor *= sampledDiffuseColor;",
          "#endif",
        ].join("\n"),
      );
    }

    // Dirt / detail / breakup: a multiplied overlay on diffuseColor.
    // CryEngine authors these as subtle surface variation that reads
    // best when applied AFTER the base diffuse (and POM if present)
    // have been sampled. SD does this inside his `physical_surface`
    // template node graph; we inject an extra sample + multiply at the
    // end of the diffuse fragment.
    if (hooks.dirtOverlayMap) {
      shader.uniforms.dirtOverlayMap = { value: hooks.dirtOverlayMap };
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <common>",
        [
          "#include <common>",
          "uniform sampler2D dirtOverlayMap;",
        ].join("\n"),
      );
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <map_fragment>",
        [
          "#include <map_fragment>",
          "#ifdef USE_MAP",
          "  vec4 dirtSample = texture2D(dirtOverlayMap, vMapUv);",
          // Multiplied overlay: 0 = full dirt darkening, 1 = clean.
          // Limit darkening so even fully-dirty texels still read.
          "  float dirtMix = mix(0.7, 1.0, dirtSample.r);",
          "  diffuseColor.rgb *= dirtMix;",
          "#endif",
        ].join("\n"),
      );
    }

    // Iridescent paint: Fresnel-weighted blend between secondary
    // (entryB) and tertiary (entryC) palette colours. Mirrors the
    // Fresnel-blend approach used by the Blender importer. The weight
    // `pow(1 - NdotV, 5)` is a standard Schlick-style approximation.
    //
    // Caller pre-sets `material.color = white(0xffffff)` so the
    // existing `<color_fragment>` and `<map_fragment>` paths leave
    // diffuseColor at the texture's raw sample — we then multiply
    // by the Fresnel-blended palette colour after those run.
    if (hooks.systemBSecondary && hooks.systemBTertiary) {
      shader.uniforms.systemBSecondary = {
        value: new THREE.Vector3(
          hooks.systemBSecondary.r,
          hooks.systemBSecondary.g,
          hooks.systemBSecondary.b,
        ),
      };
      shader.uniforms.systemBTertiary = {
        value: new THREE.Vector3(
          hooks.systemBTertiary.r,
          hooks.systemBTertiary.g,
          hooks.systemBTertiary.b,
        ),
      };
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <common>",
        [
          "#include <common>",
          "uniform vec3 systemBSecondary;",
          "uniform vec3 systemBTertiary;",
        ].join("\n"),
      );
      // Inject AFTER <map_fragment> so the diffuse texture has been
      // sampled. Three.js sets `vNormal` and `vViewPosition` as
      // varyings in the standard MeshStandardMaterial vertex shader,
      // so we can compute the view-space Fresnel directly.
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <map_fragment>",
        [
          "#include <map_fragment>",
          "{",
          "  vec3 sysBN = normalize(vNormal);",
          "  vec3 sysBV = normalize(vViewPosition);",
          "  float sysBFresnel = pow(1.0 - clamp(dot(sysBN, sysBV), 0.0, 1.0), 5.0);",
          "  vec3 sysBPalette = mix(systemBSecondary, systemBTertiary, sysBFresnel);",
          "  diffuseColor.rgb *= sysBPalette;",
          "}",
        ].join("\n"),
      );
    }
  };
}

/** Cumulative material-build metrics surfaced in the scene-viewer
 *  status line. Reset per load via `resetMaterialMetrics`; read via
 *  `getMaterialMetrics`. Counters increment as materials are built and
 *  as their texture promises settle. */
export interface MaterialBuildMetrics {
  totalBuilt: number;
  byFamily: Record<string, number>;
  /** HardSurface submats whose `tint_palette` had entries but
   *  `assigned_channel` was null — the loader fell back to `primary`. */
  paletteHeuristicPrimaryFallback: number;
  /** Submats where `resolvePaletteEntry` returned a usable entry. */
  paletteTintApplied: number;
  /** HardSurface submats with a `tint_palette` whose entry could not
   *  be resolved (no entries OR routed-channel-not-found). */
  paletteTintMissing: number;
  /** Submats where `material.map` ended up populated after texture
   *  promises settled. */
  diffuseTextureSuccess: number;
  /** HardSurface submats that authored at least one texture slot but
   *  ended with `material.map` still null (load failure or all slots
   *  routed to non-base roles). */
  diffuseTextureMiss: number;
  clearCoatFired: number;
  systemAFired: number;
  systemBFired: number;
  ddnaRoughnessHooked: number;
  dirtOverlayHooked: number;
  pomHooked: number;
}

function makeEmptyMetrics(): MaterialBuildMetrics {
  return {
    totalBuilt: 0,
    byFamily: {},
    paletteHeuristicPrimaryFallback: 0,
    paletteTintApplied: 0,
    paletteTintMissing: 0,
    diffuseTextureSuccess: 0,
    diffuseTextureMiss: 0,
    clearCoatFired: 0,
    systemAFired: 0,
    systemBFired: 0,
    ddnaRoughnessHooked: 0,
    dirtOverlayHooked: 0,
    pomHooked: 0,
  };
}

const metrics: MaterialBuildMetrics = makeEmptyMetrics();

/** Wipe metrics at the start of a fresh scene load. */
export function resetMaterialMetrics(): void {
  const fresh = makeEmptyMetrics();
  Object.assign(metrics, fresh);
  metrics.byFamily = fresh.byFamily;
}

/** Snapshot of the current metrics. The returned object is a shallow
 *  copy — `byFamily` is a fresh map so callers can mutate safely. */
export function getMaterialMetrics(): MaterialBuildMetrics {
  return { ...metrics, byFamily: { ...metrics.byFamily } };
}

/** Diagnostic counters tracking which material-build path each HardSurface
 *  submaterial takes. Emits a single console.info summary line ~1s after
 *  the last build, so the user can confirm S1 (ClearCoat) and S3 (System B
 *  Fresnel) are firing without having to grep individual log lines. */
const hsDiag = {
  basic: 0,
  clearcoat: 0,
  systemA: 0,
  systemB: 0,
  flushTimer: null as ReturnType<typeof setTimeout> | null,
};

function recordHardSurfaceBuild(
  useSystemB: boolean,
  isSystemA: boolean,
  wantsClearCoat: boolean,
): void {
  if (useSystemB) hsDiag.systemB += 1;
  else if (isSystemA) hsDiag.systemA += 1;
  else if (wantsClearCoat) hsDiag.clearcoat += 1;
  else hsDiag.basic += 1;
  if (hsDiag.flushTimer) clearTimeout(hsDiag.flushTimer);
  hsDiag.flushTimer = setTimeout(() => {
    const total =
      hsDiag.basic + hsDiag.clearcoat + hsDiag.systemA + hsDiag.systemB;
    console.info(
      `[hardsurface] built ${total}: clearcoat=${hsDiag.clearcoat} ` +
        `systemB=${hsDiag.systemB} systemA=${hsDiag.systemA} ` +
        `basic=${hsDiag.basic}`,
    );
    hsDiag.basic = 0;
    hsDiag.clearcoat = 0;
    hsDiag.systemA = 0;
    hsDiag.systemB = 0;
    hsDiag.flushTimer = null;
  }, 1000);
}

/** Detect whether a submaterial's source MTL is the `_i` iridescent
 *  variant (e.g. shimmerscale paints, two-tone variants). When the
 *  exporter emits `<stem>:<base>` blender_material_name strings, the
 *  stem mirrors the source MTL filename minus `.mtl`, so a hull that
 *  references `<paint>_i.mtl` shows up here as `<paint>_i:<submat>`.
 *
 *  The detection is intentionally narrow: only the trailing
 *  underscore-i in the stem position triggers, so panels named with
 *  embedded `_i` don't false-positive. */
function isSystemBIridescent(submaterial: SubmaterialRecord): boolean {
  const blenderName = submaterial.blender_material_name ?? "";
  const colonIdx = blenderName.indexOf(":");
  if (colonIdx > 0) {
    const stem = blenderName.slice(0, colonIdx);
    if (stem.toLowerCase().endsWith("_i")) return true;
  }
  return false;
}

/** Find a `TintPaletteEntry` by channel name (case-insensitive). */
function findEntry(
  entries: TintPaletteEntry[] | null | undefined,
  channelName: string,
): TintPaletteEntry | null {
  if (!entries) return null;
  const target = channelName.toLowerCase();
  for (const entry of entries) {
    if (entry.channel?.toLowerCase() === target) return entry;
  }
  return null;
}

/** Convert a `TintPaletteEntry`'s tint_color triplet to a Three.js
 *  Color, or null if the entry is missing or malformed. The exporter
 *  emits linear-space RGB, so no sRGB conversion is needed. */
function rgbFromEntry(entry: TintPaletteEntry | null): THREE.Color | null {
  if (!entry || !entry.tint_color) return null;
  const [r, g, b] = entry.tint_color;
  if (typeof r !== "number" || typeof g !== "number" || typeof b !== "number") {
    return null;
  }
  return new THREE.Color(r, g, b);
}

/**
 * Build a Three.js material for one submaterial record under a given
 * render style. Returns the material plus a list of texture-load
 * promises so the caller can wait for visual completion.
 *
 * Generic by design: no per-ship branches. Behaviour is keyed on
 * `submaterial.shader_family` and the per-slot role / alpha_semantic /
 * texture_transform fields, which the exporter populates for every
 * entity. Adding a new ship requires no code changes here.
 */
export function buildMaterial(
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
  style: RenderStyle = "textured",
): {
  material: THREE.Material;
  texturePromises: Promise<void>[];
} {
  const name =
    submaterial.blender_material_name ??
    submaterial.submaterial_name ??
    "submaterial";
  const family = submaterial.shader_family ?? "Unknown";

  metrics.totalBuilt += 1;
  metrics.byFamily[family] = (metrics.byFamily[family] ?? 0) + 1;

  // Non-textured styles ignore sidecar textures entirely. Build them
  // up-front and bail early. The palette tint is preserved for HardSurface
  // submaterials so a Mustang in Metallic style still reads as the painted
  // hull colour rather than reverting to the diagnostic grey.
  if (style !== "textured") {
    return {
      material: buildStyledMaterial(name, family, style, submaterial),
      texturePromises: [],
    };
  }

  // Family-specific PBR construction.
  //
  // Decal classification is by feature flag, not just `shader_family`.
  // CryEngine authors decals across multiple shader families: dedicated
  // `MeshDecal` for stencil-driven projection, `Illum` with `%DECAL`
  // and `%DECAL_OPACITY_MAP` flags for hull livery / "stickers", and
  // `HardSurface` with `%DECAL` for layered paint stripes (Mustang Beta
  // `decal_geometry`). All three need transparent + depth-write off +
  // polygon offset to overlay correctly on the host hull. Routing them
  // through `buildHardSurfaceMaterial` (the prior default) renders them
  // as opaque metal painted *over* the hull, which is the "wrong
  // sticker / wrong tile" bug the user reported on Mustang Beta —
  // siblings sharing the same source texture mutated each others'
  // sampler state, and the resulting opaque overlay fully occludes the
  // hull beneath.
  if (isDecalSubmaterial(submaterial)) {
    return buildDecalMaterial(name, submaterial, loadTexture);
  }
  // Family-specific dispatch. Mirrors SD's `template_plan_for_submaterial`
  // (upstream-sd/blender_addon/starbreaker_addon/templates.py:56-84):
  // 11 CryEngine shader families collapse to 7 template branches. SD's
  // 7 templates map to our 7 builder paths:
  //
  //   nodraw          -> buildNoDrawMaterial
  //   layered_wear    -> buildLayerMaterial (Layer / LayerBlend_V2)
  //   decal_stencil   -> buildDecalMaterial (MeshDecal + Illum-decal +
  //                                           HardSurface-decal via flag)
  //   parallax_pom    -> handled inside buildHardSurfaceMaterial via
  //                       has_parallax_occlusion_mapping flag (POM is
  //                       a HardSurface family with the %POM token)
  //   screen_hud      -> buildScreenMaterial (DisplayScreen, Monitor,
  //                                            UIPlane, UIMesh)
  //   physical_glass  -> buildGlassMaterial (GlassPBR)
  //   physical_surface-> buildHardSurfaceMaterial (HardSurface default)
  //
  // Plus three families SD groups under `biological` and one Hologram
  // path that we split out:
  //
  //   biological      -> buildSkinMaterial (HumanSkin_V2, Eye, HairPBR,
  //                                          Organic)
  //   hologram        -> buildHologramMaterial (Hologram, HologramCIG,
  //                                              Shield_Holo)
  //
  // Anything we don't recognise falls through to HardSurface, which is
  // CryEngine's default material family and the safe fallback per SD.
  switch (family) {
    case "NoDraw":
      return buildNoDrawMaterial(name);
    case "GlassPBR":
      return buildGlassMaterial(name, submaterial, loadTexture);
    case "MeshDecal":
      // Defensive: should be caught by `isDecalSubmaterial` above, but
      // older sidecars without `decoded_feature_flags` still fall here.
      return buildDecalMaterial(name, submaterial, loadTexture);
    case "Layer":
    case "LayerBlend_V2":
      return buildLayerMaterial(name, submaterial, loadTexture);
    case "Illum":
      return buildIllumMaterial(name, submaterial, loadTexture);
    case "DisplayScreen":
    case "Monitor":
    case "UIPlane":
    case "UIMesh":
      // Render-to-texture / UI screen families. Their authored slots
      // are TexSlot6 (screen mask) and TexSlot9 ($RenderToTexture, a
      // virtual path the engine substitutes at runtime — no on-disk
      // PNG). Routing them through buildHardSurfaceMaterial leaves no
      // diffuse and emits grey PBR. The emissive-panel stopgap is the
      // closest stand-in without a real render target.
      return buildScreenMaterial(name, family);
    case "Hologram":
    case "HologramCIG":
    case "Shield_Holo":
      return buildHologramMaterial(name, family);
    case "HumanSkin_V2":
    case "Eye":
    case "HairPBR":
    case "Organic":
      return buildSkinMaterial(name, family, submaterial, loadTexture);
    case "HardSurface":
      return buildHardSurfaceMaterial(name, submaterial, loadTexture);
    default: {
      // Unrecognised shader family — route through HardSurface (the
      // safe CryEngine default per SD) but re-tag as
      // family_unknown_default so the picker / debug-fallback mode
      // can surface it. The HardSurface builder already tagged
      // originalColor with the palette tint; we overwrite the kind
      // and re-apply debug colouring so the toggle picks up the new
      // kind.
      const built = buildHardSurfaceMaterial(name, submaterial, loadTexture);
      const m = built.material;
      m.userData.fallbackKind = FALLBACK_KINDS.FAMILY_UNKNOWN_DEFAULT;
      applyDebugFallbackToMaterial(m);
      return built;
    }
  }
}

/** Stopgap for shader families that author screen / HUD content via
 *  render-to-texture rather than a diffuse PNG. Returns an unlit
 *  emissive-coloured panel with light alpha so it reads as a glowing
 *  display. No texture lookups, so safe to call when slots are empty. */
function buildScreenMaterial(
  name: string,
  family: string,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const material = new THREE.MeshBasicMaterial({
    name,
    color: colourForFamily(family),
    transparent: true,
    opacity: 0.85,
    depthWrite: false,
  });
  material.userData.fallbackKind = FALLBACK_KINDS.SCREEN_STUB;
  tagOriginalColor(material);
  applyDebugFallbackToMaterial(material);
  return { material, texturePromises: [] };
}

/** Hologram / HologramCIG / Shield_Holo: animated translucent shader
 *  with Fresnel-driven rim brightening and scanlines. The shader needs
 *  per-frame `time` updates; we attach an updater so the scene-viewer's
 *  animate loop can tick all hologram materials.
 *
 *  This is a simple unlit-emissive + angular-fresnel approximation;
 *  the source format's hologram shader does more (scrolling noise,
 *  edge iridescence, fresnel-modulated transparency), but this reads
 *  as "hologram" in practice and is much closer than a flat fill. */
const HOLOGRAM_VERTEX_SHADER = /* glsl */ `
  varying vec3 vNormal;
  varying vec3 vViewDir;
  void main() {
    #ifdef USE_INSTANCING
      vec4 mvPos = modelViewMatrix * instanceMatrix * vec4(position, 1.0);
      vNormal = normalize(normalMatrix * mat3(instanceMatrix) * normal);
    #else
      vec4 mvPos = modelViewMatrix * vec4(position, 1.0);
      vNormal = normalize(normalMatrix * normal);
    #endif
    vViewDir = normalize(-mvPos.xyz);
    gl_Position = projectionMatrix * mvPos;
  }
`;

const HOLOGRAM_FRAGMENT_SHADER = /* glsl */ `
  uniform float time;
  uniform vec3 baseColor;
  varying vec3 vNormal;
  varying vec3 vViewDir;
  void main() {
    float fresnel = pow(1.0 - abs(dot(vNormal, vViewDir)), 2.5);
    float scanline = 0.85 + 0.15 * sin(gl_FragCoord.y * 2.0);
    float shimmer = 0.95 + 0.05 * sin(time * 3.0 + gl_FragCoord.x * 0.1);
    vec3 color = baseColor * (0.15 + fresnel * 0.6) * scanline * shimmer;
    float alpha = (0.08 + fresnel * 0.5) * scanline;
    gl_FragColor = vec4(color, alpha);
  }
`;

/** Registry of live hologram materials so the scene-viewer animate loop
 *  can update their `time` uniform each frame. The scene viewer pulls
 *  this list once per frame and writes elapsed time into each. Cleared
 *  when the scene unloads (per package generation). */
export const hologramMaterials: THREE.ShaderMaterial[] = [];

function buildHologramMaterial(
  name: string,
  family: string,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const tint = colourForFamily(family);
  // Tint base toward cyan to match CryEngine's hologram default; some
  // ships override via emissive textures but the family default is
  // a saturated cyan-blue.
  const blended = new THREE.Color();
  const hsl = { h: 0, s: 0, l: 0 };
  tint.getHSL(hsl);
  blended.setHSL(0.52 + (hsl.h - 0.52) * 0.15, 0.7, 0.55);
  const baseColorVec = new THREE.Vector3(blended.r, blended.g, blended.b);
  const material = new THREE.ShaderMaterial({
    name,
    uniforms: {
      time: { value: 0.0 },
      baseColor: { value: baseColorVec },
    },
    vertexShader: HOLOGRAM_VERTEX_SHADER,
    fragmentShader: HOLOGRAM_FRAGMENT_SHADER,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
  });
  material.userData.fallbackKind = FALLBACK_KINDS.HOLOGRAM_STUB;
  tagOriginalColor(material);
  applyDebugFallbackToMaterial(material);
  hologramMaterials.push(material);
  return { material, texturePromises: [] };
}

/** Layer / LayerBlend_V2: same backbone as HardSurface but the
 *  `layer_manifest` may publish multiple wear/damage layers. SD's
 *  `layered_wear` template blends Primary + Wear via a per-pixel mask;
 *  we currently pick the first layer whose `palette_channel` matches
 *  the submaterial's `material_channel` (already happens in
 *  `synthesisedLayerSlots`). The visible diffuse + normal end up in
 *  texture_slots and route through the standard HardSurface path —
 *  this builder exists so future layer-blend work has a hook, and so
 *  Layer materials don't fall through the dispatch's `default` arm
 *  alongside truly unknown families. */
function buildLayerMaterial(
  name: string,
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  return buildHardSurfaceMaterial(name, submaterial, loadTexture);
}

/** Illum: emissive-heavy material. CryEngine uses Illum for
 *  self-illuminated panels (control consoles, holographic ad boards
 *  that aren't full Hologram shaders, glowing accent panels). Most
 *  Illum submats carry an emissive texture; we route through the
 *  HardSurface backbone (which already wires emissiveMap from any
 *  emissive-roled slot) and bias emissive intensity up. */
function buildIllumMaterial(
  name: string,
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const built = buildHardSurfaceMaterial(name, submaterial, loadTexture);
  if (built.material instanceof THREE.MeshStandardMaterial) {
    // Illum panels are self-lit. Boost the emissive intensity so the
    // panel doesn't depend on scene lighting to be visible.
    built.material.emissiveIntensity = 1.5;
  }
  return built;
}

/** HumanSkin_V2 / Eye / HairPBR / Organic: character-shader family.
 *  SD's `biological` template implements proper subsurface for skin,
 *  anisotropic specular for eyes, and fibre highlights for hair. We
 *  ship a tuned MeshPhysicalMaterial baseline that reads passably for
 *  characters: low metalness, medium roughness, slight clearcoat for
 *  eye / wet-surface materials. Hair gets higher anisotropy via the
 *  built-in `anisotropy` knob in MeshPhysicalMaterial r155+. */
function buildSkinMaterial(
  name: string,
  family: string,
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const isHair = family === "HairPBR";
  const isEye = family === "Eye";
  const material = new THREE.MeshPhysicalMaterial({
    name,
    color: new THREE.Color(0xc9a48c),
    metalness: 0.0,
    roughness: isHair ? 0.5 : isEye ? 0.1 : 0.6,
    // Eyes get a clearcoat layer to simulate the cornea wet-surface
    // specular over the iris/sclera diffuse.
    clearcoat: isEye ? 1.0 : 0.0,
    clearcoatRoughness: isEye ? 0.05 : 0.0,
    // Hair gets anisotropy to streak highlights along the strand
    // direction (best with proper tangent UVs; fallback otherwise).
    anisotropy: isHair ? 0.7 : 0.0,
  });
  material.userData.fallbackKind = FALLBACK_KINDS.SKIN_DEFAULT;
  tagOriginalColor(material);
  applyDebugFallbackToMaterial(material);
  const texturePromises: Promise<void>[] = [];
  for (const slot of mergedSlots(submaterial)) {
    if (!slot.export_path) continue;
    const role = (slot.role ?? "").toLowerCase();
    const slotName = (slot.slot ?? "").toUpperCase();
    const promise = loadTexture(slot.export_path).then((sourceTex) => {
      if (!sourceTex) return;
      const tex = cloneTextureForSubmaterial(sourceTex);
      applyTextureTransform(tex, slot.texture_transform);
      if (isBaseColorRole(role) || slotName === "TEXSLOT1") {
        tex.colorSpace = THREE.SRGBColorSpace;
        material.map = tex;
        material.needsUpdate = true;
      } else if (isNormalRole(role)) {
        material.normalMap = tex;
        material.needsUpdate = true;
      } else if (isEmissiveRole(role)) {
        tex.colorSpace = THREE.SRGBColorSpace;
        material.emissiveMap = tex;
        material.emissive = new THREE.Color(0xffffff);
        material.emissiveIntensity = 0.8;
        material.needsUpdate = true;
      }
    });
    texturePromises.push(promise);
  }
  return { material, texturePromises };
}

/** NoDraw: collision proxies and other geometry that should never
 *  render. SD's `nodraw` template uses CLIP blend with full transparency.
 *  In Three.js we set the mesh fully invisible via material.visible
 *  semantics — but we still want to return a Material so the binding
 *  pipeline has something to attach. A zero-alpha transparent material
 *  with no depth write achieves the same effect.  */
function buildNoDrawMaterial(
  name: string,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const material = new THREE.MeshBasicMaterial({
    name,
    color: 0x000000,
    transparent: true,
    opacity: 0.0,
    depthWrite: false,
    // Never colorWrite either — completely invisible but still in the
    // scene graph so traversal doesn't break.
    colorWrite: false,
  });
  return { material, texturePromises: [] };
}

/** Classify a submaterial as decal-like across all shader families. The
 *  CryEngine source of truth is the `%DECAL` / `%DECAL_OPACITY_MAP` /
 *  `%STENCIL_MAP` tokens in StringGenMask, which the Rust exporter
 *  decodes into `decoded_feature_flags`. We also accept `MeshDecal`
 *  shader_family for older exports that predate the flag decoder.
 *
 *  Generic by category — keys off the feature flags, not the
 *  submaterial name or ship name. New decal shader variants surface
 *  automatically as soon as the exporter parses the right token. */
export function isDecalSubmaterial(submaterial: SubmaterialRecord): boolean {
  const family = submaterial.shader_family ?? "";
  if (family === "MeshDecal") return true;
  const flags = submaterial.decoded_feature_flags;
  if (!flags) return false;
  if (flags.has_decal) return true;
  if (flags.has_stencil_map) return true;
  return false;
}

/** Build one of the diagnostic / artistic styles. None of these consult
 *  textures so they render uniformly across all entities. Tinting is
 *  derived from the family rather than hardcoded per-asset so the modes
 *  remain visually informative (glass cf. hull cf. decal). HardSurface
 *  submaterials with a resolved palette tint use the palette colour
 *  instead of the family default — the Metallic/Opaque styles then read
 *  as the painted hull colour, which is more useful than uniform grey. */
function buildStyledMaterial(
  name: string,
  family: string,
  style: Exclude<RenderStyle, "textured">,
  submaterial: SubmaterialRecord,
): THREE.Material {
  const paletteTint = resolvePaletteTint(submaterial);
  const baseColour =
    family === "HardSurface" && paletteTint
      ? paletteTint
      : colourForFamily(family);
  switch (style) {
    case "metallic":
      return new THREE.MeshStandardMaterial({
        name,
        color: baseColour,
        metalness: 0.85,
        roughness: 0.25,
      });
    case "opaque":
      return new THREE.MeshStandardMaterial({
        name,
        color: baseColour,
        metalness: 0.05,
        roughness: 0.85,
      });
    case "glass":
      return new THREE.MeshStandardMaterial({
        name,
        color: baseColour,
        metalness: 0.6,
        roughness: 0.2,
        transparent: true,
        opacity: 0.35,
        depthWrite: false,
      });
    case "holographic":
    default:
      return new THREE.MeshBasicMaterial({
        name,
        color: baseColour,
        transparent: true,
        opacity: 0.45,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      });
  }
}

/** Family-keyed tint used by the diagnostic styles. Generic colour wheel
 *  by structural category: hulls grey, glass cyan, decals magenta,
 *  emissives amber. */
function colourForFamily(family: string): THREE.Color {
  switch (family) {
    case "GlassPBR":
      return new THREE.Color(0x6ca8c8);
    case "MeshDecal":
      return new THREE.Color(0xc864c8);
    case "Hologram":
    case "HologramCIG":
    case "Shield_Holo":
      return new THREE.Color(0x66ccff);
    case "DisplayScreen":
    case "Monitor":
    case "UIPlane":
    case "UIMesh":
      return new THREE.Color(0xffcc66);
    default:
      return new THREE.Color(0xc8c8c8);
  }
}

/** Detect whether a palette channel encodes iridescent / angle-shift paint.
 *
 *  Ported from upstream-sd's `_palette_channel_has_iridescence`
 *  (blender_addon/starbreaker_addon/runtime/palette_utils.py). The signal
 *  is purely the palette's per-channel data — no boolean flag exists in
 *  CIG's authored data, the engine deduces iridescence from how the
 *  finish_specular relates to the base tint_color:
 *
 *    1. The channel's spec_color must be visibly chromatic
 *       (max-min ≥ 0.10) — a flat grey spec is just standard
 *       reflectance, not shimmer.
 *    2. The spec_color must be far enough from the base tint_color
 *       (Euclidean distance ≥ 0.12) that the angular endpoints
 *       produce a visible shift.
 *
 *  For Aurora's shimmerscale paint, this fires on the tertiary channel
 *  (colorful spec like teal/purple, distant from the dark base color).
 *  The default RSI/Aurora_Mk2_White_Gray_Red palette and standard
 *  paints fail one or both tests and stay non-iridescent. */
function paletteChannelHasIridescence(
  entries: TintPaletteEntry[] | null | undefined,
  channelName: string | null | undefined,
): boolean {
  if (!entries || !channelName) return false;
  const channel = channelName.toLowerCase();
  if (channel !== "primary" && channel !== "secondary" && channel !== "tertiary") {
    return false;
  }
  const entry = entries.find((e) => e.channel?.toLowerCase() === channel);
  if (!entry || !entry.spec_color) return false;
  const [fr, fg, fb] = entry.spec_color;
  const [gr, gg, gb] = entry.tint_color;
  const specChroma = Math.max(fr, fg, fb) - Math.min(fr, fg, fb);
  if (specChroma < 0.1) return false;
  const colorDistance = Math.sqrt(
    (fr - gr) ** 2 + (fg - gg) ** 2 + (fb - gb) ** 2,
  );
  return colorDistance >= 0.12;
}

/** Build a HardSurface PBR material: BaseColor → map, NormalGloss → both
 *  normalMap and (alpha-derived) roughnessMap, Emissive → emissiveMap.
 *
 *  When the sidecar carries a `tint_palette` with a resolved
 *  `assigned_channel`, the channel's `tint_color` is set on
 *  `material.color` and the standard `final_albedo = tint × diffuse`
 *  composition falls out of MeshStandardMaterial's per-fragment colour
 *  × map multiplication automatically — no custom shader required.
 *
 *  When the routed palette channel encodes iridescence (per
 *  `paletteChannelHasIridescence`), this builds a `MeshPhysicalMaterial`
 *  with iridescence + sheen rather than a `MeshStandardMaterial`, so
 *  angle-shift paints (e.g. iridescent `_i.mtl` variants) get the
 *  tertiary palette spec_color blended toward at grazing angles.
 *  Three.js's built-in `iridescence` is thin-film, which combined with
 *  `sheen` driven by the spec_color gives a reasonable approximation
 *  of the in-engine look without a custom shader.
 *
 *  Layered HardSurface submaterials publish their visible diffuse/normal
 *  pair via `layer_manifest` rather than `texture_slots`, because the
 *  authored Layer chain (Primary, Wear, etc.) is preserved upstream for
 *  future blending work. We synthesise TexSlot entries from the layer
 *  whose `palette_channel` matches the submaterial's
 *  `palette_routing.material_channel`, so the unified slot pipeline below
 *  picks them up the same way it handles directly-authored textures. */
function buildHardSurfaceMaterial(
  name: string,
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const paletteTint = resolvePaletteTint(submaterial);
  const channelName = submaterial.palette_routing?.material_channel?.name ?? null;
  const paletteEntry = resolvePaletteEntry(submaterial);
  const paletteEntries = submaterial.tint_palette?.entries ?? [];

  // System B iridescence (synthesis_priority_actions.md S3): the actual
  // CryEngine shimmerscale mechanism. Source MTL ending in `_i.mtl`
  // routes through a Fresnel blend between secondary (entryB) and
  // tertiary (entryC) palette colours — NOT thin-film. The Fresnel
  // injection lives in `applyShaderHooks`; here we just gate the
  // material construction.
  const isSystemB = isSystemBIridescent(submaterial);
  const sysBSecondary = isSystemB
    ? rgbFromEntry(findEntry(paletteEntries, "secondary"))
    : null;
  const sysBTertiary = isSystemB
    ? rgbFromEntry(findEntry(paletteEntries, "tertiary"))
    : null;
  const useSystemB = isSystemB && sysBSecondary !== null && sysBTertiary !== null;

  // System A (thin-film) — kept narrow, only fires when System B is
  // not active and the explicit %IRIDESCENCE flag or SD's chromatic-
  // spec heuristic both point at it.
  const isSystemA =
    !useSystemB &&
    (submaterial.decoded_feature_flags?.has_iridescence === true ||
      paletteChannelHasIridescence(paletteEntries, channelName));

  // ClearCoat (synthesis_priority_actions.md S1): bit 1 of the engine's
  // GBuffer ShadingModelId. "Used on majority of ship exterior paint."
  // The exact SShaderGen permutation bit is an open RE question, so we
  // apply the heuristic: any HardSurface that resolves a palette tint
  // gets a clear coat — that includes both the routed-channel case and
  // the heuristic primary-fallback in resolvePaletteEntry, which covers
  // the chassis paint panels Aurora's main hull (Panel_LF_Paint_*,
  // Tile_*, Trim_*) leaves with assigned_channel = null. Bare-metal /
  // no-palette submaterials skip ClearCoat. System B materials skip it
  // because the Fresnel-blend already provides the angular highlight;
  // double-stacking ClearCoat over System B would mute the colour shift.
  const wantsClearCoat = !useSystemB && paletteEntry !== null;

  // Diagnostic breadcrumb: count which material-build path each submaterial
  // takes during a load so the user can verify in the browser console that
  // the S1+S3 (ClearCoat / System B) paths are firing. Throttled to one
  // summary line per second of build activity to avoid log spam on big ships
  // (Aurora has 1488 exterior bindings; Polaris has ~14k).
  recordHardSurfaceBuild(useSystemB, isSystemA, wantsClearCoat);

  // Persistent metrics surfaced in the UI. Same signal as the throttled
  // diag log but accumulates across the load and is reset per scene.
  if (useSystemB) metrics.systemBFired += 1;
  if (isSystemA) metrics.systemAFired += 1;
  if (wantsClearCoat) metrics.clearCoatFired += 1;
  if (paletteEntry) metrics.paletteTintApplied += 1;
  else if (submaterial.tint_palette) metrics.paletteTintMissing += 1;

  // Classify which path this submaterial took so debug-fallback mode
  // can recolour it. The heuristic case is detectable from the palette
  // shape: tint_palette present, assigned_channel null, but
  // resolvePaletteEntry returned a non-null entry (= it ran the
  // primary fallback).
  const tintPalette = submaterial.tint_palette;
  const heuristicFired =
    paletteEntry != null &&
    tintPalette != null &&
    tintPalette.assigned_channel == null;
  const fallbackKind: FallbackKind = heuristicFired
    ? FALLBACK_KINDS.HARDSURFACE_PALETTE_HEURISTIC
    : paletteEntry != null
      ? FALLBACK_KINDS.HARDSURFACE_PALETTE_RESOLVED
      : FALLBACK_KINDS.HARDSURFACE_NO_PALETTE;

  // System B sets the diffuse to white so the standard <map_fragment>
  // path doesn't pre-tint; the per-pixel Fresnel mix in the hook does
  // all the colouring. The non-tinted HardSurface fallback is also
  // white (NOT grey 0xC8C8C8 as it used to be) — CryEngine's behaviour
  // when a HardSurface submaterial has no palette routing is `color =
  // (1,1,1) * diffuse`, i.e. let the diffuse texture sample render at
  // its authored brightness. Multiplying by 0xC8C8C8 darkens the base
  // texture by ~22% and produced the washed-out "white-greybox" look
  // on the 125 of 196 HardSurface submats that have no `tint_palette`
  // (per the picker dump on Panel_Main_LF_Paint_7's neighbours). The
  // diffuse PNGs already carry the painted colour information; we just
  // need to not crush them.
  //
  // Debug-fallback overlay is applied at the end via
  // applyDebugFallbackToMaterial — keep build-time colour normal so
  // the original tint is what gets stashed in userData for restore.
  const baseColor = useSystemB
    ? new THREE.Color(0xffffff)
    : (paletteTint ?? new THREE.Color(0xffffff));

  const usePhysical = useSystemB || isSystemA || wantsClearCoat;
  const material: THREE.MeshStandardMaterial | THREE.MeshPhysicalMaterial =
    usePhysical
      ? new THREE.MeshPhysicalMaterial({
          name,
          color: baseColor,
          // Painted hull panels are slightly more reflective than bare
          // matte plastic — the clear coat layer carries most of the
          // specular response anyway, so the base BSDF can be tuned for
          // colour richness rather than gloss.
          metalness: useSystemB ? 0.4 : isSystemA ? 0.3 : 0.15,
          roughness: useSystemB ? 0.45 : isSystemA ? 0.4 : 0.55,
          // ClearCoat: a thin reflective layer on top of the base BSDF.
          // Conservative defaults — proper ClearCoat shading model
          // resolution from PublicParams is a follow-up.
          clearcoat: wantsClearCoat ? 0.5 : 0.0,
          clearcoatRoughness: wantsClearCoat ? 0.1 : 0.0,
          // System A thin-film — narrow path now; System B is the
          // shimmerscale port.
          iridescence: isSystemA ? 1.0 : 0.0,
          iridescenceIOR: isSystemA ? 1.3 : 1.5,
          iridescenceThicknessRange: isSystemA ? [100, 800] : [100, 400],
          sheen: isSystemA ? 0.7 : 0.0,
          sheenColor:
            isSystemA && paletteEntry?.spec_color
              ? new THREE.Color(
                  paletteEntry.spec_color[0],
                  paletteEntry.spec_color[1],
                  paletteEntry.spec_color[2],
                )
              : new THREE.Color(0xffffff),
          sheenRoughness: isSystemA ? 0.3 : 0.5,
        })
      : new THREE.MeshStandardMaterial({
          name,
          color: baseColor,
          // Bare-metal / non-paint default. Most CryEngine hull panels
          // are dielectric (per Baconator: "no metal maps. Roughness
          // maps are just Gloss Maps inverted"). Specifically authored
          // metallic submats (parkerized_metal, bronze, bare_metal)
          // carry their own per-channel data via tint_palette spec_color.
          metalness: 0.1,
          roughness: 0.7,
        });
  if (paletteEntry?.glossiness != null) {
    // Glossiness is the inverse of roughness. When provided we honour
    // it directly; the alpha-derived DDNA roughness path below still
    // runs and modulates per-pixel.
    material.roughness = THREE.MathUtils.clamp(1.0 - paletteEntry.glossiness, 0.0, 1.0);
  }
  material.userData.fallbackKind = fallbackKind;
  tagOriginalColor(material);
  // Default IBL intensity; tune per-scene via the Settings panel slider.
  material.envMapIntensity = 1.0;
  applyDebugFallbackToMaterial(material);
  const texturePromises: Promise<void>[] = [];
  const hooks: MaterialShaderHooks = {};
  if (useSystemB && sysBSecondary && sysBTertiary) {
    hooks.systemBSecondary = sysBSecondary;
    hooks.systemBTertiary = sysBTertiary;
  }
  // POM is gated by the explicit `%PARALLAX_OCCLUSION_MAPPING` token in
  // the StringGenMask. CryEngine uses POM for screws / bolts / vents /
  // grates / recesses where the mesh would otherwise need real
  // displacement geometry. Without the token, a height texture is just
  // displacement data the shader doesn't consume.
  const wantsPom =
    submaterial.decoded_feature_flags?.has_parallax_occlusion_mapping === true;

  for (const slot of mergedSlots(submaterial)) {
    if (!slot.export_path) continue;
    const role = (slot.role ?? "").toLowerCase();
    const slotName = (slot.slot ?? "").toUpperCase();
    const alphaSemantic = (slot.alpha_semantic ?? "").toLowerCase();
    const promise = loadTexture(slot.export_path).then((sourceTex) => {
      if (!sourceTex) return;
      // Clone before mutating: the texture cache hands out the same
      // instance to every consumer, so per-submaterial sampler state
      // (repeat, offset, colorSpace, wrap mode) MUST be local. Without
      // this, a sibling submaterial's TexMod tile/offset bleeds into
      // every other material that loaded the same source path.
      const tex = cloneTextureForSubmaterial(sourceTex);
      applyTextureTransform(tex, slot.texture_transform);
      if (isBaseColorRole(role)) {
        tex.colorSpace = THREE.SRGBColorSpace;
        material.map = tex;
        material.needsUpdate = true;
      } else if (isNormalRole(role)) {
        material.normalMap = tex;
        // CryEngine `_ddna` normals carry smoothness in the alpha
        // channel. Re-use the same texture as roughnessMap and inject a
        // shader that reads from .a instead of the default .g.
        if (alphaSemantic === "smoothness") {
          material.roughnessMap = tex;
          hooks.ddnaRoughness = true;
        }
        material.needsUpdate = true;
      } else if (isSpecularRole(role)) {
        // Spec maps drive metalness on the standard PBR path.
        material.metalnessMap = tex;
        material.needsUpdate = true;
      } else if (isEmissiveRole(role)) {
        tex.colorSpace = THREE.SRGBColorSpace;
        material.emissiveMap = tex;
        material.emissive = new THREE.Color(0xffffff);
        material.emissiveIntensity = 1.0;
        material.needsUpdate = true;
      } else if (isHeightRole(role, slotName) && wantsPom) {
        // Capture the height texture for POM injection. Sampler state
        // (wrap, anisotropy) is inherited from the source; POM samples
        // the .r channel only, so colorSpace doesn't need conversion.
        hooks.pomHeightMap = tex;
        // Default scale tuned for typical CryEngine surface detail:
        // small enough that flat panels stay flat, large enough that
        // screws/bolts read as inset. SD's `pom_library.blend` uses
        // height_scale * 0.015 as its default, matched here.
        hooks.pomScale = 0.015;
      } else if (isDirtOverlayRole(role, slotName)) {
        hooks.dirtOverlayMap = tex;
      } else if (slotName === "TEXSLOT1" && !material.map) {
        // Generic fallback: TexSlot1 is the base diffuse slot in the
        // CryEngine HardSurface template even when the role enum could
        // not classify it. Only used if no explicit BaseColor landed.
        tex.colorSpace = THREE.SRGBColorSpace;
        material.map = tex;
        material.needsUpdate = true;
      }
    });
    texturePromises.push(promise);
  }

  const hadTextureSlots = texturePromises.length > 0;

  // LayerBlend no-palette fallback: when a LayerBlend submaterial has no
  // tint_palette and no synthesised slots, bind the first layer_manifest
  // diffuse so the surface at least carries its authored base color.
  // Full multi-layer LayerBlend with edge-wear masks is a follow-up.
  if (fallbackKind === FALLBACK_KINDS.HARDSURFACE_NO_PALETTE) {
    const layers = submaterial.layer_manifest ?? [];
    const layerWithDiff = layers.find((l) => l.diffuse_export_path);
    if (layerWithDiff?.diffuse_export_path) {
      const layerDiffPromise = loadTexture(layerWithDiff.diffuse_export_path).then(
        (sourceTex) => {
          if (!sourceTex || material.map) return; // already bound by slot path -- don't clobber
          const tex = cloneTextureForSubmaterial(sourceTex);
          tex.colorSpace = THREE.SRGBColorSpace;
          material.map = tex;
          material.needsUpdate = true;
        },
      );
      texturePromises.push(layerDiffPromise);
      if (layerWithDiff.normal_export_path) {
        const layerNormPromise = loadTexture(layerWithDiff.normal_export_path).then(
          (sourceTex) => {
            if (!sourceTex || material.normalMap) return;
            const tex = cloneTextureForSubmaterial(sourceTex);
            const alphaSem = (layerWithDiff as LayerManifestEntry & { alpha_semantic?: string })
              .alpha_semantic;
            if (alphaSem === "smoothness" || layerWithDiff.normal_export_path?.includes("_ddna")) {
              material.roughnessMap = tex;
              hooks.ddnaRoughness = true;
            }
            material.normalMap = tex;
            material.needsUpdate = true;
          },
        );
        texturePromises.push(layerNormPromise);
      }
    }
  }

  // Defer the shader patch until after all texture promises settle so we
  // only compile once. needsUpdate inside the patch triggers the
  // recompile when the next frame renders.
  Promise.allSettled(texturePromises).then(() => {
    applyShaderHooks(material, hooks);
    material.needsUpdate = true;
    if (hadTextureSlots) {
      if (material.map) metrics.diffuseTextureSuccess += 1;
      else metrics.diffuseTextureMiss += 1;
    }
    if (hooks.ddnaRoughness) metrics.ddnaRoughnessHooked += 1;
    if (hooks.dirtOverlayMap) metrics.dirtOverlayHooked += 1;
    if (hooks.pomHeightMap) metrics.pomHooked += 1;
  });

  return { material, texturePromises };
}

/** Build a GlassPBR material using Three.js's built-in physical
 *  transmission rather than a custom ShaderMaterial. */
function buildGlassMaterial(
  name: string,
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const material = new THREE.MeshPhysicalMaterial({
    name,
    color: 0xffffff,
    metalness: 0.0,
    roughness: 0.05,
    transmission: 1.0,
    thickness: 0.05,
    ior: 1.5,
    transparent: true,
    depthWrite: false,
  });
  const texturePromises: Promise<void>[] = [];

  for (const slot of mergedSlots(submaterial)) {
    if (!slot.export_path) continue;
    const role = (slot.role ?? "").toLowerCase();
    const promise = loadTexture(slot.export_path).then((sourceTex) => {
      if (!sourceTex) return;
      const tex = cloneTextureForSubmaterial(sourceTex);
      applyTextureTransform(tex, slot.texture_transform);
      if (isBaseColorRole(role)) {
        tex.colorSpace = THREE.SRGBColorSpace;
        material.map = tex;
        material.needsUpdate = true;
      } else if (isNormalRole(role)) {
        material.normalMap = tex;
        // Light surface bumps without the full PBR roughness coupling —
        // glass tends to read better with a clean, low-roughness surface.
        material.needsUpdate = true;
      }
    });
    texturePromises.push(promise);
  }

  return { material, texturePromises };
}

/** Build a decal material across all decal-bearing shader families. The
 *  classifier (`isDecalSubmaterial`) routes Illum-with-DECAL,
 *  HardSurface-with-DECAL, MeshDecal, and STENCIL_MAP submaterials all
 *  through here.
 *
 *  Three CryEngine decal authoring patterns share this material shape:
 *    - **Atlas livery decals** (Illum + `%DECAL%DECAL_OPACITY_MAP`): a
 *      shared diffuse atlas with per-submaterial UV offset/tile to pick
 *      a tile. The diffuse alpha gates the decal. Texture clone is
 *      MANDATORY — adjacent submaterials select different atlas tiles.
 *    - **Line decals** (Illum or HardSurface + `%DECAL`): tiled stripes
 *      along the hull with the diffuse alpha + opacity falloff. Same
 *      polygon-offset / depth-write-off treatment.
 *    - **Stencil decals** (`MeshDecal` + `%STENCIL_MAP`): TexSlot7
 *      virtual `$TintPaletteDecal` resolves to a stencil atlas the game
 *      composites with palette colours at runtime. Without the palette
 *      we render the alpha mask alone (or skip if no slot loaded).
 *
 *  All three need `transparent + depthWrite=false + polygonOffset` so
 *  they overlay on the host hull without Z-fighting and without
 *  occluding the underlying paint. The hull-paint route does NOT —
 *  routing decal submaterials through it is what produced the wrong
 *  stickers / wrong tiles bug. */
function buildDecalMaterial(
  name: string,
  submaterial: SubmaterialRecord,
  loadTexture: (exportPath: string) => Promise<THREE.Texture | null>,
): { material: THREE.Material; texturePromises: Promise<void>[] } {
  const material = new THREE.MeshStandardMaterial({
    name,
    color: 0xffffff,
    metalness: 0.0,
    roughness: 0.8,
    transparent: true,
    // Use alphaTest so fully-transparent texels (between atlas tiles
    // and outside the stencil shape) don't write to depth. Decal atlas
    // tiles otherwise paint the gap region as a uniform black square.
    alphaTest: 0.02,
    depthWrite: false,
    polygonOffset: true,
    polygonOffsetFactor: -1,
    polygonOffsetUnits: -1,
    // Per SD's Aurora work (2026-04-23T03:09): "POMs & Decals no longer
    // reflect within 50mm of any surface". Decals are painted-on
    // surfaces, not separate reflective geometry — they should inherit
    // the host hull's reflection rather than producing their own.
    // Zeroing envMapIntensity stops the IBL from layering a second
    // specular response over the decal alpha.
    envMapIntensity: 0,
  });
  const texturePromises: Promise<void>[] = [];

  for (const slot of mergedSlots(submaterial)) {
    if (!slot.export_path) continue;
    const role = (slot.role ?? "").toLowerCase();
    const slotName = (slot.slot ?? "").toUpperCase();
    const alphaSemantic = (slot.alpha_semantic ?? "").toLowerCase();
    const promise = loadTexture(slot.export_path).then((sourceTex) => {
      if (!sourceTex) return;
      // Decal submaterials are the canonical case for per-material
      // texture clones: atlas decals share the same diffuse PNG across
      // sibling submaterials with different per-tile UV offsets, so
      // ANY mutation on the cached source texture corrupts every other
      // tile selection. See `cloneTextureForSubmaterial` for context.
      const tex = cloneTextureForSubmaterial(sourceTex);
      applyTextureTransform(tex, slot.texture_transform);
      if (isBaseColorRole(role) || slotName === "TEXSLOT1") {
        tex.colorSpace = THREE.SRGBColorSpace;
        material.map = tex;
        // Decal diffuse alpha gates the visible region. Without
        // explicitly assigning alphaMap, MeshStandardMaterial reads
        // alpha from `map` only when `transparent=true`, which is
        // already the case here.
        material.needsUpdate = true;
      } else if (isNormalRole(role)) {
        material.normalMap = tex;
        // Decal DDNA carries smoothness in alpha just like the hull
        // PBR path; reuse the same shader patch.
        if (alphaSemantic === "smoothness") {
          material.roughnessMap = tex;
        }
        material.needsUpdate = true;
      } else if (isStencilRole(role, slot.slot ?? "")) {
        // The stencil mask modulates alpha. Three.js doesn't have a
        // dedicated alpha-mask slot on MeshStandardMaterial, so use
        // alphaMap which multiplies the final alpha by the texture's
        // luminance.
        material.alphaMap = tex;
        material.needsUpdate = true;
      }
      // Specular / spec_support, height, dust, breakup roles have no
      // decal-relevant MeshStandardMaterial slot. Silently skipped.
    });
    texturePromises.push(promise);
  }

  return { material, texturePromises };
}

function mergedSlots(submaterial: SubmaterialRecord): TextureSlotRecord[] {
  const slots = submaterial.texture_slots ?? [];
  const direct = submaterial.direct_textures ?? [];
  const synthesised = synthesisedLayerSlots(submaterial);
  // `direct_textures` is the legacy / fallback per-role record set; for
  // most decal and HardSurface submaterials it duplicates entries
  // already present in `texture_slots` with the proper TexSlotN id and
  // texture_transform. Keep slots authoritative — only carry direct
  // entries whose `export_path` was not already covered by a slot.
  // Without this, decal materials run their texture-load + map-assign
  // pipeline twice for the diffuse, with the second run overwriting
  // any sampler state set by the first.
  const seenPaths = new Set<string>();
  for (const slot of slots) {
    if (slot.export_path) seenPaths.add(slot.export_path);
  }
  const uniqueDirect = direct.filter(
    (d) => !d.export_path || !seenPaths.has(d.export_path),
  );
  return [...slots, ...uniqueDirect, ...synthesised];
}

/** For layered HardSurface submaterials the visible diffuse and DDNA
 *  normal live on a layer entry whose `palette_channel` matches the
 *  submaterial's `material_channel`, not on the top-level
 *  `texture_slots`. Synthesise TexSlot1/TexSlot2-shaped records from that
 *  layer so the unified slot pipeline can wire them into the
 *  MeshStandardMaterial without family-specific branching. Only used when
 *  no slots are directly authored — keeps the cheap path fast for
 *  non-layered HardSurface and Illum materials. */
function synthesisedLayerSlots(submaterial: SubmaterialRecord): TextureSlotRecord[] {
  const slots = submaterial.texture_slots ?? [];
  const direct = submaterial.direct_textures ?? [];
  if (slots.length > 0 || direct.length > 0) return [];
  const layers = submaterial.layer_manifest ?? [];
  if (layers.length === 0) return [];
  const matIdx = submaterial.palette_routing?.material_channel?.index;
  // Prefer the layer whose palette_channel matches the submaterial's own
  // material_channel; fall back to the first layer with any diffuse path
  // so unrouted materials still get a visible base map.
  const layer =
    layers.find((l) => l.palette_channel?.index === matIdx && l.diffuse_export_path) ??
    layers.find((l) => l.diffuse_export_path) ??
    layers[0];
  if (!layer) return [];
  const synth: TextureSlotRecord[] = [];
  if (layer.diffuse_export_path) {
    synth.push({
      role: "base_color",
      slot: "TexSlot1",
      export_path: layer.diffuse_export_path,
    });
  }
  if (layer.normal_export_path) {
    synth.push({
      role: "normal_gloss",
      slot: "TexSlot2",
      export_path: layer.normal_export_path,
      alpha_semantic: "smoothness",
    });
  }
  return synth;
}

/** Resolve the palette entry (if any) referenced by a submaterial's
 *  `tint_palette.assigned_channel`. Returns null when no palette is
 *  present or when no entry matches.
 *
 *  Heuristic fallback: when entries exist but `assigned_channel` is
 *  null (the Rust-side routing didn't resolve a channel), HardSurface
 *  submaterials default to the `primary` entry. This is the chassis-
 *  paint case from the aurora-chassis-diagnostic — Aurora's main hull
 *  has 18-of-47 HardSurface submats with `tint_palette` populated but
 *  no routed channel, leaving them grey instead of livery-tinted.
 *  `primary` is the safe default because the per-channel paint
 *  vehicles all author primary as the dominant base coat. */
function resolvePaletteEntry(
  submaterial: SubmaterialRecord,
): TintPaletteEntry | null {
  const palette = submaterial.tint_palette;
  if (!palette) return null;
  const entries = palette.entries ?? [];
  let channelName = palette.assigned_channel;
  if (channelName == null) {
    if (entries.length === 0) return null;
    const family = submaterial.shader_family ?? submaterial.shader ?? "";
    const isHardSurfaceFamily =
      family === "HardSurface" || family === "Layer" || family === "LayerBlend_V2";
    if (!isHardSurfaceFamily) return null;
    // Channel naming in the sidecar is the engine-internal letter form
    // (entryA = primary, entryB = secondary, entryC = tertiary, entryD =
    // glass). Per the picker dump on Aurora's Panel_Main_LF_Paint_7,
    // the routed-case `assigned_channel` is "entryA", so the un-routed
    // fallback must match that convention to land on the same slot.
    // Fall back to the human-readable "primary" name if a future
    // exporter switches conventions.
    const primary =
      entries.find((e) => e.channel?.toLowerCase() === "entrya") ??
      entries.find((e) => e.channel?.toLowerCase() === "primary");
    if (!primary) return null;
    metrics.paletteHeuristicPrimaryFallback += 1;
    return primary;
  }
  const entry = entries.find((e) => e.channel === channelName);
  if (!entry) {
    const label =
      submaterial.blender_material_name ??
      submaterial.submaterial_name ??
      `idx:${submaterial.index ?? "?"}`;
    console.debug(
      `[decomposed-loader] tint_palette '${palette.palette_id}' on ` +
        `submaterial '${label}' references missing channel ` +
        `'${channelName}'.`,
    );
    return null;
  }
  return entry;
}

/** Convert a resolved palette entry's `tint_color` to a `THREE.Color`.
 *  Tint values are linear-space floats (sRGB→linear conversion already
 *  applied on the Rust side), and `THREE.Color` constructors treat their
 *  numeric arguments as linear too — so the values feed straight through
 *  with only a defensive clamp and a fall-back to white for short
 *  tuples. */
function resolvePaletteTint(submaterial: SubmaterialRecord): THREE.Color | null {
  const entry = resolvePaletteEntry(submaterial);
  if (!entry) return null;
  const c = entry.tint_color ?? [];
  const r = Math.max(0, typeof c[0] === "number" ? c[0] : 1);
  const g = Math.max(0, typeof c[1] === "number" ? c[1] : 1);
  const b = Math.max(0, typeof c[2] === "number" ? c[2] : 1);
  return new THREE.Color(r, g, b);
}

/**
 * Resolve a contract-relative path (`Data/...` or `Packages/...`) to an
 * absolute path under the given export root. Forward-slash join — the
 * Tauri command tolerates either separator.
 */
export function resolveContractPath(exportRoot: string, contractPath: string): string {
  const trimmedRoot = exportRoot.replace(/[\\/]+$/, "");
  const trimmedPath = contractPath.replace(/^[\\/]+/, "");
  return `${trimmedRoot}/${trimmedPath}`;
}
