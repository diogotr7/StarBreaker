from __future__ import annotations

from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


class FakeObject(dict):
    def __init__(self, name: str, **props):
        super().__init__(props)
        self.name = name
        self.parent = None
        self.children: list[FakeObject] = []
        self.type = "EMPTY"
        self.material_slots = []
        self.modifiers = []

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


class FakeMaterial(dict):
    def __init__(self, name: str, *, library=None, **props):
        super().__init__(props)
        self.name = name
        self.library = library
        self.node_tree = None

    def copy(self):
        clone = FakeMaterial(self.name, library=self.library, **dict(self))
        clone.node_tree = self.node_tree.copy() if self.node_tree is not None else None
        return clone


class FakeSlot:
    def __init__(self, material=None):
        self.material = material


class FakeModifier:
    def __init__(self, name: str, type: str, strength: float = 0.0):  # noqa: A002 - matches bpy API
        self.name = name
        self.type = type
        self.strength = strength


class FakeSocket:
    def __init__(self, default_value: float):
        self.default_value = default_value


class FakeNode:
    def __init__(
        self,
        emission_strength: float,
        *,
        label: str = "",
        include_emission: bool = True,
        emission_color=(1.0, 1.0, 1.0, 1.0),
        palette_color=(0.0, 0.0, 0.0, 1.0),
    ):
        self.bl_idname = "ShaderNodeGroup"
        self.label = label
        self.name = label or "Group"
        self.inputs = {"Palette Color": FakeSocket(palette_color)}
        if include_emission:
            self.inputs.update(
                {
                    "Emission Strength": FakeSocket(emission_strength),
                    "Emission Color": FakeSocket(emission_color),
                }
            )


class FakeNodeTree:
    def __init__(
        self,
        emission_strength: float,
        *,
        emission_color=(1.0, 1.0, 1.0, 1.0),
        palette_color=(0.0, 0.0, 0.0, 1.0),
    ):
        self.nodes = [
            FakeNode(emission_strength, label="StarBreaker Illum", emission_color=emission_color, palette_color=palette_color),
            FakeNode(emission_strength, label="Primary Layer", include_emission=False, palette_color=palette_color),
        ]

    def copy(self):
        return FakeNodeTree(
            self.nodes[0].inputs["Emission Strength"].default_value,
            emission_color=self.nodes[0].inputs["Emission Color"].default_value,
            palette_color=self.nodes[1].inputs["Palette Color"].default_value,
        )


class FakeUVLayer:
    def __init__(self, name: str):
        self.name = name
        self.active_render = False


class FakeUVLayers(list):
    def __init__(self, names: list[str], active_index: int = -1):
        super().__init__(FakeUVLayer(name) for name in names)
        self.active_index = active_index

    @property
    def active(self):
        if 0 <= self.active_index < len(self):
            return self[self.active_index]
        return None

    def find(self, name: str) -> int:
        for index, layer in enumerate(self):
            if layer.name == name:
                return index
        return -1


class FakeMeshData:
    def __init__(
        self,
        uv_layers: FakeUVLayers | None = None,
        *,
        name: str = "Mesh",
        vertex_count: int = 0,
        polygon_count: int = 0,
    ):
        self.name = name
        self.uv_layers = uv_layers or FakeUVLayers([])
        self.vertices = [object() for _index in range(vertex_count)]
        self.polygons = [object() for _index in range(polygon_count)]
        self.update_tags: list[object] = []

    def update_tag(self, refresh=None):
        self.update_tags.append(refresh)


class FakeViewLayer:
    def __init__(self):
        self.update_count = 0

    def update(self):
        self.update_count += 1


