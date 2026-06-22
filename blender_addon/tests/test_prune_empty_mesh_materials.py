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


class _FakeMaterial:
    def __init__(self, name: str, *, node_tree=None):
        self.name = name
        self.node_tree = node_tree


class _FakeMesh:
    def __init__(self, npolys: int):
        self.polygons = list(range(npolys))


class _FakeMeshNoPolygons:
    """Mesh data without a ``polygons`` attribute at all."""


class _FakeObject:
    def __init__(
        self,
        name: str,
        *,
        obj_type: str = "MESH",
        npolys: int = 0,
        material=None,
        materials=None,
        data=None,
    ):
        self.name = name
        self.type = obj_type
        if data is not None:
            self.data = data
        else:
            self.data = _FakeMesh(npolys) if obj_type == "MESH" else None
        if materials is not None:
            self.material_slots = [_FakeSlot(mat) for mat in materials]
        elif material is not None:
            self.material_slots = [_FakeSlot(material)]
        else:
            self.material_slots = []
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

    def test_skips_mesh_data_without_polygons_attribute(self) -> None:
        (prune,) = _load_ui_functions("_prune_empty_mesh_material_slots")
        root = _FakeObject("root", npolys=0, material=None)
        # A MESH whose data carries no ``polygons`` attribute at all
        # (getattr(..., "polygons", None) is None) must be skipped, not pruned.
        no_polys = _FakeObject(
            "no_polys", data=_FakeMeshNoPolygons(), material="mat_keep"
        )
        root.children = [no_polys]

        self.assertEqual(prune(root), 0)
        self.assertEqual(no_polys.material_slots[0].material, "mat_keep")

    def test_detaches_only_non_none_slots_and_counts_them(self) -> None:
        (prune,) = _load_ui_functions("_prune_empty_mesh_material_slots")
        root = _FakeObject("root", npolys=0, material=None)
        # A faceless mesh with several slots, only some of which carry a
        # material: the helper must detach exactly the non-None slots and
        # return that count.
        mixed = _FakeObject(
            "mixed", npolys=0, materials=["m0", None, "m2", None]
        )
        root.children = [mixed]

        cleared = prune(root)

        self.assertEqual(cleared, 2)
        self.assertIsNone(mixed.material_slots[0].material)
        self.assertIsNone(mixed.material_slots[1].material)
        self.assertIsNone(mixed.material_slots[2].material)
        self.assertIsNone(mixed.material_slots[3].material)


class TestDetachUnresolvedEmptyMaterials(unittest.TestCase):
    def test_detaches_empty_node_tree_material_on_face_mesh(self) -> None:
        (detach,) = _load_ui_functions("_detach_unresolved_empty_materials")
        root = _FakeObject("root", npolys=0, material=None)
        # A face-bearing mesh whose material never got a node tree (the case the
        # targeted fixes did not catch) must be detached and reported.
        broken = _FakeObject(
            "panel", npolys=12, material=_FakeMaterial("mtl_broken", node_tree=None)
        )
        good = _FakeObject(
            "hull", npolys=18, material=_FakeMaterial("mtl_ok", node_tree=object())
        )
        root.children = [broken, good]

        detached = detach(root)

        self.assertEqual(detached, ["panel / mtl_broken"])
        self.assertIsNone(broken.material_slots[0].material)
        self.assertEqual(good.material_slots[0].material.name, "mtl_ok")

    def test_returns_empty_list_when_all_materials_resolved(self) -> None:
        (detach,) = _load_ui_functions("_detach_unresolved_empty_materials")
        root = _FakeObject("root", npolys=0, material=None)
        good = _FakeObject(
            "hull", npolys=18, material=_FakeMaterial("mtl_ok", node_tree=object())
        )
        root.children = [good]

        self.assertEqual(detach(root), [])
        self.assertEqual(good.material_slots[0].material.name, "mtl_ok")

    def test_skips_non_mesh_and_none_slots(self) -> None:
        (detach,) = _load_ui_functions("_detach_unresolved_empty_materials")
        root = _FakeObject("root", npolys=0, material=None)
        empty_obj = _FakeObject("locator", obj_type="EMPTY")
        multi = _FakeObject(
            "multi", npolys=4, materials=[None, _FakeMaterial("x", node_tree=None)]
        )
        root.children = [empty_obj, multi]

        detached = detach(root)

        self.assertEqual(detached, ["multi / x"])
        self.assertIsNone(multi.material_slots[1].material)


if __name__ == "__main__":
    unittest.main()
