from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
from typing import Any
import uuid

import bpy
from mathutils import Euler, Matrix, Quaternion

from ..manifest import LayerManifestEntry, MaterialSidecar, PackageBundle, PaletteRecord, SceneInstanceRecord, SubmaterialRecord, TextureReference
from ..material_contract import ContractInput, ShaderGroupContract, TemplateContract, bundled_template_library_path, load_bundled_template_contract
from ..palette import (
    palette_color,
    palette_decal_color,
    palette_decal_texture,
    palette_finish_glossiness,
    palette_finish_specular,
    palette_for_id,
    palette_id_for_livery_instance,
    resolved_palette_id,
)
from ..templates import (
    has_virtual_input,
    material_palette_channels,
    representative_textures,
    smoothness_texture_reference,
    template_plan_for_submaterial,
)

# Phase 7.2: constants migrated to runtime.constants; re-imported here so any
# code still referencing them in this file continues to work unchanged.
from .constants import (
    GLTF_LIGHT_BASIS_CORRECTION,
    GLTF_PBR_WATTS_TO_LUMENS,
    MATERIAL_IDENTITY_SCHEMA,
    NON_COLOR_INPUT_KEYWORDS,
    PACKAGE_ROOT_PREFIX,
    PROP_ENTITY_NAME,
    PROP_EXPORT_ROOT,
    PROP_IMPORTED_SLOT_MAP,
    PROP_INSTANCE_JSON,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_MESH_ASSET,
    PROP_MISSING_ASSET,
    PROP_PACKAGE_NAME,
    PROP_PACKAGE_ROOT,
    PROP_PAINT_VARIANT_SIDECAR,
    PROP_PALETTE_ID,
    PROP_PALETTE_SCOPE,
    PROP_SCENE_PATH,
    PROP_SHADER_FAMILY,
    PROP_SOURCE_NODE_NAME,
    PROP_SUBMATERIAL_JSON,
    PROP_SURFACE_SHADER_MODE,
    PROP_TEMPLATE_KEY,
    PROP_TEMPLATE_PATH,
    SCENE_AXIS_CONVERSION,
    SCENE_AXIS_CONVERSION_INV,
    SCENE_WEAR_STRENGTH_PROP,
    SURFACE_SHADER_MODE_GLASS,
    SURFACE_SHADER_MODE_PRINCIPLED,
    TEMPLATE_COLLECTION_NAME,
)


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
    metallic: Any | None = None


@dataclass
class StencilOverlaySockets:
    color: Any | None = None
    color_factor: Any | None = None
    factor: Any | None = None
    roughness: Any | None = None
    specular: Any | None = None
    specular_tint: Any | None = None


@dataclass(frozen=True)
class SocketRef:
    node: Any
    name: str
    is_output: bool = True


#: Allowed ``bl_idname`` values for nodes at the top level of a built material.
#:
#: Phase 6 of the blender-exporter plan constrains material top-level node
#: trees to the orchestration layer only: palette groups, image textures,
#: layer / shader helper groups, and the material output. Anything else
#: belongs inside an owned group. See ``docs/StarBreaker/todo.md`` Phase 6
#: for the authoritative definition.
# Phase 7.2: validator helpers migrated to runtime.validators.
from .validators import (
    MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES,
    _assert_material_top_level_clean,
    _material_top_level_violations,
    _purge_orphaned_runtime_groups,
)

# Phase 7.4: package-lifecycle functions migrated to runtime.package_ops.
from .package_ops import (
    _effective_exterior_material_sidecars,
    _exterior_material_sidecars,
    _iter_package_objects,
    _load_package_from_root,
    _paint_variant_for_palette_id,
    _scene_instance_from_object,
    _string_prop,
    _suspend_heavy_viewports,
    apply_livery_to_package_root,
    apply_livery_to_selected_package,
    apply_paint_to_package_root,
    apply_paint_to_selected_package,
    apply_palette_to_package_root,
    apply_palette_to_selected_package,
    dump_selected_metadata,
    exterior_palette_ids,
    find_package_root,
    import_package,
)



# Phase 7.5: group-builder methods migrated to runtime.importer.groups.
from .importer.groups import GroupsMixin

# Phase 7.5b: material-builder methods migrated to runtime.importer.builders.
from .importer.builders import BuildersMixin


