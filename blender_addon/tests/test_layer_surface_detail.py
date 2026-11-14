"""The layer surface group's detail-map colour matches the engine's.

Star Engine (`common.cfib` `UnpackDetailMap`, `layerLib.cfib`
`LayerAttribs_DetailOverlay`): the detail map's red channel is centred on
mid-grey, so `albedo += albedo * (2 * R - 1) * DetailDiffuse`. A mid-grey
detail map leaves the layer colour unchanged; brighter texels brighten it.

Runs the add-on's real `StarBreaker Runtime LayerSurface` group in headless
Blender (skipped when Blender is not installed): a plane emits the group's
Color output, and a tiny Cycles render of it is compared with a render of a
plain emission of the expected value (the same colour management applies to
both, so the comparison is exact).
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

CASES = [(0.5, 1.0), (1.0, 1.0), (0.25, 0.5), (1.0, 0.0), (0.0, 0.8)]

SCRIPT = textwrap.dedent(
    """
    import bpy, json, sys
    sys.path.insert(0, {addon!r})
    from starbreaker_addon.runtime.importer import PackageImporter
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
        path = bpy.app.tempdir + "detail.exr"
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(path)
        value = img.pixels[0]
        bpy.data.images.remove(img)
        return value
    def plain(value):
        mat = bpy.data.materials.new("plain")
        mat.use_nodes = True
        nt = mat.node_tree
        nt.nodes.clear()
        e = nt.nodes.new("ShaderNodeEmission")
        o = nt.nodes.new("ShaderNodeOutputMaterial")
        e.inputs["Color"].default_value = (value, value, value, 1.0)
        nt.links.new(e.outputs[0], o.inputs[0])
        plane.data.materials.clear()
        plane.data.materials.append(mat)
        return render()
    out = []
    for r, strength in {cases!r}:
        mat = bpy.data.materials.new("m")
        mat.use_nodes = True
        nt = mat.node_tree
        nt.nodes.clear()
        g = nt.nodes.new("ShaderNodeGroup")
        g.node_tree = group
        for name in ("Base Color", "Palette Color", "Tint Color"):
            g.inputs[name].default_value = (0.4, 0.4, 0.4, 1.0) if name == "Base Color" else (1, 1, 1, 1)
        g.inputs["Base Alpha"].default_value = 1.0
        g.inputs["Metallic"].default_value = 0.0
        g.inputs["Detail Color Mask"].default_value = r
        g.inputs["Detail Diffuse Strength"].default_value = strength
        e = nt.nodes.new("ShaderNodeEmission")
        o = nt.nodes.new("ShaderNodeOutputMaterial")
        nt.links.new(g.outputs["Color"], e.inputs["Color"])
        nt.links.new(e.outputs[0], o.inputs[0])
        plane.data.materials.clear()
        plane.data.materials.append(mat)
        got = render()
        expected = plain(0.4 * (1.0 + (2.0 * r - 1.0) * strength))
        out.append([got, expected])
    print("SB_RESULT", json.dumps(out))
    """
)


@unittest.skipUnless(BLENDER.exists(), "needs Blender 5.1 (BLENDER_BIN or the default install)")
class LayerSurfaceDetailTests(unittest.TestCase):
    def test_detail_colour_is_centred_on_mid_grey(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "detail.py"
            script.write_text(SCRIPT.format(addon=str(ADDON_ROOT), cases=CASES), encoding="utf-8")
            run = subprocess.run(
                [str(BLENDER), "-b", "--factory-startup", "--python", str(script)],
                capture_output=True, text=True, timeout=300,
            )
        line = next((l for l in run.stdout.splitlines() if l.startswith("SB_RESULT")), None)
        self.assertIsNotNone(line, run.stdout[-2000:] + run.stderr[-2000:])
        got = json.loads(line[len("SB_RESULT"):])
        for (r, strength), (value, expected) in zip(CASES, got):
            self.assertAlmostEqual(value, expected, places=3, msg=f"R={r} strength={strength}; all={got}")


if __name__ == "__main__":
    unittest.main()
