"""HairPBR ``%HAIR_CAP`` parameter resolution (``runtime/hair_utils.py``)."""

from __future__ import annotations

import sys
import types
import unittest
from dataclasses import replace
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

for name, path in (
    ("starbreaker_addon", ADDON_ROOT / "starbreaker_addon"),
    ("starbreaker_addon.runtime", ADDON_ROOT / "starbreaker_addon" / "runtime"),
):
    if name not in sys.modules:
        module = types.ModuleType(name)
        module.__path__ = [str(path)]
        sys.modules[name] = module

from starbreaker_addon.runtime.hair_utils import (  # noqa: E402
    hair_lobes,
    hair_cap_params,
    hair_color,
    hair_params,
    is_hair_cap,
    is_hair_fibre,
    melanin_coefficients,
)
from test_templates import synthetic_submaterial  # noqa: E402


def cap(public_params: dict | None = None, diffuse: str = "1,1,1"):
    sub = synthetic_submaterial("HairPBR", tokens=["HAIR_CAP"])
    return replace(
        sub,
        public_params=public_params if public_params is not None else {"BaseTintColor": "0,0,0"},
        raw={"authored_attributes": [{"name": "Diffuse", "value": diffuse}]},
    )


class HairCapTests(unittest.TestCase):
    def test_only_hair_cap_submaterials_qualify(self) -> None:
        self.assertTrue(is_hair_cap(cap()))
        self.assertFalse(is_hair_cap(synthetic_submaterial("HairPBR", tokens=["HAIR_COAT"])))
        self.assertFalse(is_hair_cap(synthetic_submaterial("Illum", tokens=["HAIR_CAP"])))

    def test_black_tint_gives_clamped_near_black_colour(self) -> None:
        self.assertEqual(hair_cap_params(cap()).color_scale, (0.0, 0.0, 0.0))
        self.assertEqual(hair_cap_params(cap()).color_min, 0.0001)

    def test_tint_and_diffuse_multiply(self) -> None:
        p = hair_cap_params(cap({"BaseTintColor": "0.5,1,1"}, diffuse="1,0.5,1"))
        self.assertEqual(p.color_scale, (0.5, 0.5, 1.0))

    def test_grazing_fade_exponent(self) -> None:
        self.assertEqual(hair_cap_params(cap()).grazing_exponent, 16.0)


MTL = "Data/Objects/hair/f_hair_36.mtl"


def cards(public_params: dict | None = None, tokens=("HAIR_CARDS", "CARD_ID_MAP", "DIRECTION_MAP")):
    sub = synthetic_submaterial("HairPBR", tokens=list(tokens))
    return replace(sub, public_params=public_params or {}, raw={"authored_attributes": [{"name": "Diffuse", "value": "1,1,1"}]})


def character(index: int = 0) -> dict:
    return {
        "customization": [
            {
                "name": "HairDyeMaterial",
                "mtl_path": None,
                "applies_to": {"mtl_path": "Objects/hair/f_hair_36.mtl", "submaterial_index": index},
                "submaterials": [
                    {
                        "name": "hair_36_m",
                        "colors": {"DyeColor": [1.0, 0.5, 0.0, 1.0], "BaseTintColor": [0.5, 0.5, 0.5, 1.0]},
                        "params": {"BaseMelanin": 0.2, "DyeAmount": 0.7},
                    }
                ],
            }
        ]
    }


class HairColourTests(unittest.TestCase):
    def test_get_hair_color_matches_shader_reference(self) -> None:
        # GetHairColor(0.58, 0.14) by hand from HairPBR.cfxb: q = -ln(1 - 0.58^2)
        # = 0.41008, denom(0.3) = 5.88841, k_r = sqrt(0.86*0.5028 + 0.14*0.3390)
        # * denom = 4.0792 -> exp(-sqrt(q) * k_r) = 0.07337; k_g = 5.4346 -> 0.03080.
        r, g, b = hair_color(0.58, 0.14)
        self.assertAlmostEqual(r, 0.07337, places=4)
        self.assertAlmostEqual(g, 0.03080, places=4)
        self.assertLess(b, g)

    def test_zero_melanin_is_white(self) -> None:
        self.assertEqual(hair_color(0.0, 0.5), (1.0, 1.0, 1.0))

    def test_coefficients_redness_shifts_towards_pheomelanin(self) -> None:
        eu, pheo = melanin_coefficients(0.0), melanin_coefficients(1.0)
        self.assertLess(eu[2] / eu[0], pheo[2] / pheo[0])


