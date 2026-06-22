from __future__ import annotations

import contextlib
import io
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

if "numpy" not in sys.modules:
    numpy = types.ModuleType("numpy")
    sys.modules["numpy"] = numpy


from starbreaker_addon.runtime.constants import (
    PROP_ASSEMBLY_KIND,
    PROP_DECAL_HOST_CHANNEL,
    PROP_DECAL_HOST_RGB,
    PROP_HAS_POM,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_PALETTE_SCOPE,
    PROP_SUBMATERIAL_JSON,
    PROP_TEMPLATE_KEY,
)
from starbreaker_addon.manifest import MaterialSidecar, SceneInstanceRecord, SubmaterialRecord
from starbreaker_addon.runtime.importer.builders import (
    BuildersMixin,
    _illum_payload_is_local_opacity_decal,
    _mesh_decal_pom_payload_is_control_only as _builder_mesh_decal_pom_payload_is_control_only,
    _mesh_decal_neutral_breakup_default,
    _parallax_height_sampler_extension,
)
from starbreaker_addon.runtime.importer.decals import DecalsMixin
from starbreaker_addon.runtime.importer.materials import MaterialsMixin
from starbreaker_addon.runtime.importer.materials import (
    _material_datablock_is_valid,
    _mesh_decal_pom_payload_is_control_only as _materials_mesh_decal_pom_payload_is_control_only,
)
from starbreaker_addon.runtime.importer.orchestration import OrchestrationMixin
from starbreaker_addon.material_contract import ContractInput, ShaderGroupContract
from starbreaker_addon.runtime.importer.utils import (
    _canonical_material_sidecar_path,
    _material_identity,
    _scene_attachment_offset_to_blender,
)
from starbreaker_addon.templates import template_plan_for_submaterial


class FakeNodeTree:
    def __init__(self):
        self.nodes = []
        self.links = []

    def get(self, key, default=None):
        return getattr(self, key, default)

    def __setitem__(self, key, value):
        setattr(self, key, value)


class FakeMaterial(dict):
    def __init__(self, name: str, **props):
        library = props.pop("library", None)
        super().__init__(props)
        self.name = name
        self.node_tree = FakeNodeTree()
        self.use_nodes = True
        self.library = library

    def copy(self):
        return FakeMaterial(self.name, library=None, **dict(self))


class FakeLink:
    def __init__(self, from_socket, to_socket):
        self.from_socket = from_socket
        self.to_socket = to_socket

    @property
    def from_node(self):
        return self.from_socket.node

    @property
    def to_node(self):
        return self.to_socket.node


class FakeSocket:
    def __init__(self, name: str, *, is_output: bool):
        self.name = name
        self.is_output = is_output
        self.links: list[FakeLink] = []
        self.default_value = None
        self.node = None


class FakeSocketCollection:
    def __init__(self, sockets: list[FakeSocket]):
        self._sockets = sockets
        self._by_name = {socket.name: socket for socket in sockets}

    def __getitem__(self, index: int):
        return self._sockets[index]

    def __iter__(self):
        return iter(self._sockets)

    def get(self, name: str, default=None):
        return self._by_name.get(name, default)


class FakeLinks(list):
    def new(self, from_socket, to_socket):
        link = FakeLink(from_socket, to_socket)
        self.append(link)
        from_socket.links.append(link)
        to_socket.links.append(link)
        return link

    def remove(self, link):
        if link in self:
            super().remove(link)
        if link in link.from_socket.links:
            link.from_socket.links.remove(link)
        if link in link.to_socket.links:
            link.to_socket.links.remove(link)


class FakeNodeGroupTree:
    def __init__(self, name: str, **props):
        self.name = name
        for key, value in props.items():
            setattr(self, key, value)

    def get(self, key, default=None):
        return getattr(self, key, default)


class FakeNode:
    def __init__(
        self,
        bl_idname: str,
        *,
        name: str = "",
        node_tree: FakeNodeGroupTree | None = None,
        inputs: list[str] | None = None,
        outputs: list[str] | None = None,
    ):
        self.bl_idname = bl_idname
        self.name = name
        self.label = ""
        self.node_tree = node_tree
        self.location = types.SimpleNamespace(x=0.0, y=0.0)
        self.blend_type = ""
        self.inputs = FakeSocketCollection([FakeSocket(socket, is_output=False) for socket in (inputs or [])])
        self.outputs = FakeSocketCollection([FakeSocket(socket, is_output=True) for socket in (outputs or [])])
        self._props = {}
        for socket in [*self.inputs._sockets, *self.outputs._sockets]:
            socket.node = self

    def __setitem__(self, key, value):
        self._props[key] = value

    def get(self, key, default=None):
        return self._props.get(key, default)


class FakeNodes(list):
    def new(self, bl_idname: str):
        if bl_idname == "ShaderNodeRGB":
            node = FakeNode(bl_idname, outputs=["Color"])
        elif bl_idname == "ShaderNodeMixRGB":
            node = FakeNode(bl_idname, inputs=["Fac", "Color1", "Color2"], outputs=["Color"])
        elif bl_idname == "ShaderNodeMath":
            node = FakeNode(bl_idname, inputs=["Value", "Value"], outputs=["Value"])
        elif bl_idname == "ShaderNodeSeparateColor":
            node = FakeNode(bl_idname, inputs=["Color"], outputs=["Red", "Green", "Blue", "Alpha"])
        elif bl_idname == "ShaderNodeMix":
            node = FakeNode(bl_idname, inputs=["Factor", "A", "B", "C"], outputs=["Result"])
        elif bl_idname == "ShaderNodeGroup":
            node = FakeNode(
                bl_idname,
                inputs=[
                    "Base Color",
                    "Base Alpha",
                    "Primary Color",
                    "Primary Normal",
                    "Normal Color",
                    "Normal Strength",
                    "Use Normal",
                    "Roughness",
                    "Metallic",
                    "Height",
                    "Bump Strength",
                    "Use Bump",
                    "Alpha",
                    "Emission Strength",
                    "Host Tint",
                    "TexSlot1_DecalSource",
                    "TexSlot3_NormalGloss",
                    "TexSlot4_Height",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                ],
                outputs=["Color", "Alpha", "Shader"],
            )
        elif bl_idname == "ShaderNodeOutputMaterial":
            node = FakeNode(bl_idname, inputs=["Surface"], outputs=[])
        elif bl_idname == "ShaderNodeTexImage":
            node = FakeNode(bl_idname, inputs=["Vector"], outputs=["Color", "Alpha"])
        elif bl_idname == "ShaderNodeNormalMap":
            node = FakeNode(bl_idname, inputs=["Strength", "Color"], outputs=["Normal"])
        else:
            node = FakeNode(bl_idname)
        self.append(node)
        return node

    def get(self, name: str, default=None):
        for node in self:
            if node.name == name:
                return node
        return default


class FakeMaterialNodeTree:
    def __init__(self):
        self.nodes = FakeNodes()
        self.links = FakeLinks()

    def get(self, key, default=None):
        return getattr(self, key, default)

    def __setitem__(self, key, value):
        setattr(self, key, value)

    def clone(self):
        clone = FakeMaterialNodeTree()
        mapping: dict[int, FakeNode] = {}
        socket_mapping: dict[int, FakeSocket] = {}
        for node in self.nodes:
            copied = FakeNode(
                node.bl_idname,
                name=node.name,
                node_tree=node.node_tree,
                inputs=[socket.name for socket in node.inputs._sockets],
                outputs=[socket.name for socket in node.outputs._sockets],
            )
            copied.label = node.label
            copied.blend_type = node.blend_type
            copied.location = types.SimpleNamespace(x=node.location.x, y=node.location.y)
            clone.nodes.append(copied)
            mapping[id(node)] = copied
            for source, target in zip(node.inputs._sockets, copied.inputs._sockets):
                target.default_value = source.default_value
                socket_mapping[id(source)] = target
            for source, target in zip(node.outputs._sockets, copied.outputs._sockets):
                target.default_value = source.default_value
                socket_mapping[id(source)] = target
        for link in self.links:
            clone.links.new(socket_mapping[id(link.from_socket)], socket_mapping[id(link.to_socket)])
        return clone


class FakeNodeMaterial(FakeMaterial):
    def __init__(self, name: str, **props):
        super().__init__(name, **props)
        self.node_tree = FakeMaterialNodeTree()
        self.blend_method = "BLEND"
        self.use_screen_refraction = True
        self.users = 0

    def copy(self):
        material = FakeNodeMaterial(self.name, **dict(self))
        material.node_tree = self.node_tree.clone()
        return material


class FakeMaterialsCollection(dict):
    def __iter__(self):
        return iter(self.values())

    def get(self, name: str, default=None):
        return super().get(name, default)

    def new(self, name: str):
        material = FakeMaterial(name)
        self[name] = material
        return material

    def remove(self, material, do_unlink: bool = False):
        for key, value in list(self.items()):
            if value is material:
                del self[key]
                return


class FakeMatrixWorld:
    def __init__(self, translation: tuple[float, float, float]):
        self._rows = (
            (1.0, 0.0, 0.0, translation[0]),
            (0.0, 1.0, 0.0, translation[1]),
            (0.0, 0.0, 1.0, translation[2]),
            (0.0, 0.0, 0.0, 1.0),
        )

    def __getitem__(self, index: int):
        return self._rows[index]


class FakeSlot:
    def __init__(self, material):
        self.material = material


class FakePolygon:
    def __init__(self, material_index: int, vertices: list[int]):
        self.material_index = material_index
        self.vertices = vertices


class FakeVertex:
    def __init__(self, co: tuple[float, float, float]):
        self.co = types.SimpleNamespace(x=co[0], y=co[1], z=co[2])


class FakeMesh:
    def __init__(
        self,
        polygons: list[FakePolygon],
        vertex_count: int,
        vertices: list[tuple[float, float, float]] | None = None,
    ):
        self.polygons = polygons
        self.vertices = [
            FakeVertex(co)
            for co in (
                vertices
                if vertices is not None
                else [(0.0, 0.0, 0.0) for _index in range(vertex_count)]
            )
        ]
        self.materials = []

    def as_pointer(self) -> int:
        return id(self)


class FakeObject:
    def __init__(self, material_slots: list[FakeSlot], mesh: FakeMesh, **props):
        self.name = props.pop("name", "FakeObject")
        self.type = "MESH"
        self.material_slots = material_slots
        self.data = mesh
        if hasattr(self.data, "materials") and not self.data.materials:
            self.data.materials.extend(slot.material for slot in material_slots)
        self._props = dict(props)

    def get(self, name: str, default=None):
        return self._props.get(name, default)


class ImporterUnderTest(BuildersMixin):
    def __init__(self, *, channel: str | None = None, fallback_rgb: tuple[float, float, float] | None = None):
        self.channel = channel
        self.fallback_rgb = fallback_rgb
        self.illum_rgb_calls: list[tuple[float, float, float]] = []
        self.illum_decal_rgb_calls: list[tuple[float, float, float]] = []
        self.illum_decal_material_calls: list[tuple[str, str]] = []
        self.illum_host_decal_calls: list[tuple[str, str]] = []

    def _mesh_decal_host_channel_for_object(self, obj):
        return self.channel

    def _mesh_decal_host_rgb_for_object(self, obj):
        return self.fallback_rgb

    def _ensure_illum_pom_host_rgb_variant(self, material, rgb):
        self.illum_rgb_calls.append(rgb)
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))

    def _ensure_illum_decal_host_rgb_variant(self, material, rgb):
        self.illum_decal_rgb_calls.append(rgb)
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))

    def _ensure_illum_decal_host_material_variant(self, material, host_material):
        self.illum_decal_material_calls.append((material.name, host_material.name))
        return FakeMaterial(f"{material.name}__host_mat", **dict(material))

    def _ensure_host_with_illum_decal_opacity_variant(self, decal_material, host_material):
        self.illum_host_decal_calls.append((host_material.name, decal_material.name))
        return None


class MeshDecalRebindImporterUnderTest(BuildersMixin):
    def __init__(
        self,
        channel: str | None = None,
        authored_rgb: tuple[float, float, float] | None = None,
        assembly_kind: str | None = "fps_weapon",
    ):
        self.channel = channel
        self.authored_rgb = authored_rgb
        self.package = (
            types.SimpleNamespace(scene=types.SimpleNamespace(raw={"assembly_kind": assembly_kind}))
            if assembly_kind is not None
            else None
        )
        self.package_root = None
        self.mesh_variant_calls: list[tuple[str, str]] = []
        self.mesh_host_material_variant_calls: list[tuple[str, str]] = []
        self.mesh_rgb_variant_calls: list[tuple[str, tuple[float, float, float]]] = []

    def _mesh_decal_host_channel_for_object(self, obj):
        return self.channel

    def _mesh_decal_host_rgb_for_object(self, obj):
        return None

    def _ensure_mesh_decal_host_variant(self, material, channel, palette):
        self.mesh_variant_calls.append((material.name, channel))
        return FakeMaterial(f"{material.name}__host_{channel}", **dict(material))

    def _ensure_mesh_decal_host_material_variant(self, material, host_material):
        self.mesh_host_material_variant_calls.append((material.name, host_material.name))
        variant = FakeMaterial(f"{material.name}__host_material_{host_material.name}", **dict(material))
        variant["starbreaker_decal_host_material_key"] = self._decal_host_material_key(host_material)
        return variant

    def _ensure_mesh_decal_host_rgb_variant(self, material, rgb):
        self.mesh_rgb_variant_calls.append((material.name, rgb))
        return FakeMaterial(f"{material.name}__host_rgb", **dict(material))

    def _read_paint_tint_rgb(self, _material):
        return self.authored_rgb


