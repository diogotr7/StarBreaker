# StarBreaker Blender Addon — AI Agent Instructions

Project-wide notes live in [../AGENTS.md](../AGENTS.md). This file covers
the Blender addon specifically: target versions, layout, deploy flow,
tests, and hard-won lessons about driving Blender from an agent.

## Target Blender

- **Latest LTS** and **latest release** — currently Blender 5.1.x.
  Cycles is the priority renderer; EEVEE parity should be considered
  where it's cheap, but do not compromise Cycles output for it.
- Python 3.13 (bundled with Blender 5.1).
- The addon loads as an extension under
  `~/.config/blender/5.1/scripts/addons/starbreaker_addon/` on Linux.
  Point-version bumps (5.2, 5.3, …) change the path; update the deploy
  command below if the user moves to a newer Blender.

## Repository Layout

```
blender_addon/
├── starbreaker_addon/          # the addon itself (what gets deployed)
│   ├── __init__.py             # bl_info, register/unregister
│   ├── manifest.py             # scene.json → dataclass parsers
│   ├── material_contract.py    # shader family / slot reconstruction
│   ├── templates.py            # reusable node-group templates
│   ├── palette.py              # palette / livery record handling
│   ├── ui.py                   # N-panel + operators
│   └── runtime/
│       ├── constants.py        # tuning knobs (light gain, cd→W, …)
│       ├── package_ops.py      # apply_paint / apply_palette / apply_light_state
│       ├── node_utils.py       # shared node-graph helpers
│       ├── palette_utils.py
│       ├── record_utils.py
│       ├── validators.py
│       └── importer/           # package import pipeline (mixins)
├── tests/                      # unittest suite (stubs bpy)
└── scripts/
```

### PackageImporter composition (runtime/importer/)

`PackageImporter` is composed from mixins in this order:
`(PaletteMixin, DecalsMixin, LayersMixin, MaterialsMixin,
BuildersMixin, GroupsMixin, OrchestrationMixin)`.

Orchestration owns `create_light`, interior placement, and the
top-level import loop. When adding a new per-entity behaviour, add it
as its own mixin rather than bloating orchestration.

## Coding Practices

See the project-wide `Coding Practices` section in
[../AGENTS.md](../AGENTS.md). Addon-specific reinforcements:

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
  mutates `bpy.data` in a way that another test might observe,
  unwind it or use `bpy.ops.wm.read_homefile(app_template="")`
  between fixtures.

## Deploy (rsync)

The installed copy must stay in lockstep with the source tree or the
MCP bridge will execute stale code. After every change:

```bash
rsync -a --delete StarBreaker/blender_addon/starbreaker_addon/ \
  ~/.config/blender/5.1/scripts/addons/starbreaker_addon/
```

`--delete` is important: it removes stray stale `.py` files (deleted
modules, renamed files) that would otherwise shadow the new code.

## Running the Tests

The suite stubs `bpy` so it runs on system Python, not inside Blender:

```bash
cd StarBreaker/blender_addon
python3 -m unittest discover -s tests -q
```

Baseline: **54 tests ran, 0 failures, 0 errors, 20 skipped**. Keep
this green after every change. Skipped tests require a real `bpy` and
only run under Blender — do not try to make them pass headless.

## Driving Blender from an Agent (MCP)

The `mcp_blendermcp_execute_blender_code` tool runs Python inside a
connected Blender instance. A few rules that have burned us:

### ALWAYS reset the scene this way

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

### Purge orphaned data between imports

**Always reset the scene before importing a ship** — even if it looks
empty. Leftover `SB_*` / `POM_*` / `StarBreaker*` node groups and
`__host_*` materials can silently poison the new import, and the only
reliable way to drop them is the scene reset:

```python
import bpy
bpy.ops.wm.read_homefile(app_template="")
```

Do NOT write by-hand cleanup loops (`bpy.data.node_groups.remove`,
`bpy.data.materials.remove`, selection-based deletes, etc.) — they
miss hidden users, leave orphaned drivers, and get out of sync with
new datablock categories. `read_homefile(app_template="")` is the
single source of truth for "start from a clean slate".

