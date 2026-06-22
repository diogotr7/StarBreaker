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
    bpy.data = types.SimpleNamespace(node_groups=[], images=[], materials=None)
    sys.modules["bpy"] = bpy

if "numpy" not in sys.modules:
    numpy = types.ModuleType("numpy")
    numpy.float32 = float
    numpy.empty = lambda length, dtype=None: [0.0] * length
    sys.modules["numpy"] = numpy


from starbreaker_addon.runtime.importer.builders import (
    _clamp_unit_float,
    _layered_wear_base_layer,
    _layered_wear_base_palette_fallback,
    _layered_wear_first_diffuse_layer,
    _layered_wear_metallic_values,
    _layered_wear_first_non_neutral_tint,
    _layered_wear_uses_neutral_synthetic_palette_base,
)
from starbreaker_addon.runtime.importer.layers import _detail_strength_or_zero, _stencil_override_selection
from starbreaker_addon.runtime.importer.materials import _texture_reference_uses_packed_roughness_green


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

    def test_layered_wear_base_layer_returns_none_for_empty_manifest(self) -> None:
        submaterial = types.SimpleNamespace(layer_manifest=[])
        self.assertIsNone(_layered_wear_base_layer(submaterial))

    def test_layered_wear_base_layer_uses_first_manifest_entry(self) -> None:
        first = object()
        second = object()
        submaterial = types.SimpleNamespace(layer_manifest=[first, second])
        self.assertIs(_layered_wear_base_layer(submaterial), first)

    def test_layered_wear_base_layer_uses_single_layer_manifest_entry(self) -> None:
        only = object()
        submaterial = types.SimpleNamespace(layer_manifest=[only])
        self.assertIs(_layered_wear_base_layer(submaterial), only)

    def test_layered_wear_first_diffuse_layer_returns_first_with_diffuse(self) -> None:
        layer_a = types.SimpleNamespace(diffuse_export_path=None)
        layer_b = types.SimpleNamespace(diffuse_export_path="Data/foo.dds")
        layer_c = types.SimpleNamespace(diffuse_export_path="Data/bar.dds")
        submaterial = types.SimpleNamespace(layer_manifest=[layer_a, layer_b, layer_c])
        self.assertIs(_layered_wear_first_diffuse_layer(submaterial), layer_b)

    def test_layered_wear_first_non_neutral_tint_prefers_first_non_white(self) -> None:
        layer_a = types.SimpleNamespace(tint_color=(1.0, 1.0, 1.0))
        layer_b = types.SimpleNamespace(tint_color=(0.3, 0.4, 0.5))
        layer_c = types.SimpleNamespace(tint_color=(0.8, 0.7, 0.6))
        submaterial = types.SimpleNamespace(layer_manifest=[layer_a, layer_b, layer_c])
        self.assertEqual(
            _layered_wear_first_non_neutral_tint(submaterial),
            (0.3, 0.4, 0.5),
        )

    def test_layered_wear_first_non_neutral_tint_returns_none_when_neutral(self) -> None:
        layer_a = types.SimpleNamespace(tint_color=(1.0, 1.0, 1.0))
        layer_b = types.SimpleNamespace(tint_color=None)
        submaterial = types.SimpleNamespace(layer_manifest=[layer_a, layer_b])
        self.assertIsNone(_layered_wear_first_non_neutral_tint(submaterial))

    def test_layered_wear_neutral_synthetic_palette_base_stays_white(self) -> None:
        layer = types.SimpleNamespace(
            palette_channel=types.SimpleNamespace(name="tertiary"),
            tint_color=(1.0, 1.0, 1.0),
            source_material_path="Data/Materials/Layers/synthetic/weapon_paint_01.mtl",
        )

        self.assertTrue(_layered_wear_uses_neutral_synthetic_palette_base(layer))

    def test_layered_wear_non_neutral_synthetic_palette_base_keeps_diffuse(self) -> None:
        layer = types.SimpleNamespace(
            palette_channel=types.SimpleNamespace(name="secondary"),
            tint_color=(0.7, 0.7, 0.7),
            source_material_path="Data/Materials/Layers/synthetic/weapon_paint_01.mtl",
        )

        self.assertFalse(_layered_wear_uses_neutral_synthetic_palette_base(layer))

    def test_layered_wear_non_synthetic_palette_base_keeps_diffuse(self) -> None:
        layer = types.SimpleNamespace(
            palette_channel=types.SimpleNamespace(name="secondary"),
            tint_color=(1.0, 1.0, 1.0),
            source_material_path="Data/Materials/Layers/weapons/weapon_metal_01.mtl",
        )

        self.assertFalse(_layered_wear_uses_neutral_synthetic_palette_base(layer))

    def test_clamp_unit_float(self) -> None:
        self.assertEqual(_clamp_unit_float(-0.5), 0.0)
        self.assertEqual(_clamp_unit_float(0.25), 0.25)
        self.assertEqual(_clamp_unit_float(2.0), 1.0)

    def test_layered_wear_metallic_values_uses_both_layers(self) -> None:
        base_layer = types.SimpleNamespace(layer_snapshot={"metallic": 0.8})
        wear_layer = types.SimpleNamespace(layer_snapshot={"metallic": 0.2})
        self.assertEqual(
            _layered_wear_metallic_values(base_layer, wear_layer),
            (0.8, 0.2),
        )

    def test_layered_wear_metallic_values_falls_back_to_present_layer(self) -> None:
        wear_layer = types.SimpleNamespace(layer_snapshot={"metallic": 1.5})
        self.assertEqual(
            _layered_wear_metallic_values(None, wear_layer),
            (1.0, 1.0),
        )

    def test_layered_wear_base_palette_fallback_uses_specular_for_metal(self) -> None:
        base_layer = types.SimpleNamespace(
            tint_color=(1.0, 1.0, 1.0),
            layer_snapshot={
                "metallic": 1.0,
                "diffuse": [0.0065, 0.0065, 0.0065],
                "specular": [0.3712, 0.2195, 0.1170],
            },
        )
        submaterial = types.SimpleNamespace(layer_manifest=[base_layer])

        self.assertEqual(
            _layered_wear_base_palette_fallback(submaterial, base_layer),
            (0.3712, 0.2195, 0.1170),
        )

    def test_layered_wear_base_palette_fallback_keeps_non_neutral_tint_for_dielectric(self) -> None:
        base_layer = types.SimpleNamespace(
            tint_color=(0.2, 0.3, 0.4),
            layer_snapshot={
                "metallic": 0.0,
                "specular": [0.8, 0.6, 0.4],
            },
        )
        submaterial = types.SimpleNamespace(layer_manifest=[base_layer])

        self.assertEqual(
            _layered_wear_base_palette_fallback(submaterial, base_layer),
            (0.2, 0.3, 0.4),
        )

    def test_packed_mr_roughness_reference_uses_green_channel(self) -> None:
        roughness_texture = types.SimpleNamespace(
            role="roughness",
            packed_texture_format="gltf_metallic_roughness",
            value_channel="g",
        )
        smoothness_texture = types.SimpleNamespace(
            role="normal_gloss",
            packed_texture_format=None,
            value_channel=None,
        )
        grayscale_roughness = types.SimpleNamespace(
            role="roughness",
            packed_texture_format="roughness_grayscale",
            value_channel="r",
        )

        self.assertTrue(_texture_reference_uses_packed_roughness_green(roughness_texture))
        self.assertFalse(_texture_reference_uses_packed_roughness_green(smoothness_texture))
        self.assertFalse(_texture_reference_uses_packed_roughness_green(grayscale_roughness))


if __name__ == "__main__":
    unittest.main()
