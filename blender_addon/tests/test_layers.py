from __future__ import annotations

from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]

sys.path.insert(0, str(ADDON_ROOT))


if "starbreaker_addon" not in sys.modules:
    package = types.ModuleType("starbreaker_addon")
    package.__path__ = [str(ADDON_ROOT / "starbreaker_addon")]
    sys.modules["starbreaker_addon"] = package

if "starbreaker_addon.runtime" not in sys.modules:
    runtime_package = types.ModuleType("starbreaker_addon.runtime")
    runtime_package.__path__ = [str(ADDON_ROOT / "starbreaker_addon" / "runtime")]
    sys.modules["starbreaker_addon.runtime"] = runtime_package

if "mathutils" not in sys.modules:
    mathutils = types.ModuleType("mathutils")

    class Matrix(tuple):
        def __new__(cls, rows):
            return tuple.__new__(cls, rows)

        def inverted(self):
            return self

    class Quaternion(tuple):
        def __new__(cls, values):
            return tuple.__new__(cls, values)

    class Euler(tuple):
        def __new__(cls, values, order='XYZ'):
            return tuple.__new__(cls, values)

    mathutils.Matrix = Matrix
    mathutils.Quaternion = Quaternion
    mathutils.Euler = Euler
    sys.modules["mathutils"] = mathutils

if "bpy" not in sys.modules:
    bpy = types.ModuleType("bpy")
    bpy.types = types.SimpleNamespace(Nodes=object, NodeLinks=object, Node=object)
    sys.modules["bpy"] = bpy


from starbreaker_addon.runtime.importer.layers import _detail_strength_or_zero


class LayerDetailTests(unittest.TestCase):
    def test_missing_detail_mask_forces_neutral_strength(self) -> None:
        self.assertEqual(_detail_strength_or_zero(1.0, None), 0.0)
        self.assertEqual(_detail_strength_or_zero(0.296667, None), 0.0)

    def test_present_detail_mask_preserves_authored_strength(self) -> None:
        self.assertEqual(_detail_strength_or_zero(1.0, object()), 1.0)
        self.assertEqual(_detail_strength_or_zero(0.296667, object()), 0.296667)


if __name__ == "__main__":
    unittest.main()