class PackageImporter(BuildersMixin, GroupsMixin):
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
        self.exterior_material_sidecars = _exterior_material_sidecars(package)
        self.template_cache: dict[str, ImportedTemplate] = {}
        self.material_cache: dict[str, bpy.types.Material] = {}
        self.node_index_by_entity_name: dict[str, dict[str, bpy.types.Object]] = {}
        self.bundled_template_contract: TemplateContract | None = None
        self.import_palette_override: str | None = None
        self.import_paint_variant_sidecar: str | None = None
        self.runtime_shared_groups_ready = False
        self.material_identity_index: dict[str, bpy.types.Material] = {}
        self.material_identity_index_ready = False
        self.sidecar_submaterials_by_index: dict[str, dict[int, SubmaterialRecord]] = {}
        self.sidecar_submaterials_by_name: dict[str, dict[str, SubmaterialRecord]] = {}
        self.slot_mapping_cache: dict[int, list[int | None] | None] = {}

    def _ensure_runtime_shared_groups(self) -> None:
        if self.runtime_shared_groups_ready:
            return
        self._ensure_runtime_layer_surface_group()
        self._ensure_runtime_hard_surface_group()
        self._ensure_runtime_illum_group()
        self._ensure_runtime_wear_input_group()
        self._ensure_runtime_iridescence_input_group()
        self._ensure_runtime_nodraw_group()
        self._ensure_runtime_glass_group()
        self._ensure_runtime_screen_group()
        self._ensure_runtime_effect_group()
        self._ensure_runtime_layered_inputs_group()
        self._ensure_runtime_principled_group()
        self._ensure_runtime_hardsurface_stencil_group()
        self._ensure_runtime_channel_split_group()
        self._ensure_runtime_smoothness_roughness_group()
        self._ensure_runtime_color_to_luma_group()
        self._ensure_runtime_shadowless_wrapper_group()
        self.runtime_shared_groups_ready = True

    def _ensure_material_identity_index(self) -> None:
        if self.material_identity_index_ready:
            return
        for material in bpy.data.materials:
            material_identity = material.get(PROP_MATERIAL_IDENTITY)
            if isinstance(material_identity, str) and material_identity:
                self.material_identity_index[material_identity] = material
        self.material_identity_index_ready = True

    def _submaterials_by_index(self, sidecar_path: str, sidecar: MaterialSidecar) -> dict[int, SubmaterialRecord]:
        canonical_path = _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_index.get(canonical_path)
        if cached is not None:
            return cached
        indexed = {submaterial.index: submaterial for submaterial in sidecar.submaterials}
        self.sidecar_submaterials_by_index[canonical_path] = indexed
        return indexed

    def _submaterials_by_unique_name(self, sidecar_path: str, sidecar: MaterialSidecar) -> dict[str, SubmaterialRecord]:
        canonical_path = _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_name.get(canonical_path)
        if cached is not None:
            return cached
        indexed = _unique_submaterials_by_name(sidecar)
        self.sidecar_submaterials_by_name[canonical_path] = indexed
        return indexed

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
        initial_paint_variant = _paint_variant_for_palette_id(self.package, palette_id)
        self.import_palette_override = initial_palette_id
        self.import_paint_variant_sidecar = (
            initial_paint_variant.exterior_material_sidecar
            if initial_paint_variant is not None
            else None
        )
        package_root = self.package_root or self._create_package_root(initial_palette_id)
        self.package_root = package_root
        if initial_palette_id is not None:
            package_root[PROP_PALETTE_ID] = initial_palette_id
        if self.import_paint_variant_sidecar is not None:
            package_root[PROP_PAINT_VARIANT_SIDECAR] = self.import_paint_variant_sidecar

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

    def _effective_import_material_sidecar(self, sidecar_path: str | None) -> str | None:
        if sidecar_path is None:
            return None
        if self.import_paint_variant_sidecar is None:
            return sidecar_path
        if self.exterior_material_sidecars is None:
            return self.import_paint_variant_sidecar
        if sidecar_path in self.exterior_material_sidecars:
            return self.import_paint_variant_sidecar
        return sidecar_path

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
        data = getattr(obj, "data", None)
        data_pointer = data.as_pointer() if data is not None else 0
        slot_mapping = self.slot_mapping_cache.get(data_pointer)
        if data_pointer not in self.slot_mapping_cache:
            slot_mapping = _slot_mapping_for_object(obj)
            self.slot_mapping_cache[data_pointer] = slot_mapping
        if slot_mapping is not None:
            if mesh_materials is not None:
                while len(mesh_materials) < len(slot_mapping):
                    mesh_materials.append(None)
            source_sidecar_path = _slot_mapping_source_sidecar_path(obj, sidecar_path)
            source_sidecar = self.package.load_material_sidecar(source_sidecar_path)
            if source_sidecar is None:
                source_sidecar = sidecar
            source_submaterials_by_index = self._submaterials_by_index(source_sidecar_path, source_sidecar)
            target_submaterials_by_index = self._submaterials_by_index(sidecar_path, sidecar)
            target_submaterials_by_name = self._submaterials_by_unique_name(sidecar_path, sidecar)
            for slot_index, mapped_index in enumerate(slot_mapping):
                fallback_index = mapped_index if mapped_index is not None else slot_index
                source_submaterial = source_submaterials_by_index.get(fallback_index)
                submaterial = _remapped_submaterial_for_slot(
                    source_submaterial,
                    fallback_index,
                    target_submaterials_by_index,
                    target_submaterials_by_name,
                )
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
        self.package_root = package_root
        palette_group = self._ensure_palette_group(palette)
        if effective_palette_id is not None:
            package_root[PROP_PALETTE_ID] = effective_palette_id

        allowed_sidecars = _effective_exterior_material_sidecars(self.package, package_root)

        for material in bpy.data.materials:
            if material.node_tree is None:
                continue
            if not material.get(PROP_SUBMATERIAL_JSON):
                continue
            if allowed_sidecars is not None:
                mat_sidecar = _string_prop(material, PROP_MATERIAL_SIDECAR)
                if mat_sidecar is not None and mat_sidecar not in allowed_sidecars:
                    continue
            has_palette_node = any(
                n.bl_idname == "ShaderNodeGroup"
                and getattr(getattr(n, "node_tree", None), "name", "").startswith("StarBreaker Palette ")
                for n in material.node_tree.nodes
            )
            if has_palette_node:
                self._apply_palette_to_material(material, palette, palette_group)

        self.context.view_layer.update()
        return 0

    def _apply_palette_to_material(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
        palette_group: bpy.types.ShaderNodeTree,
    ) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        palette_node: bpy.types.Node | None = None
        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeGroup":
                continue
            node_tree_name = getattr(getattr(node, "node_tree", None), "name", "")
            if node_tree_name.startswith("StarBreaker Palette "):
                node.node_tree = palette_group
                node.label = f"StarBreaker Palette {palette.id}"
                palette_node = node
                continue
            if node_tree_name.startswith("StarBreaker Runtime LayerSurface"):
                self._update_layer_surface_palette_defaults(node, palette)
            if node_tree_name.startswith("StarBreaker Runtime HardSurface"):
                self._update_runtime_hard_surface_palette_defaults(node, palette)

        if palette_node is not None:
            self._rewire_layer_palette_channels(material, palette, palette_node)

        self._update_virtual_tint_palette_decal_nodes(material, palette)
        material[PROP_PALETTE_ID] = palette.id

    def _rewire_layer_palette_channels(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
        palette_node: bpy.types.Node,
    ) -> None:
        payload = material.get(PROP_SUBMATERIAL_JSON)
        if not isinstance(payload, str):
            return
        try:
            submaterial = SubmaterialRecord.from_value(json.loads(payload))
        except Exception:
            return
        node_tree = material.node_tree
        if node_tree is None:
            return
        channel_socket_name = {
            "primary": "Primary",
            "secondary": "Secondary",
            "tertiary": "Tertiary",
            "glass": "Glass Color",
        }
        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeGroup":
                continue
            node_tree_name = getattr(getattr(node, "node_tree", None), "name", "")
            if not node_tree_name.startswith("StarBreaker Runtime LayerSurface"):
                continue
            finish_channel = node.get("starbreaker_palette_finish_channel")
            if not isinstance(finish_channel, str) or finish_channel not in channel_socket_name:
                continue
            target_socket_name = channel_socket_name.get(finish_channel, "Primary")
            palette_color_input = _input_socket(node, "Palette Color")
            if palette_color_input is None:
                continue
            current_source = palette_color_input.links[0].from_socket.name if palette_color_input.is_linked else None
            if current_source == target_socket_name:
                continue
            if palette_color_input.is_linked:
                node_tree.links.remove(palette_color_input.links[0])
            source_socket = _output_socket(palette_node, target_socket_name)
            if source_socket is not None:
                node_tree.links.new(source_socket, palette_color_input)

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
        if not bool(group_node.get("starbreaker_angle_shift_enabled", False)):
            return
        iridescence_active = _palette_has_iridescence(palette)
        factor_socket = _input_socket(group_node, "Iridescence Factor")
        if factor_socket is not None and hasattr(factor_socket, "default_value"):
            factor_socket.default_value = 1.0 if iridescence_active else 0.0

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

        asset_path = self.package.resolve_path(mesh_asset)
        if asset_path is None or not asset_path.is_file():
            raise RuntimeError(f"Missing mesh asset: {mesh_asset}")
        asset_key = str(asset_path.resolve())

        cached = self.template_cache.get(asset_key)
        if cached is not None:
            return cached

        before = {obj.as_pointer() for obj in bpy.data.objects}
        result = bpy.ops.import_scene.gltf(filepath=str(asset_path), import_pack_images=False, merge_vertices=False)
        if "FINISHED" not in result:
            raise RuntimeError(f"Failed to import {asset_path}")

        imported = [obj for obj in bpy.data.objects if obj.as_pointer() not in before]
        imported_materials_by_pointer: dict[int, bpy.types.Material] = {}
        for obj in imported:
            for slot in getattr(obj, "material_slots", []):
                material = getattr(slot, "material", None)
                if material is not None:
                    imported_materials_by_pointer[material.as_pointer()] = material
        imported_materials = list(imported_materials_by_pointer.values())
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
        self.template_cache[asset_key] = template
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
        palette_scope = self._palette_scope()
        cache_key = _material_identity(sidecar_path, sidecar, submaterial, palette, palette_scope)
        cached = self.material_cache.get(cache_key)
        if cached is not None:
            return cached

        reusable = self._reusable_material(sidecar_path, sidecar, submaterial, palette, palette_scope, cache_key)
        if reusable is not None:
            existing_identity = reusable.get(PROP_MATERIAL_IDENTITY)
            if isinstance(existing_identity, str) and existing_identity == cache_key:
                self.material_cache[cache_key] = reusable
                self.material_identity_index[cache_key] = reusable
                return reusable
            self._build_managed_material(reusable, sidecar_path, sidecar, submaterial, palette, cache_key)
            self.material_cache[cache_key] = reusable
            self.material_identity_index[cache_key] = reusable
            return reusable

        material_name = _material_name(sidecar_path, sidecar, submaterial, cache_key)
        material = bpy.data.materials.new(material_name)
        self._build_managed_material(material, sidecar_path, sidecar, submaterial, palette, cache_key)
        self.material_cache[cache_key] = material
        self.material_identity_index[cache_key] = material
        return material

    def _reusable_material(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        palette_scope: str,
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
            palette_scope,
        ):
            return preferred

        self._ensure_material_identity_index()
        indexed_material = self.material_identity_index.get(material_identity)
        if indexed_material is not None and _material_is_compatible(
            indexed_material,
            self.package,
            sidecar_path,
            sidecar,
            submaterial,
            palette,
            palette_scope,
        ):
            return indexed_material
        return None

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
            palette_channel_name=layer_channel_name,
            palette_finish_channel_name=layer_channel_name,
            palette_glossiness=palette_finish_glossiness(palette, layer_channel_name),
            specular_value=_mean_triplet(_layer_snapshot_triplet(layer, "specular")) or 0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, layer_channel_name)) or 0.0,
            metallic_value=_layer_snapshot_float(layer, "metallic"),
            specular_color=_layer_snapshot_triplet(layer, "specular"),
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
        metallic_value: float,
        specular_color: tuple[float, float, float] | None,
        x: int,
        y: int,
        label: str,
    ) -> LayerSurfaceSockets:
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_layer_surface_group()
        _refresh_group_node_sockets(group_node)
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
        self._set_socket_default(_input_socket(group_node, "Metallic"), metallic_value)
        self._set_socket_default(
            _input_socket(group_node, "Specular Color"),
            (*specular_color, 1.0) if specular_color is not None else (0.04, 0.04, 0.04, 1.0),
        )
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
                luma_group = nodes.new("ShaderNodeGroup")
                luma_group.node_tree = self._ensure_runtime_color_to_luma_group()
                luma_group.location = (x - 20, y - 480)
                luma_group.label = "StarBreaker Color To Luma"
                links.new(palette_specular_color, luma_group.inputs["Color"])
                palette_specular_socket = luma_group.outputs["Luma"]

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
            metallic=SocketRef(group_node, "Metallic"),
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
        target_socket = _input_socket(group_node, socket_name)
        if target_socket is None:
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
        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x + 180, y)
        group_node.node_tree = self._ensure_runtime_channel_split_group()
        group_node.label = "StarBreaker Channel Split"
        links = image_node.id_data.links
        links.new(image_node.outputs[0], group_node.inputs["Color"])
        alpha_socket = _output_socket(image_node, "Alpha")
        if alpha_socket is not None:
            links.new(alpha_socket, group_node.inputs["Alpha"])
        return {
            "red": group_node.outputs["R"],
            "green": group_node.outputs["G"],
            "blue": group_node.outputs["B"],
            "alpha": group_node.outputs["Alpha"],
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
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_color_to_luma_group()
        group_node.location = (x + 180, y)
        group_node.label = "StarBreaker Color To Luma"
        image_node.id_data.links.new(image_node.outputs[0], group_node.inputs["Color"])
        return group_node.outputs["Luma"]

    def _mask_socket(self, nodes: bpy.types.Nodes, image_path: str | None, *, x: int, y: int) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        return image_node.outputs[0]

    def _tiled_image_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
        tiling: float = 1.0,
        uv_map_name: str | None = None,
    ) -> bpy.types.ShaderNodeTexImage | None:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=is_color)
        if image_node is None:
            return None
        if uv_map_name is None and math.isclose(tiling, 1.0, rel_tol=1e-6, abs_tol=1e-6):
            return image_node
        uv_source = None
        if uv_map_name:
            uv_map = nodes.new("ShaderNodeUVMap")
            uv_map.location = (x - 360, y)
            uv_map.uv_map = uv_map_name
            uv_source = _output_socket(uv_map, "UV")
        else:
            tex_coord = nodes.new("ShaderNodeTexCoord")
            tex_coord.location = (x - 360, y)
            uv_source = _output_socket(tex_coord, "UV")
        mapping = nodes.new("ShaderNodeMapping")
        mapping.location = (x - 180, y)
        scale_input = _input_socket(mapping, "Scale")
        if scale_input is not None and hasattr(scale_input, "default_value"):
            scale_input.default_value[0] = tiling
            scale_input.default_value[1] = tiling
            if len(scale_input.default_value) > 2:
                scale_input.default_value[2] = 1.0
        vector_input = _input_socket(mapping, "Vector")
        image_vector = _input_socket(image_node, "Vector")
        mapped_vector = _output_socket(mapping, "Vector")
        if uv_source is not None and vector_input is not None:
            links.new(uv_source, vector_input)
        if mapped_vector is not None and image_vector is not None:
            links.new(mapped_vector, image_vector)
        return image_node

    def _image_mask_socket_from_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        image_node: bpy.types.ShaderNodeTexImage | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if image_node is None:
            return None
        rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
        rgb_to_bw.location = (x, y)
        links.new(image_node.outputs[0], rgb_to_bw.inputs[0])
        alpha_socket = _output_socket(image_node, "Alpha")
        if alpha_socket is None:
            return rgb_to_bw.outputs[0]
        return self._multiply_value_socket(nodes, links, rgb_to_bw.outputs[0], alpha_socket, x=x + 180, y=y)

    def _masked_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        mask_socket: Any,
        color_value: tuple[float, float, float],
        *,
        x: int,
        y: int,
    ) -> Any:
        if mask_socket is None:
            return None
        tint_socket = self._value_color_socket(nodes, (*color_value, 1.0), x=x, y=y)
        black_socket = self._value_color_socket(nodes, (0.0, 0.0, 0.0, 1.0), x=x, y=y - 120)
        return self._mix_color_socket(nodes, links, black_socket, tint_socket, mask_socket, x=x + 180, y=y - 40)

    def _add_color_socket(
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
        add = nodes.new("ShaderNodeMixRGB")
        add.location = (x, y)
        add.blend_type = "ADD"
        add.inputs[0].default_value = 1.0
        self._link_color_output(socket_a, add.inputs[1])
        self._link_color_output(socket_b, add.inputs[2])
        return add.outputs[0]

    def _hard_surface_stencil_overlay_sockets(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> StencilOverlaySockets:
        if not submaterial.decoded_feature_flags.has_stencil_map:
            return StencilOverlaySockets()

        stencil_ref = _submaterial_texture_reference(
            submaterial,
            slots=("TexSlot7",),
            roles=("stencil", "stencil_source", "tint_palette_decal"),
        )
        if stencil_ref is None or stencil_ref.export_path is None:
            return StencilOverlaySockets()

        stencil_tiling = _optional_float_public_param(submaterial, "StencilTiling")
        stencil_uv_map = "UVMap.001" if (_optional_float_public_param(submaterial, "UseUV2ForStencil") or 0.0) > 0.5 else None
        stencil_node = self._tiled_image_node(
            nodes,
            links,
            stencil_ref.export_path,
            x=x,
            y=y,
            is_color=True,
            tiling=stencil_tiling if stencil_tiling is not None and stencil_tiling > 0.0 else 1.0,
            uv_map_name=stencil_uv_map,
        )
        if stencil_node is None:
            return StencilOverlaySockets()

        tint = _public_param_triplet(
            submaterial,
            "StencilDiffuseColor1",
            "StencilDiffuse1",
            "StencilTintColor",
            "TintColor",
            "StencilDiffuseColor",
        )
        tint_2 = _public_param_triplet(submaterial, "StencilDiffuseColor2", "StencilDiffuse2")
        tint_3 = _public_param_triplet(submaterial, "StencilDiffuseColor3", "StencilDiffuse3")
        specular_1 = _public_param_triplet(submaterial, "StencilSpecularColor1", "StencilSpecular1", "StencilSpecularColor")
        specular_2 = _public_param_triplet(submaterial, "StencilSpecularColor2", "StencilSpecular2")
        specular_3 = _public_param_triplet(submaterial, "StencilSpecularColor3", "StencilSpecular3")
        multi_channel_stencil = any(value is not None for value in (tint_2, tint_3, specular_2, specular_3))
        tint_override = _optional_float_public_param(submaterial, "StencilTintOverride") or 0.0
        stencil_glossiness = _optional_float_public_param(submaterial, "StencilGlossiness")
        opacity = _optional_float_public_param(submaterial, "StencilOpacity")

        # Optional breakup image texture at top level.
        breakup_node = None
        breakup_strength = 0.0
        breakup_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot8",), roles=("breakup", "grime_breakup"))
        if breakup_ref is not None and breakup_ref.export_path is not None:
            breakup_tiling = _optional_float_public_param(submaterial, "StencilBreakupTiling")
            breakup_node = self._tiled_image_node(
                nodes,
                links,
                breakup_ref.export_path,
                x=x,
                y=y - 220,
                is_color=False,
                tiling=breakup_tiling if breakup_tiling is not None and breakup_tiling > 0.0 else 1.0,
                uv_map_name=stencil_uv_map,
            )
            breakup_strength = max(
                0.0,
                min(1.0, _optional_float_public_param(submaterial, "StencilDiffuseBreakup", "StencilGlossBreakup") or 0.0),
            )

        # Instantiate the Runtime HardSurface Stencil group.
        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x + 420, y)
        group_node.node_tree = self._ensure_runtime_hardsurface_stencil_group()
        group_node.label = "StarBreaker HardSurface Stencil"

        # Wire image textures.
        links.new(stencil_node.outputs[0], group_node.inputs["Stencil Color"])
        stencil_alpha_socket = _output_socket(stencil_node, "Alpha")
        if stencil_alpha_socket is not None:
            links.new(stencil_alpha_socket, group_node.inputs["Stencil Alpha"])
        if breakup_node is not None:
            links.new(breakup_node.outputs[0], group_node.inputs["Breakup Color"])
            breakup_alpha_socket = _output_socket(breakup_node, "Alpha")
            if breakup_alpha_socket is not None:
                links.new(breakup_alpha_socket, group_node.inputs["Breakup Alpha"])
            group_node.inputs["Breakup Strength"].default_value = breakup_strength
            group_node.inputs["Breakup Enable"].default_value = 1.0 if breakup_strength > 0.0 else 0.0
        else:
            group_node.inputs["Breakup Enable"].default_value = 0.0

        if opacity is not None:
            group_node.inputs["Stencil Opacity"].default_value = max(0.0, min(1.0, opacity))
        if stencil_glossiness is not None:
            group_node.inputs["Stencil Glossiness"].default_value = max(0.0, min(1.0, stencil_glossiness))

        # Decide mode + tint defaults.
        use_override = stencil_ref.is_virtual or tint_override > 0.0
        if use_override:
            group_node.inputs["Mode"].default_value = 0.0
            group_node.inputs["Tint1"].default_value = (*(tint if tint is not None else (1.0, 1.0, 1.0)), 1.0)
            group_node.inputs["Tint1 Enable"].default_value = 1.0
            group_node.inputs["Tint2 Enable"].default_value = 0.0
            group_node.inputs["Tint3 Enable"].default_value = 0.0
        elif multi_channel_stencil:
            group_node.inputs["Mode"].default_value = 1.0
            for slot, color in (("Tint1", tint), ("Tint2", tint_2), ("Tint3", tint_3)):
                enable_key = f"{slot} Enable"
                if color is not None and (_mean_triplet(color) or 0.0) > 0.01:
                    group_node.inputs[slot].default_value = (*color, 1.0)
                    group_node.inputs[enable_key].default_value = 1.0
                else:
                    group_node.inputs[enable_key].default_value = 0.0
        else:
            group_node.inputs["Mode"].default_value = 0.0
            group_node.inputs["Tint1 Enable"].default_value = 0.0
            group_node.inputs["Tint2 Enable"].default_value = 0.0
            group_node.inputs["Tint3 Enable"].default_value = 0.0

        # Specular tints.
        for slot, color in (("Specular1", specular_1), ("Specular2", specular_2), ("Specular3", specular_3)):
            enable_key = f"{slot} Enable"
            if color is not None and (_mean_triplet(color) or 0.0) > 0.0:
                group_node.inputs[slot].default_value = (*color, 1.0)
                group_node.inputs[enable_key].default_value = 1.0
            else:
                group_node.inputs[enable_key].default_value = 0.0

        has_specular = any(
            c is not None and (_mean_triplet(c) or 0.0) > 0.0 for c in (specular_1, specular_2, specular_3)
        )

        return StencilOverlaySockets(
            color=group_node.outputs["Color"],
            color_factor=group_node.outputs["Color Factor"],
            factor=group_node.outputs["Factor"],
            roughness=group_node.outputs["Roughness"] if stencil_glossiness is not None else None,
            specular=group_node.outputs["Specular"] if has_specular else None,
            specular_tint=group_node.outputs["Specular Tint"] if has_specular else None,
        )

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
            color_socket = _output_socket(group_node, "Decal Color")
            alpha_socket = _output_socket(group_node, "Decal Alpha")
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

        palette_color_socket = None
        if palette is not None:
            palette_channel = submaterial.palette_routing.material_channel
            channel_name = palette_channel.name if palette_channel is not None else ("glass" if submaterial.shader_family == "GlassPBR" else None)
            if channel_name is not None:
                palette_color_socket = self._palette_color_socket(nodes, palette, channel_name, x=x, y=y)

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

        if palette_color_socket is not None:
            alpha_node.outputs[0].default_value = alpha
            return LayerSurfaceSockets(color=palette_color_socket, alpha=alpha_node.outputs[0])

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
        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x, y)
        group_node.node_tree = self._ensure_runtime_smoothness_roughness_group()
        group_node.label = "StarBreaker Smoothness To Roughness"
        group_node.id_data.links.new(source_socket, group_node.inputs["Smoothness"])
        return group_node.outputs["Roughness"]

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

    def _hard_surface_angle_factor_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        *,
        x: int,
        y: int,
    ) -> Any:
        layer_weight = nodes.new("ShaderNodeLayerWeight")
        layer_weight.location = (x, y)
        blend_input = _input_socket(layer_weight, "Blend")
        if blend_input is not None:
            blend_input.default_value = 0.3

        angle_factor = nodes.new("ShaderNodeMapRange")
        angle_factor.location = (x + 140, y + 100)
        angle_factor.clamp = True
        angle_factor.inputs[1].default_value = 0.0
        angle_factor.inputs[2].default_value = 0.2
        angle_factor.inputs[3].default_value = 0.0
        angle_factor.inputs[4].default_value = 1.0
        links.new(_output_socket(layer_weight, "Facing"), angle_factor.inputs[0])
        return angle_factor.outputs[0]

    def _iridescence_ramp_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        ramp_path = self._texture_path_for_slot(submaterial, "TexSlot10")
        if ramp_path is None:
            return None

        ramp_node = self._image_node(nodes, ramp_path, x=x + 360, y=y, is_color=True)
        if ramp_node is None:
            return None
        if hasattr(ramp_node, "extension"):
            ramp_node.extension = "EXTEND"

        thickness_u = _optional_float_public_param(submaterial, "IridescenceThicknessU")
        thickness_v = _optional_float_public_param(submaterial, "IridescenceThicknessV")

        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_iridescence_input_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x + 180, y)
        group_node.label = "StarBreaker Iridescence"
        self._set_socket_default(
            _input_socket(group_node, "Thickness U"),
            max(0.0, min(1.0, thickness_u if thickness_u is not None else 1.0)),
        )
        self._set_socket_default(
            _input_socket(group_node, "Thickness V"),
            max(0.0, min(1.0, thickness_v if thickness_v is not None else 0.5)),
        )

        ramp_uv = _output_socket(group_node, "Ramp UV")
        vector_input = _input_socket(ramp_node, "Vector")
        if ramp_uv is not None and vector_input is not None:
            links.new(ramp_uv, vector_input)
        return ramp_node.outputs[0]

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
        has_vertex_colors = submaterial.decoded_feature_flags.has_vertex_colors
        wear_base = _float_public_param(submaterial, "WearBlendBase", "DamagePerObjectWear")
        mask_node = None
        if not has_vertex_colors:
            mask_node = self._image_node(nodes, textures["mask"], x=x - 220, y=y, is_color=False)
        if not has_vertex_colors and mask_node is None and wear_base <= 0.0:
            return None

        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_wear_input_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = "StarBreaker Wear"
        self._set_socket_default(_input_socket(group_node, "Wear Mask"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Use Vertex Colors"), 1.0 if has_vertex_colors else 0.0)
        self._set_socket_default(
            _input_socket(group_node, "Wear Base"),
            max(0.0, wear_base if wear_base > 0.0 else 1.0),
        )
        self._set_socket_default(_input_socket(group_node, "Wear Strength"), self._wear_strength())
        self._set_socket_default(_input_socket(group_node, "Use Damage"), 0.0)

        if not has_vertex_colors and mask_node is not None:
            self._link_group_input(links, mask_node.outputs[0], group_node, "Wear Mask")

        return _output_socket(group_node, "Wear Factor")

    def _layered_damage_factor_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        if not submaterial.decoded_feature_flags.has_damage_map:
            return None
        if not submaterial.decoded_feature_flags.has_vertex_colors:
            return None

        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_wear_input_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = "StarBreaker Damage"
        self._set_socket_default(_input_socket(group_node, "Wear Mask"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Use Vertex Colors"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Wear Base"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Wear Strength"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Use Damage"), 1.0)
        return _output_socket(group_node, "Damage Factor")

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

    def _layered_wear_layer(self, submaterial: SubmaterialRecord) -> LayerManifestEntry | None:
        if len(submaterial.layer_manifest) > 1:
            return submaterial.layer_manifest[1]
        if submaterial.layer_manifest:
            return submaterial.layer_manifest[0]
        return None

    def _layer_color_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        wear_layer = self._layered_wear_layer(submaterial)
        layer = wear_layer if wear_layer is not None and wear_layer.diffuse_export_path else None
        if layer is None:
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
        wear_layer = self._layered_wear_layer(submaterial)
        layer = None
        if wear_layer is not None and (
            wear_layer.roughness_export_path
            or any(texture.alpha_semantic == "smoothness" and texture.export_path for texture in wear_layer.texture_slots)
        ):
            layer = wear_layer
        if layer is None:
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
            and getattr(getattr(node, "node_tree", None), "name", "").startswith("StarBreaker Runtime LayerSurface")
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

    def _wire_surface_shader_to_output(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        surface_shader: Any,
        output_node: bpy.types.Node,
        plan: Any,
        submaterial: SubmaterialRecord | None = None,
    ) -> None:
        """Link *surface_shader* to *output_node*'s Surface socket.

        If ``_plan_casts_no_shadows(plan, submaterial)`` is True, insert the
        shared ``StarBreaker Runtime Shadowless Wrapper`` group so the
        surface becomes invisible to shadow rays while preserving top-level
        graph hygiene. Otherwise link directly.
        """
        if surface_shader is None:
            return
        if self._plan_casts_no_shadows(plan, submaterial):
            wrapper = nodes.new("ShaderNodeGroup")
            wrapper.node_tree = self._ensure_runtime_shadowless_wrapper_group()
            _refresh_group_node_sockets(wrapper)
            wrapper.location = (output_node.location.x - 180, output_node.location.y - 140)
            wrapper.label = "StarBreaker Shadowless"
            links.new(surface_shader, wrapper.inputs["Shader"])
            links.new(wrapper.outputs["Shader"], output_node.inputs[0])
        else:
            links.new(surface_shader, output_node.inputs[0])

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
        effective_material_sidecar = self._effective_import_material_sidecar(record.material_sidecar)
        serialized = json.dumps(record.raw or {
            "entity_name": record.entity_name,
            "mesh_asset": record.mesh_asset,
            "material_sidecar": effective_material_sidecar,
            "palette_id": record.palette_id,
        }, sort_keys=True)
        for obj in objects:
            obj[PROP_SCENE_PATH] = str(self.package.scene_path)
            obj[PROP_EXPORT_ROOT] = str(self.package.export_root)
            obj[PROP_PACKAGE_NAME] = self.package.package_name
            obj[PROP_ENTITY_NAME] = record.entity_name
            if record.mesh_asset is not None:
                obj[PROP_MESH_ASSET] = record.mesh_asset
            if effective_material_sidecar is not None:
                obj[PROP_MATERIAL_SIDECAR] = effective_material_sidecar
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
        package_root[PROP_PALETTE_SCOPE] = uuid.uuid4().hex
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

    def _ensure_tint_decal_adaptor_group(self) -> bpy.types.ShaderNodeTree:
        group_name = "SB_Tint_Decal_Adaptor"
        group = bpy.data.node_groups.get(group_name)
        if group is not None:
            existing_inputs = {
                item.name
                for item in group.interface.items_tree
                if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "INPUT"
            }
            existing_outputs = {
                item.name
                for item in group.interface.items_tree
                if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "OUTPUT"
            }
            expected_inputs = {"Image", "Decal Red Tint", "Decal Green Tint", "Decal Blue Tint"}
            expected_outputs = {"Color", "Alpha"}
            if expected_inputs.issubset(existing_inputs) and expected_outputs.issubset(existing_outputs):
                return group
        if group is None:
            group = bpy.data.node_groups.new(group_name, "ShaderNodeTree")
        for item in list(group.interface.items_tree):
            group.interface.remove(item)
        group.nodes.clear()
        group.interface.new_socket(name="Image", in_out="INPUT", socket_type="NodeSocketColor")
        for tint_name in ("Decal Red Tint", "Decal Green Tint", "Decal Blue Tint"):
            sock = group.interface.new_socket(name=tint_name, in_out="INPUT", socket_type="NodeSocketColor")
            if hasattr(sock, "default_value"):
                sock.default_value = (1.0, 1.0, 1.0, 1.0)
        group.interface.new_socket(name="Color", in_out="OUTPUT", socket_type="NodeSocketColor")
        group.interface.new_socket(name="Alpha", in_out="OUTPUT", socket_type="NodeSocketFloat")

        group_input = group.nodes.new("NodeGroupInput")
        group_input.location = (-380, 200)
        group_output = group.nodes.new("NodeGroupOutput")
        group_output.location = (660, 140)

        separate_rgb = group.nodes.new("ShaderNodeSeparateColor")
        separate_rgb.location = (-140, 300)
        if hasattr(separate_rgb, "mode"):
            separate_rgb.mode = "RGB"
        group.links.new(group_input.outputs["Image"], separate_rgb.inputs[0])

        separate_hsv = group.nodes.new("ShaderNodeSeparateColor")
        separate_hsv.location = (460, 60)
        if hasattr(separate_hsv, "mode"):
            separate_hsv.mode = "HSV"
        group.links.new(group_input.outputs["Image"], separate_hsv.inputs[0])

        red_mix = group.nodes.new("ShaderNodeMix")
        red_mix.label = "Red Mix"
        red_mix.location = (60, 340)
        red_mix.data_type = "RGBA"
        red_mix.blend_type = "MIX"
        red_mix.clamp_factor = True
        red_mix.inputs[6].default_value = (0.0, 0.0, 0.0, 1.0)
        group.links.new(_output_socket(separate_rgb, "Red", "R"), red_mix.inputs[0])
        group.links.new(group_input.outputs["Decal Red Tint"], red_mix.inputs[7])

        green_mix = group.nodes.new("ShaderNodeMix")
        green_mix.label = "Green Mix"
        green_mix.location = (260, 320)
        green_mix.data_type = "RGBA"
        green_mix.blend_type = "MIX"
        green_mix.clamp_factor = True
        group.links.new(_output_socket(separate_rgb, "Green", "G"), green_mix.inputs[0])
        group.links.new(red_mix.outputs[2], green_mix.inputs[6])
        group.links.new(group_input.outputs["Decal Green Tint"], green_mix.inputs[7])

        blue_mix = group.nodes.new("ShaderNodeMix")
        blue_mix.label = "Blue Mix"
        blue_mix.location = (460, 300)
        blue_mix.data_type = "RGBA"
        blue_mix.blend_type = "MIX"
        blue_mix.clamp_factor = True
        group.links.new(_output_socket(separate_rgb, "Blue", "B"), blue_mix.inputs[0])
        group.links.new(green_mix.outputs[2], blue_mix.inputs[6])
        group.links.new(group_input.outputs["Decal Blue Tint"], blue_mix.inputs[7])

        group.links.new(blue_mix.outputs[2], group_output.inputs["Color"])
        group.links.new(_output_socket(separate_hsv, "Value", "V", "Blue"), group_output.inputs["Alpha"])

        return group

    def _ensure_palette_group(self, palette: PaletteRecord) -> bpy.types.ShaderNodeTree:
        group_name = _palette_group_name(self.package.package_name, self._palette_scope())
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
            "Glass SpecColor",
            "Glass Glossiness",
        }
        _stale_outputs = {"Palette Decal Color", "Palette Decal Alpha", "Iridescence Facing Color", "Iridescence Grazing Color", "Iridescence Strength"}
        if group.get("starbreaker_palette_signature") == group_signature and expected_inputs.issubset(existing_inputs) and expected_outputs.issubset(existing_outputs) and not _stale_outputs.intersection(existing_outputs):
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
            ("Glass SpecColor", "glass", -1020),
            ("Glass Glossiness", "glass", -1140),
        )
        for socket_name in (
            "Decal Color",
            "Decal Alpha",
        ):
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

        for item in list(group.interface.items_tree):
            if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "OUTPUT" and item.name in _stale_outputs:
                group.interface.remove(item)

        group.nodes.clear()

        group_input = group.nodes.new("NodeGroupInput")
        group_input.location = (-900, -120)
        output = group.nodes.new("NodeGroupOutput")
        output.location = (520, -120)

        primary_color = (*_palette_decal_or_fallback(palette, "red"), 1.0)
        secondary_color = (*_palette_decal_or_fallback(palette, "green"), 1.0)
        tertiary_color = (*_palette_decal_or_fallback(palette, "blue"), 1.0)

        palette_decal_node = self._image_node(group.nodes, palette_decal_texture(palette), x=-900, y=-520, is_color=True)
        if palette_decal_node is not None:
            adaptor_tree = self._ensure_tint_decal_adaptor_group()
            decal_converter = group.nodes.new("ShaderNodeGroup")
            decal_converter.name = "DecalConverter"
            decal_converter.node_tree = adaptor_tree
            decal_converter.location = (-420, -420)
            group.links.new(palette_decal_node.outputs[0], decal_converter.inputs["Image"])
            decal_converter.inputs["Decal Red Tint"].default_value = primary_color
            decal_converter.inputs["Decal Green Tint"].default_value = secondary_color
            decal_converter.inputs["Decal Blue Tint"].default_value = tertiary_color
            group.links.new(decal_converter.outputs["Color"], output.inputs["Decal Color"])
            group.links.new(decal_converter.outputs["Alpha"], output.inputs["Decal Alpha"])

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
        expected_name = f"STARBREAKER_PALETTE_{_safe_identifier(self._palette_scope()).upper()}"
        existing = next(
            (
                node
                for node in nodes
                if node.bl_idname == "ShaderNodeGroup"
                and getattr(node, "name", "") == expected_name
                and getattr(getattr(node, "node_tree", None), "name", "") == _palette_group_name(self.package.package_name, self._palette_scope())
            ),
            None,
        )
        group_node = existing or nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_palette_group(palette)
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = "StarBreaker Palette"
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




