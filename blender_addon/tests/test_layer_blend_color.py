import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "starbreaker_addon/runtime/importer/layer_blend_color.py"
spec = importlib.util.spec_from_file_location("layer_blend_color", SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LayerBlendColorTests(unittest.TestCase):
    def test_named_sparse_base_and_wear_layers_are_routed_independently(self):
        layers = [SimpleNamespace(index=i, name=name) for i, name in enumerate(
            ("BaseLayer2", "BaseLayer3", "BaseLayer4", "WearLayer2", "WearLayer3", "WearLayer4"))]
        links = SimpleNamespace(new=lambda *args: None)
        image = SimpleNamespace(outputs=[object()], id_data=SimpleNamespace(links=links))
        separate = SimpleNamespace(inputs=[object()], outputs=[object()] * 3, id_data=image.id_data)
        nodes = SimpleNamespace(new=lambda kind: separate)
        importer = SimpleNamespace(_image_node=lambda *args, **kwargs: image, _apply_uv_tiling=lambda *args, **kwargs: None)
        record = SimpleNamespace(shader_family="LayerBlend_V2", layer_manifest=layers,
            texture_slots=[SimpleNamespace(role="blend_mask", export_path="mask")], public_params={})
        surface = SimpleNamespace(id_data=image.id_data, inputs={key: object() for key in ("Base Color", "Specular Tint", "Metallic")})
        with patch.object(module, "_blend_layers", return_value=((.1,) * 3, (.2,) * 3, 0)) as blend:
            self.assertTrue(module.apply_layer_blend_color(importer, nodes, record, None, .5, surface))
            self.assertEqual(blend.call_args_list[0].args[2], [None, *layers[:3]])
            self.assertEqual(blend.call_args_list[1].args[2], [None, *layers[3:]])

    def test_omitted_layer_preserves_remaining_mask_positions(self):
        importer = SimpleNamespace(_image_node=lambda *args, **kwargs: None)
        layers = [None] + [SimpleNamespace(layer_snapshot={"diffuse": [v] * 3},
            tint_color=None, palette_channel=None, diffuse_export_path=None) for v in (.2, .3, .4)]
        for mask, expected in (((0, 1, 0), .2), ((0, 0, 1), .3), ((0, 0, 0), .4), ((1, 0, 0), .4)):
            with self.subTest(mask=mask):
                color, _, _ = module._blend_layers(importer, None, layers, mask, None, 0)
                self.assertEqual(color, (expected,) * 3)

    def setUp(self):
        def color_mix(nodes, factor, first, second, *, multiply=False):
            return tuple(a * b if multiply else a * (1 - factor) + b * factor for a, b in zip(first, second))
        self.mix_patch = patch.object(module, "_color_mix", color_mix)
        self.float_patch = patch.object(module, "_float_mix", lambda nodes, f, a, b: a * (1 - f) + b * f)
        self.mix_patch.start()
        self.float_patch.start()
        self.addCleanup(self.mix_patch.stop)
        self.addCleanup(self.float_patch.stop)

    def test_palette_multiplies_authored_tint_only_on_routed_layer(self):
        importer = SimpleNamespace(_image_node=lambda *args, **kwargs: None,
            _palette_color_socket=lambda nodes, palette, channel, **kwargs: palette[channel])
        palette = {"primary": (.04, .05, .053), "secondary": (.112, .112, .107)}
        layer = SimpleNamespace(layer_snapshot={"diffuse": [1, 1, 1], "specular": [.04] * 3},
            tint_color=(.533, .533, .533), palette_channel=SimpleNamespace(name="secondary"), diffuse_export_path=None)
        color, _, _ = module._layer_color(importer, None, layer, palette, 0)
        self.assertEqual(color, tuple(.533 * v for v in palette["secondary"]))
        layer.palette_channel = None
        color, _, _ = module._layer_color(importer, None, layer, palette, 0)
        self.assertEqual(color, (.533, .533, .533))

    def test_each_blend_channel_preserves_its_layer_color_and_reflection(self):
        importer = SimpleNamespace(_image_node=lambda *args, **kwargs: None)
        layers = [SimpleNamespace(layer_snapshot={"diffuse": [v] * 3, "specular": [v / 2] * 3},
            tint_color=None, palette_channel=None, diffuse_export_path=None) for v in (.1, .2, .3, .4)]
        for mask, expected in (((1, 0, 0), .1), ((0, 1, 0), .2), ((0, 0, 1), .3), ((0, 0, 0), .4)):
            with self.subTest(mask=mask):
                color, specular, metallic = module._blend_layers(importer, None, layers, mask, None, 0)
                self.assertEqual(color, (expected,) * 3)
                self.assertAlmostEqual(specular[0] * module.PRINCIPLED_DIELECTRIC_F0, expected / 2)
                self.assertEqual(metallic, 0)

    def test_metal_base_color_uses_reflectance_not_black_diffuse(self):
        importer = SimpleNamespace(_image_node=lambda *args, **kwargs: None)
        layer = SimpleNamespace(layer_snapshot={"diffuse": [0] * 3, "specular": [.8, .5, .2], "metallic": 1},
            tint_color=None, palette_channel=None, diffuse_export_path=None)
        self.assertEqual(module._layer_color(importer, None, layer, None, 0), ((.8, .5, .2), (1, 1, 1), 1))

    def test_bronze_keeps_colored_reflections_without_name_heuristics(self):
        layer = SimpleNamespace(layer_snapshot={"diffuse": [.147] * 3, "specular": [.2705, .1912, .0931], "metallic": 0})
        diffuse, specular, metallic = module.layer_finish_values(layer)
        self.assertEqual(diffuse, (.147, .147, .147))
        self.assertEqual(specular, (.2705, .1912, .0931))
        self.assertEqual(metallic, 0)

    def test_white_specular_is_preserved_for_authored_metal(self):
        layer = SimpleNamespace(layer_snapshot={"diffuse": [0] * 3, "specular": [1] * 3, "metallic": 1})
        self.assertEqual(module.layer_finish_values(layer), ((0, 0, 0), (1, 1, 1), 1))

    def test_default_specular_tint_preserves_default_f0(self):
        self.assertAlmostEqual(module.PRINCIPLED_DIELECTRIC_F0, .04)
        for value in (.2705, .1912, .0931):
            self.assertAlmostEqual((value / module.PRINCIPLED_DIELECTRIC_F0) * .04, value)

    def test_missing_snapshot_uses_neutral_surface_defaults(self):
        layer = SimpleNamespace(layer_snapshot={})
        diffuse, specular, metallic = module.layer_finish_values(layer)
        self.assertEqual(diffuse, (1, 1, 1))
        self.assertEqual(specular, (.04, .04, .04))
        self.assertEqual(metallic, 0)


if __name__ == "__main__":
    unittest.main()