def _load_package_ops() -> tuple[types.ModuleType, types.ModuleType]:
    bpy = sys.modules.get("bpy")
    if bpy is None:
        bpy = types.ModuleType("bpy")
        sys.modules["bpy"] = bpy
    bpy.types = types.SimpleNamespace(Context=object, Object=object, ID=object, Light=object)
    bpy.data = types.SimpleNamespace(objects=FakeObjects(), lights=[], filepath="")

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
            return types.SimpleNamespace(
                scene_path=Path(scene_path),
                package_name="Test Package",
                load_material_sidecar=lambda _path: None,
            )

    manifest_stub.PackageBundle = PackageBundle
    class SceneInstanceRecord:
        def __init__(self, **kwargs):
            self.__dict__.update(kwargs)

        @classmethod
        def from_value(cls, value):
            return cls(**value)

    manifest_stub.SceneInstanceRecord = SceneInstanceRecord
    sys.modules["sb_pkg_test_addon.manifest"] = manifest_stub

    palette_stub = types.ModuleType("sb_pkg_test_addon.palette")
    palette_stub.palette_id_for_livery_instance = lambda *args, **kwargs: None
    palette_stub.resolved_palette_id = lambda package, requested, inherited: requested or inherited
    sys.modules["sb_pkg_test_addon.palette"] = palette_stub

    validators_stub = types.ModuleType("sb_pkg_test_runtime.validators")
    validators_stub._purge_orphaned_file_backed_images = lambda: 0
    validators_stub._purge_orphaned_managed_materials = lambda: 0
    validators_stub._purge_orphaned_runtime_actions = lambda: 0
    validators_stub._purge_orphaned_runtime_groups = lambda: 0
    sys.modules["sb_pkg_test_runtime.validators"] = validators_stub

    importer_stub = types.ModuleType("sb_pkg_test_runtime.importer")
    importer_stub.events = []

    class PackageImporter:
        def __init__(self, context, package, progress_callback=None, package_root=None, create_template_collection=True):
            self.context = context
            self.package = package
            self.progress_callback = progress_callback
            self.package_root = package_root

        def import_scene(self, prefer_cycles=True, palette_id=None):
            importer_stub.events.append(("import", str(self.package.scene_path), prefer_cycles, palette_id))
            return "imported-root"

        def rebuild_object_materials(self, obj, palette_id):
            importer_stub.events.append(("rebuild", obj.name, palette_id))
            return 1

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
        self.assertEqual(events, [("cleanup", str(Path("/tmp/vulture/scene.json")))])
        self.assertEqual(
            importer_stub.events,
            [("import", str(Path("/tmp/vulture/scene.json")), False, "palette/test")],
        )

    def test_import_package_removes_unmodified_blender_startup_cube(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        cube = FakeObject("Cube")
        cube.type = "MESH"
        cube.data = FakeMeshData(name="Cube", vertex_count=8, polygon_count=6)
        cube.location = (0.0, 0.0, 0.0)
        cube.rotation_euler = (0.0, 0.0, 0.0)
        cube.scale = (1.0, 1.0, 1.0)
        self.bpy.data.objects.append(cube)
        self.package_ops._suspend_heavy_viewports = _no_suspend
        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []

        context = types.SimpleNamespace(scene=types.SimpleNamespace(render=types.SimpleNamespace(engine="BLENDER_EEVEE")))
        self.package_ops.import_package(context, "/tmp/vulture/scene.json", prefer_cycles=False)

        self.assertEqual(self.bpy.data.objects.removed, [("Cube", True)])
        self.assertNotIn(cube, self.bpy.data.objects)

    def test_startup_cube_cleanup_preserves_user_modified_cube(self) -> None:
        cube = FakeObject("Cube", user_modified=True)
        cube.type = "MESH"
        cube.data = FakeMeshData(name="Cube", vertex_count=8, polygon_count=6)
        cube.location = (0.0, 0.0, 0.0)
        cube.rotation_euler = (0.0, 0.0, 0.0)
        cube.scale = (1.0, 1.0, 1.0)
        self.bpy.data.objects.append(cube)

        removed = self.package_ops._remove_default_startup_cube()

        self.assertEqual(removed, 0)
        self.assertEqual(list(self.bpy.data.objects), [cube])
        self.assertEqual(self.bpy.data.objects.removed, [])

    def test_refresh_materials_rebuilds_only_meshes_with_sidecars(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject("hull", starbreaker_material_sidecar="hull.materials.json")
        mesh.type = "MESH"
        empty = FakeObject("helper", starbreaker_material_sidecar="helper.materials.json")
        empty.type = "EMPTY"
        mesh.parent = package_root
        empty.parent = package_root
        package_root.children.extend([mesh, empty])

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(),
                package_root,
                "palette/test",
            )
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "hull", "palette/test")])
        self.assertEqual(package_root["starbreaker_palette_id"], "palette/test")
        self.assertEqual(mesh["starbreaker_palette_id"], "palette/test")

    def test_material_refresh_session_processes_meshes_incrementally(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh_a = FakeObject("hull", starbreaker_material_sidecar="hull.materials.json")
        mesh_b = FakeObject("wing", starbreaker_material_sidecar="wing.materials.json")
        for mesh in (mesh_a, mesh_b):
            mesh.type = "MESH"
            mesh.data = FakeMeshData(FakeUVLayers(["UVMap"], active_index=-1))
            mesh.parent = package_root
            package_root.children.append(mesh)

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        original_perf_counter = self.package_ops.time.perf_counter
        perf_values = iter([0.0, 1.0, 2.0, 3.0])
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            self.package_ops.time.perf_counter = lambda: next(perf_values)
            context = types.SimpleNamespace(view_layer=FakeViewLayer())
            session = self.package_ops.MaterialRefreshSession(
                context,
                package_root,
                "palette/test",
                purge_orphans=False,
            )

            self.assertFalse(session.step(budget_seconds=0.001, min_objects=1))
            self.assertEqual(session.applied, 1)
            self.assertEqual(session.progress, 0.5)

            self.assertTrue(session.step(budget_seconds=0.001, min_objects=1))
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode
            self.package_ops.time.perf_counter = original_perf_counter

        self.assertEqual(session.applied, 2)
        self.assertEqual(session.progress, 1.0)
        self.assertTrue(session.done)
        self.assertEqual(package_root["starbreaker_palette_id"], "palette/test")
        self.assertEqual(mesh_a["starbreaker_palette_id"], "palette/test")
        self.assertEqual(mesh_b["starbreaker_palette_id"], "palette/test")
        self.assertEqual(context.view_layer.update_count, 1)
        self.assertEqual(
            importer_stub.events,
            [("rebuild", "wing", "palette/test"), ("rebuild", "hull", "palette/test")],
        )

    def test_decal_offset_control_enabled_detects_modifier_in_root_tree(self) -> None:
        package_root = FakeObject("StarBreaker RSI Aurora", starbreaker_package_root=True)
        child = FakeObject(
            "hull",
            starbreaker_material_sidecar="Data/Objects/Spaceships/Ships/RSI/aurora/exterior/aurora_ext_TEX0.materials.json",
        )
        child.modifiers = [FakeModifier("StarBreaker Decal Offset", "DISPLACE", 0.005)]
        child.parent = package_root
        package_root.children.append(child)

        self.assertTrue(self.package_ops.decal_offset_control_enabled(package_root))

    def test_apply_decal_offsets_to_package_root_updates_only_selected_root(self) -> None:
        package_root = FakeObject("StarBreaker RSI Aurora", starbreaker_package_root=True)
        exterior = FakeObject(
            "hull",
            starbreaker_material_sidecar="Data/Objects/Spaceships/Ships/RSI/aurora/exterior/aurora_ext_TEX0.materials.json",
        )
        exterior.modifiers = [FakeModifier("StarBreaker Decal Offset", "DISPLACE", 0.005)]
        interior = FakeObject(
            "cooler",
            starbreaker_material_sidecar="Data/Objects/Spaceships/Coolers/small_component/lplt/cool_lplt_s01_pl03_TEX0.materials.json",
        )
        interior.modifiers = [FakeModifier("StarBreaker Decal Offset", "DISPLACE", 0.001)]
        other_root = FakeObject("StarBreaker MISC Razor", starbreaker_package_root=True)
        other = FakeObject(
            "other_hull",
            starbreaker_material_sidecar="Data/Objects/Spaceships/Ships/MISC/razor/exterior/razor_ext_TEX0.materials.json",
        )
        other.modifiers = [FakeModifier("StarBreaker Decal Offset", "DISPLACE", 0.005)]
        exterior.parent = package_root
        interior.parent = package_root
        other.parent = other_root
        package_root.children.extend([exterior, interior])
        other_root.children.append(other)

        updated = self.package_ops.apply_decal_offsets_to_package_root(package_root, 0.0075, 0.0025)

        self.assertEqual(updated, 2)
        self.assertAlmostEqual(exterior.modifiers[0].strength, 0.0075)
        self.assertAlmostEqual(interior.modifiers[0].strength, 0.0025)
        self.assertAlmostEqual(other.modifiers[0].strength, 0.005)

    def test_apply_decal_offsets_uses_instance_sidecar_when_object_prop_missing(self) -> None:
        package_root = FakeObject("StarBreaker RSI Aurora", starbreaker_package_root=True)
        child = FakeObject(
            "weapon",
            starbreaker_instance_json=json.dumps(
                {
                    "material_sidecar": "Data/Objects/Spaceships/Weapons/KLWE/KLWE_las_rep_s1-3_TEX0.materials.json",
                }
            ),
        )
        child.modifiers = [FakeModifier("StarBreaker Decal Offset", "DISPLACE", 0.001)]
        child.parent = package_root
        package_root.children.append(child)

        updated = self.package_ops.apply_decal_offsets_to_package_root(package_root, 0.009, 0.003)

        self.assertEqual(updated, 1)
        self.assertAlmostEqual(child.modifiers[0].strength, 0.003)

    def test_material_refresh_session_splits_orphan_cleanup_across_steps(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject("hull", starbreaker_material_sidecar="hull.materials.json")
        mesh.type = "MESH"
        mesh.data = FakeMeshData(FakeUVLayers(["UVMap"], active_index=-1))
        mesh.parent = package_root
        package_root.children.append(mesh)

        cleanup_events: list[str] = []
        clock = {"value": 0.0}

        def _cleanup_step(name: str):
            def _run() -> int:
                cleanup_events.append(name)
                clock["value"] += 1.0
                return 1

            return _run

        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        original_perf_counter = self.package_ops.time.perf_counter
        original_purges = (
            self.package_ops._purge_orphaned_managed_materials,
            self.package_ops._purge_orphaned_runtime_groups,
            self.package_ops._purge_orphaned_runtime_actions,
            self.package_ops._purge_orphaned_file_backed_images,
        )
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            self.package_ops.time.perf_counter = lambda: clock["value"]
            self.package_ops._purge_orphaned_managed_materials = _cleanup_step("materials")
            self.package_ops._purge_orphaned_runtime_groups = _cleanup_step("groups")
            self.package_ops._purge_orphaned_runtime_actions = _cleanup_step("actions")
            self.package_ops._purge_orphaned_file_backed_images = _cleanup_step("images")

            context = types.SimpleNamespace(view_layer=FakeViewLayer())
            session = self.package_ops.MaterialRefreshSession(context, package_root)

            self.assertFalse(session.step(budget_seconds=0.1, min_objects=1))
            self.assertEqual(cleanup_events, ["materials"])
            self.assertFalse(session.done)
            self.assertLess(session.progress, 1.0)

            self.assertFalse(session.step(budget_seconds=0.1, min_objects=1))
            self.assertEqual(cleanup_events, ["materials", "groups"])

            self.assertFalse(session.step(budget_seconds=0.1, min_objects=1))
            self.assertEqual(cleanup_events, ["materials", "groups", "actions"])

            self.assertTrue(session.step(budget_seconds=0.1, min_objects=1))
            self.assertEqual(cleanup_events, ["materials", "groups", "actions", "images"])
            self.assertEqual(session.progress, 1.0)
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode
            self.package_ops.time.perf_counter = original_perf_counter
            (
                self.package_ops._purge_orphaned_managed_materials,
                self.package_ops._purge_orphaned_runtime_groups,
                self.package_ops._purge_orphaned_runtime_actions,
                self.package_ops._purge_orphaned_file_backed_images,
            ) = original_purges

    def test_material_refresh_session_flushes_importer_orphan_queue(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )

        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            context = types.SimpleNamespace(view_layer=FakeViewLayer())
            session = self.package_ops.MaterialRefreshSession(
                context,
                package_root,
                purge_orphans=False,
            )
            flushed = []
            session.importer._flush_pending_orphan_materials = lambda: flushed.append(True)

            self.assertTrue(session.step())
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(flushed, [True])

    def test_refresh_materials_uses_object_palette_without_explicit_override(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
            starbreaker_palette_id="palette/root",
        )
        mesh = FakeObject(
            "interior_panel",
            starbreaker_material_sidecar="interior.materials.json",
            starbreaker_palette_id="palette/interior",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(types.SimpleNamespace(), package_root)
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "interior_panel", "palette/interior")])
        self.assertEqual(package_root["starbreaker_palette_id"], "palette/root")
        self.assertEqual(mesh["starbreaker_palette_id"], "palette/interior")

    def test_refresh_materials_uses_sidecar_default_palette_when_object_palette_missing(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject(
            "interior_panel",
            starbreaker_material_sidecar="interior.materials.json",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)
        sidecar = types.SimpleNamespace(
            raw={
                "authored_material_set": {
                    "attributes": [
                        {
                            "name": "DefaultPalette",
                            "value": "Libs/Foundry/Records/TintPalettes/Brand/RSI/rsi_interior/rsi_interior_default",
                        }
                    ]
                }
            }
        )
        fake_package = types.SimpleNamespace(
            scene_path=Path("/tmp/aurora/scene.json"),
            package_name="Test Package",
            load_material_sidecar=lambda _path: sidecar,
        )

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_loader = self.package_ops._load_package_from_root
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._load_package_from_root = lambda _root: fake_package
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(types.SimpleNamespace(), package_root)
        finally:
            self.package_ops._load_package_from_root = original_loader
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "interior_panel", "palette/rsi_interior_default")])

    def test_refresh_materials_sets_active_uv_without_localizing_mesh(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Aurora",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/aurora/scene.json",
        )
        mesh = FakeObject("wing", starbreaker_material_sidecar="wing.materials.json")
        mesh.type = "MESH"
        mesh.data = FakeMeshData(FakeUVLayers(["UVMap.001", "UVMap"]))
        mesh.update_tags: list[object] = []
        mesh.update_tag = lambda refresh=None: mesh.update_tags.append(refresh)
        mesh.parent = package_root
        package_root.children.append(mesh)
        view_layer = FakeViewLayer()

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(view_layer=view_layer),
                package_root,
            )
        finally:
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(mesh.data.uv_layers.active.name, "UVMap")
        self.assertTrue(mesh.data.uv_layers.active.active_render)
        self.assertEqual(mesh.data.update_tags, [{"DATA"}])
        self.assertEqual(mesh.update_tags, [{"DATA"}])
        self.assertEqual(view_layer.update_count, 1)

    def test_package_root_needs_material_refresh_for_empty_or_linked_slots(self) -> None:
        package_root = FakeObject("Root", starbreaker_package_root=True)
        mesh = FakeObject("mesh", starbreaker_material_sidecar="mesh.materials.json")
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)
        sidecar = types.SimpleNamespace(
            submaterials=[types.SimpleNamespace(index=0, submaterial_name="local")]
        )
        package = types.SimpleNamespace(load_material_sidecar=lambda _path: sidecar)
        original_load = self.package_ops._load_package_from_root
        try:
            self.package_ops._load_package_from_root = lambda _root: package

            self.assertTrue(self.package_ops.package_root_needs_material_refresh(package_root))

            mesh.material_slots = [
                FakeSlot(FakeMaterial("mesh_mtl_local_00", library=object(), starbreaker_material_identity="id"))
            ]
            self.assertTrue(self.package_ops.package_root_needs_material_refresh(package_root))

            mesh.material_slots = [FakeSlot(FakeMaterial("mesh_mtl_local_00"))]
            self.assertTrue(self.package_ops.package_root_needs_material_refresh(package_root))

            local_built = FakeMaterial("local_built")
            local_built.node_tree = FakeNodeTree(1.0)
            mesh.material_slots = [FakeSlot(local_built)]
            self.assertFalse(self.package_ops.package_root_needs_material_refresh(package_root))

            local_managed = FakeMaterial("local_managed", starbreaker_material_identity="id")
            local_managed.node_tree = FakeNodeTree(1.0)
            mesh.material_slots = [FakeSlot(local_managed)]
            self.assertFalse(self.package_ops.package_root_needs_material_refresh(package_root))
        finally:
            self.package_ops._load_package_from_root = original_load

    def test_package_root_needs_material_refresh_ignores_unmappable_placeholder(self) -> None:
        package_root = FakeObject("Root", starbreaker_package_root=True)
        mesh = FakeObject("mesh", starbreaker_material_sidecar="mesh.materials.json")
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)
        mesh.material_slots = [FakeSlot(FakeMaterial("mesh_mtl_material_0_00"))]
        sidecar = types.SimpleNamespace(
            submaterials=[types.SimpleNamespace(index=0, submaterial_name="ActualMaterial")]
        )
        package = types.SimpleNamespace(load_material_sidecar=lambda _path: sidecar)
        original_load = self.package_ops._load_package_from_root
        try:
            self.package_ops._load_package_from_root = lambda _root: package
            self.assertFalse(self.package_ops.package_root_needs_material_refresh(package_root))
        finally:
            self.package_ops._load_package_from_root = original_load

    def test_refresh_materials_for_package_root_only_unloaded_skips_built_meshes(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker Test",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/test/scene.json",
        )
        dirty_mesh = FakeObject("dirty", starbreaker_material_sidecar="mesh.materials.json")
        dirty_mesh.type = "MESH"
        dirty_mesh.data = FakeMeshData(FakeUVLayers(["UVMap"]))
        dirty_mesh.parent = package_root
        dirty_mesh.material_slots = [FakeSlot(FakeMaterial("mesh_mtl_Mat_A_00"))]
        package_root.children.append(dirty_mesh)

        built_mesh = FakeObject("built", starbreaker_material_sidecar="mesh.materials.json")
        built_mesh.type = "MESH"
        built_mesh.data = FakeMeshData(FakeUVLayers(["UVMap"]))
        built_mesh.parent = package_root
        built_material = FakeMaterial("mesh_mtl_Mat_A_00")
        built_material.node_tree = FakeNodeTree(1.0)
        built_mesh.material_slots = [FakeSlot(built_material)]
        package_root.children.append(built_mesh)

        sidecar = types.SimpleNamespace(
            submaterials=[types.SimpleNamespace(index=0, submaterial_name="Mat_A")]
        )
        package = types.SimpleNamespace(
            scene_path=Path("/tmp/test/scene.json"),
            package_name="Test Package",
            load_material_sidecar=lambda _path: sidecar,
        )
        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        original_load = self.package_ops._load_package_from_root
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            self.package_ops._load_package_from_root = lambda _root: package
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            applied = self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(view_layer=FakeViewLayer()),
                package_root,
                only_unloaded=True,
            )
        finally:
            self.package_ops._load_package_from_root = original_load
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(importer_stub.events, [("rebuild", "dirty", None)])

    def test_refresh_materials_for_package_root_reports_coarse_progress(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker Test",
            starbreaker_package_root=True,
            starbreaker_scene_path="/tmp/test/scene.json",
        )
        meshes = []
        for name in ("mesh_a", "mesh_b", "mesh_c"):
            mesh = FakeObject(name, starbreaker_material_sidecar=f"{name}.materials.json")
            mesh.type = "MESH"
            mesh.data = FakeMeshData(FakeUVLayers(["UVMap"]))
            mesh.parent = package_root
            package_root.children.append(mesh)
            meshes.append(mesh)

        package = types.SimpleNamespace(
            scene_path=Path("/tmp/test/scene.json"),
            package_name="Test Package",
            load_material_sidecar=lambda path: types.SimpleNamespace(submaterials=[types.SimpleNamespace(index=0, submaterial_name=path)]),
        )
        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        progress_updates: list[tuple[float, str]] = []
        clock_values = iter([0.0, 1.0, 2.0, 6.0, 7.0, 8.0, 9.0, 10.0])
        original_load = self.package_ops._load_package_from_root
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        original_monotonic = self.package_ops.time.monotonic
        try:
            self.package_ops._load_package_from_root = lambda _root: package
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            self.package_ops.time.monotonic = lambda: next(clock_values)
            applied = self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(view_layer=FakeViewLayer()),
                package_root,
                progress_callback=lambda fraction, description: progress_updates.append((fraction, description)),
                progress_interval_seconds=5.0,
            )
        finally:
            self.package_ops._load_package_from_root = original_load
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode
            self.package_ops.time.monotonic = original_monotonic

        self.assertEqual(applied, 3)
        self.assertEqual(
            [description for _, description in progress_updates],
            [
                "Refreshing 0/3 objects",
                "Refreshing 3/3 objects",
                "Cleaning up...",
                "Done",
            ],
        )

    def test_apply_engine_glow_to_package_root_updates_targeted_materials(self) -> None:
        package_root = FakeObject(
            "Root",
            starbreaker_package_root=True,
            starbreaker_engine_glow_control=json.dumps(
                {
                    "targets": [
                        {
                            "geometry_path": "Data/Objects/Ships/Test/thruster.cga",
                            "mesh_asset": "Data/Objects/Ships/Test/thruster_LOD0.blend",
                            "material_sidecar": "Data/Objects/Ships/Test/root.materials.json",
                            "source_material_index": 4,
                        }
                    ]
                }
            ),
        )
        mesh = FakeObject("mesh")
        mesh.type = "MESH"
        mesh.parent = package_root
        mesh["starbreaker_instance_json"] = json.dumps(
            {
                "entity_name": "Test_Thruster",
                "geometry_path": None,
                "mesh_asset": "Data/Objects/Ships/Test/thruster_LOD0.blend",
            }
        )
        package_root.children.append(mesh)
        material = FakeMaterial(
            "Glow",
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
            starbreaker_submaterial_json=json.dumps(
                {
                    "index": 4,
                    "authored_attributes": [
                        {"name": "Emissive", "value": "0.25,0.5,0.75"},
                    ],
                }
            ),
        )
        material.node_tree = FakeNodeTree(2.0)
        mesh.material_slots = [FakeSlot(material)]
        mesh_2 = FakeObject("mesh_2")
        mesh_2.type = "MESH"
        mesh_2.parent = package_root
        mesh_2["starbreaker_instance_json"] = mesh["starbreaker_instance_json"]
        mesh_2.material_slots = [FakeSlot(material)]
        package_root.children.append(mesh_2)

        other_mesh = FakeObject("other_mesh")
        other_mesh.type = "MESH"
        other_mesh.parent = package_root
        other_mesh["starbreaker_instance_json"] = json.dumps(
            {
                "entity_name": "Test_Hull",
                "geometry_path": "Data/Objects/Ships/Test/hull.cga",
                "mesh_asset": "Data/Objects/Ships/Test/hull.blend",
            }
        )
        other_mesh.material_slots = [FakeSlot(material)]
        package_root.children.append(other_mesh)

        updated = self.package_ops.apply_engine_glow_to_package_root(package_root, 150.0)

        self.assertEqual(updated, 1)
        self.assertIsNot(mesh.material_slots[0].material, material)
        self.assertIs(mesh.material_slots[0].material, mesh_2.material_slots[0].material)
        self.assertIs(other_mesh.material_slots[0].material, material)
        self.assertAlmostEqual(mesh.material_slots[0].material.node_tree.nodes[0].inputs["Emission Strength"].default_value, 150.0)
        self.assertEqual(
            mesh.material_slots[0].material.node_tree.nodes[0].inputs["Emission Color"].default_value,
            (0.25, 0.5, 0.75, 1.0),
        )
        self.assertEqual(
            mesh.material_slots[0].material.node_tree.nodes[1].inputs["Palette Color"].default_value,
            (0.0, 0.0, 0.0, 1.0),
        )
        self.assertAlmostEqual(float(package_root.get("starbreaker_engine_glow_strength")), 150.0)

        self.package_ops.apply_engine_glow_to_package_root(package_root, 0.0)
        self.assertEqual(
            mesh.material_slots[0].material.node_tree.nodes[0].inputs["Emission Color"].default_value,
            (0.25, 0.5, 0.75, 1.0),
        )

    def test_shared_glow_control_enabled_discovers_mesh_decal_glow_targets(self) -> None:
        package_root = FakeObject("Root", starbreaker_package_root=True)
        mesh = FakeObject(
            "mesh",
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        package_root.children.append(mesh)
        sidecar = types.SimpleNamespace(
            submaterials=[
                types.SimpleNamespace(
                    index=31,
                    submaterial_name="Decal_Glow_Linked",
                    blender_material_name="test:Decal_Glow_Linked",
                    shader_family="MeshDecal",
                    decoded_feature_flags=types.SimpleNamespace(
                        has_parallax_occlusion_mapping=False,
                        has_stencil_map=False,
                    ),
                    texture_slots=[
                        types.SimpleNamespace(
                            slot="TexSlot1",
                            export_path="Data/Textures/Ships/Test/Glows/glow_diff.png",
                        )
                    ],
                ),
                types.SimpleNamespace(
                    index=53,
                    submaterial_name="Glow_Thrusters",
                    blender_material_name="test:Glow_Thrusters",
                    shader_family="HardSurface",
                    decoded_feature_flags=types.SimpleNamespace(
                        has_parallax_occlusion_mapping=True,
                        has_stencil_map=False,
                    ),
                    texture_slots=[],
                ),
            ]
        )
        package = types.SimpleNamespace(load_material_sidecar=lambda _path: sidecar)

        original_load = self.package_ops._load_package_from_root
        try:
            self.package_ops._load_package_from_root = lambda _root: package
            enabled = self.package_ops.shared_glow_control_enabled(package_root)
        finally:
            self.package_ops._load_package_from_root = original_load

        self.assertTrue(enabled)
        payload = json.loads(package_root["starbreaker_shared_glow_control"])
        self.assertEqual(payload["default_strength"], 0.0)
        self.assertEqual(payload["targets"], [
            {
                "blender_material_name": "test:Decal_Glow_Linked",
                "material_sidecar": "Data/Objects/Ships/Test/root.materials.json",
                "source_material_index": 31,
                "submaterial_name": "Decal_Glow_Linked",
            }
        ])

    def test_apply_shared_glow_to_package_root_updates_only_shared_glow_decals(self) -> None:
        package_root = FakeObject("Root", starbreaker_package_root=True)
        target_material = FakeMaterial("drak_pitbull_ext:Decal_Glow_Unlinked#cf6835ab")
        target_material.node_tree = FakeNodeTree(0.16)
        thruster_material = FakeMaterial("drak_pitbull_ext:Glow_Thrusters__engine_glow")
        thruster_material.node_tree = FakeNodeTree(0.0)

        mesh = FakeObject(
            "mesh",
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        mesh.material_slots = [FakeSlot(target_material)]
        package_root.children.append(mesh)

        mesh_2 = FakeObject(
            "mesh_2",
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
        )
        mesh_2.type = "MESH"
        mesh_2.parent = package_root
        mesh_2.material_slots = [FakeSlot(target_material)]
        package_root.children.append(mesh_2)

        thruster_mesh = FakeObject(
            "thruster_mesh",
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
        )
        thruster_mesh.type = "MESH"
        thruster_mesh.parent = package_root
        thruster_mesh.material_slots = [FakeSlot(thruster_material)]
        package_root.children.append(thruster_mesh)

        sidecar = types.SimpleNamespace(
            submaterials=[
                types.SimpleNamespace(
                    index=32,
                    submaterial_name="Decal_Glow_Unlinked",
                    blender_material_name="test:Decal_Glow_Unlinked",
                    shader_family="MeshDecal",
                    decoded_feature_flags=types.SimpleNamespace(
                        has_parallax_occlusion_mapping=False,
                        has_stencil_map=False,
                    ),
                    texture_slots=[
                        types.SimpleNamespace(
                            slot="TexSlot1",
                            export_path="Data/Textures/Ships/Test/Glows/glow_diff.png",
                        )
                    ],
                    raw={"authored_attributes": [{"name": "Glow", "value": "0.16"}]},
                ),
                types.SimpleNamespace(
                    index=53,
                    submaterial_name="Glow_Thrusters",
                    blender_material_name="test:Glow_Thrusters",
                    shader_family="HardSurface",
                    decoded_feature_flags=types.SimpleNamespace(
                        has_parallax_occlusion_mapping=True,
                        has_stencil_map=False,
                    ),
                    texture_slots=[],
                    raw={"authored_attributes": []},
                ),
            ]
        )
        package = types.SimpleNamespace(load_material_sidecar=lambda _path: sidecar)

        original_load = self.package_ops._load_package_from_root
        try:
            self.package_ops._load_package_from_root = lambda _root: package
            updated = self.package_ops.apply_shared_glow_to_package_root(package_root, 2.0)
        finally:
            self.package_ops._load_package_from_root = original_load

        self.assertEqual(updated, 1)
        self.assertIsNot(mesh.material_slots[0].material, target_material)
        self.assertIs(mesh.material_slots[0].material, mesh_2.material_slots[0].material)
        self.assertIs(thruster_mesh.material_slots[0].material, thruster_material)
        self.assertAlmostEqual(
            mesh.material_slots[0].material.node_tree.nodes[0].inputs["Emission Strength"].default_value,
            2.16,
        )
        self.assertAlmostEqual(
            thruster_mesh.material_slots[0].material.node_tree.nodes[0].inputs["Emission Strength"].default_value,
            0.0,
        )
        self.assertAlmostEqual(float(package_root.get("starbreaker_shared_glow_strength")), 2.0)

    def test_refresh_materials_for_package_root_reapplies_shared_glow(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "Root",
            starbreaker_package_root=True,
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
        )
        mesh = FakeObject(
            "mesh",
            starbreaker_material_sidecar="Data/Objects/Ships/Test/root.materials.json",
        )
        mesh.type = "MESH"
        mesh.parent = package_root
        mesh.material_slots = [FakeSlot(FakeMaterial("mat"))]
        package_root.children.append(mesh)
        package = types.SimpleNamespace(
            scene_path=Path("/tmp/test_scene.json"),
            package_name="Test Package",
            load_material_sidecar=lambda path: types.SimpleNamespace(submaterials=[types.SimpleNamespace(index=0, submaterial_name=path)]),
        )
        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        importer_stub.events = []
        shared_glow_calls: list[tuple[object, float]] = []

        original_load = self.package_ops._load_package_from_root
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        original_engine_enabled = self.package_ops.engine_glow_control_enabled
        original_shared_enabled = self.package_ops.shared_glow_control_enabled
        original_shared_strength = self.package_ops.shared_glow_strength
        original_apply_shared = self.package_ops.apply_shared_glow_to_package_root
        try:
            self.package_ops._load_package_from_root = lambda _root: package
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode
            self.package_ops.engine_glow_control_enabled = lambda _root: False
            self.package_ops.shared_glow_control_enabled = lambda _root: True
            self.package_ops.shared_glow_strength = lambda _root: 2.5
            self.package_ops.apply_shared_glow_to_package_root = (
                lambda root, value: shared_glow_calls.append((root, value)) or 1
            )
            self.package_ops.refresh_materials_for_package_root(
                types.SimpleNamespace(view_layer=FakeViewLayer()),
                package_root,
            )
        finally:
            self.package_ops._load_package_from_root = original_load
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode
            self.package_ops.engine_glow_control_enabled = original_engine_enabled
            self.package_ops.shared_glow_control_enabled = original_shared_enabled
            self.package_ops.shared_glow_strength = original_shared_strength
            self.package_ops.apply_shared_glow_to_package_root = original_apply_shared

        self.assertEqual(shared_glow_calls, [(package_root, 2.5)])

    def test_resolve_package_relative_scene_path_from_opened_blend_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scene_path = root / "Packages" / "RSI Aurora Mk2_LOD0_TEX0" / "scene.json"
            scene_path.parent.mkdir(parents=True)
            scene_path.write_text("{}", encoding="utf-8")
            self.bpy.data.filepath = str(scene_path.with_suffix(".blend"))
            package_root = FakeObject(
                "StarBreaker RSI Aurora",
                starbreaker_package_root=True,
                starbreaker_scene_path="Packages/RSI Aurora Mk2_LOD0_TEX0/scene.json",
            )

            resolved = self.package_ops.resolve_package_scene_path(package_root)

            self.assertEqual(resolved, scene_path)
        self.bpy.data.filepath = ""

    def test_apply_paint_to_package_root_restores_base_sidecar_when_leaving_variant(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker RSI Scorpius",
            starbreaker_paint_variant_sidecar="variant.materials.json",
            starbreaker_palette_id="palette/skull",
        )
        package_root.type = "EMPTY"
        child = FakeObject(
            "livery_decal_body",
            starbreaker_material_sidecar="variant.materials.json",
            starbreaker_instance_json=json.dumps({"material_sidecar": "base.materials.json"}),
        )
        child.type = "MESH"
        child.parent = package_root
        package_root.children.append(child)

        fake_package = types.SimpleNamespace(
            paints={},
            liveries={"default": types.SimpleNamespace(material_sidecars=["base.materials.json"])},
            scene=types.SimpleNamespace(root_entity=types.SimpleNamespace(material_sidecar="base.materials.json")),
        )

        rebuild_calls: list[tuple[str, str | None, str | None]] = []

        class FakeImporter:
            def __init__(self, context, package, package_root=None, create_template_collection=True):
                self.context = context
                self.package = package
                self.package_root = package_root

            def rebuild_object_materials(self, obj, palette_id):
                rebuild_calls.append((obj.name, obj.get("starbreaker_material_sidecar"), palette_id))
                return 1

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        original_importer = importer_stub.PackageImporter
        original_loader = self.package_ops._load_package_from_root
        original_scene_instance = self.package_ops._scene_instance_from_object
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            importer_stub.PackageImporter = FakeImporter
            self.package_ops._load_package_from_root = lambda _root: fake_package
            self.package_ops._scene_instance_from_object = lambda obj: types.SimpleNamespace(material_sidecar="base.materials.json")
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode

            applied = self.package_ops.apply_paint_to_package_root(
                types.SimpleNamespace(),
                package_root,
                "palette/rsi_scorpius",
            )
        finally:
            importer_stub.PackageImporter = original_importer
            self.package_ops._load_package_from_root = original_loader
            self.package_ops._scene_instance_from_object = original_scene_instance
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(child.get("starbreaker_material_sidecar"), "base.materials.json")
        self.assertNotIn("starbreaker_paint_variant_sidecar", package_root)
        self.assertEqual(rebuild_calls, [("livery_decal_body", "base.materials.json", "palette/rsi_scorpius")])

    def test_apply_livery_to_package_root_exits_active_paint_variant(self) -> None:
        @contextmanager
        def _no_suspend(_context):
            yield

        @contextmanager
        def _no_mode(_context):
            yield

        package_root = FakeObject(
            "StarBreaker Behr P6LR",
            starbreaker_paint_variant_sidecar="variant.materials.json",
            starbreaker_palette_id="palette/store01",
        )
        package_root.type = "EMPTY"
        child = FakeObject(
            "p6lr_body",
            starbreaker_material_sidecar="variant.materials.json",
            starbreaker_instance_json=json.dumps({"material_sidecar": "base.materials.json"}),
        )
        child.type = "MESH"
        child.parent = package_root
        package_root.children.append(child)

        fake_package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(
                    material_sidecar="base.materials.json",
                    palette_id="palette/base",
                )
            )
        )

        resolver_calls: list[tuple[str, str | None]] = []
        rebuild_calls: list[tuple[str, str | None, str | None]] = []

        def _resolver(_package, livery_id, _instance, sidecar):
            resolver_calls.append((livery_id, sidecar))
            return "palette/base"

        class FakeImporter:
            def __init__(self, context, package, package_root=None, create_template_collection=True):
                self.context = context
                self.package = package
                self.package_root = package_root

            def rebuild_object_materials(self, obj, palette_id):
                rebuild_calls.append((obj.name, obj.get("starbreaker_material_sidecar"), palette_id))
                return 1

        importer_stub = sys.modules["sb_pkg_test_runtime.importer"]
        original_importer = importer_stub.PackageImporter
        original_loader = self.package_ops._load_package_from_root
        original_resolver = self.package_ops.palette_id_for_livery_instance
        original_scene_instance = self.package_ops._scene_instance_from_object
        original_suspend = self.package_ops._suspend_heavy_viewports
        original_mode = self.package_ops._temporary_object_mode
        try:
            importer_stub.PackageImporter = FakeImporter
            self.package_ops._load_package_from_root = lambda _root: fake_package
            self.package_ops.palette_id_for_livery_instance = _resolver
            self.package_ops._scene_instance_from_object = (
                lambda obj: types.SimpleNamespace(material_sidecar="base.materials.json")
                if obj is child
                else None
            )
            self.package_ops._suspend_heavy_viewports = _no_suspend
            self.package_ops._temporary_object_mode = _no_mode

            applied = self.package_ops.apply_livery_to_package_root(
                types.SimpleNamespace(),
                package_root,
                "palette/behr_sniper_ballistic_01",
            )
        finally:
            importer_stub.PackageImporter = original_importer
            self.package_ops._load_package_from_root = original_loader
            self.package_ops.palette_id_for_livery_instance = original_resolver
            self.package_ops._scene_instance_from_object = original_scene_instance
            self.package_ops._suspend_heavy_viewports = original_suspend
            self.package_ops._temporary_object_mode = original_mode

        self.assertEqual(applied, 1)
        self.assertEqual(child.get("starbreaker_material_sidecar"), "base.materials.json")
        self.assertNotIn("starbreaker_paint_variant_sidecar", package_root)
        self.assertEqual(
            resolver_calls,
            [
                ("palette/behr_sniper_ballistic_01", "base.materials.json"),
                ("palette/behr_sniper_ballistic_01", "base.materials.json"),
            ],
        )
        self.assertEqual(rebuild_calls, [("p6lr_body", "base.materials.json", "palette/base")])


