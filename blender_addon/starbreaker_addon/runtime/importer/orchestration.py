"""Top-level orchestration for ``PackageImporter``.

Phase 7.5g completes the decomposition of ``runtime/_legacy.py`` by
pulling the 24 orchestration methods (``import_scene``,
``rebuild_object_materials``, ``apply_palette_to_package_root``,
``instantiate_scene_instance``, ``import_interior_container``,
``create_light``, ``ensure_template``, ``instantiate_template``, and
their private helpers) into :class:`OrchestrationMixin`, and composes
the final :class:`PackageImporter` by combining every themed mixin.
"""

from __future__ import annotations

import json
import math
import uuid
from pathlib import Path
from typing import Any, Callable

import bpy
from mathutils import Euler, Matrix

from ..constants import (
    PACKAGE_ROOT_PREFIX,
    PROP_ENTITY_NAME,
    PROP_EXPORT_ROOT,
    PROP_INSTANCE_JSON,
    PROP_LIGHT_ACTIVE_STATE,
    PROP_LIGHT_STATES_JSON,
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
    PROP_SOURCE_NODE_NAME,
    PROP_SUBMATERIAL_JSON,
    PROP_TEMPLATE_PATH,
    TEMPLATE_COLLECTION_NAME,
)
from ..package_ops import (
    _effective_exterior_material_sidecars,
    _exterior_material_sidecars,
    _paint_variant_for_palette_id,
    _string_prop,
)
from ...manifest import MaterialSidecar, PackageBundle, SceneInstanceRecord, SubmaterialRecord
from ...material_contract import TemplateContract
from ...palette import palette_for_id, resolved_palette_id

from .builders import BuildersMixin
from .decals import DecalsMixin
from .groups import GroupsMixin
from .layers import LayersMixin
from .materials import MaterialsMixin
from .palette import PaletteMixin
from .types import ImportedTemplate, _bake_bitangent_sign_attribute
from .utils import (
    _canonical_material_sidecar_path,
    _canonical_source_name,
    _remapped_submaterial_for_slot,
    _scene_attachment_offset_to_blender,
    _scene_light_quaternion_to_blender,
    _scene_matrix_to_blender,
    _scene_position_to_blender,
    _slot_mapping_for_object,
    _slot_mapping_source_sidecar_path,
    _should_neutralize_axis_root,
    _unique_submaterials_by_name,
)


