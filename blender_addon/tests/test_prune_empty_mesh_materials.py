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