class AnimationDisplayNameTests(unittest.TestCase):
    """Tests for _animation_display_name and _entity_name_prefix."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.package_ops, _ = _load_package_ops()

    def _clip(self, name: str, **extra) -> dict:
        return {"name": name, **extra}

    def test_localized_name_takes_priority_over_raw_clip_name(self) -> None:
        clip = self._clip("test_ship_vtol_deploy", localized_name="VTOL Deploy")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "VTOL Deploy")

    def test_entity_prefix_stripped_and_humanized(self) -> None:
        clip = self._clip("test_ship_vtol_deploy")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "Vtol Deploy")

    def test_entity_prefix_stripped_retract_variant(self) -> None:
        clip = self._clip("test_ship_vtol_retract")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "Vtol Retract")

    def test_no_entity_prefix_returns_raw_name(self) -> None:
        clip = self._clip("test_ship_vtol_deploy")
        result = self.package_ops._animation_display_name(clip)
        self.assertEqual(result, "test_ship_vtol_deploy")

    def test_prefix_mismatch_still_humanizes(self) -> None:
        # Prefix does not match clip name — entire name is humanized.
        clip = self._clip("other_ship_action")
        result = self.package_ops._animation_display_name(clip, entity_prefix="test_ship")
        self.assertEqual(result, "Other Ship Action")

    def test_entity_name_prefix_strips_class_namespace(self) -> None:
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(entity_name="EntityClassDefinition.Test_Ship")
            )
        )
        result = self.package_ops._entity_name_prefix(package)
        self.assertEqual(result, "test_ship")

    def test_entity_name_prefix_no_namespace(self) -> None:
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(entity_name="Other_Ship")
            )
        )
        result = self.package_ops._entity_name_prefix(package)
        self.assertEqual(result, "other_ship")

    def test_entity_name_prefix_missing_attribute_returns_empty(self) -> None:
        package = types.SimpleNamespace()
        result = self.package_ops._entity_name_prefix(package)
        self.assertEqual(result, "")

    def test_available_package_animation_items_strips_entity_prefix(self) -> None:
        """Non-fragment clips get entity prefix stripped and humanized."""
        clips = [
            {"name": "test_ship_vtol_deploy", "fps": 30, "frame_count": 60,
             "fragments": [], "sidecar": None, "start_position": None, "start_rotation": None},
            {"name": "test_ship_vtol_retract", "fps": 30, "frame_count": 60,
             "fragments": [], "sidecar": None, "start_position": None, "start_rotation": None},
        ]
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(
                    entity_name="EntityClassDefinition.Test_Ship",
                    raw={"animations": clips},
                )
            )
        )
        items = self.package_ops.available_package_animation_items(package)
        item_map = dict(items)
        self.assertEqual(item_map.get("test_ship_vtol_deploy"), "Vtol Deploy")
        self.assertEqual(item_map.get("test_ship_vtol_retract"), "Vtol Retract")

    def test_available_package_animation_items_frag_tags_override_clip_name(self) -> None:
        """Fragment clips with frag_tags use frag_tags-derived names regardless of prefix."""
        clips = [
            {
                "name": "test_ship_wings_deploy",
                "fps": 30, "frame_count": 120,
                "sidecar": None, "start_position": None, "start_rotation": None,
                "fragments": [{"fragment": "Wings", "frag_tags": ["Retract"],
                               "tags": [], "scopes": [], "animations": [],
                               "guid": "a", "option_weight": 1.0, "blend_out_duration": 0.2}],
            },
            {
                "name": "test_ship_wings_retract",
                "fps": 30, "frame_count": 120,
                "sidecar": None, "start_position": None, "start_rotation": None,
                "fragments": [{"fragment": "Wings", "frag_tags": ["Deploy"],
                               "tags": [], "scopes": [], "animations": [],
                               "guid": "b", "option_weight": 1.0, "blend_out_duration": 0.2}],
            },
        ]
        package = types.SimpleNamespace(
            scene=types.SimpleNamespace(
                root_entity=types.SimpleNamespace(
                    entity_name="EntityClassDefinition.Test_Ship",
                    raw={"animations": clips},
                )
            )
        )
        items = self.package_ops.available_package_animation_items(package)
        item_map = dict(items)
        self.assertEqual(item_map.get("fragment:0:test_ship_wings_deploy"), "Wings Retract")
        self.assertEqual(item_map.get("fragment:0:test_ship_wings_retract"), "Wings Deploy")

    def test_animation_insert_label_uses_fragment_display_name(self) -> None:
        clip = {
            "name": "test_ship_wings_retract",
            "fragments": [
                {
                    "fragment": "Wings",
                    "frag_tags": ["Deploy"],
                    "tags": [],
                    "scopes": [],
                    "animations": [],
                }
            ],
        }
        label = self.package_ops._animation_insert_label(
            "fragment:0:test_ship_wings_retract",
            clip,
        )
        self.assertEqual(label, "Wings Deploy")

    def test_animation_insert_label_falls_back_to_animation_key(self) -> None:
        clip = {"name": "test_ship_vtol_deploy", "fragments": []}
        label = self.package_ops._animation_insert_label("test_ship_vtol_deploy", clip)
        self.assertEqual(label, "test_ship_vtol_deploy")


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
