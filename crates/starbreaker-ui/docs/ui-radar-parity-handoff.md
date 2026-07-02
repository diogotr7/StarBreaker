# Radar screen (Screen_Radar_RTT) parity — handoff

Arc started 2026-06-17. Screen: `Screen_Radar_RTT` (Drake Clipper cockpit radar
MFD). Mode: fully-automated. First content parity pass (only the screen aspect
was touched before — dossier).

## Verified facts

- `Screen_Radar_RTT` binds canvas `BuildingBlocks_Canvas.MapDisplayMaster`
  (GUID `c4fd4ccf-9745-4a83-b9a7-a1aafad77855`), `binding_kind = radar`
  (from the exported LOD0 `scene.json` ui_bindings — ground truth, workflow §9).
- Both reference captures (`Screen_Radar_RTT.png`, `mapdisplaymaster.png`) show
  the SAME radar scope: elliptical concentric rings + 6 radial spokes + central
  white ship triangle + orange contact chevrons round the perimeter + perimeter
  ticks + "130°  0.7km" heading/range readout at bottom, under the DRAK CRT
  vignette. The SC cockpit radar IS a local-area map display, so MapDisplayMaster
  is the right canvas.

## What the render currently shows (baseline, md5 5afe4a36)

Near-blank: a giant cream/pale-yellow rounded panel filling most of the screen,
stray MFD chrome (nav arrows `‹ ›`, partial "APARTMENT/HABITATION" top-centre,
an "ALREADY SELECTED" cream box bottom-left, a lone orange "0" bottom-centre).
The whole radar scope is absent.

## Root-cause map (from the IR dump, 637 nodes)

MapDisplayMaster is a MULTI-MODE map component. Top-level sub-displays (all
ACTIVE at once at static rest — the bug):
`StarMapDisplay`, `InteriorMapDisplay`, `MapVelocitySFX`, `StarMapDisplayRTT`,
`InteriorMapDisplayRTT`, `UIOverlayRTT`, `GalaxyMapDisplay`, `canvas_Readouts`.

- Mode flags are engine state under `/MapNamespace`:
  `IsInteriorMapActive`, `GeneralMapData/IsStarMapActive`, `IsMapActive`,
  `IsVolumetric`, `IsRTT`; plus `GeneralMapData/DisplayRadius`,
  `DisplayOrientation/{x,y,z}`, `DisplayPosition/{x,y,z}`. Absent at static rest
  → per-mode visibility gating doesn't select one mode → InteriorMap chrome wins.
- The actual radar scope is `PlayerRadarPlane > PlaneRoot > RadarCircleBase >
  HostplaneVisuals_Small > RadarPlaneRingsBase` (rings `Circle_Ripple_Textured`)
  under `StarMapDisplayRTT > CanvasProxyRoot > WindowContainer > 3D Root` —
  currently `[OFF]`. The rings are 2D widgets parented under a 3D-projected /
  RTT plane whose transform comes from live camera/orientation; at rest they
  collapse to ~0×0. This is a live 3D-RTT render (analogous to the self-status
  hologram, ledger 70).
- Radar readouts ("130° 0.7km") come from `mapdisplaystarmap_radarreadouts`
  under `canvas_Readouts`; only "0" renders at rest. Heading 130° + contacts are
  LIVE state (unreproducible statically, like the compass live heading);
  `DisplayRadius` would drive the range.
- Contact chevrons + perimeter ticks are projected from live nearby-contact data
  (`EdgeMarkers`/`VisibleMarkers` lists — empty at rest).

## Naming caveat (owner-flagged 2026-06-17)

The canvas's `StarMapDisplay*` / `IsStarMapActive` identifiers are the GENERIC
map-render subsystem (the same pipeline that draws the radar plane) — **NOT the
in-game navigation "starmap"**. This screen is the **MFD radar**. The radar mode
is distinguished by `IsRadar` (+ `IsRTT`); the radar plane is hosted in the
StarMap-RTT display, so `IsStarMapActive` rides along as its host flag.

## LANDED — catalog #1 (mode gating)

