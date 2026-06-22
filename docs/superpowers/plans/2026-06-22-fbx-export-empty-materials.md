# FBX-Export Empty-Material Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a directly-opened decomposed `scene.blend` be exported to FBX without the Blender FBX exporter crashing on materials whose `node_tree is None`.

**Architecture:** Two independent, addon-side fixes. (a) The load-post material refresh's "can refresh" gate is taught to match an *unnamed* submaterial (name `None`/empty, e.g. a `UIPlane` screen) by its submaterial **index** so the refresh stops skipping screen objects like `screen_16x9_a`, and builds their node tree. (b) The "Make Instances Real" operator gains a structural cleanup pass that detaches material slots from geometry-less meshes (0 faces, e.g. `*_SeatAccess_*`/`*_seat_access_*` interaction proxies) whose placeholder materials have no node tree. Both fixes target the Blender addon only; the Rust exporter is unchanged.

**Tech Stack:** Python 3.13 (Blender 5.1 addon), `unittest` (stubs `bpy`, runs on system `python3`), Blender 5.1.x for headless end-to-end verification.

## Global Constraints

- **No hard-coding / no name-matching.** Both fixes must key on structural properties (submaterial index, polygon count), never on a specific object/material/ship name, texture path, or suffix. (StarBreaker `AGENTS.md`, `blender_addon/AGENTS.md`.)
- **Fix the root cause, not the symptom.** No `try/except: pass`, no clamp floors, no fallback default values copied from game data.
- **TDD.** Write the failing test first, watch it fail, implement, watch it pass, commit. Pure-Python tests (no live `bpy`) live in `blender_addon/tests/` and run on system `python3`.
- **Keep the addon test suite green.** Baseline before any change: `360 tests, OK (skipped=38)` via `cd blender_addon && python3 -m unittest discover -s tests -q`. (The "165 tests" figure in `blender_addon/AGENTS.md` is stale — do not trust it; trust the live run.)
- **Deploy before manual testing.** The live Blender install must be re-synced from source or it runs stale code:
  `rsync -a --delete blender_addon/starbreaker_addon/ ~/.config/blender/5.1/scripts/addons/starbreaker_addon/`
- **No maintainer name / no `/home/<user>` paths** in repo content. Runnable command examples use `"$HOME/..."`.
- **Run the addon test suite from `blender_addon/`:** `cd blender_addon && python3 -m unittest discover -s tests -q`.

## Background (verified during investigation)

- Opening `scene.blend` directly yields **1038 materials, all `node_tree is None`** — the exporter writes name-only material stubs (`build_material_with_node_tree_and_properties(name, 0, 0)` at `blend_assembly.rs:2016/3502/3726`); shader graphs are built by the addon at load, not baked into the blend.
- Blender's FBX exporter wraps **every** material on a selected object with `node_shader_utils.PrincipledBSDFWrapper`, which unconditionally does `tree = material.node_tree; nodes = tree.nodes` (`export_fbx_bin.py:2934` → `node_shader_utils.py:169`). A single `node_tree is None` raises `AttributeError: 'NoneType' object has no attribute 'nodes'` and aborts the whole export.
- The addon's load-post auto-refresh rebuilds **426 of 429** mesh materials. The **3 survivors** that still crash FBX:
  - `screen_16x9_a`, `screen_16x9_a_001` — real geometry (16 verts / 18 faces), sidecar `Data/materials/ui/rtt_comms_opaque_hightech_TEX0.materials.json` whose single submaterial has **`name: None`, `index: 0`, `shader: "UIPlane"`**. The exported material name `rtt_comms_opaque_hightech_mtl__00` canonicalises to `""`, so the refresh gate `_material_slot_can_refresh` rejects it (`if not canonical_name: return False`). → **Fix (a)**.
  - `RSI_Zeus_SeatAccess_Pilot_LOD0`, `rsi_aurora_mk2_component_seat_access_LOD0` — **0 verts / 0 faces**, `material_sidecar: null`; nothing to build from and nothing to shade. → **Fix (b)**.