class HairLobeTests(unittest.TestCase):
    """HairShading lobes at peak alignment: integral of Hair_g over theta = pi * e^Dp."""

    def lobes(self):
        return {lobe.name: lobe for lobe in hair_lobes()}

    def test_r_lobe_is_colourless_fresnel_reflection(self) -> None:
        r = self.lobes()["R"]
        self.assertAlmostEqual(r.energy, 0.03534, places=4)  # pi * (0.2 + 0.05) * 0.045
        self.assertEqual((r.tint, r.transmission, r.roughness_scale), ("none", False, 1.0))
        self.assertAlmostEqual(r.offset, -2 * 0.0436332, places=5)  # -2 * 2.5 deg

    def test_tt_lobe_is_backlit_sqrt_tinted_transmission(self) -> None:
        tt = self.lobes()["TT"]
        self.assertAlmostEqual(tt.energy, 0.2060, places=3)  # pi * e^-0.33 * 0.912 * 0.1
        self.assertEqual((tt.tint, tt.transmission, tt.roughness_scale), ("sqrt", True, 0.5))

    def test_trt_lobe_is_tinted_secondary_highlight(self) -> None:
        trt = self.lobes()["TRT"]
        self.assertAlmostEqual(trt.energy, 0.1607, places=3)  # pi * e^0.22 * 1.0 * 0.912 * 0.045
        self.assertEqual((trt.tint, trt.transmission, trt.roughness_scale), ("color", False, 2.0))
        self.assertAlmostEqual(trt.offset, 4 * 0.0436332, places=5)


class HairParamTests(unittest.TestCase):
    def test_fibre_families(self) -> None:
        self.assertTrue(is_hair_fibre(cards()))
        self.assertTrue(is_hair_fibre(cards(tokens=("HAIR_COAT",))))
        self.assertFalse(is_hair_fibre(cap()))

    def test_shader_defaults(self) -> None:
        p = hair_params(cards(), MTL, None)
        self.assertEqual((p.melanin, p.melanin_redness, p.melanin_variation), (0.5, 0.5, 0.25))
        self.assertEqual((p.smoothness, p.specular_variation, p.ambient_occlusion), (0.65, 0.25, 2.0))
        self.assertEqual((p.dye_amount, p.tint), (0.0, (1.0, 1.0, 1.0)))
        self.assertTrue(p.cards and p.id_map)

    def test_mtl_values(self) -> None:
        p = hair_params(cards({"BaseMelanin": 0.58, "DyeColor": "0.4,0.3,0.2", "DyeAmount": 1}), MTL, None)
        self.assertEqual((p.melanin, p.dye_color, p.dye_amount), (0.58, (0.4, 0.3, 0.2), 1.0))

    def test_chf_dye_overrides_bound_submaterial(self) -> None:
        p = hair_params(cards(), MTL, character())
        self.assertEqual(p.melanin, 0.2)
        self.assertEqual(p.dye_amount, 0.7)
        self.assertAlmostEqual(p.dye_color[1], 0.2140, places=4)  # sRGB 0.5 -> linear
        self.assertAlmostEqual(p.tint[0], 0.2140, places=4)

    def test_chf_override_ignores_other_submaterials_and_materials(self) -> None:
        self.assertEqual(hair_params(cards(), MTL, character(index=1)).melanin, 0.5)
        self.assertEqual(hair_params(cards(), "Data/Objects/hair/other.mtl", character()).melanin, 0.5)

    def test_coat_has_no_id_map(self) -> None:
        p = hair_params(cards({"OpacityMapGamma": 1.5}, tokens=("HAIR_COAT", "DIRECTION_MAP")), MTL, None)
        self.assertFalse(p.cards or p.id_map)
        self.assertEqual(p.opacity_gamma, 1.5)


if __name__ == "__main__":
    unittest.main()
