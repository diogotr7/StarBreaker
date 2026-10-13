import argparse
import json
from pathlib import Path
import sys

import bpy

parser = argparse.ArgumentParser()
parser.add_argument('export_root', type=Path)
parser.add_argument('package', type=Path)
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import starbreaker_addon
bpy.ops.wm.read_homefile(app_template='')
starbreaker_addon.register()
bpy.ops.wm.open_mainfile(filepath=str(args.package / 'scene.blend'))
from starbreaker_addon.ui import _material_refresh_prompt_timer
_material_refresh_prompt_timer()
manifest = json.loads((args.package / 'scene.json').read_text())
sidecar = json.loads((args.export_root / manifest['root_entity']['material_sidecar']).read_text())
checked = 0
for record in sidecar['submaterials']:
    material = bpy.data.materials.get(record['blender_material_name'])
    if material is None or material.node_tree is None:
        continue
    scales = {}
    for layer in record['layer_manifest']:
        if layer.get('diffuse_export_path'):
            path = str((args.export_root / layer['diffuse_export_path']).resolve()).lower()
            scales.setdefault(path, set()).add(layer.get('uv_tiling', 1.0))
    for node in material.node_tree.nodes:
        if node.type != 'TEX_IMAGE' or node.image is None:
            continue
        path = str(Path(bpy.path.abspath(node.image.filepath)).resolve()).lower()
        expected = scales.get(path)
        if not expected or len(expected) != 1:
            continue
        tile = next(iter(expected))
        if abs(tile - 1.0) < 0.0001:
            continue
        assert node.inputs['Vector'].is_linked, (material.name, path, 'missing layer UV tiling')
        mapping = node.inputs['Vector'].links[0].from_node
        assert mapping.type == 'MAPPING', (material.name, mapping.type)
        scale = mapping.inputs['Scale'].default_value
        assert abs(scale.x - tile) < 0.0001 and abs(scale.y - tile) < 0.0001, (material.name, tuple(scale), tile)
        checked += 1
assert checked > 0, 'No non-unit layer diffuse tiling tested'
print('LAYER_DIFFUSE_TILING_PASS', checked)
