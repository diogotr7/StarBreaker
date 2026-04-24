from __future__ import annotations

import json
from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent
VULTURE_ALT_A = REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/DRAK/Vulture/drak_vulture_alt_a_TEX0.materials.json"

sys.path.insert(0, str(ADDON_ROOT))


if "starbreaker_addon" not in sys.modules:
    package = types.ModuleType("starbreaker_addon")
    package.__path__ = [str(ADDON_ROOT / "starbreaker_addon")]
    sys.modules["starbreaker_addon"] = package

if "starbreaker_addon.runtime" not in sys.modules:
    runtime_package = types.ModuleType("starbreaker_addon.runtime")
    runtime_package.__path__ = [str(ADDON_ROOT / "starbreaker_addon" / "runtime")]
    sys.modules["starbreaker_addon.runtime"] = runtime_package

if "starbreaker_addon.runtime.importer" not in sys.modules:
    importer_package = types.ModuleType("starbreaker_addon.runtime.importer")
    importer_package.__path__ = [str(ADDON_ROOT / "starbreaker_addon" / "runtime" / "importer")]
    sys.modules["starbreaker_addon.runtime.importer"] = importer_package


if "mathutils" not in sys.modules:
    mathutils = types.ModuleType("mathutils")

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
    sys.modules["mathutils"] = mathutils


if "bpy" not in sys.modules:
    bpy = types.ModuleType("bpy")
    bpy.types = types.SimpleNamespace(
        Context=object,
        Material=object,
        NodeLinks=object,
        Nodes=object,
        Object=object,
        ShaderNodeTexImage=object,
    )
    bpy.data = types.SimpleNamespace(node_groups=[], images=[])
    sys.modules["bpy"] = bpy


from starbreaker_addon.runtime.constants import (
    PROP_HAS_POM,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_PALETTE_SCOPE,
    PROP_SUBMATERIAL_JSON,
    PROP_TEMPLATE_KEY,
)
from starbreaker_addon.manifest import MaterialSidecar, SubmaterialRecord
from starbreaker_addon.runtime.importer.builders import BuildersMixin
from starbreaker_addon.runtime.importer.decals import DecalsMixin
from starbreaker_addon.runtime.importer.materials import MaterialsMixin
from starbreaker_addon.runtime.importer.utils import _canonical_material_sidecar_path, _material_identity
from starbreaker_addon.templates import template_plan_for_submaterial


class FakeNodeTree:
    def __init__(self):
        self.nodes = []
        self.links = []


class FakeMaterial(dict):
    def __init__(self, name: str, **props):
        super().__init__(props)
        self.name = name
        self.node_tree = FakeNodeTree()
        self.use_nodes = True


class FakeMaterialsCollection(dict):
    def get(self, name: str, default=None):
        return super().get(name, default)

    def new(self, name: str):
        material = FakeMaterial(name)
        self[name] = material
        return material


class FakeSlot:
    def __init__(self, material):
        self.material = material


class FakePolygon:
    def __init__(self, material_index: int, vertices: list[int]):
        self.material_index = material_index
        self.vertices = vertices


class FakeVertex:
    def __init__(self, index: int):
        self.index = index


class FakeMesh:
    def __init__(self, polygons: list[FakePolygon], vertex_count: int):
        self.polygons = polygons
        self.vertices = [FakeVertex(index) for index in range(vertex_count)]


class FakeVertexGroup:
    def __init__(self, name: str):
        self.name = name
        self.members: set[int] = set()

    def add(self, indices: list[int], weight: float, mode: str) -> None:
        self.members.update(int(index) for index in indices)

    def remove(self, indices: list[int]) -> None:
        for index in indices:
            self.members.discard(int(index))


class FakeVertexGroups:
    def __init__(self):
        self._groups: dict[str, FakeVertexGroup] = {}

    def get(self, name: str):
        return self._groups.get(name)

    def new(self, name: str):
        group = FakeVertexGroup(name)
        self._groups[name] = group
        return group


