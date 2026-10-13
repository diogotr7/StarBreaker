import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'starbreaker_addon/runtime/importer/layer_blend_decals.py'
spec = importlib.util.spec_from_file_location('layer_blend_decals', SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def material(family='LayerBlend_V2', tokens=('DECALS',), path='decal.png'):
    return SimpleNamespace(shader_family=family, decoded_feature_flags=SimpleNamespace(tokens=tokens),
        texture_slots=[SimpleNamespace(slot='TexSlot9', export_path=path)])


class LayerBlendDecalTests(unittest.TestCase):
    def test_extracted_vertex_bgra_preserves_native_coordinates(self):
        rgba = (160 / 255, 37 / 255, 252 / 255, 1)
        self.assertEqual(module.decode_decal_coordinates(rgba), (2562 / 4095 * 4 - .5, 1532 / 4095 * 4 - .5, 0))

    def test_full_packed_range_and_axis_order(self):
        for u, v in ((0, 0), (4095, 4095), (256, 3072), (3072, 256)):
            packed = (u << 12) | v
            rgba = ((packed >> 16) / 255, ((packed >> 8) & 255) / 255, (packed & 255) / 255, 1)
            self.assertEqual(module.decode_decal_coordinates(rgba), (u / 4095 * 4 - .5, v / 4095 * 4 - .5, 0))

    def test_alpha_is_not_part_of_decal_coordinates(self):
        self.assertEqual(module.decode_decal_coordinates((.4, .5, .6, 0)), module.decode_decal_coordinates((.4, .5, .6, 1)))

    def test_decals_feature_enables_exported_slot(self):
        self.assertEqual(module.diffuse_decal_texture(material()).export_path, 'decal.png')

    def test_other_shader_and_vertex_wear_mode_are_not_decoded(self):
        for record in (material(family='Illum'), material(tokens=()), material(tokens=('DECALS', 'WEAR_DIRT_AO_VERTEX')), material(path=None)):
            self.assertIsNone(module.diffuse_decal_texture(record))

    def test_unrelated_mesh_is_untouched(self):
        self.assertFalse(module.prepare_decal_coordinates(SimpleNamespace(), SimpleNamespace(submaterials=[material(family='Illum')])))

    def test_missing_color_does_not_invent_decal_coordinates(self):
        obj = SimpleNamespace(data=SimpleNamespace(color_attributes={}))
        self.assertFalse(module.prepare_decal_coordinates(obj, SimpleNamespace(submaterials=[material()])))


if __name__ == '__main__':
    unittest.main()