Registry pins added (`default_value_registry_v1.json`, exact raw binding-string
keys): `IsRTT=true, IsStarMapActive=true, IsRadar=true, IsInteriorMapActive=false,
IsGalacticMapActive=false, IsVolumetric=false, IsMapActive=true`. TDD guard:
`bb_state_filter::tests_b::radar_mode_registry_defaults_select_starmap_rtt_over_interior_map`
(red→green). Re-render (md5 99cdab81): the interior-map cream over-paint is GONE,
the DRAK dashboard vignette shows, node count 637→148. Catalog #1 + #6 ✓. The
`canvas_Readouts` "RadarMagnification" bar now renders (via `IsRadar`) — currently
a `LockedIcon` + "º" heading suffix + empty magnification (catalog #3, below).

## Diff catalog (priority order) + status

1. **[CRITICAL] Mode gating — interior-map chrome over-paints.** ✅ **FIXED**
   (registry pins, above). Interior/Star/Galaxy modes deactivated; dark vignette
   shows; node count 637→148.
2. **[CRITICAL/dominant] Radar scope grid** (rings/spokes/ship triangle).
   ✅ **LANDED — data-faithful RTT renderer (owner directed "build it").** The disc
   is NOT invented: it's the REAL engine texture
   `UI/Textures/R_RadarMapScreen/3D_Object_Textures/r_radarmapscreen_radial_gradients.dds`
   (concentric rings + perimeter degree-tick scale + axis), bound by the
   `Circle_Radial_Grid` Primitive node's `ui_r_radarmapscreen_radial_grid.mtl`
   (TexSlot1). Pipeline (mirrors the SELF-STATUS hologram):
   `gfx::radar_plane::project_radar_disc` decodes + tilt-projects that texture
   (flat disc → ellipse) tinted by the brand `Accent1`, + the `Circle_Ripple_Textured`
   WidgetCircle outer ring (Accent stroke) + the white own-ship triangle;
   `HologramFetcher::fetch_radar_plane` (P4kHologramFetcher) loads the brand disc
   `.mtl`→texture (split-mip DDS) and projects; `ir_compose` composites it into the
   radar `WidgetWindow` (material `map_window`), gated so no frozen screen is
   touched. **PER-MANUFACTURER**: the IR `primitive_material` prefers the
   cascade-applied `PrimitiveMaterialPath` brand override (Greycat `ui_grin_…`,
   RSI `…_RSI`) over the authored generic (DRAK keeps generic) — so the disc
   texture is manufacturer-correct. **DATA vs owner-tuned**: disc art + ring +
   ship + tint all DATA; ONLY the camera tilt (37°, the engine runtime camera is
   absent at rest) + emissive brightness are owner-tuned (the hologram-camera
   boundary). Commits `9e38f1501`/`d76270597`/`e530c0820`. RESIDUAL: the 6 radial
   spokes read fainter than the reference (in the gradients texture the material
   binds; ref likely brighter via emissive bloom / radialUV); live contacts +
   heading/range "130° 0.7km" unreproducible at rest (live state, like compass).
   **History — ⛔ was a PROVEN 3D-RTT BLOCKER (researched per owner):**
   After the gating fix the radar plane IS active but COLLAPSED to ~0.5×0.8px at
   centre (512,392): `PlayerRadarPlane > PlaneRoot > RadarCircleBase >
   HostplaneVisuals_Small > RadarPlaneRingsBase` / `RadarSpokeLinesBase` /
   `NavigationGrid{1,2,3}{Mid,Alt}Plane`, all under `3D Root > WindowContainer`.
   **`WindowContainer` is a `BuildingBlocks_WidgetWindow`** (`rendererType:Primitive`,
   material `Materials/UI/Starmap/map_window.mtl`, **`camera {WindowCamera,
   fieldOfView:20}`**, `windowPreviewScene` RTT) — an RTT WINDOW that renders 3D
   content through a 3D camera into a texture. `PlayerRadarPlane` (a `WidgetCanvas`
   loading the radar-disc sub-canvas: rings/spokes/ship/nav-grids) is that 3D
   content. The window's camera transform comes from the live radar state
   (`/MapNamespace/GeneralMapData/DisplayPosition/{x,y,z}`,
   `DisplayOrientation/{x,y,z}`, `DisplayRadius` — decoded NumberVariables, absent
   at static rest) → the projection collapses to a point. Reproducing the scope
   needs a **WidgetWindow RTT 3D-projection renderer + radar-disc geometry
   rasterisation** — a substantial subsystem comparable to / larger than the
   self-status hologram CPU rasteriser (ledger 70), and even with it the at-rest
   camera transform is live (the ref's populated 130°-heading scope is an
   in-flight capture). EVIDENCE TRAIL: `Screen_Radar_RTT`→`MapDisplayMaster`→radar
   mode→`StarMapDisplayRTT`→`mapdisplaystarmap_window`→`WindowContainer`(WidgetWindow,
   camera FOV 20)→`PlayerRadarPlane`(3D disc). Owner declined "build it" (option 3)
   at the catalog gate; researched-further at the major-item gate confirms the
   blocker.
3. **[HIGH] Heading/range readout** "130° 0.7km".
   - **#3a lock icon** ✅ **FIXED** (`ShowRadarLocked=false` pin): the padlock
     (`LockedIcon`, `icon_common_door_locked.svg`) is gone.
   - **#3b orange backplate** ✅ **FIXED** (AR_HoloVolume Primitive-card
     suppression). The readout card `RadarMagnification` is `rendererType:Primitive`
     with material `AR_HoloVolume_standards/ui_ar_card_in_holo_volume.mtl` and
     authored `background.color = ColorStyle "Base"` (DRAK orange). The
     `Type(Card)∧Tag(Locked)→BackgroundColor:null` suppressor exists ONLY in the
     `s_grin_hud`(Greycat)/`s_rsi` brand blocks of `mapdisplaystarmap_radarreadouts`
     — the canvas has NO drak brand block and an EMPTY `defaultStyles`, so a DRAK
     (no-brand-match) ship leaks the authored "Base" backplate as an opaque orange
     bar (the reference shows bright text on the DARK vignette, RGB [54,36,14], NO
     backplate — measured). Fix: `node_background_enabled` now suppresses the flat
     background of a `rendererType:Primitive` node whose `primitiveMaterialPath`
     is an **AR HOLO-VOLUME material** (a 3D-volume card; its flat backplate isn't
     drawn on the flat UI). Discriminator is the MATERIAL, not the colour, so the
     real palette-role Primitive fills keep rendering — power `PipBox_Fill` (empty
     material) + master-mode `card_BarFill` (`materials/default_rtt.mtl`); the
     self-status hologram (`WidgetRuntimeImage`) is excluded. No frozen MFD/HUD
     screen uses AR_HoloVolume cards (grep-verified). TDD guards
     (`node_background_enabled_tests` in `ui_ir/engine_01.rs`).
     RESIDUAL (live, deferred): the readout VALUES — heading
     `FlightController/Compass/Value`, range `…/radarrangemeters` — are LIVE; the
     ref's 130°/0.7km is an in-flight capture (render shows at-rest 0°). Like the
     compass live heading, unreproducible statically. So full readout parity is
     bounded regardless.
