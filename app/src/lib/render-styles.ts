// Viewer-agnostic render-style application.
//
// The Ships viewer has a sidecar-driven material pipeline (one
// `SubmaterialRecord` per slot, dispatched by shader family). When the
// user picks a non-textured style for a ship, the loader rebuilds each
// material by calling family-specific builders that know about the
// underlying CryEngine semantics — see `decomposed-loader.ts`.
//
// The SOC scene viewer (Maps tab) has none of that metadata: it loads a
// single `.glb` whose materials are plain `MeshStandardMaterial`
// instances that GLTFLoader synthesises from `baseColorFactor` +
// userData. So a different application path is needed there: walk every
// mesh, swap the current material for its stylised variant, remember
// the original so a switch back to "textured" restores fidelity.
//
// `applyRenderStyleToScene` here is that walk. It re-uses the underlying
// `makeMobiGlasMaterial` from `decomposed-loader.ts` so the visible
// shader is identical to the Ships path; only the dispatch surface
// differs. It also keeps a per-source-material cache so 1,500 brushes
// sharing one source material allocate one stylised material, not 1,500
// of them.

import * as THREE from "three";
import {
  makeMobiGlasMaterial,
  type RenderStyle,
} from "./decomposed-loader";

// Re-export so consumers depend on this module rather than reaching into
// the Ships-specific `decomposed-loader.ts`.
export {
  RENDER_STYLES,
  blendTowardCyan,
  clearMobiGlasMaterials,
  updateMobiGlasTime,
  type RenderStyle,
} from "./decomposed-loader";

/** Marker on `mesh.userData` that holds the pre-style material so we can
 *  restore fidelity when the user switches back to "textured". The
 *  prefix keeps it clearly internal. */
const ORIGINAL_MATERIAL_KEY = "__renderStyleOriginalMaterial";

/** Marker on `mesh.userData` recording which non-textured style is
 *  currently applied. Lets the next apply pass skip work for meshes
 *  whose style has not changed and reverse cleanup correctly. */
const CURRENT_STYLE_KEY = "__renderStyleApplied";

interface CachedStyled {
  material: THREE.Material;
  unregister?: () => void;
}

/** Walk every mesh under `root` and apply `style`. Materials swapped in
 *  for non-textured styles are cached by source-material UUID, so any
 *  meshes that already shared one source material continue to share one
 *  stylised material. Switching back to "textured" restores the
 *  originals saved on `mesh.userData`. Safe to call repeatedly with the
 *  same style (becomes a near-noop after the first pass).
 *
 *  This helper does not touch lighting, shadows, environment, or scene
 *  tone-mapping — only material instances. The MobiGlas time uniform
 *  must still be advanced per frame via `updateMobiGlasTime` in the
 *  viewer's animate loop. */
export function applyRenderStyleToScene(
  root: THREE.Object3D,
  style: RenderStyle,
): void {
  // Cache stylised materials by source-material UUID so identical source
  // materials share one styled instance. Holds for one apply call; a
  // future call rebuilds (cheap, since dedupe still folds).
  const styledCache = new Map<string, CachedStyled>();

  root.traverse((obj) => {
    if (!(obj instanceof THREE.Mesh)) return;

    const mesh = obj as THREE.Mesh;
    const currentArr: THREE.Material[] = Array.isArray(mesh.material)
      ? mesh.material
      : [mesh.material];

    if (style === "textured") {
      // Restore originals if any were saved. Dispose the per-mesh
      // stylised wrappers we'd swapped in (the underlying
      // `makeMobiGlasMaterial.unregister()` lifecycle is owned by the
      // caller via `clearMobiGlasMaterials` at scene rebuild time, so
      // we don't unregister per-material here — disposing the GPU
      // resources is enough). This keeps the path for live-style-swap
      // safe to call without leaking VRAM.
      const original = mesh.userData[ORIGINAL_MATERIAL_KEY];
      if (original !== undefined) {
        // Dispose the styled material(s) currently on the mesh before
        // restoring the original. Don't dispose the original — it lives
        // on through the GLB loader's own lifecycle.
        for (const m of currentArr) {
          // Be defensive: don't dispose the original even if userData
          // somehow points at it.
          if (m !== original && !sameMaterial(m, original)) {
            m.dispose();
          }
        }
        mesh.material = original;
        delete mesh.userData[ORIGINAL_MATERIAL_KEY];
        delete mesh.userData[CURRENT_STYLE_KEY];
      }
      return;
    }

    // Non-textured style: stash the current material as the original
    // (only on first apply for this mesh) and swap in the stylised
    // variant. Re-applying the same style is a no-op past the first
    // pass.
    const previousStyle = mesh.userData[CURRENT_STYLE_KEY] as
      | RenderStyle
      | undefined;
    if (previousStyle === style) return;

    if (mesh.userData[ORIGINAL_MATERIAL_KEY] === undefined) {
      mesh.userData[ORIGINAL_MATERIAL_KEY] = mesh.material;
    }

    // Build the stylised material(s) per source. For multi-material
    // meshes we apply per-slot so each slot's source colour drives its
    // stylised variant.
    const sources: THREE.Material[] = Array.isArray(
      mesh.userData[ORIGINAL_MATERIAL_KEY],
    )
      ? (mesh.userData[ORIGINAL_MATERIAL_KEY] as THREE.Material[])
      : [mesh.userData[ORIGINAL_MATERIAL_KEY] as THREE.Material];

    const replacements: THREE.Material[] = sources.map((src) =>
      stylisedFor(src, style, styledCache),
    );

    mesh.material = Array.isArray(mesh.userData[ORIGINAL_MATERIAL_KEY])
      ? replacements
      : replacements[0];
    mesh.userData[CURRENT_STYLE_KEY] = style;
  });
}

