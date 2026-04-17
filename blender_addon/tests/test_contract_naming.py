from __future__ import annotations

from pathlib import Path
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.contract_naming import (
    palette_input_name,
    public_param_input_name,
    texture_input_name,
    texture_input_semantic,
    virtual_input_name,
)


class ContractNamingTests(unittest.TestCase):
    def test_texture_input_names_preserve_slot_prefix(self) -> None:
        self.assertEqual(texture_input_name("HardSurface", "TexSlot1"), "TexSlot1_BaseColor")
        self.assertEqual(texture_input_name("LayerBlend_V2", "TexSlot13"), "TexSlot13_HalControl")
        self.assertEqual(texture_input_name("DisplayScreen", "TexSlot15"), "TexSlot15_CondensationNormal")
        self.assertEqual(texture_input_name("HumanSkin_V2", "TexSlot11"), "TexSlot11_WrinkleMask")

    def test_texture_input_semantics_follow_rules(self) -> None:
        self.assertEqual(texture_input_semantic("MeshDecal", "TexSlot2"), "specular")
        self.assertEqual(texture_input_semantic("UIPlane", "TexSlot17"), "pixel_layout")
        self.assertEqual(texture_input_semantic("HairPBR", "TexSlot4"), "id_map")
        self.assertEqual(texture_input_semantic("Illum", "TexSlot17"), "subsurface_mask")
        self.assertEqual(texture_input_semantic("Monitor", "TexSlot1"), "base_color")
        self.assertIsNone(texture_input_semantic("UIMesh", "TexSlot1"))

    def test_non_texture_input_names_preserve_source_identity(self) -> None:
        self.assertEqual(public_param_input_name("FarGlowStartDistance"), "Param_FarGlowStartDistance")
        self.assertEqual(palette_input_name("primary"), "Palette_Primary")
        self.assertEqual(virtual_input_name("$RenderToTexture"), "Virtual_RenderToTexture")


if __name__ == "__main__":
    unittest.main()