class OrchestrationMixin:
    def __init__(
        self,
        context: bpy.types.Context,
        package: PackageBundle,
        package_root: bpy.types.Object | None = None,
        progress_callback: Callable[[float, str], None] | None = None,
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
        self.progress_callback = progress_callback
        self._progress_total_steps = 1
        self._progress_completed_steps = 0

    def _start_progress(self, total_steps: int, description: str) -> None:
        self._progress_total_steps = max(int(total_steps), 1)
        self._progress_completed_steps = 0
        self._emit_progress(description)

    def _advance_progress(self, description: str) -> None:
        self._progress_completed_steps = min(
            self._progress_completed_steps + 1,
            self._progress_total_steps,
        )
        self._emit_progress(description)

    def _emit_progress(self, description: str) -> None:
        if self.progress_callback is None:
            return
        fraction = self._progress_completed_steps / max(self._progress_total_steps, 1)
        self.progress_callback(fraction, description)

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
        # A per-instance palette_id (e.g. interiors/cabins using
        # `palette/rsi_interior_default`) must win over the
        # exterior-wide paint override — the override represents the
        # user's choice of *exterior* livery, not a blanket palette
        # swap across every subsystem of the package. Only fall back
        # to the override when the instance is palette-agnostic.
        effective_request = palette_id or self.import_palette_override
        if (
            self.import_palette_override is not None
            and palette_id == self.package.scene.root_entity.palette_id
        ):
            effective_request = self.import_palette_override
        return resolved_palette_id(
            self.package,
            effective_request,
            inherited_palette_id or self.package.scene.root_entity.palette_id,
        )

    # Material sidecar path fragments that identify an "interior
    # subsystem" entity (seats, dashboards, cabin trim). Child
    # entities with palette_id=None whose material path contains one
    # of these markers should inherit the package's interior palette
    # rather than the root exterior palette — this mirrors the
    # in-game behaviour where interior geometry picks up the cabin
    # palette regardless of where the entity lives in the scene tree.
    _INTERIOR_MATERIAL_PATTERNS = ("_int_master", "/interior/", "/Interior/")

    def _interior_palette_id(self) -> str | None:
        for interior in self.package.scene.interiors:
            pid = getattr(interior, "palette_id", None)
            if pid:
                return pid
        return None

    def _palette_id_for_instance(self, record: SceneInstanceRecord) -> str | None:
        """Return the effective palette_id request for a
        ``SceneInstanceRecord``, preferring the explicit per-instance
        ``palette_id`` and otherwise routing interior-subsystem
        children to the package's interior palette.
        """
        if record.palette_id:
            return record.palette_id
        material_path = record.material_path or record.material_sidecar or ""
        if any(marker in material_path for marker in self._INTERIOR_MATERIAL_PATTERNS):
            interior_pid = self._interior_palette_id()
            if interior_pid:
                return interior_pid
        return record.palette_id

    def import_scene(self, prefer_cycles: bool = True, palette_id: str | None = None) -> bpy.types.Object:
        total_steps = (
            2
            + len(self.package.scene.children)
            + len(self.package.scene.interiors)
            + sum(len(interior.placements) for interior in self.package.scene.interiors)
        )
        self._start_progress(total_steps, f"Preparing {self.package.package_name}")
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

        self._advance_progress(f"Importing {self.package.scene.root_entity.entity_name}")
        root_anchor, root_nodes = self.instantiate_scene_instance(self.package.scene.root_entity, parent=package_root)
        self.node_index_by_entity_name[self.package.scene.root_entity.entity_name] = self._index_nodes(root_nodes)
        root_anchor.parent = package_root
        scene_root_parent = self._scene_root_parent(root_nodes) or package_root

        for child in self.package.scene.children:
            self._advance_progress(f"Importing {child.entity_name}")
            parent_node = None
            if child.parent_entity_name:
                parent_node = self.node_index_by_entity_name.get(child.parent_entity_name, {}).get(child.parent_node_name or "")
            anchor, child_nodes = self.instantiate_scene_instance(child, parent=scene_root_parent, parent_node=parent_node)
            self.node_index_by_entity_name.setdefault(child.entity_name, {}).update(self._index_nodes(child_nodes))

        for interior in self.package.scene.interiors:
            self._advance_progress(f"Preparing {interior.name}")
            self.import_interior_container(interior, scene_root_parent)

        self._emit_progress(f"Finalizing {self.package.package_name}")
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
            # Phase 30: lift decal faces off the host geometry.
            self._apply_decal_offset_modifier(obj)
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
        # Phase 30: lift decal faces off the host geometry.
        self._apply_decal_offset_modifier(obj)
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
        effective_palette_id = self._effective_palette_id(self._palette_id_for_instance(record))
        anchor = bpy.data.objects.new(record.entity_name, None)
        anchor.empty_display_type = "PLAIN_AXES"
        self.collection.objects.link(anchor)

        target_parent = parent_node or parent
        anchor.parent = target_parent
        anchor.rotation_mode = "QUATERNION"
        if record.local_transform_sc is not None and record.source_transform_basis == "cryengine_z_up":
            anchor.matrix_basis = _scene_matrix_to_blender(record.local_transform_sc)
        else:
            parent_world_matrix = None
            if parent_node is not None:
                parent_world_matrix = tuple(tuple(parent_node.matrix_world[index][column] for column in range(4)) for index in range(4))
            anchor.location = _scene_attachment_offset_to_blender(
                tuple(record.offset_position),
                tuple(record.offset_rotation),
                no_rotation=record.no_rotation,
                parent_world_matrix=parent_world_matrix,
            )
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
        interior_collection = self._ensure_interior_collection()
        interior_collection.objects.link(anchor)

        for placement in interior.placements:
            # A placement may carry its own `palette_id` (loadout-attached
            # gadgets such as the fire-extinguisher tank whose own entity
            # references a tint palette like `kegr_red_black`). When present
            # it overrides the container's palette so each gadget tints from
            # its own palette record.
            placement_palette_id = None
            if isinstance(placement.raw, dict):
                placement_palette_id = placement.raw.get("palette_id")
            effective_placement_palette = placement_palette_id or interior.palette_id

            instance = SceneInstanceRecord(
                entity_name=placement.entity_class_guid or Path(placement.cgf_path or "interior").stem,
                geometry_path=placement.cgf_path,
                material_path=placement.material_path,
                material_sidecar=placement.material_sidecar,
                mesh_asset=placement.mesh_asset,
                palette_id=effective_placement_palette,
                raw=placement.raw,
            )
            effective_palette_id = self._effective_palette_id(instance.palette_id)
            self._advance_progress(f"Importing {instance.entity_name}")
            placement_anchor = bpy.data.objects.new(instance.entity_name, None)
            placement_anchor.parent = anchor
            placement_anchor.matrix_local = _scene_matrix_to_blender(placement.transform)
            interior_collection.objects.link(placement_anchor)

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
                target_collection=interior_collection,
            )
            self._apply_instance_metadata([placement_anchor, *clones], instance, effective_palette_id)
            for clone in clones:
                self.rebuild_object_materials(clone, effective_palette_id)

        for light in interior.lights:
            self.create_light(light, anchor)

        return anchor

    def create_light(self, light: Any, parent: bpy.types.Object) -> bpy.types.Object:
        from .utils import _blender_light_type, _light_energy_to_blender

        blender_light_type = _blender_light_type(light)
        active_state = None
        state_name = getattr(light, "active_state", None)
        state_map = getattr(light, "states", None)
        if state_name and isinstance(state_map, dict):
            active_state = state_map.get(state_name)
        active_intensity_raw = getattr(active_state, "intensity_raw", None) if active_state is not None else None
        active_intensity_candela_proxy = (
            getattr(active_state, "intensity_candela_proxy", None) if active_state is not None else None
        )
        light_intensity_candela_proxy = getattr(light, "intensity_candela_proxy", None)
        light_data = bpy.data.lights.new(name=light.name or "StarBreaker Light", type=blender_light_type)
        light_data.energy = _light_energy_to_blender(
            active_intensity_candela_proxy
            if active_intensity_candela_proxy is not None
            else light_intensity_candela_proxy
            if light_intensity_candela_proxy is not None
            else 0.0,
            blender_light_type,
            intensity_raw=active_intensity_raw,
        )
        light_data.color = light.color
        if blender_light_type != "SUN" and hasattr(light_data, "cutoff_distance"):
            light_data.cutoff_distance = light.radius
        if blender_light_type == "AREA":
            light_data.shape = "RECTANGLE"
            light_data.size = max(float(light.radius or 0.0), 0.05)
            if hasattr(light_data, "size_y"):
                light_data.size_y = max(float(light.radius or 0.0), 0.05)
        if blender_light_type == "SPOT" and hasattr(light_data, "spot_size"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            light_data.spot_size = math.radians(outer_angle) * 2.0
        if blender_light_type == "SPOT" and hasattr(light_data, "spot_blend"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            inner_angle = min(light.inner_angle or 0.0, outer_angle)
            inner_ratio = min(max(inner_angle / outer_angle, 0.0), 1.0)
            light_data.spot_blend = 1.0 - inner_ratio
        # Phase 25: give point/spot lights a non-zero shadow soft size so
        # shadow edges aren't pin-sharp. Star Citizen doesn't publish a
        # dedicated emitter radius, so fall back to a small floor.
        if blender_light_type in {"POINT", "SPOT"} and hasattr(light_data, "shadow_soft_size"):
            light_data.shadow_soft_size = max(float(getattr(light_data, "shadow_soft_size", 0.0) or 0.0), 0.02)

        self._wire_light_gobo(light_data, light)

        # Phase 28: stash the full state map + active state name on the Light
        # datablock so the runtime state switcher can swap between
        # defaultState/auxiliaryState/emergencyState/cinematicState.
        states = getattr(light, "states", None) or {}
        if states:
            import json as _json
            light_data[PROP_LIGHT_STATES_JSON] = _json.dumps(
                {
                    name: {
                        "intensity_cd": s.intensity_cd,
                        "temperature": s.temperature,
                        "use_temperature": s.use_temperature,
                        "color": list(s.color),
                    }
                    for name, s in states.items()
                }
            )
            light_data[PROP_LIGHT_ACTIVE_STATE] = str(getattr(light, "active_state", "") or "")

        light_object = bpy.data.objects.new(light.name or "StarBreaker Light", light_data)
        light_object.parent = parent
        light_object.location = _scene_position_to_blender(light.position)
        light_object.rotation_mode = "QUATERNION"
        light_object.rotation_quaternion = _scene_light_quaternion_to_blender(light.rotation)
        self.collection.objects.link(light_object)
        return light_object

    def _wire_light_gobo(self, light_data: bpy.types.Light, light: Any) -> None:
        """Enable and author a gobo shader graph on ``light_data`` if ``light``
        references a projector texture path.

        No-op when ``light.projector_texture`` is empty or the texture cannot
        be resolved under the current package. Uses the shared
        ``StarBreaker Runtime Gobo`` group so the light's top-level graph
        stays minimal (TexCoord -> Mapping -> Image -> Gobo -> Output).
        """
        from .utils import _light_gobo_strength, _light_gobo_texcoord_output_name

        projector_texture = getattr(light, "projector_texture", None)
        if not projector_texture:
            return
        resolved = self.package.resolve_path(projector_texture)
        if resolved is None or not resolved.is_file():
            return

        gobo_group = self._ensure_runtime_gobo_group()

        light_data.use_nodes = True
        node_tree = light_data.node_tree
        if node_tree is None:
            return
        nodes = node_tree.nodes
        links = node_tree.links
        nodes.clear()

        tex_coord = nodes.new("ShaderNodeTexCoord")
        tex_coord.location = (-800, 0)
        mapping = nodes.new("ShaderNodeMapping")
        mapping.location = (-600, 0)
        tex_image = nodes.new("ShaderNodeTexImage")
        tex_image.location = (-400, 0)
        tex_image.image = bpy.data.images.load(str(resolved), check_existing=True)
        gobo = nodes.new("ShaderNodeGroup")
        gobo.node_tree = gobo_group
        gobo.location = (-100, 0)
        image = tex_image.image
        mean_luminance = image.get("starbreaker_gobo_mean_luminance") if image is not None else None
        if image is not None and mean_luminance is None:
            pixels = image.pixels[:]
            luminance_total = 0.0
            sample_count = 0
            for index in range(0, len(pixels), 4):
                luminance_total += (pixels[index] + pixels[index + 1] + pixels[index + 2]) / 3.0
                sample_count += 1
            mean_luminance = (luminance_total / sample_count) if sample_count else 0.0
            image["starbreaker_gobo_mean_luminance"] = mean_luminance
        # Headlight cookies are sparse masks; normalize them so the cookie
        # shape does not erase most of the authored projector energy.
        gobo.inputs["Strength"].default_value = _light_gobo_strength(
            projector_texture,
            mean_luminance=float(mean_luminance) if mean_luminance is not None else None,
        )
        output = nodes.new("ShaderNodeOutputLight")
        output.location = (100, 0)

        links.new(tex_coord.outputs[_light_gobo_texcoord_output_name()], mapping.inputs["Vector"])
        links.new(mapping.outputs["Vector"], tex_image.inputs["Vector"])
        links.new(tex_image.outputs["Color"], gobo.inputs["Gobo Image"])
        links.new(gobo.outputs["Shader"], output.inputs["Surface"])

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

        # Phase 19 — glTF meshes coming out of the Rust exporter carry
        # per-vertex normals that the exporter reconstructs from the
        # source mesh. The reconstruction does not weight shared-vertex
        # normals by face area, which leaves subtle flat-spots on every
        # curved panel once Blender averages them at runtime. Attaching
        # a default Weighted Normal modifier (Face Area / weight 50 /
        # threshold 0.01) to every imported mesh object restores the
        # expected rounded shading without mutating the mesh data, so
        # it composes cleanly with any later edits. ``source.copy()``
        # in ``_duplicate_object_tree`` carries the modifier stack into
        # every clone, so adding it once here covers every instance.
        for obj in imported:
            if getattr(obj, "type", None) != "MESH":
                continue
            if obj.data is None or not obj.data.polygons:
                continue
            if any(m.type == "WEIGHTED_NORMAL" for m in obj.modifiers):
                continue
            modifier = obj.modifiers.new(name="StarBreaker Weighted Normal", type="WEIGHTED_NORMAL")
            modifier.mode = "FACE_AREA"
            modifier.weight = 50
            modifier.thresh = 0.01

        template = ImportedTemplate(mesh_asset=mesh_asset, root_names=[obj.name for obj in root_objects])
        self.template_cache[asset_key] = template
        return template

    def instantiate_template(
        self,
        template: ImportedTemplate,
        anchor: bpy.types.Object,
        neutralize_axis_root: bool = False,
        force_neutralize_axis_root: bool = False,
        target_collection: bpy.types.Collection | None = None,
    ) -> list[bpy.types.Object]:
        clones: list[bpy.types.Object] = []
        mapping: dict[str, bpy.types.Object] = {}
        needs_view_layer_update = False
        link_collection = target_collection or self.collection
        for root_name in template.root_names:
            source = bpy.data.objects.get(root_name)
            if source is None:
                continue
            neutralize_root = neutralize_axis_root and (
                force_neutralize_axis_root or _should_neutralize_axis_root(source, template.mesh_asset)
            )
            clone = self._duplicate_object_tree(source, template.mesh_asset, mapping, link_collection)
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
        link_collection: bpy.types.Collection | None = None,
    ) -> bpy.types.Object:
        clone = source.copy()
        if source.data is not None:
            clone.data = source.data
        clone.animation_data_clear()
        clone.hide_set(False)
        clone.hide_render = False
        clone[PROP_TEMPLATE_PATH] = mesh_asset
        clone[PROP_SOURCE_NODE_NAME] = source.get(PROP_SOURCE_NODE_NAME, source.name)
        (link_collection or self.collection).objects.link(clone)
        clone.matrix_basis = source.matrix_basis.copy()
        mapping[source.name] = clone

        for child in source.children:
            if child.get(PROP_TEMPLATE_PATH) != mesh_asset:
                continue
            child_clone = self._duplicate_object_tree(child, mesh_asset, mapping, link_collection)
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

    def _ensure_interior_collection(self) -> bpy.types.Collection:
        """Return (and lazily create) a per-package Interior sub-collection.

        The interior collection is kept fully render-visible (camera,
        transmission, diffuse, glossy, shadow) so interior geometry
        appears both directly and through exterior canopy glass.
        """
        package_collection = self.collection
        interior_name = f"{package_collection.name} Interior"
        interior_collection = bpy.data.collections.get(interior_name)
        if interior_collection is None:
            interior_collection = bpy.data.collections.new(interior_name)
            package_collection.children.link(interior_collection)
        # Always reset in case a previous import left it disabled for render.
        interior_collection.hide_render = False
        return interior_collection

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


class PackageImporter(
    PaletteMixin,
    DecalsMixin,
    LayersMixin,
    MaterialsMixin,
    BuildersMixin,
    GroupsMixin,
    OrchestrationMixin,
):
    """Final composed importer. All behaviour lives in themed mixins."""
