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
import mathutils

from ..constants import (
    DECAL_OFFSET_MODIFIER_NAME,
    DECAL_OFFSET_TEMPLATE_CAP,
    PACKAGE_ROOT_PREFIX,
    PROP_ASSEMBLY_KIND,
    PROP_DECAL_HOST_CHANNEL,
    PROP_DECAL_HOST_RGB,
    PROP_ENGINE_GLOW_CONTROL_JSON,
    PROP_ENGINE_GLOW_STRENGTH,
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
    PROP_UI_DEFAULT_STATE_IS_OFF,
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
    _gltf_matrix_to_blender_basis,
    _remapped_submaterial_for_slot,
    _scene_attachment_offset_to_blender,
    _scene_light_quaternion_to_blender,
    _scene_matrix_to_blender,
    _scene_position_to_blender,
    _slot_mapping_from_slot_names,
    _slot_mapping_for_object,
    _slot_mapping_source_sidecar_path,
    _slot_names_for_object,
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
        create_template_collection: bool = True,
    ) -> None:
        self.context = context
        self.package = package
        self.collection = self._ensure_collection(package.package_name)
        self.template_collection = self._ensure_template_collection() if create_template_collection else None
        self.package_root = package_root
        self.exterior_material_sidecars = _exterior_material_sidecars(package)
        self.template_cache: dict[str, ImportedTemplate] = {}
        self.material_cache: dict[str, bpy.types.Material] = {}
        self.node_index_by_entity_name: dict[str, dict[str, bpy.types.Object]] = {}
        self.bundled_template_contract: TemplateContract | None = None
        self.import_palette_override: str | None = None
        self.import_paint_variant_sidecar: str | None = None
        self.runtime_shared_groups_ready = False
        self.material_identity_cache: dict[tuple[str, int, str, str], str] = {}
        self.material_identity_index: dict[str, bpy.types.Material] = {}
        self.material_identity_index_ready = False
        self.sidecar_submaterials_by_index: dict[str, dict[int, SubmaterialRecord]] = {}
        self.sidecar_submaterials_by_name: dict[str, dict[str, SubmaterialRecord]] = {}
        self.sidecar_submaterials_by_name_all: dict[str, dict[str, list[SubmaterialRecord]]] = {}
        self.slot_mapping_cache: dict[int, list[int | None] | None] = {}
        self.material_slot_layout_cache: dict[
            tuple[int, str, str, str | None, tuple[int | None, ...], object | None],
            tuple[tuple[bpy.types.Material | None, ...], int],
        ] = {}
        self.host_channel_cache: dict[tuple[int, tuple[int, ...]], str | None] = {}
        self.host_rgb_cache: dict[tuple[int, tuple[int, ...]], tuple[float, float, float] | None] = {}
        self.mesh_polygon_counts_cache: dict[int, dict[int, int]] = {}
        self.progress_callback = progress_callback
        self._progress_total_steps = 1
        self._progress_completed_steps = 0
        # Phase A perf fix: defer view_layer.update() between template clones.
        # The original per-template update was costing ~67s on a Clipper import
        # because each interior placement (~600+) triggered a full depsgraph
        # evaluation. We now batch-flush only when subsequent code is about to
        # read matrix_world (the start of instantiate_scene_instance) or after
        # finishing a placement loop. See ``_flush_pending_view_layer_update``.
        self._pending_view_layer_update = False
        # Phase B perf fix: defer ``bpy.data.materials.remove`` until end of
        # import. Removing per-template was costing ~8s on a Clipper because
        # each call triggers internal RNA bookkeeping. ``bpy.data.batch_remove``
        # is much cheaper for bulk removal at the end. See
        # ``_flush_pending_orphan_materials``.
        self._pending_orphan_materials: set[bpy.types.Material] = set()

    def _flush_pending_view_layer_update(self) -> None:
        if self._pending_view_layer_update:
            self.context.view_layer.update()
            self._pending_view_layer_update = False

    def _flush_pending_orphan_materials(self) -> None:
        if not self._pending_orphan_materials:
            return
        # Re-check users at flush time; intervening work may have re-bound
        # materials we previously marked orphan, and the per-template path
        # also accepts "users == 0" as the contract.
        orphans = [m for m in self._pending_orphan_materials if m and m.users == 0]
        self._pending_orphan_materials.clear()
        if orphans:
            bpy.data.batch_remove(ids=orphans)

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
        # Index every material (including decal-host variants) by the identity that
        # builders stamped at creation. Re-deriving the identity here is unsafe:
        # builders computes it from call-site-specific parts, so a fixed-arity
        # reconstruction would not match the stamped value, defeating reuse and
        # accumulating duplicate host-variant materials across re-imports.
        for material in bpy.data.materials:
            material_identity = material.get(PROP_MATERIAL_IDENTITY)
            if isinstance(material_identity, str) and material_identity:
                self.material_identity_index[material_identity] = material
        self.material_identity_index_ready = True

    def _remove_replaced_slot_material(self, material: bpy.types.Material | None) -> None:
        if material is None or getattr(material, "library", None) is not None:
            return
        if material.users != 0:
            return
        if material.get(PROP_MATERIAL_IDENTITY):
            return
        self._pending_orphan_materials.add(material)

    def _submaterials_by_index(self, sidecar_path: str, sidecar: MaterialSidecar) -> dict[int, SubmaterialRecord]:
        cache_key = sidecar_path or _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_index.get(cache_key)
        if cached is not None:
            return cached
        indexed = {submaterial.index: submaterial for submaterial in sidecar.submaterials}
        self.sidecar_submaterials_by_index[cache_key] = indexed
        return indexed

    def _submaterials_by_unique_name(self, sidecar_path: str, sidecar: MaterialSidecar) -> dict[str, SubmaterialRecord]:
        cache_key = sidecar_path or _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_name.get(cache_key)
        if cached is not None:
            return cached
        indexed = _unique_submaterials_by_name(sidecar)
        self.sidecar_submaterials_by_name[cache_key] = indexed
        return indexed

    def _submaterials_by_name_all(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
    ) -> dict[str, list[SubmaterialRecord]]:
        cache_key = sidecar_path or _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_name_all.get(cache_key)
        if cached is not None:
            return cached
        grouped: dict[str, list[SubmaterialRecord]] = {}
        for submaterial in sidecar.submaterials:
            submaterial_name = submaterial.submaterial_name.strip()
            if not submaterial_name:
                continue
            grouped.setdefault(submaterial_name, []).append(submaterial)
        self.sidecar_submaterials_by_name_all[cache_key] = grouped
        return grouped

    def _slot_submaterials_for_object(
        self,
        obj: bpy.types.Object,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        slot_mapping: list[int | None] | None,
    ) -> list[SubmaterialRecord | None]:
        slot_count = len(getattr(obj, "material_slots", []) or [])
        if slot_mapping is not None:
            slot_count = max(slot_count, len(slot_mapping))
        by_index = self._submaterials_by_index(sidecar_path, sidecar)
        slot_submaterials: list[SubmaterialRecord | None] = []
        for slot_index in range(slot_count):
            mapped_index = (
                slot_mapping[slot_index]
                if slot_mapping is not None and slot_index < len(slot_mapping)
                else slot_index
            )
            slot_submaterials.append(
                by_index.get(mapped_index) if mapped_index is not None else None
            )
        return slot_submaterials

    def _store_object_decal_host_route(
        self,
        obj: bpy.types.Object,
        channel: str | None,
        rgb: tuple[float, float, float] | None,
    ) -> None:
        try:
            if channel:
                obj[PROP_DECAL_HOST_CHANNEL] = channel
            elif PROP_DECAL_HOST_CHANNEL in obj:
                del obj[PROP_DECAL_HOST_CHANNEL]
        except Exception:
            pass
        try:
            if rgb is not None:
                obj[PROP_DECAL_HOST_RGB] = [float(rgb[0]), float(rgb[1]), float(rgb[2])]
            elif PROP_DECAL_HOST_RGB in obj:
                del obj[PROP_DECAL_HOST_RGB]
        except Exception:
            pass

    def _precomputed_decal_host_route(
        self,
        obj: bpy.types.Object,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        slot_mapping: list[int | None] | None,
    ) -> tuple[str | None, tuple[float, float, float] | None]:
        stored_channel = self._stored_decal_host_channel(obj)
        stored_rgb = self._stored_decal_host_rgb(obj)
        if stored_channel is not None or stored_rgb is not None:
            return stored_channel, stored_rgb

        channel, rgb = self._derive_decal_host_route_from_submaterials(
            obj,
            self._slot_submaterials_for_object(obj, sidecar_path, sidecar, slot_mapping),
        )
        if channel is None and rgb is None:
            parent = getattr(obj, "parent", None)
            parent_sidecar_path = _string_prop(parent, PROP_MATERIAL_SIDECAR) if parent is not None else None
            if parent is not None and getattr(parent, "type", None) == "MESH" and parent_sidecar_path is not None:
                parent_sidecar = self.package.load_material_sidecar(parent_sidecar_path)
                if parent_sidecar is not None:
                    parent_data = getattr(parent, "data", None)
                    parent_pointer = parent_data.as_pointer() if parent_data is not None else 0
                    parent_slot_mapping = self.slot_mapping_cache.get(parent_pointer)
                    if parent_pointer not in self.slot_mapping_cache:
                        parent_slot_mapping = _slot_mapping_for_object(parent)
                        self.slot_mapping_cache[parent_pointer] = parent_slot_mapping
                    channel, rgb = self._precomputed_decal_host_route(
                        parent,
                        parent_sidecar_path,
                        parent_sidecar,
                        parent_slot_mapping,
                    )
        self._store_object_decal_host_route(obj, channel, rgb)
        return channel, rgb

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

    def _palette_id_for_instance(self, record: SceneInstanceRecord) -> str | None:
        """Return the effective palette_id request for a
        ``SceneInstanceRecord``, preferring explicit per-instance
        ``palette_id`` and falling back to sidecar-authored
        ``DefaultPalette`` metadata when available.
        """
        if record.palette_id:
            return record.palette_id
        if record.material_sidecar:
            return self._sidecar_default_palette_id(record.material_sidecar)
        return None

    def _instance_payload_dict(self, obj: bpy.types.Object) -> dict[str, Any] | None:
        payload = obj.get(PROP_INSTANCE_JSON)
        if not isinstance(payload, str) or not payload:
            return None
        try:
            decoded = json.loads(payload)
        except (json.JSONDecodeError, TypeError, ValueError):
            return None
        return decoded if isinstance(decoded, dict) else None

    @staticmethod
    def _source_names_from_payload(payload: dict[str, Any] | None) -> set[str]:
        if payload is None:
            return set()
        source_names: set[str] = set()
        for value in (
            payload.get("source_object_name"),
            payload.get("source_parent_name"),
        ):
            if isinstance(value, str) and value:
                source_names.add(value)
        source_ancestors = payload.get("source_ancestors")
        if isinstance(source_ancestors, list):
            for value in source_ancestors:
                if isinstance(value, str) and value:
                    source_names.add(value)
        return source_names

    def _manifest_ui_binding_records(
        self,
    ) -> dict[tuple[str, str], list[SceneInstanceRecord]]:
        cached = getattr(self, "_ui_binding_records_by_asset", None)
        if cached is not None:
            return cached
        indexed: dict[tuple[str, str], list[SceneInstanceRecord]] = {}
        for record in getattr(self.package.scene, "children", []):
            if not isinstance(record, SceneInstanceRecord):
                continue
            if not record.mesh_asset or not record.ui_bindings:
                continue
            key = (record.mesh_asset, record.material_sidecar or "")
            indexed.setdefault(key, []).append(record)
        self._ui_binding_records_by_asset = indexed
        return indexed

    def _manifest_ui_bindings_for_object(
        self,
        obj: bpy.types.Object,
        payload: dict[str, Any] | None,
    ) -> list[dict[str, Any]] | None:
        mesh_asset = _string_prop(obj, PROP_MESH_ASSET)
        if mesh_asset is None:
            return None
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR) or ""
        candidates = self._manifest_ui_binding_records().get((mesh_asset, sidecar_path))
        if not candidates and sidecar_path:
            candidates = self._manifest_ui_binding_records().get((mesh_asset, ""))
        if not candidates:
            return None
        if len(candidates) == 1:
            return candidates[0].ui_bindings

        ancestor_names: set[str] = set()
        current = obj.parent
        while current is not None:
            name = getattr(current, "name", None)
            if isinstance(name, str) and name:
                ancestor_names.add(name.lower())
            current = current.parent
        ancestry_matches = [
            record
            for record in candidates
            if record.parent_node_name
            and any(record.parent_node_name.lower() in ancestor_name for ancestor_name in ancestor_names)
        ]
        narrowed = ancestry_matches or candidates
        if len(narrowed) == 1:
            return narrowed[0].ui_bindings

        source_names = self._source_names_from_payload(payload)
        helper_matches = [
            record
            for record in narrowed
            if any(
                isinstance(binding.get("helper_name"), str)
                and binding.get("helper_name") in source_names
                and isinstance(binding.get("generated_image_path"), str)
                and binding.get("generated_image_path")
                for binding in record.ui_bindings
            )
        ]
        narrowed = helper_matches or narrowed
        if len(narrowed) == 1:
            return narrowed[0].ui_bindings

        unique_bindings = {
            json.dumps(bindings, sort_keys=True): bindings
            for bindings in (record.ui_bindings for record in narrowed)
        }
        if len(unique_bindings) == 1:
            return next(iter(unique_bindings.values()))
        return None

    def _ui_binding_for_object(self, obj: bpy.types.Object) -> dict[str, Any] | None:
        payload = self._instance_payload_dict(obj)
        bindings = payload.get("ui_bindings") if payload is not None else None
        if (not isinstance(bindings, list) or not bindings) and payload is not None:
            bindings = self._manifest_ui_bindings_for_object(obj, payload)
        if not isinstance(bindings, list) or not bindings:
            return None
        source_names = self._source_names_from_payload(payload)
        fallback = None
        for binding in bindings:
            if not isinstance(binding, dict):
                continue
            image_path = binding.get("generated_image_path")
            if not isinstance(image_path, str) or not image_path:
                continue
            helper_name = binding.get("helper_name")
            if isinstance(helper_name, str) and helper_name:
                if helper_name in source_names:
                    return binding
                if fallback is None:
                    fallback = binding
                continue
            if fallback is None:
                fallback = binding
        return fallback

    def _sidecar_default_palette_id(self, sidecar_path: str) -> str | None:
        load_sidecar = getattr(self.package, "load_material_sidecar", None)
        if not callable(load_sidecar):
            return None
        sidecar = load_sidecar(sidecar_path)
        if sidecar is None:
            return None
        attributes = (
            getattr(sidecar, "raw", {})
            .get("authored_material_set", {})
            .get("attributes", [])
        )
        for attribute in attributes:
            name = str(attribute.get("name", ""))
            if name.lower() != "defaultpalette":
                continue
            value = str(attribute.get("value", "")).replace("\\", "/").strip()
            source_name = value.rsplit("/", 1)[-1].strip().lower()
            if source_name:
                return f"palette/{source_name}"
        return None

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

        # Final flush in case anything else deferred a depsgraph update.
        self._flush_pending_view_layer_update()
        # Drain the per-template orphan-material queue with a single
        # ``bpy.data.batch_remove`` instead of N per-template ``remove`` calls.
        self._flush_pending_orphan_materials()
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
        target_submaterials_by_name = self._submaterials_by_unique_name(sidecar_path, sidecar)
        target_submaterials_by_name_all = self._submaterials_by_name_all(sidecar_path, sidecar)
        if slot_mapping is None:
            slot_names = _slot_names_for_object(obj)
            if slot_names is not None:
                slot_mapping = _slot_mapping_from_slot_names(
                    slot_names,
                    target_submaterials_by_name,
                    target_submaterials_by_name_all,
                )
        if slot_mapping is None:
            inferred_slot_mapping: list[int | None] = []
            inferred_matches = 0
            if mesh_materials is not None:
                for slot_material in mesh_materials:
                    if slot_material is None:
                        inferred_slot_mapping.append(None)
                        continue
                    canonical_slot_name = _canonical_source_name(slot_material.name)
                    if canonical_slot_name is None:
                        inferred_slot_mapping.append(None)
                        continue
                    matched_submaterial = target_submaterials_by_name.get(canonical_slot_name)
                    if matched_submaterial is None:
                        candidates = target_submaterials_by_name_all.get(canonical_slot_name)
                        if candidates:
                            matched_submaterial = min(
                                candidates,
                                key=lambda item: abs(item.index - len(inferred_slot_mapping)),
                            )
                    if matched_submaterial is None:
                        inferred_slot_mapping.append(None)
                        continue
                    inferred_slot_mapping.append(matched_submaterial.index)
                    inferred_matches += 1
            if inferred_matches > 0:
                slot_mapping = inferred_slot_mapping
        if slot_mapping is None:
            inferred_slot_mapping: list[int | None] = []
            inferred_matches = 0
            for slot_index, slot in enumerate(obj.material_slots):
                slot_material = slot.material
                if slot_material is None:
                    inferred_slot_mapping.append(None)
                    continue
                canonical_slot_name = _canonical_source_name(slot_material.name)
                if canonical_slot_name is None:
                    inferred_slot_mapping.append(None)
                    continue
                matched_submaterial = target_submaterials_by_name.get(canonical_slot_name)
                if matched_submaterial is None:
                    candidates = target_submaterials_by_name_all.get(canonical_slot_name)
                    if candidates:
                        matched_submaterial = min(
                            candidates,
                            key=lambda item: abs(item.index - slot_index),
                        )
                if matched_submaterial is None:
                    inferred_slot_mapping.append(None)
                    continue
                inferred_slot_mapping.append(matched_submaterial.index)
                inferred_matches += 1
            if inferred_matches > 0:
                slot_mapping = inferred_slot_mapping
        precomputed_host_channel, precomputed_host_rgb = self._precomputed_decal_host_route(
            obj,
            sidecar_path,
            sidecar,
            slot_mapping,
        )
        ui_binding = self._ui_binding_for_object(obj)
        ui_image_path = (
            ui_binding.get("generated_image_path")
            if isinstance(ui_binding, dict) and isinstance(ui_binding.get("generated_image_path"), str)
            else None
        )
        if isinstance(ui_binding, dict) and bool(ui_binding.get("default_state_is_off")):
            obj[PROP_UI_DEFAULT_STATE_IS_OFF] = True
        elif callable(getattr(obj, "get", None)) and obj.get(PROP_UI_DEFAULT_STATE_IS_OFF) is not None:
            del obj[PROP_UI_DEFAULT_STATE_IS_OFF]
        if slot_mapping is not None:
            if mesh_materials is not None:
                while len(mesh_materials) < len(slot_mapping):
                    mesh_materials.append(None)
            source_sidecar_path = _slot_mapping_source_sidecar_path(obj, sidecar_path)
            layout_key = (
                data_pointer,
                sidecar_path,
                source_sidecar_path,
                effective_palette_id,
                ui_image_path,
                tuple(slot_mapping),
                ("channel", precomputed_host_channel)
                if precomputed_host_channel is not None
                else ("rgb", tuple(round(component, 6) for component in precomputed_host_rgb))
                if precomputed_host_rgb is not None
                else None,
            )
            material_slot_layout_cache = getattr(self, "material_slot_layout_cache", None)
            if material_slot_layout_cache is None:
                material_slot_layout_cache = {}
                self.material_slot_layout_cache = material_slot_layout_cache
            cached_layout = material_slot_layout_cache.get(layout_key)
            if cached_layout is not None:
                cached_materials, cached_applied = cached_layout
                if len(obj.material_slots) >= len(cached_materials):
                    for slot_index, material in enumerate(cached_materials):
                        slot = obj.material_slots[slot_index]
                        replaced_material = slot.material
                        slot.link = "OBJECT"
                        slot.material = material
                        if replaced_material is not material:
                            self._remove_replaced_slot_material(replaced_material)
                    if effective_palette_id is not None:
                        obj[PROP_PALETTE_ID] = effective_palette_id
                    return cached_applied
            source_sidecar = self.package.load_material_sidecar(source_sidecar_path)
            if source_sidecar is None:
                source_sidecar = sidecar
            source_submaterials_by_index = self._submaterials_by_index(source_sidecar_path, source_sidecar)
            target_submaterials_by_index = self._submaterials_by_index(sidecar_path, sidecar)
            for slot_index, mapped_index in enumerate(slot_mapping):
                fallback_index = mapped_index if mapped_index is not None else slot_index
                source_submaterial = source_submaterials_by_index.get(fallback_index)
                slot_material = (
                    obj.material_slots[slot_index].material
                    if slot_index < len(obj.material_slots)
                    else None
                )
                submaterial = _remapped_submaterial_for_slot(
                    source_submaterial,
                    fallback_index,
                    target_submaterials_by_index,
                    target_submaterials_by_name,
                    target_submaterials_by_name_all,
                )
                if submaterial is None and slot_material is not None:
                    canonical_slot_name = _canonical_source_name(slot_material.name)
                    if canonical_slot_name is not None:
                        submaterial = target_submaterials_by_name.get(canonical_slot_name)
                if submaterial is None:
                    if mapped_index is None and slot_material is None:
                        continue
                    print(
                        f"StarBreaker: missing sidecar submaterial index {fallback_index} for {obj.name}"
                    )
                    continue
                if slot_index >= len(obj.material_slots):
                    print(
                        f"StarBreaker: slot index {slot_index} exceeds material slot count for {obj.name}"
                    )
                    continue
                material = self.material_for_submaterial(
                    sidecar_path,
                    sidecar,
                    submaterial,
                    palette,
                    ui_image_path=ui_image_path,
                )
                slot = obj.material_slots[slot_index]
                replaced_material = slot.material
                slot.link = "OBJECT"
                slot.material = material
                if replaced_material is not material:
                    self._remove_replaced_slot_material(replaced_material)
                applied += 1
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
            self._restore_generated_decal_host_variant_polygons(
                obj,
                protected_slot_count=min(len(slot_mapping), len(obj.material_slots)),
            )
            # Option E2-Lite: after every slot is assigned, rebind decal
            # slots to per-host-channel clones so each decal picks up the
            # palette colour of the nearest paint material on the mesh.
            self._rebind_mesh_decal_for_host(
                obj,
                palette,
                host_channel=precomputed_host_channel,
                fallback_rgb=precomputed_host_rgb,
            )
            material_slot_layout_cache[layout_key] = (
                tuple(
                    obj.material_slots[index].material
                    for index in range(min(len(slot_mapping), len(obj.material_slots)))
                ),
                applied,
            )
            return applied
        assigned_slot_count = 0
        for submaterial in sorted(sidecar.submaterials, key=lambda item: item.index):
            if mesh_materials is not None:
                while len(mesh_materials) <= submaterial.index:
                    mesh_materials.append(None)
            if submaterial.index >= len(obj.material_slots):
                print(
                    f"StarBreaker: submaterial index {submaterial.index} exceeds material slot count for {obj.name}"
                )
                continue
            material = self.material_for_submaterial(
                sidecar_path,
                sidecar,
                submaterial,
                palette,
                ui_image_path=ui_image_path,
            )
            slot = obj.material_slots[submaterial.index]
            replaced_material = slot.material
            slot.link = "OBJECT"
            slot.material = material
            if replaced_material is not material:
                self._remove_replaced_slot_material(replaced_material)
            applied += 1
            assigned_slot_count = max(assigned_slot_count, submaterial.index + 1)
        if effective_palette_id is not None:
            obj[PROP_PALETTE_ID] = effective_palette_id
        self._restore_generated_decal_host_variant_polygons(
            obj,
            protected_slot_count=assigned_slot_count,
        )
        # Option E2-Lite: after every slot is assigned, rebind decal
        # slots to per-host-channel clones so each decal picks up the
        # palette colour of the nearest paint material on the mesh.
        self._rebind_mesh_decal_for_host(
            obj,
            palette,
            host_channel=precomputed_host_channel,
            fallback_rgb=precomputed_host_rgb,
        )
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
        # If a previous instantiate_template deferred a view_layer.update(),
        # flush it now — we may be about to read ``parent_node.matrix_world``.
        if parent_node is not None:
            self._flush_pending_view_layer_update()
        effective_palette_id = self._effective_palette_id(self._palette_id_for_instance(record))
        anchor = bpy.data.objects.new(record.entity_name, None)
        anchor.empty_display_type = "PLAIN_AXES"
        self.collection.objects.link(anchor)

        target_parent = parent_node or parent
        anchor.parent = target_parent
        anchor.rotation_mode = "QUATERNION"
        if record.local_transform_sc is not None and record.source_transform_basis == "cryengine_z_up":
            anchor.matrix_basis = _scene_matrix_to_blender(record.local_transform_sc)
        elif record.local_transform_sc is not None and record.source_transform_basis == "gltf_y_up":
            anchor.matrix_basis = _gltf_matrix_to_blender_basis(record.local_transform_sc)
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
            desired_rotation = mathutils.Euler(tuple(math.radians(value) for value in record.offset_rotation), "XYZ").to_quaternion()
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

        # When attached to a parent_node that itself carries a non-identity local
        # rotation (e.g. a ``*_attach`` helper inside a loadout component), the
        # template's ``CryEngine_Z_up`` wrapper would apply its glTF→Blender axis
        # conversion in the rotated parent frame, double-applying the rotation
        # and flipping child geometry (e.g. missiles ending up pointing
        # ship-backward). Force-neutralize the wrapper for those cases. Top-level
        # instances and entities attached to identity-rotation body parts
        # (e.g. ``RSI_Scorpius.001``) keep the original guard so the wrapper
        # composes correctly with offset_rotation-driven anchor placements.
        force_neutralize = False
        if parent_node is not None:
            parent_local_quat = parent_node.matrix_basis.to_quaternion()
            # Identity quat (within tolerance) means no extra parent-frame rotation.
            if (
                abs(parent_local_quat.w - 1.0) > 1e-4
                or abs(parent_local_quat.x) > 1e-4
                or abs(parent_local_quat.y) > 1e-4
                or abs(parent_local_quat.z) > 1e-4
            ):
                force_neutralize = True
        has_authored_offset_rotation = any(abs(value) > 1e-6 for value in record.offset_rotation)
        clones = self.instantiate_template(
            template,
            anchor,
            neutralize_axis_root=parent_node is not None and (force_neutralize or not has_authored_offset_rotation),
            force_neutralize_axis_root=force_neutralize,
        )
        self._apply_instance_metadata([anchor, *clones], record, effective_palette_id)

        for clone in clones:
            self.rebuild_object_materials(clone, effective_palette_id)
        return anchor, clones

    def import_interior_container(self, interior: Any, package_root: bpy.types.Object) -> bpy.types.Object:
        anchor_name = interior.name if interior.name.startswith("interior_") else f"interior_{interior.name}"
        anchor = bpy.data.objects.new(anchor_name, None)
        anchor.empty_display_type = "CUBE"
        parent = package_root
        if interior.parent_entity_name:
            parent_nodes = self.node_index_by_entity_name.get(interior.parent_entity_name, {})
            parent = (
                parent_nodes.get(interior.parent_node_name or "")
                or parent_nodes.get(interior.parent_entity_name)
                or package_root
            )
        anchor.parent = parent
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
            placement_decal_host_channel = None
            placement_decal_host_rgb = None
            if isinstance(placement.raw, dict):
                placement_palette_id = placement.raw.get("palette_id")
                channel_value = placement.raw.get("decal_host_channel")
                if channel_value:
                    placement_decal_host_channel = str(channel_value)
                rgb_value = placement.raw.get("decal_host_rgb")
                if isinstance(rgb_value, (list, tuple)) and len(rgb_value) >= 3:
                    try:
                        placement_decal_host_rgb = (
                            float(rgb_value[0]),
                            float(rgb_value[1]),
                            float(rgb_value[2]),
                        )
                    except (TypeError, ValueError):
                        placement_decal_host_rgb = None
            effective_placement_palette = placement_palette_id or interior.palette_id

            instance = SceneInstanceRecord(
                entity_name=placement.entity_class_guid or Path(placement.cgf_path or "interior").stem,
                geometry_path=placement.cgf_path,
                material_path=placement.material_path,
                material_sidecar=placement.material_sidecar,
                mesh_asset=placement.mesh_asset,
                palette_id=effective_placement_palette,
                ui_bindings=list(placement.ui_bindings),
                decal_host_channel=placement_decal_host_channel,
                decal_host_rgb=placement_decal_host_rgb,
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

        # Flush any deferred view_layer.update() once at the end of the
        # placement loop instead of per placement.
        self._flush_pending_view_layer_update()
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
        semantic_light_kind = str(getattr(light, "semantic_light_kind", "") or "").strip().lower()
        light_data = bpy.data.lights.new(name=light.name or "StarBreaker Light", type=blender_light_type)
        light_data.energy = _light_energy_to_blender(
            active_intensity_candela_proxy
            if active_intensity_candela_proxy is not None
            else light_intensity_candela_proxy
            if light_intensity_candela_proxy is not None
            else 0.0,
            blender_light_type,
            intensity_raw=active_intensity_raw,
            semantic_light_kind=semantic_light_kind,
        )
        light_data.color = light.color
        light_data["starbreaker_light_semantic_kind"] = semantic_light_kind
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
        # shadow edges aren't pin-sharp. Map from authored attenuation
        # radius to avoid a fixed constant across all fixtures.
        if blender_light_type in {"POINT", "SPOT"} and hasattr(light_data, "shadow_soft_size"):
            light_data.shadow_soft_size = (max(float(light.radius or 0.0), 0.0) * 0.05)
            light_data.shadow_soft_size = min(max(light_data.shadow_soft_size, 0.01), 0.5)

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
                        "intensity_raw": s.intensity_raw,
                        "intensity_cd": s.intensity_cd,
                        "intensity_candela_proxy": s.intensity_candela_proxy,
                        "temperature": s.temperature,
                        "use_temperature": s.use_temperature,
                        "color": list(s.color),
                        "light_style": int(getattr(s, "light_style", 0) or 0),
                        "preset_tag": getattr(s, "preset_tag", None),
                    }
                    for name, s in states.items()
                }
            )
            light_data[PROP_LIGHT_ACTIVE_STATE] = str(getattr(light, "active_state", "") or "")

        light_object = bpy.data.objects.new(light.name or "StarBreaker Light", light_data)
        light_object.parent = parent
        light_object.location = _scene_position_to_blender(light.position)
        light_object.rotation_mode = "QUATERNION"
        direction_quaternion = None
        if blender_light_type in {"SPOT", "SUN"}:
            direction_quaternion = self._scene_light_direction_to_blender(getattr(light, "direction_sc", None))
        light_object.rotation_quaternion = direction_quaternion or _scene_light_quaternion_to_blender(light.rotation)
        self.collection.objects.link(light_object)
        return light_object

    def _scene_light_direction_to_blender(
        self,
        direction_sc: tuple[float, float, float] | None,
    ) -> mathutils.Quaternion | None:
        if direction_sc is None:
            return None
        dx, dy, dz = _scene_position_to_blender(direction_sc)
        length = math.sqrt(dx * dx + dy * dy + dz * dz)
        if length <= 1e-8:
            return None
        target = (dx / length, dy / length, dz / length)
        source = (0.0, 0.0, -1.0)
        dot = source[0] * target[0] + source[1] * target[1] + source[2] * target[2]
        if dot > 0.999999:
            return mathutils.Quaternion((1.0, 0.0, 0.0, 0.0))
        if dot < -0.999999:
            return mathutils.Quaternion((0.0, 0.0, 1.0, 0.0))
        cross = (
            source[1] * target[2] - source[2] * target[1],
            source[2] * target[0] - source[0] * target[2],
            source[0] * target[1] - source[1] * target[0],
        )
        return mathutils.Quaternion((1.0 + dot, cross[0], cross[1], cross[2])).normalized()

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

        imported = self._load_template_asset(asset_path)
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
            if self.template_collection is None:
                raise RuntimeError("Template collection was not created; cannot import mesh templates during materials-only refresh")
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

    def _load_template_asset(self, asset_path: Path) -> list[bpy.types.Object]:
        if asset_path.suffix.lower() == ".blend":
            return self._load_blend_template_asset(asset_path)
        return self._load_gltf_template_asset(asset_path)

    def _load_blend_template_asset(self, asset_path: Path) -> list[bpy.types.Object]:
        with bpy.data.libraries.load(str(asset_path), link=False) as (data_from, data_to):
            data_to.objects = list(data_from.objects)
        return [obj for obj in data_to.objects if obj is not None]

    def _load_gltf_template_asset(self, asset_path: Path) -> list[bpy.types.Object]:
        # Phase R3 perf fix: pass ``import_select_created_objects=False``
        # to skip the Blender glTF addon's post-import O(n) selection
        # pass directly via the official op param (replacing the prior
        # monkey-patch on ``BlenderScene.select_imported_objects``). With
        # hundreds of small template imports against a growing scene the
        # selection step was costing ~24s on a Clipper import; the
        # StarBreaker addon never reads selection state.
        before = {obj.as_pointer() for obj in bpy.data.objects}
        result = bpy.ops.import_scene.gltf(
            filepath=str(asset_path),
            import_pack_images=False,
            merge_vertices=False,
            import_select_created_objects=False,
        )
        if "FINISHED" not in result:
            raise RuntimeError(f"Failed to import {asset_path}")
        return [obj for obj in bpy.data.objects if obj.as_pointer() not in before]

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
                clone.matrix_local = mathutils.Matrix.Identity(4)
                needs_view_layer_update = True
            clones.append(clone)
        if needs_view_layer_update:
            # Defer to a batched flush. See ``_flush_pending_view_layer_update``
            # and the docstring above on ``_pending_view_layer_update``.
            self._pending_view_layer_update = True
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
        OrchestrationMixin._normalize_template_decal_offset_modifier(clone)
        clone.hide_set(False)
        clone.hide_render = False
        clone[PROP_TEMPLATE_PATH] = mesh_asset
        source_node_name = str(source.get(PROP_SOURCE_NODE_NAME, source.name) or source.name)
        clone[PROP_SOURCE_NODE_NAME] = source_node_name
        hide_by_default = self._should_hide_source_node_by_default(source_node_name)
        (link_collection or self.collection).objects.link(clone)
        if hide_by_default:
            clone.hide_viewport = True
            clone.hide_render = True
        clone.matrix_basis = source.matrix_basis.copy()
        mapping[source.name] = clone

        for child in source.children:
            if child.get(PROP_TEMPLATE_PATH) != mesh_asset:
                continue
            child_clone = self._duplicate_object_tree(child, mesh_asset, mapping, link_collection)
            child_clone.parent = clone
            child_clone.matrix_parent_inverse = child.matrix_parent_inverse.copy()
        return clone

    @staticmethod
    def _normalize_template_decal_offset_modifier(obj: bpy.types.Object) -> None:
        modifiers = getattr(obj, "modifiers", None)
        if modifiers is None:
            return
        dimensions = tuple(float(value) for value in getattr(obj, "dimensions", ()) if float(value) > 0.0)
        relative_strength = min(dimensions) * 0.005 if dimensions else DECAL_OFFSET_TEMPLATE_CAP
        normalized_strength = min(DECAL_OFFSET_TEMPLATE_CAP, max(0.00001, relative_strength))
        for modifier in modifiers:
            if (
                getattr(modifier, "name", "") == DECAL_OFFSET_MODIFIER_NAME
                and getattr(modifier, "type", "") == "DISPLACE"
            ):
                modifier.strength = min(float(getattr(modifier, "strength", normalized_strength)), normalized_strength)

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
            "decal_host_channel": record.decal_host_channel,
            "decal_host_rgb": record.decal_host_rgb,
        }, sort_keys=True)
        port_flags = {part.strip().lower() for part in record.port_flags.split() if part.strip()}
        hidden_by_port = "invisible" in port_flags
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
            if record.decal_host_channel:
                obj[PROP_DECAL_HOST_CHANNEL] = record.decal_host_channel
            if record.decal_host_rgb is not None:
                obj[PROP_DECAL_HOST_RGB] = [float(component) for component in record.decal_host_rgb]
            obj[PROP_INSTANCE_JSON] = serialized
            if hidden_by_port:
                obj.hide_viewport = True
                obj.hide_render = True
                obj.hide_set(True)

    def _create_package_root(self, palette_id: str | None = None) -> bpy.types.Object:
        package_root = bpy.data.objects.new(f"{PACKAGE_ROOT_PREFIX} {self.package.package_name}", None)
        package_root.empty_display_type = "ARROWS"
        package_root[PROP_PACKAGE_ROOT] = True
        package_root[PROP_SCENE_PATH] = str(self.package.scene_path)
        package_root[PROP_EXPORT_ROOT] = str(self.package.export_root)
        package_root[PROP_PACKAGE_NAME] = self.package.package_name
        assembly_kind = str(getattr(self.package.scene, "raw", {}).get("assembly_kind", "") or "")
        if assembly_kind:
            package_root[PROP_ASSEMBLY_KIND] = assembly_kind
        package_root[PROP_PALETTE_ID] = palette_id or self.package.scene.root_entity.palette_id or ""
        package_root[PROP_PALETTE_SCOPE] = uuid.uuid4().hex
        engine_glow_control = getattr(self.package.scene, "engine_glow_control", None)
        if engine_glow_control is not None:
            package_root[PROP_ENGINE_GLOW_CONTROL_JSON] = json.dumps(engine_glow_control.raw, separators=(",", ":"), sort_keys=True)
            package_root[PROP_ENGINE_GLOW_STRENGTH] = float(engine_glow_control.default_strength)
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


    def _should_hide_source_node_by_default(self, source_node_name: str) -> bool:
        name = source_node_name.strip().lower()
        return (
            name.startswith("damage_")
            or name.startswith("debris_")
            or name.startswith("helper_")
        )

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
