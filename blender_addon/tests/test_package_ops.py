from __future__ import annotations

from contextlib import contextmanager
from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


class FakeObject(dict):
    def __init__(self, name: str, **props):
        super().__init__(props)
        self.name = name
        self.parent = None
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


class FakeObjects(list):
    def __init__(self, items: list[FakeObject] | None = None):
        super().__init__(items or [])
        self.removed: list[tuple[str, bool]] = []

    def remove(self, obj, do_unlink: bool = False):  # noqa: A003 - matches bpy API name
        self.removed.append((obj.name, do_unlink))
        if obj in self:
            super().remove(obj)


def _load_package_ops() -> tuple[types.ModuleType, types.ModuleType]:
    bpy = sys.modules.get("bpy")
    if bpy is None:
        bpy = types.ModuleType("bpy")
        sys.modules["bpy"] = bpy
    bpy.types = types.SimpleNamespace(Context=object, Object=object, ID=object, Light=object)
    bpy.data = types.SimpleNamespace(objects=FakeObjects(), lights=[])

    mathutils = sys.modules.get("mathutils")
    if mathutils is None:
        mathutils = types.ModuleType("mathutils")
        sys.modules["mathutils"] = mathutils

    class Matrix(tuple):
        def __new__(cls, rows):
            return tuple.__new__(cls, rows)

        def inverted(self):
            return self

    class Quaternion(tuple):
        def __new__(cls, values):
            return tuple.__new__(cls, values)

    mathutils.Matrix = Matrix
    mathutils.Quaternion = Quaternion

    def _load(name: str, path: Path) -> types.ModuleType:
        spec = __import__("importlib.util").util.spec_from_file_location(name, str(path))
        assert spec is not None and spec.loader is not None
        module = __import__("importlib.util").util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
        return module

    constants = _load("sb_pkg_test_runtime.constants", ADDON_ROOT / "runtime" / "constants.py")
    runtime_pkg = types.ModuleType("sb_pkg_test_runtime")
    runtime_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
    sys.modules["sb_pkg_test_runtime"] = runtime_pkg

    addon_pkg = types.ModuleType("sb_pkg_test_addon")
    addon_pkg.__path__ = [str(ADDON_ROOT)]
    sys.modules["sb_pkg_test_addon"] = addon_pkg

    manifest_stub = types.ModuleType("sb_pkg_test_addon.manifest")

    class PackageBundle:
        @staticmethod
        def load(scene_path):
            return types.SimpleNamespace(scene_path=Path(scene_path), package_name="Test Package")

    manifest_stub.PackageBundle = PackageBundle
    manifest_stub.SceneInstanceRecord = type("SceneInstanceRecord", (), {})
    sys.modules["sb_pkg_test_addon.manifest"] = manifest_stub

    palette_stub = types.ModuleType("sb_pkg_test_addon.palette")
    palette_stub.palette_id_for_livery_instance = lambda *args, **kwargs: None
    palette_stub.resolved_palette_id = lambda package, requested, inherited: requested or inherited
    sys.modules["sb_pkg_test_addon.palette"] = palette_stub

    validators_stub = types.ModuleType("sb_pkg_test_runtime.validators")
    validators_stub._purge_orphaned_file_backed_images = lambda: 0
    validators_stub._purge_orphaned_runtime_groups = lambda: 0
    sys.modules["sb_pkg_test_runtime.validators"] = validators_stub

    importer_stub = types.ModuleType("sb_pkg_test_runtime.importer")
    importer_stub.events = []

    class PackageImporter:
        def __init__(self, context, package, progress_callback=None):
            self.context = context
            self.package = package
            self.progress_callback = progress_callback

        def import_scene(self, prefer_cycles=True, palette_id=None):
            importer_stub.events.append(("import", str(self.package.scene_path), prefer_cycles, palette_id))
            return "imported-root"

    importer_stub.PackageImporter = PackageImporter
    sys.modules["sb_pkg_test_runtime.importer"] = importer_stub

    source = (ADDON_ROOT / "runtime" / "package_ops.py").read_text()
    source = source.replace("from ..manifest import", "from sb_pkg_test_addon.manifest import")
    source = source.replace("from ..palette import", "from sb_pkg_test_addon.palette import")
    source = source.replace("from .constants import", "from sb_pkg_test_runtime.constants import")
    source = source.replace("from .validators import", "from sb_pkg_test_runtime.validators import")
    source = source.replace("from .importer import PackageImporter", "from sb_pkg_test_runtime.importer import PackageImporter")
    module = types.ModuleType("sb_pkg_test_runtime.package_ops")
    module.__file__ = str(ADDON_ROOT / "runtime" / "package_ops.py")
    module.__package__ = "sb_pkg_test_runtime"
    sys.modules[module.__name__] = module
    exec(compile(source, module.__file__, "exec"), module.__dict__)
    return module, bpy


class PackageOpsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.package_ops, cls.bpy = _load_package_ops()
        cls._original_remove_existing_package_instances = cls.package_ops._remove_existing_package_instances
        cls._original_suspend_heavy_viewports = cls.package_ops._suspend_heavy_viewports

    def setUp(self) -> None:
        self.bpy.data.objects = FakeObjects()
        self.package_ops._remove_existing_package_instances = type(self)._original_remove_existing_package_instances
        self.package_ops._suspend_heavy_viewports = type(self)._original_suspend_heavy_viewports

    def test_remove_existing_package_instances_replaces_matching_scene_path(self) -> None:
        scene_path = Path("/tmp/vulture/scene.json")
        other_scene_path = Path("/tmp/aurora/scene.json")
        package_root = FakeObject(
            "StarBreaker DRAK Vulture",
            starbreaker_package_root=True,
            starbreaker_scene_path=str(scene_path),
        )
        child = FakeObject("Vulture Child")
        child.parent = package_root
        package_root.children.append(child)
        other_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path=str(other_scene_path),
        )
        self.bpy.data.objects.extend([package_root, child, other_root])

        removed = self.package_ops._remove_existing_package_instances(scene_path)

        self.assertEqual(removed, 2)
        self.assertEqual(self.bpy.data.objects.removed, [("Vulture Child", True), ("StarBreaker DRAK Vulture", True)])
        self.assertEqual(list(self.bpy.data.objects), [other_root])

    def test_import_package_removes_existing_package_before_import(self) -> None:
        events: list[tuple[str, str]] = []

        def _cleanup(scene_path):
            events.append(("cleanup", str(scene_path)))
            return 1

        @contextmanager
        def _no_suspend(_context):
            yield

        self.package_ops._remove_existing_package_instances = _cleanup
        self.package_ops._suspend_heavy_viewports = _no_suspend
        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []

        context = types.SimpleNamespace(scene=types.SimpleNamespace(render=types.SimpleNamespace(engine="BLENDER_EEVEE")))
        root = self.package_ops.import_package(context, "/tmp/vulture/scene.json", prefer_cycles=False, palette_id="palette/test")

        self.assertEqual(root, "imported-root")
        self.assertEqual(events, [("cleanup", "/tmp/vulture/scene.json")])
        self.assertEqual(
            importer_stub.events,
            [("import", "/tmp/vulture/scene.json", False, "palette/test")],
        )


if __name__ == "__main__":  # pragma: no cover
    unittest.main()