- Validated end-to-end in headless Blender: forcing the screens through `refresh_materials_for_package_root(..., target_objects=[screens])` builds a 5-node UIPlane tree; detaching materials from the 2 face-less meshes; then `export_scene.fbx(use_selection=True)` **succeeds** with **0 residual empty materials**.

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `blender_addon/starbreaker_addon/runtime/package_ops.py` | Material refresh + match gate | Modify `_material_slot_can_refresh` to fall back to index-match for unnamed submaterials |
| `blender_addon/tests/test_package_ops.py` | Unit tests for `package_ops` (already stubs `bpy` via `_load_package_ops`) | Add tests for the index fallback |
| `blender_addon/starbreaker_addon/ui.py` | N-panel + operators (incl. `STARBREAKER_OT_make_instances_real`, `_make_package_*` helpers) | Add `_prune_empty_mesh_material_slots` helper; wire it into the operator |
| `blender_addon/tests/test_prune_empty_mesh_materials.py` | Unit tests for the prune helper (AST-extraction pattern) | Create |

These two fixes are independent and independently landable; Task 3 verifies them together because they share the same end-to-end repro.

---

### Task 1: Fix (a) — refresh gate matches unnamed submaterials by index

**Files:**
- Modify: `blender_addon/starbreaker_addon/runtime/package_ops.py:680-700` (`_material_slot_can_refresh`)
- Test: `blender_addon/tests/test_package_ops.py` (add methods to the existing `PackageOpsTests` class)

**Interfaces:**
- Consumes (already exist in `package_ops.py`): `_slot_mapping_for_object(obj) -> list[int|None]|None`, `_sidecar_has_submaterial_index(sidecar, index) -> bool`, `_canonical_source_name(name) -> str`, `_unique_submaterials_by_name(sidecar)`, `_submaterials_by_name(sidecar)`, `_object_needs_material_refresh(obj, sidecar) -> bool`, `_material_slot_needs_refresh(material) -> bool`.
- Test fakes already in `test_package_ops.py`: `FakeObject` (dict subclass; `type`, `material_slots`, `children_recursive`; has **no** `.data` attribute, so `getattr(obj, "data", None)` is `None`), `FakeMaterial` (`name`, `library=None`, `node_tree=None`), `FakeSlot(material)`.
- Produces: unchanged public signature `_material_slot_can_refresh(obj, slot_index, material, sidecar) -> bool`; only its return for the empty-canonical-name case changes from constant `False` to an index lookup.

- [ ] **Step 1: Write the failing tests**

Add these two methods inside the existing `class PackageOpsTests(unittest.TestCase):` in `blender_addon/tests/test_package_ops.py` (it already exposes `self.package_ops`, and `FakeObject`/`FakeMaterial`/`FakeSlot`/`types` are module-level):

```python
    def test_unnamed_submaterial_is_refreshable_by_index(self) -> None:
        """A UIPlane-style submaterial with no name (e.g. screen_16x9_a) must
        still be matched to its sidecar entry by slot index so the load-post
        refresh rebuilds it instead of skipping it."""
        po = self.package_ops
        sidecar = types.SimpleNamespace(submaterials=[types.SimpleNamespace(index=0)])
        material = FakeMaterial("rtt_comms_opaque_hightech_mtl__00")  # node_tree=None
        obj = FakeObject("screen_16x9_a")
        obj.type = "MESH"
        obj.material_slots = [FakeSlot(material)]

        # Precondition: the exported name canonicalises to empty.
        self.assertEqual(
            po._canonical_source_name("rtt_comms_opaque_hightech_mtl__00"), ""
        )
        self.assertTrue(po._material_slot_can_refresh(obj, 0, material, sidecar))
        self.assertTrue(po._object_needs_material_refresh(obj, sidecar))

    def test_unnamed_submaterial_without_matching_index_is_not_refreshable(self) -> None:
        """The index fallback must not green-light a slot whose index has no
        submaterial in the sidecar."""
        po = self.package_ops
        sidecar = types.SimpleNamespace(submaterials=[types.SimpleNamespace(index=0)])
        material = FakeMaterial("rtt_comms_opaque_hightech_mtl__00")
        obj = FakeObject("screen_16x9_a")
        obj.type = "MESH"
        obj.material_slots = [FakeSlot(material)]

        # Slot index 1 has no submaterial in the sidecar -> not refreshable.
        self.assertFalse(po._material_slot_can_refresh(obj, 1, material, sidecar))
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd blender_addon && python3 -m unittest tests.test_package_ops.PackageOpsTests.test_unnamed_submaterial_is_refreshable_by_index -v`
Expected: FAIL — `AssertionError: False is not true` (current gate returns `False` for an empty canonical name).