### Import a ship

```python
bpy.ops.starbreaker.import_decomposed_package(
    filepath="<path to a decomposed scene.json>",
)
```

Decomposed exports put the ship `scene.json` under a
`<output>/Packages/<name>/scene.json` layout. The CLI's `<output>`
argument is the **shared export root** (the parent that holds
`Packages/` and the deduplicated `Data/` tree), **not** the package
folder itself.

Canonical target for this workspace: pass `ships` as the output so
Aurora lands at `ships/Packages/RSI Aurora Mk2/scene.json` and all
shared meshes/textures/materials land in `ships/Data/...` for reuse
across ships. Do **not** pass `ships/Packages/RSI Aurora Mk2` — that
produces a double-nested `ships/Packages/RSI Aurora Mk2/Packages/RSI
Aurora Mk2/scene.json`.

```bash
SC_DATA_P4K="…/Data.p4k" \
  StarBreaker/target/release/starbreaker entity export "aurora_mk2" \
  "ships" --kind decomposed --lod 0 --mip 0 --materials all
```

Point the Blender import operator at the resulting `scene.json`, not
the outer folder.

### MCP output size

`execute_blender_code` will spill to a temp JSON file if stdout is
big. Keep probes targeted; filter with list comprehensions before
printing.

## Light Pipeline (current, post-Phase 28)

- Exporter emits `color`, `intensity` (cd), `temperature` (K),
  `use_temperature`, `radius`, `inner_angle`, `outer_angle`,
  `projector_texture`, `active_state`, `states` on every light.
- Addon stashes the full state map as JSON on the `bpy.types.Light`
  datablock: `starbreaker_light_states`,
  `starbreaker_light_active_state`.
- Energy conversion (runtime/constants.py):
  - Point / Spot / Area: `energy_W = intensity_cd * (4π/683) * LIGHT_VISUAL_GAIN`
    with `LIGHT_VISUAL_GAIN = 20.0`.
  - Sun: `energy_W_per_m2 = intensity / 683`.
  - Tune `LIGHT_VISUAL_GAIN` in `constants.py` if scenes are dim/bright.
- Runtime state switcher: `STARBREAKER_OT_switch_light_state` (N-panel
  buttons) calls `runtime.package_ops.apply_light_state(name)` which
  reapplies colour, temperature, and energy per light from the
  chosen state. Lights that lack that state keep their current values.

See the workspace-root `AGENTS.md` for the full light-schema notes and
per-phase history (stored outside this repo).

## Material Pipeline Notes

- **Glass** is rendered double-sided and uses a Light Path trick to
  stay visible through stacked interior+exterior panes (Phase 20, 26).
- **POM decals** gate host-material tinting: decals with POM height
  inherit the host palette; flat MeshDecals do not (Phases 10, 11, 16).
- **Shimmerscale paint** green channel is documented as engine-authored
  teal (Phase 12). Primary channel handling uses palette tint as the
  dominant colour (Phase 13).
- **Interior palette** routes through the exterior palette for
  specific ship parts (chairs) that DataCore marks as interior-paint
  targets (Phases 14, 21).
- POM node groups collapse to a small fixed set instead of one per
  texture (Phase 17).
- All imported meshes get a **Weighted Normal modifier** (Face Area,
  Weight=50, Threshold=0.01) to smooth shading across flat faces
  (Phase 19).

See `../docs/blender-material-contract-naming-rules.md` for the
in-repo material contract. Higher-level material research lives in
the workspace-root `AGENTS.md`.

## Phased Plan

The live plan lives in the workspace (outside this repo) — see the
workspace-root `AGENTS.md` for the path. Each phase has Context /
Acceptance / Steps sections and is marked `✅` when landed with a
commit hash. When starting a new phase, re-read the most recent
completed phase for conventions, then update the todo file as you go.
