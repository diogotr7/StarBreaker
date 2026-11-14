"""The layer surface group's detail gloss and detail normals match the engine.

Runs the add-on's real `StarBreaker Runtime LayerSurface` group in headless
Blender (skipped when Blender is not installed) and compares tiny Cycles
renders with plain emissions of `detail_utils`' reference values:

- Roughness: the group's roughness, with the detail gloss applied in
  smoothness space (the add-on maps smoothness to roughness as sqrt(1 - s)).
- Normal: on a +Z plane with axis-aligned UVs the tangent frame is the world
  frame, so the emitted world normal is the tangent-space result.
"""

from __future__ import annotations

import json
import math
import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
BLENDER = Path(os.environ.get("BLENDER_BIN", r"C:\Program Files\Blender Foundation\Blender 5.1\blender.exe"))

# (roughness source, detail B, DetailGloss)
GLOSS_CASES = [(0.6, 0.5, 1.0), (0.6, 0.75, 0.5), (0.3, 0.2, 1.0), (0.8, 1.0, 0.8), (0.5, 0.9, 0.0)]


def _encode(n):
    return [c * 0.5 + 0.5 for c in n]


_TILT = (0.3, -0.2, math.sqrt(1 - 0.13))
# (base normal colour, detail G, detail A, DetailBump)
NORMAL_CASES = [
    (_encode((0.0, 0.0, 1.0)), 128 / 255, 128 / 255, 1.0),
    (_encode((0.0, 0.0, 1.0)), 0.8, 0.4, 1.0),
    (_encode(_TILT), 128 / 255, 128 / 255, 1.0),
    (_encode(_TILT), 0.7, 0.3, 0.5),
    (_encode(_TILT), 0.9, 0.9, 0.0),
]

SCRIPT = textwrap.dedent(
    """
    import bpy, json, sys
    sys.path.insert(0, {addon!r})
    from starbreaker_addon.runtime.importer import PackageImporter
    from starbreaker_addon.runtime import detail_utils
    bpy.ops.wm.read_factory_settings(use_empty=True)
    importer = object.__new__(PackageImporter)
    group = importer._ensure_runtime_layer_surface_group()
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 1
    scene.render.resolution_x = scene.render.resolution_y = 4
    scene.view_settings.view_transform = "Standard"
    scene.render.image_settings.file_format = "OPEN_EXR"
    bpy.ops.mesh.primitive_plane_add(size=2)
    plane = bpy.context.object
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scene.collection.objects.link(cam)
    cam.location = (0, 0, 3)
    scene.camera = cam
    def render():
        path = bpy.app.tempdir + "overlay.exr"
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(path)
        value = list(img.pixels[0:3])
        bpy.data.images.remove(img)
        return value
    def material(build):
        mat = bpy.data.materials.new("m")
        mat.use_nodes = True
        nt = mat.node_tree
        nt.nodes.clear()
        e = nt.nodes.new("ShaderNodeEmission")
        o = nt.nodes.new("ShaderNodeOutputMaterial")
        nt.links.new(e.outputs[0], o.inputs[0])
        build(nt, e)
        plane.data.materials.clear()
        plane.data.materials.append(mat)
        return render()
    def plain(rgb):
        def build(nt, e):
            e.inputs["Color"].default_value = (*rgb, 1.0)
        return material(build)
    def surface(values, output, encode):
        def build(nt, e):
            g = nt.nodes.new("ShaderNodeGroup")
            g.node_tree = group
            g.inputs["Base Color"].default_value = (0.4, 0.4, 0.4, 1.0)
            g.inputs["Base Alpha"].default_value = 1.0
            for name, value in values.items():
                g.inputs[name].default_value = value
            socket = g.outputs[output]
            if encode:
                enc = nt.nodes.new("ShaderNodeVectorMath")
                enc.operation = "MULTIPLY_ADD"
                enc.inputs[1].default_value = (0.5, 0.5, 0.5)
                enc.inputs[2].default_value = (0.5, 0.5, 0.5)
                nt.links.new(socket, enc.inputs[0])
                socket = enc.outputs[0]
            nt.links.new(socket, e.inputs["Color"])
        return material(build)
    gloss = []
    for rough, b, strength in {gloss_cases!r}:
        got = surface({{"Roughness Source": rough, "Palette Glossiness": 0.0,
                        "Detail Gloss Mask": b, "Detail Gloss Strength": strength}}, "Roughness", False)
        s = detail_utils.detail_smoothness(1.0 - rough * rough, b, strength)
        want = (1.0 - s) ** 0.5
        gloss.append([got[0], plain((want, want, want))[0], want])
    normals = []
    for base, g_, a_, bump in {normal_cases!r}:
        got = surface({{"Normal Color": (*base, 1.0), "Detail Normal X": g_, "Detail Normal Y": a_,
                        "Detail Bump Strength": bump}}, "Normal", True)
        n = [c * 2.0 - 1.0 for c in base]
        want = detail_utils.add_detail_normals(n, detail_utils.detail_normal(g_, a_, bump))
        normals.append([got, plain([c * 0.5 + 0.5 for c in want]), list(want)])
    print("SB_RESULT", json.dumps({{"gloss": gloss, "normals": normals}}))
    """
)


@unittest.skipUnless(BLENDER.exists(), "needs Blender 5.1 (BLENDER_BIN or the default install)")
class LayerSurfaceDetailOverlayTests(unittest.TestCase):
    result: dict

    @classmethod
    def setUpClass(cls) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "overlay.py"
            script.write_text(
                SCRIPT.format(addon=str(ADDON_ROOT), gloss_cases=GLOSS_CASES, normal_cases=NORMAL_CASES),
                encoding="utf-8",
            )
            run = subprocess.run(
                [str(BLENDER), "-b", "--factory-startup", "--python", str(script)],
                capture_output=True, text=True, timeout=300,
            )
        line = next((l for l in run.stdout.splitlines() if l.startswith("SB_RESULT")), None)
        if line is None:
            raise AssertionError(run.stdout[-2000:] + run.stderr[-2000:])
        cls.result = json.loads(line[len("SB_RESULT"):])

    def test_detail_gloss_scales_smoothness(self) -> None:
        for case, (value, expected, raw) in zip(GLOSS_CASES, self.result["gloss"]):
            self.assertAlmostEqual(value, expected, places=3, msg=f"case={case} engine={raw}")

    def test_detail_normals_use_green_alpha_and_rnm(self) -> None:
        for case, (value, expected, raw) in zip(NORMAL_CASES, self.result["normals"]):
            for channel in range(3):
                self.assertAlmostEqual(
                    value[channel], expected[channel], places=2, msg=f"case={case} engine={raw} got={value}"
                )


if __name__ == "__main__":
    unittest.main()
