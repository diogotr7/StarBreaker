from __future__ import annotations

from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
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


from starbreaker_addon.runtime.constants import PROP_HAS_POM, PROP_TEMPLATE_KEY
from starbreaker_addon.runtime.importer.builders import BuildersMixin


class FakeMaterial(dict):
    def __init__(self, name: str, **props):
        super().__init__(props)
        self.name = name


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


if __name__ == "__main__":
    unittest.main()