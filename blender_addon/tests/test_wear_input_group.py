"""The wear input group's Wear Factor matches the engine's wear alpha.

Runs the add-on's real `StarBreaker Wear Input` group in headless Blender
(skipped when Blender is not installed): a plane with a `Color` attribute of
alpha 0.4 emits the group's Wear Factor, and a tiny Cycles render of it is
compared with a render of a plain emission of `wear_utils`' reference value
(the same colour management applies to both).
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
BLENDER = Path(os.environ.get("BLENDER_BIN", r"C:\Program Files\Blender Foundation\Blender 5.1\blender.exe"))

VERTEX_ALPHA = 0.4
# (blend map, falloff, WearBlendBase, instance wear, use vertex colours, remap start)
CASES = [
    (0.5, 0.5, 0.0, 0.0, 0.0, 0.0),
    (0.5, 1.0, 0.25, 0.0, 0.0, 0.0),
    (0.3, 0.01, 0.18, 0.0, 0.0, 0.0),
    (0.7, 0.5, 0.18, 0.9, 1.0, 0.0),
    (0.2, 0.8, 0.6, 0.0, 1.0, 0.0),
    (0.1, 0.5, 1.0, 0.0, 0.0, 0.5),
    (0.6, 0.3, 0.9, 0.0, 0.0, 0.25),
]

SCRIPT = textwrap.dedent(
    """
    import bpy, json, sys
    sys.path.insert(0, {addon!r})
    from starbreaker_addon.runtime.importer import PackageImporter
    from starbreaker_addon.runtime import wear_utils
    bpy.ops.wm.read_factory_settings(use_empty=True)
    importer = object.__new__(PackageImporter)
    group = importer._ensure_runtime_wear_input_group()
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 1
    scene.render.resolution_x = scene.render.resolution_y = 4
    scene.view_settings.view_transform = "Standard"
    scene.render.image_settings.file_format = "OPEN_EXR"
    bpy.ops.mesh.primitive_plane_add(size=2)
    plane = bpy.context.object
    colors = plane.data.color_attributes.new("Color", "BYTE_COLOR", "CORNER")
    for item in colors.data:
        item.color = (1.0, 1.0, 1.0, {alpha!r})
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scene.collection.objects.link(cam)
    cam.location = (0, 0, 3)
    scene.camera = cam
    def render():
        path = bpy.app.tempdir + "wear.exr"
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(path)
        value = img.pixels[0]
        bpy.data.images.remove(img)
        return value
    def emit(socket_or_value):
        mat = bpy.data.materials.new("m")
        mat.use_nodes = True
        nt = mat.node_tree
        nt.nodes.clear()
        e = nt.nodes.new("ShaderNodeEmission")
        o = nt.nodes.new("ShaderNodeOutputMaterial")
        nt.links.new(e.outputs[0], o.inputs[0])
        if isinstance(socket_or_value, float):
            v = socket_or_value
            e.inputs["Color"].default_value = (v, v, v, 1.0)
        else:
            g = nt.nodes.new("ShaderNodeGroup")
            g.node_tree = group
            for name, value in socket_or_value.items():
                g.inputs[name].default_value = (value, value, value, 1.0) if name == "Blend Map" else value
            nt.links.new(g.outputs["Wear Factor"], e.inputs["Color"])
        plane.data.materials.clear()
        plane.data.materials.append(mat)
        return render()
    out = []
    for blend, falloff, base, inst, use_vc, remap in {cases!r}:
        got = emit({{
            "Blend Map": blend, "Blend Falloff": falloff, "Wear Blend Base": base,
            "Instance Wear": inst, "Use Vertex Colors": use_vc, "Remap Start": remap,
        }})
        amount = wear_utils.wear_amount(base, inst, {alpha!r} if use_vc else 0.0)
        expected_value = wear_utils.remap_wear(wear_utils.engine_wear_alpha(blend, amount, falloff), remap)
        out.append([got, emit(float(expected_value)), expected_value])
    print("SB_RESULT", json.dumps(out))
    """
)


@unittest.skipUnless(BLENDER.exists(), "needs Blender 5.1 (BLENDER_BIN or the default install)")
class WearInputGroupTests(unittest.TestCase):
    def test_wear_factor_matches_engine_wear_alpha(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "wear.py"
            script.write_text(
                SCRIPT.format(addon=str(ADDON_ROOT), cases=CASES, alpha=VERTEX_ALPHA), encoding="utf-8"
            )
            run = subprocess.run(
                [str(BLENDER), "-b", "--factory-startup", "--python", str(script)],
                capture_output=True, text=True, timeout=300,
            )
        line = next((l for l in run.stdout.splitlines() if l.startswith("SB_RESULT")), None)
        self.assertIsNotNone(line, run.stdout[-2000:] + run.stderr[-2000:])
        got = json.loads(line[len("SB_RESULT"):])
        for case, (value, expected, raw) in zip(CASES, got):
            self.assertAlmostEqual(value, expected, places=3, msg=f"case={case} engine={raw}; all={got}")


if __name__ == "__main__":
    unittest.main()
