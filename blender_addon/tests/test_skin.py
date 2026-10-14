"""HumanSkin_V2 parameter resolution (``runtime/skin_utils.py``)."""

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

from starbreaker_addon.runtime.skin_utils import (  # noqa: E402
    chf_submaterial_overrides,
    human_skin_params,
    luminance,
    srgb_to_linear,
    tone_constants,
)
from test_templates import synthetic_submaterial  # noqa: E402

MTL = "Data/Objects/Characters/Human/heads/female/npc/female16/female16_t1_head_material.mtl"


def skin_submaterial(tokens: list[str] | None = None):
    sub = synthetic_submaterial("HumanSkin_V2", tokens=tokens or ["TONE_ADJUSTMENT", "WRINKLE_BLENDING_2"])
    return replace(
        sub,
        submaterial_name="shader_head",
        public_params={
            "SourceAverageColor": "0.6,0.35,0.25",
            "FinalSkinTone": "0.5,0.25,0.15",
            "SourceDetail": 1.6,
            "TranslucencyMultiplier": 0.8,
        },
        raw={
            "authored_attributes": [
                {"name": "Diffuse", "value": "1,1,1"},
                {"name": "Specular", "value": "0.03,0.03,0.03"},
                {"name": "Shininess", "value": "255"},
            ]
        },
    )


def character(mtl_path: str = MTL[5:]) -> dict:
    return {
        "customization": [
            {
                "name": "Head Material",
                "mtl_path": mtl_path,
                "submaterials": [
                    {
                        "name": "shader_head",
                        "colors": {"BodyColor": [0.8, 0.6, 0.5, 1.0]},
                        "params": {"FrecklesAmount": 0.4, "FrecklesOpacity": 0.3, "SunSpotsOpacity": 0.0},
                        "textures": {"TexSlot5": "Objects/x/mask.tif"},
                        "texture_exports": {"TexSlot5": "Data/x/mask_TEX0.png", "TexSlot10": "Data/x/id_TEX0.png"},
                    }
                ],
            }
        ],
        "wrinkles": {"slots": [{"panel": 0, "channel": 0, "band": 0, "tile": 1}, {"panel": 1, "channel": 3, "band": 2, "tile": 2}]},
    }


class ToneConstantTests(unittest.TestCase):
    def test_darker_target_raises_detail_and_lighter_lowers_it(self) -> None:
        source = (0.6, 0.35, 0.25)
        darker, _ = tone_constants(source, (0.2, 0.1, 0.05))
        lighter, _ = tone_constants(source, (0.9, 0.8, 0.7))
        self.assertGreater(darker, 1.0)
        self.assertLess(lighter, 1.0)
        self.assertAlmostEqual(darker, 1.0 + min(1.0, 2.0 * abs(luminance((0.2, 0.1, 0.05)) - luminance(source))))

    def test_identity_tone_is_neutral(self) -> None:
        self.assertEqual(tone_constants((0.6, 0.35, 0.25), (0.6, 0.35, 0.25)), (1.0, 0.0))

    def test_gb_desaturation_saturates_at_quarter_green_delta(self) -> None:
        _, t = tone_constants((0.6, 0.5, 0.25), (0.6, 0.2, 0.25))
        self.assertEqual(t, 1.0)


class SkinParamTests(unittest.TestCase):
    def test_mtl_constants_without_character(self) -> None:
        p = human_skin_params(skin_submaterial(), MTL, None)
        self.assertEqual(p.final_tone, (0.5, 0.25, 0.15))
        self.assertEqual(p.specular_color, (0.03, 0.03, 0.03))
        self.assertAlmostEqual(p.smoothness_scale, 1.0)
        self.assertAlmostEqual(p.translucency, 0.8)
        self.assertAlmostEqual(p.micro_detail_tiling, 1.0)  # shader default
        self.assertTrue(p.tone_adjustment and p.wrinkle_blending)
        self.assertFalse(p.blemishes)
        self.assertEqual(p.wrinkle_slots, [])

    def test_chf_body_color_becomes_linear_final_skin_tone(self) -> None:
        p = human_skin_params(skin_submaterial(), MTL, character())
        self.assertEqual(p.final_tone, tuple(srgb_to_linear(c) for c in (0.8, 0.6, 0.5)))

    def test_chf_blemishes_and_wrinkle_layout(self) -> None:
        p = human_skin_params(skin_submaterial(), MTL, character())
        self.assertTrue(p.blemishes)
        self.assertAlmostEqual(p.freckles_amount, 0.4)
        self.assertEqual(p.override_textures["TexSlot10"], "Data/x/id_TEX0.png")
        self.assertEqual([(s.panel, s.channel, s.band, s.tile) for s in p.wrinkle_slots], [(0, 0, 0, 1), (1, 3, 2, 2)])

    def test_override_requires_matching_material(self) -> None:
        self.assertIsNone(chf_submaterial_overrides(character("Objects/other.mtl"), MTL, "shader_head"))
        self.assertIsNone(chf_submaterial_overrides(character(), MTL, "shader_body"))
        p = human_skin_params(skin_submaterial(), MTL, character("Objects/other.mtl"))
        self.assertEqual(p.final_tone, (0.5, 0.25, 0.15))

    def test_no_wrinkle_slots_without_wrinkle_token(self) -> None:
        p = human_skin_params(skin_submaterial(["TONE_ADJUSTMENT"]), MTL, character())
        self.assertEqual(p.wrinkle_slots, [])

    def test_srgb_to_linear_endpoints(self) -> None:
        self.assertEqual(srgb_to_linear(0.0), 0.0)
        self.assertAlmostEqual(srgb_to_linear(1.0), 1.0)
        self.assertAlmostEqual(srgb_to_linear(0.5), 0.2140, places=4)


if __name__ == "__main__":
    unittest.main()