class MeshDecalVariantImporterUnderTest(BuildersMixin):
    def _palette_group_node(self, nodes, links, palette, *, x: int, y: int):
        node = FakeNode(
            "ShaderNodeGroup",
            name="Palette",
            node_tree=FakeNodeGroupTree("StarBreaker Palette test"),
            outputs=["Secondary"],
        )
        node.location = types.SimpleNamespace(x=float(x), y=float(y))
        nodes.append(node)
        return node


class HostRoutingImporterUnderTest(BuildersMixin):
    def __init__(self):
        self.host_channel_cache = {}
        self.host_rgb_cache = {}


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
    def __init__(self, *, assembly_kind: str | None = "fps_weapon"):
        self.material_cache = {}
        self.material_identity_index = {}
        self.material_identity_index_ready = False
        scene_raw = {}
        if assembly_kind is not None:
            scene_raw["assembly_kind"] = assembly_kind
        self.package = types.SimpleNamespace(scene=types.SimpleNamespace(raw=scene_raw))
        self.package_root = None
        self.rebuild_calls: list[str] = []

    def _palette_scope(self, palette=None) -> str:
        return "test-scope"

    def _ensure_material_identity_index(self) -> None:
        self.material_identity_index_ready = True

    def _group_contract_for_submaterial(self, submaterial):
        if getattr(submaterial, "shader_family", None) != "MeshDecal":
            return None
        return ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )

    def _build_managed_material(
        self,
        material,
        sidecar_path,
        sidecar,
        submaterial,
        palette,
        material_identity,
        ui_image_path=None,
    ) -> None:
        self.rebuild_calls.append(material.name)
        material[PROP_TEMPLATE_KEY] = template_plan_for_submaterial(submaterial).template_key
        material[PROP_MATERIAL_IDENTITY] = material_identity
        material[PROP_MATERIAL_SIDECAR] = _canonical_material_sidecar_path(sidecar_path, sidecar)
        material[PROP_SUBMATERIAL_JSON] = json.dumps(submaterial.raw, sort_keys=True)
        material[PROP_PALETTE_SCOPE] = self._palette_scope(palette)


class PackageImporterMroUnderTest(MaterialsMixin, BuildersMixin):
    pass


class ControlOnlyPomReliefBuilderUnderTest(MaterialsMixin, BuildersMixin):
    def __init__(self, *, assembly_kind: str | None = "fps_weapon"):
        scene_raw = {}
        if assembly_kind is not None:
            scene_raw["assembly_kind"] = assembly_kind
        self.package = types.SimpleNamespace(scene=types.SimpleNamespace(raw=scene_raw))

    def _ensure_contract_group(self, group_contract):
        return FakeNodeGroupTree(group_contract.name)

    def _ensure_runtime_principled_group(self):
        return FakeNodeGroupTree("StarBreaker Runtime Principled")

    def _set_socket_default(self, socket, value) -> None:
        if socket is not None:
            socket.default_value = value

    def _image_node(self, nodes, image_path, *, x: int, y: int, is_color: bool, **_kwargs):
        if not image_path:
            return None
        node = FakeNode("ShaderNodeTexImage", name=Path(image_path).name, outputs=["Color", "Alpha"])
        node.location = types.SimpleNamespace(x=float(x), y=float(y))
        node.is_color = is_color
        nodes.append(node)
        return node

    def _link_group_input(self, links, source_socket, group_node, input_name: str) -> None:
        target = group_node.inputs.get(input_name)
        if source_socket is not None and target is not None:
            links.new(source_socket, target)

    def _wire_surface_shader_to_output(self, nodes, links, surface_shader, output, plan, submaterial) -> None:
        if surface_shader is not None:
            links.new(surface_shader, output.inputs[0])

    def _configure_material(self, material, *, blend_method: str, shadow_method: str) -> None:
        material.blend_method = blend_method
        material.shadow_method = shadow_method


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


class FakeSidecar:
    def __init__(self, submaterials):
        self.submaterials = submaterials


class FakeSubmaterial:
    def __init__(self, index: int, name: str):
        self.index = index
        self.submaterial_name = name


class TestMeshDecalNeutralBreakupDefault(unittest.TestCase):
    def test_mesh_decal_template_normal_strength_does_not_use_decal_source_alpha(self) -> None:
        group = FakeMaterialNodeTree()
        group_input = FakeNode(
            "NodeGroupInput",
            outputs=[
                "Emission Strength",
                "Use Vert Col",
                "TexSlot1_DecalSource",
                "TexSlot1_DecalSource_alpha",
            ],
        )
        principled = FakeNode(
            "ShaderNodeBsdfPrincipled",
            name="Principled BSDF",
            inputs=["Emission Color", "Emission Strength", "Alpha"],
            outputs=["BSDF"],
        )
        base_alpha = FakeNode("ShaderNodeMath", name="Maths.008", outputs=["Value"])
        normal_map = FakeNode(
            "ShaderNodeNormalMap",
            name="Normal Map",
            inputs=["Strength", "Color"],
            outputs=["Normal"],
        )
        normal_map.inputs.get("Strength").default_value = 1.0
        group.nodes.extend([group_input, principled, base_alpha, normal_map])

        BuildersMixin._patch_mesh_decal_template_emission(group)

        alpha_mask = group.nodes.get("SB MeshDecal Normal Alpha Mask")
        self.assertIsNotNone(alpha_mask)
        self.assertEqual(len(alpha_mask.inputs[1].links), 1)
        self.assertIs(alpha_mask.inputs[1].links[0].from_socket, base_alpha.outputs.get("Value"))
        self.assertIsNot(
            alpha_mask.inputs[1].links[0].from_socket,
            group_input.outputs.get("TexSlot1_DecalSource_alpha"),
        )

    def test_returns_white_for_stencil_mesh_decal_without_breakup_texture(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_stencil_map": True, "tokens": ["STENCIL_MAP"]},
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )
        contract_input = ContractInput(
            name="TexSlot8_GrimeBreakup",
            socket_type="NodeSocketColor",
            semantic="grime_breakup",
            source_slot="TexSlot8",
            required=False,
            default_value=None,
            raw={},
        )

        self.assertEqual(
            _mesh_decal_neutral_breakup_default(
                group_contract,
                submaterial,
                contract_input,
                source_socket=None,
            ),
            (1.0, 1.0, 1.0, 1.0),
        )

    def test_returns_white_for_pom_mesh_decal_without_breakup_texture(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "has_parallax_occlusion_mapping": True,
                    "tokens": ["PARALLAX_OCCLUSION_MAPPING"],
                },
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )
        contract_input = ContractInput(
            name="TexSlot8_GrimeBreakup",
            socket_type="NodeSocketColor",
            semantic="grime_breakup",
            source_slot="TexSlot8",
            required=False,
            default_value=None,
            raw={},
        )

        self.assertEqual(
            _mesh_decal_neutral_breakup_default(
                group_contract,
                submaterial,
                contract_input,
                source_socket=None,
            ),
            (1.0, 1.0, 1.0, 1.0),
        )

    def test_returns_none_when_breakup_texture_is_present(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_stencil_map": True, "tokens": ["STENCIL_MAP"]},
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )
        contract_input = ContractInput(
            name="TexSlot8_GrimeBreakup",
            socket_type="NodeSocketColor",
            semantic="grime_breakup",
            source_slot="TexSlot8",
            required=False,
            default_value=None,
            raw={},
        )

        self.assertIsNone(
            _mesh_decal_neutral_breakup_default(
                group_contract,
                submaterial,
                contract_input,
                source_socket=object(),
            )
        )


class FakePackageWithSidecars:
    def __init__(self, sidecar):
        self.sidecar = sidecar
        self.palettes = {}
        self.scene = types.SimpleNamespace(root_entity=types.SimpleNamespace(palette_id=None), children=[])

    def load_material_sidecar(self, sidecar_path):
        return self.sidecar


class OrchestrationImporterUnderTest(OrchestrationMixin, BuildersMixin):
    def __init__(self, sidecar):
        self.package = FakePackageWithSidecars(sidecar)
        self.package_root = None
        self.import_palette_override = None
        self.import_paint_variant_sidecar = None
        self.exterior_material_sidecars = None
        self.mesh_polygon_counts_cache = {}
        self.slot_mapping_cache = {}
        self.sidecar_submaterials_by_index = {}
        self.sidecar_submaterials_by_name = {}
        self.sidecar_submaterials_by_name_all = {}

    def _ensure_runtime_shared_groups(self) -> None:
        return None

    def _effective_palette_id(self, palette_id: str | None) -> str | None:
        return palette_id

    def material_for_submaterial(self, sidecar_path, sidecar, submaterial, palette, ui_image_path=None):
        return FakeMaterial(f"material_{submaterial.index}")

    def _remove_replaced_slot_material(self, material) -> None:
        return None

    def _rebind_mesh_decal_for_host(self, obj, palette, **_kwargs) -> int:
        return 0


class OrphanRemovalImporterUnderTest(OrchestrationMixin):
    def __init__(self):
        self._pending_orphan_materials = set()


class FakeHashableMaterial:
    library = None

    def __init__(self):
        self.users = 0

    def get(self, _key, default=None):
        return default


class FakeInvalidMaterial:
    @property
    def name(self):
        raise ReferenceError("StructRNA of type Material has been removed")