class FakeModifier:
    def __init__(self, name: str, modifier_type: str):
        self.name = name
        self.type = modifier_type
        self.strength = None
        self.mid_level = None
        self.direction = None
        self.space = None
        self.vertex_group = ""


class FakeModifiers:
    def __init__(self):
        self._modifiers: list[FakeModifier] = []

    def get(self, name: str):
        for modifier in self._modifiers:
            if modifier.name == name:
                return modifier
        return None

    def new(self, name: str, type: str):
        modifier = FakeModifier(name, type)
        self._modifiers.append(modifier)
        return modifier

    def remove(self, modifier: FakeModifier) -> None:
        self._modifiers.remove(modifier)


class FakeObject:
    def __init__(self, material_slots: list[FakeSlot], mesh: FakeMesh, **props):
        self.material_slots = material_slots
        self.data = mesh
        self.vertex_groups = FakeVertexGroups()
        self.modifiers = FakeModifiers()
        self._props = dict(props)

    def get(self, name: str, default=None):
        return self._props.get(name, default)


class ImporterUnderTest(BuildersMixin):
    def __init__(self, *, channel: str | None = None, fallback_rgb: tuple[float, float, float] | None = None):
        self.channel = channel
        self.fallback_rgb = fallback_rgb
        self.illum_rgb_calls: list[tuple[float, float, float]] = []

    def _mesh_decal_host_channel_for_object(self, obj):
        return self.channel

    def _mesh_decal_host_rgb_for_object(self, obj):
        return self.fallback_rgb

    def _ensure_illum_pom_host_rgb_variant(self, material, rgb):
        self.illum_rgb_calls.append(rgb)
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))


class FakePackage:
    def __init__(self, has_decal_texture: bool):
        self.has_decal_texture = has_decal_texture

    def resolve_path(self, relative_path):
        if self.has_decal_texture and relative_path:
            return Path("/tmp") / Path(relative_path).name
        return None


class DecalDefaultsImporterUnderTest(DecalsMixin):
    def __init__(self, has_decal_texture: bool):
        self.package = FakePackage(has_decal_texture)


class MaterialReuseImporterUnderTest(MaterialsMixin):
    def __init__(self):
        self.material_cache = {}
        self.material_identity_index = {}
        self.material_identity_index_ready = False
        self.package = None
        self.package_root = None
        self.rebuild_calls: list[str] = []

    def _palette_scope(self, palette=None) -> str:
        return "test-scope"

    def _ensure_material_identity_index(self) -> None:
        self.material_identity_index_ready = True

    def _build_managed_material(
        self,
        material,
        sidecar_path,
        sidecar,
        submaterial,
        palette,
        material_identity,
    ) -> None:
        self.rebuild_calls.append(material.name)
        material[PROP_TEMPLATE_KEY] = template_plan_for_submaterial(submaterial).template_key
        material[PROP_MATERIAL_IDENTITY] = material_identity
        material[PROP_MATERIAL_SIDECAR] = _canonical_material_sidecar_path(sidecar_path, sidecar)
        material[PROP_SUBMATERIAL_JSON] = json.dumps(submaterial.raw, sort_keys=True)
        material[PROP_PALETTE_SCOPE] = self._palette_scope(palette)


class ManagedMaterialBuildImporterUnderTest(BuildersMixin):
    def __init__(self):
        self.build_calls: list[str] = []

    def _build_nodraw_material(self, material) -> None:
        self.build_calls.append("nodraw")

    def _build_illum_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("illum")

    def _build_hard_surface_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("hard_surface")

    def _group_contract_for_submaterial(self, submaterial):
        return None

    def _build_contract_group_material(self, material, submaterial, palette, plan, group_contract) -> bool:
        return False

    def _build_glass_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("glass")

    def _build_screen_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("screen")

    def _build_effect_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("effects")

    def _build_principled_material(self, material, submaterial, palette, plan) -> None:
        self.build_calls.append("principled")

    def _apply_material_node_layout(self, material) -> None:
        return None

    def _sweep_unreachable_nodes(self, material) -> None:
        return None

    def _palette_scope(self, palette=None) -> str:
        return "test-scope"


