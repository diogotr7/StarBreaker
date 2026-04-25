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


from starbreaker_addon.runtime.importer.layers import _detail_strength_or_zero, _stencil_override_selection


class LayerDetailTests(unittest.TestCase):
    def test_missing_detail_mask_forces_neutral_strength(self) -> None:
        self.assertEqual(_detail_strength_or_zero(1.0, None), 0.0)
        self.assertEqual(_detail_strength_or_zero(0.296667, None), 0.0)

    def test_present_detail_mask_preserves_authored_strength(self) -> None:
        self.assertEqual(_detail_strength_or_zero(1.0, object()), 1.0)
        self.assertEqual(_detail_strength_or_zero(0.296667, object()), 0.296667)

    def test_single_tint_override_selects_requested_slot(self) -> None:
        tint, specular, color_enable, tone_mode = _stencil_override_selection(
            2.0,
            is_virtual=False,
            tint_1=(1.0, 0.0, 0.0),
            tint_2=(0.0, 1.0, 0.0),
            tint_3=(0.0, 0.0, 1.0),
            specular_1=None,
            specular_2=(0.2, 0.2, 0.2),
            specular_3=None,
            stencil_glossiness=0.5,
        )
        self.assertEqual(tint, (0.0, 1.0, 0.0))
        self.assertEqual(specular, (0.2, 0.2, 0.2))
        self.assertEqual(color_enable, 1.0)
        self.assertEqual(tone_mode, 0.0)

    def test_neutral_non_virtual_override_can_disable_diffuse_color(self) -> None:
        tint, specular, color_enable, tone_mode = _stencil_override_selection(
            2.0,
            is_virtual=False,
            tint_1=(1.0, 1.0, 1.0),
            tint_2=(1.0, 1.0, 1.0),
            tint_3=(1.0, 1.0, 1.0),
            specular_1=None,
            specular_2=None,
            specular_3=None,
            stencil_glossiness=None,
        )
        self.assertEqual(tint, (1.0, 1.0, 1.0))
        self.assertIsNone(specular)
        self.assertEqual(color_enable, 0.0)
        self.assertEqual(tone_mode, 1.0)

    def test_virtual_override_keeps_diffuse_color_enabled(self) -> None:
        tint, specular, color_enable, tone_mode = _stencil_override_selection(
            2.0,
            is_virtual=True,
            tint_1=(1.0, 1.0, 1.0),
            tint_2=(1.0, 1.0, 1.0),
            tint_3=(1.0, 1.0, 1.0),
            specular_1=None,
            specular_2=None,
            specular_3=None,
            stencil_glossiness=None,
        )
        self.assertEqual(tint, (1.0, 1.0, 1.0))
        self.assertIsNone(specular)
        self.assertEqual(color_enable, 1.0)
        self.assertEqual(tone_mode, 0.0)


if __name__ == "__main__":
    unittest.main()