- [ ] **Step 3: Implement the index fallback**

In `blender_addon/starbreaker_addon/runtime/package_ops.py`, change the empty-canonical-name early return inside `_material_slot_can_refresh`.

Replace:

```python
    canonical_name = _canonical_source_name(material_name)
    if not canonical_name:
        return False
    if canonical_name in _unique_submaterials_by_name(sidecar):
        return True
    return canonical_name in _submaterials_by_name(sidecar)
```

with:

```python
    canonical_name = _canonical_source_name(material_name)
    if not canonical_name:
        # An unnamed submaterial (e.g. a UIPlane screen) exports to a material
        # whose name canonicalises to "". It cannot be matched by name, so fall
        # back to the structural 1:1 slot->submaterial index used by the
        # exporter. This only fires for slots the caller already flagged as
        # needing a refresh (node_tree None/empty or linked).
        return _sidecar_has_submaterial_index(sidecar, slot_index)
    if canonical_name in _unique_submaterials_by_name(sidecar):
        return True
    return canonical_name in _submaterials_by_name(sidecar)
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd blender_addon && python3 -m unittest tests.test_package_ops -v 2>&1 | tail -5`
Expected: both new tests PASS, and the rest of `test_package_ops` stays green.

- [ ] **Step 5: Run the full addon suite (no regressions)**

Run: `cd blender_addon && python3 -m unittest discover -s tests -q 2>&1 | tail -3`
Expected: `OK (skipped=38)` with the total count increased by 2 (362 tests).

- [ ] **Step 6: Commit**

```bash
cd /home/tom/projects/scorg_tools/StarBreaker
git add blender_addon/starbreaker_addon/runtime/package_ops.py blender_addon/tests/test_package_ops.py
git commit -m "fix(addon): match unnamed UIPlane submaterials by index in material refresh

Screen objects (UIPlane) export with an unnamed submaterial whose name
canonicalises to empty, so the load-post refresh skipped them and left
their material node_tree None, crashing FBX export. Fall back to the
exporter's 1:1 slot->submaterial index when the name is empty."
```

---

### Task 2: Fix (b) — prune material slots on geometry-less meshes

**Files:**
- Modify: `blender_addon/starbreaker_addon/ui.py` — add `_prune_empty_mesh_material_slots` after `_make_package_linked_object_data_local` (which ends at line 850), and call it from `STARBREAKER_OT_make_instances_real.execute` (lines 1318-1345).
- Create: `blender_addon/tests/test_prune_empty_mesh_materials.py`

**Interfaces:**
- Produces: `_prune_empty_mesh_material_slots(package_root) -> int` (count of material slots detached). Used by `STARBREAKER_OT_make_instances_real.execute`.
- Consumes: nothing new; mirrors the traversal style of `_collection_instance_objects` (`[package_root, *children_recursive]`).

- [ ] **Step 1: Write the failing test**

Create `blender_addon/tests/test_prune_empty_mesh_materials.py`:

