"""Headless check: append packages into a host .blend saved outside their export root.

Run with Blender (no addon install needed; the source tree is used):

    blender -b --factory-startup --python blender_addon/scripts/check_package_append.py -- \
        <host_dir> <Packages/<pkg>/scene.blend> [<scene.blend> ...]

Pass the same scene.blend twice to check two copies of one ship. ``host_dir``
must be outside every package's export root. The check:

1. saves an empty host .blend in ``host_dir`` (so library paths are host-relative),
2. runs ``starbreaker.add_package`` for each scene.blend and checks every package
   root's materials were refreshed and that copies of one package share mesh data,
3. saves the host uncompressed and reopens it; fails if any library, image or
   append weak-reference path in it is absolute, or if the saved bytes contain
   an export-root path,
4. resolves every package's scene.json again from the reopened host.

Exits non-zero on the first failed check.
"""

from __future__ import annotations

import os
from pathlib import Path
import sys
import time

import bpy

ADDON_PARENT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ADDON_PARENT))


def _fail(message: str) -> None:
    print(f"CHECK FAILED: {message}")
    sys.stdout.flush()
    os._exit(1)


def _register_addon():
    import starbreaker_addon

    try:
        starbreaker_addon.unregister()
    except Exception:
        pass
    starbreaker_addon.register()
    from starbreaker_addon.runtime import package_ops

    return package_ops


def _package_roots() -> list[bpy.types.Object]:
    return [obj for obj in bpy.data.objects if obj.get("starbreaker_package_root")]


def _mesh_signature(root: bpy.types.Object) -> list[tuple[str, int]]:
    """Linked mesh datablocks of a package, in a copy-independent order."""

    meshes = [
        obj.data
        for obj in root.children_recursive
        if obj.type == "MESH" and obj.data is not None and obj.data.library is not None
    ]
    return sorted((mesh.name, mesh.as_pointer()) for mesh in meshes)


def _check_no_absolute_paths(host: Path, export_roots: set[Path]) -> None:
    for library in bpy.data.libraries:
        if not library.filepath.startswith("//"):
            _fail(f"library path is absolute: {library.filepath}")
    for image in bpy.data.images:
        if image.source == "FILE" and image.filepath and not image.filepath.startswith("//"):
            _fail(f"image path is absolute: {image.name} -> {image.filepath}")
    for obj in bpy.data.objects:
        export_root = obj.get("starbreaker_export_root")
        if export_root:
            _fail(f"{obj.name} records an export root: {export_root}")
    # Blender records the source file of every appended ID (read-only
    # ``library_weak_reference``); it is relative only if the append was.
    for id_block in bpy.data.user_map():
        weak = id_block.library_weak_reference
        if weak is not None and not weak.filepath.startswith("//"):
            _fail(f"{id_block.name} keeps an absolute append reference: {weak.filepath}")
    data = host.read_bytes()
    for export_root in export_roots:
        for spelling in {str(export_root), export_root.as_posix()}:
            if spelling.encode("utf-8") in data:
                _fail(f"saved host contains the export root path {spelling}")


def main(argv: list[str]) -> None:
    if len(argv) < 2:
        _fail("usage: -- <host_dir> <scene.blend> [<scene.blend> ...]")
    host_dir = Path(argv[0]).resolve()
    scene_blends = [Path(arg).resolve() for arg in argv[1:]]
    host = host_dir / "host.blend"
    host_dir.mkdir(parents=True, exist_ok=True)

    bpy.ops.wm.read_homefile(app_template="")
    bpy.ops.wm.save_as_mainfile(filepath=str(host), compress=False)
    package_ops = _register_addon()

    for scene_blend in scene_blends:
        before = {obj.name for obj in _package_roots()}
        started = time.perf_counter()
        result = bpy.ops.starbreaker.add_package(filepath=str(scene_blend))
        added = [obj for obj in _package_roots() if obj.name not in before]
        print(f"add_package {scene_blend.name} -> {result} {[obj.name for obj in added]} in {time.perf_counter() - started:.1f}s")
        if result != {"FINISHED"} or len(added) != 1:
            _fail(f"add_package did not add exactly one package root for {scene_blend}")

    roots = _package_roots()
    if len(roots) != len(scene_blends):
        _fail(f"expected {len(scene_blends)} package roots, found {[obj.name for obj in roots]}")
    export_roots: set[Path] = set()
    for root in roots:
        scene_json = package_ops.resolve_package_scene_path(root)
        export_roots.add(scene_json.parents[2])
        if package_ops.package_root_needs_material_refresh(root):
            _fail(f"{root.name} still has unrefreshed materials")
        dirty = package_ops.dirty_package_material_objects(root)
        if dirty:
            _fail(f"{root.name} has {len(dirty)} objects with unrefreshed materials")
        print(f"refreshed {root.name}: {scene_json}")
    for export_root in export_roots:
        if host_dir == export_root or export_root in host_dir.parents:
            _fail(f"host folder {host_dir} is inside export root {export_root}")

    by_scene: dict[str, list[bpy.types.Object]] = {}
    for root in roots:
        by_scene.setdefault(str(package_ops.resolve_package_scene_path(root)), []).append(root)
    for copies in by_scene.values():
        signatures = [_mesh_signature(root) for root in copies]
        if not signatures[0] or any(signature != signatures[0] for signature in signatures):
            _fail(f"copies {[root.name for root in copies]} do not share their linked mesh data")
        if len(copies) > 1:
            print(f"{len(copies)} copies share {len(signatures[0])} linked meshes")

    bpy.ops.wm.save_mainfile(compress=False)
    # Check what was written: reopen, so unused in-memory data is gone.
    bpy.ops.wm.open_mainfile(filepath=str(host))
    _check_no_absolute_paths(host, export_roots)
    package_ops = _register_addon()
    for root in _package_roots():
        scene_json = package_ops.resolve_package_scene_path(root)
        if not scene_json.is_file():
            _fail(f"after reopen, {root.name} resolves to a missing {scene_json}")
        if package_ops.package_root_needs_material_refresh(root):
            _fail(f"after reopen, {root.name} needs a material refresh")
    print(f"CHECK PASSED: {len(roots)} package root(s) in {host}")
    sys.stdout.flush()


if __name__ == "__main__":
    main(sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else [])
