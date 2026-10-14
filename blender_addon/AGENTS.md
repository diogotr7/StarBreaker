# StarBreaker Blender Addon — AI Agent Instructions

Project-wide notes live in [../AGENTS.md](../AGENTS.md). This file covers
the Blender addon specifically: target versions, layout, deploy flow,
tests, and hard-won lessons about driving Blender from an agent.

## Agent Self-Maintenance Rule

If you hit a problem during a session that cost real time and isn't already
covered in this file — wrong operator name, confusing CLI output path, a
Blender API pitfall — **document the fix here before committing**. Keep
entries short and factual. This file must stay concise to limit token use;
prefer a one-liner or a compact code block over prose.

- Headless validation (`blender -b --factory-startup --python x.py`): the load-post
  material refresh is timer-driven and never runs. `sys.path.insert(0, blender_addon)`,
  `bpy.ops.wm.open_mainfile(<package>/scene.blend)`, *then* `starbreaker_addon.register()`
  (opening a file drops it), then `package_ops.refresh_materials_for_package_root`.
- Run the stub suite with `PYTHONUTF8=1` on Windows (fixtures are UTF-8).
- Load-post material auto-refresh must treat local placeholder materials with no
  node tree as "needs refresh"; saved refreshed materials are local too, so
  checking only for linked materials is insufficient.
- VIEW_3D `Region.active_panel_category` may revert during load-post; reapply
  the `StarBreaker` tab from a short deferred timer after the first load callback.

## Target Blender

- **Latest LTS** and **latest release** — currently Blender 5.1.x.
  Cycles is the priority renderer; EEVEE parity should be considered
  where it's cheap, but do not compromise Cycles output for it.
- Python 3.13 (bundled with Blender 5.1).
- The addon is a legacy `bl_info` addon (not a Blender extension — there
  is no `blender_manifest.toml`). It installs into the user addons folder:
  - Windows: `%APPDATA%\Blender Foundation\Blender\5.1\scripts\addons\starbreaker_addon\`
  - Linux: `~/.config/blender/5.1/scripts/addons/starbreaker_addon/`
  - macOS: `~/Library/Application Support/Blender/5.1/scripts/addons/starbreaker_addon/`

  Point-version bumps (5.2, 5.3, …) change the path; update the deploy
  commands below if the user moves to a newer Blender.

## Repository Layout

```
blender_addon/
├── starbreaker_addon/          # the addon itself (what gets deployed)
│   ├── __init__.py             # bl_info, register/unregister
│   ├── manifest.py             # scene.json → dataclass parsers
│   ├── material_contract.py    # shader family / slot reconstruction
│   ├── templates.py            # reusable node-group templates
│   ├── palette.py              # palette / livery record handling
│   ├── shader_inventory.py     # CLI: shader-family usage summary of an export root
│   ├── ui.py                   # N-panel + operators
│   └── runtime/
│       ├── constants.py        # tuning knobs (light gain, cd→W, …)
│       ├── package_ops.py      # apply_paint / apply_palette / apply_light_state
│       ├── animation_tools.py  # MCP-friendly animation wrappers
│       ├── node_utils.py       # shared node-graph helpers
│       ├── palette_utils.py
│       ├── record_utils.py
│       ├── skin_utils.py / hair_utils.py   # bpy-free character material params
│       ├── validators.py
│       └── importer/           # package import pipeline (mixins)
├── tests/                      # unittest suite (stubs bpy)
└── scripts/                    # material-template library tooling
```

### PackageImporter composition (runtime/importer/)

`PackageImporter` is composed from mixins in this order:
`(PaletteMixin, DecalsMixin, LayersMixin, MaterialsMixin,
BuildersMixin, SkinMixin, HairMixin, GroupsMixin, OrchestrationMixin)`.
`HairMixin` (`importer/hair.py`) builds `HairPBR` caps and fibres (params,
incl. CHF dye bound via `applies_to`, in `runtime/hair_utils.py`). Shared node
math for both builders lives in `importer/node_math.py` (`NodeMath`).
`SkinMixin` (`importer/skin.py`) builds `HumanSkin_V2`; its parameters
(incl. CHF overrides from `root_entity.character`) resolve in the bpy-free
`runtime/skin_utils.py`. Tests that stub mixin modules by name
(`test_scene_instances.py`) must list every mixin module.

Orchestration owns `create_light`, interior placement, and the
top-level import loop. When adding a new per-entity behaviour, add it
as its own mixin rather than bloating orchestration.

## Coding Practices

See the project-wide `Coding Practices` and `Troubleshooting` sections in
[../AGENTS.md](../AGENTS.md) (root cause, no hard-coding, ask how Star
Engine handles it). Addon-specific reinforcements:

- **Keep module files small.** The addon already splits imports into
  mixins and runtime helpers; when a file passes ~500 lines, extract
  a sibling module (`runtime/<thing>_utils.py`) or a new mixin
  instead of letting it grow.
- **Prefer the runtime helpers.** `runtime/node_utils.py`,
  `palette_utils.py`, and `record_utils.py` exist so operators and
  mixins stay focused on orchestration. Reach for them before
  inlining ad-hoc node or palette code.
- **Tune via `runtime/constants.py`.** Visible knobs
  (`LIGHT_VISUAL_GAIN`, `LIGHT_CANDELA_TO_WATT`, shadow-radius
  floors, custom-property names) live there. Never hard-code a
  magic number inside a mixin.
- **Never leave the scene dirty for the next test.** If an operator
  mutates `bpy.data` in a way that another test might observe, unwind
  it or reset between fixtures (see "Reset the scene" below).

## Deploy

The installed copy must stay in lockstep with the source tree or the
MCP bridge will execute stale code. **Always sync the live Blender
install before asking the user to test** — Blender keeps running the
installed copy until its addon directory is updated. From the repo root:

```bash
# Linux
rsync -a --delete blender_addon/starbreaker_addon/ \
  "$HOME/.config/blender/5.1/scripts/addons/starbreaker_addon/"