class DecalOffsetTests(unittest.TestCase):
    def test_mesh_decal_pom_with_authored_vertex_diffuse_remains_control_only(self) -> None:
        payload = {
            "shader_family": "MeshDecal",
            "decoded_feature_flags": {
                "tokens": ["DIFFUSE_MAP", "VERTDATA", "PARALLAX_OCCLUSION_MAPPING", "USE_DAMAGE_MAP"],
                "has_decal": False,
                "has_stencil_map": False,
                "has_parallax_occlusion_mapping": True,
            },
            "texture_slots": [
                {
                    "slot": "TexSlot1",
                    "role": "base_color",
                    "export_path": "Data/textures/vehicles/manufacturer/orig/decals/orig_pom_decals_diff_TEX0.png",
                    "is_virtual": False,
                },
                {
                    "slot": "TexSlot3",
                    "role": "normal_gloss",
                    "export_path": "Data/textures/vehicles/manufacturer/orig/decals/ORIG_Pom_Decals_ddna_TEX0.png",
                },
                {
                    "slot": "TexSlot4",
                    "role": "height",
                    "export_path": "Data/textures/vehicles/manufacturer/orig/decals/orig_pom_decals_displ_TEX0.png",
                },
            ],
        }

        self.assertTrue(_builder_mesh_decal_pom_payload_is_control_only(payload))
        self.assertTrue(_materials_mesh_decal_pom_payload_is_control_only(payload))

    def test_mesh_decal_pom_without_visible_authoring_signals_remains_control_only(self) -> None:
        payload = {
            "shader_family": "MeshDecal",
            "decoded_feature_flags": {
                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                "has_decal": False,
                "has_stencil_map": False,
                "has_parallax_occlusion_mapping": True,
            },
            "texture_slots": [
                {
                    "slot": "TexSlot1",
                    "role": "base_color",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    "is_virtual": False,
                },
                {
                    "slot": "TexSlot3",
                    "role": "normal_gloss",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                },
                {
                    "slot": "TexSlot4",
                    "role": "height",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_height_TEX0.png",
                },
            ],
        }

        self.assertTrue(_builder_mesh_decal_pom_payload_is_control_only(payload))
        self.assertTrue(_materials_mesh_decal_pom_payload_is_control_only(payload))

    def test_mesh_decal_pom_payload_handles_null_tokens(self) -> None:
        # decoded_feature_flags.tokens may be explicitly null; both copies must
        # treat it as empty rather than raising TypeError while iterating.
        payload = {
            "shader_family": "MeshDecal",
            "decoded_feature_flags": {
                "tokens": None,
                "has_decal": False,
                "has_stencil_map": False,
                "has_parallax_occlusion_mapping": True,
            },
            "texture_slots": [
                {
                    "slot": "TexSlot1",
                    "role": "base_color",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    "is_virtual": False,
                },
                {
                    "slot": "TexSlot3",
                    "role": "normal_gloss",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                },
                {
                    "slot": "TexSlot4",
                    "role": "height",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_height_TEX0.png",
                },
            ],
        }

        self.assertTrue(_builder_mesh_decal_pom_payload_is_control_only(payload))
        self.assertTrue(_materials_mesh_decal_pom_payload_is_control_only(payload))

    def test_invalid_material_datablock_is_detected(self) -> None:
        self.assertFalse(_material_datablock_is_valid(FakeInvalidMaterial()))

    def test_replaced_slot_material_cleanup_is_deferred(self) -> None:
        importer = OrphanRemovalImporterUnderTest()
        material = FakeHashableMaterial()

        importer._remove_replaced_slot_material(material)

        self.assertIn(material, importer._pending_orphan_materials)

    def test_rebuild_object_materials_skips_empty_unmapped_slots_without_warning(self) -> None:
        sidecar = FakeSidecar([
            FakeSubmaterial(0, "decal pom"),
            FakeSubmaterial(1, "tint_secondary"),
        ])
        importer = OrchestrationImporterUnderTest(sidecar)
        mesh = FakeMesh(polygons=[], vertex_count=0)
        importer.slot_mapping_cache[mesh.as_pointer()] = [0, 1, None, None]
        obj = FakeObject(
            material_slots=[FakeSlot(None), FakeSlot(None), FakeSlot(None), FakeSlot(None)],
            mesh=mesh,
            name="flair_poster_hook_mesh_005",
            starbreaker_material_sidecar="Data/Objects/props/flair/poster/flair_poster_1_a_TEX0.materials.json",
        )

        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            applied = importer.rebuild_object_materials(obj, None)

        self.assertEqual(applied, 2)
        self.assertEqual(output.getvalue(), "")
        self.assertIsNotNone(obj.material_slots[0].material)
        self.assertIsNotNone(obj.material_slots[1].material)
        self.assertIsNone(obj.material_slots[2].material)
        self.assertIsNone(obj.material_slots[3].material)

    def test_ui_binding_for_object_falls_back_to_manifest_bindings_for_native_blend_meshes(self) -> None:
        sidecar_path = "Data/Materials/vehicles/manufacturer/DRAK/drak_int_master_01_TEX0.materials.json"
        mesh_asset = "Data/Objects/Spaceships/Seats/DRAK/clipper/drak_clipper_dashboard_pilot_LOD0.blend"
        importer = OrchestrationImporterUnderTest(FakeSidecar([FakeSubmaterial(0, "RTT_Screen")]))
        importer.package.scene.children = [
            SceneInstanceRecord.from_value(
                {
                    "entity_name": "DRAK_Clipper_Dashboard",
                    "mesh_asset": mesh_asset,
                    "material_sidecar": sidecar_path,
                    "parent_node_name": "hardpoint_seat_pilot_dashboard",
                    "ui_bindings": [
                        {
                            "helper_name": "Screen_Left_Upper_RTT",
                            "generated_image_path": "Data/UI/Generated/1e7b5c1786ffb083_TEX0.png",
                        }
                    ],
                }
            )
        ]
        parent = FakeObject(material_slots=[], mesh=FakeMesh(polygons=[], vertex_count=0), name="body_50_hardpoint_seat_pilot_dashboard")
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            name="Screen_Left_Upper_RTT",
            starbreaker_mesh_asset=mesh_asset,
            starbreaker_material_sidecar=sidecar_path,
            starbreaker_instance_json=json.dumps(
                {
                    "source_object_name": "Screen_Left_Upper_RTT",
                    "source_parent_name": "screen_top_left_geo",
                    "source_ancestors": ["Frame", "Main_Body"],
                    "ui_bindings": [],
                }
            ),
        )
        obj.parent = parent

        binding = importer._ui_binding_for_object(obj)

        self.assertEqual(
            binding,
            {
                "helper_name": "Screen_Left_Upper_RTT",
                "generated_image_path": "Data/UI/Generated/1e7b5c1786ffb083_TEX0.png",
            },
        )

    def test_ui_binding_for_object_uses_helper_binding_as_fallback_when_unmatched(self) -> None:
        importer = OrchestrationImporterUnderTest(FakeSidecar([FakeSubmaterial(0, "screen")]))
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            name="mesh_end_screen_plane",
            starbreaker_instance_json=json.dumps(
                {
                    "source_object_name": "mesh_end_screen_plane",
                    "source_parent_name": None,
                    "source_ancestors": [],
                    "ui_bindings": [
                        {
                            "helper_name": "$slot_standing_screen",
                            "generated_image_path": "Data/UI/Generated/ship/drak/Clipper/buildingblocks_canvas_i_med_medicalendofbed_a.png",
                        }
                    ],
                }
            ),
        )

        binding = importer._ui_binding_for_object(obj)

        self.assertEqual(
            binding,
            {
                "helper_name": "$slot_standing_screen",
                "generated_image_path": "Data/UI/Generated/ship/drak/Clipper/buildingblocks_canvas_i_med_medicalendofbed_a.png",
            },
        )

    def test_illum_pom_rebind_keeps_original_material_with_palette_channel(self) -> None:
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

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_rgb_calls, [])
        self.assertIs(obj.material_slots[0].material, decal)

    def test_illum_pom_rebind_keeps_original_material_with_fallback_rgb(self) -> None:
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
            primary=(0.85, 0.72, 0.12),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = ImporterUnderTest(
            channel="primary",
            fallback_rgb=(0.0627, 0.0627, 0.0627),
        )

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_rgb_calls, [])
        self.assertIs(obj.material_slots[0].material, decal)

    def test_illum_decal_opacity_without_spatial_host_does_not_use_rgb_fallback(self) -> None:
        decal = FakeMaterial(
            "optc_s03_behr_02_mat:decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: False,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "Illum",
                        "decoded_feature_flags": {
                            "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                            "has_decal": True,
                        }
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = ImporterUnderTest(channel=None, fallback_rgb=(0.02, 0.03, 0.04))

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_rgb_calls, [])
        self.assertEqual(importer.illum_decal_rgb_calls, [])
        self.assertIs(obj.material_slots[0].material, decal)

    def test_illum_decal_opacity_payload_uses_local_texture_semantics(self) -> None:
        payload = {
            "shader_family": "Illum",
            "decoded_feature_flags": {
                "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                "has_decal": True,
                "has_parallax_occlusion_mapping": False,
            },
            "texture_slots": [
                {"slot": "TexSlot1", "role": "base_color", "is_virtual": False},
                {"slot": "TexSlot2", "role": "normal_gloss", "is_virtual": False},
            ],
            "virtual_inputs": [],
        }

        self.assertTrue(_illum_payload_is_local_opacity_decal(payload))

    def test_illum_decal_opacity_payload_rejects_virtual_tint_palette_source(self) -> None:
        payload = {
            "shader_family": "Illum",
            "decoded_feature_flags": {
                "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                "has_decal": True,
                "has_parallax_occlusion_mapping": False,
            },
            "texture_slots": [
                {"slot": "TexSlot7", "role": "tint_palette_decal", "is_virtual": True},
            ],
            "virtual_inputs": ["$TintPaletteDecal"],
        }

        self.assertFalse(_illum_payload_is_local_opacity_decal(payload))

    def test_illum_decal_host_rgb_variant_keeps_decal_texture_at_illum_input(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs[0], layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs[1], layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs[0], illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs[1], illum.inputs.get("Primary Alpha"))

            variant = BuildersMixin()._ensure_illum_decal_host_rgb_variant(decal, (0.1, 0.2, 0.3))

            variant_illum = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime Illum"
            )
            self.assertEqual(variant.get("starbreaker_decal_host_composite_mode"), "illum_primary_color_v2")
            primary_color = variant_illum.inputs.get("Primary Color")
            primary_alpha = variant_illum.inputs.get("Primary Alpha")
            self.assertEqual(primary_alpha.links, [])
            self.assertEqual(primary_alpha.default_value, 1.0)
            self.assertEqual(primary_color.links[0].from_socket.name, "Color")
            self.assertEqual(primary_color.links[0].from_socket.node.name, "SB_DecalHostCompositeMix")
            mix = primary_color.links[0].from_socket.node
            self.assertEqual(mix.inputs[2].links[0].from_socket.node.name, "Layer Surface")
            self.assertEqual(mix.inputs[2].links[0].from_socket.name, "Color")
            self.assertEqual(variant.blend_method, "OPAQUE")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_illum_opacity_nearest_host_rebind_keeps_original_decal_material(self) -> None:
        decal = FakeMaterial(
            "optc_s03_behr_02_mat:decals",
            starbreaker_shader_family="Illum",
            **{
                PROP_HAS_POM: False,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "Illum",
                        "decoded_feature_flags": {
                            "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                            "has_decal": True,
                        }
                    }
                ),
            },
        )
        host = FakeMaterial(
            "optc_s03_behr_02_mat:H_Paint_01_B",
            starbreaker_shader_family="Illum",
        )
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(0, [3, 4, 5]),
            ],
            vertex_count=6,
            vertices=[
                (0.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (0.0, 1.0, 0.0),
                (0.1, 0.1, 0.0),
                (1.1, 0.1, 0.0),
                (0.1, 1.1, 0.0),
            ],
        )
        obj = FakeObject(material_slots=[FakeSlot(decal), FakeSlot(host)], mesh=mesh)
        importer = ImporterUnderTest(channel=None, fallback_rgb=(0.02, 0.03, 0.04))

        rebound = importer._rebind_illum_opacity_decals_by_nearest_host(obj, None, [(0, decal)])

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.illum_host_decal_calls, [])
        self.assertEqual(importer.illum_decal_material_calls, [])
        self.assertEqual(importer.illum_decal_rgb_calls, [])
        self.assertEqual(mesh.polygons[1].material_index, 0)
        self.assertEqual(len(mesh.materials), 2)

    def test_host_with_illum_decal_variant_is_disabled(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs.get("Color"), layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs.get("Alpha"), layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs.get("Color"), illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs.get("Alpha"), illum.inputs.get("Primary Alpha"))

            host = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B")
            host_layered = FakeNode(
                "ShaderNodeGroup",
                name="Host LayeredInputs",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayeredInputs"),
                outputs=["Color"],
            )
            host_principled = FakeNode(
                "ShaderNodeGroup",
                name="Host Principled",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color", "Alpha", "Normal Color"],
                outputs=["Shader"],
            )
            host.node_tree.nodes.extend([host_layered, host_principled])
            host.node_tree.links.new(host_layered.outputs.get("Color"), host_principled.inputs.get("Base Color"))

            variant = BuildersMixin()._ensure_host_with_illum_decal_opacity_variant(decal, host)

            self.assertIsNone(variant)
            self.assertIsNone(bpy.data.materials.get("optc_s03_behr_02_mat:H_Paint_01_B__decal_e54f8ee9"))
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_host_with_illum_decal_variant_does_not_clone_for_public_opacity(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial(
                "optc_s03_behr_02_mat:decals",
                **{
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "Illum",
                            "decoded_feature_flags": {
                                "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                                "has_decal": True,
                            },
                            "public_params": {
                                "DecalDiffuseOpacity": 0.5,
                                "DecalAlphaMult": 0.25,
                            },
                        }
                    )
                },
            )
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs.get("Color"), layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs.get("Alpha"), layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs.get("Color"), illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs.get("Alpha"), illum.inputs.get("Primary Alpha"))

            host = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B")
            host_layered = FakeNode(
                "ShaderNodeGroup",
                name="Host LayeredInputs",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayeredInputs"),
                outputs=["Color"],
            )
            host_principled = FakeNode(
                "ShaderNodeGroup",
                name="Host Principled",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color", "Alpha"],
                outputs=["Shader"],
            )
            host.node_tree.nodes.extend([host_layered, host_principled])
            host.node_tree.links.new(host_layered.outputs.get("Color"), host_principled.inputs.get("Base Color"))

            variant = BuildersMixin()._ensure_host_with_illum_decal_opacity_variant(decal, host)

            self.assertIsNone(variant)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_host_with_illum_decal_variant_does_not_replace_stale_clone(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            decal = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
            image = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
            layer = FakeNode(
                "ShaderNodeGroup",
                name="Layer Surface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Base Color", "Base Alpha"],
                outputs=["Color", "Alpha"],
            )
            illum = FakeNode(
                "ShaderNodeGroup",
                name="Illum",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Illum"),
                inputs=["Primary Color", "Primary Alpha"],
                outputs=["Shader"],
            )
            decal.node_tree.nodes.extend([image, layer, illum])
            decal.node_tree.links.new(image.outputs.get("Color"), layer.inputs.get("Base Color"))
            decal.node_tree.links.new(image.outputs.get("Alpha"), layer.inputs.get("Base Alpha"))
            decal.node_tree.links.new(layer.outputs.get("Color"), illum.inputs.get("Primary Color"))
            decal.node_tree.links.new(layer.outputs.get("Alpha"), illum.inputs.get("Primary Alpha"))

            host = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B")
            host_layered = FakeNode(
                "ShaderNodeGroup",
                name="Host LayeredInputs",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayeredInputs"),
                outputs=["Color"],
            )
            host_principled = FakeNode(
                "ShaderNodeGroup",
                name="Host Principled",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color"],
                outputs=["Shader"],
            )
            host.node_tree.nodes.extend([host_layered, host_principled])
            host.node_tree.links.new(host_layered.outputs.get("Color"), host_principled.inputs.get("Base Color"))

            old = FakeNodeMaterial("optc_s03_behr_02_mat:H_Paint_01_B__decal_deadbeef")
            old["starbreaker_illum_decal_composite_mode"] = "host_with_illum_decal_opacity_v1"
            bpy.data.materials[old.name] = old

            variant = BuildersMixin()._ensure_host_with_illum_decal_opacity_variant(decal, host)

            self.assertIsNone(variant)
            self.assertIs(bpy.data.materials.get(old.name), old)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_pom_uses_actual_host_material_variant(self) -> None:
        pom_control = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                        ],
                    }
                ),
            },
        )
        host = FakeMaterial(
            "test:H_Paint_01_B",
            starbreaker_shader_family="LayerBlend",
            **{
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {"palette_routing": {"material_channel": {"name": "secondary"}}}
                )
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control), FakeSlot(host)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = MeshDecalRebindImporterUnderTest(channel="secondary")

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_variant_calls, [])
        self.assertEqual(
            importer.mesh_host_material_variant_calls,
            [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B")],
        )
        self.assertEqual(obj.material_slots[0].material.name, "bsnp_fps_behr_p6lr_mat:poms__host_material_test:H_Paint_01_B")

    def test_control_only_mesh_decal_pom_ignores_object_level_rgb_fallback(self) -> None:
        pom_control = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                        ],
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = MeshDecalRebindImporterUnderTest(channel=None)

        rebound = importer._rebind_mesh_decal_for_host(
            obj,
            None,
            fallback_rgb=(0.023, 0.023, 0.023),
        )

        self.assertEqual(rebound, 0)
        self.assertEqual(importer.mesh_variant_calls, [])
        self.assertEqual(importer.mesh_rgb_variant_calls, [])
        self.assertIs(obj.material_slots[0].material, pom_control)

    def test_control_only_mesh_decal_pom_spatial_rebind_is_fps_weapon_only(self) -> None:
        pom_control = FakeMaterial(
            "ship_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                    }
                ),
            },
        )
        host = FakeMaterial("ship_mat:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(0, [3, 4, 5]),
            ],
            vertex_count=6,
            vertices=[
                (0.0, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.0, 0.2, 0.0),
                (0.05, 0.05, 0.05),
                (0.15, 0.05, 0.05),
                (0.05, 0.15, 0.05),
            ],
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control), FakeSlot(host)],
            mesh=mesh,
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = MeshDecalRebindImporterUnderTest(channel="secondary", assembly_kind="ship")

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_variant_calls, [("ship_mat:poms", "secondary")])
        self.assertEqual(importer.mesh_host_material_variant_calls, [])
        self.assertEqual(obj.material_slots[0].material.name, "ship_mat:poms__host_secondary")
        self.assertEqual(mesh.polygons[1].material_index, 0)

    def test_control_only_mesh_decal_pom_ship_rebind_allows_rgb_fallback(self) -> None:
        pom_control = FakeMaterial(
            "ship_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = MeshDecalRebindImporterUnderTest(channel=None, assembly_kind="ship")

        rebound = importer._rebind_mesh_decal_for_host(
            obj,
            None,
            fallback_rgb=(0.2, 0.3, 0.4),
        )

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_rgb_variant_calls, [("ship_mat:poms", (0.2, 0.3, 0.4))])
        self.assertEqual(obj.material_slots[0].material.name, "ship_mat:poms__host_rgb")

    def test_control_only_mesh_decal_pom_rebind_reads_fps_weapon_from_package_root(self) -> None:
        pom_control = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                    }
                ),
            },
        )
        host = FakeMaterial("test:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(0, [3, 4, 5]),
            ],
            vertex_count=6,
            vertices=[
                (0.0, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.0, 0.2, 0.0),
                (0.05, 0.05, 0.05),
                (0.15, 0.05, 0.05),
                (0.05, 0.15, 0.05),
            ],
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_control), FakeSlot(host)],
            mesh=mesh,
        )
        package_root = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            starbreaker_package_root=True,
            **{PROP_ASSEMBLY_KIND: "fps_weapon"},
        )
        obj.parent = package_root
        importer = MeshDecalRebindImporterUnderTest(assembly_kind=None)

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 1)
        self.assertEqual(
            importer.mesh_host_material_variant_calls,
            [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B")],
        )
        self.assertEqual(mesh.polygons[1].material_index, 2)

    def test_precomputed_mesh_pom_host_route_still_uses_spatial_rebind_when_hosts_exist(self) -> None:
        poms = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                    }
                ),
            },
        )
        primary_host = FakeMaterial(
            "test:H_Paint_01_B",
            starbreaker_shader_family="LayerBlend",
            **{
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {"palette_routing": {"material_channel": {"name": "primary"}}}
                )
            },
        )
        secondary_host = FakeMaterial(
            "test:H_Parkerized_01_B",
            starbreaker_shader_family="LayerBlend",
            **{
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {"palette_routing": {"material_channel": {"name": "secondary"}}}
                )
            },
        )
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(2, [3, 4, 5]),
                FakePolygon(0, [6, 7, 8]),
                FakePolygon(0, [9, 10, 11]),
            ],
            vertex_count=12,
            vertices=[
                (0.0, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.0, 0.2, 0.0),
                (10.0, 0.0, 0.0),
                (10.2, 0.0, 0.0),
                (10.0, 0.2, 0.0),
                (0.05, 0.05, 0.05),
                (0.15, 0.05, 0.05),
                (0.05, 0.15, 0.05),
                (10.05, 0.05, 0.05),
                (10.15, 0.05, 0.05),
                (10.05, 0.15, 0.05),
            ],
        )
        obj = FakeObject(
            material_slots=[FakeSlot(poms), FakeSlot(primary_host), FakeSlot(secondary_host)],
            mesh=mesh,
            **{PROP_DECAL_HOST_CHANNEL: "secondary"},
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = MeshDecalRebindImporterUnderTest(channel=None)

        rebound = importer._rebind_mesh_decal_for_host(
            obj,
            palette,
            host_channel="secondary",
        )

        self.assertEqual(rebound, 2)
        self.assertEqual(importer.mesh_variant_calls, [])
        self.assertEqual(
            importer.mesh_host_material_variant_calls,
            [
                ("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B"),
                ("bsnp_fps_behr_p6lr_mat:poms", "test:H_Parkerized_01_B"),
            ],
        )
        self.assertEqual(mesh.polygons[2].material_index, 3)
        self.assertEqual(mesh.polygons[3].material_index, 4)

    def test_mesh_pom_spatial_rebind_ignores_generated_host_variants_as_hosts(self) -> None:
        payload = {
            "shader_family": "MeshDecal",
            "decoded_feature_flags": {
                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                "has_decal": False,
                "has_stencil_map": False,
                "has_parallax_occlusion_mapping": True,
            },
        }
        poms = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{PROP_HAS_POM: True, PROP_TEMPLATE_KEY: "decal_stencil", PROP_SUBMATERIAL_JSON: json.dumps(payload)},
        )
        stale_variant = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms__host_old",
            starbreaker_shader_family="LayerBlend",
            **{"starbreaker_decal_host_material_key": "old"},
        )
        real_host = FakeMaterial("test:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(2, [3, 4, 5]),
                FakePolygon(0, [6, 7, 8]),
            ],
            vertex_count=9,
            vertices=[
                (0.01, 0.0, 0.0),
                (0.01, 0.0, 0.0),
                (0.01, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0),
            ],
        )
        obj = FakeObject(
            material_slots=[FakeSlot(poms), FakeSlot(stale_variant), FakeSlot(real_host)],
            mesh=mesh,
        )
        importer = MeshDecalRebindImporterUnderTest()

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 1)
        self.assertEqual(
            importer.mesh_host_material_variant_calls,
            [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B")],
        )
        self.assertEqual(mesh.polygons[2].material_index, 3)

    def test_mesh_pom_spatial_rebind_uses_local_host_neighborhood(self) -> None:
        payload = {
            "shader_family": "MeshDecal",
            "decoded_feature_flags": {
                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                "has_decal": False,
                "has_stencil_map": False,
                "has_parallax_occlusion_mapping": True,
            },
        }
        poms = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{PROP_HAS_POM: True, PROP_TEMPLATE_KEY: "decal_stencil", PROP_SUBMATERIAL_JSON: json.dumps(payload)},
        )
        stray_host = FakeMaterial("test:H_Paint_01_A", starbreaker_shader_family="LayerBlend")
        receiver_host = FakeMaterial("test:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
        polygons = [FakePolygon(1, [0, 1, 2])]
        vertices = [(0.1, 0.0, 0.0)] * 3
        for index in range(7):
            base = len(vertices)
            y = float(index) * 0.001
            polygons.append(FakePolygon(2, [base, base + 1, base + 2]))
            vertices.extend([(0.11, y, 0.0)] * 3)
        decal_base = len(vertices)
        polygons.append(FakePolygon(0, [decal_base, decal_base + 1, decal_base + 2]))
        vertices.extend([(0.0, 0.0, 0.0)] * 3)
        mesh = FakeMesh(polygons=polygons, vertex_count=len(vertices), vertices=vertices)
        obj = FakeObject(
            material_slots=[FakeSlot(poms), FakeSlot(stray_host), FakeSlot(receiver_host)],
            mesh=mesh,
        )
        importer = MeshDecalRebindImporterUnderTest()

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 1)
        self.assertEqual(
            importer.mesh_host_material_variant_calls,
            [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B")],
        )
        self.assertEqual(mesh.polygons[-1].material_index, 3)

    def test_precomputed_control_only_mesh_pom_without_host_reverts_old_channel_variant(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            payload = {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {"slot": "TexSlot1", "role": "base_color"},
                    {"slot": "TexSlot3", "role": "normal_gloss"},
                    {"slot": "TexSlot4", "role": "height"},
                ],
            }
            base = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{PROP_HAS_POM: True, PROP_SUBMATERIAL_JSON: json.dumps(payload)},
            )
            old_variant = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms__host_primary",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(payload),
                    "starbreaker_decal_host_channel": "primary",
                },
            )
            bpy.data.materials[base.name] = base
            obj = FakeObject(
                material_slots=[FakeSlot(old_variant)],
                mesh=FakeMesh(polygons=[], vertex_count=0),
            )
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )
            importer = MeshDecalRebindImporterUnderTest(channel="primary")

            rebound = importer._rebind_mesh_decal_for_host(obj, palette, host_channel="primary")

            self.assertEqual(rebound, 1)
            self.assertIs(obj.material_slots[0].material, base)
            self.assertEqual(importer.mesh_variant_calls, [])
            self.assertEqual(importer.mesh_host_material_variant_calls, [])
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_variant_blends_host_tint_and_keeps_texture(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "TexSlot1_DecalSource_alpha",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            alpha_source = FakeNode("ShaderNodeTexImage", name="POM Mask", outputs=["Color", "Alpha"])
            pom_control.node_tree.nodes.append(alpha_source)
            pom_control.node_tree.links.new(alpha_source.outputs.get("Alpha"), decal_group.inputs.get("TexSlot1_DecalSource_alpha"))
            decal_group.inputs.get("TexSlot1_DecalSource_alpha").default_value = 0.0
            decal_group.inputs.get("Param_DecalDiffuseOpacity").default_value = 0.0
            decal_group.inputs.get("Param_DecalAlphaMultiplier").default_value = 0.0
            pom_control.node_tree.nodes.append(decal_group)
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_variant(
                pom_control,
                "secondary",
                palette,
            )

            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            self.assertEqual(variant.name, "bsnp_fps_behr_p6lr_mat:poms__host_secondary")
            self.assertEqual(variant.get("starbreaker_mesh_decal_variant_mode"), "control_only_pom_host_tinted_v8")
            self.assertEqual(variant_group.inputs.get("TexSlot1_DecalSource").default_value, (1.0, 1.0, 1.0, 1.0))
            alpha = variant_group.inputs.get("TexSlot1_DecalSource_alpha")
            self.assertEqual(alpha.default_value, 0.0)
            self.assertEqual(alpha.links[0].from_socket.node.name, "POM Mask")
            self.assertEqual(alpha.links[0].from_socket.name, "Alpha")
            self.assertEqual(variant_group.inputs.get("Param_DecalDiffuseOpacity").default_value, 1.0)
            self.assertEqual(variant_group.inputs.get("Param_DecalAlphaMultiplier").default_value, 1.0)
            host_tint = variant_group.inputs.get("Host Tint")
            self.assertEqual(len(host_tint.links), 1)
            self.assertEqual(host_tint.links[0].from_socket.node.name, "Palette")
            self.assertEqual(host_tint.links[0].from_socket.name, "Secondary")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_variant_rebuilds_stale_white_clone(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {
                                    "slot": "TexSlot1",
                                    "role": "base_color",
                                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                                },
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            base_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            texture = FakeNode("ShaderNodeTexImage", name="POM Diffuse", outputs=["Color", "Alpha"])
            pom_control.node_tree.nodes.extend([texture, base_group])
            pom_control.node_tree.links.new(texture.outputs.get("Color"), base_group.inputs.get("TexSlot1_DecalSource"))

            stale_clone = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms__host_secondary",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: pom_control.get(PROP_SUBMATERIAL_JSON),
                    "starbreaker_decal_host_channel": "secondary",
                    "starbreaker_mesh_decal_variant_mode": "control_only_pom_host_tinted_v6",
                },
            )
            stale_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            stale_group.inputs.get("TexSlot1_DecalSource").default_value = (1.0, 1.0, 1.0, 1.0)
            stale_clone.node_tree.nodes.append(stale_group)
            bpy.data.materials[pom_control.name] = pom_control
            bpy.data.materials[stale_clone.name] = stale_clone

            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_variant(
                pom_control,
                "secondary",
                palette,
            )

            self.assertIsNot(variant, stale_clone)
            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            self.assertEqual(variant.get("starbreaker_mesh_decal_variant_mode"), "control_only_pom_host_tinted_v8")
            tex_source = variant_group.inputs.get("TexSlot1_DecalSource")
            self.assertEqual(len(tex_source.links), 1)
            self.assertEqual(tex_source.links[0].from_socket.node.name, "POM Diffuse")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_pom_builds_relief_material_with_diffuse_alpha_mask(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "public_params": {"PomDisplacement": 0.001481},
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    },
                    {
                        "slot": "TexSlot3",
                        "role": "normal_gloss",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                    },
                    {
                        "slot": "TexSlot4",
                        "role": "height",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_height_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat_rrs_arctic_ops:poms")

        ok = ControlOnlyPomReliefBuilderUnderTest()._build_control_only_mesh_decal_pom_material(
            material,
            submaterial,
            types.SimpleNamespace(shadow_method="OPAQUE"),
        )

        self.assertTrue(ok)
        image_names = {node.name for node in material.node_tree.nodes if node.bl_idname == "ShaderNodeTexImage"}
        self.assertIn("behr_pom_diff_TEX0.png", image_names)
        self.assertIn("behr_pom_ddna_TEX0.png", image_names)
        self.assertIn("behr_pom_height_TEX0.png", image_names)
        relief_group = next(node for node in material.node_tree.nodes if node.bl_idname == "ShaderNodeGroup")
        self.assertEqual(relief_group.label, "StarBreaker POM Relief")
        self.assertEqual(len(relief_group.inputs.get("Base Color").links), 0)
        self.assertEqual(len(relief_group.inputs.get("Alpha").links), 1)
        self.assertEqual(relief_group.inputs.get("Alpha").links[0].from_socket.node.name, "behr_pom_diff_TEX0.png")
        self.assertEqual(relief_group.inputs.get("Alpha").links[0].from_socket.name, "Alpha")
        self.assertEqual(len(relief_group.inputs.get("Normal Color").links), 1)
        self.assertEqual(relief_group.inputs.get("Normal Color").links[0].from_socket.node.name, "behr_pom_ddna_TEX0.png")
        self.assertEqual(len(relief_group.inputs.get("Height").links), 1)
        self.assertEqual(material.get("starbreaker_mesh_decal_material_mode"), "control_only_pom_relief_v1")
        self.assertEqual(material.blend_method, "HASHED")

    def test_control_only_mesh_decal_pom_contract_relief_is_fps_weapon_only(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "path": "ship_pom_diff.dds",
                        "export_path": "textures/ship_pom_diff_TEX0.png",
                    }
                ],
                "public_params": {
                    "DecalDiffuseOpacity": 0.65,
                    "DecalAlphaMultiplier": 0.75,
                },
            }
        )
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[
                ContractInput(
                    name="TexSlot1_DecalSource",
                    socket_type="NodeSocketColor",
                    semantic="decal_source",
                    source_slot="TexSlot1",
                    required=False,
                    default_value=None,
                    raw={},
                ),
                ContractInput(
                    name="Param_DecalDiffuseOpacity",
                    socket_type="NodeSocketFloat",
                    semantic="public_param_decaldiffuseopacity",
                    source_slot=None,
                    required=False,
                    default_value=None,
                    raw={},
                ),
                ContractInput(
                    name="Param_DecalAlphaMultiplier",
                    socket_type="NodeSocketFloat",
                    semantic="public_param_decalalphamultiplier",
                    source_slot=None,
                    required=False,
                    default_value=None,
                    raw={},
                ),
            ],
            metadata={},
            raw={},
        )
        plan = types.SimpleNamespace(uses_alpha=False, blend_method="BLEND", shadow_method="HASHED")

        ship_material = FakeNodeMaterial("ship_mat:poms")
        ship_ok = ControlOnlyPomReliefBuilderUnderTest(assembly_kind="ship")._build_contract_group_material(
            ship_material,
            submaterial,
            None,
            plan,
            group_contract,
        )

        self.assertTrue(ship_ok)
        self.assertIsNone(ship_material.get("starbreaker_mesh_decal_material_mode"))
        ship_group = next(node for node in ship_material.node_tree.nodes if node.bl_idname == "ShaderNodeGroup")
        self.assertEqual(ship_group.node_tree.name, "SB_MeshDecal_v1")
        self.assertNotEqual(ship_group.label, "StarBreaker POM Relief")
        self.assertEqual(len(ship_group.inputs.get("TexSlot1_DecalSource").links), 1)
        self.assertEqual(
            ship_group.inputs.get("TexSlot1_DecalSource").links[0].from_socket.node.name,
            "ship_pom_diff_TEX0.png",
        )
        self.assertEqual(ship_group.inputs.get("Param_DecalDiffuseOpacity").default_value, 0.65)
        self.assertEqual(ship_group.inputs.get("Param_DecalAlphaMultiplier").default_value, 0.75)

        weapon_material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat:poms")
        weapon_ok = ControlOnlyPomReliefBuilderUnderTest(assembly_kind="fps_weapon")._build_contract_group_material(
            weapon_material,
            submaterial,
            None,
            plan,
            group_contract,
        )

        self.assertTrue(weapon_ok)
        self.assertEqual(weapon_material.get("starbreaker_mesh_decal_material_mode"), "control_only_pom_relief_v1")
        weapon_group = next(node for node in weapon_material.node_tree.nodes if node.bl_idname == "ShaderNodeGroup")
        self.assertEqual(weapon_group.label, "StarBreaker POM Relief")

    def test_control_only_mesh_decal_relief_host_variant_uses_host_material_with_pom_overlay(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            payload = {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    },
                    {
                        "slot": "TexSlot3",
                        "role": "normal_gloss",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                    },
                    {
                        "slot": "TexSlot4",
                        "role": "height",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_height_TEX0.png",
                    },
                ],
            }
            pom_relief = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(payload),
                    "starbreaker_mesh_decal_material_mode": "control_only_pom_relief_v1",
                },
            )
            relief_group = FakeNode(
                "ShaderNodeGroup",
                name="POM Relief",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime Principled"),
                inputs=["Base Color", "Alpha", "Normal Color", "Height"],
                outputs=["Shader"],
            )
            relief_group.label = "StarBreaker POM Relief"
            alpha = FakeNode("ShaderNodeTexImage", name="POM Alpha", outputs=["Color", "Alpha"])
            normal = FakeNode("ShaderNodeTexImage", name="POM Normal", outputs=["Color", "Alpha"])
            height = FakeNode("ShaderNodeTexImage", name="POM Height", outputs=["Color", "Alpha"])
            pom_relief.node_tree.nodes.extend([alpha, normal, height, relief_group])
            pom_relief.node_tree.links.new(alpha.outputs.get("Alpha"), relief_group.inputs.get("Alpha"))
            pom_relief.node_tree.links.new(normal.outputs.get("Color"), relief_group.inputs.get("Normal Color"))
            pom_relief.node_tree.links.new(height.outputs.get("Color"), relief_group.inputs.get("Height"))

            host_material = FakeNodeMaterial(
                "optc_s03_behr_02_mat:H_Paint_01_B",
                starbreaker_shader_family="HardSurface",
                **{PROP_MATERIAL_IDENTITY: "host-material-identity"},
            )
            real_color = FakeNode("ShaderNodeRGB", name="Actual Host Color", outputs=["Color"])
            hard_surface = FakeNode(
                "ShaderNodeGroup",
                name="Host HardSurface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime HardSurface"),
                inputs=["Primary Color", "Primary Normal", "Alpha", "Height"],
                outputs=["Shader"],
            )
            host_material.node_tree.nodes.extend([real_color, hard_surface])
            host_material.node_tree.links.new(real_color.outputs.get("Color"), hard_surface.inputs.get("Primary Color"))

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_material_variant(
                pom_relief,
                host_material,
            )

            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime HardSurface"
            )
            primary_color = variant_group.inputs.get("Primary Color")
            self.assertEqual(len(primary_color.links), 1)
            self.assertEqual(primary_color.links[0].from_socket.node.name, "Actual Host Color")
            normal_input = variant_group.inputs.get("Primary Normal")
            self.assertEqual(len(normal_input.links), 1)
            self.assertEqual(normal_input.links[0].from_socket.node.name, "SB_POM_POM Normal")
            alpha_input = variant_group.inputs.get("Alpha")
            self.assertEqual(len(alpha_input.links), 1)
            self.assertEqual(alpha_input.links[0].from_socket.node.name, "SB_POM_POM Alpha")
            height_input = variant_group.inputs.get("Height")
            self.assertEqual(len(height_input.links), 1)
            height_separate = height_input.links[0].from_socket.node
            self.assertEqual(height_separate.name, "SB_POM_POM Height Channel")
            # Regression: the height texture must actually feed the channel split.
            # The old socket-copy dropped it, leaving an orphaned SeparateColor
            # (height never reached the shader).
            height_color_in = height_separate.inputs.get("Color")
            self.assertEqual(len(height_color_in.links), 1)
            self.assertEqual(height_color_in.links[0].from_socket.node.name, "SB_POM_POM Height")
            self.assertEqual(variant.get("starbreaker_mesh_decal_variant_mode"), "control_only_pom_host_material_v5")
            self.assertEqual(variant.get("starbreaker_shader_family"), "HardSurface")
            self.assertFalse(
                any(getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1" for node in variant.node_tree.nodes)
            )
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_variant_rebuilds_clone_from_other_skin(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_MATERIAL_IDENTITY: "current-skin-material",
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {
                                    "slot": "TexSlot1",
                                    "role": "base_color",
                                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                                },
                            ],
                        }
                    ),
                },
            )
            base_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=["TexSlot1_DecalSource", "Host Tint"],
                outputs=["Shader"],
            )
            current_texture = FakeNode("ShaderNodeTexImage", name="Current Skin POM Diffuse", outputs=["Color", "Alpha"])
            pom_control.node_tree.nodes.extend([current_texture, base_group])
            pom_control.node_tree.links.new(current_texture.outputs.get("Color"), base_group.inputs.get("TexSlot1_DecalSource"))

            stale_clone = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms__host_secondary",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: pom_control.get(PROP_SUBMATERIAL_JSON),
                    "starbreaker_decal_host_channel": "secondary",
                    "starbreaker_decal_host_base_key": "previous-skin-key",
                    "starbreaker_mesh_decal_variant_mode": "control_only_pom_host_tinted_v7",
                },
            )
            stale_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=["TexSlot1_DecalSource", "Host Tint"],
                outputs=["Shader"],
            )
            old_texture = FakeNode("ShaderNodeTexImage", name="Previous Skin POM Diffuse", outputs=["Color", "Alpha"])
            stale_clone.node_tree.nodes.extend([old_texture, stale_group])
            stale_clone.node_tree.links.new(old_texture.outputs.get("Color"), stale_group.inputs.get("TexSlot1_DecalSource"))
            bpy.data.materials[pom_control.name] = pom_control
            bpy.data.materials[stale_clone.name] = stale_clone

            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_variant(
                pom_control,
                "secondary",
                palette,
            )

            self.assertIsNot(variant, stale_clone)
            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            tex_source = variant_group.inputs.get("TexSlot1_DecalSource")
            self.assertEqual(len(tex_source.links), 1)
            self.assertEqual(tex_source.links[0].from_socket.node.name, "Current Skin POM Diffuse")
            self.assertNotEqual(variant.get("starbreaker_decal_host_base_key"), "previous-skin-key")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_black_palette_channel_keeps_neutral_host_tint(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            decal_group.inputs.get("Host Tint").default_value = (0.0, 0.0, 0.0, 1.0)
            pom_control.node_tree.nodes.append(decal_group)
            palette = types.SimpleNamespace(
                primary=(0.0, 0.0, 0.0),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_variant(
                pom_control,
                "primary",
                palette,
            )

            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            host_tint = variant_group.inputs.get("Host Tint")
            self.assertEqual(len(host_tint.links), 0)
            self.assertEqual(host_tint.default_value, (1.0, 1.0, 1.0, 1.0))
            self.assertEqual(variant_group.inputs.get("Param_DecalDiffuseOpacity").default_value, 1.0)
            self.assertEqual(variant_group.inputs.get("Param_DecalAlphaMultiplier").default_value, 1.0)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_material_variant_copies_host_color_chain(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            pom_control.node_tree.nodes.append(decal_group)

            host_material = FakeNodeMaterial(
                "optc_s03_behr_02_mat:H_Paint_01_B",
                starbreaker_shader_family="LayerBlend",
                **{PROP_MATERIAL_IDENTITY: "host-material-identity"},
            )
            host_color = FakeNode(
                "ShaderNodeGroup",
                name="Host LayerSurface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                outputs=["Color"],
            )
            host_material.node_tree.nodes.append(host_color)

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_material_variant(
                pom_control,
                host_material,
            )

            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            host_tint = variant_group.inputs.get("Host Tint")
            self.assertEqual(len(host_tint.links), 1)
            self.assertEqual(host_tint.links[0].from_socket.name, "Color")
            self.assertEqual(host_tint.links[0].from_socket.node.name, "SB_HostMaterial_Host LayerSurface")
            self.assertEqual(variant.get("starbreaker_mesh_decal_variant_mode"), "control_only_pom_host_material_v5")
            self.assertEqual(variant.get("starbreaker_decal_host_material_name"), "optc_s03_behr_02_mat:H_Paint_01_B")
            self.assertEqual(variant_group.inputs.get("Param_DecalDiffuseOpacity").default_value, 1.0)
            self.assertEqual(variant_group.inputs.get("Param_DecalAlphaMultiplier").default_value, 1.0)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_material_variant_does_not_copy_host_normal_chain(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=["TexSlot1_DecalSource", "Param_DecalDiffuseOpacity", "Param_DecalAlphaMultiplier", "Host Tint"],
                outputs=["Shader"],
            )
            pom_control.node_tree.nodes.append(decal_group)

            host_material = FakeNodeMaterial(
                "optc_s03_behr_02_mat:H_Paint_01_B",
                starbreaker_shader_family="LayerBlend",
                **{PROP_MATERIAL_IDENTITY: "host-material-identity"},
            )
            host_color = FakeNode(
                "ShaderNodeGroup",
                name="Host LayerSurface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                inputs=["Primary Color", "Primary Normal"],
                outputs=["Color"],
            )
            basic_image = FakeNode("ShaderNodeTexImage", name="Basic Image", outputs=["Color", "Alpha"])
            host_material.node_tree.nodes.extend([host_color, basic_image])
            host_material.node_tree.links.new(basic_image.outputs.get("Color"), host_color.inputs.get("Primary Normal"))
            for node in host_material.node_tree.nodes:
                node.id_data = host_material.node_tree

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_material_variant(
                pom_control,
                host_material,
            )

            copied_layer = next(
                node
                for node in variant.node_tree.nodes
                if node.name == "SB_HostMaterial_Host LayerSurface"
            )
            normal_input = copied_layer.inputs.get("Primary Normal")
            self.assertEqual(len(normal_input.links), 0)
            self.assertFalse(
                any(node.name == "SB_HostMaterial_Basic Image" for node in variant.node_tree.nodes),
                "host color copy must not bring over a normal-only Basic Image sampler",
            )
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_material_variant_rebuilds_clone_missing_relief_inputs(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            payload = {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    },
                    {
                        "slot": "TexSlot3",
                        "role": "normal_gloss",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                    },
                    {
                        "slot": "TexSlot4",
                        "role": "height",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_height_TEX0.png",
                    },
                ],
            }
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{PROP_HAS_POM: True, PROP_SUBMATERIAL_JSON: json.dumps(payload)},
            )
            base_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=[
                    "TexSlot1_DecalSource",
                    "TexSlot3_NormalGloss",
                    "TexSlot4_Height",
                    "Param_DecalDiffuseOpacity",
                    "Param_DecalAlphaMultiplier",
                    "Host Tint",
                ],
                outputs=["Shader"],
            )
            diffuse = FakeNode("ShaderNodeTexImage", name="POM Diffuse", outputs=["Color", "Alpha"])
            normal = FakeNode("ShaderNodeTexImage", name="POM Normal", outputs=["Color", "Alpha"])
            height = FakeNode("ShaderNodeTexImage", name="POM Height", outputs=["Color", "Alpha"])
            pom_control.node_tree.nodes.extend([diffuse, normal, height, base_group])
            pom_control.node_tree.links.new(diffuse.outputs.get("Color"), base_group.inputs.get("TexSlot1_DecalSource"))
            pom_control.node_tree.links.new(normal.outputs.get("Color"), base_group.inputs.get("TexSlot3_NormalGloss"))
            pom_control.node_tree.links.new(height.outputs.get("Color"), base_group.inputs.get("TexSlot4_Height"))

            host_material = FakeNodeMaterial(
                "optc_s03_behr_02_mat:H_Paint_01_B",
                starbreaker_shader_family="LayerBlend",
                **{PROP_MATERIAL_IDENTITY: "host-material-identity"},
            )
            host_color = FakeNode(
                "ShaderNodeGroup",
                name="Host LayerSurface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                outputs=["Color"],
            )
            host_material.node_tree.nodes.append(host_color)

            stale_clone_name = f"bsnp_fps_behr_p6lr_mat:poms__host_{BuildersMixin._decal_host_material_key(host_material)}"
            stale_clone = FakeNodeMaterial(
                stale_clone_name,
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(payload),
                    "starbreaker_decal_host_material_key": BuildersMixin._decal_host_material_key(host_material),
                    "starbreaker_decal_host_base_key": BuildersMixin._decal_host_material_key(pom_control),
                    "starbreaker_mesh_decal_variant_mode": "control_only_pom_host_material_v2",
                },
            )
            stale_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=["TexSlot1_DecalSource", "TexSlot3_NormalGloss", "TexSlot4_Height", "Host Tint"],
                outputs=["Shader"],
            )
            stale_host = FakeNode("ShaderNodeRGB", name="Stale Host Tint", outputs=["Color"])
            stale_clone.node_tree.nodes.extend([stale_host, stale_group])
            stale_clone.node_tree.links.new(stale_host.outputs.get("Color"), stale_group.inputs.get("Host Tint"))
            bpy.data.materials[stale_clone.name] = stale_clone

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_material_variant(
                pom_control,
                host_material,
            )

            self.assertIsNot(variant, stale_clone)
            self.assertNotEqual(variant.get(PROP_MATERIAL_IDENTITY), host_material.get(PROP_MATERIAL_IDENTITY))
            self.assertTrue(str(variant.get(PROP_MATERIAL_IDENTITY)).startswith("decal_host_variant:"))
            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            self.assertEqual(len(variant_group.inputs.get("TexSlot3_NormalGloss").links), 1)
            self.assertEqual(variant_group.inputs.get("TexSlot3_NormalGloss").links[0].from_socket.node.name, "POM Normal")
            self.assertEqual(len(variant_group.inputs.get("TexSlot4_Height").links), 1)
            self.assertEqual(variant_group.inputs.get("TexSlot4_Height").links[0].from_socket.node.name, "POM Height")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_material_variant_prefers_final_hardsurface_color(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=["TexSlot1_DecalSource", "Param_DecalDiffuseOpacity", "Param_DecalAlphaMultiplier", "Host Tint"],
                outputs=["Shader"],
            )
            pom_control.node_tree.nodes.append(decal_group)

            host_material = FakeNodeMaterial(
                "optc_s03_behr_02_mat:H_Paint_01_B",
                starbreaker_shader_family="HardSurface",
                **{PROP_MATERIAL_IDENTITY: "host-material-identity"},
            )
            white_helper = FakeNode(
                "ShaderNodeGroup",
                name="White Helper",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime LayerSurface"),
                outputs=["Color"],
            )
            real_color = FakeNode("ShaderNodeRGB", name="Actual Host Color", outputs=["Color"])
            hard_surface = FakeNode(
                "ShaderNodeGroup",
                name="Host HardSurface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime HardSurface"),
                inputs=["Primary Color"],
                outputs=["Shader"],
            )
            host_material.node_tree.nodes.extend([white_helper, real_color, hard_surface])
            host_material.node_tree.links.new(real_color.outputs.get("Color"), hard_surface.inputs.get("Primary Color"))

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_material_variant(
                pom_control,
                host_material,
            )

            variant_group = next(
                node
                for node in variant.node_tree.nodes
                if getattr(getattr(node, "node_tree", None), "name", "") == "SB_MeshDecal_v1"
            )
            host_tint = variant_group.inputs.get("Host Tint")
            self.assertEqual(len(host_tint.links), 1)
            self.assertEqual(host_tint.links[0].from_socket.node.name, "SB_HostMaterial_Actual Host Color")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_control_only_mesh_decal_host_material_variant_preserves_rgb_host_color_value(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            pom_control = FakeNodeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            decal_group = FakeNode(
                "ShaderNodeGroup",
                name="Mesh Decal",
                node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
                inputs=["TexSlot1_DecalSource", "Param_DecalDiffuseOpacity", "Param_DecalAlphaMultiplier", "Host Tint"],
                outputs=["Shader"],
            )
            pom_control.node_tree.nodes.append(decal_group)

            host_material = FakeNodeMaterial(
                "optc_s03_behr_02_mat:H_Paint_01_B",
                starbreaker_shader_family="HardSurface",
                **{PROP_MATERIAL_IDENTITY: "host-material-identity"},
            )
            authored_color = (0.18, 0.27, 0.36, 1.0)
            real_color = FakeNode("ShaderNodeRGB", name="Actual Host Color", outputs=["Color"])
            real_color.outputs.get("Color").default_value = authored_color
            hard_surface = FakeNode(
                "ShaderNodeGroup",
                name="Host HardSurface",
                node_tree=FakeNodeGroupTree("StarBreaker Runtime HardSurface"),
                inputs=["Primary Color"],
                outputs=["Shader"],
            )
            host_material.node_tree.nodes.extend([real_color, hard_surface])
            host_material.node_tree.links.new(real_color.outputs.get("Color"), hard_surface.inputs.get("Primary Color"))

            variant = MeshDecalVariantImporterUnderTest()._ensure_mesh_decal_host_material_variant(
                pom_control,
                host_material,
            )

            copied_rgb = next(node for node in variant.node_tree.nodes if node.name == "SB_HostMaterial_Actual Host Color")
            self.assertEqual(copied_rgb.outputs.get("Color").default_value, authored_color)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_mesh_pom_decal_polygons_rebind_to_nearest_host_material(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            poms = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            primary_host = FakeMaterial(
                "test:H_Paint_01_B",
                starbreaker_shader_family="LayerBlend",
                **{
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {"palette_routing": {"material_channel": {"name": "primary"}}}
                    )
                },
            )
            secondary_host = FakeMaterial(
                "test:H_Parkerized_01_B",
                starbreaker_shader_family="LayerBlend",
                **{
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {"palette_routing": {"material_channel": {"name": "secondary"}}}
                    )
                },
            )
            mesh = FakeMesh(
                polygons=[
                    FakePolygon(1, [0, 1, 2]),
                    FakePolygon(2, [3, 4, 5]),
                    FakePolygon(0, [6, 7, 8]),
                    FakePolygon(0, [9, 10, 11]),
                ],
                vertex_count=12,
                vertices=[
                    (0.0, 0.0, 0.0),
                    (0.2, 0.0, 0.0),
                    (0.0, 0.2, 0.0),
                    (10.0, 0.0, 0.0),
                    (10.2, 0.0, 0.0),
                    (10.0, 0.2, 0.0),
                    (0.05, 0.05, 0.05),
                    (0.15, 0.05, 0.05),
                    (0.05, 0.15, 0.05),
                    (10.05, 0.05, 0.05),
                    (10.15, 0.05, 0.05),
                    (10.05, 0.15, 0.05),
                ],
            )
            obj = FakeObject(
                material_slots=[FakeSlot(poms), FakeSlot(primary_host), FakeSlot(secondary_host)],
                mesh=mesh,
            )
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )
            importer = MeshDecalRebindImporterUnderTest(channel="secondary")

            rebound = importer._rebind_mesh_decal_for_host(obj, palette)

            self.assertEqual(rebound, 2)
            self.assertEqual(importer.mesh_variant_calls, [])
            self.assertEqual(
                importer.mesh_host_material_variant_calls,
                [
                    ("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B"),
                    ("bsnp_fps_behr_p6lr_mat:poms", "test:H_Parkerized_01_B"),
                ],
            )
            self.assertEqual(mesh.polygons[2].material_index, 3)
            self.assertEqual(mesh.polygons[3].material_index, 4)
            self.assertEqual(mesh.materials[3].name, "bsnp_fps_behr_p6lr_mat:poms__host_material_test:H_Paint_01_B")
            self.assertEqual(mesh.materials[4].name, "bsnp_fps_behr_p6lr_mat:poms__host_material_test:H_Parkerized_01_B")
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_connected_mesh_pom_decal_island_uses_one_majority_host_material(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            poms = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            dark_host = FakeMaterial("test:H_Parkerized_01_A", starbreaker_shader_family="LayerBlend")
            light_host = FakeMaterial("test:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
            mesh = FakeMesh(
                polygons=[
                    FakePolygon(1, [0, 1, 2]),
                    FakePolygon(2, [3, 4, 5]),
                    FakePolygon(0, [6, 7, 8]),
                    FakePolygon(0, [8, 9, 10]),
                    FakePolygon(0, [10, 11, 12]),
                ],
                vertex_count=13,
                vertices=[
                    (0.0, 0.0, 0.0),
                    (0.2, 0.0, 0.0),
                    (0.0, 0.2, 0.0),
                    (2.0, 0.0, 0.0),
                    (2.2, 0.0, 0.0),
                    (2.0, 0.2, 0.0),
                    (0.04, 0.04, 0.02),
                    (0.12, 0.04, 0.02),
                    (0.04, 0.12, 0.02),
                    (0.16, 0.12, 0.02),
                    (0.12, 0.20, 0.02),
                    (2.08, 0.04, 0.02),
                    (2.04, 0.12, 0.02),
                ],
            )
            obj = FakeObject(
                material_slots=[FakeSlot(poms), FakeSlot(dark_host), FakeSlot(light_host)],
                mesh=mesh,
            )
            importer = MeshDecalRebindImporterUnderTest(channel=None)

            rebound = importer._rebind_mesh_decal_for_host(obj, None)

            self.assertEqual(rebound, 3)
            self.assertEqual(
                importer.mesh_host_material_variant_calls,
                [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Parkerized_01_A")],
            )
            self.assertEqual(mesh.polygons[2].material_index, 3)
            self.assertEqual(mesh.polygons[3].material_index, 3)
            self.assertEqual(mesh.polygons[4].material_index, 3)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_mesh_pom_decal_host_search_ignores_flat_text_decals(self) -> None:
        import bpy

        original_materials = getattr(bpy.data, "materials", None)
        bpy.data.materials = FakeMaterialsCollection()
        try:
            poms = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:poms",
                starbreaker_shader_family="MeshDecal",
                **{
                    PROP_HAS_POM: True,
                    PROP_SUBMATERIAL_JSON: json.dumps(
                        {
                            "shader_family": "MeshDecal",
                            "decoded_feature_flags": {
                                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                                "has_decal": False,
                                "has_stencil_map": False,
                                "has_parallax_occlusion_mapping": True,
                            },
                            "texture_slots": [
                                {"slot": "TexSlot1", "role": "base_color"},
                                {"slot": "TexSlot3", "role": "normal_gloss"},
                                {"slot": "TexSlot4", "role": "height"},
                            ],
                        }
                    ),
                },
            )
            text_decal = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:decals",
                starbreaker_shader_family="MeshDecal",
                **{PROP_HAS_POM: False},
            )
            glow_decal = FakeMaterial(
                "bsnp_fps_behr_p6lr_mat:glow_mat",
                starbreaker_shader_family="Illum",
                **{PROP_HAS_POM: False},
            )
            dark_host = FakeMaterial("test:H_Parkerized_01_A", starbreaker_shader_family="LayerBlend")
            mesh = FakeMesh(
                polygons=[
                    FakePolygon(2, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                    FakePolygon(3, [6, 7, 8]),
                    FakePolygon(0, [9, 10, 11]),
                ],
                vertex_count=12,
                vertices=[
                    (0.0, 0.0, 0.0),
                    (0.2, 0.0, 0.0),
                    (0.0, 0.2, 0.0),
                    (0.03, 0.03, 0.03),
                    (0.13, 0.03, 0.03),
                    (0.03, 0.13, 0.03),
                    (0.05, 0.05, 0.05),
                    (0.15, 0.05, 0.05),
                    (0.05, 0.15, 0.05),
                    (0.06, 0.06, 0.06),
                    (0.16, 0.06, 0.06),
                    (0.06, 0.16, 0.06),
                ],
            )
            obj = FakeObject(
                material_slots=[FakeSlot(poms), FakeSlot(text_decal), FakeSlot(dark_host), FakeSlot(glow_decal)],
                mesh=mesh,
            )
            importer = MeshDecalRebindImporterUnderTest(channel=None)

            rebound = importer._rebind_mesh_decal_for_host(obj, None)

            self.assertEqual(rebound, 1)
            self.assertEqual(
                importer.mesh_host_material_variant_calls,
                [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Parkerized_01_A")],
            )
            self.assertEqual(mesh.polygons[3].material_index, 4)
        finally:
            if original_materials is None:
                delattr(bpy.data, "materials")
            else:
                bpy.data.materials = original_materials

    def test_paint_rebuild_restores_stale_pom_host_variant_polygons_to_current_base_slot(self) -> None:
        current_poms = FakeMaterial(
            "bsnp_fps_behr_p6lr_store01_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{PROP_HAS_POM: True},
        )
        current_paint = FakeMaterial(
            "bsnp_fps_behr_p6lr_store01_mat:H_Paint_01_B",
            starbreaker_shader_family="LayerBlend_V2",
        )
        stale_poms_host = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms__host_87f505c1",
            starbreaker_shader_family="LayerBlend_V2",
            starbreaker_decal_host_material_name="bsnp_fps_behr_p6lr_mat:H_Paint_01_B",
            starbreaker_decal_host_material_key="87f505c1",
            starbreaker_mesh_decal_variant_mode="control_only_pom_host_material_v5",
        )
        mesh = FakeMesh(
            polygons=[
                FakePolygon(2, [0, 1, 2]),
                FakePolygon(1, [3, 4, 5]),
                FakePolygon(2, [6, 7, 8]),
            ],
            vertex_count=9,
        )
        obj = FakeObject(
            material_slots=[
                FakeSlot(current_poms),
                FakeSlot(current_paint),
                FakeSlot(stale_poms_host),
            ],
            mesh=mesh,
        )
        importer = MeshDecalRebindImporterUnderTest(channel=None)

        changed = importer._restore_generated_decal_host_variant_polygons(
            obj,
            protected_slot_count=2,
        )

        self.assertEqual(changed, 2)
        self.assertEqual([polygon.material_index for polygon in mesh.polygons], [0, 1, 0])

    def test_control_only_mesh_pom_spatial_rebind_does_not_create_rgb_variant(self) -> None:
        poms = FakeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": False,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                        ],
                    }
                ),
            },
        )
        host = FakeMaterial("test:H_Paint_01_B", starbreaker_shader_family="LayerBlend")
        mesh = FakeMesh(
            polygons=[
                FakePolygon(1, [0, 1, 2]),
                FakePolygon(0, [3, 4, 5]),
            ],
            vertex_count=6,
            vertices=[
                (0.0, 0.0, 0.0),
                (0.2, 0.0, 0.0),
                (0.0, 0.2, 0.0),
                (0.05, 0.05, 0.05),
                (0.15, 0.05, 0.05),
                (0.05, 0.15, 0.05),
            ],
        )
        obj = FakeObject(material_slots=[FakeSlot(poms), FakeSlot(host)], mesh=mesh)
        importer = MeshDecalRebindImporterUnderTest(channel=None, authored_rgb=(0.023, 0.023, 0.023))

        rebound = importer._rebind_mesh_decal_for_host(obj, None)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_variant_calls, [])
        self.assertEqual(importer.mesh_host_material_variant_calls, [("bsnp_fps_behr_p6lr_mat:poms", "test:H_Paint_01_B")])
        self.assertEqual(importer.mesh_rgb_variant_calls, [])
        self.assertEqual(mesh.polygons[1].material_index, 2)
        self.assertEqual(len(mesh.materials), 3)

    def test_visible_mesh_decal_pom_still_receives_host_tint(self) -> None:
        pom_decal = FakeMaterial(
            "drak_clipper_ext:Decal_POM_A",
            starbreaker_shader_family="MeshDecal",
            **{
                PROP_HAS_POM: True,
                PROP_TEMPLATE_KEY: "decal_stencil",
                PROP_SUBMATERIAL_JSON: json.dumps(
                    {
                        "shader_family": "MeshDecal",
                        "decoded_feature_flags": {
                            "tokens": ["DECAL", "PARALLAX_OCCLUSION_MAPPING"],
                            "has_decal": True,
                            "has_stencil_map": False,
                            "has_parallax_occlusion_mapping": True,
                        },
                        "texture_slots": [
                            {"slot": "TexSlot1", "role": "base_color"},
                            {"slot": "TexSlot3", "role": "normal_gloss"},
                            {"slot": "TexSlot4", "role": "height"},
                            {"slot": "TexSlot6", "role": "tint_mask"},
                        ],
                    }
                ),
            },
        )
        obj = FakeObject(
            material_slots=[FakeSlot(pom_decal)],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        palette = types.SimpleNamespace(
            primary=(0.2, 0.3, 0.4),
            secondary=(0.5, 0.6, 0.7),
            tertiary=(0.8, 0.1, 0.2),
            glass=(0.9, 0.9, 0.95),
        )
        importer = MeshDecalRebindImporterUnderTest(channel="secondary")

        rebound = importer._rebind_mesh_decal_for_host(obj, palette)

        self.assertEqual(rebound, 1)
        self.assertEqual(importer.mesh_variant_calls, [("drak_clipper_ext:Decal_POM_A", "secondary")])
        self.assertEqual(obj.material_slots[0].material.name, "drak_clipper_ext:Decal_POM_A__host_secondary")

    def test_mesh_decal_host_variant_names_do_not_chain_existing_host_suffixes(self) -> None:
        bpy = sys.modules["bpy"]
        original_materials = getattr(bpy.data, "materials", None)
        materials = FakeMaterialsCollection()
        bpy.data.materials = materials
        try:
            decal = FakeMaterial(
                "drak_clipper_ext:Decal_POM_A#03e817cc__host_secondary",
                starbreaker_shader_family="MeshDecal",
                **{PROP_HAS_POM: True},
            )
            importer = ImporterUnderTest(channel="tertiary")
            palette = types.SimpleNamespace(
                primary=(0.2, 0.3, 0.4),
                secondary=(0.5, 0.6, 0.7),
                tertiary=(0.8, 0.1, 0.2),
                glass=(0.9, 0.9, 0.95),
            )

            variant = importer._ensure_mesh_decal_host_variant(decal, "tertiary", palette)

            self.assertEqual(
                variant.name,
                "drak_clipper_ext:Decal_POM_A#03e817cc__host_tertiary",
            )
        finally:
            bpy.data.materials = original_materials

    def test_host_channel_scan_returns_single_channel_without_polygon_counts(self) -> None:
        obj = FakeObject(
            material_slots=[FakeSlot(FakeMaterial("drak_clipper_ext_Paint_Secondary"))],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        importer = ImporterUnderTest()

        self.assertEqual(importer._scan_slots_for_host_channel(obj), "secondary")

    def test_host_channel_scan_uses_polygon_weight_when_channels_compete(self) -> None:
        obj = FakeObject(
            material_slots=[
                FakeSlot(FakeMaterial("drak_clipper_ext_Paint_Primary")),
                FakeSlot(FakeMaterial("drak_clipper_ext_Paint_Tertiary")),
            ],
            mesh=FakeMesh(
                polygons=[
                    FakePolygon(0, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                    FakePolygon(1, [6, 7, 8]),
                ],
                vertex_count=9,
            ),
        )
        importer = ImporterUnderTest()

        self.assertEqual(importer._scan_slots_for_host_channel(obj), "tertiary")

    def test_mesh_decal_host_channel_prefers_precomputed_object_property(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_CHANNEL: "glass"},
        )
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_channel_for_object(obj), "glass")

    def test_mesh_decal_host_channel_falls_back_to_parent_precomputed_property(self) -> None:
        parent = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_CHANNEL: "secondary"},
        )
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
        )
        obj.parent = parent
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_channel_for_object(obj), "secondary")

    def test_mesh_decal_host_rgb_prefers_precomputed_object_property(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_RGB: [0.2, 0.4, 0.6]},
        )
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_rgb_for_object(obj), (0.2, 0.4, 0.6))

    def test_mesh_decal_host_rgb_accepts_rust_json_string_property(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[], vertex_count=0),
            **{PROP_DECAL_HOST_RGB: "[0.2,0.4,0.6]"},
        )
        importer = HostRoutingImporterUnderTest()

        self.assertEqual(importer._mesh_decal_host_rgb_for_object(obj), (0.2, 0.4, 0.6))

    def test_derive_decal_host_route_from_submaterials_uses_polygon_weight(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(
                polygons=[
                    FakePolygon(0, [0, 1, 2]),
                    FakePolygon(1, [3, 4, 5]),
                    FakePolygon(1, [6, 7, 8]),
                ],
                vertex_count=9,
            ),
        )
        importer = HostRoutingImporterUnderTest()
        slot_submaterials = [
            SubmaterialRecord.from_value(
                {
                    "shader_family": "HardSurface",
                    "palette_routing": {"material_channel": {"name": "primary"}},
                }
            ),
            SubmaterialRecord.from_value(
                {
                    "shader_family": "HardSurface",
                    "palette_routing": {"material_channel": {"name": "tertiary"}},
                }
            ),
        ]

        channel, rgb = importer._derive_decal_host_route_from_submaterials(obj, slot_submaterials)

        self.assertEqual(channel, "tertiary")
        self.assertIsNone(rgb)

    def test_derive_decal_host_route_from_submaterials_falls_back_to_authored_tint(self) -> None:
        obj = FakeObject(
            material_slots=[],
            mesh=FakeMesh(polygons=[FakePolygon(0, [0, 1, 2])], vertex_count=3),
        )
        importer = HostRoutingImporterUnderTest()
        slot_submaterials = [
            SubmaterialRecord.from_value(
                {
                    "shader_family": "HardSurface",
                    "layer_manifest": [
                        {
                            "tint_color": [0.2, 0.4, 0.6],
                        }
                    ],
                }
            )
        ]

        channel, rgb = importer._derive_decal_host_route_from_submaterials(obj, slot_submaterials)

        self.assertIsNone(channel)
        self.assertEqual(rgb, (0.2, 0.4, 0.6))

    def test_parallax_bias_value_prefers_authored_height_bias(self) -> None:
        importer = ImporterUnderTest()
        submaterial = SubmaterialRecord.from_value(
            {
                "public_params": {
                    "HeightBias": 0.75,
                    "PomDisplacement": 0.04,
                }
            }
        )

        self.assertAlmostEqual(importer._parallax_bias_value(submaterial), 0.75)

    def test_parallax_height_sampler_extension_always_repeats(self) -> None:
        # The POM ray-march walks the UV across the height field, so the height
        # sampler must REPEAT regardless of authored tiling. CLIP returns black
        # outside the 0-1 range and collapses the march (the reference plane
        # goes inert and the parallax degenerates to a constant UV shift, so a
        # decal like "SEMI" renders offset as "EMI-F").
        self.assertEqual(_parallax_height_sampler_extension(1.0), "REPEAT")
        self.assertEqual(_parallax_height_sampler_extension(0.75), "REPEAT")
        self.assertEqual(_parallax_height_sampler_extension(3.0), "REPEAT")

    def test_parallax_bias_value_none_without_authored_height_bias(self) -> None:
        # No HeightBias -> None so the caller falls back to the height-map
        # background (decal atlases ship no HeightBias).
        importer = ImporterUnderTest()
        submaterial = SubmaterialRecord.from_value(
            {"public_params": {"PomDisplacement": 0.04}}
        )
        self.assertIsNone(importer._parallax_bias_value(submaterial))

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
    def test_image_node_keeps_color_and_non_color_uses_on_separate_image_datablocks(self) -> None:
        import bpy
        import tempfile

        class FakeImage:
            def __init__(self, filepath: str):
                self.filepath = filepath
                self.library = None
                self.colorspace_settings = types.SimpleNamespace(name="sRGB")
                self.alpha_mode = "STRAIGHT"

        class FakeImages(list):
            def load(self, filepath: str, *, check_existing: bool = True):
                if check_existing:
                    for image in self:
                        if image.filepath == filepath:
                            return image
                image = FakeImage(filepath)
                self.append(image)
                return image

        class ImageNodeImporter(MaterialsMixin):
            def __init__(self, root: Path, *, assembly_kind: str | None = "fps_weapon") -> None:
                scene_raw = {}
                if assembly_kind is not None:
                    scene_raw["assembly_kind"] = assembly_kind
                self.package = types.SimpleNamespace(
                    resolve_path=lambda value: root / value,
                    scene=types.SimpleNamespace(raw=scene_raw),
                )

        original_images = getattr(bpy.data, "images", None)
        original_path = getattr(bpy, "path", None)
        bpy.data.images = FakeImages()
        bpy.path = types.SimpleNamespace(abspath=lambda filepath, library=None: filepath)
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                root = Path(temp_dir)
                (root / "shared.png").write_bytes(b"not a real png; existence is enough")
                importer = ImageNodeImporter(root)
                nodes = FakeNodes()

                color_node = importer._image_node(nodes, "shared.png", x=0, y=0, is_color=True)
                non_color_node = importer._image_node(nodes, "shared.png", x=0, y=0, is_color=False)

                self.assertIsNotNone(color_node)
                self.assertIsNotNone(non_color_node)
                self.assertIsNot(color_node.image, non_color_node.image)
                self.assertNotEqual(color_node.image.colorspace_settings.name, "Non-Color")
                self.assertEqual(non_color_node.image.colorspace_settings.name, "Non-Color")
        finally:
            bpy.data.images = original_images
            if original_path is None:
                delattr(bpy, "path")
            else:
                bpy.path = original_path

    def test_image_node_uses_legacy_shared_image_datablock_for_non_fps_packages(self) -> None:
        import bpy
        import tempfile

        class FakeImage:
            def __init__(self, filepath: str):
                self.filepath = filepath
                self.library = None
                self.colorspace_settings = types.SimpleNamespace(name="sRGB")
                self.alpha_mode = "STRAIGHT"

        class FakeImages(list):
            def load(self, filepath: str, *, check_existing: bool = True):
                if check_existing:
                    for image in self:
                        if image.filepath == filepath:
                            return image
                image = FakeImage(filepath)
                self.append(image)
                return image

        class ImageNodeImporter(MaterialsMixin):
            def __init__(self, root: Path) -> None:
                self.package = types.SimpleNamespace(
                    resolve_path=lambda value: root / value,
                    scene=types.SimpleNamespace(raw={"assembly_kind": "ship"}),
                )

        original_images = getattr(bpy.data, "images", None)
        original_path = getattr(bpy, "path", None)
        bpy.data.images = FakeImages()
        bpy.path = types.SimpleNamespace(abspath=lambda filepath, library=None: filepath)
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                root = Path(temp_dir)
                (root / "shared.png").write_bytes(b"not a real png; existence is enough")
                importer = ImageNodeImporter(root)
                nodes = FakeNodes()

                color_node = importer._image_node(nodes, "shared.png", x=0, y=0, is_color=True)
                non_color_node = importer._image_node(nodes, "shared.png", x=0, y=0, is_color=False)

                self.assertIsNotNone(color_node)
                self.assertIsNotNone(non_color_node)
                self.assertIs(color_node.image, non_color_node.image)
                self.assertEqual(color_node.image.colorspace_settings.name, "Non-Color")
        finally:
            bpy.data.images = original_images
            if original_path is None:
                delattr(bpy, "path")
            else:
                bpy.path = original_path

    def test_illum_opacity_decal_straight_alpha_image_forces_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "Illum",
                "decoded_feature_flags": {
                    "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                    "has_decal": True,
                    "has_parallax_occlusion_mapping": False,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_decals_diff_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
        image = types.SimpleNamespace(
            filepath="C:/exports/Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_decals_diff_TEX0.png",
            alpha_mode="STRAIGHT",
        )
        node = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
        node.image = image
        material.node_tree.nodes.append(node)

        importer = MaterialReuseImporterUnderTest()

        self.assertTrue(importer._material_needs_illum_decal_alpha_mode_refresh(material, submaterial))

    def test_illum_opacity_decal_alpha_mode_refresh_is_fps_weapon_only(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "Illum",
                "decoded_feature_flags": {
                    "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                    "has_decal": True,
                    "has_parallax_occlusion_mapping": False,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/Spaceships/Ships/ORIG/textures/orig_decals_diff_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("orig_ship:decals")
        image = types.SimpleNamespace(
            filepath="C:/exports/Data/Objects/Spaceships/Ships/ORIG/textures/orig_decals_diff_TEX0.png",
            alpha_mode="STRAIGHT",
        )
        node = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
        node.image = image
        material.node_tree.nodes.append(node)

        importer = MaterialReuseImporterUnderTest(assembly_kind="ship")

        self.assertFalse(importer._material_needs_illum_decal_alpha_mode_refresh(material, submaterial))

    def test_illum_opacity_decal_premul_alpha_image_does_not_force_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "Illum",
                "decoded_feature_flags": {
                    "tokens": ["NORMAL_MAP", "DECAL", "DECAL_OPACITY_MAP"],
                    "has_decal": True,
                    "has_parallax_occlusion_mapping": False,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_decals_diff_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("optc_s03_behr_02_mat:decals")
        image = types.SimpleNamespace(
            filepath="C:/exports/Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_decals_diff_TEX0.png",
            alpha_mode="PREMUL",
        )
        node = FakeNode("ShaderNodeTexImage", name="Decal Image", outputs=["Color", "Alpha"])
        node.image = image
        material.node_tree.nodes.append(node)

        importer = MaterialReuseImporterUnderTest()

        self.assertFalse(importer._material_needs_illum_decal_alpha_mode_refresh(material, submaterial))

    def test_image_node_alpha_mode_setter_marks_premultiplied_decals(self) -> None:
        image = types.SimpleNamespace(alpha_mode="STRAIGHT")

        MaterialsMixin._set_image_alpha_mode(image, "PREMUL")

        self.assertEqual(image.alpha_mode, "PREMUL")

    def test_mesh_decal_patch_v7_material_does_not_force_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_parallax_occlusion_mapping": True,
                },
            }
        )
        material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat:poms")
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree(
                "SB_MeshDecal_v1",
                starbreaker_mesh_decal_emission_patch_version=7,
            ),
            inputs=["Emission Strength", "Use Vert Col"],
        )
        group.inputs.get("Emission Strength").default_value = 0.0
        group.inputs.get("Use Vert Col").default_value = False
        material.node_tree.nodes.append(group)

        importer = MaterialReuseImporterUnderTest()

        self.assertFalse(importer._material_needs_mesh_decal_emission_refresh(material, submaterial))

    def test_mesh_decal_old_patch_version_forces_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_parallax_occlusion_mapping": True,
                },
            }
        )
        material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat:poms")
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree(
                "SB_MeshDecal_v1",
                starbreaker_mesh_decal_emission_patch_version=6,
            ),
            inputs=["Emission Strength", "Use Vert Col"],
        )
        group.inputs.get("Emission Strength").default_value = 0.0
        group.inputs.get("Use Vert Col").default_value = False
        material.node_tree.nodes.append(group)

        importer = MaterialReuseImporterUnderTest()

        self.assertTrue(importer._material_needs_mesh_decal_emission_refresh(material, submaterial))

    def test_mesh_decal_pom_with_unlinked_decal_source_forces_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    },
                    {
                        "slot": "TexSlot3",
                        "role": "normal_gloss",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat:poms")
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree(
                "SB_MeshDecal_v1",
                starbreaker_mesh_decal_emission_patch_version=7,
            ),
            inputs=["TexSlot1_DecalSource", "Emission Strength", "Use Vert Col"],
        )
        group.inputs.get("TexSlot1_DecalSource").default_value = (1.0, 1.0, 1.0, 1.0)
        group.inputs.get("Emission Strength").default_value = 0.0
        group.inputs.get("Use Vert Col").default_value = False
        material.node_tree.nodes.append(group)

        importer = MaterialReuseImporterUnderTest()

        self.assertTrue(importer._material_needs_mesh_decal_emission_refresh(material, submaterial))

    def test_mesh_decal_pom_missing_control_links_does_not_force_ship_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/textures/vehicles/manufacturer/orig/decals/orig_pom_decals_diff_TEX0.png",
                    },
                    {
                        "slot": "TexSlot3",
                        "role": "normal_gloss",
                        "export_path": "Data/textures/vehicles/manufacturer/orig/decals/orig_pom_decals_ddna_TEX0.png",
                    },
                    {
                        "slot": "TexSlot4",
                        "role": "height",
                        "export_path": "Data/textures/vehicles/manufacturer/orig/decals/orig_pom_decals_displ_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("orig_m80:poms")
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree(
                "SB_MeshDecal_v1",
                starbreaker_mesh_decal_emission_patch_version=7,
            ),
            inputs=[
                "TexSlot1_DecalSource",
                "TexSlot3_NormalGloss",
                "TexSlot4_Height",
                "Emission Strength",
                "Use Vert Col",
            ],
        )
        group.inputs.get("Emission Strength").default_value = 0.0
        group.inputs.get("Use Vert Col").default_value = False
        material.node_tree.nodes.append(group)

        importer = MaterialReuseImporterUnderTest(assembly_kind="ship")

        self.assertFalse(importer._material_needs_mesh_decal_emission_refresh(material, submaterial))

    def test_mesh_decal_pom_with_only_linked_decal_source_forces_relief_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat:poms")
        image = FakeNode("ShaderNodeTexImage", name="POM Diffuse", outputs=["Color"])
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree(
                "SB_MeshDecal_v1",
                starbreaker_mesh_decal_emission_patch_version=7,
            ),
            inputs=["TexSlot1_DecalSource", "Emission Strength", "Use Vert Col"],
        )
        group.inputs.get("Emission Strength").default_value = 0.0
        group.inputs.get("Use Vert Col").default_value = False
        material.node_tree.nodes.extend([image, group])
        material.node_tree.links.new(image.outputs.get("Color"), group.inputs.get("TexSlot1_DecalSource"))

        importer = MaterialReuseImporterUnderTest()

        self.assertTrue(importer._material_needs_mesh_decal_emission_refresh(material, submaterial))

    def test_mesh_decal_pom_missing_relief_links_forces_refresh(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {
                    "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                    "has_decal": False,
                    "has_stencil_map": False,
                    "has_parallax_occlusion_mapping": True,
                },
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_diff_TEX0.png",
                    },
                    {
                        "slot": "TexSlot3",
                        "role": "normal_gloss",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                    },
                    {
                        "slot": "TexSlot4",
                        "role": "height",
                        "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_height_TEX0.png",
                    },
                ],
            }
        )
        material = FakeNodeMaterial("bsnp_fps_behr_p6lr_mat:poms")
        image = FakeNode("ShaderNodeTexImage", name="POM Diffuse", outputs=["Color"])
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree(
                "SB_MeshDecal_v1",
                starbreaker_mesh_decal_emission_patch_version=7,
            ),
            inputs=[
                "TexSlot1_DecalSource",
                "TexSlot3_NormalGloss",
                "TexSlot4_Height",
                "Emission Strength",
                "Use Vert Col",
            ],
        )
        group.inputs.get("Emission Strength").default_value = 0.0
        group.inputs.get("Use Vert Col").default_value = False
        material.node_tree.nodes.extend([image, group])
        material.node_tree.links.new(image.outputs.get("Color"), group.inputs.get("TexSlot1_DecalSource"))

        importer = MaterialReuseImporterUnderTest()

        self.assertTrue(importer._material_needs_mesh_decal_emission_refresh(material, submaterial))

    def test_mesh_decal_host_clone_relief_check_uses_material_payload_under_package_mro(self) -> None:
        payload = {
            "shader_family": "MeshDecal",
            "decoded_feature_flags": {
                "tokens": ["DIFFUSE_MAP", "PARALLAX_OCCLUSION_MAPPING"],
                "has_decal": False,
                "has_stencil_map": False,
                "has_parallax_occlusion_mapping": True,
            },
            "texture_slots": [
                {
                    "slot": "TexSlot3",
                    "role": "normal_gloss",
                    "export_path": "Data/Objects/fps_weapons/weapons_v7/behr/textures/behr_pom_ddna_TEX0.png",
                },
            ],
        }
        material = FakeNodeMaterial(
            "bsnp_fps_behr_p6lr_mat:poms__host_49bc02bf",
            starbreaker_shader_family="MeshDecal",
            **{PROP_HAS_POM: True, PROP_SUBMATERIAL_JSON: json.dumps(payload)},
        )
        group = FakeNode(
            "ShaderNodeGroup",
            node_tree=FakeNodeGroupTree("SB_MeshDecal_v1"),
            inputs=["TexSlot3_NormalGloss"],
        )
        material.node_tree.nodes.append(group)

        self.assertFalse(
            PackageImporterMroUnderTest._mesh_decal_pom_required_texture_inputs_are_linked(material)
        )

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
        "Vulture fixtures not present; skipping material localization regression test",
    )
    def test_linked_reusable_material_is_copied_before_rebuild(self) -> None:
        sidecar = MaterialSidecar.from_file(VULTURE_ALT_A)
        submaterial = next(
            candidate
            for candidate in sidecar.submaterials
            if candidate.submaterial_name == "livery_decal"
        )

        bpy = sys.modules["bpy"]
        original_materials = getattr(bpy.data, "materials", None)
        materials = FakeMaterialsCollection()
        bpy.data.materials = materials
        try:
            sidecar_path = _canonical_material_sidecar_path("", sidecar)
            palette_scope = "test-scope"
            material_identity = _material_identity(sidecar_path, sidecar, submaterial, None, palette_scope)
            linked = FakeMaterial(
                submaterial.blender_material_name or "DRAK_Vulture:livery_decal",
                library=object(),
                **{
                    PROP_TEMPLATE_KEY: "wrong-template",
                    PROP_MATERIAL_IDENTITY: material_identity,
                    PROP_MATERIAL_SIDECAR: sidecar_path,
                    PROP_SUBMATERIAL_JSON: json.dumps(submaterial.raw, sort_keys=True),
                    PROP_PALETTE_SCOPE: palette_scope,
                },
            )
            materials[linked.name] = linked

            importer = MaterialReuseImporterUnderTest()
            material = importer.material_for_submaterial(sidecar_path, sidecar, submaterial, None)
            second = importer.material_for_submaterial(sidecar_path, sidecar, submaterial, None)

            self.assertIsNot(material, linked)
            self.assertIs(second, material)
            self.assertIsNone(material.library)
            self.assertEqual(material[PROP_MATERIAL_IDENTITY], material_identity)
            self.assertEqual(importer.rebuild_calls, [material.name])
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