/** Build (or fetch from cache) the stylised material that corresponds
 *  to `source` + `style`. Cache key is the source material's UUID, so
 *  shared sources collapse to shared replacements. */
function stylisedFor(
  source: THREE.Material,
  style: Exclude<RenderStyle, "textured">,
  cache: Map<string, CachedStyled>,
): THREE.Material {
  const key = `${source.uuid}|${style}`;
  const hit = cache.get(key);
  if (hit) return hit.material;

  const built = buildStyleVariant(source, style);
  cache.set(key, built);
  return built.material;
}

/** Construct a stylised variant of `source` for the given non-textured
 *  style. Mirrors the per-arm shapes in `decomposed-loader.ts`'s
 *  `buildStyledMaterial`, but reads inputs from a generic
 *  `THREE.Material` instead of a Ships-specific `SubmaterialRecord`. */
function buildStyleVariant(
  source: THREE.Material,
  style: Exclude<RenderStyle, "textured">,
): CachedStyled {
  const baseColor = readBaseColor(source);
  const name = source.name || "styled";

  switch (style) {
    case "metallic":
      return {
        material: new THREE.MeshStandardMaterial({
          name,
          color: baseColor,
          metalness: 0.85,
          roughness: 0.25,
        }),
      };
    case "opaque":
      return {
        material: new THREE.MeshStandardMaterial({
          name,
          color: baseColor,
          metalness: 0.05,
          roughness: 0.85,
        }),
      };
    case "glass":
      return {
        material: new THREE.MeshStandardMaterial({
          name,
          color: baseColor,
          metalness: 0.6,
          roughness: 0.2,
          transparent: true,
          opacity: 0.35,
          depthWrite: false,
        }),
      };
    case "mobiglas": {
      const result = makeMobiGlasMaterial(baseColor.getHex());
      result.material.name = name;
      return { material: result.material, unregister: result.unregister };
    }
    case "holographic":
    default:
      return {
        material: new THREE.MeshBasicMaterial({
          name,
          color: baseColor,
          transparent: true,
          opacity: 0.45,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
        }),
      };
  }
}

/** Pull a base RGB tint out of `source`. MeshStandardMaterial /
 *  MeshPhysicalMaterial / MeshBasicMaterial / MeshLambertMaterial all
 *  expose `.color`; everything else falls back to a neutral grey so the
 *  stylised pass still produces a renderable material. */
function readBaseColor(source: THREE.Material): THREE.Color {
  const colored = source as unknown as { color?: THREE.Color };
  if (colored.color instanceof THREE.Color) {
    return colored.color.clone();
  }
  return new THREE.Color(0xc8c8c8);
}

/** Loose equality for materials — suffices for the safety check in the
 *  textured-restore path where we want to avoid disposing the very
 *  material we're restoring. */
function sameMaterial(a: THREE.Material, b: THREE.Material): boolean {
  return a === b || a.uuid === b.uuid;
}
