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
        "skin": "SkinMixin",
        "hair": "HairMixin",
    }.items():
        stub = types.ModuleType(f"sb_scene_test_runtime.importer.{submodule}")
        setattr(stub, class_name, type(class_name, (), {}))
        sys.modules[f"sb_scene_test_runtime.importer.{submodule}"] = stub

    types_stub = types.ModuleType("sb_scene_test_runtime.importer.types")
    types_stub.ImportedTemplate = type("ImportedTemplate", (), {})
    types_stub._bake_bitangent_sign_attribute = lambda *args, **kwargs: None
    types_stub.BITANGENT_SIGN_ATTRIBUTE = "starbreaker_bitangent_sign"
    sys.modules["sb_scene_test_runtime.importer.types"] = types_stub

    utils_stub = types.ModuleType("sb_scene_test_runtime.importer.utils")
    utils_stub._canonical_material_sidecar_path = lambda *args, **kwargs: None
    utils_stub._canonical_source_name = lambda *args, **kwargs: None
    utils_stub._gltf_matrix_to_blender_basis = lambda matrix: ("gltf", matrix)
    utils_stub._remapped_submaterial_for_slot = lambda *args, **kwargs: None
    utils_stub._scene_attachment_offset_to_blender = lambda *args, **kwargs: None
    utils_stub._scene_light_quaternion_to_blender = lambda *args, **kwargs: None
    utils_stub._scene_matrix_to_blender = lambda matrix: ("converted", matrix)
    utils_stub._scene_position_to_blender = lambda *args, **kwargs: None
    utils_stub._slot_mapping_from_slot_names = lambda *args, **kwargs: None
    utils_stub._slot_mapping_for_object = lambda *args, **kwargs: None
    utils_stub._slot_mapping_source_sidecar_path = lambda *args, **kwargs: None
    utils_stub._slot_names_for_object = lambda *args, **kwargs: None
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

            def _flush_pending_view_layer_update(self):
                pass

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

    def test_gltf_transform_basis_uses_direct_matrix_payload(self) -> None:
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

            def _flush_pending_view_layer_update(self):
                pass

        importer = _Importer()
        parent = SimpleNamespace(name="root")
        parent_node = SimpleNamespace(name="hardpoint")
        record = SimpleNamespace(
            entity_name="child",
            resolved_no_rotation=False,
            local_transform_sc=((1.0, 0.0, 0.0, 0.0), (0.0, 1.0, 0.0, 0.0), (0.0, 0.0, 1.0, 0.0), (0.0, 0.0, 0.0, 1.0)),
            source_transform_basis="gltf_y_up",
            offset_position=(0.0, 0.0, 0.0),
            offset_rotation=(0.0, 0.0, 0.0),
            no_rotation=False,
            mesh_asset="missing.glb",
        )

        anchor, clones = self.orchestration.OrchestrationMixin.instantiate_scene_instance(
            importer,
            record,
            parent,
            parent_node,
        )

        self.assertIs(anchor.parent, parent_node)
        self.assertEqual(anchor.matrix_basis, ("gltf", record.local_transform_sc))
        self.assertEqual(clones, [anchor])
        self.assertEqual(linked, [anchor])





class EntityLightImportTests(unittest.TestCase):
    def test_entity_helper_frames_and_missing_helper_fallback(self):
        from starbreaker_addon.manifest import SceneInstanceRecord
        module = _load_orchestration()
        anchor, joint = object(), object()
        calls = []
        importer = SimpleNamespace(_index_nodes=lambda _: {"Joint": joint},
                                   create_light=lambda light, parent: calls.append((light, parent)))
        record = SceneInstanceRecord.from_value({"lights": [
            {"name": "attached", "parent_node_name": "joint", "position": [8,9,10],
             "direction_sc": [0,1,0], "parent_local_position": [1,2,3], "parent_local_rotation": [1,0,0,0]},
            {"name": "unresolved", "parent_node_name": "absent", "position": [4,5,6]},
        ]})
        module.import_entity_lights(importer, record, anchor, [])
        self.assertEqual(len(calls), 2)
        self.assertIs(calls[0][1], joint)
        self.assertEqual(calls[0][0].position, (1,2,3))
        self.assertEqual(calls[0][0].rotation, (1,0,0,0))
        self.assertIsNone(calls[0][0].direction_sc)
        self.assertIs(calls[1][1], anchor)
        self.assertEqual(calls[1][0].position, (4,5,6))

    def test_scene_import_keeps_root_child_and_interior_lights_once(self):
        from starbreaker_addon.manifest import SceneManifest
        module = _load_orchestration()
        scene = SceneManifest.from_value({"root_entity": {"entity_name": "root", "lights": [{"name": "root-light"}]},
            "children": [{"entity_name": "child", "lights": [{"name": "child-light"}]}],
            "interiors": [{"name": "room", "lights": [{"name": "interior-light"}]}]})
        calls = []
        noop = lambda *a, **k: None
        importer = SimpleNamespace(package=SimpleNamespace(scene=scene, package_name="synthetic"),
            package_root=_FakeObject("package", None), node_index_by_entity_name={},
            _create_package_root=lambda _: _FakeObject("package", None),
            _start_progress=noop, _ensure_runtime_shared_groups=noop, _advance_progress=noop,
            instantiate_scene_instance=lambda *a, **k: (_FakeObject("anchor", None), []),
            _index_nodes=lambda _: {}, _scene_root_parent=lambda _: None,
            create_light=lambda light, parent: calls.append(light.name),
            import_interior_container=lambda interior, _: calls.extend(l.name for l in interior.lights),
            _bake_pom_mesh_bitangent_signs=noop, _apply_deferred_pom_background_bias=noop,
            _flush_pending_view_layer_update=noop, _flush_pending_orphan_materials=noop, _emit_progress=noop)
        module.OrchestrationMixin.import_scene(importer, prefer_cycles=False)
        self.assertEqual(calls, ["root-light", "child-light", "interior-light"])


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