class DecalOffsetTests(unittest.TestCase):
    def test_illum_pom_loadout_decal_uses_smaller_offset_strength(self) -> None:
        decal = FakeMaterial(
            "KLWE_las_rep_s1-3:pom_decals__host_rgb_070707",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
            },
        )
        host = FakeMaterial("KLWE_las_rep_s1-3:H_painted_metal_dark_gray_01", starbreaker_shader_family="LayerBlend_V2")
        obj = FakeObject(
            material_slots=[FakeSlot(decal), FakeSlot(host)],
            mesh=FakeMesh(
                polygons=[
                    FakePolygon(0, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                ],
                vertex_count=6,
            ),
            starbreaker_material_sidecar="Data/Objects/Spaceships/Weapons/KLWE/KLWE_las_rep_s1-3_TEX0.materials.json",
        )

        importer = ImporterUnderTest()

        self.assertTrue(importer._apply_decal_offset_modifier(obj))
        group = obj.vertex_groups.get(importer._DECAL_OFFSET_GROUP_NAME)
        self.assertIsNotNone(group)
        self.assertEqual(group.members, {0, 1, 2})

        modifier = obj.modifiers.get(importer._DECAL_OFFSET_MODIFIER_NAME)
        self.assertIsNotNone(modifier)
        self.assertEqual(modifier.type, "DISPLACE")
        self.assertEqual(modifier.vertex_group, importer._DECAL_OFFSET_GROUP_NAME)
        self.assertAlmostEqual(modifier.strength, importer._LOADOUT_DECAL_OFFSET_STRENGTH)
        self.assertEqual(modifier.direction, "NORMAL")
        self.assertEqual(modifier.space, "LOCAL")

    def test_ship_decal_keeps_default_offset_strength(self) -> None:
        decal = FakeMaterial(
            "rsi_aurora_mk2:pom_decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
            },
        )
        host = FakeMaterial("rsi_aurora_mk2:hull", starbreaker_shader_family="LayerBlend_V2")
        obj = FakeObject(
            material_slots=[FakeSlot(decal), FakeSlot(host)],
            mesh=FakeMesh(
                polygons=[
                    FakePolygon(0, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                ],
                vertex_count=6,
            ),
            starbreaker_material_sidecar="Data/Objects/Spaceships/Ships/RSI/aurora_mk2/rsi_aurora_mk2_TEX0.materials.json",
        )

        importer = ImporterUnderTest()

        self.assertTrue(importer._apply_decal_offset_modifier(obj))
        modifier = obj.modifiers.get(importer._DECAL_OFFSET_MODIFIER_NAME)
        self.assertIsNotNone(modifier)
        self.assertAlmostEqual(modifier.strength, importer._DECAL_OFFSET_STRENGTH)

    def test_illum_pom_rebind_uses_palette_channel_rgb_when_no_authored_fallback_exists(self) -> None:
        decal = FakeMaterial(
            "drak_vulture:pom_decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = ImporterUnderTest(channel="primary", fallback_rgb=None)

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.illum_rgb_calls, [palette.primary])
        self.assertEqual(obj.material_slots[0].material.name, "drak_vulture:pom_decals__host_rgb")

    def test_missing_mesh_decal_texture_defaults_alpha_to_zero(self) -> None:
        submaterial = SubmaterialRecord.from_value({"shader_family": "MeshDecal"})
        palette = types.SimpleNamespace(decal_texture=None)
        importer = DecalDefaultsImporterUnderTest(has_decal_texture=False)

        _, alpha = importer._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=importer._has_palette_decal_texture(palette),
        )

        self.assertEqual(alpha, 0.0)

    def test_missing_stencil_map_texture_defaults_alpha_to_zero(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "LayerBlend_V2",
                "decoded_feature_flags": {"has_stencil_map": True},
            }
        )
        palette = types.SimpleNamespace(decal_texture=None)
        importer = DecalDefaultsImporterUnderTest(has_decal_texture=False)

        _, alpha = importer._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=importer._has_palette_decal_texture(palette),
        )

        self.assertEqual(alpha, 0.0)

    def test_mesh_decal_with_texture_keeps_existing_default_alpha(self) -> None:
        submaterial = SubmaterialRecord.from_value({"shader_family": "MeshDecal"})
        palette = types.SimpleNamespace(decal_texture="Data/Textures/paint/decal.png")
        importer = DecalDefaultsImporterUnderTest(has_decal_texture=True)

        _, alpha = importer._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=importer._has_palette_decal_texture(palette),
        )

        self.assertEqual(alpha, 0.85)