```python
from __future__ import annotations

import ast
import types
import unittest
from pathlib import Path


ADDON_ROOT = Path(__file__).resolve().parents[1]


def _load_ui_functions(*names: str):
    """Extract named top-level functions from ui.py and exec them against a
    minimal bpy stub (mirrors tests/test_make_instances_real.py)."""
    ui_path = ADDON_ROOT / "starbreaker_addon" / "ui.py"
    source = ui_path.read_text(encoding="utf-8")
    tree = ast.parse(source)
    namespace: dict = {
        "bpy": types.SimpleNamespace(
            types=types.SimpleNamespace(Context=object, Object=object),
        ),
    }
    pending = set(names)
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name in pending:
            func_source = ast.get_source_segment(source, node)
            if func_source:
                exec(compile(ast.parse(func_source), str(ui_path), "exec"), namespace)  # noqa: S102
                pending.remove(node.name)
                if not pending:
                    break
    return tuple(namespace[name] for name in names)


class _FakeSlot:
    def __init__(self, material):
        self.material = material


class _FakeMesh:
    def __init__(self, npolys: int):
        self.polygons = list(range(npolys))


class _FakeObject:
    def __init__(self, name: str, *, obj_type: str = "MESH", npolys: int = 0, material=None):
        self.name = name
        self.type = obj_type
        self.data = _FakeMesh(npolys) if obj_type == "MESH" else None
        self.material_slots = [_FakeSlot(material)] if material is not None else []
        self.children: list["_FakeObject"] = []

    @property
    def children_recursive(self):
        result = []
        stack = list(self.children)
        while stack:
            child = stack.pop()
            result.append(child)
            stack.extend(child.children)
        return result


class TestPruneEmptyMeshMaterials(unittest.TestCase):
    def test_detaches_material_from_faceless_mesh_only(self) -> None:
        (prune,) = _load_ui_functions("_prune_empty_mesh_material_slots")
        root = _FakeObject("root", npolys=0, material=None)
        faceless = _FakeObject("seat_access", npolys=0, material="mat_empty")
        real = _FakeObject("hull", npolys=18, material="mat_hull")
        empty_obj = _FakeObject("locator", obj_type="EMPTY")
        root.children = [faceless, real, empty_obj]

        cleared = prune(root)

        self.assertEqual(cleared, 1)
        self.assertIsNone(faceless.material_slots[0].material)
        self.assertEqual(real.material_slots[0].material, "mat_hull")

    def test_counts_every_detached_slot(self) -> None:
        (prune,) = _load_ui_functions("_prune_empty_mesh_material_slots")
        root = _FakeObject("root", npolys=0, material=None)
        a = _FakeObject("a", npolys=0, material="m")
        b = _FakeObject("b", npolys=0, material="m")
        root.children = [a, b]

        self.assertEqual(prune(root), 2)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd blender_addon && python3 -m unittest tests.test_prune_empty_mesh_materials -v`
Expected: FAIL — `KeyError: '_prune_empty_mesh_material_slots'` (function does not exist yet).

- [ ] **Step 3: Implement `_prune_empty_mesh_material_slots`**

In `blender_addon/starbreaker_addon/ui.py`, insert this function immediately after `_make_package_linked_object_data_local` (after its `return localized_count` at line 850, before `def _selected_package`):

```python
def _prune_empty_mesh_material_slots(package_root: bpy.types.Object) -> int:
    """Detach material slots from geometry-less meshes under ``package_root``.

    A mesh with zero polygons (e.g. a seat-access interaction proxy the
    exporter writes with no geometry) shades nothing, yet it still carries a
    placeholder material with no node tree. Blender's FBX exporter wraps every
    material slot on a selected object and crashes on ``node_tree is None``.
    Detaching the material is a structural cleanup keyed on geometry (polygon
    count), not on any asset name, so it generalises to every package.
    """
    cleared = 0
    candidates = [package_root, *list(getattr(package_root, "children_recursive", ()))]
    for obj in candidates:
        if getattr(obj, "type", None) != "MESH":
            continue
        data = getattr(obj, "data", None)
        polygons = getattr(data, "polygons", None) if data is not None else None
        if polygons is None or len(polygons) != 0:
            continue
        for slot in getattr(obj, "material_slots", ()):
            if getattr(slot, "material", None) is not None:
                slot.material = None
                cleared += 1
    return cleared
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd blender_addon && python3 -m unittest tests.test_prune_empty_mesh_materials -v`
Expected: both tests PASS.