4. **[MED] Contact chevrons absent** — live nearby-contact data
   (`EdgeMarkers`/`VisibleMarkers`, empty at rest). Deferred-with-proof.
5. **[MED] Perimeter ticks absent** — part of the radar plane (#2).
6. **[LOW] Background vignette** ✅ correct (dark DRAK vignette shows after #1).

## Landed commits
- (this arc) registry radar-mode pins + `ShowRadarLocked` + TDD guards
  (`bb_state_filter::tests_b::radar_mode_registry_defaults_select_starmap_rtt_over_interior_map`,
  `radar_locked_icon_hidden_when_show_radar_locked_pinned_false`).

## Session 2 (owner-driven polish, 2026-06-17) — LANDED

- ✅ **Heading + range readout** (`e1c23ec37`): heading text shares the compass
  source (`FlightController/Compass/Value` derived from `COMPASS_AT_REST_HEADING_DEG`,
  so radar + compass agree) → "0°"; range = registry `radarrangemeters=700` + the
  `LocalizedSIUnitFromNumber forcedSIPrefix=Kilo` fix → "0.7km". (It IS the radar
  range/magnification, not altitude — the field is `RadarMagnification`.)
- ✅ **Disc orientation** (`63b3bc915`): apply the disc material's `ViewingAngle=180`
  (data-backed, per-manufacturer) — fixes the inverted disc.
- ✅ **Sweep wedge** (`a525fb7c6` + flip/stretch `…`): the real `idle_animation`
  wedge, clockwise (flipped), radially stretched toward centre.
- ✅ **Background** (`ec9772a07`): the missing `DRAK_GroundVehicle_Dashboard_background_2`
  orange-glow panel, via a scoped `forced_active_widgets_with_defaults` (activates
  authored-false `WidgetImage` nodes whose `IsActive` resolves genuine `Some(true)`
  — the `NOT(IsVolumetric)` gate). WidgetImage-scoped to avoid the medical bed
  (workflow §10). `--full` GREEN.
- ✅ **Spokes** (`e677ef971`): 8 `Circle_Line` radial bars (cardinals longer than
  diagonals), data-driven geometry, drawn in the tilted plane.
  - ⚠️ **OVERTURNED 2026-06-17 — those were INVENTED (generated), not data.** Owner
    pushed ("are they generated? … look for a texture"; "must be data backed … will
    a re-render pick up an 8→4 change?"). Re-derived from data:
    - The real spokes are the `Circle_Line_000…315` Primitive nodes in
      `rc_radarmapscreen_hostplane_visuals_LARGE.json` (`line_a.mtl`): 8 bars, 45°
      spacing (node-name angle), `sizing` width 0.002 + height 0.4 (cardinals) /
      0.3 (diagonals) Percent, `orientation.z` 0/45/90/135, per-spoke colour
      cardinals `Accent1` / diagonals `Base` (alpha 0.1, `135`/`315` 0.2). Soft look
      = the `line_a` material (`Gradient=1`, `InnerAlpha=0.5`, `OuterAlpha=1`,
      `Glow=0.23`; the `.dds` is a plain white quad). All DATA.
    - **The spokes live in a DIFFERENT, mutually-exclusive host-plane than the disc.**
      `rc_radarmapscreen_hostplane.json` op-graph:
      `HostplaneVisuals_Large.Instantiated = StarMapData/CommonData/IsFullScreen`,
      `HostplaneVisuals_Small.Instantiated = NOT IsFullScreen`. `IsFullScreen` is a
      `/`-path toggle (escapes `apply_idle_defaults` `.`-grouping) → genuinely UNSET.
      SMALL (cockpit) = the `radial_grid` textured disc + sweep, **empty spokes**;
      LARGE (full-screen) = the 8 spokes + ring borders, **no `radial_grid` disc**.
      `map_window.mtl` is `UIPlane`/`$RenderToTexture` (the disc is RTT scene content,
      not the window bg). The reference shows BOTH disc and spokes → **owner chose
      "composite the real Large spokes over the Small disc"** (AskUserQuestion).
    - FIX: a generic `bb_state_filter` rule — when two SUB-CANVAS variants
      (`WidgetCanvas` + `canvas` URL) are gated `X` / `NOT X` on an UNSET toggle,
      keep BOTH instantiated (can't pick a mode at rest → composite both authored
      variants). Scoped to sub-canvas variants via `is_subcanvas_variant` so in-scene
      widget toggles (medical/target MFD) keep exclusivity (the unscoped first cut
      regressed `ui_target_a` +1 draw-order). Then `ir_compose::collect_radar_spokes`
      reads the now-loaded `Circle_Line` nodes (geometry + resolved Accent1/Base fill)
      → `fetch_radar_plane` reads `line_a.mtl` (Gradient/InnerAlpha/OuterAlpha/Glow)
      → `radar_plane::project_radar_disc` draws soft faded glow bars (no crisp outer
      ring; `outer_ring_alpha=0`). 100% data-backed + per-manufacturer (brand-resolved
      `primitive_material`); an 8→4 / colour / length change re-renders without code.
- ⏸ **Readout kerning (#12)**: DEFERRED with proof — no authored LetterSpacing
  exists; it's the Electrolize SWF font's intrinsic advance (a global SWF
  text-metrics model question, same class as the prior power/target deferrals).

## Session 3 (2026-06-17) — spokes overturned, #13, chrome

- ✅ **Spokes data-backed** (`f980a7189`) — see [[radar-hostplane-composite-spokes]]
  / the OVERTURNED note above. Real `Circle_Line` nodes composited from the
  full-screen host-plane via the generic unset-`X`/`NOT X` rule.
- ✅ **#13 spokes + sweep reach centre** (`9cd34e58e`) — `spoke_inner_reach=0.35`
  (pull inner ends to ~centre), `spoke_outer_reach=0.82` (pull outer ends to the
  bright ring → diagonals stop jutting past the ellipse), `SWEEP_RADIAL_GAMMA`
  0.45→0.30. Owner-tuned radial-extent (runtime plane scale absent at rest).
- ✅ **Outer heading-tape chrome (#10) LANDED** — the `HeadingTape` ring is REAL
  data, like the spokes (NOT the "procedural" the note below feared). It's the
  `coordinates_novalue` tick-marker atlas (`ui_r_radarmapscreen_headingtape_simple.mtl`,
  TexSlot1) wrapped around the perimeter. Lives in `canvas_RadarNavElements`
  (`volumetricnavelements.json`), gated `Instantiated = IsRadar` — but the BARE
  `IsRadar` (not the qualified `/~/MapNamespace~/StarMapData/CommonData/IsRadar`
  the readout uses; confirmed via `BB_STATE_PROBE`). Registry pin `"IsRadar": true`
  activates it (radar-only binding → safe; `ui_check` GREEN). It's RTT content
  (collapses at rest) so it's PROJECTED like the disc: `ir_compose` detects the
  `headingtape` Primitive with a TILED atlas window (`UVSize.x>1`, the structural
  discriminator vs the single-cell `NorthPoint`/readouts) and passes its material +
  authored `UVStart (-0.24,0.44)`/`UVSize (18,0.06)` (new `#[serde(skip)]`
  `primitive_uv_start/size` IR fields) → `fetch_radar_plane` loads the atlas →
  `radar_plane::project_radar_disc` wraps the tick row around the perimeter ellipse
  (`HEADING_RING_RADIUS 0.94`, owner-tuned curl radius — the runtime
  `radialTransform.transformMultiplier=0` at rest; `RADAR_HEADING_ALPHA 2.0`). The
  atlas window + content + UV are DATA + per-manufacturer (brand-resolved
  material). DEFERRED still: contact chevrons (live), `HeadingRotation` (live, 0 at
  rest). `map_window.mtl` is `UIPlane`/`$RenderToTexture`.

## Remaining

- **Outer heading-label chrome (#10)** — ✅ LANDED (see Session 3). Original note:
  the `Headings`/`HeadingTape` ring
  (`rc_radarmapscreen_volumetricnavelements.json`: 36 degree labels from the
  `r_radarmapscreen_coordinates_novalue` atlas, curled into a ring via
  `radialTransform` curvatureAxis Y, 10° spacing, `HeadingRotation`=0 at rest;
  per-manufacturer `grin_coordinates`). **That canvas is NOT loaded in our radar
  IR** (only a collapsed `canvas_RadarNavElements`), so it can't be resolved from
  a node — it needs a PROCEDURAL heading-ring: brand-resolve the coordinates
  atlas, extract the per-heading cell (grid 9 cols × 4 rows, UStart=(i%9)/9,
  VStart=(i//9)*0.08203+0.0249), and place 36 cells around the foreshortened
  ellipse (a focused follow-up; full mechanism in the research notes).
- **Contact chevrons** — live nearby-contact data (deferred, like the compass).

## Earlier remaining items (now resolved — see Session 2 above)

- **Background image (#11)** — the radar's `DRAK_GroundVehicle_Dashboard_background_2.dds`
  (dark panel + orange edge/corner glow; a DIFFERENT texture than other MFDs) is
  the `image_Background` root node, authored `isActive=false` with `IsActive ←
  NOT(IsVolumetric)`. At the flat radar (IsVolumetric=false) it SHOULD activate,
  but `bb_state_filter` only DEACTIVATES nodes — it never ACTIVATES an
  authored-false node on a true binding. RISK: a generic IsActive-activation pass
  is the documented medical-breaker (workflow §10). Possible SCOPED fix: activate
  authored-false nodes whose `IsActive` op resolves a genuine `Some(true)` from a
  PINNED/resolved state (not the unset→override true that medical's live-gated
  nodes hit) — must validate with `--full` (medical platinum is the canary).
- **Outer dots/chrome (#10)** — `r_radarmapscreen_coordinates(_novalue).dds` is a
  heading-degree-label + glyph ATLAS (010–350 grid); the `headingtape` material
  places cells around the outer ring per heading. Complex (angular atlas-cell
  placement on the tilted ring). Per-manufacturer (`grin_coordinates.dds`).
- **Spokes (#9)** — the radial spokes are in the disc `radial_gradients` texture
  (the material's bound TexSlot1) but read faint; the reference's prominence is
  likely emissive bloom + the radial-UV mapping. Not a separate asset (the
  navigation-grid texture is a plain white line tile; the grid is geometry).
- **Readout kerning (#12)** — the bottom readout (`Text_Radar*` = WidgetTextField
  components, Electrolize font) is too tightly spaced. No authored LetterSpacing
  on the field; it's the font advance / text-format — a delicate text-metrics
  residual (cf. prior LetterSpacing-pitch deferrals).

## Next step
Major-item blocker gate for #2 (radar scope = 3D-RTT render). Owner decision on
#3b (attempt the token-leak fix vs defer).
