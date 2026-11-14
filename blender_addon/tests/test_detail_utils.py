"""Engine detail-map gloss and normals (reference formulas).

Star Engine: `common.cfib` `GetDetailMapGrad` / `UnpackDetailMap` (the texel
is read `.garb`; normal x/y = min(2 * G|A - 256/255, 1), gloss = 2B - 1),
`layerLib.cfib` `LayerAttribs_ReadDetail` / `LayerAttribs_DetailOverlay`
(smoothness += smoothness * gloss * DetailGloss; normal x/y * DetailBump),
`NormalsCommon.cfib` `ReconstructNormalZ` / `AddDetailNormals`.
"""

from __future__ import annotations

import importlib.util
import math
import sys
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
_SPEC = importlib.util.spec_from_file_location(
    "sb_detail_utils", ADDON_ROOT / "starbreaker_addon" / "runtime" / "detail_utils.py"
)
detail_utils = importlib.util.module_from_spec(_SPEC)
sys.modules["sb_detail_utils"] = detail_utils
_SPEC.loader.exec_module(detail_utils)


class DetailSmoothnessTests(unittest.TestCase):
    def test_mid_grey_gloss_keeps_smoothness(self) -> None:
        self.assertAlmostEqual(detail_utils.detail_smoothness(0.6, 0.5, 1.0), 0.6, places=6)

    def test_gloss_scales_smoothness(self) -> None:
        # 0.6 * (1 + (2 * 0.75 - 1) * 0.5) = 0.75
        self.assertAlmostEqual(detail_utils.detail_smoothness(0.6, 0.75, 0.5), 0.75, places=6)
        self.assertAlmostEqual(detail_utils.detail_smoothness(0.6, 0.0, 1.0), 0.0, places=6)

    def test_smoothness_is_clamped(self) -> None:
        self.assertEqual(detail_utils.detail_smoothness(0.9, 1.0, 1.0), 1.0)


class DetailNormalTests(unittest.TestCase):
    def test_unpack_uses_the_128_midpoint(self) -> None:
        x, y, z = detail_utils.detail_normal(128 / 255, 128 / 255, 1.0)
        self.assertAlmostEqual(x, 0.0, places=6)
        self.assertAlmostEqual(y, 0.0, places=6)
        self.assertAlmostEqual(z, 1.0, places=6)

    def test_unpack_scales_by_bump_and_reconstructs_z(self) -> None:
        x, y, z = detail_utils.detail_normal(1.0, 0.0, 0.5)
        self.assertAlmostEqual(x, (2.0 - 256 / 255) * 0.5, places=6)
        self.assertAlmostEqual(y, (0.0 - 256 / 255) * 0.5, places=6)
        self.assertAlmostEqual(z, math.sqrt(1.0 - x * x - y * y), places=6)

    def test_flat_detail_keeps_the_base_normal(self) -> None:
        base = (0.3, -0.2, math.sqrt(1 - 0.13))
        out = detail_utils.add_detail_normals(base, (0.0, 0.0, 1.0))
        for got, want in zip(out, base):
            self.assertAlmostEqual(got, want, places=6)

    def test_flat_base_takes_the_detail_normal(self) -> None:
        detail = (0.4, 0.1, math.sqrt(1 - 0.17))
        out = detail_utils.add_detail_normals((0.0, 0.0, 1.0), detail)
        for got, want in zip(out, detail):
            self.assertAlmostEqual(got, want, places=6)


if __name__ == "__main__":
    unittest.main()
