"""Engine wear parameters and the reference wear-alpha formula.

Star Engine (`HardSurface.cfxb` `GetLayerAttribs` / `CalcAlphas_BlendBased`,
`layerLib.cfib` `CalcBlendAlpha` / `Layer_RemapWear`, `Layer.cfxb`):
wearAmt = saturate(max(WearBlendBase, instWear) - 0.95 * vertex alpha), the
primary layer's TexSlot8 blend map thresholds it with the layer's
WearDirtBlendFalloff softness, and Paint / Cloth layers remap the result.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
_SPEC = importlib.util.spec_from_file_location(
    "sb_wear_utils", ADDON_ROOT / "starbreaker_addon" / "runtime" / "wear_utils.py"
)
wear_utils = importlib.util.module_from_spec(_SPEC)
sys.modules["sb_wear_utils"] = wear_utils
_SPEC.loader.exec_module(wear_utils)


class _Layer:
    def __init__(self, params: dict[str, str], slots: list[dict], uv_tiling=None):
        self.resolved_material = {
            "authored_public_params": [{"name": k, "value": v} for k, v in params.items()]
        }
        self.texture_slots = [type("T", (), s)() for s in slots]
        self.uv_tiling = uv_tiling


BLEND = {"slot": "TexSlot8", "export_path": "Data/Textures/x_blend_TEX0.png"}


class EngineWearAlphaTests(unittest.TestCase):
    def test_no_wear_amount_means_no_wear(self) -> None:
        for blend in (0.0, 0.5, 1.0):
            for softness in (0.01, 0.5, 1.0):
                self.assertAlmostEqual(wear_utils.engine_wear_alpha(blend, 0.0, softness), 0.0, places=6)

    def test_soft_blend_below_half_amount(self) -> None:
        # softness 1 -> InAlpha = softAlpha = b' * 0.5 + 0.5; OutAlpha = 1.
        # b' = 0.5 * (1 - 1/255 - 0.001) + 1/255 + 0.001 = 0.5024608
        self.assertAlmostEqual(wear_utils.engine_wear_alpha(0.5, 0.25, 1.0), 0.2487696, places=6)

    def test_full_amount_wears_everything_but_white(self) -> None:
        # amount 1: OutAlpha = blend^2, so blend 0 is fully worn.
        self.assertAlmostEqual(wear_utils.engine_wear_alpha(0.0, 1.0, 0.5), 1.0, places=6)

    def test_paint_remap_starts_at_primer(self) -> None:
        self.assertAlmostEqual(wear_utils.remap_wear(0.5, 0.5), 0.0, places=6)
        self.assertAlmostEqual(wear_utils.remap_wear(0.75, 0.5), 0.5, places=6)
        self.assertAlmostEqual(wear_utils.remap_wear(0.3, 0.0), 0.3, places=6)

    def test_wear_amount_subtracts_vertex_alpha(self) -> None:
        self.assertAlmostEqual(wear_utils.wear_amount(0.18, 0.0, 0.1), 0.18 - 0.095, places=6)
        self.assertAlmostEqual(wear_utils.wear_amount(0.18, 0.6, 0.1), 0.6 - 0.095, places=6)
        self.assertEqual(wear_utils.wear_amount(0.18, 0.0, 1.0), 0.0)


class PrimaryLayerWearParamsTests(unittest.TestCase):
    def test_reads_falloff_tiling_and_blend_map(self) -> None:
        layer = _Layer({"WearDirtBlendFalloff": "0.0099999998", "ShadingModel": "0"}, [BLEND], uv_tiling=4.0)
        params = wear_utils.primary_layer_wear_params(layer)
        self.assertEqual(params.blend_map_path, BLEND["export_path"])
        self.assertAlmostEqual(params.falloff, 0.0099999998)
        self.assertEqual(params.tiling, 4.0)
        self.assertEqual(params.remap_start, 0.0)

    def test_missing_params_use_layer_shader_defaults(self) -> None:
        params = wear_utils.primary_layer_wear_params(_Layer({}, [BLEND]))
        self.assertEqual(params.falloff, 0.5)
        self.assertEqual(params.tiling, 1.0)
        self.assertEqual(params.remap_start, 0.0)

    def test_paint_and_cloth_remap_starts(self) -> None:
        for model, start in (("2", 0.5), ("3", 0.5), ("4", 0.25), ("1", 0.0)):
            params = wear_utils.primary_layer_wear_params(_Layer({"ShadingModel": model}, [BLEND]))
            self.assertEqual(params.remap_start, start, model)

    def test_no_blend_map_means_no_params(self) -> None:
        self.assertIsNone(wear_utils.primary_layer_wear_params(_Layer({}, [])))
        self.assertIsNone(wear_utils.primary_layer_wear_params(None))


if __name__ == "__main__":
    unittest.main()
