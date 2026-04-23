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

### Purge orphaned shader / material data between imports

When re-importing the same ship to test a change, leftover
`SB_*` / `POM_*` / `StarBreaker*` node groups and `__host_*`
materials can silently poison the new import:

```python
for ng in [n for n in bpy.data.node_groups
           if n.name.startswith(("SB_", "POM_", "StarBreaker"))]:
    bpy.data.node_groups.remove(ng)
for m in [x for x in bpy.data.materials
          if x.users == 0 or "__host_" in x.name]:
    bpy.data.materials.remove(m)
```

### Import a ship

```python
bpy.ops.starbreaker.import_decomposed_package(
    filepath="/home/tom/projects/scorg_tools/ships/Packages/"
             "RSI Aurora Mk2/Packages/RSI Aurora Mk2/scene.json"
)
```

Note the nested `Packages/` in the path — decomposed exports put the
ship scene.json one level deeper than the export root.

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

See `../../docs/StarBreaker/lights-research.md` for the full schema
and per-phase history.

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

See `../docs/blender-material-contract-naming-rules.md` and
`../../docs/starbreaker-materials.md` for the material contract.

## Phased Plan

The live plan is `../../docs/StarBreaker/todo.md` (outside the git
repo). Each phase has Context / Acceptance / Steps sections and is
marked `✅` when landed with a commit hash. When starting a new phase,
re-read the most recent completed phase for conventions, then update
the todo file as you go.
