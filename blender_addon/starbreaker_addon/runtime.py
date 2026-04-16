from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
from typing import Any

import bpy
from mathutils import Euler, Matrix

from .manifest import MaterialSidecar, PackageBundle, PaletteRecord, SceneInstanceRecord, SubmaterialRecord
from .palette import palette_color, palette_for_id, palette_id_for_livery_instance
from .templates import (
    has_virtual_input,
    material_palette_channels,
    representative_textures,
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
PROP_TEMPLATE_PATH = "starbreaker_template_path"
PROP_SOURCE_NODE_NAME = "starbreaker_source_node_name"
PROP_MISSING_ASSET = "starbreaker_missing_asset"

PACKAGE_ROOT_PREFIX = "StarBreaker"
TEMPLATE_COLLECTION_NAME = "StarBreaker Template Cache"


@dataclass(frozen=True)
class ImportedTemplate:
    mesh_asset: str
    root_names: list[str]


def import_package(context: bpy.types.Context, scene_path: str | Path, prefer_cycles: bool = True) -> bpy.types.Object:
    package = PackageBundle.load(scene_path)
    importer = PackageImporter(context, package)
    return importer.import_scene(prefer_cycles=prefer_cycles)


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
    applied = 0
    for obj in _iter_package_objects(package_root):
        applied += importer.rebuild_object_materials(obj, palette_id)
    package_root[PROP_PALETTE_ID] = palette_id
    return applied


def apply_livery_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, livery_id: str) -> int:
    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root)
    applied = 0
    for obj in _iter_package_objects(package_root):
        instance = _scene_instance_from_object(obj)
        if instance is None:
            continue
        resolved_palette_id = palette_id_for_livery_instance(
            package,
            livery_id,
            instance,
            _string_prop(obj, PROP_MATERIAL_SIDECAR),
        )
        applied += importer.rebuild_object_materials(obj, resolved_palette_id)
        if resolved_palette_id is not None:
            obj[PROP_PALETTE_ID] = resolved_palette_id
    package_root[PROP_PALETTE_ID] = livery_id
    return applied


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

    def import_scene(self, prefer_cycles: bool = True) -> bpy.types.Object:
        if prefer_cycles and hasattr(self.context.scene.render, "engine"):
            self.context.scene.render.engine = "CYCLES"

        package_root = self.package_root or self._create_package_root()
        self.package_root = package_root

        root_anchor, root_nodes = self.instantiate_scene_instance(self.package.scene.root_entity, parent=package_root)
        self.node_index_by_entity_name[self.package.scene.root_entity.entity_name] = self._index_nodes(root_nodes)
        root_anchor.parent = package_root

        for child in self.package.scene.children:
            parent_node = None
            if child.parent_entity_name:
                parent_node = self.node_index_by_entity_name.get(child.parent_entity_name, {}).get(child.parent_node_name or "")
            anchor, child_nodes = self.instantiate_scene_instance(child, parent=package_root, parent_node=parent_node)
            self.node_index_by_entity_name.setdefault(child.entity_name, {}).update(self._index_nodes(child_nodes))
            if parent_node is None:
                anchor.parent = package_root

        for interior in self.package.scene.interiors:
            self.import_interior_container(interior, package_root)

        return package_root

    def rebuild_object_materials(self, obj: bpy.types.Object, palette_id: str | None) -> int:
        if obj.type != "MESH":
            return 0
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        if sidecar_path is None:
            return 0
        sidecar = self.package.load_material_sidecar(sidecar_path)
        if sidecar is None:
            return 0
        palette = palette_for_id(self.package, palette_id)
        applied = 0
        mesh_materials = getattr(obj.data, "materials", None)
        for submaterial in sidecar.submaterials:
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
        if palette_id is not None:
            obj[PROP_PALETTE_ID] = palette_id
        return applied

    def instantiate_scene_instance(
        self,
        record: SceneInstanceRecord,
        parent: bpy.types.Object,
        parent_node: bpy.types.Object | None = None,
    ) -> tuple[bpy.types.Object, list[bpy.types.Object]]:
        anchor = bpy.data.objects.new(record.entity_name, None)
        anchor.empty_display_type = "PLAIN_AXES"
        self.collection.objects.link(anchor)

        target_parent = parent_node or parent
        anchor.parent = target_parent
        anchor.rotation_mode = "QUATERNION"
        if parent_node is not None:
            anchor.location = record.offset_position
            desired_rotation = Euler(tuple(math.radians(value) for value in record.offset_rotation), "XYZ").to_quaternion()
            if record.no_rotation:
                anchor.rotation_quaternion = parent_node.matrix_world.to_quaternion().inverted() @ desired_rotation
            else:
                anchor.rotation_quaternion = desired_rotation

        try:
            template = self.ensure_template(record.mesh_asset)
        except RuntimeError:
            anchor.empty_display_type = "SPHERE"
            if record.mesh_asset is not None:
                anchor[PROP_MISSING_ASSET] = record.mesh_asset
            self._apply_instance_metadata([anchor], record)
            return anchor, [anchor]

        clones = self.instantiate_template(template, anchor)
        self._apply_instance_metadata([anchor, *clones], record)

        for clone in clones:
            self.rebuild_object_materials(clone, record.palette_id)
        return anchor, clones

    def import_interior_container(self, interior: Any, package_root: bpy.types.Object) -> bpy.types.Object:
        anchor = bpy.data.objects.new(interior.name, None)
        anchor.empty_display_type = "CUBE"
        anchor.parent = package_root
        anchor.matrix_local = Matrix(interior.container_transform)
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
            placement_anchor = bpy.data.objects.new(instance.entity_name, None)
            placement_anchor.parent = anchor
            placement_anchor.matrix_local = Matrix(placement.transform)
            self.collection.objects.link(placement_anchor)

            try:
                template = self.ensure_template(instance.mesh_asset)
            except RuntimeError:
                placement_anchor.empty_display_type = "SPHERE"
                if instance.mesh_asset is not None:
                    placement_anchor[PROP_MISSING_ASSET] = instance.mesh_asset
                self._apply_instance_metadata([placement_anchor], instance)
                continue

            clones = self.instantiate_template(template, placement_anchor)
            self._apply_instance_metadata([placement_anchor, *clones], instance)
            for clone in clones:
                self.rebuild_object_materials(clone, instance.palette_id)

        for light in interior.lights:
            self.create_light(light, anchor)

        return anchor

    def create_light(self, light: Any, parent: bpy.types.Object) -> bpy.types.Object:
        light_data = bpy.data.lights.new(name=light.name or "StarBreaker Light", type="SPOT")
        light_data.energy = light.intensity
        light_data.color = light.color
        if hasattr(light_data, "cutoff_distance"):
            light_data.cutoff_distance = light.radius
        if hasattr(light_data, "spot_size"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            light_data.spot_size = math.radians(outer_angle)
        if hasattr(light_data, "spot_blend"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            inner_angle = min(light.inner_angle or 0.0, outer_angle)
            inner_ratio = min(max(inner_angle / outer_angle, 0.0), 1.0)
            light_data.spot_blend = 1.0 - inner_ratio

        light_object = bpy.data.objects.new(light.name or "StarBreaker Light", light_data)
        light_object.parent = parent
        light_object.location = light.position
        light_object.rotation_mode = "QUATERNION"
        light_object.rotation_quaternion = light.rotation
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
            obj[PROP_SOURCE_NODE_NAME] = obj.name

        self._clear_template_material_bindings(imported)
        self._purge_unused_materials(imported_materials)

        template = ImportedTemplate(mesh_asset=mesh_asset, root_names=[obj.name for obj in root_objects])
        self.template_cache[mesh_asset] = template
        return template

    def instantiate_template(self, template: ImportedTemplate, anchor: bpy.types.Object) -> list[bpy.types.Object]:
        clones: list[bpy.types.Object] = []
        mapping: dict[str, bpy.types.Object] = {}
        for root_name in template.root_names:
            source = bpy.data.objects.get(root_name)
            if source is None:
                continue
            clone = self._duplicate_object_tree(source, template.mesh_asset, mapping)
            clone.parent = anchor
            clones.append(clone)
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
        palette_key = palette.id if palette is not None else "none"
        cache_key = _material_identity(sidecar, submaterial, palette_key)
        cached = self.material_cache.get(cache_key)
        if cached is not None:
            return cached

        material_name = _material_name(cache_key, submaterial.submaterial_name, palette_key)
        material = bpy.data.materials.new(material_name)
        material.use_nodes = True
        plan = template_plan_for_submaterial(submaterial)

        if plan.template_key == "nodraw":
            self._build_nodraw_material(material)
        elif plan.template_key == "screen_hud":
            self._build_screen_material(material, submaterial, palette, plan)
        elif plan.template_key == "effects":
            self._build_effect_material(material, submaterial, palette, plan)
        else:
            self._build_principled_material(material, submaterial, palette, plan)

        material[PROP_SHADER_FAMILY] = submaterial.shader_family
        material[PROP_TEMPLATE_KEY] = plan.template_key
        material[PROP_PALETTE_ID] = palette_key
        material[PROP_MATERIAL_SIDECAR] = sidecar_path
        material[PROP_SUBMATERIAL_JSON] = json.dumps(submaterial.raw, sort_keys=True)

        self.material_cache[cache_key] = material
        return material

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
        principled = nodes.new("ShaderNodeBsdfPrincipled")
        principled.location = (420, 0)
        links.new(principled.outputs[0], output.inputs[0])

        textures = representative_textures(submaterial)
        base_socket = self._color_source_socket(nodes, submaterial, palette, textures["base_color"], x=40, y=140)
        if base_socket is not None:
            links.new(base_socket, _input_socket(principled, "Base Color"))
        elif palette is not None and plan.uses_palette:
            primary = self._palette_rgb_node(nodes, palette, "primary", x=100, y=120)
            links.new(primary.outputs[0], _input_socket(principled, "Base Color"))

        roughness_socket = _input_socket(principled, "Roughness")
        roughness_node = self._image_node(nodes, textures["roughness"], x=80, y=-120, is_color=False)
        if roughness_socket is not None:
            if roughness_node is not None:
                links.new(roughness_node.outputs[0], roughness_socket)
            else:
                roughness_socket.default_value = 0.45 if submaterial.shader_family != "GlassPBR" else 0.08

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
                    emissive = self._palette_rgb_node(nodes, palette, "primary", x=100, y=300)
                    links.new(emissive.outputs[0], emission_color)
            emission_strength = _input_socket(principled, "Emission Strength")
            if emission_strength is not None:
                emission_strength.default_value = 2.0

        if plan.template_key == "biological":
            subsurface = _input_socket(principled, "Subsurface Weight", "Subsurface")
            if subsurface is not None:
                subsurface.default_value = 0.15

        if plan.template_key == "hair":
            alpha_socket = _input_socket(principled, "Alpha")
            if alpha_socket is not None:
                alpha_socket.default_value = 0.85
            anisotropic = _input_socket(principled, "Anisotropic")
            if anisotropic is not None:
                anisotropic.default_value = 0.4

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

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

        palette_node = self._palette_rgb_node(nodes, palette, active_channel.name, x=x, y=y - 140)
        if image_node is None:
            return palette_node.outputs[0]

        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x + 180, y)
        mix.blend_type = "MULTIPLY"
        mix.inputs[0].default_value = 1.0
        mix.inputs[1].default_value = (1.0, 1.0, 1.0, 1.0)
        self._link_color_output(image_node.outputs[0], mix.inputs[1])
        self._link_color_output(palette_node.outputs[0], mix.inputs[2])
        return mix.outputs[0]

    def _palette_rgb_node(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> bpy.types.ShaderNodeRGB:
        node = nodes.new("ShaderNodeRGB")
        node.location = (x, y)
        node.label = f"StarBreaker Palette {channel_name}"
        node.name = f"STARBREAKER_PALETTE_{channel_name.upper()}"
        node.outputs[0].default_value = (*palette_color(palette, channel_name), 1.0)
        return node

    def _image_node(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
    ) -> bpy.types.ShaderNodeTexImage | None:
        resolved = self.package.resolve_path(image_path)
        if resolved is None or not resolved.is_file():
            return None
        node = nodes.new("ShaderNodeTexImage")
        node.location = (x, y)
        node.image = bpy.data.images.load(str(resolved), check_existing=True)
        if not is_color and node.image is not None and hasattr(node.image, "colorspace_settings"):
            node.image.colorspace_settings.name = "Non-Color"
        return node

    def _configure_material(self, material: bpy.types.Material, *, blend_method: str, shadow_method: str) -> None:
        if hasattr(material, "blend_method"):
            material.blend_method = blend_method
        if hasattr(material, "shadow_method"):
            material.shadow_method = shadow_method
        material.use_backface_culling = False

    def _apply_instance_metadata(self, objects: list[bpy.types.Object], record: SceneInstanceRecord) -> None:
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
            if record.palette_id is not None:
                obj[PROP_PALETTE_ID] = record.palette_id
            obj[PROP_INSTANCE_JSON] = serialized

    def _create_package_root(self) -> bpy.types.Object:
        package_root = bpy.data.objects.new(f"{PACKAGE_ROOT_PREFIX} {self.package.package_name}", None)
        package_root.empty_display_type = "ARROWS"
        package_root[PROP_PACKAGE_ROOT] = True
        package_root[PROP_SCENE_PATH] = str(self.package.scene_path)
        package_root[PROP_EXPORT_ROOT] = str(self.package.export_root)
        package_root[PROP_PACKAGE_NAME] = self.package.package_name
        package_root[PROP_PALETTE_ID] = self.package.scene.root_entity.palette_id or ""
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
            indexed[str(source_name)] = obj
        return indexed

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


def _material_identity(sidecar: MaterialSidecar, submaterial: SubmaterialRecord, palette_key: str) -> str:
    payload = {
        "palette": palette_key,
        "source_material_path": sidecar.source_material_path,
        "submaterial": submaterial.raw,
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.blake2s(encoded, digest_size=16).hexdigest()


def _material_name(material_identity: str, submaterial_name: str, palette_key: str) -> str:
    safe_palette = palette_key.replace("/", "_")
    return f"SB::{submaterial_name}::{safe_palette}::{material_identity[:10]}"


def _input_socket(node: Any, *names: str) -> Any:
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
    return None