# Phase 7.3: palette + record helpers migrated to runtime.{palette_utils,record_utils}.
from .palette_utils import (
    _palette_decal_or_fallback,
    _palette_group_signature,
    _palette_has_iridescence,
)
from .record_utils import (
    _authored_attribute_string,
    _authored_attribute_triplet,
    _float_authored_attribute,
    _float_layer_public_param,
    _float_public_param,
    _hard_surface_angle_shift_enabled,
    _is_virtual_tint_palette_stencil_decal,
    _layer_snapshot_float,
    _layer_snapshot_triplet,
    _layer_texture_reference,
    _matching_texture_reference,
    _mean_triplet,
    _optional_float_public_param,
    _public_param_triplet,
    _resolved_submaterial_palette_color,
    _routes_virtual_tint_palette_decal_alpha_to_decal_source,
    _routes_virtual_tint_palette_decal_to_decal_source,
    _submaterial_texture_reference,
    _suppresses_virtual_tint_palette_stencil_input,
    _triplet_from_any,
    _triplet_from_string,
    _triplet_from_value,
    _uses_virtual_tint_palette_decal,
)



def _material_identity(
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
    palette_scope: str,
) -> str:
    payload = {
        "schema": MATERIAL_IDENTITY_SCHEMA,
        "material_sidecar": _canonical_material_sidecar_path(sidecar_path, sidecar),
        "submaterial": submaterial.raw,
        "palette_scope": palette_scope,
    }
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
    palette_scope: str,
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

    return _string_prop(material, PROP_PALETTE_SCOPE) == palette_scope