```

```powershell
# Windows
robocopy blender_addon\starbreaker_addon "$env:APPDATA\Blender Foundation\Blender\5.1\scripts\addons\starbreaker_addon" /MIR
```

Mirroring (`--delete` / `/MIR`) is important: it removes stray stale
`.py` files (deleted modules, renamed files) that would otherwise
shadow the new code.

After syncing, force-reload the addon in Blender before validating
behaviour (see "Reload the addon after deploy"); Python module caching
keeps stale function objects alive otherwise.

## Running the Tests

The suite stubs `bpy` so it runs on system Python, not inside Blender:

```bash
cd blender_addon
python3 -m unittest discover -s tests -q
```

The suite must pass with **zero failures** after every change; the
skips are bpy-only tests that require a real `bpy` and run only under
Blender — do not try to make them pass headless. Three tests
(`test_layer_manifest_preserves_texture_slots`,
`test_representative_textures_pick_exportable_maps`,
`test_smoothness_texture_reference_falls_back_to_layer_texture_slots`)
read `component_master_01_TEX0.materials.json` from a decomposed export at
`../../ships/Data` and error when that export is absent.

**TDD rule:** When a bug is found, write a failing test that reproduces
it *before* changing any code. Verify the test fails. Then fix the code.
Verify the test passes. Pure-Python tests (no `bpy` calls) live in
`tests/` and run immediately. Tests that require a live Blender session
are decorated with `@unittest.skipUnless(HAVE_BPY, "requires bpy")`.
Do not mark a test as skip-unless-bpy if it can be tested with stubs.

## Driving Blender from an Agent (MCP)

The Blender MCP's `execute_blender_code` tool runs Python inside a
connected Blender instance. A few rules that have burned us:

### Reset the scene

Always reset between experiments in a live session — even if the scene
looks empty.
Leftover `SB_*` / `POM_*` / `StarBreaker*` node groups and `__host_*`
materials silently poison the new import.

```python
import bpy
bpy.ops.wm.read_homefile(app_template="")
```

**Do NOT** try to clear the scene by hand (`bpy.data.objects.remove`
loops, `bpy.ops.wm.read_factory_settings`, scene unlinks, etc.).
`read_homefile(app_template="")` is the only path that reliably
restores a clean Blender default without leaving orphaned data,
broken templates, or unregistered addons. Everything else either
leaves residue (lights, world, view layers, node groups) or crashes
Blender outright.

### Reload the addon after deploy

When iterating, modules get cached:

```python
import sys, bpy
for name in [n for n in sys.modules if n.startswith("starbreaker_addon")]:
    del sys.modules[name]
