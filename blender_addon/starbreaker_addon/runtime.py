from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
from typing import Any

import bpy
from mathutils import Euler, Matrix, Quaternion

from .manifest import LayerManifestEntry, MaterialSidecar, PackageBundle, PaletteRecord, SceneInstanceRecord, SubmaterialRecord, TextureReference
from .material_contract import ContractInput, ShaderGroupContract, TemplateContract, bundled_template_library_path, load_bundled_template_contract
from .palette import (
    palette_color,
    palette_decal_color,
    palette_decal_texture,
    palette_finish_glossiness,
    palette_finish_specular,
    palette_for_id,
    palette_id_for_livery_instance,
    palette_signature_for_submaterial,
    resolved_palette_id,
)
from .templates import (
    has_virtual_input,
    material_palette_channels,
    representative_textures,
    smoothness_texture_reference,
    template_plan_for_submaterial,
)

PROP_PACKAGE_ROOT = "starbreaker_package_root"
PROP_SCENE_PATH = "starbreaker_scene_path"
PROP_EXPORT_ROOT = "starbreaker_export_root"
PROP_PACKAGE_NAME = "starbreaker_package_name"
PROP_ENTITY_NAME = "starbreaker_entity_name"
PROP_INSTANCE_JSON = "starbreaker_instance_json"
PROP_MESH_ASSET = "starbreaker_mesh_asset"
PROP_MATERIAL_SIDECAR = "starbreaker_material_sidecar"
PROP_PALETTE_ID = "starbreaker_palette_id"
PROP_SHADER_FAMILY = "starbreaker_shader_family"
PROP_TEMPLATE_KEY = "starbreaker_template_key"
PROP_SUBMATERIAL_JSON = "starbreaker_submaterial_json"
PROP_MATERIAL_IDENTITY = "starbreaker_material_identity"
PROP_IMPORTED_SLOT_MAP = "starbreaker_imported_slot_map"
PROP_TEMPLATE_PATH = "starbreaker_template_path"
PROP_SOURCE_NODE_NAME = "starbreaker_source_node_name"
PROP_MISSING_ASSET = "starbreaker_missing_asset"
PROP_SURFACE_SHADER_MODE = "starbreaker_surface_shader_mode"
SCENE_WEAR_STRENGTH_PROP = "starbreaker_wear_strength"
SURFACE_SHADER_MODE_PRINCIPLED = "principled_first"
SURFACE_SHADER_MODE_GLASS = "glass_bsdf"

PACKAGE_ROOT_PREFIX = "StarBreaker"
TEMPLATE_COLLECTION_NAME = "StarBreaker Template Cache"
GLTF_PBR_WATTS_TO_LUMENS = 683.0
SCENE_AXIS_CONVERSION = Matrix(
    (
        (1.0, 0.0, 0.0, 0.0),
        (0.0, 0.0, -1.0, 0.0),
        (0.0, 1.0, 0.0, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    )
)
SCENE_AXIS_CONVERSION_INV = SCENE_AXIS_CONVERSION.inverted()
GLTF_LIGHT_BASIS_CORRECTION = Quaternion((math.sqrt(0.5), math.sqrt(0.5), 0.0, 0.0))
NON_COLOR_INPUT_KEYWORDS = ("normal", "roughness", "gloss", "mask", "height", "specular", "opacity", "id_map")
MATERIAL_IDENTITY_SCHEMA = "runtime_material_v6"


@dataclass(frozen=True)
class ImportedTemplate:
    mesh_asset: str
    root_names: list[str]


@dataclass(frozen=True)
class MaterialNodeLayout:
    texture_x: float = -300.0
    texture_start_y: float = 160.0
    texture_vertical_step: float = 260.0
    texture_width: float = 300.0
    primary_x: float = 200.0
    primary_y: float = -120.0
    group_width: float = 460.0
    output_x: float = 780.0
    output_y: float = -120.0
    shadow_mix_x: float = 500.0
    shadow_mix_y: float = -120.0
    shadow_transparent_x: float = 260.0
    shadow_transparent_y: float = -300.0
    shadow_light_path_x: float = 260.0
    shadow_light_path_y: float = -480.0


MATERIAL_NODE_LAYOUT = MaterialNodeLayout()


@dataclass
class LayerSurfaceSockets:
    color: Any | None = None
    alpha: Any | None = None
    normal: Any | None = None
    roughness: Any | None = None
    specular: Any | None = None
    specular_tint: Any | None = None

@dataclass(frozen=True)
class SocketRef:
    node: Any
    name: str
    is_output: bool = True


def import_package(
    context: bpy.types.Context,
    scene_path: str | Path,
    prefer_cycles: bool = True,
    palette_id: str | None = None,
) -> bpy.types.Object:
    package = PackageBundle.load(scene_path)
    importer = PackageImporter(context, package)
    return importer.import_scene(prefer_cycles=prefer_cycles, palette_id=palette_id)


def find_package_root(obj: bpy.types.Object | None) -> bpy.types.Object | None:
    current = obj
    while current is not None:
        if bool(current.get(PROP_PACKAGE_ROOT)):
            return current
        current = current.parent
    return None


def apply_palette_to_selected_package(context: bpy.types.Context, palette_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_palette_to_package_root(context, package_root, palette_id)


def apply_livery_to_selected_package(context: bpy.types.Context, livery_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_livery_to_package_root(context, package_root, livery_id)


def dump_selected_metadata(context: bpy.types.Context) -> list[str]:
    obj = context.active_object
    if obj is None:
        raise RuntimeError("Select an imported StarBreaker object first")

    text_names: list[str] = []
    instance_json = obj.get(PROP_INSTANCE_JSON)
    if isinstance(instance_json, str):
        text = bpy.data.texts.new(f"starbreaker_instance_{obj.name}.json")
        text.from_string(json.dumps(json.loads(instance_json), indent=2, sort_keys=True))
        text_names.append(text.name)

    material = obj.active_material
    if material is not None:
        submaterial_json = material.get(PROP_SUBMATERIAL_JSON)
        if isinstance(submaterial_json, str):
            text = bpy.data.texts.new(f"starbreaker_material_{material.name}.json")
            text.from_string(json.dumps(json.loads(submaterial_json), indent=2, sort_keys=True))
            text_names.append(text.name)

    return text_names


def apply_palette_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, palette_id: str) -> int:
    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root)
    with _suspend_heavy_viewports(context):
        return importer.apply_palette_to_package_root(package_root, palette_id)


def apply_livery_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, livery_id: str) -> int:
    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root)
    applied = 0
    with _suspend_heavy_viewports(context):
        for obj in _iter_package_objects(package_root):
            instance = _scene_instance_from_object(obj)
            if instance is None:
                continue
            effective_palette_id = palette_id_for_livery_instance(
                package,
                livery_id,
                instance,
                _string_prop(obj, PROP_MATERIAL_SIDECAR),
            )
            applied += importer.rebuild_object_materials(obj, effective_palette_id)
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
        root_palette_id = palette_id_for_livery_instance(
            package,
            livery_id,
            package.scene.root_entity,
            package.scene.root_entity.material_sidecar,
        )
        package_root[PROP_PALETTE_ID] = resolved_palette_id(
            package,
            root_palette_id,
            package.scene.root_entity.palette_id,
        ) or ""
    return applied


@contextmanager
def _suspend_heavy_viewports(context: bpy.types.Context):
    window_manager = getattr(context, "window_manager", None)
    if window_manager is None:
        yield
        return

    suspended: list[tuple[Any, str]] = []
    try:
        for window in window_manager.windows:
            screen = getattr(window, "screen", None)
            if screen is None:
                continue
            for area in screen.areas:
                if area.type != "VIEW_3D":
                    continue
                space = area.spaces.active
                shading = getattr(space, "shading", None)
                shading_type = getattr(shading, "type", None)
                if shading is None or shading_type not in {"RENDERED", "MATERIAL"}:
                    continue
                suspended.append((shading, shading_type))
                shading.type = "SOLID"
        yield
    finally:
        for shading, shading_type in suspended:
            try:
                shading.type = shading_type
            except Exception:
                continue