class SceneAttachmentOffsetTests(unittest.TestCase):
    def test_duplicate_no_rotation_helper_offset_is_suppressed(self) -> None:
        location = _scene_attachment_offset_to_blender(
            (0.0, -1.2599999904632568, 3.371000051498413),
            (0.0, 0.0, 0.0),
            no_rotation=True,
            parent_world_matrix=(
                (1.0000001192092896, 4.76837158203125e-7, 1.7484583736404602e-7, 0.0002016690996242687),
                (-1.7484549630353285e-7, -7.085781135174329e-7, 0.9999999403953552, -1.2602757215499878),
                (4.768372150465439e-7, -1.0000001192092896, -9.088388424061122e-7, 3.3709583282470703),
                (0.0, 0.0, 0.0, 1.0),
            ),
        )

        self.assertEqual(location, (0.0, 0.0, 0.0))

    def test_nonzero_rotation_no_rotation_attachment_keeps_offset(self) -> None:
        location = _scene_attachment_offset_to_blender(
            (0.9120000004768372, -1.2000000476837158, 1.0),
            (0.0, 0.0, 45.0),
            no_rotation=True,
            parent_world_matrix=(
                (1.0, 0.0, 0.0, 0.9120000004768372),
                (0.0, 1.0, 0.0, -1.2000000476837158),
                (0.0, 0.0, 1.0, 1.0),
                (0.0, 0.0, 0.0, 1.0),
            ),
        )

        self.assertEqual(location, (0.9120000004768372, -1.0, -1.2000000476837158))


if __name__ == "__main__":
    unittest.main()