try:
    bpy.ops.preferences.addon_disable(module="starbreaker_addon")
except Exception:
    pass
bpy.ops.wm.read_homefile(app_template="")
bpy.ops.preferences.addon_enable(module="starbreaker_addon")
```

Without the `sys.modules` purge, `importlib.reload` is not enough —
sub-modules keep serving stale code.

### Export and open a ship

Export with the wrapper from the root [AGENTS.md](../AGENTS.md#cli-re-export)
(decomposed, `--lod 0 --mip 0 --materials all` by default; `Data.p4k`
is auto-detected — do not set `SC_DATA_P4K`):

```bash
scripts/export_ship.sh aurora_mk2 ships
```

The output argument is the **shared export root** (the parent that
holds `Packages/` and the deduplicated `Data/` tree), **not** the
package folder. The package folder is named `<entity>_LOD<n>_TEX<n>`,
so Aurora lands at `ships/Packages/RSI Aurora Mk2_LOD0_TEX0/scene.json`
and shared meshes/textures/materials land in `ships/Data/...` for reuse
across ships. Passing a package folder as the root produces a
double-nested `Packages/.../Packages/...` layout.

The exporter also assembles `scene.blend` next to `scene.json`. There is
no import operator: open that file (`bpy.ops.wm.open_mainfile`), and with
the addon enabled its load-post handler rebuilds the package materials
(preference "Refresh unloaded materials after file load", on by default;
the N-panel's **Refresh Materials** does the same on demand).

**Default test target: LOD 0, TEX0.** Validate against the
`<entity>_LOD0_TEX0` package; TEX0 is the canonical test baseline, not
TEX2.

### MCP animation tools

`starbreaker_addon/runtime/animation_tools.py` provides MCP-friendly
wrappers — no window-manager context or active-object selection required.

```python
from starbreaker_addon.runtime import animation_tools
root = "StarBreaker RSI Scorpius_LOD0_TEX0"
```

**`get_animation_list(context, root)`** → list of dicts, one per clip.
Each entry: `short_name` (e.g. `"Canopy"`), `long_name`
(`"Canopy — canopy_open"`), `fragment`, `clip_names`, `clip_fps`,
`frame_count`, `bone_count`, `modes`
(`["none", "snap_first", "snap_last", "action"]`).

**`apply_animation_mode(context, root, animation_name, mode)`**
Resolves a human name (fragment or clip name) to the internal
`fragment:<idx>:<clip>` key and applies it. Modes: `"none"`,
`"snap_first"`, `"snap_last"`, `"action"`. Returns
`{"status": "ok", "updated_count": N}`.

```python
animation_tools.apply_animation_mode(bpy.context, root, "Canopy", "snap_first")
animation_tools.apply_animation_mode(bpy.context, root, "landing_gear_extend", "snap_last")
```

**`resolve_animation_key(context, root, animation_name)`** — low-level
helper; returns the raw `fragment:<idx>:<clip>` key for use with
`apply_animation_mode_to_package_root` directly.

**`clear_animation_mode(context, root, animation_name)`** — shorthand
for `apply_animation_mode(..., mode="none")`.

To apply using the lower-level API directly (e.g. when you already have
the fragment key from `scene.json`):

```python
from starbreaker_addon.runtime.package_ops import apply_animation_mode_to_package_root
obj = bpy.data.objects.get("StarBreaker RSI Scorpius_LOD0_TEX0")
apply_animation_mode_to_package_root(bpy.context, obj, "fragment:4:canopy_close", "action")
```

### MCP output size

`execute_blender_code` will spill to a temp JSON file if stdout is
big. Keep probes targeted; filter with list comprehensions before
printing.

## Light Pipeline

- Exporter emits `color`, `intensity` (cd), `temperature` (K),
  `use_temperature`, `radius`, `inner_angle`, `outer_angle`,
  `projector_texture`, `active_state`, `states` on every light.
- Addon stashes the full state map as JSON on the `bpy.types.Light`
  datablock: `starbreaker_light_states`,
  `starbreaker_light_active_state`.
- Energy conversion (`runtime/importer/utils.py`, constants in
  `runtime/constants.py`):
  - Point / Spot: `energy_W = intensity_cd * (4π/683) * LIGHT_VISUAL_GAIN`
    with `LIGHT_VISUAL_GAIN = 20.0` (ambient proxies skip the gain).
  - Area: `energy_W = lumens / LUMENS_PER_WATT_WHITE`.
  - Sun: `energy_W_per_m2 = intensity / 683`.
  - Tune `LIGHT_VISUAL_GAIN` in `constants.py` if scenes are dim/bright.
- Runtime state switcher: `STARBREAKER_OT_switch_light_state` (N-panel
  buttons) calls `runtime.package_ops.apply_light_state(name)` which
  reapplies colour, temperature, and energy per light from the
  chosen state. Lights that lack that state keep their current values.

The light schema is in `../docs/decomposed-export-contract.md` (Light
Records); engine research is in `../docs/star-citizen-lighting-research.md`.

## Material Pipeline Notes

- **Glass** is rendered double-sided and uses a Light Path trick to
  stay visible through stacked interior+exterior panes.
- **POM decals** gate host-material tinting: decals with POM height
  inherit the host palette; flat MeshDecals do not.
- **Shimmerscale paint** green channel is documented as engine-authored
  teal. Primary channel handling uses palette tint as the dominant colour.
- **Interior palette** routes through the exterior palette for
  specific ship parts (chairs) that DataCore marks as interior-paint
  targets.
- POM node groups collapse to a small fixed set instead of one per
  texture.
- **POM (parallax-occlusion) relief** is a port of the upstream
  [scorg-tools/Blender-Tools](https://github.com/scorg-tools/Blender-Tools)
  addon (not vendored in this repo). Before
  touching it read `../docs/blender-pom-parallax-occlusion.md` — it
  documents the `POM_Vector` chain, the `Scale`/`Bias`/`Layers`
  parameters, and the failure modes that cost real time. The non-obvious
  ones: the height sampler **must be `REPEAT`** (`CLIP` collapses the
  ray-march → inert Bias + a constant UV shift); `Bias` is the
  **reference plane** = the height-map background (top-left pixel), not
  0.5 — getting it wrong reads as a recessed "hole", and that is a
  mid-level problem, **not** a `Scale` problem; mirrored-UV faces need
  the `starbreaker_bitangent_sign` correction or vertical parallax
  inverts; and `image.pixels`/`image.size` are unreliable mid-import
  (read the background from a fresh throwaway image load). Diagnose POM
  with data (Bias/Scale distribution across nodes, pixel-diffs, UV
  winding), not by eyeballing renders — motion artifacts don't show in
  stills.
- Imported meshes without authored normals get a **Weld** + **Weighted
  Normal modifier** (Face Area, Weight=50, Threshold=0.01) to smooth
  shading across flat faces. Meshes with authored normals carry
  them as the `custom_normal` corner attribute instead and get neither
  (both would override or misdecode them).

See `../docs/blender-material-contract-naming-rules.md` for the
in-repo material contract, `../docs/blender-material-template-authoring.md`
for node templates, and `../docs/blender-pom-parallax-occlusion.md`
for the POM runbook.
