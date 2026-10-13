from pathlib import Path
import sys
import types
import unittest

ADDON_ROOT = Path(__file__).resolve().parents[1]
if "starbreaker_addon" not in sys.modules:
    package = types.ModuleType("starbreaker_addon")
    package.__path__ = [str(ADDON_ROOT / "starbreaker_addon")]
    sys.modules["starbreaker_addon"] = package

from starbreaker_addon.manifest import SubmaterialRecord
from starbreaker_addon.templates import representative_textures


def material(family="LayerBlend_V2", role="alternate_base_color", tokens=None):
    return SubmaterialRecord.from_value({
        "shader_family": family,
        "decoded_feature_flags": {"tokens": tokens or []},
        "texture_slots": [{"slot": "TexSlot9", "role": role, "export_path": "labels.png"}],
        "layer_manifest": [{"index": 0, "diffuse_export_path": "surface.png"}],
    })


class LayeredBaseColorTests(unittest.TestCase):
    def test_existing_decals_sidecar_does_not_use_atlas_as_base(self):
        self.assertEqual(representative_textures(material(tokens=["NORMAL_MAP", "BLEND_MAP", "DECALS"]))["base_color"], "surface.png")

    def test_classified_decal_sheet_is_not_a_layer_base(self):
        self.assertEqual(representative_textures(material(role="decal_sheet"))["base_color"], "surface.png")

    def test_explicit_surface_color_is_preserved(self):
        record = material(tokens=["DECALS"])
        record.texture_slots.append(type(record.texture_slots[0]).from_value({"slot": "TexSlot1", "role": "base_color", "export_path": "base.png"}))
        self.assertEqual(representative_textures(record)["base_color"], "base.png")

    def test_missing_layer_does_not_fall_back_to_atlas(self):
        record = material(tokens=["DECALS"])
        record.layer_manifest.clear()
        self.assertIsNone(representative_textures(record)["base_color"])

    def test_illum_alternate_color_is_unchanged(self):
        self.assertEqual(representative_textures(material("Illum"))["base_color"], "labels.png")

    def test_mesh_decal_keeps_its_sheet(self):
        self.assertEqual(representative_textures(material("MeshDecal", "decal_sheet"))["base_color"], "labels.png")


if __name__ == "__main__":
    unittest.main()