class PackageImporter:
    def __init__(
        self,
        context: bpy.types.Context,
        package: PackageBundle,
        package_root: bpy.types.Object | None = None,
    ) -> None:
        self.context = context
        self.package = package
        self.collection = self._ensure_collection(package.package_name)
        self.template_collection = self._ensure_template_collection()
        self.package_root = package_root
        self.template_cache: dict[str, ImportedTemplate] = {}
        self.material_cache: dict[str, bpy.types.Material] = {}
        self.node_index_by_entity_name: dict[str, dict[str, bpy.types.Object]] = {}
        self.bundled_template_contract: TemplateContract | None = None
        self.import_palette_override: str | None = None

    def _ensure_runtime_shared_groups(self) -> None:
        self._ensure_runtime_layer_surface_group()
        self._ensure_runtime_hard_surface_group()
        self._ensure_runtime_illum_group()

    def _effective_palette_id(self, palette_id: str | None) -> str | None:
        inherited_palette_id = None
        if self.package_root is not None:
            inherited_palette_id = _string_prop(self.package_root, PROP_PALETTE_ID)
        return resolved_palette_id(
            self.package,
            self.import_palette_override or palette_id,
            inherited_palette_id or self.package.scene.root_entity.palette_id,
        )

    def import_scene(self, prefer_cycles: bool = True, palette_id: str | None = None) -> bpy.types.Object:
        if prefer_cycles and hasattr(self.context.scene.render, "engine"):
            self.context.scene.render.engine = "CYCLES"
            self._ensure_cycles_denoising_support()

        self._ensure_runtime_shared_groups()

        initial_palette_id = resolved_palette_id(
            self.package,
            palette_id,
            self.package.scene.root_entity.palette_id,
        )
        self.import_palette_override = initial_palette_id
        package_root = self.package_root or self._create_package_root(initial_palette_id)
        self.package_root = package_root
        if initial_palette_id is not None:
            package_root[PROP_PALETTE_ID] = initial_palette_id

        root_anchor, root_nodes = self.instantiate_scene_instance(self.package.scene.root_entity, parent=package_root)
        self.node_index_by_entity_name[self.package.scene.root_entity.entity_name] = self._index_nodes(root_nodes)
        root_anchor.parent = package_root
        scene_root_parent = self._scene_root_parent(root_nodes) or package_root

        for child in self.package.scene.children:
            parent_node = None
            if child.parent_entity_name:
                parent_node = self.node_index_by_entity_name.get(child.parent_entity_name, {}).get(child.parent_node_name or "")
            anchor, child_nodes = self.instantiate_scene_instance(child, parent=scene_root_parent, parent_node=parent_node)
            self.node_index_by_entity_name.setdefault(child.entity_name, {}).update(self._index_nodes(child_nodes))

        for interior in self.package.scene.interiors:
            self.import_interior_container(interior, scene_root_parent)

        return package_root

    def rebuild_object_materials(self, obj: bpy.types.Object, palette_id: str | None) -> int:
        self._ensure_runtime_shared_groups()
        if obj.type != "MESH":
            return 0
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        if sidecar_path is None:
            return 0
        sidecar = self.package.load_material_sidecar(sidecar_path)
        if sidecar is None:
            return 0
        effective_palette_id = self._effective_palette_id(palette_id)
        palette = palette_for_id(self.package, effective_palette_id)
        applied = 0
        mesh_materials = getattr(obj.data, "materials", None)
        slot_mapping = _slot_mapping_for_object(obj)
        if slot_mapping is not None:
            if mesh_materials is not None:
                while len(mesh_materials) < len(slot_mapping):
                    mesh_materials.append(None)
            submaterials_by_index = {submaterial.index: submaterial for submaterial in sidecar.submaterials}
            for slot_index, mapped_index in enumerate(slot_mapping):
                submaterial = submaterials_by_index.get(mapped_index if mapped_index is not None else slot_index)
                if submaterial is None:
                    print(
                        f"StarBreaker: missing sidecar submaterial index {mapped_index} for {obj.name}"
                    )
                    continue
                if slot_index >= len(obj.material_slots):
                    print(
                        f"StarBreaker: slot index {slot_index} exceeds material slot count for {obj.name}"
                    )
                    continue
                material = self.material_for_submaterial(sidecar_path, sidecar, submaterial, palette)
                slot = obj.material_slots[slot_index]
                slot.link = "OBJECT"
                slot.material = material
                applied += 1
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
            return applied
        for submaterial in sorted(sidecar.submaterials, key=lambda item: item.index):
            if mesh_materials is not None:
                while len(mesh_materials) <= submaterial.index:
                    mesh_materials.append(None)
            if submaterial.index >= len(obj.material_slots):
                print(
                    f"StarBreaker: submaterial index {submaterial.index} exceeds material slot count for {obj.name}"
                )
                continue
            material = self.material_for_submaterial(sidecar_path, sidecar, submaterial, palette)
            slot = obj.material_slots[submaterial.index]
            slot.link = "OBJECT"
            slot.material = material
            applied += 1
        if effective_palette_id is not None:
            obj[PROP_PALETTE_ID] = effective_palette_id
        return applied

    def apply_palette_to_package_root(self, package_root: bpy.types.Object, palette_id: str | None) -> int:
        effective_palette_id = self._effective_palette_id(palette_id)
        palette = palette_for_id(self.package, effective_palette_id)
        if palette is None:
            return 0

        self._ensure_runtime_shared_groups()
        palette_group = self._ensure_palette_group(palette)
        touched_materials: set[int] = set()
        applied = 0
        for obj in _iter_package_objects(package_root):
            if obj.type != "MESH":
                continue
            if any(
                slot.material is not None and not _managed_material_runtime_graph_is_sane(slot.material)
                for slot in obj.material_slots
            ):
                applied += self.rebuild_object_materials(obj, effective_palette_id)
                if effective_palette_id is not None:
                    obj[PROP_PALETTE_ID] = effective_palette_id
                continue
            for slot in obj.material_slots:
                material = slot.material
                if material is None:
                    continue
                applied += 1
                material_pointer = material.as_pointer()
                if material_pointer not in touched_materials:
                    if _string_prop(material, PROP_PALETTE_ID) != palette.id:
                        self._apply_palette_to_material(material, palette, palette_group)
                    touched_materials.add(material_pointer)
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id

        if effective_palette_id is not None:
            package_root[PROP_PALETTE_ID] = effective_palette_id
        self.context.view_layer.update()
        return applied

    def _apply_palette_to_material(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
        palette_group: bpy.types.ShaderNodeTree,
    ) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeGroup":
                continue
            node_tree_name = getattr(getattr(node, "node_tree", None), "name", "")
            if node_tree_name.startswith("StarBreaker Palette "):
                node.node_tree = palette_group
                node.label = f"StarBreaker Palette {palette.id}"
                continue
            if node_tree_name == "StarBreaker Runtime LayerSurface":
                self._update_layer_surface_palette_defaults(node, palette)
            if node_tree_name == "StarBreaker Runtime HardSurface":
                self._update_runtime_hard_surface_palette_defaults(node, palette)

        self._update_virtual_tint_palette_decal_nodes(material, palette)
        material[PROP_PALETTE_ID] = palette.id

    def _update_layer_surface_palette_defaults(
        self,
        group_node: bpy.types.Node,
        palette: PaletteRecord,
    ) -> None:
        channel_name = group_node.get("starbreaker_palette_finish_channel")
        if not isinstance(channel_name, str) or channel_name not in {"primary", "secondary", "tertiary", "glass"}:
            palette_color_input = _input_socket(group_node, "Palette Color")
            if palette_color_input is None or not palette_color_input.is_linked:
                return

            source_socket_name = palette_color_input.links[0].from_socket.name
            channel_name = {
                "Primary": "primary",
                "Secondary": "secondary",
                "Tertiary": "tertiary",
                "Glass Color": "glass",
            }.get(source_socket_name)
        if channel_name is None:
            return

        self._set_socket_default(
            _input_socket(group_node, "Palette Glossiness"),
            palette_finish_glossiness(palette, channel_name) or 0.0,
        )
        self._set_socket_default(
            _input_socket(group_node, "Palette Specular"),
            _mean_triplet(palette_finish_specular(palette, channel_name)) or 0.0,
        )

    def _update_runtime_hard_surface_palette_defaults(
        self,
        group_node: bpy.types.Node,
        palette: PaletteRecord,
    ) -> None:
        palette_driven = bool(group_node.get("starbreaker_angle_shift_palette_driven", False))
        enabled = bool(group_node.get("starbreaker_angle_shift_enabled", False)) or (
            palette_driven and _palette_angle_shift_strength(palette) > 0.0
        )
        self._set_socket_default(
            _input_socket(group_node, "Iridescence Facing Color"),
            (*(_palette_finish_or_color(palette, "secondary") if enabled else (0.0, 0.0, 0.0)), 1.0),
        )
        self._set_socket_default(
            _input_socket(group_node, "Iridescence Grazing Color"),
            (*(_palette_finish_or_color(palette, "tertiary") if enabled else (0.0, 0.0, 0.0)), 1.0),
        )
        self._set_socket_default(
            _input_socket(group_node, "Iridescence Strength"),
            _palette_angle_shift_strength(palette) if enabled else 0.0,
        )

    def _update_virtual_tint_palette_decal_nodes(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
    ) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        color_node = next(
            (node for node in node_tree.nodes if node.bl_idname == "ShaderNodeRGB" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"),
            None,
        )
        alpha_node = next(
            (node for node in node_tree.nodes if node.bl_idname == "ShaderNodeValue" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"),
            None,
        )
        if color_node is None and alpha_node is None:
            return

        payload = material.get(PROP_SUBMATERIAL_JSON)
        if not isinstance(payload, str):
            return
        try:
            submaterial = SubmaterialRecord.from_value(json.loads(payload))
        except Exception:
            return

        color, alpha = self._virtual_tint_palette_decal_defaults(submaterial, palette)
        if color_node is not None:
            color_node.outputs[0].default_value = (*color, 1.0)
        if alpha_node is not None:
            alpha_node.outputs[0].default_value = alpha

    def instantiate_scene_instance(
        self,
        record: SceneInstanceRecord,
        parent: bpy.types.Object,
        parent_node: bpy.types.Object | None = None,
    ) -> tuple[bpy.types.Object, list[bpy.types.Object]]:
        effective_palette_id = self._effective_palette_id(record.palette_id)
        anchor = bpy.data.objects.new(record.entity_name, None)
        anchor.empty_display_type = "PLAIN_AXES"
        self.collection.objects.link(anchor)

        target_parent = parent_node or parent
        anchor.parent = target_parent
        anchor.rotation_mode = "QUATERNION"
        anchor.location = _scene_position_to_blender(record.offset_position)
        desired_rotation = Euler(tuple(math.radians(value) for value in record.offset_rotation), "XYZ").to_quaternion()
        if parent_node is not None and record.no_rotation:
            anchor.rotation_quaternion = parent_node.matrix_world.to_quaternion().inverted() @ desired_rotation
        else:
            anchor.rotation_quaternion = desired_rotation

        try:
            template = self.ensure_template(record.mesh_asset)
        except RuntimeError:
            anchor.empty_display_type = "SPHERE"
            if record.mesh_asset is not None:
                anchor[PROP_MISSING_ASSET] = record.mesh_asset
            self._apply_instance_metadata([anchor], record, effective_palette_id)
            return anchor, [anchor]

        clones = self.instantiate_template(template, anchor, neutralize_axis_root=parent_node is not None)
        self._apply_instance_metadata([anchor, *clones], record, effective_palette_id)

        for clone in clones:
            self.rebuild_object_materials(clone, effective_palette_id)
        return anchor, clones

    def import_interior_container(self, interior: Any, package_root: bpy.types.Object) -> bpy.types.Object:
        anchor_name = interior.name if interior.name.startswith("interior_") else f"interior_{interior.name}"
        anchor = bpy.data.objects.new(anchor_name, None)
        anchor.empty_display_type = "CUBE"
        anchor.parent = package_root
        anchor.matrix_local = _scene_matrix_to_blender(interior.container_transform)
        self.collection.objects.link(anchor)

        for placement in interior.placements:
            instance = SceneInstanceRecord(
                entity_name=placement.entity_class_guid or Path(placement.cgf_path or "interior").stem,
                geometry_path=placement.cgf_path,
                material_path=placement.material_path,
                material_sidecar=placement.material_sidecar,
                mesh_asset=placement.mesh_asset,
                palette_id=interior.palette_id,
                raw=placement.raw,
            )
            effective_palette_id = self._effective_palette_id(instance.palette_id)
            placement_anchor = bpy.data.objects.new(instance.entity_name, None)
            placement_anchor.parent = anchor
            placement_anchor.matrix_local = _scene_matrix_to_blender(placement.transform)
            self.collection.objects.link(placement_anchor)

            try:
                template = self.ensure_template(instance.mesh_asset)
            except RuntimeError:
                placement_anchor.empty_display_type = "SPHERE"
                if instance.mesh_asset is not None:
                    placement_anchor[PROP_MISSING_ASSET] = instance.mesh_asset
                self._apply_instance_metadata([placement_anchor], instance, effective_palette_id)
                continue

            clones = self.instantiate_template(
                template,
                placement_anchor,
                neutralize_axis_root=True,
                force_neutralize_axis_root=True,
            )
            self._apply_instance_metadata([placement_anchor, *clones], instance, effective_palette_id)
            for clone in clones:
                self.rebuild_object_materials(clone, effective_palette_id)

        for light in interior.lights:
            self.create_light(light, anchor)

        return anchor

    def create_light(self, light: Any, parent: bpy.types.Object) -> bpy.types.Object:
        blender_light_type = _blender_light_type(light)
        light_data = bpy.data.lights.new(name=light.name or "StarBreaker Light", type=blender_light_type)
        light_data.energy = _light_energy_to_blender(light.intensity, blender_light_type)
        light_data.color = light.color
        if blender_light_type != "SUN" and hasattr(light_data, "cutoff_distance"):
            light_data.cutoff_distance = light.radius
        if blender_light_type == "SPOT" and hasattr(light_data, "spot_size"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            light_data.spot_size = math.radians(outer_angle) * 2.0
        if blender_light_type == "SPOT" and hasattr(light_data, "spot_blend"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            inner_angle = min(light.inner_angle or 0.0, outer_angle)
            inner_ratio = min(max(inner_angle / outer_angle, 0.0), 1.0)
            light_data.spot_blend = 1.0 - inner_ratio

        light_object = bpy.data.objects.new(light.name or "StarBreaker Light", light_data)
        light_object.parent = parent
        light_object.location = _scene_position_to_blender(light.position)
        light_object.rotation_mode = "QUATERNION"
        light_object.rotation_quaternion = _scene_light_quaternion_to_blender(light.rotation)
        self.collection.objects.link(light_object)
        return light_object

    def ensure_template(self, mesh_asset: str | None) -> ImportedTemplate:
        if not mesh_asset:
            raise RuntimeError("Scene instance is missing mesh_asset")
        cached = self.template_cache.get(mesh_asset)
        if cached is not None:
            return cached

        asset_path = self.package.resolve_path(mesh_asset)
        if asset_path is None or not asset_path.is_file():
            raise RuntimeError(f"Missing mesh asset: {mesh_asset}")

        before = {obj.as_pointer() for obj in bpy.data.objects}
        before_materials = {material.as_pointer() for material in bpy.data.materials}
        result = bpy.ops.import_scene.gltf(filepath=str(asset_path), import_pack_images=False, merge_vertices=False)
        if "FINISHED" not in result:
            raise RuntimeError(f"Failed to import {asset_path}")

        imported = [obj for obj in bpy.data.objects if obj.as_pointer() not in before]
        imported_materials = [material for material in bpy.data.materials if material.as_pointer() not in before_materials]
        root_objects = self._root_objects(imported)
        for obj in imported:
            for collection in list(obj.users_collection):
                collection.objects.unlink(obj)
            self.template_collection.objects.link(obj)
            obj.hide_set(True)
            obj.hide_render = True
            obj[PROP_TEMPLATE_PATH] = mesh_asset
            obj[PROP_SOURCE_NODE_NAME] = _canonical_source_name(obj.name)

        self._clear_template_material_bindings(imported)
        self._purge_unused_materials(imported_materials)

        template = ImportedTemplate(mesh_asset=mesh_asset, root_names=[obj.name for obj in root_objects])
        self.template_cache[mesh_asset] = template
        return template

    def instantiate_template(
        self,
        template: ImportedTemplate,
        anchor: bpy.types.Object,
        neutralize_axis_root: bool = False,
        force_neutralize_axis_root: bool = False,
    ) -> list[bpy.types.Object]:
        clones: list[bpy.types.Object] = []
        mapping: dict[str, bpy.types.Object] = {}
        needs_view_layer_update = False
        for root_name in template.root_names:
            source = bpy.data.objects.get(root_name)
            if source is None:
                continue
            neutralize_root = neutralize_axis_root and (
                force_neutralize_axis_root or _should_neutralize_axis_root(source, template.mesh_asset)
            )
            clone = self._duplicate_object_tree(source, template.mesh_asset, mapping)
            clone.parent = anchor
            if neutralize_root:
                clone.matrix_local = Matrix.Identity(4)
                needs_view_layer_update = True
            clones.append(clone)
        if needs_view_layer_update:
            self.context.view_layer.update()
        return list(mapping.values()) or clones

    def _duplicate_object_tree(
        self,
        source: bpy.types.Object,
        mesh_asset: str,
        mapping: dict[str, bpy.types.Object],
    ) -> bpy.types.Object:
        clone = source.copy()
        if source.data is not None:
            clone.data = source.data
        clone.animation_data_clear()
        clone.hide_set(False)
        clone.hide_render = False
        clone[PROP_TEMPLATE_PATH] = mesh_asset
        clone[PROP_SOURCE_NODE_NAME] = source.get(PROP_SOURCE_NODE_NAME, source.name)
        self.collection.objects.link(clone)
        clone.matrix_basis = source.matrix_basis.copy()
        mapping[source.name] = clone

        for child in source.children:
            if child.get(PROP_TEMPLATE_PATH) != mesh_asset:
                continue
            child_clone = self._duplicate_object_tree(child, mesh_asset, mapping)
            child_clone.parent = clone
            child_clone.matrix_parent_inverse = child.matrix_parent_inverse.copy()
        return clone

    def material_for_submaterial(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
    ) -> bpy.types.Material:
        cache_key = _material_identity(sidecar_path, sidecar, submaterial, palette)
        cached = self.material_cache.get(cache_key)
        if cached is not None:
            return cached

        reusable = self._reusable_material(sidecar_path, sidecar, submaterial, palette, cache_key)
        if reusable is not None:
            existing_identity = reusable.get(PROP_MATERIAL_IDENTITY)
            if isinstance(existing_identity, str) and existing_identity == cache_key:
                self.material_cache[cache_key] = reusable
                return reusable
            self._build_managed_material(reusable, sidecar_path, sidecar, submaterial, palette, cache_key)
            self.material_cache[cache_key] = reusable
            return reusable

        material_name = _material_name(sidecar_path, sidecar, submaterial, cache_key)
        material = bpy.data.materials.new(material_name)
        self._build_managed_material(material, sidecar_path, sidecar, submaterial, palette, cache_key)
        self.material_cache[cache_key] = material
        return material

    def _reusable_material(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        material_identity: str,
    ) -> bpy.types.Material | None:
        preferred_name = submaterial.blender_material_name or _derived_material_name(sidecar_path, sidecar, submaterial)
        preferred = bpy.data.materials.get(preferred_name)
        if preferred is not None and _material_is_compatible(
            preferred,
            self.package,
            sidecar_path,
            sidecar,
            submaterial,
            palette,
        ):
            return preferred

        for material in bpy.data.materials:
            existing_identity = material.get(PROP_MATERIAL_IDENTITY)
            if (
                isinstance(existing_identity, str)
                and existing_identity == material_identity
                and _material_is_compatible(material, self.package, sidecar_path, sidecar, submaterial, palette)
            ):
                return material
        return None

    def _build_managed_material(
        self,
        material: bpy.types.Material,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        material_identity: str,
    ) -> None:
        palette_key = palette.id if palette is not None else "none"
        material.use_nodes = True
        plan = template_plan_for_submaterial(submaterial)
        surface_mode = SURFACE_SHADER_MODE_PRINCIPLED
        if submaterial.shader_family == "HardSurface":
            self._build_hard_surface_material(material, submaterial, palette, plan)
            if not _managed_material_runtime_graph_is_sane(material):
                try:
                    bpy.context.view_layer.update()
                except Exception:
                    pass
                self._build_hard_surface_material(material, submaterial, palette, plan)
        elif submaterial.shader_family == "Illum":
            self._build_illum_material(material, submaterial, palette, plan)
        else:
            group_contract = self._group_contract_for_submaterial(submaterial)
            if group_contract is not None and self._build_contract_group_material(material, submaterial, palette, plan, group_contract):
                if submaterial.shader_family == "GlassPBR":
                    surface_mode = SURFACE_SHADER_MODE_GLASS
            elif submaterial.shader_family == "GlassPBR":
                self._build_glass_material(material, submaterial, palette, plan)
                surface_mode = SURFACE_SHADER_MODE_GLASS
            elif plan.template_key == "nodraw":
                self._build_nodraw_material(material)
            elif plan.template_key == "screen_hud":
                self._build_screen_material(material, submaterial, palette, plan)
            elif plan.template_key == "effects":
                self._build_effect_material(material, submaterial, palette, plan)
            else:
                self._build_principled_material(material, submaterial, palette, plan)

        self._apply_material_node_layout(material)

        material[PROP_SHADER_FAMILY] = submaterial.shader_family
        material[PROP_TEMPLATE_KEY] = plan.template_key
        material[PROP_PALETTE_ID] = palette_key
        material[PROP_MATERIAL_SIDECAR] = _canonical_material_sidecar_path(sidecar_path, sidecar)
        material[PROP_MATERIAL_IDENTITY] = material_identity
        material[PROP_SUBMATERIAL_JSON] = json.dumps(submaterial.raw, sort_keys=True)
        material[PROP_SURFACE_SHADER_MODE] = surface_mode

    def _template_contract(self) -> TemplateContract:
        if self.bundled_template_contract is None:
            self.bundled_template_contract = load_bundled_template_contract()
        return self.bundled_template_contract

    def _group_contract_for_submaterial(self, submaterial: SubmaterialRecord) -> ShaderGroupContract | None:
        return self._template_contract().group_for_shader_family(submaterial.shader_family)

    def _ensure_contract_group(self, group_contract: ShaderGroupContract) -> bpy.types.ShaderNodeTree | None:
        group = bpy.data.node_groups.get(group_contract.name)
        if group is not None:
            return group
        library_path = bundled_template_library_path()
        if not library_path.is_file():
            return None
        with bpy.data.libraries.load(str(library_path), link=False) as (data_from, data_to):
            if group_contract.name not in data_from.node_groups:
                return None
            data_to.node_groups = [group_contract.name]
        return bpy.data.node_groups.get(group_contract.name)

    def _build_contract_group_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
        group_contract: ShaderGroupContract,
    ) -> bool:
        group_tree = self._ensure_contract_group(group_contract)
        if group_tree is None:
            return False

        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = group_tree
        group_node.location = (220, 0)

        shader_output = _output_socket(group_node, group_contract.shader_output)
        if shader_output is None:
            return False
        surface_shader = shader_output

        y = 280
        for contract_input in group_contract.inputs:
            target_socket = _input_socket(group_node, contract_input.name)
            if target_socket is None:
                continue
            semantic = (contract_input.semantic or contract_input.name).lower()
            if "disable" in semantic and "shadow" in semantic:
                if hasattr(target_socket, "default_value"):
                    target_socket.default_value = bool(self._plan_casts_no_shadows(plan, submaterial))
                source_socket = None
            elif semantic == "emission_strength" and hasattr(target_socket, "default_value"):
                target_socket.default_value = self._illum_emission_strength(submaterial)
                source_socket = None
            else:
                if (
                    group_contract.name == "SB_HardSurface_v1"
                    and semantic == "base_color"
                    and hasattr(target_socket, "default_value")
                ):
                    target_socket.default_value = (1.0, 1.0, 1.0, 1.0)
                elif (
                    group_contract.name == "SB_HardSurface_v1"
                    and semantic == "base_color_alpha"
                    and hasattr(target_socket, "default_value")
                ):
                    target_socket.default_value = 1.0
                elif ("alpha" in semantic or "opacity" in semantic) and hasattr(target_socket, "default_value"):
                    target_socket.default_value = 0.0
                source_socket = self._contract_input_source_socket(
                    nodes,
                    submaterial,
                    palette,
                    group_contract,
                    contract_input,
                    x=-220,
                    y=y,
                )
            if source_socket is not None:
                links.new(source_socket, target_socket)
            y -= 180

        group_handles_alpha = any(
            (contract_input.semantic or contract_input.name).lower() in {"alpha", "opacity"}
            or "alpha" in (contract_input.semantic or contract_input.name).lower()
            or "opacity" in (contract_input.semantic or contract_input.name).lower()
            for contract_input in group_contract.inputs
        )

        if plan.uses_alpha and not group_handles_alpha:
            alpha_source = self._alpha_source_socket(
                nodes,
                submaterial,
                representative_textures(submaterial),
                x=-220,
                y=y,
            )
            if alpha_source is not None:
                transparent = nodes.new("ShaderNodeBsdfTransparent")
                transparent.location = (400, -180)
                mix = nodes.new("ShaderNodeMixShader")
                mix.location = (560, 0)
                links.new(alpha_source, mix.inputs[0])
                links.new(transparent.outputs[0], mix.inputs[1])
                links.new(surface_shader, mix.inputs[2])
                surface_shader = mix.outputs[0]

        links.new(surface_shader, output.inputs[0])

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)
        return True

    def _build_hard_surface_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)

        top_base = _submaterial_texture_reference(submaterial, slots=("TexSlot1",), roles=("base_color", "diffuse"))
        top_base_node = self._image_node(nodes, top_base.export_path if top_base is not None else None, x=-720, y=520, is_color=True)
        top_base_color = top_base_node.outputs[0] if top_base_node is not None else None
        top_base_alpha = _output_socket(top_base_node, "Alpha") if top_base_node is not None else None
        palette_angle_shift_supported = _hard_surface_supports_palette_angle_shift(submaterial)
        angle_shift_enabled = _hard_surface_angle_shift_enabled(submaterial) or (
            palette_angle_shift_supported and _palette_angle_shift_strength(palette) > 0.0
        )

        primary_layer = submaterial.layer_manifest[0] if submaterial.layer_manifest else None
        secondary_layer = submaterial.layer_manifest[1] if len(submaterial.layer_manifest) > 1 else None
        primary = self._connect_manifest_layer_surface_group(
            nodes,
            links,
            submaterial,
            primary_layer,
            palette,
            x=-240,
            y=240,
            label="Primary Layer",
            detail_slots=("TexSlot7", "TexSlot13", "TexSlot6"),
        )
        secondary = self._connect_manifest_layer_surface_group(
            nodes,
            links,
            submaterial,
            secondary_layer,
            palette,
            x=-240,
            y=-120,
            label="Secondary Layer",
            detail_slots=("TexSlot7", "TexSlot13", "TexSlot6"),
        )
        wear_factor = self._layered_wear_factor_socket(nodes, links, submaterial, x=-720, y=-120)

        macro_normal_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot3",), roles=("normal_gloss",))
        macro_normal_node = self._image_node(
            nodes,
            macro_normal_ref.export_path if macro_normal_ref is not None else None,
            x=-720,
            y=-420,
            is_color=False,
        )
        displacement_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot6",), roles=("height", "displacement"))
        displacement_node = self._image_node(
            nodes,
            displacement_ref.export_path if displacement_ref is not None else None,
            x=-720,
            y=-720,
            is_color=False,
        )
        emissive_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot14",), roles=("emissive",))
        emissive_node = self._image_node(
            nodes,
            emissive_ref.export_path if emissive_ref is not None else None,
            x=-720,
            y=-1020,
            is_color=True,
        )

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_hard_surface_group()
        shader_group.location = (140, 0)
        shader_group.label = "StarBreaker HardSurface"
        self._set_socket_default(_input_socket(shader_group, "Top Base Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Top Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Primary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Roughness"), 0.45)
        self._set_socket_default(_input_socket(shader_group, "Primary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Specular Tint"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Primary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Roughness"), 0.45)
        self._set_socket_default(_input_socket(shader_group, "Secondary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Specular Tint"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Iridescence Facing Color"), (0.0, 0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Iridescence Grazing Color"), (0.0, 0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Iridescence Strength"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Wear Factor"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Macro Normal Color"), (0.5, 0.5, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Macro Normal Strength"), 0.4)
        self._set_socket_default(_input_socket(shader_group, "Displacement Strength"), 0.05)
        self._set_socket_default(_input_socket(shader_group, "Emission Color"), (0.0, 0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Emission Strength"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Disable Shadows"), self._plan_casts_no_shadows(plan))
        shader_group["starbreaker_angle_shift_enabled"] = angle_shift_enabled
        shader_group["starbreaker_angle_shift_palette_driven"] = palette_angle_shift_supported

        iridescence_facing_socket = None
        iridescence_grazing_socket = None
        if angle_shift_enabled and palette is not None:
            iridescence_facing_socket = self._palette_specular_socket(nodes, palette, "secondary", x=-260, y=620)
            iridescence_grazing_socket = self._palette_specular_socket(nodes, palette, "tertiary", x=-260, y=760)
        self._set_socket_default(
            _input_socket(shader_group, "Iridescence Facing Color"),
            (*(_palette_finish_or_color(palette, "secondary") if angle_shift_enabled else (0.0, 0.0, 0.0)), 1.0),
        )
        self._set_socket_default(
            _input_socket(shader_group, "Iridescence Grazing Color"),
            (*(_palette_finish_or_color(palette, "tertiary") if angle_shift_enabled else (0.0, 0.0, 0.0)), 1.0),
        )
        self._set_socket_default(
            _input_socket(shader_group, "Iridescence Strength"),
            _palette_angle_shift_strength(palette) if angle_shift_enabled else 0.0,
        )

        self._link_group_input(links, top_base_color, shader_group, "Top Base Color")
        self._link_group_input(links, top_base_alpha, shader_group, "Top Alpha")
        self._link_group_input(links, primary.color, shader_group, "Primary Color")
        self._link_group_input(links, primary.alpha, shader_group, "Primary Alpha")
        self._link_group_input(links, primary.roughness, shader_group, "Primary Roughness")
        self._link_group_input(links, primary.specular, shader_group, "Primary Specular")
        self._link_group_input(links, primary.specular_tint, shader_group, "Primary Specular Tint")
        self._link_group_input(links, primary.normal, shader_group, "Primary Normal")
        self._link_group_input(links, secondary.color, shader_group, "Secondary Color")
        self._link_group_input(links, secondary.alpha, shader_group, "Secondary Alpha")
        self._link_group_input(links, secondary.roughness, shader_group, "Secondary Roughness")
        self._link_group_input(links, secondary.specular, shader_group, "Secondary Specular")
        self._link_group_input(links, secondary.specular_tint, shader_group, "Secondary Specular Tint")
        self._link_group_input(links, secondary.normal, shader_group, "Secondary Normal")
        self._link_group_input(links, iridescence_facing_socket, shader_group, "Iridescence Facing Color")
        self._link_group_input(links, iridescence_grazing_socket, shader_group, "Iridescence Grazing Color")
        self._link_group_input(links, wear_factor, shader_group, "Wear Factor")
        self._link_group_input(
            links,
            macro_normal_node.outputs[0] if macro_normal_node is not None else None,
            shader_group,
            "Macro Normal Color",
        )
        self._link_group_input(
            links,
            displacement_node.outputs[0] if displacement_node is not None else None,
            shader_group,
            "Displacement Height",
        )
        self._link_group_input(
            links,
            emissive_node.outputs[0] if emissive_node is not None else None,
            shader_group,
            "Emission Color",
        )
        if emissive_node is not None:
            self._set_socket_default(_input_socket(shader_group, "Emission Strength"), 1.0)

        surface_shader = _output_socket(shader_group, "Shader")
        if surface_shader is not None:
            links.new(surface_shader, output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_illum_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)

        blend_mask_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot12",), roles=("wear_mask", "pattern_mask", "blend_mask"))
        blend_mask_node = self._image_node(
            nodes,
            blend_mask_ref.export_path if blend_mask_ref is not None else None,
            x=-720,
            y=160,
            is_color=False,
        )
        blend_mask_socket = blend_mask_node.outputs[0] if blend_mask_node is not None else None

        material_channel = submaterial.palette_routing.material_channel.name if submaterial.palette_routing.material_channel is not None else None

        primary_color_node = self._image_node(
            nodes,
            self._texture_export_path(submaterial, "base_color", "diffuse") or self._texture_path_for_slot(submaterial, "TexSlot1"),
            x=-720,
            y=520,
            is_color=True,
        )
        decal_palette = self._palette_decal_sockets(
            nodes,
            links,
            palette,
            material_channel,
            x=-420,
            y=520,
        )
        primary_normal_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot2",), roles=("normal_gloss",))
        primary_normal_node = self._image_node(
            nodes,
            primary_normal_ref.export_path if primary_normal_ref is not None else None,
            x=-720,
            y=-140,
            is_color=False,
        )
        primary_detail = self._detail_texture_channels(nodes, self._texture_path_for_slot(submaterial, "TexSlot6"), x=-720, y=-420)
        primary_roughness, primary_roughness_is_smoothness = self._roughness_socket_for_texture_reference(nodes, primary_normal_ref, x=-460, y=-140)
        primary_specular = self._specular_socket_for_texture_path(nodes, self._texture_path_for_slot(submaterial, "TexSlot4"), x=-720, y=760)
        primary = self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=decal_palette.color if decal_palette.color is not None else (primary_color_node.outputs[0] if primary_color_node is not None else None),
            base_alpha_socket=decal_palette.alpha if decal_palette.alpha is not None else (_output_socket(primary_color_node, "Alpha") if primary_color_node is not None else None),
            normal_color_socket=primary_normal_node.outputs[0] if primary_normal_node is not None else None,
            roughness_socket=primary_roughness,
            roughness_source_is_smoothness=primary_roughness_is_smoothness,
            detail_channels=primary_detail,
            detail_diffuse_strength=0.35,
            detail_gloss_strength=0.35,
            detail_bump_strength=0.15,
            tint_color=None,
            palette=palette,
            palette_channel_name=material_channel,
            palette_finish_channel_name=material_channel,
            palette_glossiness=palette_finish_glossiness(palette, material_channel),
            specular_value=0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, material_channel)) or 0.0,
            x=-180,
            y=220,
            label="Primary Layer",
        )

        secondary_color_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot9",), roles=("alternate_base_color", "base_color", "diffuse"))
        secondary_color_node = self._image_node(
            nodes,
            secondary_color_ref.export_path if secondary_color_ref is not None else None,
            x=-720,
            y=20,
            is_color=True,
        )
        secondary_normal_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot3",), roles=("normal_gloss",))
        secondary_normal_node = self._image_node(
            nodes,
            secondary_normal_ref.export_path if secondary_normal_ref is not None else None,
            x=-720,
            y=-700,
            is_color=False,
        )
        secondary_detail = self._detail_texture_channels(nodes, self._texture_path_for_slot(submaterial, "TexSlot13"), x=-720, y=-980)
        secondary_roughness, secondary_roughness_is_smoothness = self._roughness_socket_for_texture_reference(nodes, secondary_normal_ref, x=-460, y=-700)
        secondary_specular = self._specular_socket_for_texture_path(nodes, self._texture_path_for_slot(submaterial, "TexSlot10"), x=-720, y=980)
        secondary = self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=secondary_color_node.outputs[0] if secondary_color_node is not None else None,
            base_alpha_socket=_output_socket(secondary_color_node, "Alpha") if secondary_color_node is not None else None,
            normal_color_socket=secondary_normal_node.outputs[0] if secondary_normal_node is not None else None,
            roughness_socket=secondary_roughness,
            roughness_source_is_smoothness=secondary_roughness_is_smoothness,
            detail_channels=secondary_detail,
            detail_diffuse_strength=0.35,
            detail_gloss_strength=0.35,
            detail_bump_strength=0.15,
            tint_color=None,
            palette=palette,
            palette_channel_name=material_channel,
            palette_finish_channel_name=material_channel,
            palette_glossiness=palette_finish_glossiness(palette, material_channel),
            specular_value=0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, material_channel)) or 0.0,
            x=-180,
            y=-140,
            label="Secondary Layer",
        )

        height_primary = self._mask_socket(nodes, self._texture_path_for_slot(submaterial, "TexSlot8"), x=-720, y=-1240)
        height_secondary = self._mask_socket(nodes, self._texture_path_for_slot(submaterial, "TexSlot11"), x=-720, y=-1400)

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_illum_group()
        shader_group.location = (140, 0)
        shader_group.label = "StarBreaker Illum"
        self._set_socket_default(_input_socket(shader_group, "Primary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Primary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Roughness"), 0.35)
        self._set_socket_default(_input_socket(shader_group, "Primary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Roughness"), 0.35)
        self._set_socket_default(_input_socket(shader_group, "Secondary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Blend Mask"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "POM Strength"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Emission Strength"), self._illum_emission_strength(submaterial))
        self._set_socket_default(_input_socket(shader_group, "Disable Shadows"), self._plan_casts_no_shadows(plan))

        self._link_group_input(links, primary.color, shader_group, "Primary Color")
        self._link_group_input(links, primary.alpha, shader_group, "Primary Alpha")
        self._link_group_input(links, primary.roughness, shader_group, "Primary Roughness")
        self._link_group_input(links, primary.specular, shader_group, "Primary Specular")
        self._link_group_input(links, primary.normal, shader_group, "Primary Normal")
        self._link_group_input(links, secondary.color, shader_group, "Secondary Color")
        self._link_group_input(links, secondary.alpha, shader_group, "Secondary Alpha")
        self._link_group_input(links, secondary.roughness, shader_group, "Secondary Roughness")
        self._link_group_input(links, secondary.specular, shader_group, "Secondary Specular")
        self._link_group_input(links, secondary.normal, shader_group, "Secondary Normal")
        self._link_group_input(links, blend_mask_socket, shader_group, "Blend Mask")
        if plan.template_key == "parallax_pom":
            self._link_group_input(links, height_primary, shader_group, "Primary Height")
            self._link_group_input(links, height_secondary, shader_group, "Secondary Height")
            self._set_socket_default(
                _input_socket(shader_group, "POM Strength"),
                max(0.03, min(0.2, _float_public_param(submaterial, "PomDisplacement", "HeightBias") or 0.08)),
            )

        surface_shader = _output_socket(shader_group, "Shader")
        if surface_shader is not None:
            links.new(surface_shader, output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _ensure_cycles_denoising_support(self) -> None:
        cycles = getattr(self.context.scene, "cycles", None)
        view_layer = getattr(self.context, "view_layer", None)
        view_layer_cycles = getattr(view_layer, "cycles", None) if view_layer is not None else None
        if cycles is None or view_layer_cycles is None:
            return
        if not getattr(cycles, "use_denoising", False):
            return
        if getattr(cycles, "denoiser", None) != "OPENIMAGEDENOISE":
            return
        if getattr(cycles, "denoising_input_passes", "RGB") == "RGB":
            return
        if hasattr(view_layer_cycles, "denoising_store_passes"):
            view_layer_cycles.denoising_store_passes = True

    def _begin_runtime_shared_group(
        self,
        group_name: str,
        *,
        signature: str,
        inputs: list[tuple[str, str]],
        outputs: list[tuple[str, str]],
    ) -> tuple[bpy.types.ShaderNodeTree, bpy.types.Node, bpy.types.Node]:
        group_tree = bpy.data.node_groups.get(group_name)
        if group_tree is None:
            group_tree = bpy.data.node_groups.new(group_name, "ShaderNodeTree")
        existing_signature = group_tree.get("starbreaker_runtime_signature")
        built_signature = group_tree.get("starbreaker_runtime_built_signature")
        group_input = next((node for node in group_tree.nodes if node.bl_idname == "NodeGroupInput"), None)
        group_output = next((node for node in group_tree.nodes if node.bl_idname == "NodeGroupOutput"), None)
        if (
            existing_signature == signature
            and built_signature == signature
            and group_input is not None
            and group_output is not None
        ):
            return group_tree, group_input, group_output
        group_tree.use_fake_user = False
        group_tree.nodes.clear()
        for item in list(group_tree.interface.items_tree):
            group_tree.interface.remove(item)
        for socket_name, socket_type in inputs:
            group_tree.interface.new_socket(name=socket_name, in_out="INPUT", socket_type=socket_type)
        for socket_name, socket_type in outputs:
            group_tree.interface.new_socket(name=socket_name, in_out="OUTPUT", socket_type=socket_type)

        group_input = group_tree.nodes.new("NodeGroupInput")
        group_input.location = (-980, 0)
        group_output = group_tree.nodes.new("NodeGroupOutput")
        group_output.location = (980, 0)
        group_tree["starbreaker_runtime_signature"] = signature
        group_tree["starbreaker_runtime_built_signature"] = ""
        return group_tree, group_input, group_output

    def _invalidate_runtime_group_if_unexpected(
        self,
        group_name: str,
        signature: str,
        expected_node_counts: dict[str, int],
    ) -> None:
        group_tree = bpy.data.node_groups.get(group_name)
        if group_tree is None or group_tree.get("starbreaker_runtime_signature") != signature:
            return
        for bl_idname, expected_count in expected_node_counts.items():
            actual_count = sum(1 for node in group_tree.nodes if node.bl_idname == bl_idname)
            if actual_count != expected_count:
                group_tree["starbreaker_runtime_built_signature"] = ""
                return

    def _ensure_runtime_layer_surface_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime LayerSurface",
            "layer_surface_v2",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeNormalMap": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime LayerSurface",
            signature="layer_surface_v2",
            inputs=[
                ("Base Color", "NodeSocketColor"),
                ("Base Alpha", "NodeSocketFloat"),
                ("Palette Color", "NodeSocketColor"),
                ("Tint Color", "NodeSocketColor"),
                ("Detail Color Mask", "NodeSocketFloat"),
                ("Detail Height Mask", "NodeSocketFloat"),
                ("Detail Gloss Mask", "NodeSocketFloat"),
                ("Detail Diffuse Strength", "NodeSocketFloat"),
                ("Detail Gloss Strength", "NodeSocketFloat"),
                ("Detail Bump Strength", "NodeSocketFloat"),
                ("Normal Color", "NodeSocketColor"),
                ("Roughness Source", "NodeSocketFloat"),
                ("Roughness Source Is Smoothness", "NodeSocketBool"),
                ("Palette Glossiness", "NodeSocketFloat"),
                ("Specular Value", "NodeSocketFloat"),
                ("Palette Specular", "NodeSocketFloat"),
            ],
            outputs=[
                ("Color", "NodeSocketColor"),
                ("Alpha", "NodeSocketFloat"),
                ("Roughness", "NodeSocketFloat"),
                ("Specular", "NodeSocketFloat"),
                ("Normal", "NodeSocketVector"),
            ],
        )
        nodes = group_tree.nodes
        links = group_tree.links

        tint = nodes.new("ShaderNodeMixRGB")
        tint.location = (-720, 280)
        tint.blend_type = "MULTIPLY"
        tint.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Base Color"), tint.inputs[1])
        links.new(_output_socket(group_input, "Tint Color"), tint.inputs[2])

        palette_mix = nodes.new("ShaderNodeMixRGB")
        palette_mix.location = (-520, 280)
        palette_mix.blend_type = "MULTIPLY"
        palette_mix.inputs[0].default_value = 1.0
        links.new(tint.outputs[0], palette_mix.inputs[1])
        links.new(_output_socket(group_input, "Palette Color"), palette_mix.inputs[2])

        detail_gray = nodes.new("ShaderNodeValToRGB")
        detail_gray.location = (-720, 40)
        links.new(_output_socket(group_input, "Detail Color Mask"), detail_gray.inputs[0])

        white = nodes.new("ShaderNodeRGB")
        white.location = (-720, -120)
        white.outputs[0].default_value = (1.0, 1.0, 1.0, 1.0)

        detail_mix = nodes.new("ShaderNodeMixRGB")
        detail_mix.location = (-520, 40)
        detail_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Detail Diffuse Strength"), detail_mix.inputs[0])
        links.new(white.outputs[0], detail_mix.inputs[1])
        links.new(detail_gray.outputs[0], detail_mix.inputs[2])

        final_color = nodes.new("ShaderNodeMixRGB")
        final_color.location = (-320, 220)
        final_color.blend_type = "MULTIPLY"
        final_color.inputs[0].default_value = 1.0
        links.new(palette_mix.outputs[0], final_color.inputs[1])
        links.new(detail_mix.outputs[0], final_color.inputs[2])

        roughness_invert = nodes.new("ShaderNodeMath")
        roughness_invert.location = (-720, -300)
        roughness_invert.operation = "SUBTRACT"
        roughness_invert.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Roughness Source"), roughness_invert.inputs[1])

        roughness_source = nodes.new("ShaderNodeMix")
        roughness_source.location = (-520, -300)
        if hasattr(roughness_source, "data_type"):
            roughness_source.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Roughness Source Is Smoothness"), roughness_source.inputs[0])
        links.new(_output_socket(group_input, "Roughness Source"), roughness_source.inputs[2])
        links.new(roughness_invert.outputs[0], roughness_source.inputs[3])

        palette_gloss_factor = nodes.new("ShaderNodeMath")
        palette_gloss_factor.location = (-720, -180)
        palette_gloss_factor.operation = "SUBTRACT"
        palette_gloss_factor.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Palette Glossiness"), palette_gloss_factor.inputs[1])

        roughness_base = nodes.new("ShaderNodeMath")
        roughness_base.location = (-320, -240)
        roughness_base.operation = "MULTIPLY"
        links.new(roughness_source.outputs[0], roughness_base.inputs[0])
        links.new(palette_gloss_factor.outputs[0], roughness_base.inputs[1])

        detail_gloss = nodes.new("ShaderNodeMix")
        detail_gloss.location = (-120, -240)
        if hasattr(detail_gloss, "data_type"):
            detail_gloss.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Detail Gloss Strength"), detail_gloss.inputs[0])
        detail_gloss.inputs[2].default_value = 1.0
        links.new(_output_socket(group_input, "Detail Gloss Mask"), detail_gloss.inputs[3])

        roughness = nodes.new("ShaderNodeMath")
        roughness.location = (80, -240)
        roughness.operation = "MULTIPLY"
        links.new(roughness_base.outputs[0], roughness.inputs[0])
        links.new(detail_gloss.outputs[0], roughness.inputs[1])

        specular = nodes.new("ShaderNodeMath")
        specular.location = (-320, -420)
        specular.operation = "ADD"
        specular.use_clamp = True
        links.new(_output_socket(group_input, "Specular Value"), specular.inputs[0])
        links.new(_output_socket(group_input, "Palette Specular"), specular.inputs[1])

        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.location = (-520, -620)
        links.new(_output_socket(group_input, "Normal Color"), _input_socket(normal_map, "Color"))

        bump = nodes.new("ShaderNodeBump")
        bump.location = (-320, -620)
        links.new(_output_socket(group_input, "Detail Bump Strength"), bump.inputs[0])
        links.new(_output_socket(group_input, "Detail Height Mask"), bump.inputs[2])
        links.new(_output_socket(normal_map, "Normal"), bump.inputs[3])

        links.new(final_color.outputs[0], group_output.inputs["Color"])
        links.new(_output_socket(group_input, "Base Alpha"), group_output.inputs["Alpha"])
        links.new(roughness.outputs[0], group_output.inputs["Roughness"])
        links.new(specular.outputs[0], group_output.inputs["Specular"])
        links.new(bump.outputs[0], group_output.inputs["Normal"])
        group_tree["starbreaker_runtime_built_signature"] = "layer_surface_v2"
        return group_tree

    def _ensure_runtime_hard_surface_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime HardSurface",
            "hard_surface_v6",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeMixShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime HardSurface",
            signature="hard_surface_v6",
            inputs=[
                ("Top Base Color", "NodeSocketColor"),
                ("Top Alpha", "NodeSocketFloat"),
                ("Primary Color", "NodeSocketColor"),
                ("Primary Alpha", "NodeSocketFloat"),
                ("Primary Roughness", "NodeSocketFloat"),
                ("Primary Specular", "NodeSocketFloat"),
                ("Primary Specular Tint", "NodeSocketColor"),
                ("Primary Normal", "NodeSocketVector"),
                ("Secondary Color", "NodeSocketColor"),
                ("Secondary Alpha", "NodeSocketFloat"),
                ("Secondary Roughness", "NodeSocketFloat"),
                ("Secondary Specular", "NodeSocketFloat"),
                ("Secondary Specular Tint", "NodeSocketColor"),
                ("Secondary Normal", "NodeSocketVector"),
                ("Iridescence Facing Color", "NodeSocketColor"),
                ("Iridescence Grazing Color", "NodeSocketColor"),
                ("Iridescence Strength", "NodeSocketFloat"),
                ("Wear Factor", "NodeSocketFloat"),
                ("Macro Normal Color", "NodeSocketColor"),
                ("Macro Normal Strength", "NodeSocketFloat"),
                ("Displacement Height", "NodeSocketFloat"),
                ("Displacement Strength", "NodeSocketFloat"),
                ("Emission Color", "NodeSocketColor"),
                ("Emission Strength", "NodeSocketFloat"),
                ("Disable Shadows", "NodeSocketBool"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        nodes = group_tree.nodes
        links = group_tree.links

        color_mix = nodes.new("ShaderNodeMixRGB")
        color_mix.location = (-700, 260)
        color_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Wear Factor"), color_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Color"), color_mix.inputs[1])
        links.new(_output_socket(group_input, "Secondary Color"), color_mix.inputs[2])

        final_color = nodes.new("ShaderNodeMixRGB")
        final_color.location = (-500, 260)
        final_color.blend_type = "MULTIPLY"
        final_color.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Top Base Color"), final_color.inputs[1])
        links.new(color_mix.outputs[0], final_color.inputs[2])

        layer_weight = nodes.new("ShaderNodeLayerWeight")
        layer_weight.location = (-720, 520)
        blend_input = _input_socket(layer_weight, "Blend")
        if blend_input is not None:
            blend_input.default_value = 0.3

        angle_factor = nodes.new("ShaderNodeMapRange")
        angle_factor.location = (-580, 620)
        angle_factor.clamp = True
        angle_factor.inputs[1].default_value = 0.2
        angle_factor.inputs[2].default_value = 0.85
        angle_factor.inputs[3].default_value = 1.0
        angle_factor.inputs[4].default_value = 0.0
        links.new(_output_socket(layer_weight, "Facing"), angle_factor.inputs[0])

        iridescence_color = nodes.new("ShaderNodeMixRGB")
        iridescence_color.location = (-500, 520)
        iridescence_color.blend_type = "MIX"
        links.new(_output_socket(group_input, "Iridescence Facing Color"), iridescence_color.inputs[1])
        links.new(_output_socket(group_input, "Iridescence Grazing Color"), iridescence_color.inputs[2])
        links.new(angle_factor.outputs[0], iridescence_color.inputs[0])

        iridescence_strength = nodes.new("ShaderNodeMath")
        iridescence_strength.location = (-300, 700)
        iridescence_strength.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Iridescence Strength"), iridescence_strength.inputs[0])
        links.new(angle_factor.outputs[0], iridescence_strength.inputs[1])

        color_sheen = nodes.new("ShaderNodeMixRGB")
        color_sheen.location = (-260, 460)
        color_sheen.blend_type = "SCREEN"
        links.new(iridescence_strength.outputs[0], color_sheen.inputs[0])
        links.new(final_color.outputs[0], color_sheen.inputs[1])
        links.new(iridescence_color.outputs[0], color_sheen.inputs[2])

        alpha_mix = nodes.new("ShaderNodeMix")
        alpha_mix.location = (-700, 80)
        if hasattr(alpha_mix, "data_type"):
            alpha_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Wear Factor"), alpha_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Alpha"), alpha_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Alpha"), alpha_mix.inputs[3])

        alpha_mul = nodes.new("ShaderNodeMath")
        alpha_mul.location = (-500, 80)
        alpha_mul.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Top Alpha"), alpha_mul.inputs[0])
        links.new(alpha_mix.outputs[0], alpha_mul.inputs[1])

        roughness_mix = nodes.new("ShaderNodeMix")
        roughness_mix.location = (-700, -100)
        if hasattr(roughness_mix, "data_type"):
            roughness_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Wear Factor"), roughness_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Roughness"), roughness_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Roughness"), roughness_mix.inputs[3])

        specular_mix = nodes.new("ShaderNodeMix")
        specular_mix.location = (-700, -280)
        if hasattr(specular_mix, "data_type"):
            specular_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Wear Factor"), specular_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Specular"), specular_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Specular"), specular_mix.inputs[3])

        specular_tint_mix = nodes.new("ShaderNodeMixRGB")
        specular_tint_mix.location = (-700, -420)
        specular_tint_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Wear Factor"), specular_tint_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Specular Tint"), specular_tint_mix.inputs[1])
        links.new(_output_socket(group_input, "Secondary Specular Tint"), specular_tint_mix.inputs[2])

        normal_mix = nodes.new("ShaderNodeMix")
        normal_mix.location = (-700, -500)
        if hasattr(normal_mix, "data_type"):
            normal_mix.data_type = "VECTOR"
        links.new(_output_socket(group_input, "Wear Factor"), normal_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Normal"), normal_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Normal"), normal_mix.inputs[3])

        macro_normal = nodes.new("ShaderNodeNormalMap")
        macro_normal.location = (-500, -680)
        strength_input = _input_socket(macro_normal, "Strength")
        if strength_input is not None:
            links.new(_output_socket(group_input, "Macro Normal Strength"), strength_input)
        links.new(_output_socket(group_input, "Macro Normal Color"), _input_socket(macro_normal, "Color"))

        normal_add = nodes.new("ShaderNodeVectorMath")
        normal_add.location = (-300, -520)
        normal_add.operation = "ADD"
        links.new(normal_mix.outputs[0], normal_add.inputs[0])
        links.new(_output_socket(macro_normal, "Normal"), normal_add.inputs[1])

        normal_normalize = nodes.new("ShaderNodeVectorMath")
        normal_normalize.location = (-100, -520)
        normal_normalize.operation = "NORMALIZE"
        links.new(normal_add.outputs[0], normal_normalize.inputs[0])

        bump = nodes.new("ShaderNodeBump")
        bump.location = (100, -520)
        links.new(_output_socket(group_input, "Displacement Strength"), bump.inputs[0])
        links.new(_output_socket(group_input, "Displacement Height"), bump.inputs[2])
        links.new(normal_normalize.outputs[0], bump.inputs[3])

        principled = self._create_surface_bsdf(nodes)
        principled.location = (320, 40)
        links.new(color_sheen.outputs[0], _input_socket(principled, "Base Color"))
        links.new(alpha_mul.outputs[0], _input_socket(principled, "Alpha"))
        links.new(roughness_mix.outputs[0], _input_socket(principled, "Roughness"))
        metallic_mix = nodes.new("ShaderNodeMapRange")
        metallic_mix.location = (80, -180)
        metallic_mix.clamp = True
        metallic_mix.inputs[1].default_value = 0.0
        metallic_mix.inputs[2].default_value = 1.0
        metallic_mix.inputs[3].default_value = 0.08
        metallic_mix.inputs[4].default_value = 0.42
        links.new(iridescence_strength.outputs[0], metallic_mix.inputs[0])
        metallic_input = _input_socket(principled, "Metallic")
        if metallic_input is not None:
            links.new(metallic_mix.outputs[0], metallic_input)
        specular_input = _input_socket(principled, "Specular IOR Level", "Specular")
        if specular_input is not None:
            links.new(specular_mix.outputs[0], specular_input)
        specular_tint_input = _input_socket(principled, "Specular Tint")
        if specular_tint_input is not None:
            links.new(specular_tint_mix.outputs[0], specular_tint_input)
        coat_tint_input = _input_socket(principled, "Coat Tint")
        if coat_tint_input is not None:
            links.new(iridescence_color.outputs[0], coat_tint_input)
        coat_weight_input = _input_socket(principled, "Coat Weight")
        if coat_weight_input is not None:
            links.new(iridescence_strength.outputs[0], coat_weight_input)
        coat_roughness_input = _input_socket(principled, "Coat Roughness")
        if coat_roughness_input is not None:
            coat_roughness_input.default_value = 0.08
        normal_input = _input_socket(principled, "Normal")
        if normal_input is not None:
            links.new(bump.outputs[0], normal_input)
        emission_color = _input_socket(principled, "Emission Color", "Emission")
        emission_strength = _input_socket(principled, "Emission Strength")
        if emission_color is not None:
            links.new(_output_socket(group_input, "Emission Color"), emission_color)
        if emission_strength is not None:
            links.new(_output_socket(group_input, "Emission Strength"), emission_strength)

        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (520, -200)
        shadow_toggle = nodes.new("ShaderNodeMath")
        shadow_toggle.location = (700, -200)
        shadow_toggle.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Disable Shadows"), shadow_toggle.inputs[0])
        links.new(_output_socket(light_path, "Is Shadow Ray"), shadow_toggle.inputs[1])
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (700, -380)
        shadow_mix = nodes.new("ShaderNodeMixShader")
        shadow_mix.location = (900, -40)
        links.new(shadow_toggle.outputs[0], shadow_mix.inputs[0])
        links.new(principled.outputs[0], shadow_mix.inputs[1])
        links.new(transparent.outputs[0], shadow_mix.inputs[2])
        links.new(shadow_mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "hard_surface_v6"
        return group_tree

    def _ensure_runtime_illum_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Illum",
            "illum_v2",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeEmission": 1,
                "ShaderNodeAddShader": 1,
                "ShaderNodeMixShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Illum",
            signature="illum_v2",
            inputs=[
                ("Primary Color", "NodeSocketColor"),
                ("Primary Alpha", "NodeSocketFloat"),
                ("Primary Roughness", "NodeSocketFloat"),
                ("Primary Specular", "NodeSocketFloat"),
                ("Primary Normal", "NodeSocketVector"),
                ("Secondary Color", "NodeSocketColor"),
                ("Secondary Alpha", "NodeSocketFloat"),
                ("Secondary Roughness", "NodeSocketFloat"),
                ("Secondary Specular", "NodeSocketFloat"),
                ("Secondary Normal", "NodeSocketVector"),
                ("Blend Mask", "NodeSocketFloat"),
                ("Primary Height", "NodeSocketFloat"),
                ("Secondary Height", "NodeSocketFloat"),
                ("POM Strength", "NodeSocketFloat"),
                ("Emission Strength", "NodeSocketFloat"),
                ("Disable Shadows", "NodeSocketBool"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        nodes = group_tree.nodes
        links = group_tree.links

        color_mix = nodes.new("ShaderNodeMixRGB")
        color_mix.location = (-700, 260)
        color_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Blend Mask"), color_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Color"), color_mix.inputs[1])
        links.new(_output_socket(group_input, "Secondary Color"), color_mix.inputs[2])

        alpha_mix = nodes.new("ShaderNodeMix")
        alpha_mix.location = (-700, 80)
        if hasattr(alpha_mix, "data_type"):
            alpha_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), alpha_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Alpha"), alpha_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Alpha"), alpha_mix.inputs[3])

        roughness_mix = nodes.new("ShaderNodeMix")
        roughness_mix.location = (-700, -100)
        if hasattr(roughness_mix, "data_type"):
            roughness_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), roughness_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Roughness"), roughness_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Roughness"), roughness_mix.inputs[3])

        specular_mix = nodes.new("ShaderNodeMix")
        specular_mix.location = (-700, -280)
        if hasattr(specular_mix, "data_type"):
            specular_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), specular_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Specular"), specular_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Specular"), specular_mix.inputs[3])

        normal_mix = nodes.new("ShaderNodeMix")
        normal_mix.location = (-700, -500)
        if hasattr(normal_mix, "data_type"):
            normal_mix.data_type = "VECTOR"
        links.new(_output_socket(group_input, "Blend Mask"), normal_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Normal"), normal_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Normal"), normal_mix.inputs[3])

        height_mix = nodes.new("ShaderNodeMix")
        height_mix.location = (-700, -680)
        if hasattr(height_mix, "data_type"):
            height_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), height_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Height"), height_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Height"), height_mix.inputs[3])

        bump = nodes.new("ShaderNodeBump")
        bump.location = (-500, -560)
        links.new(_output_socket(group_input, "POM Strength"), bump.inputs[0])
        links.new(height_mix.outputs[0], bump.inputs[2])
        links.new(normal_mix.outputs[0], bump.inputs[3])

        principled = self._create_surface_bsdf(nodes)
        principled.location = (-120, 40)
        links.new(color_mix.outputs[0], _input_socket(principled, "Base Color"))
        links.new(alpha_mix.outputs[0], _input_socket(principled, "Alpha"))
        links.new(roughness_mix.outputs[0], _input_socket(principled, "Roughness"))
        specular_input = _input_socket(principled, "Specular IOR Level", "Specular")
        if specular_input is not None:
            links.new(specular_mix.outputs[0], specular_input)
        normal_input = _input_socket(principled, "Normal")
        if normal_input is not None:
            links.new(bump.outputs[0], normal_input)

        emission = nodes.new("ShaderNodeEmission")
        emission.location = (-120, -220)
        links.new(color_mix.outputs[0], emission.inputs["Color"])
        links.new(_output_socket(group_input, "Emission Strength"), emission.inputs["Strength"])

        add_shader = nodes.new("ShaderNodeAddShader")
        add_shader.location = (120, -40)
        links.new(principled.outputs[0], add_shader.inputs[0])
        links.new(emission.outputs[0], add_shader.inputs[1])

        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (320, -220)
        shadow_toggle = nodes.new("ShaderNodeMath")
        shadow_toggle.location = (500, -220)
        shadow_toggle.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Disable Shadows"), shadow_toggle.inputs[0])
        links.new(_output_socket(light_path, "Is Shadow Ray"), shadow_toggle.inputs[1])
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (500, -400)
        shadow_mix = nodes.new("ShaderNodeMixShader")
        shadow_mix.location = (700, -40)
        links.new(shadow_toggle.outputs[0], shadow_mix.inputs[0])
        links.new(add_shader.outputs[0], shadow_mix.inputs[1])
        links.new(transparent.outputs[0], shadow_mix.inputs[2])
        links.new(shadow_mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "illum_v2"
        return group_tree

    def _connect_manifest_layer_surface_group(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        layer: LayerManifestEntry | None,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
        label: str,
        detail_slots: tuple[str, ...],
    ) -> LayerSurfaceSockets:
        if layer is None:
            return LayerSurfaceSockets()

        base_texture = _layer_texture_reference(layer, slots=("TexSlot1",), roles=("base_color", "diffuse"))
        base_node = self._image_node(nodes, base_texture.export_path if base_texture is not None else None, x=x, y=y, is_color=True)
        detail_ref = _layer_texture_reference(layer, slots=detail_slots)
        detail_channels = self._detail_texture_channels(nodes, detail_ref.export_path if detail_ref is not None else None, x=x, y=y - 420)
        normal_ref = _layer_texture_reference(layer, roles=("normal_gloss",), alpha_semantic="smoothness")
        normal_node = self._image_node(nodes, normal_ref.export_path if normal_ref is not None else None, x=x, y=y - 560, is_color=False)
        roughness, roughness_is_smoothness = self._roughness_socket_for_texture_reference(nodes, normal_ref, x=x + 180, y=y - 560)
        layer_channel_name = layer.palette_channel.name if layer.palette_channel is not None else None
        return self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=base_node.outputs[0] if base_node is not None else None,
            base_alpha_socket=_output_socket(base_node, "Alpha") if base_node is not None else None,
            normal_color_socket=normal_node.outputs[0] if normal_node is not None else None,
            roughness_socket=roughness,
            roughness_source_is_smoothness=roughness_is_smoothness,
            detail_channels=detail_channels,
            detail_diffuse_strength=max(0.0, min(1.0, _float_layer_public_param(layer, "DetailDiffuse"))),
            detail_gloss_strength=max(0.0, min(1.0, _float_layer_public_param(layer, "DetailGloss"))),
            detail_bump_strength=max(0.0, _float_layer_public_param(layer, "DetailBump")),
            tint_color=layer.tint_color,
            palette=palette,
            palette_channel_name=_resolved_layer_palette_diffuse_channel(submaterial, layer_channel_name, palette),
            palette_finish_channel_name=layer_channel_name,
            palette_glossiness=palette_finish_glossiness(palette, layer_channel_name),
            specular_value=_mean_triplet(_layer_snapshot_triplet(layer, "specular")) or 0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, layer_channel_name)) or 0.0,
            x=x + 420,
            y=y - 120,
            label=label,
        )

    def _connect_layer_surface_group(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        *,
        base_color_socket: Any,
        base_alpha_socket: Any,
        normal_color_socket: Any,
        roughness_socket: Any,
        roughness_source_is_smoothness: bool,
        detail_channels: dict[str, Any] | None,
        detail_diffuse_strength: float,
        detail_gloss_strength: float,
        detail_bump_strength: float,
        tint_color: tuple[float, float, float] | None,
        palette: PaletteRecord | None,
        palette_channel_name: str | None,
        palette_finish_channel_name: str | None,
        palette_glossiness: float | None,
        specular_value: float,
        palette_specular_value: float,
        x: int,
        y: int,
        label: str,
    ) -> LayerSurfaceSockets:
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_layer_surface_group()
        group_node.location = (x, y)
        group_node.label = label

        self._set_socket_default(_input_socket(group_node, "Base Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(group_node, "Base Alpha"), 1.0)
        self._set_socket_default(_input_socket(group_node, "Palette Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(
            _input_socket(group_node, "Tint Color"),
            (*tint_color, 1.0) if tint_color is not None else (1.0, 1.0, 1.0, 1.0),
        )
        self._set_socket_default(_input_socket(group_node, "Detail Diffuse Strength"), detail_diffuse_strength)
        self._set_socket_default(_input_socket(group_node, "Detail Gloss Strength"), detail_gloss_strength)
        self._set_socket_default(_input_socket(group_node, "Detail Bump Strength"), detail_bump_strength)
        self._set_socket_default(_input_socket(group_node, "Normal Color"), (0.5, 0.5, 1.0, 1.0))
        self._set_socket_default(_input_socket(group_node, "Roughness Source"), 0.45)
        self._set_socket_default(_input_socket(group_node, "Roughness Source Is Smoothness"), roughness_source_is_smoothness)
        self._set_socket_default(
            _input_socket(group_node, "Palette Glossiness"),
            max(0.0, min(1.0, palette_glossiness)) if palette_glossiness is not None else 0.0,
        )
        self._set_socket_default(_input_socket(group_node, "Specular Value"), specular_value)
        self._set_socket_default(_input_socket(group_node, "Palette Specular"), palette_specular_value)
        if palette_finish_channel_name is not None:
            group_node["starbreaker_palette_finish_channel"] = palette_finish_channel_name

        palette_color_socket = None
        palette_gloss_socket = None
        palette_specular_socket = None
        palette_specular_tint_socket = None
        if palette is not None and palette_channel_name is not None:
            palette_color_socket = self._palette_color_socket(nodes, palette, palette_channel_name, x=x - 220, y=y - 160)
            finish_channel_name = palette_finish_channel_name or palette_channel_name
            palette_gloss_socket = self._palette_glossiness_socket(nodes, palette, finish_channel_name, x=x - 220, y=y - 320)
            palette_specular_color = self._palette_specular_socket(nodes, palette, finish_channel_name, x=x - 220, y=y - 480)
            if palette_specular_color is not None:
                palette_specular_tint_socket = palette_specular_color
                rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
                rgb_to_bw.location = (x - 20, y - 480)
                links.new(palette_specular_color, rgb_to_bw.inputs[0])
                palette_specular_socket = rgb_to_bw.outputs[0]

        self._link_group_input(links, base_color_socket, group_node, "Base Color")
        self._link_group_input(links, base_alpha_socket, group_node, "Base Alpha")
        self._link_group_input(links, normal_color_socket, group_node, "Normal Color")
        self._link_group_input(links, roughness_socket, group_node, "Roughness Source")
        self._link_group_input(links, palette_color_socket, group_node, "Palette Color")
        self._link_group_input(links, palette_gloss_socket, group_node, "Palette Glossiness")
        self._link_group_input(links, palette_specular_socket, group_node, "Palette Specular")
        if detail_channels is not None:
            self._link_group_input(links, detail_channels.get("red"), group_node, "Detail Color Mask")
            self._link_group_input(links, detail_channels.get("green"), group_node, "Detail Height Mask")
            self._link_group_input(links, detail_channels.get("blue"), group_node, "Detail Gloss Mask")

        return LayerSurfaceSockets(
            color=SocketRef(group_node, "Color"),
            alpha=SocketRef(group_node, "Alpha"),
            normal=SocketRef(group_node, "Normal"),
            roughness=SocketRef(group_node, "Roughness"),
            specular=SocketRef(group_node, "Specular"),
            specular_tint=(SocketRef(palette_specular_tint_socket.node, palette_specular_tint_socket.name) if palette_specular_tint_socket is not None else None),
        )

    def _link_group_input(self, links: bpy.types.NodeLinks, source_socket: Any, group_node: bpy.types.Node, socket_name: str) -> None:
        if source_socket is None:
            return
        if isinstance(source_socket, SocketRef):
            _refresh_group_node_sockets(source_socket.node)
            source_socket = (
                _output_socket(source_socket.node, source_socket.name)
                if source_socket.is_output
                else _input_socket(source_socket.node, source_socket.name)
            )
            if source_socket is None:
                return
        _refresh_group_node_sockets(group_node)
        source_node = getattr(source_socket, "node", None)
        source_name = getattr(source_socket, "name", "")
        _refresh_group_node_sockets(source_node)
        if source_node is not None and source_name:
            source_socket = _output_socket(source_node, source_name) or source_socket

        target_socket = _input_socket(group_node, socket_name)
        if target_socket is None:
            return
        target_node = getattr(target_socket, "node", None)
        target_name = getattr(target_socket, "name", "")
        if target_node is not None and target_name:
            target_socket = _input_socket(target_node, target_name) or target_socket

        if not getattr(source_socket, "is_output", False):
            source_node = getattr(source_socket, "node", None)
            source_socket = _output_socket(source_node, getattr(source_socket, "name", "")) if source_node is not None else None
        if getattr(target_socket, "is_output", False):
            target_node = getattr(target_socket, "node", None)
            target_socket = _input_socket(target_node, getattr(target_socket, "name", "")) if target_node is not None else None
        if source_socket is None or target_socket is None:
            return
        if not getattr(source_socket, "is_output", False) or getattr(target_socket, "is_output", False):
            return
        try:
            links.new(source_socket, target_socket)
        except RuntimeError as exc:
            if "Same input/output direction of sockets" in str(exc):
                return
            raise

    def _set_socket_default(self, socket: Any, value: Any) -> None:
        if socket is not None and hasattr(socket, "default_value"):
            socket.default_value = value

    def _apply_material_palette_tint(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        color_socket: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        channel = submaterial.palette_routing.material_channel
        if color_socket is None or channel is None or palette is None:
            return color_socket
        palette_socket = self._palette_color_socket(nodes, palette, channel.name, x=x, y=y)
        return self._multiply_color_socket(nodes, links, color_socket, palette_socket, x=x + 180, y=y)

    def _detail_texture_channels(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> dict[str, Any] | None:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        separate = nodes.new("ShaderNodeSeparateColor")
        separate.location = (x + 180, y)
        if hasattr(separate, "mode"):
            separate.mode = "RGB"
        image_node.id_data.links.new(image_node.outputs[0], separate.inputs[0])
        return {
            "red": _output_socket(separate, "Red", "R"),
            "green": _output_socket(separate, "Green", "G"),
            "blue": _output_socket(separate, "Blue", "B"),
            "alpha": _output_socket(image_node, "Alpha"),
        }

    def _apply_detail_color(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        color_socket: Any,
        detail_channels: dict[str, Any] | None,
        *,
        strength: float,
        x: int,
        y: int,
    ) -> Any:
        if color_socket is None or detail_channels is None or detail_channels.get("red") is None or strength <= 0.0:
            return color_socket
        grayscale = nodes.new("ShaderNodeValToRGB")
        grayscale.location = (x, y)
        links.new(detail_channels["red"], grayscale.inputs[0])
        white = self._value_color_socket(nodes, (1.0, 1.0, 1.0, 1.0), x=x, y=y - 120)
        tint_mix = nodes.new("ShaderNodeMixRGB")
        tint_mix.location = (x + 180, y)
        tint_mix.blend_type = "MIX"
        tint_mix.inputs[0].default_value = max(0.0, min(1.0, strength))
        self._link_color_output(white, tint_mix.inputs[1])
        links.new(grayscale.outputs[0], tint_mix.inputs[2])
        return self._multiply_color_socket(nodes, links, color_socket, tint_mix.outputs[0], x=x + 360, y=y)

    def _apply_detail_gloss(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        roughness_socket: Any,
        detail_channels: dict[str, Any] | None,
        *,
        strength: float,
        x: int,
        y: int,
    ) -> Any:
        if roughness_socket is None or detail_channels is None or detail_channels.get("blue") is None or strength <= 0.0:
            return roughness_socket
        detail_value = nodes.new("ShaderNodeMix")
        detail_value.location = (x, y)
        if hasattr(detail_value, "data_type"):
            detail_value.data_type = "FLOAT"
        detail_value.inputs[0].default_value = max(0.0, min(1.0, strength))
        detail_value.inputs[2].default_value = 1.0
        links.new(detail_channels["blue"], detail_value.inputs[3])
        return self._multiply_value_socket(nodes, links, roughness_socket, detail_value.outputs[0], x=x + 180, y=y)

    def _roughness_socket_for_texture_reference(
        self,
        nodes: bpy.types.Nodes,
        texture: TextureReference | None,
        *,
        x: int,
        y: int,
    ) -> tuple[Any, bool]:
        if texture is None or texture.export_path is None:
            return None, False
        if texture.alpha_semantic == "smoothness":
            smoothness = self._texture_alpha_socket(nodes, texture.export_path, x=x, y=y, is_color=False)
            if smoothness is not None:
                return smoothness, True
        image_node = self._image_node(nodes, texture.export_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None, False
        return image_node.outputs[0], False

    def _specular_socket_for_texture_path(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
        rgb_to_bw.location = (x + 180, y)
        image_node.id_data.links.new(image_node.outputs[0], rgb_to_bw.inputs[0])
        return rgb_to_bw.outputs[0]

    def _mask_socket(self, nodes: bpy.types.Nodes, image_path: str | None, *, x: int, y: int) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        return image_node.outputs[0]

    def _mix_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        factor_socket: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        if factor_socket is None:
            return socket_a
        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x, y)
        mix.blend_type = "MIX"
        links.new(factor_socket, mix.inputs[0])
        self._link_color_output(socket_a, mix.inputs[1])
        self._link_color_output(socket_b, mix.inputs[2])
        return mix.outputs[0]

    def _multiply_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x, y)
        mix.blend_type = "MULTIPLY"
        mix.inputs[0].default_value = 1.0
        self._link_color_output(socket_a, mix.inputs[1])
        self._link_color_output(socket_b, mix.inputs[2])
        return mix.outputs[0]

    def _mix_value_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        factor_socket: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        if factor_socket is None:
            return socket_a
        mix = nodes.new("ShaderNodeMix")
        mix.location = (x, y)
        if hasattr(mix, "data_type"):
            mix.data_type = "FLOAT"
        links.new(factor_socket, mix.inputs[0])
        links.new(socket_a, mix.inputs[2])
        links.new(socket_b, mix.inputs[3])
        return mix.outputs[0]

    def _multiply_value_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        multiply = nodes.new("ShaderNodeMath")
        multiply.location = (x, y)
        multiply.operation = "MULTIPLY"
        links.new(socket_a, multiply.inputs[0])
        links.new(socket_b, multiply.inputs[1])
        return multiply.outputs[0]

    def _add_clamped_value_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        add = nodes.new("ShaderNodeMath")
        add.location = (x, y)
        add.operation = "ADD"
        add.use_clamp = True
        links.new(socket_a, add.inputs[0])
        links.new(socket_b, add.inputs[1])
        return add.outputs[0]

    def _normal_from_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        color_socket: Any,
        *,
        x: int,
        y: int,
        strength: float,
    ) -> Any:
        if color_socket is None:
            return None
        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.location = (x, y)
        strength_input = _input_socket(normal_map, "Strength")
        if strength_input is not None:
            strength_input.default_value = strength
        links.new(color_socket, _input_socket(normal_map, "Color"))
        return _output_socket(normal_map, "Normal")

    def _bump_normal_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        height_socket: Any,
        base_normal_socket: Any,
        *,
        strength: float | None = None,
        strength_socket: Any = None,
        x: int,
        y: int,
    ) -> Any:
        if height_socket is None:
            return base_normal_socket
        bump = nodes.new("ShaderNodeBump")
        bump.location = (x, y)
        if strength_socket is not None:
            links.new(strength_socket, bump.inputs[0])
        elif strength is not None:
            bump.inputs[0].default_value = strength
        links.new(height_socket, bump.inputs[2])
        if base_normal_socket is not None:
            links.new(base_normal_socket, bump.inputs[3])
        return bump.outputs[0]

    def _combine_normal_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        add = nodes.new("ShaderNodeVectorMath")
        add.location = (x, y)
        add.operation = "ADD"
        links.new(socket_a, add.inputs[0])
        links.new(socket_b, add.inputs[1])
        normalize = nodes.new("ShaderNodeVectorMath")
        normalize.location = (x + 180, y)
        normalize.operation = "NORMALIZE"
        links.new(add.outputs[0], normalize.inputs[0])
        return normalize.outputs[0]

    def _texture_path_for_slot(self, submaterial: SubmaterialRecord, slot: str) -> str | None:
        texture = _submaterial_texture_reference(submaterial, slots=(slot,))
        return texture.export_path if texture is not None else None
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)
        return True

    def _contract_input_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        group_contract: ShaderGroupContract,
        contract_input: ContractInput,
        *,
        x: int,
        y: int,
    ) -> Any:
        if contract_input.name.startswith("Palette_"):
            if palette is None:
                return None
            channel_name = contract_input.name.removeprefix("Palette_").lower()
            used_channels = {channel.name.lower() for channel in material_palette_channels(submaterial)}
            if channel_name not in used_channels:
                return None
            return self._palette_color_socket(nodes, palette, channel_name, x=x, y=y)

        semantic = (contract_input.semantic or contract_input.name).lower()
        if _routes_virtual_tint_palette_decal_to_decal_source(submaterial, contract_input):
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).color
        if _suppresses_virtual_tint_palette_stencil_input(submaterial, contract_input):
            return None
        if contract_input.source_slot is not None and contract_input.name.lower().endswith("_alpha"):
            return self._source_slot_alpha_socket(nodes, submaterial, contract_input, palette, x=x, y=y)

        texture = self._texture_reference_for_contract_input(submaterial, contract_input)
        if texture is not None and texture.is_virtual and texture.role == "tint_palette_decal":
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).color

        if "alpha" in semantic or "opacity" in semantic:
            return self._alpha_source_socket(
                nodes,
                submaterial,
                representative_textures(submaterial),
                x=x,
                y=y,
            )

        if contract_input.source_slot is None and "roughness" in semantic:
            return self._roughness_group_source_socket(
                nodes,
                submaterial,
                representative_textures(submaterial)["roughness"],
                x=x,
                y=y,
            )

        image_path = texture.export_path if texture is not None else self._texture_path_for_contract_input(submaterial, contract_input)
        if _contract_input_uses_color(contract_input):
            if any(item.name.startswith("Palette_") for item in group_contract.inputs):
                image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=True)
                if image_node is None:
                    return None
                return image_node.outputs[0]
            return self._color_source_socket(nodes, submaterial, palette, image_path, x=x, y=y)
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        return image_node.outputs[0]

    def _roughness_group_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if image_path:
            image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
            if image_node is not None:
                image_node.label = "METALLIC ROUGHNESS"

                separate = nodes.new("ShaderNodeSeparateColor")
                separate.location = (x + 180, y)
                if hasattr(separate, "mode"):
                    separate.mode = "RGB"
                image_node.id_data.links.new(image_node.outputs[0], separate.inputs[0])
                return _output_socket(separate, "Green")

        smoothness_texture = self._smoothness_texture_reference(submaterial)
        if smoothness_texture is None:
            return None
        smoothness_alpha = self._texture_alpha_socket(
            nodes,
            smoothness_texture.export_path,
            x=x,
            y=y,
            is_color=False,
        )
        if smoothness_alpha is None:
            return None
        return self._invert_value_socket(nodes, smoothness_alpha, x=x + 180, y=y)

    def _smoothness_texture_reference(self, submaterial: SubmaterialRecord) -> TextureReference | None:
        return smoothness_texture_reference(submaterial)

    def _texture_reference_for_contract_input(self, submaterial: SubmaterialRecord, contract_input: ContractInput) -> TextureReference | None:
        source_slot = contract_input.source_slot
        if source_slot is None:
            return None
        texture = _matching_texture_reference(
            [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures],
            slots=(source_slot,),
        )
        if texture is not None:
            return texture

        for layer in submaterial.layer_manifest:
            texture = _layer_texture_reference(layer, slots=(source_slot,))
            if texture is not None:
                return texture
        return None

    def _texture_alpha_socket(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
    ) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=is_color)
        if image_node is None:
            return None
        return _output_socket(image_node, "Alpha")

    def _virtual_tint_palette_decal_sockets(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> LayerSurfaceSockets:
        fallback_color, alpha = self._virtual_tint_palette_decal_defaults(submaterial, palette)
        if palette is not None and self.package.resolve_path(palette_decal_texture(palette)) is not None:
            group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
            color_socket = _output_socket(group_node, "Palette Decal Color")
            alpha_socket = _output_socket(group_node, "Palette Decal Alpha")
            if color_socket is not None and alpha_socket is not None:
                if abs(alpha - 1.0) < 1e-6:
                    return LayerSurfaceSockets(color=color_socket, alpha=alpha_socket)
                alpha_multiply = next(
                    (node for node in nodes if node.bl_idname == "ShaderNodeMath" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA_MULTIPLY"),
                    None,
                )
                if alpha_multiply is None:
                    alpha_multiply = nodes.new("ShaderNodeMath")
                    alpha_multiply.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA_MULTIPLY"
                    alpha_multiply.label = "StarBreaker Virtual Tint Palette Decal Alpha"
                    alpha_multiply.operation = "MULTIPLY"
                alpha_multiply.location = (x + 220, y - 140)
                alpha_multiply.inputs[1].default_value = alpha
                nodes.id_data.links.new(alpha_socket, alpha_multiply.inputs[0])
                return LayerSurfaceSockets(color=color_socket, alpha=alpha_multiply.outputs[0])

        color_node = next(
            (node for node in nodes if node.bl_idname == "ShaderNodeRGB" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"),
            None,
        )
        alpha_node = next(
            (node for node in nodes if node.bl_idname == "ShaderNodeValue" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"),
            None,
        )

        if color_node is None:
            color_node = nodes.new("ShaderNodeRGB")
            color_node.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"
            color_node.label = "StarBreaker Virtual Tint Palette Decal"
        color_node.location = (x, y)

        if alpha_node is None:
            alpha_node = nodes.new("ShaderNodeValue")
            alpha_node.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"
            alpha_node.label = "StarBreaker Virtual Tint Palette Decal Alpha"
        alpha_node.location = (x, y - 140)

        color_node.outputs[0].default_value = (*fallback_color, 1.0)
        alpha_node.outputs[0].default_value = alpha
        return LayerSurfaceSockets(color=color_node.outputs[0], alpha=alpha_node.outputs[0])

    def _virtual_tint_palette_decal_defaults(
        self,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
    ) -> tuple[tuple[float, float, float], float]:
        color = (
            _public_param_triplet(submaterial, "StencilDiffuseColor1", "StencilDiffuse1", "StencilTintColor", "TintColor", "StencilDiffuseColor")
            or _resolved_submaterial_palette_color(submaterial, palette)
            or _authored_attribute_triplet(submaterial, "Diffuse")
            or (1.0, 1.0, 1.0)
        )
        alpha = _optional_float_public_param(submaterial, "StencilOpacity", "DecalDiffuseOpacity", "DecalAlphaMult")
        if alpha is None:
            alpha = 0.85 if submaterial.shader_family == "MeshDecal" else 0.5
        return color, max(0.0, min(1.0, alpha))

    def _invert_value_socket(self, nodes: bpy.types.Nodes, source_socket: Any, *, x: int, y: int) -> Any:
        invert = nodes.new("ShaderNodeMath")
        invert.location = (x, y)
        invert.operation = "SUBTRACT"
        invert.inputs[0].default_value = 1.0
        invert.id_data.links.new(source_socket, invert.inputs[1])
        return invert.outputs[0]

    def _source_slot_alpha_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        contract_input: ContractInput,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if _routes_virtual_tint_palette_decal_alpha_to_decal_source(submaterial, contract_input):
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).alpha
        if _suppresses_virtual_tint_palette_stencil_input(submaterial, contract_input):
            return None
        texture = self._texture_reference_for_contract_input(submaterial, contract_input)
        if texture is None:
            return None
        if texture.is_virtual and texture.role == "tint_palette_decal":
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).alpha
        return self._texture_alpha_socket(nodes, texture.export_path, x=x, y=y, is_color=True)

    def _texture_path_for_contract_input(self, submaterial: SubmaterialRecord, contract_input: ContractInput) -> str | None:
        texture = self._texture_reference_for_contract_input(submaterial, contract_input)
        return texture.export_path if texture is not None else None

    def _build_nodraw_material(self, material: bpy.types.Material) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()
        output = nodes.new("ShaderNodeOutputMaterial")
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        links.new(transparent.outputs[0], output.inputs[0])
        self._configure_material(material, blend_method="CLIP", shadow_method="NONE")

    def _build_screen_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (550, 0)
        emission = nodes.new("ShaderNodeEmission")
        emission.location = (250, 0)
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (250, -180)
        mix = nodes.new("ShaderNodeMixShader")
        mix.location = (400, 0)
        mix.inputs[0].default_value = 0.12

        image_path = representative_textures(submaterial)["base_color"]
        color_source = self._color_source_socket(nodes, submaterial, palette, image_path, x=0, y=0)
        if color_source is not None:
            links.new(color_source, _input_socket(emission, "Color"))
        elif has_virtual_input(submaterial, "$RenderToTexture"):
            checker = nodes.new("ShaderNodeTexChecker")
            checker.location = (0, 0)
            links.new(_output_socket(checker, "Color"), _input_socket(emission, "Color"))

        emission_strength = _input_socket(emission, "Strength")
        if emission_strength is not None:
            emission_strength.default_value = 3.0

        links.new(emission.outputs[0], mix.inputs[2])
        links.new(transparent.outputs[0], mix.inputs[1])
        links.new(mix.outputs[0], output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_effect_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (550, 0)
        emission = nodes.new("ShaderNodeEmission")
        emission.location = (250, 0)
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (250, -180)
        mix = nodes.new("ShaderNodeMixShader")
        mix.location = (400, 0)
        mix.inputs[0].default_value = 0.35

        color_source = self._color_source_socket(nodes, submaterial, palette, representative_textures(submaterial)["base_color"], x=0, y=0)
        if color_source is not None:
            links.new(color_source, _input_socket(emission, "Color"))
        emission_strength = _input_socket(emission, "Strength")
        if emission_strength is not None:
            emission_strength.default_value = 2.5

        links.new(emission.outputs[0], mix.inputs[2])
        links.new(transparent.outputs[0], mix.inputs[1])
        links.new(mix.outputs[0], output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_principled_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)
        principled = self._create_surface_bsdf(nodes)
        surface_shader = principled.outputs[0]

        textures = representative_textures(submaterial)
        base_socket = self._color_source_socket(nodes, submaterial, palette, textures["base_color"], x=40, y=140)
        if base_socket is None and palette is not None and plan.uses_palette:
            primary = self._palette_color_socket(nodes, palette, "primary", x=80, y=120)
            base_socket = primary

        wear_factor_socket = None
        if plan.template_key == "layered_wear":
            wear_factor_socket = self._layered_wear_factor_socket(nodes, links, submaterial, x=40, y=-20)
            base_socket = self._mix_layered_base_color(nodes, links, submaterial, palette, base_socket, wear_factor_socket)

        if base_socket is not None:
            links.new(base_socket, _input_socket(principled, "Base Color"))

        if plan.uses_alpha:
            alpha_socket = _input_socket(principled, "Alpha")
            alpha_source = self._alpha_source_socket(nodes, submaterial, textures, x=80, y=20)
            if alpha_socket is not None:
                if alpha_source is not None:
                    links.new(alpha_source, alpha_socket)
                elif plan.template_key == "hair":
                    alpha_socket.default_value = 0.85

        roughness_socket = _input_socket(principled, "Roughness")
        roughness_default = 0.45 if submaterial.shader_family != "GlassPBR" else 0.08
        roughness_source = self._roughness_group_source_socket(
            nodes,
            submaterial,
            textures["roughness"],
            x=80,
            y=-120,
        )
        if plan.template_key == "layered_wear":
            roughness_source = self._mix_layered_roughness(
                nodes,
                links,
                submaterial,
                roughness_source,
                wear_factor_socket,
                default_value=roughness_default,
            )
        if roughness_socket is not None:
            if roughness_source is not None:
                links.new(roughness_source, roughness_socket)
            else:
                roughness_socket.default_value = roughness_default

        normal_input = _input_socket(principled, "Normal")
        normal_node = self._image_node(nodes, textures["normal"], x=80, y=-280, is_color=False)
        bump_node = None
        if textures["height"] or plan.template_key == "parallax_pom":
            bump_node = nodes.new("ShaderNodeBump")
            bump_node.location = (240, -320)
            bump_input = _input_socket(bump_node, "Height")
            bump_input.default_value = 0.02
            height_node = self._image_node(nodes, textures["height"] or textures["mask"], x=40, y=-420, is_color=False)
            if height_node is not None:
                links.new(height_node.outputs[0], bump_input)
        if normal_node is not None:
            normal_map = nodes.new("ShaderNodeNormalMap")
            normal_map.location = (240, -220)
            links.new(normal_node.outputs[0], _input_socket(normal_map, "Color"))
            if bump_node is not None:
                links.new(_output_socket(normal_map, "Normal"), _input_socket(bump_node, "Normal"))
            elif normal_input is not None:
                links.new(_output_socket(normal_map, "Normal"), normal_input)
        if bump_node is not None and normal_input is not None:
            links.new(_output_socket(bump_node, "Normal"), normal_input)

        if plan.uses_transmission:
            transmission = _input_socket(principled, "Transmission Weight", "Transmission")
            if transmission is not None:
                transmission.default_value = 1.0
            ior_socket = _input_socket(principled, "IOR")
            if ior_socket is not None:
                ior_socket.default_value = 1.45
            alpha_socket = _input_socket(principled, "Alpha")
            if alpha_socket is not None:
                alpha_socket.default_value = 0.2

        if plan.uses_emission:
            emission_color = _input_socket(principled, "Emission Color", "Emission")
            if emission_color is not None:
                if base_socket is not None:
                    links.new(base_socket, emission_color)
                elif palette is not None and plan.uses_palette:
                    emissive = self._palette_color_socket(nodes, palette, "primary", x=80, y=300)
                    links.new(emissive, emission_color)
            emission_strength = _input_socket(principled, "Emission Strength")
            if emission_strength is not None:
                emission_strength.default_value = 2.0

        if plan.template_key == "biological":
            subsurface = _input_socket(principled, "Subsurface Weight", "Subsurface")
            if subsurface is not None:
                subsurface.default_value = 0.15

        if plan.template_key == "hair":
            anisotropic = _input_socket(principled, "Anisotropic")
            if anisotropic is not None:
                anisotropic.default_value = 0.4

        if self._plan_casts_no_shadows(plan):
            surface_shader = self._shadowless_surface_output(nodes, links, surface_shader)

        links.new(surface_shader, output.inputs[0])

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_glass_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (620, 0)
        glass = nodes.new("ShaderNodeBsdfGlass")
        glass.location = (360, 0)
        glass.label = "StarBreaker Glass"
        links.new(glass.outputs[0], output.inputs[0])

        textures = representative_textures(submaterial)
        base_path = textures["base_color"]
        roughness_path = textures["roughness"] or self._texture_export_path(submaterial, "wear_gloss")
        normal_path = textures["normal"]

        base_socket = self._color_source_socket(nodes, submaterial, palette, base_path, x=40, y=140)
        if base_socket is None and palette is not None:
            base_socket = self._palette_color_socket(nodes, palette, "glass", x=80, y=120)
        if base_socket is None:
            base_socket = self._value_color_socket(nodes, (1.0, 1.0, 1.0, 1.0), x=80, y=120)
        if base_socket is not None:
            links.new(base_socket, _input_socket(glass, "Color"))

        roughness_socket = _input_socket(glass, "Roughness")
        roughness_node = self._image_node(nodes, roughness_path, x=80, y=-120, is_color=False)
        if roughness_socket is not None:
            if roughness_node is not None:
                links.new(roughness_node.outputs[0], roughness_socket)
            else:
                roughness_socket.default_value = 0.08

        ior_socket = _input_socket(glass, "IOR")
        if ior_socket is not None:
            ior_socket.default_value = 1.05

        normal_input = _input_socket(glass, "Normal")
        normal_node = self._image_node(nodes, normal_path, x=80, y=-280, is_color=False)
        if normal_node is not None and normal_input is not None:
            normal_map = nodes.new("ShaderNodeNormalMap")
            normal_map.location = (240, -220)
            strength_socket = _input_socket(normal_map, "Strength")
            if strength_socket is not None:
                strength_socket.default_value = 0.25
            links.new(normal_node.outputs[0], _input_socket(normal_map, "Color"))
            links.new(_output_socket(normal_map, "Normal"), normal_input)

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _create_surface_bsdf(self, nodes: bpy.types.Nodes) -> bpy.types.ShaderNodeBsdfPrincipled:
        principled = nodes.new("ShaderNodeBsdfPrincipled")
        principled.location = (420, 0)
        principled.label = "StarBreaker Surface"
        return principled

    def _wear_strength(self) -> float:
        raw_value = getattr(self.context.scene, SCENE_WEAR_STRENGTH_PROP, 1.0)
        try:
            value = float(raw_value)
        except (TypeError, ValueError):
            value = 1.0
        return max(0.0, min(2.0, value))

    def _layered_wear_factor_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        textures = representative_textures(submaterial)
        source = None
        mask_node = self._image_node(nodes, textures["mask"], x=x, y=y, is_color=False)
        if mask_node is not None:
            source = mask_node.outputs[0]

        wear_base = _float_public_param(submaterial, "WearBlendBase", "DamagePerObjectWear")
        if source is None and wear_base <= 0.0:
            return None

        if source is None:
            source = self._value_socket(nodes, min(1.0, wear_base if wear_base > 0.0 else 1.0), x=x + 180, y=y)
        elif wear_base > 0.0 and abs(wear_base - 1.0) > 1e-6:
            multiply = nodes.new("ShaderNodeMath")
            multiply.location = (x + 180, y)
            multiply.operation = "MULTIPLY"
            multiply.use_clamp = True
            links.new(source, multiply.inputs[0])
            multiply.inputs[1].default_value = wear_base
            source = multiply.outputs[0]

        wear_strength = self._wear_strength()
        if abs(wear_strength - 1.0) > 1e-6:
            strength = nodes.new("ShaderNodeMath")
            strength.location = (x + 360, y)
            strength.operation = "MULTIPLY"
            strength.use_clamp = True
            links.new(source, strength.inputs[0])
            strength.inputs[1].default_value = wear_strength
            source = strength.outputs[0]
        return source

    def _mix_layered_base_color(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        base_socket: Any,
        wear_factor_socket: Any,
    ) -> Any:
        layer_color = self._layer_color_socket(nodes, submaterial, palette, x=40, y=320)
        if wear_factor_socket is None:
            return base_socket or layer_color
        if base_socket is None:
            return layer_color
        if layer_color is None:
            return base_socket

        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (320, 160)
        mix.blend_type = "MIX"
        links.new(wear_factor_socket, mix.inputs[0])
        self._link_color_output(base_socket, mix.inputs[1])
        self._link_color_output(layer_color, mix.inputs[2])
        return mix.outputs[0]

    def _mix_layered_roughness(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        base_source: Any,
        wear_factor_socket: Any,
        *,
        default_value: float,
    ) -> Any:
        layer_source = self._layer_roughness_socket(nodes, submaterial, x=80, y=-500)
        if wear_factor_socket is None:
            return base_source or layer_source
        if base_source is None:
            base_source = self._value_socket(nodes, default_value, x=260, y=-120)
        if layer_source is None:
            return base_source

        mix = nodes.new("ShaderNodeMix")
        mix.location = (320, -260)
        if hasattr(mix, "data_type"):
            mix.data_type = "FLOAT"
        links.new(wear_factor_socket, mix.inputs[0])
        links.new(base_source, mix.inputs[2])
        links.new(layer_source, mix.inputs[3])
        return mix.outputs[0]

    def _layer_color_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        layer = next((item for item in submaterial.layer_manifest if item.diffuse_export_path), None)
        if layer is None:
            return None

        source = self._image_node(nodes, layer.diffuse_export_path, x=x, y=y, is_color=True)
        if source is None:
            return None
        output = source.outputs[0]

        if layer.tint_color is not None and any(abs(channel - 1.0) > 1e-6 for channel in layer.tint_color):
            tint = nodes.new("ShaderNodeRGB")
            tint.location = (x, y - 160)
            tint.outputs[0].default_value = (*layer.tint_color, 1.0)
            mix = nodes.new("ShaderNodeMixRGB")
            mix.location = (x + 180, y)
            mix.inputs[0].default_value = 1.0
            self._link_color_output(output, mix.inputs[1])
            self._link_color_output(tint.outputs[0], mix.inputs[2])
            output = mix.outputs[0]

        if layer.palette_channel is not None and palette is not None:
            palette_socket = self._palette_color_socket(nodes, palette, layer.palette_channel.name, x=x, y=y - 320)
            mix = nodes.new("ShaderNodeMixRGB")
            mix.location = (x + 360, y)
            mix.blend_type = "MULTIPLY"
            mix.inputs[0].default_value = 1.0
            self._link_color_output(output, mix.inputs[1])
            self._link_color_output(palette_socket, mix.inputs[2])
            output = mix.outputs[0]

        return output

    def _layer_roughness_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        layer = next(
            (
                item
                for item in submaterial.layer_manifest
                if item.roughness_export_path
                or any(texture.alpha_semantic == "smoothness" and texture.export_path for texture in item.texture_slots)
            ),
            None,
        )
        if layer is None:
            return None
        if layer.roughness_export_path:
            image_node = self._image_node(nodes, layer.roughness_export_path, x=x, y=y, is_color=False)
            if image_node is not None:
                return image_node.outputs[0]
        smoothness_texture = next(
            (texture for texture in layer.texture_slots if texture.alpha_semantic == "smoothness" and texture.export_path),
            None,
        )
        if smoothness_texture is None:
            return None
        smoothness_alpha = self._texture_alpha_socket(
            nodes,
            smoothness_texture.export_path,
            x=x,
            y=y,
            is_color=False,
        )
        if smoothness_alpha is None:
            return None
        return self._invert_value_socket(nodes, smoothness_alpha, x=x + 180, y=y)

    def _value_socket(self, nodes: bpy.types.Nodes, value: float, *, x: int, y: int) -> Any:
        node = nodes.new("ShaderNodeValue")
        node.location = (x, y)
        node.outputs[0].default_value = value
        return node.outputs[0]

    def _value_color_socket(self, nodes: bpy.types.Nodes, value: tuple[float, float, float, float], *, x: int, y: int) -> Any:
        node = nodes.new("ShaderNodeRGB")
        node.location = (x, y)
        node.outputs[0].default_value = value
        return node.outputs[0]

    def _texture_export_path(self, submaterial: SubmaterialRecord, *roles: str) -> str | None:
        for texture in [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures]:
            if texture.role in roles and texture.export_path:
                return texture.export_path
        return None

    def _alpha_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        textures: dict[str, str | None],
        *,
        x: int,
        y: int,
    ) -> Any:
        opacity_path = textures.get("opacity")
        if opacity_path:
            opacity_node = self._image_node(nodes, opacity_path, x=x, y=y, is_color=False)
            if opacity_node is not None:
                return opacity_node.outputs[0]

        alpha_image_path = (
            textures.get("base_color")
            or self._texture_export_path(submaterial, "decal_sheet", "diffuse", "alternate_base_color")
        )
        alpha_node = self._image_node(nodes, alpha_image_path, x=x, y=y, is_color=True)
        if alpha_node is None:
            return None
        return _output_socket(alpha_node, "Alpha")

    def _illum_emission_strength(self, submaterial: SubmaterialRecord) -> float:
        glow_value = _float_authored_attribute(submaterial, "Glow")
        if glow_value > 0.0:
            return glow_value

        if self._texture_export_path(submaterial, "emissive"):
            return 1.0

        material_name = " ".join(
            part.lower()
            for part in (submaterial.submaterial_name, submaterial.blender_material_name)
            if part
        )
        if "glow" in material_name or "emissive" in material_name:
            return 0.35
        return 0.0

    def _color_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=True)
        channels = material_palette_channels(submaterial)
        active_channel = submaterial.palette_routing.material_channel or (channels[0] if channels else None)

        if image_node is None and active_channel is None:
            return None
        if active_channel is None or palette is None:
            return image_node.outputs[0] if image_node is not None else None

        palette_socket = self._palette_color_socket(nodes, palette, active_channel.name, x=x, y=y - 180)
        if image_node is None:
            return palette_socket

        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x + 180, y)
        mix.blend_type = "MULTIPLY"
        mix.inputs[0].default_value = 1.0
        mix.inputs[1].default_value = (1.0, 1.0, 1.0, 1.0)
        self._link_color_output(image_node.outputs[0], mix.inputs[1])
        self._link_color_output(palette_socket, mix.inputs[2])
        return mix.outputs[0]

    def _image_node(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
        reuse_any_existing: bool = False,
    ) -> bpy.types.ShaderNodeTexImage | None:
        resolved = self.package.resolve_path(image_path)
        if resolved is None or not resolved.is_file():
            return None
        resolved_str = str(resolved)
        for existing in nodes:
            if existing.bl_idname != "ShaderNodeTexImage":
                continue
            image = getattr(existing, "image", None)
            if image is None:
                continue
            if bpy.path.abspath(image.filepath, library=image.library) != resolved_str:
                continue
            if reuse_any_existing:
                existing.location = (x, y)
                return existing
            color_space = getattr(getattr(image, "colorspace_settings", None), "name", "")
            if is_color and color_space != "Non-Color":
                existing.location = (x, y)
                return existing
            if not is_color and color_space == "Non-Color":
                existing.location = (x, y)
                return existing
        node = nodes.new("ShaderNodeTexImage")
        node.location = (x, y)
        node.image = bpy.data.images.load(str(resolved), check_existing=True)
        if not is_color and node.image is not None and hasattr(node.image, "colorspace_settings"):
            node.image.colorspace_settings.name = "Non-Color"
        return node

    def _apply_material_node_layout(self, material: bpy.types.Material) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        nodes = node_tree.nodes
        links = node_tree.links
        layout = MATERIAL_NODE_LAYOUT

        output = next((node for node in nodes if node.bl_idname == "ShaderNodeOutputMaterial"), None)
        if output is not None:
            output.location = (layout.output_x, layout.output_y)

        primary_node = self._primary_surface_node(nodes, links, output)
        if primary_node is not None:
            primary_node.location = (layout.primary_x, layout.primary_y)
            if primary_node.bl_idname == "ShaderNodeGroup":
                primary_node.width = layout.group_width

        shadow_mix = next((node for node in nodes if node.bl_idname == "ShaderNodeMixShader" and node != primary_node), None)
        if shadow_mix is not None:
            shadow_mix.location = (layout.shadow_mix_x, layout.shadow_mix_y)

        shadow_transparent = next((node for node in nodes if node.bl_idname == "ShaderNodeBsdfTransparent"), None)
        if shadow_transparent is not None:
            shadow_transparent.location = (layout.shadow_transparent_x, layout.shadow_transparent_y)

        shadow_light_path = next((node for node in nodes if node.bl_idname == "ShaderNodeLightPath"), None)
        if shadow_light_path is not None:
            shadow_light_path.location = (layout.shadow_light_path_x, layout.shadow_light_path_y)

        texture_nodes = [node for node in nodes if node.bl_idname == "ShaderNodeTexImage"]
        texture_nodes.sort(key=lambda node: (float(node.location.y), node.name), reverse=True)
        next_y = layout.texture_start_y
        for node in texture_nodes:
            node.location = (layout.texture_x, next_y)
            node.width = layout.texture_width
            next_y -= layout.texture_vertical_step

        palette_groups = [
            node
            for node in nodes
            if node.bl_idname == "ShaderNodeGroup"
            and node != primary_node
            and getattr(getattr(node, "node_tree", None), "name", "").startswith("StarBreaker Palette ")
        ]
        palette_groups.sort(key=lambda node: node.name)
        palette_y = 120.0
        for node in palette_groups:
            node.location = (layout.primary_x - 620.0, palette_y)
            node.width = 240.0
            palette_y -= 220.0

        layer_groups = [
            node
            for node in nodes
            if node.bl_idname == "ShaderNodeGroup"
            and node != primary_node
            and getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime LayerSurface"
        ]
        layer_groups.sort(key=lambda node: float(node.location.y), reverse=True)
        layer_y = 80.0
        for node in layer_groups:
            node.location = (layout.primary_x - 300.0, layer_y)
            node.width = 320.0
            layer_y -= 240.0

    def _primary_surface_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        output: bpy.types.Node | None,
    ) -> bpy.types.Node | None:
        if output is None:
            return None

        surface_input = _input_socket(output, "Surface")
        if surface_input is None or not surface_input.is_linked:
            return None

        primary_node = next(
            (link.from_node for link in links if link.to_node == output and link.to_socket == surface_input),
            None,
        )
        if primary_node is None:
            return None

        if self._is_shadow_wrapper_mix(primary_node):
            shader_input = primary_node.inputs[2]
            if shader_input.is_linked:
                return shader_input.links[0].from_node

        return primary_node

    def _is_shadow_wrapper_mix(self, node: bpy.types.Node | None) -> bool:
        if node is None or node.bl_idname != "ShaderNodeMixShader":
            return False

        factor_input = node.inputs[0]
        transparent_input = node.inputs[1]
        shader_input = node.inputs[2]
        if not factor_input.is_linked or not transparent_input.is_linked or not shader_input.is_linked:
            return False

        return (
            factor_input.links[0].from_node.bl_idname == "ShaderNodeLightPath"
            and transparent_input.links[0].from_node.bl_idname == "ShaderNodeBsdfTransparent"
        )

    def _plan_casts_no_shadows(self, plan: Any, submaterial: SubmaterialRecord | None = None) -> bool:
        if getattr(plan, "template_key", "") in {"decal_stencil", "parallax_pom"}:
            return True
        return submaterial is not None and submaterial.shader_family == "MeshDecal"

    def _shadowless_surface_output(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        surface_shader: Any,
    ) -> Any:
        light_path = nodes.new("ShaderNodeLightPath")
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        mix = nodes.new("ShaderNodeMixShader")
        shadow_ray = _output_socket(light_path, "Is Shadow Ray")
        if shadow_ray is None:
            return surface_shader
        links.new(shadow_ray, mix.inputs[0])
        links.new(surface_shader, mix.inputs[1])
        links.new(transparent.outputs[0], mix.inputs[2])
        return mix.outputs[0]

    def _configure_material(self, material: bpy.types.Material, *, blend_method: str, shadow_method: str) -> None:
        if hasattr(material, "blend_method"):
            material.blend_method = blend_method
        if hasattr(material, "shadow_method"):
            material.shadow_method = shadow_method
        material.use_backface_culling = False

    def _apply_instance_metadata(
        self,
        objects: list[bpy.types.Object],
        record: SceneInstanceRecord,
        effective_palette_id: str | None,
    ) -> None:
        serialized = json.dumps(record.raw or {
            "entity_name": record.entity_name,
            "mesh_asset": record.mesh_asset,
            "material_sidecar": record.material_sidecar,
            "palette_id": record.palette_id,
        }, sort_keys=True)
        for obj in objects:
            obj[PROP_SCENE_PATH] = str(self.package.scene_path)
            obj[PROP_EXPORT_ROOT] = str(self.package.export_root)
            obj[PROP_PACKAGE_NAME] = self.package.package_name
            obj[PROP_ENTITY_NAME] = record.entity_name
            if record.mesh_asset is not None:
                obj[PROP_MESH_ASSET] = record.mesh_asset
            if record.material_sidecar is not None:
                obj[PROP_MATERIAL_SIDECAR] = record.material_sidecar
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
            obj[PROP_INSTANCE_JSON] = serialized

    def _create_package_root(self, palette_id: str | None = None) -> bpy.types.Object:
        package_root = bpy.data.objects.new(f"{PACKAGE_ROOT_PREFIX} {self.package.package_name}", None)
        package_root.empty_display_type = "ARROWS"
        package_root[PROP_PACKAGE_ROOT] = True
        package_root[PROP_SCENE_PATH] = str(self.package.scene_path)
        package_root[PROP_EXPORT_ROOT] = str(self.package.export_root)
        package_root[PROP_PACKAGE_NAME] = self.package.package_name
        package_root[PROP_PALETTE_ID] = palette_id or self.package.scene.root_entity.palette_id or ""
        self.collection.objects.link(package_root)
        return package_root

    def _ensure_collection(self, package_name: str) -> bpy.types.Collection:
        collection_name = f"StarBreaker {package_name}"
        collection = bpy.data.collections.get(collection_name)
        if collection is None:
            collection = bpy.data.collections.new(collection_name)
            self.context.scene.collection.children.link(collection)
        return collection

    def _ensure_template_collection(self) -> bpy.types.Collection:
        collection = bpy.data.collections.get(TEMPLATE_COLLECTION_NAME)
        if collection is None:
            collection = bpy.data.collections.new(TEMPLATE_COLLECTION_NAME)
            self.context.scene.collection.children.link(collection)
        collection.hide_viewport = True
        collection.hide_render = True
        return collection

    def _index_nodes(self, objects: list[bpy.types.Object]) -> dict[str, bpy.types.Object]:
        indexed: dict[str, bpy.types.Object] = {}
        for obj in objects:
            source_name = obj.get(PROP_SOURCE_NODE_NAME, obj.name)
            source_name_str = str(source_name)
            indexed[source_name_str] = obj
            indexed[_canonical_source_name(source_name_str)] = obj
        return indexed

    def _scene_root_parent(self, objects: list[bpy.types.Object]) -> bpy.types.Object | None:
        indexed = self._index_nodes(objects)
        return indexed.get("CryEngine_Z_up")

    def _clear_template_material_bindings(self, objects: list[bpy.types.Object]) -> None:
        seen_meshes: set[int] = set()
        for obj in objects:
            if obj.type != "MESH" or obj.data is None:
                continue
            pointer = obj.data.as_pointer()
            if pointer in seen_meshes:
                continue
            seen_meshes.add(pointer)
            materials = getattr(obj.data, "materials", None)
            if materials is None:
                continue
            slot_mapping = _imported_slot_mapping_from_materials(materials)
            if slot_mapping is not None:
                obj.data[PROP_IMPORTED_SLOT_MAP] = json.dumps(slot_mapping)
            for index in range(len(materials)):
                materials[index] = None

    def _purge_unused_materials(self, materials: list[bpy.types.Material]) -> None:
        for material in materials:
            if material.users == 0:
                bpy.data.materials.remove(material)

    def _root_objects(self, objects: list[bpy.types.Object]) -> list[bpy.types.Object]:
        imported_pointers = {obj.as_pointer() for obj in objects}
        return [obj for obj in objects if obj.parent is None or obj.parent.as_pointer() not in imported_pointers]

    def _link_color_output(self, output: Any, input_socket: Any) -> None:
        output.node.id_data.links.new(output, input_socket)

    def _ensure_palette_group(self, palette: PaletteRecord) -> bpy.types.ShaderNodeTree:
        group_name = _palette_group_name(self.package.package_name, palette.id)
        group_signature = _palette_group_signature(palette)
        group = bpy.data.node_groups.get(group_name)
        if group is None:
            group = bpy.data.node_groups.new(group_name, "ShaderNodeTree")

        existing_outputs = {
            item.name
            for item in group.interface.items_tree
            if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "OUTPUT"
        }
        existing_inputs = {
            item.name
            for item in group.interface.items_tree
            if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "INPUT"
        }

        expected_inputs: set[str] = set()
        expected_outputs = {
            "Decal Color",
            "Decal Alpha",
            "Palette Decal Color",
            "Palette Decal Alpha",
            "Primary",
            "Primary SpecColor",
            "Primary Glossiness",
            "Secondary",
            "Secondary SpecColor",
            "Secondary Glossiness",
            "Tertiary",
            "Tertiary SpecColor",
            "Tertiary Glossiness",
            "Glass Color",
        }
        if group.get("starbreaker_palette_signature") == group_signature and expected_inputs.issubset(existing_inputs) and expected_outputs.issubset(existing_outputs):
            return group

        channel_specs = (
            ("Primary", "primary", 240),
            ("Primary SpecColor", "primary", 120),
            ("Primary Glossiness", "primary", 0),
            ("Secondary", "secondary", -140),
            ("Secondary SpecColor", "secondary", -260),
            ("Secondary Glossiness", "secondary", -380),
            ("Tertiary", "tertiary", -520),
            ("Tertiary SpecColor", "tertiary", -640),
            ("Tertiary Glossiness", "tertiary", -760),
            ("Glass Color", "glass", -900),
        )
        for socket_name in ("Decal Color", "Decal Alpha", "Palette Decal Color", "Palette Decal Alpha"):
            if socket_name not in existing_outputs:
                group.interface.new_socket(
                    name=socket_name,
                    in_out="OUTPUT",
                    socket_type="NodeSocketFloat" if socket_name.endswith("Alpha") else "NodeSocketColor",
                )
        for socket_name, _channel_name, _y in channel_specs:
            if socket_name not in existing_outputs:
                socket_type = "NodeSocketFloat" if "Glossiness" in socket_name else "NodeSocketColor"
                group.interface.new_socket(name=socket_name, in_out="OUTPUT", socket_type=socket_type)

        group.nodes.clear()

        group_input = group.nodes.new("NodeGroupInput")
        group_input.location = (-900, -120)
        output = group.nodes.new("NodeGroupOutput")
        output.location = (520, -120)

        primary_color = (*_palette_decal_or_fallback(palette, "red", "primary"), 1.0)
        secondary_color = (*_palette_decal_or_fallback(palette, "green", "secondary"), 1.0)
        tertiary_color = (*_palette_decal_or_fallback(palette, "blue", "tertiary"), 1.0)

        palette_decal_node = self._image_node(group.nodes, palette_decal_texture(palette), x=-900, y=-520, is_color=True)
        if palette_decal_node is not None:
            palette_decal_rgb = group.nodes.new("ShaderNodeSeparateColor")
            palette_decal_rgb.location = (-680, -360)
            if hasattr(palette_decal_rgb, "mode"):
                palette_decal_rgb.mode = "RGB"
            group.links.new(palette_decal_node.outputs[0], palette_decal_rgb.inputs[0])

            palette_decal_hsv = group.nodes.new("ShaderNodeSeparateColor")
            palette_decal_hsv.location = (-680, -560)
            if hasattr(palette_decal_hsv, "mode"):
                palette_decal_hsv.mode = "HSV"
            group.links.new(palette_decal_node.outputs[0], palette_decal_hsv.inputs[0])

            palette_mix_red = group.nodes.new("ShaderNodeMixRGB")
            palette_mix_red.location = (-420, -360)
            palette_mix_red.blend_type = "MIX"
            palette_mix_red.inputs[1].default_value = (0.0, 0.0, 0.0, 1.0)
            palette_mix_red.inputs[2].default_value = primary_color
            group.links.new(_output_socket(palette_decal_rgb, "Red", "R"), palette_mix_red.inputs[0])

            palette_mix_green = group.nodes.new("ShaderNodeMixRGB")
            palette_mix_green.location = (-220, -360)
            palette_mix_green.blend_type = "MIX"
            palette_mix_green.inputs[2].default_value = secondary_color
            group.links.new(palette_mix_red.outputs[0], palette_mix_green.inputs[1])
            group.links.new(_output_socket(palette_decal_rgb, "Green", "G"), palette_mix_green.inputs[0])

            palette_mix_blue = group.nodes.new("ShaderNodeMixRGB")
            palette_mix_blue.location = (-20, -360)
            palette_mix_blue.blend_type = "MIX"
            palette_mix_blue.inputs[2].default_value = tertiary_color
            group.links.new(palette_mix_green.outputs[0], palette_mix_blue.inputs[1])
            group.links.new(_output_socket(palette_decal_rgb, "Blue", "B"), palette_mix_blue.inputs[0])

            palette_invert = group.nodes.new("ShaderNodeInvert")
            palette_invert.location = (-220, -560)
            group.links.new(_output_socket(palette_decal_hsv, "Value", "V", "Blue"), palette_invert.inputs[1])

            group.links.new(palette_mix_blue.outputs[0], output.inputs["Decal Color"])
            group.links.new(palette_invert.outputs[0], output.inputs["Decal Alpha"])
            group.links.new(palette_mix_blue.outputs[0], output.inputs["Palette Decal Color"])
            group.links.new(palette_invert.outputs[0], output.inputs["Palette Decal Alpha"])

        for socket_name, channel_name, y in channel_specs:
            if socket_name.endswith("SpecColor"):
                rgb = group.nodes.new("ShaderNodeRGB")
                rgb.location = (120, y)
                rgb.label = socket_name
                spec = palette_finish_specular(palette, channel_name) or (0.0, 0.0, 0.0)
                rgb.outputs[0].default_value = (*spec, 1.0)
                group.links.new(rgb.outputs[0], output.inputs[socket_name])
            elif socket_name.endswith("Glossiness"):
                value = group.nodes.new("ShaderNodeValue")
                value.location = (120, y)
                value.label = socket_name
                value.outputs[0].default_value = palette_finish_glossiness(palette, channel_name) or 0.0
                group.links.new(value.outputs[0], output.inputs[socket_name])
            else:
                rgb = group.nodes.new("ShaderNodeRGB")
                rgb.location = (120, y)
                rgb.label = socket_name
                rgb.outputs[0].default_value = (*palette_color(palette, channel_name), 1.0)
                group.links.new(rgb.outputs[0], output.inputs[socket_name])
        group["starbreaker_palette_signature"] = group_signature
        return group

    def _palette_group_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        palette: PaletteRecord,
        *,
        x: int,
        y: int,
    ) -> bpy.types.Node:
        expected_name = f"STARBREAKER_PALETTE_{_safe_identifier(palette.id).upper()}"
        existing = next(
            (
                node
                for node in nodes
                if node.bl_idname == "ShaderNodeGroup"
                and getattr(node, "name", "") == expected_name
                and getattr(getattr(node, "node_tree", None), "name", "") == _palette_group_name(self.package.package_name, palette.id)
            ),
            None,
        )
        group_node = existing or nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_palette_group(palette)
        group_node.location = (x, y)
        group_node.label = f"StarBreaker Palette {palette.id}"
        group_node.name = expected_name
        return group_node

    def _palette_color_socket(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> Any:
        group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
        socket_name = {
            "primary": "Primary",
            "secondary": "Secondary",
            "tertiary": "Tertiary",
            "glass": "Glass Color",
        }.get(channel_name, "Primary")
        return _output_socket(group_node, socket_name)

    def _palette_specular_socket(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> Any:
        group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
        socket_name = {
            "primary": "Primary SpecColor",
            "secondary": "Secondary SpecColor",
            "tertiary": "Tertiary SpecColor",
        }.get(channel_name)
        return _output_socket(group_node, socket_name) if socket_name is not None else None

    def _palette_glossiness_socket(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> Any:
        group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
        socket_name = {
            "primary": "Primary Glossiness",
            "secondary": "Secondary Glossiness",
            "tertiary": "Tertiary Glossiness",
        }.get(channel_name)
        return _output_socket(group_node, socket_name) if socket_name is not None else None

    def _palette_decal_sockets(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        palette: PaletteRecord | None,
        channel_name: str | None,
        *,
        x: int,
        y: int,
    ) -> LayerSurfaceSockets:
        if palette is None:
            return LayerSurfaceSockets()
        group_node = self._palette_group_node(nodes, links, palette, x=x, y=y)
        return LayerSurfaceSockets(
            color=_output_socket(group_node, "Decal Color"),
            alpha=_output_socket(group_node, "Decal Alpha"),
        )


def _load_package_from_root(package_root: bpy.types.Object) -> PackageBundle:
    scene_path = _string_prop(package_root, PROP_SCENE_PATH)
    if scene_path is None:
        raise RuntimeError("Selected object is missing StarBreaker scene metadata")
    return PackageBundle.load(scene_path)


def _scene_instance_from_object(obj: bpy.types.Object) -> SceneInstanceRecord | None:
    payload = obj.get(PROP_INSTANCE_JSON)
    if not isinstance(payload, str):
        return None
    try:
        return SceneInstanceRecord.from_value(json.loads(payload))
    except (json.JSONDecodeError, ValueError, TypeError):
        return None


def _iter_package_objects(package_root: bpy.types.Object) -> list[bpy.types.Object]:
    return [package_root, *package_root.children_recursive]


def _string_prop(obj: bpy.types.ID, name: str) -> str | None:
    value = obj.get(name)
    if isinstance(value, str) and value:
        return value
    return None


def _mean_triplet(value: tuple[float, float, float] | None) -> float | None:
    if value is None:
        return None
    return sum(value) / 3.0


def _palette_finish_or_color(
    palette: PaletteRecord | None,
    channel_name: str,
) -> tuple[float, float, float]:
    if palette is None:
        return (0.0, 0.0, 0.0)
    return palette_finish_specular(palette, channel_name) or palette_color(palette, channel_name)


def _palette_decal_or_fallback(
    palette: PaletteRecord | None,
    decal_channel: str,
    fallback_channel: str,
) -> tuple[float, float, float]:
    return palette_decal_color(palette, decal_channel) or palette_color(palette, fallback_channel)


def _palette_angle_shift_strength(palette: PaletteRecord | None) -> float:
    if palette is None:
        return 0.0
    primary = palette_color(palette, "primary")
    facing = _palette_finish_or_color(palette, "secondary")
    grazing = _palette_finish_or_color(palette, "tertiary")
    primary_luma = _mean_triplet(primary) or 0.0
    facing_chroma = max(facing) - min(facing)
    grazing_chroma = max(grazing) - min(grazing)
    color_distance = math.sqrt(
        (facing[0] - grazing[0]) ** 2
        + (facing[1] - grazing[1]) ** 2
        + (facing[2] - grazing[2]) ** 2
    )
    if primary_luma > 0.08:
        return 0.0
    if max(facing_chroma, grazing_chroma) < 0.18 or color_distance < 0.25:
        return 0.0
    return max(0.0, min(0.65, 0.32 + color_distance * 0.4))


def _palette_group_signature(palette: PaletteRecord) -> str:
    payload = {
        'schema': 'palette_group_v1',
        'id': palette.id,
        'primary': palette_color(palette, 'primary'),
        'secondary': palette_color(palette, 'secondary'),
        'tertiary': palette_color(palette, 'tertiary'),
        'glass': palette_color(palette, 'glass'),
        'decal_red': palette_decal_color(palette, 'red'),
        'decal_green': palette_decal_color(palette, 'green'),
        'decal_blue': palette_decal_color(palette, 'blue'),
        'decal_texture': palette_decal_texture(palette),
        'primary_spec': palette_finish_specular(palette, 'primary'),
        'secondary_spec': palette_finish_specular(palette, 'secondary'),
        'tertiary_spec': palette_finish_specular(palette, 'tertiary'),
        'primary_gloss': palette_finish_glossiness(palette, 'primary'),
        'secondary_gloss': palette_finish_glossiness(palette, 'secondary'),
        'tertiary_gloss': palette_finish_glossiness(palette, 'tertiary'),
    }
    return hashlib.sha1(json.dumps(payload, sort_keys=True).encode('utf-8')).hexdigest()


def _resolved_layer_palette_diffuse_channel(
    submaterial: SubmaterialRecord,
    channel_name: str | None,
    palette: PaletteRecord | None,
) -> str | None:
    if channel_name in (None, "primary") or palette is None:
        return channel_name
    if submaterial.shader_family != "HardSurface":
        return channel_name
    if not bool(submaterial.variant_membership.get("layered")) or not bool(submaterial.variant_membership.get("palette_routed")):
        return channel_name
    if submaterial.decoded_feature_flags.has_iridescence:
        return channel_name
    primary_luma = _mean_triplet(palette_color(palette, "primary")) or 0.0
    finish = palette_finish_specular(palette, channel_name)
    if primary_luma > 0.03 or finish is None:
        return channel_name
    finish_chroma = max(finish) - min(finish)
    if finish_chroma < 0.12:
        return channel_name
    return "primary"


def _hard_surface_angle_shift_enabled(submaterial: SubmaterialRecord) -> bool:
    if submaterial.decoded_feature_flags.has_iridescence:
        return True
    strength = _optional_float_public_param(submaterial, "IridescenceStrength")
    if strength is None or strength <= 0.0:
        return False
    thickness_u = _optional_float_public_param(submaterial, "IridescenceThicknessU")
    thickness_v = _optional_float_public_param(submaterial, "IridescenceThicknessV")
    has_thickness = (thickness_u is not None and thickness_u > 0.0) or (thickness_v is not None and thickness_v > 0.0)
    has_support_texture = any(texture.slot == "TexSlot10" and bool(texture.export_path) for texture in submaterial.texture_slots)
    return has_thickness or has_support_texture


def _hard_surface_supports_palette_angle_shift(submaterial: SubmaterialRecord) -> bool:
    if submaterial.shader_family != "HardSurface":
        return False
    if not bool(submaterial.variant_membership.get("layered")) or not bool(submaterial.variant_membership.get("palette_routed")):
        return False
    if submaterial.decoded_feature_flags.has_iridescence:
        return False
    if submaterial.texture_slots:
        return False
    material_channel = getattr(submaterial.palette_routing, "material_channel", None)
    if material_channel is not None and material_channel.name != "primary":
        return True
    return len(submaterial.layer_manifest) > 1


def _triplet_from_value(value: Any) -> tuple[float, float, float] | None:
    if not isinstance(value, (list, tuple)) or len(value) < 3:
        return None
    try:
        return (float(value[0]), float(value[1]), float(value[2]))
    except (TypeError, ValueError):
        return None


def _triplet_from_string(value: Any) -> tuple[float, float, float] | None:
    if not isinstance(value, str):
        return None
    parts = [part.strip() for part in value.split(",")]
    if len(parts) < 3:
        return None
    try:
        return (float(parts[0]), float(parts[1]), float(parts[2]))
    except (TypeError, ValueError):
        return None


def _triplet_from_any(value: Any) -> tuple[float, float, float] | None:
    return _triplet_from_value(value) or _triplet_from_string(value)


def _optional_float_public_param(submaterial: SubmaterialRecord, *names: str) -> float | None:
    for name in names:
        value = submaterial.public_params.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return None


def _authored_attribute_string(submaterial: SubmaterialRecord, *names: str) -> str | None:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        value = attribute.get("value")
        if isinstance(value, str) and value:
            return value
    return None


def _uses_virtual_tint_palette_decal(submaterial: SubmaterialRecord) -> bool:
    texture = _submaterial_texture_reference(submaterial, slots=("TexSlot7",), roles=("tint_palette_decal",))
    return texture is not None and bool(texture.is_virtual)


def _is_virtual_tint_palette_stencil_decal(submaterial: SubmaterialRecord) -> bool:
    if submaterial.shader_family != "MeshDecal" or not _uses_virtual_tint_palette_decal(submaterial):
        return False
    string_gen_mask = (_authored_attribute_string(submaterial, "StringGenMask") or "").upper()
    if "STENCIL_MAP" in string_gen_mask:
        return True
    if any(
        name in submaterial.public_params
        for name in ("StencilOpacity", "StencilDiffuseBreakup", "StencilTiling", "StencilTintOverride")
    ):
        return True
    lowered_name = (submaterial.submaterial_name or "").lower()
    return "_stencil" in lowered_name or "branding" in lowered_name


def _routes_virtual_tint_palette_decal_to_decal_source(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot1" and (contract_input.semantic or contract_input.name).lower() == "decal_source"


def _suppresses_virtual_tint_palette_stencil_input(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot7" and (contract_input.semantic or contract_input.name).lower() in {
        "stencil_source",
        "stencil_source_alpha",
    }


def _routes_virtual_tint_palette_decal_alpha_to_decal_source(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot1" and (contract_input.semantic or contract_input.name).lower() == "decal_source_alpha"


def _public_param_triplet(submaterial: SubmaterialRecord, *names: str) -> tuple[float, float, float] | None:
    for name in names:
        triplet = _triplet_from_any(submaterial.public_params.get(name))
        if triplet is not None:
            return triplet
    return None


def _authored_attribute_triplet(submaterial: SubmaterialRecord, *names: str) -> tuple[float, float, float] | None:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        triplet = _triplet_from_any(attribute.get("value"))
        if triplet is not None:
            return triplet
    return None


def _resolved_submaterial_palette_color(
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
) -> tuple[float, float, float] | None:
    if palette is None:
        return None
    channel = submaterial.palette_routing.material_channel
    if channel is not None:
        return palette_color(palette, channel.name)
    if submaterial.shader_family == "GlassPBR":
        return palette_color(palette, "glass")
    return None


def _matching_texture_reference(
    textures: list[TextureReference],
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    for texture in textures:
        if slots and texture.slot not in slots:
            continue
        if roles and texture.role not in roles:
            continue
        if alpha_semantic is not None and texture.alpha_semantic != alpha_semantic:
            continue
        if texture.export_path:
            return texture
    for texture in textures:
        if slots and texture.slot not in slots:
            continue
        if roles and texture.role not in roles:
            continue
        if alpha_semantic is not None and texture.alpha_semantic != alpha_semantic:
            continue
        return texture
    return None


def _submaterial_texture_reference(
    submaterial: SubmaterialRecord,
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    return _matching_texture_reference(
        [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures],
        slots=slots,
        roles=roles,
        alpha_semantic=alpha_semantic,
    )


def _layer_texture_reference(
    layer: LayerManifestEntry,
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    return _matching_texture_reference(layer.texture_slots, slots=slots, roles=roles, alpha_semantic=alpha_semantic)


def _float_layer_public_param(layer: LayerManifestEntry, *names: str) -> float:
    wanted = set(names)
    for param in layer.resolved_material.get("authored_public_params", []):
        if param.get("name") not in wanted:
            continue
        try:
            return float(param.get("value"))
        except (TypeError, ValueError):
            continue
    return 0.0


def _layer_snapshot_triplet(layer: LayerManifestEntry, name: str) -> tuple[float, float, float] | None:
    return _triplet_from_value(layer.layer_snapshot.get(name))


def _float_public_param(submaterial: SubmaterialRecord, *names: str) -> float:
    for name in names:
        value = submaterial.public_params.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return 0.0


def _float_authored_attribute(submaterial: SubmaterialRecord, *names: str) -> float:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        value = attribute.get("value")
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return 0.0


def _material_identity(
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
) -> str:
    payload = {
        "schema": MATERIAL_IDENTITY_SCHEMA,
        "material_sidecar": _canonical_material_sidecar_path(sidecar_path, sidecar),
        "submaterial": submaterial.raw,
    }
    palette_signature = palette_signature_for_submaterial(submaterial, palette)
    if palette_signature is not None:
        payload["palette_channels"] = palette_signature
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.blake2s(encoded, digest_size=16).hexdigest()


def _material_name(
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    material_identity: str,
) -> str:
    preferred_name = submaterial.blender_material_name or _derived_material_name(sidecar_path, sidecar, submaterial)
    existing = bpy.data.materials.get(preferred_name)
    if existing is None:
        return preferred_name

    existing_identity = existing.get(PROP_MATERIAL_IDENTITY)
    if isinstance(existing_identity, str) and existing_identity == material_identity:
        return preferred_name
    return f"{preferred_name}#{material_identity[:8]}"


def _canonical_material_sidecar_path(sidecar_path: str, sidecar: MaterialSidecar) -> str:
    return sidecar.normalized_export_relative_path or sidecar_path or sidecar.source_material_path or "material"


def _material_is_compatible(
    material: bpy.types.Material,
    package: PackageBundle,
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
) -> bool:
    existing_sidecar_path = _string_prop(material, PROP_MATERIAL_SIDECAR)
    canonical_sidecar_path = _canonical_material_sidecar_path(sidecar_path, sidecar)
    if existing_sidecar_path is None or existing_sidecar_path not in {sidecar_path, canonical_sidecar_path}:
        return False

    payload = material.get(PROP_SUBMATERIAL_JSON)
    if not isinstance(payload, str):
        return False
    try:
        existing_submaterial = json.loads(payload)
    except json.JSONDecodeError:
        return False
    if existing_submaterial != submaterial.raw:
        return False

    if not _managed_material_runtime_graph_is_sane(material):
        return False

    existing_palette = palette_for_id(package, _string_prop(material, PROP_PALETTE_ID))
    return palette_signature_for_submaterial(submaterial, existing_palette) == palette_signature_for_submaterial(
        submaterial,
        palette,
    )


def _managed_material_runtime_graph_is_sane(material: bpy.types.Material) -> bool:
    node_tree = material.node_tree
    if node_tree is None:
        return False

    hard_surface_nodes = [
        node
        for node in node_tree.nodes
        if node.bl_idname == "ShaderNodeGroup"
        and getattr(getattr(node, "node_tree", None), "name", "") == "StarBreaker Runtime HardSurface"
    ]
    if not hard_surface_nodes:
        return True

    for node in hard_surface_nodes:
        linked_inputs = {
            link.to_socket.name
            for link in node_tree.links
            if link.to_node == node
        }
        if not {
            "Primary Color",
            "Primary Alpha",
            "Primary Roughness",
            "Secondary Color",
            "Secondary Alpha",
            "Secondary Roughness",
        }.issubset(linked_inputs):
            return False
        if not any(
            link.from_node == node
            and link.from_socket.name == "Shader"
            and link.to_node.bl_idname == "ShaderNodeOutputMaterial"
            and link.to_socket.name == "Surface"
            for link in node_tree.links
        ):
            return False

    return True


def _derived_material_name(sidecar_path: str, sidecar: MaterialSidecar, submaterial: SubmaterialRecord) -> str:
    normalized_path = sidecar.normalized_export_relative_path or sidecar_path or sidecar.source_material_path or "material"
    sidecar_name = Path(normalized_path).name
    if sidecar_name.endswith(".materials.json"):
        sidecar_name = sidecar_name[: -len(".materials.json")]
    elif sidecar_name.endswith(".json"):
        sidecar_name = sidecar_name[: -len(".json")]

    submaterial_name = submaterial.submaterial_name or f"slot_{submaterial.index}"
    return f"{sidecar_name}:{submaterial_name}"


def _safe_identifier(value: str) -> str:
    safe = "".join(character if character.isalnum() else "_" for character in value)
    return safe.strip("_") or "value"


def _palette_group_name(package_name: str, palette_id: str) -> str:
    return f"StarBreaker Palette {package_name} {_safe_identifier(palette_id)}"


def _canonical_source_name(name: str) -> str:
    if len(name) > 4 and name[-4] == "." and name[-3:].isdigit():
        return name[:-4]
    return name


def _scene_position_to_blender(position: tuple[float, float, float]) -> tuple[float, float, float]:
    return (position[0], -position[2], position[1])


def _scene_matrix_to_blender(matrix_rows: Any) -> Matrix:
    matrix = Matrix(matrix_rows).transposed()
    return SCENE_AXIS_CONVERSION @ matrix @ SCENE_AXIS_CONVERSION_INV


def _scene_quaternion_to_blender(rotation: tuple[float, float, float, float]) -> Quaternion:
    if all(abs(component) <= 1e-8 for component in rotation):
        return Quaternion((1.0, 0.0, 0.0, 0.0))
    matrix = Quaternion(rotation).to_matrix().to_4x4()
    return (SCENE_AXIS_CONVERSION @ matrix @ SCENE_AXIS_CONVERSION_INV).to_quaternion().normalized()


def _scene_light_quaternion_to_blender(rotation: tuple[float, float, float, float]) -> Quaternion:
    return (_scene_quaternion_to_blender(rotation) @ GLTF_LIGHT_BASIS_CORRECTION).normalized()


def _blender_light_type(light: Any) -> str:
    light_type = str(getattr(light, "light_type", "") or "").strip().lower()
    if light_type in {"directional", "sun"}:
        return "SUN"
    if light_type in {"projector", "spot"}:
        return "SPOT"
    if light_type in {"omni", "point"}:
        return "POINT"
    if (light.inner_angle or 0.0) > 0.0 or (light.outer_angle or 0.0) > 0.0:
        return "SPOT"
    return "POINT"


def _light_energy_to_blender(intensity: float, blender_light_type: str) -> float:
    intensity = max(float(intensity), 0.0)
    if blender_light_type == "SUN":
        return intensity / GLTF_PBR_WATTS_TO_LUMENS
    return intensity * 4.0 * math.pi / GLTF_PBR_WATTS_TO_LUMENS


def _is_axis_conversion_root(obj: bpy.types.Object) -> bool:
    source_name = _canonical_source_name(str(obj.get(PROP_SOURCE_NODE_NAME, obj.name) or ""))
    return obj.data is None and source_name == "CryEngine_Z_up"


def _should_neutralize_axis_root(obj: bpy.types.Object, mesh_asset: str) -> bool:
    return _is_axis_conversion_root(obj) and _has_non_identity_descendants(obj, mesh_asset)


def _has_non_identity_descendants(root: bpy.types.Object, mesh_asset: str) -> bool:
    stack = list(root.children)
    while stack:
        current = stack.pop()
        if current.get(PROP_TEMPLATE_PATH) != mesh_asset:
            continue
        if not _matrix_is_identityish(current.matrix_local):
            return True
        stack.extend(current.children)
    return False


def _matrix_is_identityish(matrix: Matrix, epsilon: float = 1e-4) -> bool:
    identity = Matrix.Identity(4)
    for row_index in range(4):
        for column_index in range(4):
            if abs(matrix[row_index][column_index] - identity[row_index][column_index]) > epsilon:
                return False
    return True


def _slot_mapping_for_object(obj: bpy.types.Object) -> list[int | None] | None:
    data = getattr(obj, "data", None)
    if data is None:
        return None
    mapping_raw = data.get(PROP_IMPORTED_SLOT_MAP)
    if not isinstance(mapping_raw, str) or not mapping_raw:
        return None
    try:
        parsed = json.loads(mapping_raw)
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, list):
        return None
    mapping: list[int | None] = []
    for value in parsed:
        if value is None:
            mapping.append(None)
            continue
        try:
            mapping.append(int(value))
        except (TypeError, ValueError):
            mapping.append(None)
    return mapping


def _imported_slot_mapping_from_materials(materials: Any) -> list[int | None] | None:
    mapping: list[int | None] = []
    has_explicit_mapping = False
    for material in materials:
        submaterial_index = _imported_submaterial_index(material)
        if submaterial_index is not None:
            has_explicit_mapping = True
        mapping.append(submaterial_index)
    if not has_explicit_mapping:
        return None
    return mapping


def _imported_submaterial_index(material: bpy.types.Material | None) -> int | None:
    if material is None:
        return None
    semantic = material.get("semantic")
    if hasattr(semantic, "to_dict"):
        semantic = semantic.to_dict()
    if not isinstance(semantic, dict):
        return None
    material_set_identity = semantic.get("material_set_identity")
    if hasattr(material_set_identity, "to_dict"):
        material_set_identity = material_set_identity.to_dict()
    if not isinstance(material_set_identity, dict):
        return None
    submaterial_index = material_set_identity.get("submaterial_index")
    try:
        return int(submaterial_index)
    except (TypeError, ValueError):
        return None


def _input_socket(node: Any, *names: str) -> Any:
    for name in names:
        socket = node.inputs.get(name)
        if socket is not None:
            return socket
    if getattr(node, "bl_idname", "") == "ShaderNodeGroup":
        node_tree = getattr(node, "node_tree", None)
        if node_tree is not None:
            try:
                node.node_tree = node_tree
            except Exception:
                return None
            for name in names:
                socket = node.inputs.get(name)
                if socket is not None:
                    return socket
            try:
                bpy.context.view_layer.update()
            except Exception:
                return None
            try:
                node.node_tree = node_tree
            except Exception:
                return None
            for name in names:
                socket = node.inputs.get(name)
                if socket is not None:
                    return socket
    return None


def _output_socket(node: Any, *names: str) -> Any:
    for name in names:
        socket = node.outputs.get(name)
        if socket is not None:
            return socket
    if getattr(node, "bl_idname", "") == "ShaderNodeGroup":
        node_tree = getattr(node, "node_tree", None)
        if node_tree is not None:
            try:
                node.node_tree = node_tree
            except Exception:
                return None
            for name in names:
                socket = node.outputs.get(name)
                if socket is not None:
                    return socket
            try:
                bpy.context.view_layer.update()
            except Exception:
                return None
            try:
                node.node_tree = node_tree
            except Exception:
                return None
            for name in names:
                socket = node.outputs.get(name)
                if socket is not None:
                    return socket
    return None


def _refresh_group_node_sockets(node: Any) -> None:
    if getattr(node, "bl_idname", "") != "ShaderNodeGroup":
        return
    node_tree = getattr(node, "node_tree", None)
    if node_tree is None:
        return
    try:
        node.node_tree = node_tree
        bpy.context.view_layer.update()
    except Exception:
        return


def _contract_input_uses_color(contract_input: ContractInput) -> bool:
    semantic = (contract_input.semantic or contract_input.name).lower()
    return not any(keyword in semantic for keyword in NON_COLOR_INPUT_KEYWORDS)
