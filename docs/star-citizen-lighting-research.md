# Star Citizen Lighting Research (Game-File Source of Truth)

**Status: implemented.** The light-export rewrite this research called for
(semantic classification, a separate ambient-proxy path, per-state type-aware
conversion) is in the exporter and the Blender addon; see
[Implementation](#implementation) for where. The evidence below is kept as the
rationale.

This document is based only on game assets in `Data.p4k`, not on exporter JSON
or generated `.blend` files.

## Scope

Compared ship lighting authoring between:

- `Data\ObjectContainers\Ships\DRAK\Clipper\base_int_body_main.socpak`
- `Data\ObjectContainers\Ships\DRAK\Clipper\base_ext_lg.socpak`
- `Data\ObjectContainers\Ships\RSI\aurora_mk2\rsi_aurora_mk2_cabin.socpak`
- `Data\ObjectContainers\Ships\RSI\aurora_mk2\rsi_aurora_mk2_ext_lighting.socpak`

Method:

1. Read `.socpak` payloads from `Data.p4k`.
2. Open embedded `.soc` files.
3. Parse CrCh `CRYXMLB` chunks.
4. Extract `EntityComponentLight` fields directly from CryXML:
   - `lightType`, `useTemperature`
   - state blocks (`off/default/auxiliary/emergency/cinematic`) and their `intensity`
   - child param blocks (`sizeParams`, `projectorParams`, `fadeParams`, `miscParams`, etc.)

## Evidence

### 1. Raw authored default-state intensity is ship-dependent and not globally comparable

Combined per-ship `defaultState.intensity` from raw SOC CryXML:

| Ship | Lights | Mean | Median | Max |
| --- | ---: | ---: | ---: | ---: |
| Clipper (int+ext) | 349 | 0.1949 | 0.1000 | 3.7000 |
| Aurora Mk2 (int+ext) | 59 | 9.9571 | 0.2000 | 200.0000 |

Aurora has extreme outliers (200, 40, 30, 10), while Clipper is mostly sub-1.

### 2. Aurora interior brightness is dominated by `Ambient` lights

Aurora interior (`rsi_aurora_mk2_cabin.socpak`) light types:

- `Projector`: 41
- `Ambient`: 10
- `Omni`: 2
- `SoftOmni`: 2

Aurora interior `Ambient` default intensities are: `3, 5, 5, 6, 10, 10, 30, 30, 40, 40`.

Clipper interior has **no** `Ambient` lights (types are Projector/Planar/SoftOmni/Omni only).

### 3. Aurora `Ambient` authoring has distinct structural traits

For Aurora interior `Ambient` lights, raw component data repeatedly shows:

- `lightType = Ambient`
- `importance = Key` (10/10)
- `affectsThisAreaOnly = 1` (9/10; one sparse record)
- `enabledWithGI = Always` (9/10; one sparse record)
- `projectorParams.FOV = 167.19922`
- `projectorParams.texture = textures/lights/generic/spot_075.dds`
- `miscParams.glowMultiplier = 0.2`
- `defaultState.presetTag = slow`

This signature does not match a normal punctual-light population.

### 4. Large authored values coexist with attenuating multipliers in raw data

Examples from raw game data:

- Aurora interior `Ambient`: high `intensity` (up to 40) with `glowMultiplier=0.2`
- Aurora exterior `Projector` headlights: `intensity=200`, `FOV=5`, texture `light_tube_soft_edge_02.dds`
- Clipper projectors commonly carry low or medium `intensity` with varied `glowMultiplier` and populated fade/style params

This indicates intensity is not the only authored control channel.

### 5. `cinematicState` is normalized at ~1.0 across both ships

From raw states:

- Clipper: `cinematicState` mean ≈ 0.9922, median 1.0
- Aurora: `cinematicState` mean = 1.0, median 1.0

But `defaultState` differs by orders of magnitude.  
This reinforces that state values are contextual authoring signals, not a globally uniform linear candela scale.

### 6. Authoring contains separate attenuation radius vs emitter-size controls

Raw `EntityComponentLight` components expose `sizeParams.lightRadius` for
attenuation, while visual size/shape controls live separately (`bulbRadius`,
`planeWidth`, `planeHeight`) and should not be conflated with attenuation.

Using one fixed Blender shadow-soft-size for every point/spot light discards
that authored variation and creates visibly inconsistent penumbra across ships.

### 7. MeshDecal Emissive triplets are not a reliable "this emits light" signal

From game material authoring, `Decal_POM` records can carry
`Emissive="1,1,1"` as a neutral baseline even when they are not intended to
act as visible emitters. Emission mapping should prefer explicit emission
channels (`Glow`, emissive texture) over non-zero emissive triplet alone.

Direct file evidence:

- `Data\\Objects\\Spaceships\\Ships\\RSI\\aurora_mk2\\rsi_aurora_mk2_int.mtl`
  - `Decal_POM` (`Shader=MeshDecal`) uses
    `StringGenMask=%DIFFUSE_MAP%VERTCOLORS%PARALLAX_OCCLUSION_MAPPING`,
    `Emissive=1,1,1`, and high opacity (`~0.99`) — not intended as a glow decal.
- `Data\\Materials\\vehicles\\manufacturer\\DRAK\\drak_int_master_01.mtl`
  - `Decal_Glow_Linked` (`Shader=MeshDecal`) uses
    `StringGenMask=%DIFFUSE_MAP`, `Emissive=1,1,1`, low opacity (`0.2`),
    and a dedicated glow texture under `.../Glows/.../crus_glows_diff.tif`.

So MeshDecal emission needs to be gated by structural features
(`has_parallax_occlusion_mapping`, texture class/path), not by the emissive
triplet alone.

## Evidence-backed hypothesis

High confidence hypothesis from game files:

1. Star Citizen uses **mixed light semantics** in the same SOC ecosystem:
   - normal punctual lights (`Projector`, `Omni`, `SoftOmni`, `Planar`)
   - **ambient/GI proxy lights** (`lightType = Ambient`) with a distinct parameter signature
2. Aurora Mk2 contains a concentrated set of high-intensity `Ambient` proxy lights; Clipper does not.
3. Treating `Ambient` as regular punctual emitters in Blender causes major over-brightness in Aurora interiors.
4. A single global scalar on `intensity` cannot satisfy both ships because raw authoring domains differ by type and role.

## Implementation

The rewrite direction this research derived is implemented as follows:

1. **Semantic classification before unit conversion** — `socpak.rs`
   `semantic_light_kind_for_light` classifies `lightType = Ambient` as
   `ambient_proxy` (punctual types map to `point` / `spot`, `Planar` to
   `area`). The kind is emitted as `semantic_light_kind` beside the raw
   `light_type` (`docs/decomposed-export-contract.md`, Light Records).
2. **Dedicated ambient-proxy path** — `socpak.rs`
   `authored_light_intensity_to_candela_semantic` scales ambient proxies by
   the authored `miscParams.glowMultiplier` and `AMBIENT_PROXY_DIRECT_FACTOR`;
   the addon's `_light_energy_to_blender`
   (`runtime/importer/utils.py`) skips `LIGHT_VISUAL_GAIN` for them.
3. **Per-state, type-aware conversion** — every authored state is exported in
   the light's `states` map with `intensity_raw`, the semantic candela proxy,
   `light_style`, and `preset_tag`; the addon's light-state switcher reapplies
   the per-state values.
4. **Raw payloads** — only partly done: the export keeps `intensity_raw`,
   `lightRadius` (`radius`), projector `FOV` (as half-angles) and texture,
   `glowMultiplier` (folded into the ambient conversion), and per-state
   `lightStyle` / `presetTag`. The other param blocks (`fadeParams`,
   `shadowParams`, `groupParams`, the emitter-size fields `bulbRadius` /
   `planeWidth` / `planeHeight`) are not exported, so the addon still derives
   shadow soft size from the attenuation radius (Evidence §6).

## What this rules out

- A pure global multiplier tweak as a robust fix.
- Assuming authored `intensity` is globally linear and directly comparable across ships/types.
- Treating `sizeParams.lightRadius` as the only emitter-size control.
- Treating non-zero MeshDecal `Emissive` triplets as a universal emission flag.

## Confidence

Confidence is high that the Aurora-vs-Clipper mismatch is primarily a **semantic mapping error** (ambient proxies being treated as normal punctual lights), not merely a scalar mismatch.
