from __future__ import annotations

import types
import unittest
from pathlib import Path
from types import SimpleNamespace

from tests.test_scene_instances import _load_orchestration


class _FakeObject(dict):
    _next_pointer = 1

    def __init__(self, name: str, data: object | None = None) -> None:
        super().__init__()
        self.name = name
        self.data = data
        self.parent = None
        self.children: list[_FakeObject] = []
        self.material_slots: list[object] = []
        self.users_collection: list[object] = []
        self.hide_render = False
        self.hidden = False
        self._pointer = _FakeObject._next_pointer
        _FakeObject._next_pointer += 1

    def as_pointer(self) -> int:
        return self._pointer

    def hide_set(self, value: bool) -> None:
        self.hidden = value


class _FakeMatrix:
    def copy(self):
        return self


class _FakeModifier:
    def __init__(self, name: str, type: str) -> None:  # noqa: A002 - matches bpy API
        self.name = name
        self.type = type
        self.strength = 0.005


class _CopyableFakeObject(_FakeObject):
    def __init__(self, name: str, data: object | None = None) -> None:
        super().__init__(name, data)
        self.matrix_basis = _FakeMatrix()
        self.dimensions = (0.05, 0.1, 0.2)
        self.modifiers: list[_FakeModifier] = []

    def copy(self):
        clone = _CopyableFakeObject(f"{self.name}.001", self.data)
        clone.modifiers = [_FakeModifier(modifier.name, modifier.type) for modifier in self.modifiers]
        for clone_modifier, source_modifier in zip(clone.modifiers, self.modifiers):
            clone_modifier.strength = source_modifier.strength
        return clone

    def animation_data_clear(self) -> None:
        pass


class _FakeObjectStore:
    def __init__(self) -> None:
        self._objects: list[_FakeObject] = []

    def __iter__(self):
        return iter(self._objects)

    def append(self, obj: _FakeObject) -> None:
        self._objects.append(obj)


class _FakeLinker:
    def __init__(self) -> None:
        self.linked: list[_FakeObject] = []

    def link(self, obj: _FakeObject) -> None:
        self.linked.append(obj)
        obj.users_collection.append(SimpleNamespace(objects=SimpleNamespace(unlink=lambda item: None)))


class _FakeBlendLibraryLoad:
    def __init__(self, objects: _FakeObjectStore) -> None:
        self.objects = objects
        self.data_from = SimpleNamespace(objects=["body", "barrel"])
        self.data_to = SimpleNamespace(objects=[])
        self.loaded_path: str | None = None

    def __call__(self, path: str, *, link: bool = False):
        self.loaded_path = path
        self.link = link
        return self

    def __enter__(self):
        return self.data_from, self.data_to

    def __exit__(self, exc_type, exc, tb) -> None:
        loaded = [_FakeObject(name) for name in self.data_to.objects]
        for obj in loaded:
            self.objects.append(obj)
        self.data_to.objects = loaded


class TemplateAssetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.orchestration = _load_orchestration()

    def test_blend_mesh_assets_load_from_blender_library_not_gltf(self) -> None:
        temp_dir = Path(__file__).resolve().parent / "__pycache__"
        temp_dir.mkdir(exist_ok=True)
        blend_path = temp_dir / "template_asset.blend"
        blend_path.write_bytes(b"BLENDER")

        object_store = _FakeObjectStore()
        blend_loader = _FakeBlendLibraryLoad(object_store)

        def fail_gltf(**_kwargs):
            raise AssertionError("blend mesh assets must not be imported through glTF")

        fake_bpy = SimpleNamespace(
            data=SimpleNamespace(
                objects=object_store,
                libraries=SimpleNamespace(load=blend_loader),
            ),
            ops=SimpleNamespace(import_scene=SimpleNamespace(gltf=fail_gltf)),
        )
        self.orchestration.bpy = fake_bpy
        self.orchestration.ImportedTemplate = lambda mesh_asset, root_names: SimpleNamespace(
            mesh_asset=mesh_asset,
            root_names=root_names,
        )
        self.orchestration._bake_bitangent_sign_attribute = lambda _mesh: False

        class _Package:
            def resolve_path(self, mesh_asset):
                self.mesh_asset = mesh_asset
                return blend_path

        class _Importer(self.orchestration.OrchestrationMixin):
            def __init__(self) -> None:
                self.package = _Package()
                self.template_cache = {}
                self.template_collection = SimpleNamespace(objects=_FakeLinker())
                self.cleared_objects = None
                self.purged_materials = None

            def _clear_template_material_bindings(self, objects):
                self.cleared_objects = objects

            def _purge_unused_materials(self, materials):
                self.purged_materials = materials

        importer = _Importer()

        template = self.orchestration.OrchestrationMixin.ensure_template(importer, "Data/Objects/template.blend")

        self.assertEqual(template.mesh_asset, "Data/Objects/template.blend")
        self.assertEqual(template.root_names, ["body", "barrel"])
        self.assertEqual(blend_loader.loaded_path, str(blend_path))
        self.assertFalse(blend_loader.link)
        self.assertEqual([obj.name for obj in importer.template_collection.objects.linked], ["body", "barrel"])
        self.assertTrue(all(obj.hidden and obj.hide_render for obj in importer.template_collection.objects.linked))
        self.assertEqual(importer.cleared_objects, importer.template_collection.objects.linked)
        self.assertEqual(importer.purged_materials, [])

    def test_duplicate_object_tree_normalizes_template_decal_offset_modifier(self) -> None:
        source = _CopyableFakeObject("root", data=object())
        source.modifiers = [
            _FakeModifier("StarBreaker Weld", "WELD"),
            _FakeModifier("StarBreaker Decal Offset", "DISPLACE"),
            _FakeModifier("Other Displace", "DISPLACE"),
        ]
        linked: list[_FakeObject] = []

        importer = SimpleNamespace(
            collection=SimpleNamespace(objects=SimpleNamespace(link=linked.append)),
            _should_hide_source_node_by_default=lambda _name: False,
        )

        clone = self.orchestration.OrchestrationMixin._duplicate_object_tree(
            importer,
            source,
            "Data/Objects/template.blend",
            {},
        )

        self.assertEqual(
            [modifier.name for modifier in clone.modifiers],
            ["StarBreaker Weld", "StarBreaker Decal Offset", "Other Displace"],
        )
        self.assertAlmostEqual(clone.modifiers[1].strength, 0.00025)
        self.assertAlmostEqual(clone.modifiers[2].strength, 0.005)
        self.assertEqual(linked, [clone])


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