- [ ] **Step 5: Wire the prune into the operator**

In `blender_addon/starbreaker_addon/ui.py`, in `STARBREAKER_OT_make_instances_real.execute`, replace this block:

```python
        realized_instances = _make_package_collection_instances_real(context, package_root)
        localized_data = _make_package_linked_object_data_local(package_root)
        if realized_instances == 0 and localized_data == 0:
            self.report(
                {"INFO"},
                "No collection instances or linked object data found under the selected StarBreaker package",
            )
            return {"FINISHED"}

        _focus_loaded_package_root(context, package_root)
        self.report(
            {"INFO"},
            (
                f"Made {realized_instances} instance container(s) real and "
                f"localized {localized_data} linked datablock(s)"
            ),
        )
        return {"FINISHED"}
```

with:

```python
        realized_instances = _make_package_collection_instances_real(context, package_root)
        localized_data = _make_package_linked_object_data_local(package_root)
        pruned_slots = _prune_empty_mesh_material_slots(package_root)
        if realized_instances == 0 and localized_data == 0 and pruned_slots == 0:
            self.report(
                {"INFO"},
                "No collection instances, linked object data, or empty-mesh materials found under the selected StarBreaker package",
            )
            return {"FINISHED"}

        _focus_loaded_package_root(context, package_root)
        self.report(
            {"INFO"},
            (
                f"Made {realized_instances} instance container(s) real, "
                f"localized {localized_data} linked datablock(s), and "
                f"pruned {pruned_slots} empty-mesh material slot(s)"
            ),
        )
        return {"FINISHED"}
```

- [ ] **Step 6: Run the full addon suite (no regressions)**

Run: `cd blender_addon && python3 -m unittest discover -s tests -q 2>&1 | tail -3`
Expected: `OK (skipped=38)`, total count increased by 2 over Task 1's result (364 tests).

- [ ] **Step 7: Commit**

```bash
cd /home/tom/projects/scorg_tools/StarBreaker
git add blender_addon/starbreaker_addon/ui.py blender_addon/tests/test_prune_empty_mesh_materials.py
git commit -m "fix(addon): prune material slots on geometry-less meshes in Make Instances Real

Face-less proxy meshes (e.g. seat-access volumes) carry placeholder
materials with no node tree and have no sidecar to rebuild from. Detach
their material slots during Make Instances Real so the FBX exporter no
longer crashes wrapping a node_tree-None material."
```

---

### Task 3: Deploy + headless end-to-end verification

**Files:**
- Create (temporary, not committed): `verify_fbx_export.py` at the workspace root.

**Interfaces:**
- Consumes: the deployed addon (Task 1 + Task 2), a decomposed Aurora export at `"$HOME/projects/scorg_tools/ships/Packages/RSI Aurora Mk2_LOD0_TEX0/scene.blend"`. If that export is absent, regenerate it per `blender_addon/AGENTS.md` ("Import a ship") before verifying.

- [ ] **Step 1: Deploy the addon to the live Blender install**

```bash
cd /home/tom/projects/scorg_tools/StarBreaker
rsync -a --delete blender_addon/starbreaker_addon/ ~/.config/blender/5.1/scripts/addons/starbreaker_addon/
```

- [ ] **Step 2: Write the headless verification script**

Create `/home/tom/projects/scorg_tools/verify_fbx_export.py`:

