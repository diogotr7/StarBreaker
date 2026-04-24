from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path
from types import SimpleNamespace

ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


class _FakeObject(dict):
    def __init__(self, name: str, data: object | None):
        super().__init__()
        self.name = name
        self.data = data
        self.empty_display_type = None
        self.parent = None
        self.rotation_mode = None
        self.matrix_basis = None


def _load_orchestration() -> types.ModuleType:
    bpy = sys.modules.get("bpy")
    if bpy is None:
        bpy = types.ModuleType("bpy")
        sys.modules["bpy"] = bpy
    bpy.types = types.SimpleNamespace(Context=object, Object=object)

    class _Objects:
        def new(self, name: str, data: object | None):
            return _FakeObject(name, data)

    bpy.data = types.SimpleNamespace(objects=_Objects())

    mathutils = sys.modules.get("mathutils")
    if mathutils is None:
        mathutils = types.ModuleType("mathutils")
        sys.modules["mathutils"] = mathutils

    class _Euler:
        def __init__(self, *args, **kwargs):
            pass

        def to_quaternion(self):
            return SimpleNamespace()

    class _Matrix:
        def __init__(self, *args, **kwargs):
            self.args = args

        def inverted(self):
            return self

    class _Quaternion:
        def __init__(self, *args, **kwargs):
            pass

    mathutils.Euler = _Euler
    mathutils.Matrix = _Matrix
    mathutils.Quaternion = _Quaternion

    def _load(name: str, path: Path) -> types.ModuleType:
        spec = importlib.util.spec_from_file_location(name, str(path))
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
        return module

    constants = _load(
        "sb_scene_test_constants",
        ADDON_ROOT / "runtime" / "constants.py",
    )
    runtime_pkg = types.ModuleType("sb_scene_test_runtime")
    runtime_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
    sys.modules["sb_scene_test_runtime"] = runtime_pkg
    sys.modules["sb_scene_test_runtime.constants"] = constants

    importer_pkg = types.ModuleType("sb_scene_test_runtime.importer")
    importer_pkg.__path__ = [str(ADDON_ROOT / "runtime" / "importer")]
    sys.modules["sb_scene_test_runtime.importer"] = importer_pkg

    addon_pkg = types.ModuleType("sb_scene_test_addon")
    addon_pkg.__path__ = [str(ADDON_ROOT)]
    sys.modules["sb_scene_test_addon"] = addon_pkg

    package_ops_stub = types.ModuleType("sb_scene_test_runtime.package_ops")
    package_ops_stub._effective_exterior_material_sidecars = lambda *args, **kwargs: None
    package_ops_stub._exterior_material_sidecars = lambda *args, **kwargs: None
    package_ops_stub._paint_variant_for_palette_id = lambda *args, **kwargs: None
    package_ops_stub._string_prop = lambda *args, **kwargs: None
    sys.modules["sb_scene_test_runtime.package_ops"] = package_ops_stub

    for submodule, names in {
        "manifest": ("MaterialSidecar", "PackageBundle", "SceneInstanceRecord", "SubmaterialRecord"),
        "material_contract": ("TemplateContract",),
        "palette": ("palette_for_id", "resolved_palette_id"),
    }.items():
        stub = types.ModuleType(f"sb_scene_test_addon.{submodule}")
        for name in names:
            if name[0].islower():
                setattr(stub, name, lambda *args, **kwargs: None)
            else:
                setattr(stub, name, type(name, (), {}))
        sys.modules[f"sb_scene_test_addon.{submodule}"] = stub

    for submodule, class_name in {
        "builders": "BuildersMixin",
        "decals": "DecalsMixin",
        "groups": "GroupsMixin",
        "layers": "LayersMixin",
        "materials": "MaterialsMixin",
        "palette": "PaletteMixin",
    }.items():
        stub = types.ModuleType(f"sb_scene_test_runtime.importer.{submodule}")
        setattr(stub, class_name, type(class_name, (), {}))
        sys.modules[f"sb_scene_test_runtime.importer.{submodule}"] = stub

    types_stub = types.ModuleType("sb_scene_test_runtime.importer.types")
    types_stub.ImportedTemplate = type("ImportedTemplate", (), {})
    types_stub._bake_bitangent_sign_attribute = lambda *args, **kwargs: None
    sys.modules["sb_scene_test_runtime.importer.types"] = types_stub

    utils_stub = types.ModuleType("sb_scene_test_runtime.importer.utils")
    utils_stub._canonical_material_sidecar_path = lambda *args, **kwargs: None
    utils_stub._canonical_source_name = lambda *args, **kwargs: None
    utils_stub._remapped_submaterial_for_slot = lambda *args, **kwargs: None
    utils_stub._scene_attachment_offset_to_blender = lambda *args, **kwargs: None
    utils_stub._scene_light_quaternion_to_blender = lambda *args, **kwargs: None
    utils_stub._scene_matrix_to_blender = lambda matrix: ("converted", matrix)
    utils_stub._scene_position_to_blender = lambda *args, **kwargs: None
    utils_stub._slot_mapping_for_object = lambda *args, **kwargs: None
    utils_stub._slot_mapping_source_sidecar_path = lambda *args, **kwargs: None
    utils_stub._should_neutralize_axis_root = lambda *args, **kwargs: False
    utils_stub._unique_submaterials_by_name = lambda *args, **kwargs: []
    sys.modules["sb_scene_test_runtime.importer.utils"] = utils_stub

    source = (ADDON_ROOT / "runtime" / "importer" / "orchestration.py").read_text()
    source = source.replace("from ...", "from sb_scene_test_addon.")
    source = source.replace("from ..constants import", "from sb_scene_test_runtime.constants import")
    source = source.replace("from ..package_ops import", "from sb_scene_test_runtime.package_ops import")
    source = source.replace("from .", "from sb_scene_test_runtime.importer.")
    module = types.ModuleType("sb_scene_test_runtime.importer.orchestration")
    module.__file__ = str(ADDON_ROOT / "runtime" / "importer" / "orchestration.py")
    exec(compile(source, module.__file__, "exec"), module.__dict__)
    return module


class InstantiateSceneInstanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.orchestration = _load_orchestration()

    def test_resolved_no_rotation_keeps_parent_node_parenting(self) -> None:
        linked: list[object] = []

        class _Importer:
            def __init__(self) -> None:
                self.collection = SimpleNamespace(objects=SimpleNamespace(link=linked.append))

            def _effective_palette_id(self, palette_id):
                return palette_id

            def _palette_id_for_instance(self, record):
                return None

            def ensure_template(self, mesh_asset):
                raise RuntimeError("missing template")

            def _apply_instance_metadata(self, objects, record, effective_palette_id):
                self.metadata = (objects, record, effective_palette_id)

        importer = _Importer()
        parent = SimpleNamespace(name="root")
        parent_node = SimpleNamespace(name="hardpoint")
        record = SimpleNamespace(
            entity_name="child",
            resolved_no_rotation=True,
            local_transform_sc=((1.0, 0.0, 0.0, 0.0), (0.0, 1.0, 0.0, 0.0), (0.0, 0.0, 1.0, 0.0), (0.0, 0.0, 0.0, 1.0)),
            source_transform_basis="cryengine_z_up",
            offset_position=(0.0, 0.0, 0.0),
            offset_rotation=(0.0, 0.0, 0.0),
            no_rotation=True,
            mesh_asset="missing.glb",
        )

        anchor, clones = self.orchestration.OrchestrationMixin.instantiate_scene_instance(
            importer,
            record,
            parent,
            parent_node,
        )

        self.assertIs(anchor.parent, parent_node)
        self.assertEqual(anchor.matrix_basis, ("converted", record.local_transform_sc))
        self.assertEqual(clones, [anchor])
        self.assertEqual(linked, [anchor])


if __name__ == "__main__":  # pragma: no cover
    unittest.main()