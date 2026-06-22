# Blender POM (Parallax Occlusion Mapping) — Architecture & Runbook

How the addon renders parallax-occlusion relief for MeshDecal / FPS-weapon
materials, the parameters that drive it, and the failure modes (with fixes) that
have actually bitten us. **Read this before changing anything POM-related** — the
pipeline is subtle and several "obvious" levers are the wrong ones.

## Source of truth

The whole POM system is a port of the **old StarFab `scorg_tools` addon**, kept
in this repo at:

- `reference/Blender-Tools/scorg_tools/pom.blend` — the authored node groups
  (`POM_Vector`, `POM_10x_Layer_Steps`, `POM_parallax`, `POM_disp`,
  `tangent_space`) and a working `scorg_pom` material.
- `reference/Blender-Tools/scorg_tools/blender_utils.py` — `replace_pom_materials`
  (copies `scorg_pom`, swaps textures) and `find_and_set_displacement_image`
  (sets the height image **and** the `POM_Vector` `Bias` from the height map's
  top-left pixel, or the material's `HeightBias` property).

When POM behaviour looks wrong, diff against this reference. The displace
*modifiers* in that addon are only a z-fighting offset — **not** POM; ignore them.

## Architecture

- Bundled node library: `blender_addon/starbreaker_addon/resources/pom_library.blend`.
  Chain: `POM_Vector` → `POM_10x_Layer_Steps` → `POM_parallax` → `POM_disp`
  (+ shared `tangent_space`). `POM_disp` holds the height `ShaderNodeTexImage` +
  a `MapRange`; the chain is an unrolled ray-march (~40 steps).
- Because a Blender node group cannot take an image datablock as an *input*,
  `POM_disp` (and everything that references it up the chain) is **copied
  per-height-image**; samplerless helpers (`tangent_space`, `Clamp`) are collapsed
  to one shared canonical copy. See `_ensure_runtime_parallax_group`
  (`runtime/importer/groups.py`).
- The per-image root is named `StarBreaker POM [<height image>] [<REPEAT|CLIP>]`.
- Code map (all in `runtime/importer/`):
  - `builders._wire_runtime_parallax` — instantiates the POM group on a material,
    sets `Layers`/`Scale`/`Bias`/`Non-planar`, wires the offset Vector into the
    coverage/normal/colour samplers.
  - `builders._resolve_parallax_bias` / `_height_image_background_bias` — resolve
    the reference plane (see Bias below).
  - `builders._apply_deferred_pom_background_bias` — post-import Bias fix-up.
  - `constants.POM_SCALE_MULTIPLIER/POM_SCALE_MIN/POM_SCALE_MAX/POM_VECTOR_LAYERS`.
  - `types._bake_bitangent_sign_attribute` + `orchestration._bake_pom_mesh_bitangent_signs`
    — the mirrored-UV correction (see Vertical inversion below).
  - Control-only relief + host-variant builders:
    `builders._build_control_only_mesh_decal_pom_material` and
    `_apply_control_only_pom_overlay_to_host_material`.
- Needs **LOD0** — small HUD/weapon screens and decals are culled at LOD1.

## Parameters (`POM_Vector` inputs)

| Input | Meaning | How it's set |
|---|---|---|
| `Layers` (int) | ray-march resolution | `POM_VECTOR_LAYERS` = **40** (reference value). `apply_pom_detail_mode` can change it per the detail profile; it does **not** affect depth. |
| `Scale` (float) | parallax depth | authored `PomDisplacement` × `POM_SCALE_MULTIPLIER` (30), clamped to `[POM_SCALE_MIN 1.5, POM_SCALE_MAX 3.0]`. The reference reads at ~1.5. |
| `Bias` (float) | **reference plane** (height that = flush surface) | the height-map **background** = top-left-pixel luminance, or an authored `HeightBias`. NOT 0.5 unless the map is centred at 0.5. |
| `Non-planar` (bool) | curved-surface mode | `True` (reference value). |

`PomDisplacement` is authored *tiny* (e.g. behr ≈ 0.003) because in-game relief is
subtle; the floor keeps it visible. The functional Bias is what lets `Scale`
grow independently without the mid-level swimming.

## Failure modes → fixes (the runbook)

1. **Flat / "only bump", no real depth.** Either `Scale` is ~0 (tiny
   `PomDisplacement` not floored) **or** the ray-march is collapsed (see #3).
   Check the live `POM_Vector.Scale` value first.
2. **"Too deep" / looks like a hole with a plane below (not flush).** This is a
   **Bias / mid-level** problem, *not* Scale. `Bias` must equal the height-map
   background (~0.69 for a gray atlas), not 0.5. Increasing Scale will not fix
   it and is the wrong lever.
3. **Height sampler must be `REPEAT`, never `CLIP`.** The march walks the UV
   across the height field; `CLIP` returns transparent black outside 0–1 and
   collapses the march — the reference plane goes **inert** and the parallax
   degenerates to a constant UV shift (a "SEMI" decal renders offset as
   "EMI-F"). `_parallax_height_sampler_extension` always returns `REPEAT`.
4. **Bias appears inert (changing it does nothing).** First suspect #3 (CLIP).
   Then confirm the bundled `POM_Vector` chain matches the reference `pom.blend`
   (a divergent/edited copy can break the Bias wiring while keeping the same
   interface/node-counts). Verify with a **pixel-diff** of two Bias values, using
   a `Scale` change as a positive control.
5. **Atlas height maps (FPS weapons).** The height map is an *atlas* of many
   decals; each face uses a sub-cell. At grazing angles the march can leave the
   cell and sample neighbours. The background-referenced Bias keeps the offset
   small; keep `Scale` modest. True per-cell clamping is unimplemented (future).
6. **Parallax shifts the wrong way vertically (inverted) on some faces.** Models
   mirror one side onto the other → inverted UVs (~25% of faces). On a mirrored
   face the bitangent flips. `tangent_space` multiplies its bitangent by the
   per-corner `starbreaker_bitangent_sign` (±1 MikkTSpace sign) to compensate.
   Both halves must be present: the `Attribute`+`Multiply` in `tangent_space`
   (in `pom_library.blend`) **and** the baked attribute on the *rendered* mesh
   (`_bake_pom_mesh_bitangent_signs`, not just templates). This is a **motion**
   artifact — invisible in static renders; verify by orbiting.
7. **Bias correct on the relief but 0.5 on host-variant materials.** The
   `:POM__host_*` overlays (the ones on rendered faces) must inherit the same
   bias resolution as the `:POM` relief: pass `None` (→ background), never a
   literal 0.5. `_wire_runtime_parallax`'s default `bias_value` is `None`.

## Reading the height image (gotcha)

`image.pixels` / `image.size` on the **shared datablock are unreliable mid-import**
— Blender evicts image buffers under load pressure, so the read returns empty and
Bias silently falls back to 0.5. Read the background by loading a **fresh
throwaway copy** from `image.filepath` (`bpy.data.images.load`), which decodes
reliably, and/or do it in a deferred post-import pass
(`_apply_deferred_pom_background_bias`). `image.size` also reports `(0,0)` for a
freshly loaded image even when the pixels are readable — don't gate on it.

## Diagnostic methodology

POM bugs are spatial/motion bugs; **use data, not eyeballing renders**:

- Dump the **Bias/Scale/Layers distribution** across every `StarBreaker POM […]`
  node (which value each material got) — this is what exposed the relief-vs-host
  split.
- **Pixel-diff** two parameter values with a known-good positive control (a
  `Scale` change visibly differs; if `Bias` doesn't, it's inert).
- Check **UV winding/handedness** (signed UV area) and the
  `starbreaker_bitangent_sign` attribute via `bmesh` (note: in Edit mode the
  non-bmesh `mesh.uv_layers`/`mesh.attributes` arrays read empty).
- Motion artifacts (parallax direction, atlas-cell bleed) **cannot** be seen in
  static renders — implement from the diagnosis and verify by orbiting.

## History

Landed 2026-06-22 (`feature/ui`): `d2b93f0e8` (Bias + REPEAT), `a6c9d44df`
(mirrored-UV bitangent sign), `fdf3569dc` (host-variant Bias fallback). Earlier
`276d5261c`/`00c7590dc` (Scale calibration) were superseded — they treated the
flat/too-deep symptoms before the real Bias/REPEAT causes were found.