```python
"""End-to-end: open scene.blend, run the real load-post refresh + Make
Instances Real, select all, export FBX. Exits non-zero on any failure."""
import os
import sys
import bpy

BLEND = os.path.expanduser(
    "~/projects/scorg_tools/ships/Packages/RSI Aurora Mk2_LOD0_TEX0/scene.blend"
)
FBX_OUT = "/tmp/verify_aurora.fbx"


def main() -> int:
    bpy.ops.preferences.addon_enable(module="starbreaker_addon")
    bpy.ops.wm.open_mainfile(filepath=BLEND)

    # Fix (a): the load-post auto-refresh (timer fires in the GUI; drive it
    # directly here because --background has no event loop).
    import starbreaker_addon.ui as ui
    ui._material_refresh_prompt_timer(None)

    # Fix (b): Make Instances Real prunes empty-mesh material slots.
    from starbreaker_addon.runtime.package_ops import find_package_root
    root = next((find_package_root(o) for o in bpy.data.objects if find_package_root(o)), None)
    assert root is not None, "no StarBreaker package root found"
    bpy.ops.object.select_all(action="DESELECT")
    root.select_set(True)
    bpy.context.view_layer.objects.active = root
    bpy.ops.starbreaker.make_instances_real()

    # Residual check: no selected mesh may carry a node_tree-None material.
    bpy.ops.object.select_all(action="SELECT")
    residual = [
        (o.name, s.material.name)
        for o in bpy.context.selected_objects
        if o.type == "MESH"
        for s in o.material_slots
        if s.material is not None and s.material.node_tree is None
    ]
    print("residual node_tree-None materials on selected meshes:", len(residual), residual[:5], flush=True)
    if residual:
        return 1

    try:
        bpy.ops.export_scene.fbx(filepath=FBX_OUT, use_selection=True)
    except Exception as exc:  # noqa: BLE001 - verification surface
        print("FBX FAILED ->", str(exc).strip().splitlines()[-1], flush=True)
        return 1
    print("FBX OK ->", FBX_OUT, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 3: Run the verification in headless Blender**

Run:
```bash
cd /home/tom/projects/scorg_tools
blender --background --python verify_fbx_export.py 2>&1 | grep -E "residual|FBX (OK|FAILED)"
```
Expected output:
```
residual node_tree-None materials on selected meshes: 0 []
FBX OK -> /tmp/verify_aurora.fbx
```

- [ ] **Step 4: Confirm the FBX was written**

Run: `ls -la /tmp/verify_aurora.fbx`
Expected: a non-empty `.fbx` file.

- [ ] **Step 5: Clean up the temporary artifacts**

```bash
rm -f /home/tom/projects/scorg_tools/verify_fbx_export.py /tmp/verify_aurora.fbx
```

- [ ] **Step 6 (optional, owner-driven): GUI smoke test**

In the actual Blender GUI: open the same `scene.blend`, let the materials finish loading, click **Make Instances Real**, Select All (`A`), then **File ▸ Export ▸ FBX** with *Selected Objects* checked. Confirm the export completes with no Python error popup.

---

## Self-Review

**1. Spec coverage.**
- (a) "make the UI system match the missing `screen_16x9_a` objects" → Task 1 makes the refresh gate match the unnamed UIPlane submaterial by index, so the load-post refresh stops skipping the screens and builds their node tree. ✓
- (b) "pruning the empty mesh materials" → Task 2 detaches material slots from 0-polygon meshes inside Make Instances Real. ✓
- End-to-end proof the FBX export now succeeds → Task 3. ✓

**2. Placeholder scan.** No TBD/TODO/"handle edge cases"/"similar to Task N". Every code and test step shows complete code and an exact command with expected output. ✓

**3. Type consistency.** `_material_slot_can_refresh(obj, slot_index, material, sidecar) -> bool` and `_sidecar_has_submaterial_index(sidecar, index) -> bool` are used with their real signatures. `_prune_empty_mesh_material_slots(package_root) -> int` is defined in Task 2 and called with one positional arg in the operator. Test fakes match the attributes the functions read (`type`, `data.polygons`, `material_slots[*].material`, `children_recursive`; `FakeObject` has no `.data`, exercising the `getattr(obj, "data", None) is None` path in `_slot_mapping_for_object`). ✓

**Known limitation (documented, not a gap):** Fix (a) runs inside the load-post auto-refresh (automatic in the GUI; gated by the `auto_refresh_unloaded_materials_on_load` preference). Fix (b) runs inside **Make Instances Real**, matching the user's stated workflow. A user who disables the auto-refresh *or* skips Make Instances Real before exporting would still hit an empty material. If broader robustness is wanted later, `_prune_empty_mesh_material_slots` and a screen refresh could also be invoked from the load-post handler — out of scope for this plan.
