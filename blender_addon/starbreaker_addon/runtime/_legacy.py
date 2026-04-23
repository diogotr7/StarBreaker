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


BITANGENT_SIGN_ATTRIBUTE = "starbreaker_bitangent_sign"


def _bake_bitangent_sign_attribute(mesh: bpy.types.Mesh) -> bool:
    """Bake per-corner MikkTSpace bitangent sign into a float attribute.

    The POM ``tangent_space`` group multiplies its bitangent projection by
    this attribute to compensate for UV-mirrored regions, where a shared
    tangent direction produces an inverted bitangent. Meshes without a UV
    map, or without loops, are skipped silently.
    """
    if mesh is None or not getattr(mesh, "loops", None):
        return False
    if not mesh.uv_layers:
        return False
    try:
        mesh.calc_tangents()
    except Exception:
        return False
    existing = mesh.attributes.get(BITANGENT_SIGN_ATTRIBUTE)
    if existing is not None:
        mesh.attributes.remove(existing)
    attr = mesh.attributes.new(BITANGENT_SIGN_ATTRIBUTE, "FLOAT", "CORNER")
    for idx, loop in enumerate(mesh.loops):
        attr.data[idx].value = loop.bitangent_sign
    try:
        mesh.free_tangents()
    except Exception:
        pass
    return True


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

# Phase 7.5c: palette wiring + palette node group migrated to runtime.importer.palette.
from .importer.palette import PaletteMixin

# Phase 7.5d: virtual-tint decal sources + shadow wrapper wiring migrated to runtime.importer.decals.
from .importer.decals import DecalsMixin

# Phase 7.5e: layer/wear/detail/stencil/iridescence wiring migrated to runtime.importer.layers.
from .importer.layers import LayersMixin

# Phase 7.5f: material lifecycle + node/socket utilities migrated to runtime.importer.materials.
from .importer.materials import MaterialsMixin


class PackageImporter(
    PaletteMixin,
    DecalsMixin,
    LayersMixin,
    MaterialsMixin,
    BuildersMixin,
    GroupsMixin,
):
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
            # Option E2-Lite: after every slot is assigned, rebind decal
            # slots to per-host-channel clones so each decal picks up the
            # palette colour of the nearest paint material on the mesh.
            self._rebind_mesh_decal_for_host(obj, palette)
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
        # Option E2-Lite: after every slot is assigned, rebind decal
        # slots to per-host-channel clones so each decal picks up the
        # palette colour of the nearest paint material on the mesh.
        self._rebind_mesh_decal_for_host(obj, palette)
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

        baked_meshes: set[int] = set()
        for obj in imported:
            mesh = obj.data if getattr(obj, "type", None) == "MESH" else None
            if mesh is None or mesh.as_pointer() in baked_meshes:
                continue
            if _bake_bitangent_sign_attribute(mesh):
                baked_meshes.add(mesh.as_pointer())

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

    def _root_objects(self, objects: list[bpy.types.Object]) -> list[bpy.types.Object]:
        imported_pointers = {obj.as_pointer() for obj in objects}
        return [obj for obj in objects if obj.parent is None or obj.parent.as_pointer() not in imported_pointers]

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
    """Phase 12: neutralize any ``CryEngine_Z_up`` template root when the
    instance is parented to a hardpoint.

    Historically this additionally required ``_has_non_identity_descendants``
    to be true, which meant templates whose axis-conversion empty had
    only identity-transformed descendants (e.g. ``geo_vtol_fan`` inside
    ``rsi_aurora_mk2_fan_vtol.glb``) kept the 90° Z-up→Y-up rotation
    baked into their top-level transform even though the parent
    hardpoint already provides the correct orientation. For
    hardpoint-attached instances the axis conversion is always
    redundant — the scene matrix conversion
    (``_scene_matrix_to_blender``) has already brought the hardpoint
    into Blender space — so we strip it unconditionally.
    """
    return _is_axis_conversion_root(obj)


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
