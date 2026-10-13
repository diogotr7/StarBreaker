from pathlib import Path
import importlib.util
import math
from types import SimpleNamespace
import unittest


SOURCE = Path(__file__).resolve().parents[1] / "starbreaker_addon/runtime/importer/layer_blend_gloss.py"
spec = importlib.util.spec_from_file_location("layer_blend_gloss", SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Socket:
    def __init__(self, node=None):
        self.node = node
        self.default_value = 0.0
        self.source = None

    def value(self):
        if self.source is not None:
            return self.source.value()
        if self.node is None:
            return self.default_value
        n = self.node
        values = [s.value() for s in n.inputs]
        if n.operation == "MULTIPLY":
            result = values[0] * values[1]
        elif n.operation == "SUBTRACT":
            result = values[0] - values[1]
        elif n.operation == "SQRT":
            result = math.sqrt(values[0])
        elif n.operation == "MIX":
            result = values[2] * (1 - values[0]) + values[3] * values[0]
        else:
            raise AssertionError(n.operation)
        return max(0.0, min(1.0, result)) if n.use_clamp else result


class Nodes(list):
    def __init__(self):
        super().__init__()
        self.id_data = SimpleNamespace(links=SimpleNamespace(new=lambda a, b: setattr(b, "source", a)))

    def new(self, kind):
        n = SimpleNamespace(operation="MIX", use_clamp=False, inputs=[Socket() for _ in range(4)], id_data=self.id_data)
        n.outputs = [Socket(n)]
        if kind == "ShaderNodeSeparateColor":
            n.outputs = [Socket() for _ in range(3)]
            for socket, value in zip(n.outputs, self.mask):
                socket.default_value = value
        self.append(n)
        return n


class Importer:
    def __init__(self, mask):
        self.mask = mask
        self.tilings = []

    def _image_node(self, nodes, path, **kwargs):
        if not path:
            return None
        node = SimpleNamespace(outputs=[Socket(), Socket()], id_data=nodes.id_data)
        node.outputs[0].default_value = 0.2
        node.outputs[1].default_value = 0.96
        nodes.mask = self.mask
        return node

    def _apply_uv_tiling(self, nodes, links, image, scale, **kwargs):
        self.tilings.append(scale)

    def _invert_value_socket(self, nodes, source, **kwargs):
        invert = nodes.new("ShaderNodeMath")
        invert.operation = "SUBTRACT"
        invert.inputs[0].default_value = 1.0
        nodes.id_data.links.new(source, invert.inputs[1])
        root = nodes.new("ShaderNodeMath")
        root.operation = "SQRT"
        nodes.id_data.links.new(invert.outputs[0], root.inputs[0])
        return root.outputs[0]


def material():
    layers = [SimpleNamespace(index=i, name=f"{'Base' if i < 4 else 'Wear'}Layer{i % 4 + 1}", gloss_mult=g, uv_tiling=3.0,
        roughness_texture=SimpleNamespace(export_path="roughness", alpha_semantic=None),
        roughness_export_path="roughness", texture_slots=[], layer_snapshot={"shininess": 255.0})
        for i, g in enumerate((1.0, 0.49, 0.28, 0.55, 0.0, 0.0, 1.0, 1.0))]
    return SimpleNamespace(shader_family="LayerBlend_V2", layer_manifest=layers,
        texture_slots=[SimpleNamespace(role="blend_mask", export_path="mask")], public_params={})


class LayerBlendGlossTests(unittest.TestCase):
    def test_sparse_named_layers_do_not_promote_wear_into_base(self):
        record = material()
        record.layer_manifest = [layer for layer in record.layer_manifest if layer.name not in ("BaseLayer1", "WearLayer1")]
        for index, layer in enumerate(record.layer_manifest):
            layer.index = index
        for mask, gloss, wear_gloss in (((0, 0, 0), .55, 1), ((0, 1, 0), .49, 0), ((0, 0, 1), .28, 1)):
            with self.subTest(mask=mask):
                base, wear = module.build_layer_blend_roughness(Importer(mask), Nodes(), record)
                self.assertAlmostEqual(base.value(), math.sqrt(1 - .96 * gloss))
                self.assertIsNotNone(wear)
                self.assertAlmostEqual(wear.value(), math.sqrt(1 - .96 * wear_gloss))

    def test_authored_gloss_uses_each_blend_channel(self):
        for mask, gloss in (((1, 0, 0), 1), ((0, 1, 0), .49), ((0, 0, 1), .28), ((0, 0, 0), .55)):
            with self.subTest(mask=mask):
                importer = Importer(mask)
                base, wear = module.build_layer_blend_roughness(importer, Nodes(), material())
                self.assertAlmostEqual(base.value(), math.sqrt(1 - .96 * gloss))
                self.assertIn(3.0, importer.tilings)

    def test_red_has_priority_over_green_and_blue(self):
        base, _ = module.build_layer_blend_roughness(Importer((1, 1, 1)), Nodes(), material())
        self.assertAlmostEqual(base.value(), .2)

    def test_zero_wear_gloss_is_preserved(self):
        _, wear = module.build_layer_blend_roughness(Importer((0, 1, 0)), Nodes(), material())
        self.assertAlmostEqual(wear.value(), 1.0)

    def test_missing_multiplier_is_neutral(self):
        record = material()
        record.layer_manifest[2].gloss_mult = None
        base, _ = module.build_layer_blend_roughness(Importer((0, 0, 1)), Nodes(), record)
        self.assertAlmostEqual(base.value(), .2)

    def test_authored_shininess_modulates_texture(self):
        record = material()
        record.layer_manifest[0].layer_snapshot["shininess"] = 127.5
        base, _ = module.build_layer_blend_roughness(Importer((1, 0, 0)), Nodes(), record)
        self.assertAlmostEqual(base.value(), math.sqrt(1 - .96 * .5))

    def test_alpha_smoothness_and_derived_roughness_agree(self):
        record = material()
        for layer in record.layer_manifest:
            layer.roughness_texture.alpha_semantic = "smoothness"
        base, _ = module.build_layer_blend_roughness(Importer((0, 0, 1)), Nodes(), record)
        self.assertAlmostEqual(base.value(), math.sqrt(1 - .96 * .28))

    def test_fractional_masks_blend_in_smoothness_space(self):
        base, _ = module.build_layer_blend_roughness(Importer((.2, .4, .6)), Nodes(), material())
        gloss = ((.55 * .4 + .28 * .6) * .6 + .49 * .4) * .8 + 1.0 * .2
        self.assertAlmostEqual(base.value(), math.sqrt(1 - .96 * gloss))

    def test_other_families_keep_existing_path(self):
        record = material()
        record.shader_family = "Illum"
        self.assertIsNone(module.build_layer_blend_roughness(Importer((0, 0, 1)), Nodes(), record))

    def test_missing_mask_keeps_existing_path(self):
        record = material()
        record.texture_slots = []
        self.assertIsNone(module.build_layer_blend_roughness(Importer((0, 0, 1)), Nodes(), record))


if __name__ == "__main__":
    unittest.main()
