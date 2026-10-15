"""Export-root location from Blender library bookkeeping (bpy-free module)."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


# runtime/constants.py imports mathutils; stub it when not running inside Blender.
if "mathutils" not in sys.modules:
    class _Matrix(tuple):
        def inverted(self):
            return self

    _mathutils = types.ModuleType("mathutils")
    _mathutils.Matrix = _Matrix
    _mathutils.Quaternion = tuple
    sys.modules["mathutils"] = _mathutils

_runtime_pkg = types.ModuleType("sb_location_test")
_runtime_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
sys.modules["sb_location_test"] = _runtime_pkg


def _load_module(name: str, filename: str) -> types.ModuleType:
    spec = importlib.util.spec_from_file_location(name, ADDON_ROOT / "runtime" / filename)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


package_location = _load_module("sb_location_test.package_location", "package_location.py")
package_append = _load_module("sb_location_test.package_append", "package_append.py")
image_paths = _load_module("sb_location_test.image_paths", "image_paths.py")


class FakeLibrary:
    def __init__(self, filepath: str, parent: FakeLibrary | None = None):
        self.filepath = filepath
        self.parent = parent


class FakeObject(dict):
    def __init__(self, name: str, *, library: FakeLibrary | None = None, **props):
        super().__init__(props)
        self.name = name
        self.data = types.SimpleNamespace(library=library) if library is not None else None
        self.children: list[FakeObject] = []

    @property
    def children_recursive(self) -> list[FakeObject]:
        result: list[FakeObject] = []
        stack = list(self.children)
        while stack:
            child = stack.pop()
            result.append(child)
            stack.extend(child.children)
        return result


def _blender_abspath(host_blend: Path):
    """Mimic ``bpy.path.abspath``: ``//`` is the host .blend folder; no normalisation."""

    def abspath(path: str, *, start=None, library=None) -> str:
        if not path.startswith("//"):
            return path
        if library is not None:
            base = os.path.dirname(abspath(library.filepath, library=library.parent))
        else:
            base = str(host_blend.parent)
        return os.path.join(base, path[2:])

    return abspath


def _package_root(*children: FakeObject) -> FakeObject:
    root = FakeObject("StarBreaker Ship", starbreaker_package_root=1)
    root.children.extend(children)
    return root


def _linked_mesh(name: str, library_path: str, mesh_asset: str) -> FakeObject:
    return FakeObject(name, library=FakeLibrary(library_path), starbreaker_mesh_asset=mesh_asset)


class ExportRootFromLibrariesTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.host = self.tmp / "scenes" / "host.blend"
        self.abspath = _blender_abspath(self.host)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_export_root_is_found_for_host_saved_outside_the_export_root(self) -> None:
        root = _package_root(
            _linked_mesh("hull", "//../exports/zeus/Data/Objects/Ships/hull.blend", "Data/Objects/Ships/hull.blend"),
            _linked_mesh("seat", "//../exports/zeus/Data/Objects/Seats/seat.blend", "Data/Objects/Seats/seat.blend"),
        )

        export_root = package_location.export_root_from_linked_meshes(root, self.abspath)

        self.assertEqual(export_root, self.tmp / "exports" / "zeus")

    def test_windows_separators_and_case_in_library_path(self) -> None:
        if os.name != "nt":
            self.skipTest("backslash paths and case-insensitive matching are Windows behaviour")
        root = _package_root(
            _linked_mesh("hull", "//..\\exports\\zeus\\Data\\Objects\\Ships\\HULL.blend", "Data/Objects/Ships/hull.blend"),
        )

        export_root = package_location.export_root_from_linked_meshes(root, self.abspath)

        self.assertEqual(export_root, self.tmp / "exports" / "zeus")

    def test_nested_package_objects_are_considered(self) -> None:
        wrapper = FakeObject("interior")
        wrapper.children.append(
            _linked_mesh("bed", "//../exports/zeus/Data/Objects/Interior/bed.blend", "Data/Objects/Interior/bed.blend")
        )
        root = _package_root(wrapper)

        export_root = package_location.export_root_from_linked_meshes(root, self.abspath)

        self.assertEqual(export_root, self.tmp / "exports" / "zeus")

    def test_meshes_from_different_export_roots_fail(self) -> None:
        root = _package_root(
            _linked_mesh("hull", "//../exports/zeus/Data/Objects/hull.blend", "Data/Objects/hull.blend"),
            _linked_mesh("seat", "//../exports/other/Data/Objects/seat.blend", "Data/Objects/seat.blend"),
        )

        with self.assertRaises(package_location.PackageLocationError) as raised:
            package_location.export_root_from_linked_meshes(root, self.abspath)

        message = str(raised.exception)
        self.assertIn("more than one export root", message)
        self.assertIn(str(self.tmp / "exports" / "zeus"), message)
        self.assertIn(str(self.tmp / "exports" / "other"), message)

    def test_root_without_linked_meshes_fails(self) -> None:
        root = _package_root(FakeObject("light"), FakeObject("local_mesh", starbreaker_mesh_asset="Data/Objects/x.blend"))

        with self.assertRaises(package_location.PackageLocationError) as raised:
            package_location.export_root_from_linked_meshes(root, self.abspath)

        self.assertIn("no mesh linked from a StarBreaker export", str(raised.exception))

    def test_linked_data_without_mesh_asset_metadata_is_ignored(self) -> None:
        root = _package_root(
            FakeObject("user_linked", library=FakeLibrary("//../library/props.blend")),
            _linked_mesh("hull", "//../exports/zeus/Data/Objects/hull.blend", "Data/Objects/hull.blend"),
        )

        export_root = package_location.export_root_from_linked_meshes(root, self.abspath)

        self.assertEqual(export_root, self.tmp / "exports" / "zeus")

    def test_library_path_that_does_not_end_with_the_mesh_asset_fails(self) -> None:
        root = _package_root(
            _linked_mesh("hull", "//../exports/zeus/Data/Objects/renamed.blend", "Data/Objects/hull.blend"),
        )

        with self.assertRaises(package_location.PackageLocationError) as raised:
            package_location.export_root_from_linked_meshes(root, self.abspath)

        self.assertIn("does not end with its mesh asset", str(raised.exception))

    def test_indirect_library_resolves_against_its_parent_library(self) -> None:
        parent = FakeLibrary("//../exports/zeus/Packages/Zeus/scene.blend")
        hull = FakeObject(
            "hull",
            library=FakeLibrary("//../../Data/Objects/hull.blend", parent=parent),
            starbreaker_mesh_asset="Data/Objects/hull.blend",
        )

        export_root = package_location.export_root_from_linked_meshes(_package_root(hull), self.abspath)

        self.assertEqual(export_root, self.tmp / "exports" / "zeus")


class FakeCollection:
    def __init__(self, name: str, objects: list[FakeObject] | None = None):
        self.name = name
        self.objects = list(objects or [])
        self.children: list[FakeCollection] = []

    @property
    def all_objects(self) -> list[FakeObject]:
        result = list(self.objects)
        for child in self.children:
            result.extend(child.all_objects)
        return result


class PackageCollectionSelectionTests(unittest.TestCase):
    def test_top_level_collections_holding_a_package_root_are_selected(self) -> None:
        root = FakeObject("StarBreaker Zeus", starbreaker_package_root=1)
        main = FakeCollection("StarBreaker Zeus", [root, FakeObject("hull")])
        interior = FakeCollection("StarBreaker Zeus Interior", [FakeObject("bed")])
        main.children.append(interior)
        empty = FakeCollection("Collection")

        selected = package_append.package_collections([interior, main, empty])

        self.assertEqual(selected, [main])

    def test_package_roots_are_found_in_selected_collections(self) -> None:
        root = FakeObject("StarBreaker Zeus", starbreaker_package_root=1)
        main = FakeCollection("StarBreaker Zeus", [FakeObject("hull"), root])

        self.assertEqual(package_append.package_roots([main]), [root])

    def test_no_package_collection_is_an_error(self) -> None:
        with self.assertRaises(package_append.PackageAppendError):
            package_append.package_collections([FakeCollection("Collection")])



class HostRelativePathTests(unittest.TestCase):
    def test_path_is_made_relative_to_the_host_blend_folder(self) -> None:
        base = Path(tempfile.gettempdir())
        texture = base / "exports" / "ships" / "Data" / "Textures" / "hull_diff.png"
        host = base / "scenes" / "hangar.blend"

        self.assertEqual(
            image_paths.host_relative_path(str(texture), str(host)),
            "//../exports/ships/Data/Textures/hull_diff.png",
        )

    def test_unsaved_host_has_no_relative_path(self) -> None:
        self.assertIsNone(image_paths.host_relative_path(str(Path(tempfile.gettempdir()) / "a.png"), ""))

    def test_other_drive_has_no_relative_path(self) -> None:
        if os.name != "nt":
            self.skipTest("drive letters are Windows-only")
        self.assertIsNone(image_paths.host_relative_path("Q:\exports\a.png", "C:\scenes\hangar.blend"))

if __name__ == "__main__":
    unittest.main()