def _managed_material_runtime_graph_is_sane(material: bpy.types.Material) -> bool:
    node_tree = material.node_tree
    if node_tree is None:
        return False

    hard_surface_nodes = [
        node
        for node in node_tree.nodes
        if node.bl_idname == "ShaderNodeGroup"
        and getattr(getattr(node, "node_tree", None), "name", "").startswith("StarBreaker Runtime HardSurface")
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


def _palette_group_name(package_name: str, palette_scope: str) -> str:
    return f"StarBreaker Palette {package_name} {_safe_identifier(palette_scope)}"


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


def _slot_mapping_source_sidecar_path(obj: bpy.types.Object, current_sidecar_path: str) -> str:
    instance = _scene_instance_from_object(obj)
    if instance is not None and instance.material_sidecar:
        return instance.material_sidecar
    return current_sidecar_path


def _unique_submaterials_by_name(sidecar: MaterialSidecar) -> dict[str, SubmaterialRecord]:
    grouped: dict[str, list[SubmaterialRecord]] = {}
    for submaterial in sidecar.submaterials:
        name = submaterial.submaterial_name.strip()
        if not name:
            continue
        grouped.setdefault(name, []).append(submaterial)
    return {
        name: submaterials[0]
        for name, submaterials in grouped.items()
        if len(submaterials) == 1
    }


def _remapped_submaterial_for_slot(
    source_submaterial: SubmaterialRecord | None,
    fallback_index: int,
    target_submaterials_by_index: dict[int, SubmaterialRecord],
    target_submaterials_by_name: dict[str, SubmaterialRecord],
) -> SubmaterialRecord | None:
    if source_submaterial is not None:
        source_name = source_submaterial.submaterial_name.strip()
        if source_name:
            remapped = target_submaterials_by_name.get(source_name)
            if remapped is not None:
                return remapped
    return target_submaterials_by_index.get(fallback_index)


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
    from .node_utils import _input_socket as _impl
    return _impl(node, *names)


def _output_socket(node: Any, *names: str) -> Any:
    from .node_utils import _output_socket as _impl
    return _impl(node, *names)


def _set_group_input_default(group_input_node: Any, socket_name: str, value: Any) -> None:
    from .node_utils import _set_group_input_default as _impl
    _impl(group_input_node, socket_name, value)


def _refresh_group_node_sockets(node: Any) -> None:
    from .node_utils import _refresh_group_node_sockets as _impl
    _impl(node)


def _contract_input_uses_color(contract_input: ContractInput) -> bool:
    semantic = (contract_input.semantic or contract_input.name).lower()
    return not any(keyword in semantic for keyword in NON_COLOR_INPUT_KEYWORDS)