class MaterialReuseTests(unittest.TestCase):
    @unittest.skipUnless(
        VULTURE_ALT_A.is_file(),
        "Vulture fixtures not present; skipping material reuse regression test",
    )
    def test_stale_template_key_forces_managed_material_rebuild(self) -> None:
        sidecar = MaterialSidecar.from_file(VULTURE_ALT_A)
        submaterial = next(
            candidate
            for candidate in sidecar.submaterials
            if candidate.submaterial_name == "livery_decal"
        )
        self.assertEqual(template_plan_for_submaterial(submaterial).template_key, "nodraw")

        bpy = sys.modules["bpy"]
        original_materials = getattr(bpy.data, "materials", None)
        materials = FakeMaterialsCollection()
        bpy.data.materials = materials
        try:
            sidecar_path = _canonical_material_sidecar_path("", sidecar)
            palette_scope = "test-scope"
            material_identity = _material_identity(sidecar_path, sidecar, submaterial, None, palette_scope)
            stale = FakeMaterial(
                submaterial.blender_material_name or "DRAK_Vulture:livery_decal",
                **{
                    PROP_TEMPLATE_KEY: "physical_surface",
                    PROP_MATERIAL_IDENTITY: material_identity,
                    PROP_MATERIAL_SIDECAR: sidecar_path,
                    PROP_SUBMATERIAL_JSON: json.dumps(submaterial.raw, sort_keys=True),
                    PROP_PALETTE_SCOPE: palette_scope,
                },
            )
            materials[stale.name] = stale

            importer = MaterialReuseImporterUnderTest()
            material = importer.material_for_submaterial(sidecar_path, sidecar, submaterial, None)

            self.assertIs(material, stale)
            self.assertEqual(importer.rebuild_calls, [stale.name])
            self.assertEqual(material[PROP_TEMPLATE_KEY], "nodraw")
        finally:
            bpy.data.materials = original_materials

    @unittest.skipUnless(
        VULTURE_ALT_A.is_file(),
        "Vulture fixtures not present; skipping managed material dispatch regression test",
    )
    def test_illum_nodraw_submaterial_uses_nodraw_builder(self) -> None:
        sidecar = MaterialSidecar.from_file(VULTURE_ALT_A)
        submaterial = next(
            candidate
            for candidate in sidecar.submaterials
            if candidate.submaterial_name == "livery_decal"
        )
        importer = ManagedMaterialBuildImporterUnderTest()
        material = FakeMaterial(submaterial.blender_material_name or "DRAK_Vulture:livery_decal")

        importer._build_managed_material(
            material,
            _canonical_material_sidecar_path("", sidecar),
            sidecar,
            submaterial,
            None,
            "identity",
        )

        self.assertEqual(importer.build_calls, ["nodraw"])
        self.assertEqual(material[PROP_TEMPLATE_KEY], "nodraw")


if __name__ == "__main__":
    unittest.main()