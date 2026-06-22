"""Public entry points and package-lifecycle helpers.

Extracted in Phase 7.4. These are the functions the rest of the add-on
(``ui.py``, operators) calls into. They orchestrate
:class:`PackageImporter` (which still lives in ``_legacy.py`` for now).

``PackageImporter`` is imported lazily inside each function to avoid a
circular import between this module and ``_legacy``.
"""

from __future__ import annotations

import json
import math
import re
import time
import uuid
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import Any, Callable

import bpy

from ..manifest import PackageBundle, SceneInstanceRecord
from ..palette import palette_id_for_livery_instance, resolved_palette_id
from .constants import (
    DECAL_OFFSET_EXTERNAL_DEFAULT,
    DECAL_OFFSET_INTERNAL_DEFAULT,
    DECAL_OFFSET_MODIFIER_NAME,
    PROP_DECAL_OFFSET_EXTERNAL,
    PROP_DECAL_OFFSET_INTERNAL,
    PROP_ENGINE_GLOW_CONTROL_JSON,
    PROP_ENGINE_GLOW_STRENGTH,
    PROP_INSTANCE_JSON,
    PROP_IMPORTED_SLOT_MAP,
    PROP_LIGHT_ACTIVE_STATE,
    PROP_LIGHT_SEMANTIC_KIND,
    PROP_LIGHT_STATES_JSON,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_MESH_ASSET,
    PROP_PACKAGE_ROOT,
    PROP_PAINT_VARIANT_SIDECAR,
    PROP_PALETTE_ID,
    PROP_SCENE_PATH,
    PROP_SHARED_GLOW_CONTROL_JSON,
    PROP_SHARED_GLOW_STRENGTH,
    PROP_SOURCE_NODE_NAME,
    PROP_SUBMATERIAL_JSON,
    PROP_TEMPLATE_PATH,
)
from .animation_fps import (
    FPS_POLICY_ADAPT_SCENE,
    describe_reconciliation,
    extend_frame_end,
    reconcile_animation_fps,
)
from .validators import (
    _purge_orphaned_file_backed_images,
    _purge_orphaned_managed_materials,
    _purge_orphaned_runtime_actions,
    _purge_orphaned_runtime_groups,
)


def import_package(
    context: bpy.types.Context,
    scene_path: str | Path,
    prefer_cycles: bool = True,
    palette_id: str | None = None,
    progress_callback: Callable[[float, str], None] | None = None,
) -> bpy.types.Object:
    from .importer import PackageImporter

    package = PackageBundle.load(scene_path)
    _remove_default_startup_cube()
    _remove_existing_package_instances(package.scene_path)
    importer = PackageImporter(context, package, progress_callback=progress_callback)
    with _suspend_heavy_viewports(context):
        root = importer.import_scene(prefer_cycles=prefer_cycles, palette_id=palette_id)
    _purge_orphaned_runtime_groups()
    _purge_orphaned_file_backed_images()
    return root


def find_package_root(obj: bpy.types.Object | None) -> bpy.types.Object | None:
    current = obj
    while current is not None:
        if bool(current.get(PROP_PACKAGE_ROOT)):
            return current
        current = current.parent
    return None


def _normalized_scene_path(scene_path: str | Path) -> str:
    return str(Path(scene_path).expanduser().resolve())


def _existing_package_roots(scene_path: str | Path) -> list[bpy.types.Object]:
    normalized_scene_path = _normalized_scene_path(scene_path)
    roots: list[bpy.types.Object] = []
    for obj in bpy.data.objects:
        if not bool(obj.get(PROP_PACKAGE_ROOT)):
            continue
        existing_scene_path = _string_prop(obj, PROP_SCENE_PATH)
        if existing_scene_path is None:
            continue
        if _normalized_scene_path(existing_scene_path) == normalized_scene_path:
            roots.append(obj)
    return roots


def _remove_existing_package_instances(scene_path: str | Path) -> int:
    removed = 0
    for package_root in _existing_package_roots(scene_path):
        for obj in reversed(_iter_package_objects(package_root)):
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1
    return removed


def _remove_default_startup_cube() -> int:
    """Drop Blender's untouched startup Cube before package import.

    StarBreaker import supports replacing an existing package inside a
    working scene, so the importer must not call ``read_homefile`` here.  The
    only safe automatic cleanup is the unmodified default Cube that Blender
    creates in a new file before any package is imported.
    """

    removed = 0
    for obj in list(getattr(bpy.data, "objects", []) or []):
        if not _is_default_startup_cube(obj):
            continue
        bpy.data.objects.remove(obj, do_unlink=True)
        removed += 1
    return removed


def _is_default_startup_cube(obj: bpy.types.Object) -> bool:
    if getattr(obj, "name", "") != "Cube":
        return False
    if getattr(obj, "type", None) != "MESH":
        return False
    if getattr(obj, "parent", None) is not None:
        return False
    if list(getattr(obj, "children", []) or []):
        return False
    if list(getattr(obj, "modifiers", []) or []):
        return False
    if any(getattr(slot, "material", None) is not None for slot in getattr(obj, "material_slots", []) or []):
        return False
    if hasattr(obj, "keys") and list(obj.keys()):
        return False

    data = getattr(obj, "data", None)
    if data is None or getattr(data, "name", "") != "Cube":
        return False
    if not _sequence_matches(getattr(obj, "location", None), (0.0, 0.0, 0.0)):
        return False
    if not _sequence_matches(getattr(obj, "rotation_euler", None), (0.0, 0.0, 0.0)):
        return False
    if not _sequence_matches(getattr(obj, "scale", None), (1.0, 1.0, 1.0)):
        return False
    vertices = getattr(data, "vertices", None)
    polygons = getattr(data, "polygons", None)
    if vertices is not None and len(vertices) != 8:
        return False
    if polygons is not None and len(polygons) != 6:
        return False
    return True


def _sequence_matches(value: Any, expected: tuple[float, ...], *, epsilon: float = 1e-6) -> bool:
    if value is None:
        return True
    try:
        values = tuple(float(value[index]) for index in range(len(expected)))
    except Exception:
        return False
    return all(abs(actual - target) <= epsilon for actual, target in zip(values, expected))


def _exterior_material_sidecars(package: PackageBundle) -> set[str] | None:
    """Return the set of material sidecar paths from the exterior livery group.

    The exterior group is the one whose material_sidecars include the root
    entity's sidecar.  Returns None if livery data is absent or unresolvable
    (caller falls back to applying to all materials).
    """
    if not package.liveries:
        return None
    root_sidecar = package.scene.root_entity.material_sidecar
    if not root_sidecar:
        return None
    for livery in package.liveries.values():
        if root_sidecar in livery.material_sidecars:
            return set(livery.material_sidecars)
    return None


def _effective_exterior_material_sidecars(
    package: PackageBundle,
    package_root: bpy.types.Object | None,
) -> set[str] | None:
    """Return the exterior sidecar set, extended with any active paint variant sidecar.

    When a paint variant with a different material file is active, its sidecar
    is stored on the package root object.  This helper ensures that
    palette-change operations also reach materials that were rebuilt from that
    variant sidecar.
    """
    base = _exterior_material_sidecars(package)
    paint_sidecar = _string_prop(package_root, PROP_PAINT_VARIANT_SIDECAR) if package_root is not None else None
    if paint_sidecar is None:
        return base
    if base is None:
        return {paint_sidecar}
    return base | {paint_sidecar}


def exterior_palette_ids(package: PackageBundle) -> list[str]:
    """Return palette IDs applicable to the exterior livery group.

    Includes both palette-based IDs (from palettes.json) and paint-variant IDs
    (from paints.json), minus any IDs that are interior-only.
    """
    all_ids = set(package.palettes.keys()) | set(package.paints.keys())
    if not all_ids:
        return []
    if not package.liveries:
        return sorted(all_ids)
    exterior_sidecars = _exterior_material_sidecars(package)
    if exterior_sidecars is None:
        return sorted(all_ids)
    interior_only_palette_ids: set[str] = set()
    for livery in package.liveries.values():
        if not set(livery.material_sidecars).intersection(exterior_sidecars):
            if livery.palette_id:
                interior_only_palette_ids.add(livery.palette_id)
    return sorted(pid for pid in all_ids if pid not in interior_only_palette_ids)


def _paint_variant_for_palette_id(package: PackageBundle, palette_id: str | None) -> Any | None:
    if not palette_id:
        return None
    direct = package.paints.get(palette_id)
    if direct is not None:
        return direct
    canonical_id = resolved_palette_id(package, palette_id)
    if canonical_id is None:
        return None
    for candidate_id, variant in package.paints.items():
        if resolved_palette_id(package, candidate_id) == canonical_id:
            return variant
    return None


def _restore_paint_object_sidecar(instance: SceneInstanceRecord | None, target_sidecar: str | None) -> str | None:
    """Return the material sidecar an exterior object should use for a paint switch.

    When a paint variant carries its own material sidecar, every exterior mesh
    is rebuilt from that variant file. Switching back to a paint that does not
    provide a variant sidecar must restore each object's original per-instance
    sidecar from the scene record rather than taking the palette-only fast path.
    """

    if target_sidecar:
        return target_sidecar
    if instance is None:
        return None
    sidecar = getattr(instance, "material_sidecar", None)
    return sidecar if isinstance(sidecar, str) and sidecar else None


def apply_palette_to_selected_package(context: bpy.types.Context, palette_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_palette_to_package_root(context, package_root, palette_id)


def apply_paint_to_selected_package(context: bpy.types.Context, palette_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_paint_to_package_root(context, package_root, palette_id)


def apply_livery_to_selected_package(context: bpy.types.Context, livery_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_livery_to_package_root(context, package_root, livery_id)


def refresh_materials_for_package_root(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    palette_id: str | None = None,
    *,
    only_unloaded: bool = False,
    target_objects: list[bpy.types.Object] | None = None,
    progress_callback: Callable[[float, str], None] | None = None,
    progress_interval_seconds: float = 0.0,
) -> int:
    from .importer import PackageImporter

    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root, create_template_collection=False)
    applied = 0
    needs_view_layer_update = False
    sidecar_cache: dict[str, Any] = {}
    objects = target_objects if target_objects is not None else list(_iter_package_objects(package_root))
    work_items: list[tuple[bpy.types.Object, str, Any]] = []
    for obj in objects:
        if getattr(obj, "type", None) != "MESH":
            continue
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        if sidecar_path is None:
            continue
        sidecar = _material_refresh_sidecar(package, sidecar_cache, sidecar_path)
        if only_unloaded and target_objects is None and not _object_needs_material_refresh(obj, sidecar):
            continue
        work_items.append((obj, sidecar_path, sidecar))
    total_work_items = len(work_items)
    last_progress_update = time.monotonic()
    if progress_callback is not None:
        progress_callback(0.0, f"Refreshing 0/{total_work_items} objects")
    with _suspend_heavy_viewports(context), _temporary_object_mode(context):
        for index, (obj, _sidecar_path, _sidecar) in enumerate(work_items, start=1):
            object_palette_id = _material_refresh_palette_id(package, obj, palette_id)
            applied += importer.rebuild_object_materials(obj, object_palette_id)
            needs_view_layer_update = _refresh_mesh_material_evaluation(obj) or needs_view_layer_update
            if palette_id is not None:
                obj[PROP_PALETTE_ID] = palette_id
            if progress_callback is not None:
                now = time.monotonic()
                if (
                    index == total_work_items
                    or now - last_progress_update >= progress_interval_seconds
                ):
                    last_progress_update = now
                    progress_callback(
                        0.95 * (index / max(total_work_items, 1)),
                        f"Refreshing {index}/{total_work_items} objects",
                    )
        if palette_id is not None:
            package_root[PROP_PALETTE_ID] = palette_id
    if needs_view_layer_update:
        view_layer = getattr(context, "view_layer", None)
        update = getattr(view_layer, "update", None)
        if callable(update):
            update()
    if engine_glow_control_enabled(package_root):
        apply_engine_glow_to_package_root(package_root, engine_glow_strength(package_root))
    if shared_glow_control_enabled(package_root):
        apply_shared_glow_to_package_root(package_root, shared_glow_strength(package_root))
    if progress_callback is not None:
        progress_callback(0.95, "Cleaning up...")
    _purge_orphaned_managed_materials()
    _purge_orphaned_runtime_groups()
    _purge_orphaned_runtime_actions()
    _purge_orphaned_file_backed_images()
    if progress_callback is not None:
        progress_callback(1.0, "Done")
    return applied


class MaterialRefreshSession:
    """Incremental material refresh for post-file-open timer execution."""

    def __init__(
        self,
        context: bpy.types.Context,
        package_root: bpy.types.Object,
        palette_id: str | None = None,
        *,
        purge_orphans: bool = True,
    ) -> None:
        from .importer import PackageImporter

        self.context = context
        self.package_root = package_root
        self.package = _load_package_from_root(package_root)
        self.importer = PackageImporter(
            context,
            self.package,
            package_root=package_root,
            create_template_collection=False,
        )
        self.palette_id = palette_id
        self.objects = [
            obj
            for obj in _iter_package_objects(package_root)
            if (
                getattr(obj, "type", None) == "MESH"
                and _string_prop(obj, PROP_MATERIAL_SIDECAR) is not None
            )
        ]
        self.index = 0
        self.applied = 0
        self.needs_view_layer_update = False
        self.done = False
        self.purge_orphans = purge_orphans
        self._objects_finalized = False
        self._cleanup_steps: list[Callable[[], int]] = (
            [
                _purge_orphaned_managed_materials,
                _purge_orphaned_runtime_groups,
                _purge_orphaned_runtime_actions,
                _purge_orphaned_file_backed_images,
            ]
            if purge_orphans
            else []
        )
        self._cleanup_index = 0
        self.default_palette_cache: dict[str, str | None] = {}
        self._context_stack = ExitStack()
        self._context_stack.enter_context(_suspend_heavy_viewports(context))
        self._context_stack.enter_context(_temporary_object_mode(context))

    @property
    def total(self) -> int:
        return len(self.objects)

    @property
    def progress(self) -> float:
        if self.done:
            return 1.0
        if self.total == 0:
            object_progress = 1.0
        else:
            object_progress = min(1.0, self.index / self.total)
        if not self._cleanup_steps:
            return object_progress
        if not self._objects_finalized:
            return min(0.95, object_progress * 0.95)
        cleanup_progress = self._cleanup_index / len(self._cleanup_steps)
        return min(0.99, 0.95 + cleanup_progress * 0.05)

    def step(self, budget_seconds: float = 0.05, min_objects: int = 1) -> bool:
        if self.done:
            return True

        deadline = time.perf_counter() + max(0.001, budget_seconds)
        processed = 0
        while not self._objects_finalized and self.index < len(self.objects):
            obj = self.objects[self.index]
            self.index += 1
            processed += 1

            if getattr(obj, "type", None) != "MESH":
                if processed >= min_objects and time.perf_counter() >= deadline:
                    break
                continue
            if _string_prop(obj, PROP_MATERIAL_SIDECAR) is None:
                if processed >= min_objects and time.perf_counter() >= deadline:
                    break
                continue

            object_palette_id = _material_refresh_palette_id(
                self.package,
                obj,
                self.palette_id,
                self.default_palette_cache,
            )
            self.applied += self.importer.rebuild_object_materials(obj, object_palette_id)
            self.needs_view_layer_update = _refresh_mesh_material_evaluation(obj) or self.needs_view_layer_update
            if self.palette_id is not None:
                obj[PROP_PALETTE_ID] = self.palette_id

            if processed >= min_objects and time.perf_counter() >= deadline:
                break

        if not self._objects_finalized and self.index >= len(self.objects):
            self._finish_objects()
            if not self._cleanup_steps:
                self.done = True
                return True
            if processed >= min_objects and time.perf_counter() >= deadline:
                return False

        while self._cleanup_index < len(self._cleanup_steps):
            self._cleanup_steps[self._cleanup_index]()
            self._cleanup_index += 1
            if self._cleanup_index >= len(self._cleanup_steps):
                break
            if time.perf_counter() >= deadline:
                return False

        if self._objects_finalized:
            self.done = True
            return True
        return False

    def _finish_objects(self) -> None:
        if self._objects_finalized:
            return
        try:
            flush_orphans = getattr(self.importer, "_flush_pending_orphan_materials", None)
            if callable(flush_orphans):
                flush_orphans()
            if self.palette_id is not None:
                self.package_root[PROP_PALETTE_ID] = self.palette_id
            if engine_glow_control_enabled(self.package_root):
                apply_engine_glow_to_package_root(self.package_root, engine_glow_strength(self.package_root))
            if shared_glow_control_enabled(self.package_root):
                apply_shared_glow_to_package_root(self.package_root, shared_glow_strength(self.package_root))
            if self.needs_view_layer_update:
                view_layer = getattr(self.context, "view_layer", None)
                update = getattr(view_layer, "update", None) if view_layer is not None else None
                if callable(update):
                    update()
            self._objects_finalized = True
        finally:
            self._context_stack.close()

    def finish(self) -> None:
        if self.done:
            return
        self._finish_objects()
        while self._cleanup_index < len(self._cleanup_steps):
            self._cleanup_steps[self._cleanup_index]()
            self._cleanup_index += 1
        self.done = True


def _material_refresh_palette_id(
    package: PackageBundle,
    obj: bpy.types.Object,
    explicit_palette_id: str | None,
    default_palette_cache: dict[str, str | None] | None = None,
) -> str | None:
    if explicit_palette_id is not None:
        return explicit_palette_id
    object_palette_id = _string_prop(obj, PROP_PALETTE_ID)
    if object_palette_id is not None:
        return object_palette_id
    sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
    if sidecar_path is None:
        return None
    if default_palette_cache is not None and sidecar_path in default_palette_cache:
        return default_palette_cache[sidecar_path]
    palette_id = _sidecar_default_palette_id(package, sidecar_path)
    if default_palette_cache is not None:
        default_palette_cache[sidecar_path] = palette_id
    return palette_id


def _sidecar_default_palette_id(package: PackageBundle, sidecar_path: str) -> str | None:
    load_sidecar = getattr(package, "load_material_sidecar", None)
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
        if not name.lower() == "defaultpalette":
            continue
        value = str(attribute.get("value", "")).replace("\\", "/").strip()
        source_name = value.rsplit("/", 1)[-1].strip().lower()
        if source_name:
            return f"palette/{source_name}"
    return None


def _refresh_mesh_material_evaluation(
    obj: bpy.types.Object,
    _context: bpy.types.Context | None = None,
) -> bool:
    data = getattr(obj, "data", None)
    changed = _ensure_active_uv_layer(data)
    data_tagged = _tag_id_for_refresh(data)
    object_tagged = _tag_id_for_refresh(obj)
    return changed or data_tagged or object_tagged


def _ensure_active_uv_layer(mesh: Any) -> bool:
    uv_layers = getattr(mesh, "uv_layers", None)
    if uv_layers is None or len(uv_layers) == 0:
        return False
    preferred_index = _uv_layer_index(uv_layers, "UVMap")
    target_index = preferred_index if preferred_index is not None else 0
    changed = False
    active = getattr(uv_layers, "active", None)
    active_index = int(getattr(uv_layers, "active_index", -1))
    if active is None or active_index != target_index:
        uv_layers.active_index = target_index
        changed = True
    target_layer = uv_layers[target_index]
    if hasattr(target_layer, "active_render") and not bool(getattr(target_layer, "active_render")):
        target_layer.active_render = True
        changed = True
    return changed


def _uv_layer_index(uv_layers: Any, name: str) -> int | None:
    find = getattr(uv_layers, "find", None)
    if callable(find):
        index = int(find(name))
        if index >= 0:
            return index
    for index, layer in enumerate(uv_layers):
        if getattr(layer, "name", None) == name:
            return index
    return None


def _tag_id_for_refresh(data_block: Any) -> bool:
    update_tag = getattr(data_block, "update_tag", None)
    if not callable(update_tag):
        return False
    try:
        update_tag(refresh={"DATA"})
    except (RuntimeError, TypeError):
        update_tag()
    return True


def package_root_needs_material_refresh(package_root: bpy.types.Object) -> bool:
    return bool(dirty_package_material_objects(package_root, limit=1))


def dirty_package_material_objects(
    package_root: bpy.types.Object,
    *,
    limit: int | None = None,
) -> list[bpy.types.Object]:
    package: PackageBundle | None = None
    sidecar_cache: dict[str, Any] = {}
    dirty_objects: list[bpy.types.Object] = []
    for obj in _iter_package_objects(package_root):
        if getattr(obj, "type", None) != "MESH":
            continue
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        if sidecar_path is None:
            continue
        if package is None:
            package = _load_package_from_root(package_root)
        sidecar = _material_refresh_sidecar(package, sidecar_cache, sidecar_path)
        if _object_needs_material_refresh(obj, sidecar):
            dirty_objects.append(obj)
            if limit is not None and len(dirty_objects) >= limit:
                break
    return dirty_objects


def _material_slot_needs_refresh(material: Any) -> bool:
    if getattr(material, "library", None) is not None:
        return True
    node_tree = getattr(material, "node_tree", None)
    if node_tree is None:
        return True
    nodes = getattr(node_tree, "nodes", None)
    if nodes is not None and len(nodes) == 0:
        return True
    return False


def _object_needs_material_refresh(obj: bpy.types.Object, sidecar: Any) -> bool:
    material_slots = getattr(obj, "material_slots", ())
    if len(material_slots) == 0:
        return _sidecar_has_submaterial_index(sidecar, 0)
    for slot_index, slot in enumerate(material_slots):
        material = getattr(slot, "material", None)
        if material is None or not _material_slot_needs_refresh(material):
            continue
        if _material_slot_can_refresh(obj, slot_index, material, sidecar):
            return True
    return False


def _material_slot_can_refresh(
    obj: bpy.types.Object,
    slot_index: int,
    material: Any,
    sidecar: Any,
) -> bool:
    slot_mapping = _slot_mapping_for_object(obj)
    if slot_mapping is not None and slot_index < len(slot_mapping):
        mapped_index = slot_mapping[slot_index]
        if mapped_index is not None and _sidecar_has_submaterial_index(sidecar, mapped_index):
            return True

    material_name = getattr(material, "name", "")
    if not isinstance(material_name, str) or not material_name:
        return False
    canonical_name = _canonical_source_name(material_name)
    if not canonical_name:
        # An unnamed submaterial (e.g. a UIPlane screen) exports to a material
        # whose name canonicalises to "". Match it by the exporter's 1:1
        # slot->submaterial index — but only when the object carries no slot
        # map. A present map that sends this slot to None is a deliberate
        # "no match" and must not be widened.
        if slot_mapping is None:
            return _sidecar_has_submaterial_index(sidecar, slot_index)
        return False
    if canonical_name in _unique_submaterials_by_name(sidecar):
        return True
    return canonical_name in _submaterials_by_name(sidecar)


def _canonical_source_name(name: str) -> str:
    if len(name) > 4 and name[-4] == "." and name[-3:].isdigit():
        name = name[:-4]
    if "_mtl_" in name:
        material_name = name.rsplit("_mtl_", 1)[1]
        stem, separator, suffix = material_name.rpartition("_")
        if separator and suffix.isdigit():
            name = stem
        else:
            name = material_name
    if ":" in name:
        name = name.rsplit(":", 1)[1]
    if "__" in name:
        name = name.split("__", 1)[0]
    if "#" in name:
        name = name.split("#", 1)[0]
    return name


def _slot_mapping_for_object(obj: bpy.types.Object) -> list[int | None] | None:
    data = getattr(obj, "data", None)
    if data is None:
        return None
    get_prop = getattr(data, "get", None)
    if not callable(get_prop):
        return None
    mapping_raw = get_prop(PROP_IMPORTED_SLOT_MAP)
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


def _unique_submaterials_by_name(sidecar: Any) -> dict[str, Any]:
    grouped: dict[str, list[Any]] = {}
    for submaterial in getattr(sidecar, "submaterials", ()):
        name = getattr(submaterial, "submaterial_name", "").strip()
        if not name:
            continue
        grouped.setdefault(name, []).append(submaterial)
    return {
        name: submaterials[0]
        for name, submaterials in grouped.items()
        if len(submaterials) == 1
    }


def _submaterials_by_name(sidecar: Any) -> dict[str, list[Any]]:
    grouped: dict[str, list[Any]] = {}
    for submaterial in getattr(sidecar, "submaterials", ()):
        name = getattr(submaterial, "submaterial_name", "").strip()
        if not name:
            continue
        grouped.setdefault(name, []).append(submaterial)
    return grouped


def _material_refresh_sidecar(
    package: PackageBundle,
    cache: dict[str, Any],
    sidecar_path: str,
) -> Any:
    if sidecar_path not in cache:
        cache[sidecar_path] = package.load_material_sidecar(sidecar_path)
    return cache[sidecar_path]


def _sidecar_has_submaterial_index(sidecar: Any, index: int) -> bool:
    if sidecar is None:
        return False
    return any(int(getattr(submaterial, "index", -1)) == index for submaterial in sidecar.submaterials)


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
    from .importer import PackageImporter

    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root, create_template_collection=False)
    with _suspend_heavy_viewports(context), _temporary_object_mode(context):
        return importer.apply_palette_to_package_root(package_root, palette_id)


def apply_paint_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, palette_id: str) -> int:
    """Switch to the paint variant whose palette_id matches, rebuilding exterior
    materials from the variant's material sidecar when it differs from the
    current one.

    Falls back to a fast palette-only update when no matching paint variant is
    found or when the variant does not carry a different material sidecar.
    """
    from .importer import PackageImporter

    package = _load_package_from_root(package_root)
    variant = package.paints.get(palette_id)
    target_sidecar = variant.exterior_material_sidecar if variant is not None else None

    active_paint_sidecar = _string_prop(package_root, PROP_PAINT_VARIANT_SIDECAR)
    if target_sidecar is None and active_paint_sidecar is None:
        # No paint-variant sidecar is active or requested: fast palette-only path.
        return apply_palette_to_package_root(context, package_root, palette_id)

    # Determine which objects are currently exterior so we know what to rebuild.
    # We check against both the original livery sidecars AND any previously-active
    # paint variant sidecar so that consecutive paint switches work correctly.
    effective_exterior = _effective_exterior_material_sidecars(package, package_root)
    base_exterior = _exterior_material_sidecars(package)
    check_sidecars = effective_exterior or base_exterior

    importer = PackageImporter(context, package, package_root=package_root, create_template_collection=False)
    applied = 0
    with _suspend_heavy_viewports(context), _temporary_object_mode(context):
        for obj in _iter_package_objects(package_root):
            if obj.type != "MESH":
                continue
            obj_sidecar = _string_prop(obj, PROP_MATERIAL_SIDECAR)
            if check_sidecars is not None and (obj_sidecar is None or obj_sidecar not in check_sidecars):
                continue
            instance = _scene_instance_from_object(obj)
            restored_sidecar = _restore_paint_object_sidecar(instance, target_sidecar)
            if restored_sidecar is None:
                continue
            # Point the object at the target sidecar (or restore its original
            # per-instance sidecar when leaving a variant paint), then rebuild.
            obj[PROP_MATERIAL_SIDECAR] = restored_sidecar
            applied += importer.rebuild_object_materials(obj, palette_id)

    # Record the active paint variant sidecar so palette-only changes still work.
    if target_sidecar is not None:
        package_root[PROP_PAINT_VARIANT_SIDECAR] = target_sidecar
    else:
        package_root.pop(PROP_PAINT_VARIANT_SIDECAR, None)
    package_root[PROP_PALETTE_ID] = palette_id
    _purge_orphaned_runtime_groups()
    _purge_orphaned_file_backed_images()
    return applied


def apply_livery_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, livery_id: str) -> int:
    from .importer import PackageImporter

    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root, create_template_collection=False)
    applied = 0
    with _suspend_heavy_viewports(context), _temporary_object_mode(context):
        for obj in _iter_package_objects(package_root):
            instance = _scene_instance_from_object(obj)
            if instance is None:
                continue
            restored_sidecar = _restore_paint_object_sidecar(instance, None)
            sidecar_for_livery = restored_sidecar or _string_prop(obj, PROP_MATERIAL_SIDECAR)
            effective_palette_id = palette_id_for_livery_instance(
                package,
                livery_id,
                instance,
                sidecar_for_livery,
            )
            if restored_sidecar is not None:
                obj[PROP_MATERIAL_SIDECAR] = restored_sidecar
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
        package_root.pop(PROP_PAINT_VARIANT_SIDECAR, None)
    _purge_orphaned_runtime_groups()
    _purge_orphaned_file_backed_images()
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


@contextmanager
def _temporary_object_mode(context: bpy.types.Context):
    view_layer = getattr(context, "view_layer", None)
    active_object = getattr(view_layer.objects, "active", None) if view_layer is not None else None
    original_mode = getattr(active_object, "mode", "OBJECT") if active_object is not None else "OBJECT"
    switched = False

    def _mode_set(mode: str) -> bool:
        if active_object is None:
            return False
        window = getattr(context, "window", None)
        screen = getattr(window, "screen", None) if window is not None else None
        area = None
        region = None
        if screen is not None:
            area = next((candidate for candidate in screen.areas if candidate.type == "VIEW_3D"), None)
            if area is not None:
                region = next((candidate for candidate in area.regions if candidate.type == "WINDOW"), None)
        override = {
            "active_object": active_object,
            "object": active_object,
            "selected_objects": [active_object],
            "selected_editable_objects": [active_object],
        }
        if window is not None:
            override["window"] = window
        if screen is not None:
            override["screen"] = screen
        if area is not None:
            override["area"] = area
        if region is not None:
            override["region"] = region
        with context.temp_override(**override):
            bpy.ops.object.mode_set(mode=mode)
        return True

    try:
        if active_object is not None and original_mode != "OBJECT":
            switched = _mode_set("OBJECT")
        yield
    finally:
        if switched and active_object is not None and view_layer is not None:
            try:
                if view_layer.objects.active is not active_object:
                    view_layer.objects.active = active_object
                _mode_set(original_mode)
            except Exception:
                pass


def _load_package_from_root(package_root: bpy.types.Object) -> PackageBundle:
    scene_path = resolve_package_scene_path(package_root)
    return PackageBundle.load(scene_path)


def resolve_package_scene_path(package_root: bpy.types.Object) -> Path:
    scene_path = _string_prop(package_root, PROP_SCENE_PATH)
    if scene_path is None:
        raise RuntimeError("Selected object is missing StarBreaker scene metadata")
    raw_path = Path(scene_path)
    if raw_path.is_absolute():
        return raw_path

    blend_file = Path(bpy.data.filepath) if getattr(bpy.data, "filepath", "") else None
    search_roots: list[Path] = []
    if blend_file is not None:
        blend_parent = blend_file.parent
        search_roots.append(blend_parent)
        search_roots.extend(blend_parent.parents)
    search_roots.append(Path.cwd())

    for root in search_roots:
        candidate = (root / raw_path).resolve()
        if candidate.is_file():
            return candidate
    if blend_file is not None:
        return (blend_file.parent / raw_path).resolve()
    return raw_path.resolve()


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


def _float_prop_or_default(obj: object, name: str, default: float) -> float:
    value = getattr(obj, name, None)
    if isinstance(value, (int, float)):
        return float(value)
    get_prop = getattr(obj, "get", None)
    if callable(get_prop):
        value = get_prop(name)
        if isinstance(value, (int, float)):
            return float(value)
    return float(default)


def _decal_offset_modifier(obj: bpy.types.Object) -> Any:
    for modifier in getattr(obj, "modifiers", ()):
        if (
            getattr(modifier, "name", "") == DECAL_OFFSET_MODIFIER_NAME
            and getattr(modifier, "type", "") == "DISPLACE"
        ):
            return modifier
    return None


def _decal_offset_sidecar(obj: bpy.types.Object) -> str | None:
    sidecar = _string_prop(obj, PROP_MATERIAL_SIDECAR)
    if sidecar is not None:
        return sidecar
    instance = _scene_instance_from_object(obj)
    value = getattr(instance, "material_sidecar", None) if instance is not None else None
    return value if isinstance(value, str) and value else None


def _is_exterior_decal_object(obj: bpy.types.Object) -> bool:
    sidecar = _decal_offset_sidecar(obj)
    if not isinstance(sidecar, str) or not sidecar:
        return False
    lower = sidecar.replace("\\", "/").lower()
    return "/ships/" in lower and "_int_" not in lower and "_int_master" not in lower


def decal_offset_control_enabled(package_root: bpy.types.Object) -> bool:
    return any(_decal_offset_modifier(obj) is not None for obj in _iter_package_objects(package_root))


def decal_offset_external_strength(package_root: bpy.types.Object) -> float:
    return _float_prop_or_default(
        package_root,
        PROP_DECAL_OFFSET_EXTERNAL,
        DECAL_OFFSET_EXTERNAL_DEFAULT,
    )


def decal_offset_internal_strength(package_root: bpy.types.Object) -> float:
    return _float_prop_or_default(
        package_root,
        PROP_DECAL_OFFSET_INTERNAL,
        DECAL_OFFSET_INTERNAL_DEFAULT,
    )


def apply_decal_offsets_to_package_root(
    package_root: bpy.types.Object,
    external_strength: float,
    internal_strength: float,
) -> int:
    updated = 0
    external_strength = float(external_strength)
    internal_strength = float(internal_strength)
    for obj in _iter_package_objects(package_root):
        modifier = _decal_offset_modifier(obj)
        if modifier is None:
            continue
        modifier.strength = external_strength if _is_exterior_decal_object(obj) else internal_strength
        updated += 1
    return updated


def _triplet_from_any(value: Any) -> tuple[float, float, float] | None:
    if isinstance(value, (list, tuple)) and len(value) >= 3:
        try:
            return (float(value[0]), float(value[1]), float(value[2]))
        except (TypeError, ValueError):
            return None
    if isinstance(value, str):
        parts = [part.strip() for part in value.split(",")]
        if len(parts) >= 3:
            try:
                return (float(parts[0]), float(parts[1]), float(parts[2]))
            except (TypeError, ValueError):
                return None
    return None


def _submaterial_authored_glow(submaterial: Any) -> float:
    raw = getattr(submaterial, "raw", None)
    if not isinstance(raw, dict):
        return 0.0
    for attribute in raw.get("authored_attributes", []) or []:
        if not isinstance(attribute, dict) or attribute.get("name") != "Glow":
            continue
        try:
            return float(attribute.get("value"))
        except (TypeError, ValueError):
            continue
    return 0.0


_ENGINE_GLOW_OVERRIDE_PROP = "starbreaker_engine_glow_override_material"
_ENGINE_GLOW_OVERRIDE_SIDECAR_PROP = "starbreaker_engine_glow_override_sidecar"
_ENGINE_GLOW_OVERRIDE_INDEX_PROP = "starbreaker_engine_glow_override_source_index"


def _material_submaterial_payload(material: bpy.types.Material) -> dict[str, Any] | None:
    payload = material.get(PROP_SUBMATERIAL_JSON)
    if not isinstance(payload, str):
        return None
    try:
        data = json.loads(payload)
    except (json.JSONDecodeError, TypeError, ValueError):
        return None
    if not isinstance(data, dict):
        return None
    return data


def _material_source_index(material: bpy.types.Material) -> int | None:
    data = _material_submaterial_payload(material)
    if data is None:
        return None
    raw_index = data.get("source_material_index", data.get("index"))
    try:
        return int(raw_index)
    except (TypeError, ValueError):
        return None


def _material_authored_emissive_color(material: bpy.types.Material) -> tuple[float, float, float, float] | None:
    data = _material_submaterial_payload(material)
    if data is None:
        return None
    for attribute in data.get("authored_attributes", []) or []:
        if not isinstance(attribute, dict) or attribute.get("name") != "Emissive":
            continue
        triplet = _triplet_from_any(attribute.get("value"))
        if triplet is None:
            continue
        if all(abs(component) <= 1e-6 for component in triplet):
            return None
        return (*triplet, 1.0)
    return None


def _normalize_engine_glow_path(value: str | None) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    normalized = value.replace("\\", "/")
    if normalized.startswith("Data/"):
        return normalized.casefold()
    return f"Data/{normalized}".casefold()


def _normalize_material_sidecar_path(value: str | None) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    return value.replace("\\", "/").casefold()


def _submaterial_texture_export_path(submaterial: Any, slot_name: str) -> str | None:
    for texture in getattr(submaterial, "texture_slots", []) or []:
        if getattr(texture, "slot", None) != slot_name:
            continue
        export_path = getattr(texture, "export_path", None)
        if isinstance(export_path, str) and export_path:
            return export_path
    return None


def _shared_glow_target_submaterial(submaterial: Any) -> bool:
    if getattr(submaterial, "shader_family", None) != "MeshDecal":
        return False
    decoded = getattr(submaterial, "decoded_feature_flags", None)
    if bool(getattr(decoded, "has_parallax_occlusion_mapping", False)):
        return False
    if bool(getattr(decoded, "has_stencil_map", False)):
        return False
    texslot1 = _submaterial_texture_export_path(submaterial, "TexSlot1")
    if not isinstance(texslot1, str):
        return False
    return "/glows/" in texslot1.replace("\\", "/").casefold()


def _shared_glow_control_payload(package_root: bpy.types.Object) -> dict[str, Any] | None:
    payload = _string_prop(package_root, PROP_SHARED_GLOW_CONTROL_JSON)
    if payload is not None:
        try:
            data = json.loads(payload)
        except (json.JSONDecodeError, TypeError, ValueError):
            data = None
        if isinstance(data, dict):
            return data
    try:
        package = _load_package_from_root(package_root)
    except (FileNotFoundError, json.JSONDecodeError, OSError, TypeError, ValueError):
        return None

    targets: list[dict[str, Any]] = []
    seen_targets: set[tuple[str, int]] = set()
    for obj in _iter_package_objects(package_root):
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        if sidecar_path is None:
            continue
        normalized_sidecar = _normalize_material_sidecar_path(sidecar_path)
        if normalized_sidecar is None:
            continue
        sidecar = package.load_material_sidecar(sidecar_path)
        if sidecar is None:
            continue
        for submaterial in getattr(sidecar, "submaterials", []):
            if not _shared_glow_target_submaterial(submaterial):
                continue
            key = (normalized_sidecar, int(getattr(submaterial, "index", 0)))
            if key in seen_targets:
                continue
            seen_targets.add(key)
            targets.append(
                {
                    "material_sidecar": sidecar_path,
                    "source_material_index": key[1],
                    "submaterial_name": getattr(submaterial, "submaterial_name", None),
                    "blender_material_name": getattr(submaterial, "blender_material_name", None),
                }
            )
    if not targets:
        return None
    data = {
        "label": "Shared Glow",
        "units": "emission_strength_delta",
        "min_strength": 0.0,
        "max_strength": 10.0,
        "default_strength": 0.0,
        "targets": targets,
    }
    package_root[PROP_SHARED_GLOW_CONTROL_JSON] = json.dumps(data, separators=(",", ":"), sort_keys=True)
    if not isinstance(package_root.get(PROP_SHARED_GLOW_STRENGTH), (int, float)):
        package_root[PROP_SHARED_GLOW_STRENGTH] = 0.0
    return data


def _shared_glow_targets(package_root: bpy.types.Object) -> dict[str, set[int]]:
    data = _shared_glow_control_payload(package_root)
    if data is None:
        return {}
    targets = data.get("targets")
    if not isinstance(targets, list):
        return {}
    by_sidecar: dict[str, set[int]] = {}
    for target in targets:
        if not isinstance(target, dict):
            continue
        sidecar_key = _normalize_material_sidecar_path(target.get("material_sidecar"))
        if sidecar_key is None:
            continue
        try:
            source_index = int(target.get("source_material_index", 0))
        except (TypeError, ValueError):
            continue
        by_sidecar.setdefault(sidecar_key, set()).add(source_index)
    return by_sidecar


def _engine_glow_targets(package_root: bpy.types.Object) -> dict[tuple[str, str], set[int]]:
    payload = _string_prop(package_root, PROP_ENGINE_GLOW_CONTROL_JSON)
    if payload is None:
        try:
            package = _load_package_from_root(package_root)
        except (FileNotFoundError, json.JSONDecodeError, OSError, TypeError, ValueError):
            package = None
        control = getattr(getattr(package, "scene", None), "engine_glow_control", None)
        if control is not None:
            payload = json.dumps(control.raw, separators=(",", ":"), sort_keys=True)
            package_root[PROP_ENGINE_GLOW_CONTROL_JSON] = payload
            if not isinstance(package_root.get(PROP_ENGINE_GLOW_STRENGTH), (int, float)):
                package_root[PROP_ENGINE_GLOW_STRENGTH] = float(control.default_strength)
    if payload is None:
        return {}
    try:
        data = json.loads(payload)
    except (json.JSONDecodeError, TypeError, ValueError):
        return {}
    if not isinstance(data, dict):
        return {}
    targets = data.get("targets")
    if not isinstance(targets, list):
        return {}
    by_instance_and_sidecar: dict[tuple[str, str], set[int]] = {}
    for target in targets:
        if not isinstance(target, dict):
            continue
        sidecar_key = _normalize_material_sidecar_path(target.get("material_sidecar"))
        if sidecar_key is None:
            continue
        mesh_asset = _normalize_engine_glow_path(target.get("mesh_asset"))
        if mesh_asset is None:
            continue
        try:
            source_index = int(target.get("source_material_index", 0))
        except (TypeError, ValueError):
            continue
        key = (mesh_asset, sidecar_key)
        by_instance_and_sidecar.setdefault(key, set()).add(source_index)
    return by_instance_and_sidecar


def engine_glow_control_enabled(package_root: bpy.types.Object) -> bool:
    return bool(_engine_glow_targets(package_root))


def engine_glow_strength(package_root: bpy.types.Object) -> float:
    value = package_root.get(PROP_ENGINE_GLOW_STRENGTH)
    if isinstance(value, (int, float)):
        return float(value)
    payload = _string_prop(package_root, PROP_ENGINE_GLOW_CONTROL_JSON)
    if payload is None:
        _engine_glow_targets(package_root)
        payload = _string_prop(package_root, PROP_ENGINE_GLOW_CONTROL_JSON)
    if payload is None:
        return 3.0
    try:
        data = json.loads(payload)
    except (json.JSONDecodeError, TypeError, ValueError):
        return 3.0
    if isinstance(data, dict):
        default_strength = data.get("default_strength", data.get("default_percent"))
        if isinstance(default_strength, (int, float)):
            return float(default_strength)
    return 3.0


def shared_glow_control_enabled(package_root: bpy.types.Object) -> bool:
    return bool(_shared_glow_targets(package_root))


def shared_glow_strength(package_root: bpy.types.Object) -> float:
    value = package_root.get(PROP_SHARED_GLOW_STRENGTH)
    if isinstance(value, (int, float)):
        return float(value)
    data = _shared_glow_control_payload(package_root)
    if isinstance(data, dict):
        default_strength = data.get("default_strength")
        if isinstance(default_strength, (int, float)):
            return float(default_strength)
    return 0.0


_SHARED_GLOW_OVERRIDE_PROP = "starbreaker_shared_glow_override_material"
_SHARED_GLOW_OVERRIDE_SIDECAR_PROP = "starbreaker_shared_glow_override_sidecar"
_SHARED_GLOW_OVERRIDE_INDEX_PROP = "starbreaker_shared_glow_override_source_index"


def _shared_glow_override_matches(material: bpy.types.Material, key: tuple[str, int], base_name: str) -> bool:
    if not bool(material.get(_SHARED_GLOW_OVERRIDE_PROP)):
        return False
    stored_sidecar = material.get(_SHARED_GLOW_OVERRIDE_SIDECAR_PROP)
    stored_index = material.get(_SHARED_GLOW_OVERRIDE_INDEX_PROP)
    if isinstance(stored_sidecar, str) and stored_sidecar == key[0]:
        try:
            if int(stored_index) == key[1]:
                return True
        except (TypeError, ValueError):
            pass
    return material.name.startswith(f"{base_name}__shared_glow")


def _shared_glow_override_sort_key(material: bpy.types.Material, base_name: str) -> tuple[int, str]:
    exact_name = f"{base_name}__shared_glow"
    return (0 if material.name == exact_name else 1, material.name)


def _find_shared_glow_override_material(key: tuple[str, int], base_name: str) -> bpy.types.Material | None:
    materials = getattr(getattr(bpy, "data", None), "materials", ())
    candidates = [
        material
        for material in materials
        if _shared_glow_override_matches(material, key, base_name)
    ]
    if not candidates:
        return None
    candidates.sort(key=lambda material: _shared_glow_override_sort_key(material, base_name))
    material = candidates[0]
    material[_SHARED_GLOW_OVERRIDE_SIDECAR_PROP] = key[0]
    material[_SHARED_GLOW_OVERRIDE_INDEX_PROP] = int(key[1])
    return material


def _material_for_shared_glow_slot(
    material: bpy.types.Material,
    sidecar_key: str,
    source_index: int,
    override_cache: dict[tuple[str, int], bpy.types.Material],
) -> bpy.types.Material:
    key = (sidecar_key, source_index)
    existing = override_cache.get(key)
    if existing is not None:
        return existing
    base_name = material.name.split("__shared_glow", 1)[0]
    existing = _find_shared_glow_override_material(key, base_name)
    if existing is not None:
        override_cache[key] = existing
        return existing
    if bool(material.get(_SHARED_GLOW_OVERRIDE_PROP)):
        material[_SHARED_GLOW_OVERRIDE_SIDECAR_PROP] = sidecar_key
        material[_SHARED_GLOW_OVERRIDE_INDEX_PROP] = int(source_index)
        override_cache[key] = material
        return material
    copy = material.copy()
    copy.name = f"{material.name}__shared_glow"
    copy[_SHARED_GLOW_OVERRIDE_PROP] = True
    copy[_SHARED_GLOW_OVERRIDE_SIDECAR_PROP] = sidecar_key
    copy[_SHARED_GLOW_OVERRIDE_INDEX_PROP] = int(source_index)
    override_cache[key] = copy
    return copy


def _purge_duplicate_shared_glow_overrides(override_cache: dict[tuple[str, int], bpy.types.Material]) -> None:
    materials = getattr(getattr(bpy, "data", None), "materials", None)
    if materials is None:
        return
    for key, canonical in override_cache.items():
        base_name = canonical.name.split("__shared_glow", 1)[0]
        for material in list(materials):
            if material is canonical:
                continue
            if not _shared_glow_override_matches(material, key, base_name):
                continue
            if getattr(material, "users", 0) != 0:
                continue
            remove = getattr(materials, "remove", None)
            if callable(remove):
                remove(material)


def _source_submaterial_for_slot(obj: bpy.types.Object, slot_index: int, material: Any, sidecar: Any) -> Any | None:
    slot_mapping = _slot_mapping_for_object(obj)
    if slot_mapping is not None and slot_index < len(slot_mapping):
        mapped_index = slot_mapping[slot_index]
        if mapped_index is not None:
            for submaterial in getattr(sidecar, "submaterials", ()):
                if int(getattr(submaterial, "index", -1)) == mapped_index:
                    return submaterial

    material_name = getattr(material, "name", "")
    if not isinstance(material_name, str) or not material_name:
        return None
    canonical_name = _canonical_source_name(material_name)
    if not canonical_name:
        return None
    unique = _unique_submaterials_by_name(sidecar).get(canonical_name)
    if unique is not None:
        return unique
    candidates = _submaterials_by_name(sidecar).get(canonical_name, [])
    if not candidates:
        return None
    return min(candidates, key=lambda item: abs(int(getattr(item, "index", slot_index)) - slot_index))


def apply_shared_glow_to_package_root(package_root: bpy.types.Object, strength: float) -> int:
    targets_by_sidecar = _shared_glow_targets(package_root)
    if not targets_by_sidecar:
        return 0
    try:
        package = _load_package_from_root(package_root)
    except (FileNotFoundError, json.JSONDecodeError, OSError, TypeError, ValueError):
        return 0

    strength = float(strength)
    updated = 0
    seen_materials: set[int] = set()
    override_cache: dict[tuple[str, int], bpy.types.Material] = {}
    sidecar_cache: dict[str, Any] = {}
    for obj in _iter_package_objects(package_root):
        slots = list(getattr(obj, "material_slots", []))
        if not slots:
            continue
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        sidecar_key = _normalize_material_sidecar_path(sidecar_path)
        if sidecar_path is None or sidecar_key is None:
            continue
        target_indices = targets_by_sidecar.get(sidecar_key)
        if not target_indices:
            continue
        sidecar = _material_refresh_sidecar(package, sidecar_cache, sidecar_path)
        if sidecar is None:
            continue
        for slot_index, slot in enumerate(slots):
            material = getattr(slot, "material", None)
            if material is None:
                continue
            submaterial = _source_submaterial_for_slot(obj, slot_index, material, sidecar)
            if submaterial is None:
                continue
            source_index = int(getattr(submaterial, "index", -1))
            if source_index not in target_indices:
                continue
            material = _material_for_shared_glow_slot(material, sidecar_key, source_index, override_cache)
            slot.material = material
            material_id = id(material)
            if material_id in seen_materials:
                continue
            seen_materials.add(material_id)
            node_tree = getattr(material, "node_tree", None)
            if node_tree is None:
                continue
            base_strength = _submaterial_authored_glow(submaterial)
            material_changed = False
            for node in getattr(node_tree, "nodes", []):
                if getattr(node, "bl_idname", "") != "ShaderNodeGroup":
                    continue
                strength_input = getattr(node, "inputs", {}).get("Emission Strength")
                if strength_input is None:
                    continue
                strength_input.default_value = base_strength + strength
                material_changed = True
            if material_changed:
                updated += 1
    package_root[PROP_SHARED_GLOW_STRENGTH] = float(strength)
    _purge_duplicate_shared_glow_overrides(override_cache)
    return updated


def _material_for_engine_glow_slot(
    material: bpy.types.Material,
    sidecar_key: str,
    source_index: int,
    override_cache: dict[tuple[str, int], bpy.types.Material],
) -> bpy.types.Material:
    key = (sidecar_key, source_index)
    existing = override_cache.get(key)
    if existing is not None:
        return existing
    base_name = material.name.split("__engine_glow", 1)[0]
    existing = _find_engine_glow_override_material(key, base_name)
    if existing is not None:
        override_cache[key] = existing
        return existing
    if bool(material.get(_ENGINE_GLOW_OVERRIDE_PROP)):
        material[_ENGINE_GLOW_OVERRIDE_SIDECAR_PROP] = sidecar_key
        material[_ENGINE_GLOW_OVERRIDE_INDEX_PROP] = int(source_index)
        override_cache[key] = material
        return material
    copy = material.copy()
    copy.name = f"{material.name}__engine_glow"
    copy[_ENGINE_GLOW_OVERRIDE_PROP] = True
    copy[_ENGINE_GLOW_OVERRIDE_SIDECAR_PROP] = sidecar_key
    copy[_ENGINE_GLOW_OVERRIDE_INDEX_PROP] = int(source_index)
    override_cache[key] = copy
    return copy


def _find_engine_glow_override_material(key: tuple[str, int], base_name: str) -> bpy.types.Material | None:
    materials = getattr(getattr(bpy, "data", None), "materials", ())
    candidates = [
        material
        for material in materials
        if _engine_glow_override_matches(material, key, base_name)
    ]
    if not candidates:
        return None
    candidates.sort(key=lambda material: _engine_glow_override_sort_key(material, base_name))
    material = candidates[0]
    material[_ENGINE_GLOW_OVERRIDE_SIDECAR_PROP] = key[0]
    material[_ENGINE_GLOW_OVERRIDE_INDEX_PROP] = int(key[1])
    return material


def _engine_glow_override_matches(material: bpy.types.Material, key: tuple[str, int], base_name: str) -> bool:
    if not bool(material.get(_ENGINE_GLOW_OVERRIDE_PROP)):
        return False
    stored_sidecar = material.get(_ENGINE_GLOW_OVERRIDE_SIDECAR_PROP)
    stored_index = material.get(_ENGINE_GLOW_OVERRIDE_INDEX_PROP)
    if isinstance(stored_sidecar, str) and stored_sidecar == key[0]:
        try:
            if int(stored_index) == key[1]:
                return True
        except (TypeError, ValueError):
            pass
    return material.name.startswith(f"{base_name}__engine_glow")


def _engine_glow_override_sort_key(material: bpy.types.Material, base_name: str) -> tuple[int, str]:
    exact_name = f"{base_name}__engine_glow"
    return (0 if material.name == exact_name else 1, material.name)


def _purge_duplicate_engine_glow_overrides(override_cache: dict[tuple[str, int], bpy.types.Material]) -> None:
    materials = getattr(getattr(bpy, "data", None), "materials", None)
    if materials is None:
        return
    for key, canonical in override_cache.items():
        base_name = canonical.name.split("__engine_glow", 1)[0]
        for material in list(materials):
            if material is canonical:
                continue
            if not _engine_glow_override_matches(material, key, base_name):
                continue
            if getattr(material, "users", 0) != 0:
                continue
            remove = getattr(materials, "remove", None)
            if callable(remove):
                remove(material)


def apply_engine_glow_to_package_root(package_root: bpy.types.Object, strength: float) -> int:
    targets_by_instance_and_sidecar = _engine_glow_targets(package_root)
    if not targets_by_instance_and_sidecar:
        return 0
    strength = float(strength)
    updated = 0
    seen_materials: set[int] = set()
    override_cache: dict[tuple[str, int], bpy.types.Material] = {}
    target_sidecars = {key[1] for key in targets_by_instance_and_sidecar}
    for obj in _iter_package_objects(package_root):
        slots = list(getattr(obj, "material_slots", []))
        if not slots:
            continue
        object_sidecars: set[str] = set()
        for slot in slots:
            material = getattr(slot, "material", None)
            if material is None:
                continue
            sidecar = _string_prop(material, PROP_MATERIAL_SIDECAR)
            if sidecar is None:
                continue
            object_sidecars.add(sidecar.replace("\\", "/").casefold())
        if object_sidecars.isdisjoint(target_sidecars):
            continue

        instance = _scene_instance_from_object(obj)
        instance_asset = _normalize_engine_glow_path(instance.mesh_asset if instance is not None else None)
        if instance_asset is None:
            continue
        for slot in slots:
            material = getattr(slot, "material", None)
            if material is None:
                continue
            sidecar = _string_prop(material, PROP_MATERIAL_SIDECAR)
            if sidecar is None:
                continue
            sidecar_key = sidecar.replace("\\", "/").casefold()
            target_indices = targets_by_instance_and_sidecar.get((instance_asset, sidecar_key))
            if not target_indices:
                continue
            source_index = _material_source_index(material)
            if source_index is None or source_index not in target_indices:
                continue
            material = _material_for_engine_glow_slot(material, sidecar_key, source_index, override_cache)
            slot.material = material
            material_id = id(material)
            if material_id in seen_materials:
                continue
            seen_materials.add(material_id)
            node_tree = getattr(material, "node_tree", None)
            if node_tree is None:
                continue
            authored_emissive_color = _material_authored_emissive_color(material)
            material_changed = False
            for node in getattr(node_tree, "nodes", []):
                if getattr(node, "bl_idname", "") != "ShaderNodeGroup":
                    continue
                strength_input = getattr(node, "inputs", {}).get("Emission Strength")
                if strength_input is None:
                    continue
                strength_input.default_value = strength
                if authored_emissive_color is not None:
                    for color_socket_name in ("Emission Color", "Emission"):
                        color_input = getattr(node, "inputs", {}).get(color_socket_name)
                        if color_input is not None:
                            color_input.default_value = authored_emissive_color
                            break
                material_changed = True
            if material_changed:
                updated += 1
    package_root[PROP_ENGINE_GLOW_STRENGTH] = float(strength)
    _purge_duplicate_engine_glow_overrides(override_cache)
    return updated


_LIGHT_STATE_PRIORITY = (
    "defaultState",
    "auxiliaryState",
    "emergencyState",
    "cinematicState",
    "offState",
)


def _iter_starbreaker_lights() -> list[bpy.types.Light]:
    """Yield every ``bpy.types.Light`` datablock that carries a Phase 28
    ``PROP_LIGHT_STATES_JSON`` custom property (i.e. was imported with the
    multi-state manifest from the StarBreaker exporter)."""
    result: list[bpy.types.Light] = []
    for light in bpy.data.lights:
        if _string_prop(light, PROP_LIGHT_STATES_JSON):
            result.append(light)
    return result


def _kelvin_to_linear_rgb(kelvin: float) -> tuple[float, float, float]:
    """Convert a colour temperature in Kelvin to a linear sRGB triple.

    Mirrors the Tanner Helland approximation used by the Rust exporter
    (``starbreaker_3d::socpak::kelvin_to_rgb``) so per-state colours in the
    addon match the exporter's top-level ``LightInfo.color`` and the
    in-game blackbody appearance when ``useTemperature`` is set. Values
    outside 1000-40000 K are clamped.
    """
    import math as _math

    kelvin = max(1000.0, min(40000.0, float(kelvin)))
    temp = kelvin / 100.0
    if temp <= 66.0:
        r = 1.0
    else:
        x = temp - 60.0
        r = max(0.0, min(1.0, 329.698727446 * (x ** -0.1332047592) / 255.0))
    if temp <= 66.0:
        g = max(0.0, min(255.0, 99.4708025861 * _math.log(temp) - 161.1195681661)) / 255.0
    else:
        x = temp - 60.0
        g = max(0.0, min(1.0, 288.1221695283 * (x ** -0.0755148492) / 255.0))
    if temp >= 66.0:
        b = 1.0
    elif temp <= 19.0:
        b = 0.0
    else:
        x = temp - 10.0
        b = max(0.0, min(255.0, 138.5177312231 * _math.log(x) - 305.0447927307)) / 255.0
    return (r, g, b)


def available_light_state_names() -> list[str]:
    """Return the union of all state names authored across every
    StarBreaker light in the current .blend, ordered with the canonical
    CryEngine priority first."""
    import json as _json

    seen: set[str] = set()
    for light in _iter_starbreaker_lights():
        raw = _string_prop(light, PROP_LIGHT_STATES_JSON) or "{}"
        try:
            payload = _json.loads(raw)
        except Exception:
            continue
        if isinstance(payload, dict):
            seen.update(payload.keys())
    ordered: list[str] = [name for name in _LIGHT_STATE_PRIORITY if name in seen]
    ordered.extend(sorted(name for name in seen if name not in _LIGHT_STATE_PRIORITY))
    return ordered


def _is_strobe_state_payload(state: dict[str, Any]) -> bool:
    light_style = int(state.get("light_style") or 0)
    preset_tag = str(state.get("preset_tag") or "").strip().lower()
    return light_style in {4, 28} or preset_tag == "fast"


def _apply_state_to_light(
    light: bpy.types.Light,
    state_name: str,
    *,
    include_strobe: bool = True,
) -> bool:
    """Apply the ``state_name`` snapshot to ``light`` in-place. Returns True
    if the light had the named state and was updated, False otherwise."""
    import json as _json
    from .importer.utils import _light_energy_to_blender

    raw = _string_prop(light, PROP_LIGHT_STATES_JSON)
    if not raw:
        return False
    try:
        payload = _json.loads(raw)
    except Exception:
        return False
    if not isinstance(payload, dict):
        return False
    state = payload.get(state_name)
    if not isinstance(state, dict):
        return False

    if state_name == "emergencyState" and not include_strobe and _is_strobe_state_payload(state):
        light.energy = 0.0
        light[PROP_LIGHT_ACTIVE_STATE] = state_name
        return True

    intensity_candela_proxy = state.get("intensity_candela_proxy")
    if intensity_candela_proxy is None:
        intensity_candela_proxy = state.get("intensity_cd")
    intensity_raw = state.get("intensity_raw")
    temperature = float(state.get("temperature") or 6500.0)
    use_temperature = bool(state.get("use_temperature"))
    color = state.get("color") or [1.0, 1.0, 1.0]
    if not (isinstance(color, (list, tuple)) and len(color) >= 3):
        color = [1.0, 1.0, 1.0]

    light.energy = _light_energy_to_blender(
        float(intensity_candela_proxy) if intensity_candela_proxy is not None else 0.0,
        light.type,
        intensity_raw=float(intensity_raw) if intensity_raw is not None else None,
        semantic_light_kind=_string_prop(light, PROP_LIGHT_SEMANTIC_KIND),
    )

    if use_temperature:
        # CryEngine's ``useTemperature`` flag tells the engine to discard the
        # authored RGB and render the blackbody colour at ``temperature``
        # (same as the exporter's kelvin_to_rgb). Compute the blackbody RGB
        # here so state switching matches the in-game appearance — without
        # this, Blender was keeping the authored fallback colour (often
        # warm-orange or saturated blue) while the engine renders the
        # temperature-derived colour.
        color = _kelvin_to_linear_rgb(temperature)
    light.color = (float(color[0]), float(color[1]), float(color[2]))
    light[PROP_LIGHT_ACTIVE_STATE] = state_name
    # Preserve temperature as a custom prop for round-tripping.
    light["starbreaker_light_temperature"] = temperature
    return True


def apply_light_state(state_name: str, *, include_strobe: bool = True) -> int:
    """Switch every StarBreaker light in the current .blend to the named
    state. Lights that lack the requested state keep their current values.
    Returns the number of lights that were updated."""
    updated = 0
    for light in _iter_starbreaker_lights():
        if _apply_state_to_light(light, state_name, include_strobe=include_strobe):
            updated += 1
    return updated


_ANIMATION_MODES_PROP = "starbreaker_animation_modes"
_ANIMATION_BIND_TRS_PROP = "starbreaker_animation_bind_trs"
_ANIMATION_INSTANCES_PROP = "starbreaker_animation_instances_v1"
_ANIMATION_INSTANCE_ID_PROP = "starbreaker_animation_instance_id"
_ANIMATION_INSTANCE_NAME_PROP = "starbreaker_animation_name"
_FRAGMENT_ANIMATION_PREFIX = "fragment:"


def _ensure_str_list(values: Any) -> list[str]:
    if not isinstance(values, list):
        return []
    return [str(value) for value in values if isinstance(value, str)]


def _animation_instance_from_value(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    instance_id = value.get("id")
    animation_name = value.get("animation_name")
    if not isinstance(instance_id, str) or not instance_id:
        return None
    if not isinstance(animation_name, str) or not animation_name:
        return None
    try:
        start_frame = float(value.get("start_frame", 1.0))
    except (TypeError, ValueError):
        start_frame = 1.0
    try:
        duration_frames = max(0.0, float(value.get("duration_frames", 0.0)))
    except (TypeError, ValueError):
        duration_frames = 0.0
    driven_hashes = sorted(set(_ensure_str_list(value.get("driven_hashes"))))
    return {
        "id": instance_id,
        "animation_name": animation_name,
        "start_frame": start_frame,
        "duration_frames": duration_frames,
        "driven_hashes": driven_hashes,
    }


def _load_animation_instances(package_root: bpy.types.Object) -> list[dict[str, Any]]:
    payload = package_root.get(_ANIMATION_INSTANCES_PROP)
    if not isinstance(payload, str) or not payload:
        return []
    try:
        loaded = json.loads(payload)
    except json.JSONDecodeError:
        return []
    if not isinstance(loaded, list):
        return []
    result: list[dict[str, Any]] = []
    for value in loaded:
        parsed = _animation_instance_from_value(value)
        if parsed is not None:
            result.append(parsed)
    return result


def _store_animation_instances(package_root: bpy.types.Object, instances: list[dict[str, Any]]) -> None:
    package_root[_ANIMATION_INSTANCES_PROP] = json.dumps(instances, separators=(",", ":"), sort_keys=True)


def _instance_end_frame(instance: dict[str, Any]) -> float:
    return float(instance.get("start_frame", 1.0)) + float(instance.get("duration_frames", 0.0))


def _intervals_overlap(start_a: float, end_a: float, start_b: float, end_b: float) -> bool:
    return max(start_a, start_b) < min(end_a, end_b)


def _animation_overlap_warnings(
    animation_name: str,
    start_frame: float,
    duration_frames: float,
    driven_hashes: set[str],
    existing_instances: list[dict[str, Any]],
) -> list[str]:
    end_frame = start_frame + duration_frames
    warnings: list[str] = []
    if duration_frames <= 0.0 or not driven_hashes:
        return warnings
    for existing in existing_instances:
        existing_name = str(existing.get("animation_name", ""))
        existing_hashes = set(_ensure_str_list(existing.get("driven_hashes")))
        if not (driven_hashes & existing_hashes):
            continue
        existing_start = float(existing.get("start_frame", 1.0))
        existing_end = _instance_end_frame(existing)
        if not _intervals_overlap(start_frame, end_frame, existing_start, existing_end):
            continue
        warnings.append(
            (
                f"{animation_name} overlaps {existing_name} "
                f"({int(round(existing_start))}-{int(round(existing_end))}) "
                f"on {len(driven_hashes & existing_hashes)} shared channels"
            )
        )
    return warnings


def _clip_duration_frames(clip: dict[str, Any], frame_scale: float) -> float:
    trim_frame = _clip_cyclic_transition_target_frame(clip)
    bones = _normalized_bone_channels(clip)
    channel_times: list[float] = []
    for channel_variants in bones.values():
        for channel in channel_variants:
            if not isinstance(channel, dict):
                continue
            rotations = channel.get("rotation") if isinstance(channel.get("rotation"), list) else []
            positions = channel.get("position") if isinstance(channel.get("position"), list) else []
            rotation_times = _channel_times(channel, "rotation_time", len(rotations))
            position_times = _channel_times(channel, "position_time", len(positions))
            for sample_time in [*rotation_times, *position_times]:
                if trim_frame is not None and sample_time > trim_frame:
                    continue
                channel_times.append(float(sample_time))
    if not channel_times:
        return 0.0
    return max(channel_times) * frame_scale

def available_package_animation_names(package: PackageBundle) -> list[str]:
    """Return animation names exported on the package root entity."""
    return [name for name, _ in available_package_animation_items(package)]


def available_package_animation_items(package: PackageBundle) -> list[tuple[str, str]]:
    """Return ``(clip_name, display_name)`` pairs for exported animations.

    ``clip_name`` is the canonical sidecar key used for lookups. ``display_name``
    prefers localized metadata when present, then falls back to the clip name
    with the entity prefix stripped and humanized (e.g. ``ship_vtol_deploy``
    → ``"Vtol Deploy"``).
    """
    clips = _animation_clips(package)
    preferred_exact_names = {
        str(clip.get("name", "")).strip()
        for clip in clips
        if _is_preferred_package_animation_name(str(clip.get("name", "")).strip())
    }
    entity_prefix = _entity_name_prefix(package)

    fragment_items: dict[tuple[str, str], tuple[int, str, str]] = {}
    items: list[tuple[str, str]] = []
    for clip in clips:
        clip_name = str(clip.get("name", "")).strip()
        if not clip_name:
            continue
        variants = _fragment_animation_variants(clip)
        if variants:
            for key, display_name, specificity, dedupe_key in variants:
                previous = fragment_items.get(dedupe_key)
                if previous is None or specificity > previous[0]:
                    fragment_items[dedupe_key] = (specificity, key, display_name)
            continue
        if preferred_exact_names and clip_name not in preferred_exact_names:
            continue
        items.append((clip_name, _animation_display_name(clip, entity_prefix=entity_prefix)))
    items.extend((key, display_name) for _, key, display_name in fragment_items.values())
    return items


def _fragment_animation_key(clip_name: str, fragment_index: int) -> str:
    return f"{_FRAGMENT_ANIMATION_PREFIX}{fragment_index}:{clip_name}"


def _parse_fragment_animation_key(animation_name: str) -> tuple[int, str] | None:
    if not animation_name.startswith(_FRAGMENT_ANIMATION_PREFIX):
        return None
    payload = animation_name[len(_FRAGMENT_ANIMATION_PREFIX) :]
    raw_index, separator, clip_name = payload.partition(":")
    if not separator or not clip_name:
        return None
    try:
        return int(raw_index), clip_name
    except ValueError:
        return None


def _fragment_animation_variants(clip: dict[str, Any]) -> list[tuple[str, str, int, tuple[str, str]]]:
    clip_name = str(clip.get("name", "")).strip()
    fragments = clip.get("fragments")
    if not clip_name or not isinstance(fragments, list):
        return []
    variants: list[tuple[str, str, int, tuple[str, str]]] = []
    for index, fragment in enumerate(fragments):
        if not isinstance(fragment, dict):
            continue
        frag_tags = _fragment_tags(fragment, "frag_tags")
        if not frag_tags:
            continue
        fragment_name = str(fragment.get("fragment", "")).strip()
        tags = _fragment_tags(fragment, "tags")
        scopes = fragment.get("scopes") if isinstance(fragment.get("scopes"), list) else []
        display_parts = [fragment_name]
        display_parts.extend(tag for tag in tags if tag.lower() != fragment_name.lower())
        display_parts.extend(frag_tags)
        display_name = " ".join(_humanize_fragment_part(part) for part in display_parts if part)
        if not display_name:
            display_name = _animation_display_name(clip)
        specificity = len(tags) + len(scopes)
        dedupe_key = (fragment_name.lower(), "+".join(tag.lower() for tag in frag_tags))
        variants.append((_fragment_animation_key(clip_name, index), display_name, specificity, dedupe_key))
    return variants


def _animation_insert_label(animation_name: str | None, clip: dict[str, Any]) -> str:
    """Return the visible label used for inserted Action/NLA strip names.

    Keeps internal IDs canonical (raw clip/fragment key) but surfaces the
    same human-readable fragment labels shown in the animation UI.
    """

    key = (animation_name or "").strip()
    if key:
        for variant_key, display_name, _, _ in _fragment_animation_variants(clip):
            if variant_key == key and display_name:
                return display_name
        return key
    return str(clip.get("name", "animation")).strip() or "animation"


def _fragment_tags(fragment: dict[str, Any], key: str) -> list[str]:
    value = fragment.get(key)
    if isinstance(value, list):
        return [str(item).strip() for item in value if str(item).strip()]
    if isinstance(value, str) and value.strip():
        return [value.strip()]
    return []


def _humanize_fragment_part(value: str) -> str:
    return value.replace("_", " ").replace("-", " ").title()


def _fragment_reverse_playback(fragment: dict[str, Any] | None) -> bool:
    if not isinstance(fragment, dict):
        return False
    animations = fragment.get("animations")
    if not isinstance(animations, list):
        return False
    saw_animation = False
    for animation in animations:
        if not isinstance(animation, dict):
            continue
        saw_animation = True
        speed = animation.get("speed", 1.0)
        if not isinstance(speed, (int, float)) or float(speed) >= 0.0:
            return False
    return saw_animation


def _fragment_semantic_reverse_playback(fragment: dict[str, Any] | None) -> bool:
    """Infer reverse playback from fragment-tag/clip-name semantic mismatch.

    Some Mannequin fragments encode transition intent via tags (Deploy,
    Retract, Open, Close, etc.) but omit an explicit ``speed`` key. When the
    referenced animation name carries the opposite semantic token (for example
    ``frag_tags=["Deploy"]`` with ``..._retract``), engine-authored content
    expects reverse playback even though speed metadata is absent.

    This fallback is metadata-driven and only applies when no explicit speed is
    authored on fragment animations.
    """

    if not isinstance(fragment, dict):
        return False

    animations = fragment.get("animations")
    if not isinstance(animations, list) or not animations:
        return False

    # Explicit speed metadata is authoritative.
    for animation in animations:
        if isinstance(animation, dict) and "speed" in animation:
            return False

    tags = {tag.lower() for key in ("frag_tags", "tags") for tag in _fragment_tags(fragment, key)}
    if not tags:
        return False

    semantic_pairs = (
        ("deploy", "retract"),
        ("open", "close"),
        ("extend", "retract"),
        ("unstow", "stow"),
    )

    forward_signal = False
    reverse_signal = False
    for animation in animations:
        if not isinstance(animation, dict):
            continue
        raw_name = str(animation.get("name", "")).strip().lower()
        if not raw_name:
            continue
        name = raw_name.rsplit("/", 1)[-1]
        if "." in name:
            name = name.split(".", 1)[0]
        tokens = {token for token in re.split(r"[^a-z0-9]+", name) if token}
        if not tokens:
            continue

        for positive, negative in semantic_pairs:
            if positive in tags:
                if positive in tokens:
                    forward_signal = True
                if negative in tokens:
                    reverse_signal = True
            if negative in tags:
                if negative in tokens:
                    forward_signal = True
                if positive in tokens:
                    reverse_signal = True

    # Only flip when all semantic evidence points to an inverse pairing.
    return reverse_signal and not forward_signal


def _effective_fragment_reverse_playback(fragment: dict[str, Any] | None) -> bool:
    return _fragment_reverse_playback(fragment) or _fragment_semantic_reverse_playback(fragment)


def _fragment_endpoint_policy(fragment: dict[str, Any] | None, mode: str) -> str | None:
    """Map a Mannequin fragment + snap mode to a transition state policy.

    Each Mannequin transition fragment references a single CryEngine clip and
    plays it either forward (``speed >= 0``) or in reverse (``speed < 0``).
    The clip itself encodes a transition from one steady state ("start") to
    another ("end"). We resolve start/end purely in clip-time:

    * ``start`` = first clip sample (clip-time = 0).
        * ``end`` = the per-channel "other endpoint": the last sample for
            non-cyclic channels, or the mid-clip extreme for cyclic channels
            (those whose first and last samples coincide, e.g. a front
            landing-gear channel bound in the stowed pose that arcs back to it).

    For a forward fragment, ``snap_first`` -> ``start`` and ``snap_last`` ->
    ``end``. For a reverse-playback fragment (``speed = -1``), playback
    starts at clip-end and finishes at clip-start, so the mapping flips:
    ``snap_first`` -> ``end`` and ``snap_last`` -> ``start``.

    Returns ``None`` for fragments that do not encode a transition (so the
    caller falls back to the legacy bind-distance heuristic).
    """

    if not isinstance(fragment, dict):
        return None
    tags = {tag.lower() for key in ("frag_tags", "tags") for tag in _fragment_tags(fragment, key)}
    if not tags:
        return None
    normalized_mode = mode.strip().lower()
    if normalized_mode not in {"snap_first", "snap_last"}:
        return None

    transition_tags = {
        "open", "close", "extend", "unstow", "stow",
        "deploy", "retract",
    }
    if not (tags & transition_tags):
        return None

    reverse = _effective_fragment_reverse_playback(fragment)
    if normalized_mode == "snap_first":
        return "transition_end" if reverse else "transition_start"
    return "transition_start" if reverse else "transition_end"


def _is_preferred_package_animation_name(name: str) -> bool:
    normalized = name.strip()
    if not normalized:
        return False
    if "/" in normalized:
        return False
    if normalized.startswith("$"):
        return False
    return True


def package_animation_mode_map(package_root: bpy.types.Object) -> dict[str, str]:
    payload = package_root.get(_ANIMATION_MODES_PROP)
    if not isinstance(payload, str) or not payload:
        return {}
    try:
        loaded = json.loads(payload)
    except json.JSONDecodeError:
        return {}
    if not isinstance(loaded, dict):
        return {}
    result: dict[str, str] = {}
    for key, value in loaded.items():
        if isinstance(key, str) and isinstance(value, str):
            result[key] = value
    return result


def _iter_instance_actions(package_root: bpy.types.Object, instance_id: str) -> list[Any]:
    actions: list[Any] = []
    # Check live action references on each object
    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        action = getattr(animation_data, "action", None) if animation_data is not None else None
        if action is None:
            continue
        try:
            action_instance_id = action.get(_ANIMATION_INSTANCE_ID_PROP)
        except Exception:
            action_instance_id = None
        if action_instance_id == instance_id:
            actions.append(action)
    # Also scan bpy.data.actions (live action is cleared to None for NLA-driven playback)
    pkg_prefix = f"SB_{package_root.name}_"
    for action in bpy.data.actions:
        if not action.name.startswith(pkg_prefix):
            continue
        try:
            action_instance_id = action.get(_ANIMATION_INSTANCE_ID_PROP)
        except Exception:
            action_instance_id = None
        if action_instance_id == instance_id and action not in actions:
            actions.append(action)
    return actions


def _iter_instance_strips(
    package_root: bpy.types.Object,
    instance_id: str,
    instances: list[dict[str, Any]] | None = None,
    fallback_track_name: str | None = None,
) -> list[Any]:
    # Build fallback track name and animation name prefix for matching
    fallback_anim_prefix: str | None = None
    if instances is not None or fallback_track_name is not None:
        if fallback_track_name is None and instances is not None:
            inst_meta = next((i for i in instances if i.get("id") == instance_id), None)
            if inst_meta is not None:
                anim_name = inst_meta.get("animation_name", "")
                start = int(round(float(inst_meta.get("start_frame", 0))))
                fallback_track_name = f"{anim_name}@{start}"
        if fallback_track_name is not None:
            fallback_anim_prefix = fallback_track_name.rsplit("@", 1)[0] + "@"
    strips: list[Any] = []
    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        tracks = getattr(animation_data, "nla_tracks", None) if animation_data is not None else None
        if tracks is None:
            continue
        for track in tracks:
            for strip in track.strips:
                matched = False
                try:
                    strip_instance_id = strip.get(_ANIMATION_INSTANCE_ID_PROP)
                    if strip_instance_id == instance_id:
                        matched = True
                except Exception:
                    pass
                if not matched and fallback_track_name is not None and track.name == fallback_track_name:
                    matched = True
                # Last resort: match by animation name prefix (handles stale @N in track name)
                if not matched and fallback_anim_prefix is not None and track.name.startswith(fallback_anim_prefix):
                    matched = True
                if matched:
                    strips.append(strip)
    return strips


def _shift_action_frames(action: Any, delta: float) -> None:
    if abs(delta) < 1e-9:
        return
    for fcurve in _action_fcurves(action):
        keyframe_points = getattr(fcurve, "keyframe_points", None)
        if keyframe_points is None:
            continue
        for keyframe in keyframe_points:
            try:
                keyframe.co.x = float(keyframe.co.x) + delta
                keyframe.handle_left.x = float(keyframe.handle_left.x) + delta
                keyframe.handle_right.x = float(keyframe.handle_right.x) + delta
            except Exception:
                continue
        try:
            fcurve.update()
        except Exception:
            continue


def _resync_animation_instances_from_scene(
    package_root: bpy.types.Object,
    package: PackageBundle,
    persist: bool = True,
) -> list[dict[str, Any]]:
    instances = _load_animation_instances(package_root)
    by_id: dict[str, dict[str, Any]] = {
        instance["id"]: dict(instance)
        for instance in instances
        if isinstance(instance.get("id"), str)
    }
    seen_ids: set[str] = set()

    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        if animation_data is None:
            continue

        action = getattr(animation_data, "action", None)
        if action is not None:
            try:
                instance_id = action.get(_ANIMATION_INSTANCE_ID_PROP)
                animation_name = action.get(_ANIMATION_INSTANCE_NAME_PROP)
            except Exception:
                instance_id, animation_name = None, None
            if isinstance(instance_id, str) and instance_id and isinstance(animation_name, str) and animation_name:
                seen_ids.add(instance_id)
                clip = _find_animation_clip(package, animation_name)
                if clip is None:
                    continue
                entry = by_id.get(instance_id)
                if entry is None:
                    entry = {
                        "id": instance_id,
                        "animation_name": animation_name,
                        "start_frame": 1.0,
                        "duration_frames": 0.0,
                        "driven_hashes": sorted(_clip_bone_hashes(clip)),
                    }
                    by_id[instance_id] = entry
                try:
                    frame_low, frame_high = action.frame_range
                    entry["start_frame"] = float(frame_low)
                    entry["duration_frames"] = max(0.0, float(frame_high) - float(frame_low))
                except Exception:
                    pass

        tracks = getattr(animation_data, "nla_tracks", None)
        if tracks is None:
            continue
        for track in tracks:
            for strip in track.strips:
                try:
                    instance_id = strip.get(_ANIMATION_INSTANCE_ID_PROP)
                    animation_name = strip.get(_ANIMATION_INSTANCE_NAME_PROP)
                except Exception:
                    instance_id, animation_name = None, None
                if not (isinstance(instance_id, str) and instance_id and isinstance(animation_name, str) and animation_name):
                    continue
                seen_ids.add(instance_id)
                clip = _find_animation_clip(package, animation_name)
                if clip is None:
                    continue
                entry = by_id.get(instance_id)
                if entry is None:
                    entry = {
                        "id": instance_id,
                        "animation_name": animation_name,
                        "start_frame": float(getattr(strip, "frame_start", 1.0)),
                        "duration_frames": max(0.0, float(getattr(strip, "frame_end", 1.0)) - float(getattr(strip, "frame_start", 1.0))),
                        "driven_hashes": sorted(_clip_bone_hashes(clip)),
                    }
                    by_id[instance_id] = entry
                else:
                    strip_start = float(getattr(strip, "frame_start", entry.get("start_frame", 1.0)))
                    entry["start_frame"] = min(float(entry.get("start_frame", strip_start)), strip_start)
                    strip_duration = max(
                        0.0,
                        float(getattr(strip, "frame_end", strip_start)) - strip_start,
                    )
                    entry["duration_frames"] = max(float(entry.get("duration_frames", 0.0)), strip_duration)

    resynced = [instance for instance_id, instance in by_id.items() if instance_id in seen_ids]

    # When NLA-driven playback is active (anim.action == None) and NlaStrip
    # IDProperties are not supported, seen_ids may be empty even though valid
    # instances exist.  In that case, scan bpy.data.actions for actions tagged
    # with instance IDs that belong to this package root, so the stored
    # instances are confirmed to still have live data in the scene.
    if not seen_ids:
        pkg_prefix = f"SB_{package_root.name}_"
        for action in bpy.data.actions:
            if not action.name.startswith(pkg_prefix):
                continue
            try:
                instance_id = action.get(_ANIMATION_INSTANCE_ID_PROP)
            except Exception:
                continue
            if isinstance(instance_id, str) and instance_id and instance_id in by_id:
                seen_ids.add(instance_id)
        resynced = [instance for instance_id, instance in by_id.items() if instance_id in seen_ids]
    resynced.sort(key=lambda item: (str(item.get("animation_name", "")), float(item.get("start_frame", 1.0))))
    if persist:
        _store_animation_instances(package_root, resynced)
    return resynced


def package_animation_instances(
    package_root: bpy.types.Object,
    package: PackageBundle | None = None,
    persist: bool = True,
) -> list[dict[str, Any]]:
    if package is None:
        package = _load_package_from_root(package_root)
    return _resync_animation_instances_from_scene(package_root, package, persist=persist)


def animation_overlap_warnings(
    package_root: bpy.types.Object,
    animation_name: str,
    start_frame: float,
    context_scene: Any | None = None,
    fps_policy: str = FPS_POLICY_ADAPT_SCENE,
    package: PackageBundle | None = None,
) -> list[str]:
    if package is None:
        package = _load_package_from_root(package_root)
    clip = _find_animation_clip(package, animation_name)
    if clip is None:
        return []
    scene = context_scene if context_scene is not None else getattr(bpy.context, "scene", None)
    if scene is None:
        frame_scale = 1.0
    else:
        frame_scale = reconcile_animation_fps(scene, clip.get("fps"), fps_policy).frame_scale
    duration_frames = _clip_duration_frames(clip, frame_scale)
    instances = package_animation_instances(package_root, package)
    return _animation_overlap_warnings(
        animation_name,
        float(start_frame),
        duration_frames,
        _clip_bone_hashes(clip),
        instances,
    )


def update_animation_instance_start_frame(
    package_root: bpy.types.Object,
    instance_id: str,
    start_frame: float,
    package: PackageBundle | None = None,
) -> bool:
    if package is None:
        package = _load_package_from_root(package_root)
    instances = package_animation_instances(package_root, package)
    target = next((instance for instance in instances if instance.get("id") == instance_id), None)
    if target is None:
        return False
    old_start = float(target.get("start_frame", 1.0))
    new_start = float(start_frame)
    delta = new_start - old_start
    if abs(delta) < 1e-9:
        return True
    anim_name = target.get("animation_name", "")
    old_track_name = f"{anim_name}@{int(round(old_start))}"
    new_track_name = f"{anim_name}@{int(round(new_start))}"
    for action in _iter_instance_actions(package_root, instance_id):
        _shift_action_frames(action, delta)
    for strip in _iter_instance_strips(package_root, instance_id, fallback_track_name=old_track_name):
        try:
            strip.frame_start = float(strip.frame_start) + delta
            strip.frame_end = float(strip.frame_end) + delta
        except Exception:
            continue
    # Rename NLA tracks so future lookups find them under the new start frame
    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        tracks = getattr(animation_data, "nla_tracks", None) if animation_data is not None else None
        if tracks is None:
            continue
        for track in tracks:
            if track.name == old_track_name:
                try:
                    track.name = new_track_name
                except Exception:
                    pass
    for instance in instances:
        if instance.get("id") == instance_id:
            instance["start_frame"] = new_start
            break
    _store_animation_instances(package_root, instances)
    return True


def delete_animation_instance(
    package_root: bpy.types.Object,
    instance_id: str,
    package: PackageBundle | None = None,
) -> bool:
    if package is None:
        package = _load_package_from_root(package_root)
    instances = package_animation_instances(package_root, package)
    before = len(instances)
    instances_before = list(instances)
    instances = [instance for instance in instances if instance.get("id") != instance_id]
    if len(instances) == before:
        return False

    for action in _iter_instance_actions(package_root, instance_id):
        for obj in _iter_package_objects(package_root):
            animation_data = getattr(obj, "animation_data", None)
            if animation_data is not None and getattr(animation_data, "action", None) is action:
                try:
                    animation_data.action = None
                except Exception:
                    pass
        try:
            bpy.data.actions.remove(action, do_unlink=True)
        except Exception:
            pass

    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        tracks = getattr(animation_data, "nla_tracks", None) if animation_data is not None else None
        if tracks is None:
            continue
        for track in list(tracks):
            # Match by IDProperty (may fail on NlaStrip) or by track name pattern
            track_matches = False
            for strip in list(track.strips):
                try:
                    strip_instance_id = strip.get(_ANIMATION_INSTANCE_ID_PROP)
                except Exception:
                    strip_instance_id = None
                if strip_instance_id == instance_id:
                    track_matches = True
                    break
            # Fallback: match track by name "animation_name@start_frame"
            if not track_matches:
                track_name = track.name
                track_matches = any(
                    track_name == f"{inst.get('animation_name')}@{int(round(inst.get('start_frame', 0)))}"
                    for inst in [next((i for i in instances_before if i.get('id') == instance_id), None)]
                    if inst is not None
                )
            if not track_matches:
                continue
            for strip in list(track.strips):
                try:
                    track.strips.remove(strip)
                except Exception:
                    pass
            try:
                tracks.remove(track)
            except Exception:
                pass

    _store_animation_instances(package_root, instances)
    return True


def set_animation_instance_muted(
    package_root: bpy.types.Object,
    instance_id: str,
    muted: bool | None = None,
    package: PackageBundle | None = None,
) -> bool | None:
    """Mute/unmute one animation instance by id.

    Returns:
      - ``True`` when the instance is muted after the call
      - ``False`` when the instance is unmuted after the call
      - ``None`` when the instance could not be resolved
    """
    if package is None:
        package = _load_package_from_root(package_root)
    instances = package_animation_instances(package_root, package)
    target = next((instance for instance in instances if instance.get("id") == instance_id), None)
    if target is None:
        return None

    track_name = f"{target.get('animation_name', '')}@{int(round(float(target.get('start_frame', 1.0))))}"
    current_state: bool | None = None
    tracks: list[Any] = []
    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        nla_tracks = getattr(animation_data, "nla_tracks", None) if animation_data is not None else None
        if nla_tracks is None:
            continue
        for track in nla_tracks:
            if track.name == track_name:
                tracks.append(track)
                if current_state is None:
                    current_state = bool(track.mute)

    if not tracks:
        return None

    next_state = (not bool(current_state)) if muted is None else bool(muted)
    for track in tracks:
        try:
            track.mute = next_state
        except Exception:
            continue
    return next_state


def solo_animation_instance(
    package_root: bpy.types.Object,
    instance_id: str,
    package: PackageBundle | None = None,
) -> bool:
    """Solo one animation instance by muting all other tracked instance tracks."""
    if package is None:
        package = _load_package_from_root(package_root)
    instances = package_animation_instances(package_root, package)
    target = next((instance for instance in instances if instance.get("id") == instance_id), None)
    if target is None:
        return False

    track_names = {
        f"{instance.get('animation_name', '')}@{int(round(float(instance.get('start_frame', 1.0))))}"
        for instance in instances
    }
    target_track_name = f"{target.get('animation_name', '')}@{int(round(float(target.get('start_frame', 1.0))))}"

    target_seen = False
    for obj in _iter_package_objects(package_root):
        animation_data = getattr(obj, "animation_data", None)
        nla_tracks = getattr(animation_data, "nla_tracks", None) if animation_data is not None else None
        if nla_tracks is None:
            continue
        for track in nla_tracks:
            if track.name not in track_names:
                continue
            should_mute = track.name != target_track_name
            if track.name == target_track_name:
                target_seen = True
            try:
                track.mute = should_mute
            except Exception:
                continue
    return target_seen


def package_animation_diagnostics(
    package: PackageBundle,
    package_root: bpy.types.Object,
    animation_name: str,
) -> dict[str, Any]:
    clip = _find_animation_clip(package, animation_name)
    if clip is None:
        raise RuntimeError(f"Animation '{animation_name}' not found in package sidecar")

    bones = clip.get("bones")
    channel_hashes: list[str] = []
    if isinstance(bones, dict):
        channel_hashes = [
            canonical
            for key in bones.keys()
            if isinstance(key, str)
            for canonical in [
                _canonical_bone_hash_key(key),
            ]
            if canonical is not None
        ]

    hash_to_objects: dict[str, list[str]] = {}
    for obj in _iter_candidate_bone_objects(package_root):
        bone_hash = _canonical_bone_hash_key(_object_bone_hash(obj)) or _object_bone_hash(obj)
        source_name = str(obj.get(PROP_SOURCE_NODE_NAME, obj.name) or "")
        hash_to_objects.setdefault(bone_hash, []).append(source_name)

    matched_hashes: list[str] = []
    unmatched_hashes: list[str] = []
    matched_objects: set[str] = set()
    ambiguous_hashes: list[str] = []

    for bone_hash in channel_hashes:
        names = hash_to_objects.get(bone_hash, [])
        if names:
            matched_hashes.append(bone_hash)
            matched_objects.update(names)
            if len(names) > 1:
                ambiguous_hashes.append(bone_hash)
        else:
            unmatched_hashes.append(bone_hash)

    top_matches = sorted(
        (
            {
                "hash": bone_hash,
                "objects": sorted(hash_to_objects.get(bone_hash, [])),
            }
            for bone_hash in matched_hashes
        ),
        key=lambda item: len(item["objects"]),
        reverse=True,
    )

    return {
        "animation_name": animation_name,
        "display_name": _animation_display_name(clip),
        "channel_hash_count": len(channel_hashes),
        "matched_hash_count": len(matched_hashes),
        "unmatched_hash_count": len(unmatched_hashes),
        "matched_object_count": len(matched_objects),
        "ambiguous_hash_count": len(ambiguous_hashes),
        "unmatched_hashes": sorted(unmatched_hashes),
        "matched_objects": sorted(matched_objects),
        "top_matches": top_matches[:20],
    }


def apply_animation_mode_to_package_root(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    animation_name: str,
    mode: str,
    fps_policy: str = FPS_POLICY_ADAPT_SCENE,
) -> int:
    """Apply one animation in one of: none, snap_first, snap_last, action."""
    package = _load_package_from_root(package_root)
    selection = _find_animation_selection(package, animation_name)
    if selection is None:
        raise RuntimeError(f"Animation '{animation_name}' not found in package sidecar")
    clip, fragment = selection
    reverse_playback = _effective_fragment_reverse_playback(fragment)

    normalized_mode = mode.strip().lower()
    if normalized_mode not in {"none", "snap_first", "snap_last", "action"}:
        raise RuntimeError(f"Unsupported animation mode: {mode}")

    mode_map = package_animation_mode_map(package_root)
    # If enabling a clip that overlaps channels with already enabled clips,
    # disable those conflicting modes first so poses do not stack into an
    # impossible "exploded" state.
    if normalized_mode != "none":
        target_hashes = _clip_bone_hashes(clip)
        conflicting_names: set[str] = set()
        if target_hashes:
            for other_name, other_mode in mode_map.items():
                if other_name == animation_name or other_mode == "none":
                    continue
                other_clip = _find_animation_clip(package, other_name)
                if other_clip is None:
                    continue
                if target_hashes & _clip_bone_hashes(other_clip):
                    conflicting_names.add(other_name)

        if conflicting_names:
            # Rebuild the active pose stack from bind so removed conflicts are
            # guaranteed to stop contributing transforms.
            _restore_bind_pose(package_root)
            for other_name in conflicting_names:
                mode_map[other_name] = "none"
            for other_name, other_mode in mode_map.items():
                if other_name == animation_name or other_mode == "none":
                    continue
                other_clip = _find_animation_clip(package, other_name)
                if other_clip is None:
                    continue
                other_selection = _find_animation_selection(package, other_name)
                other_fragment = other_selection[1] if other_selection is not None else None
                _apply_animation_mode_for_clip(
                    context,
                    package_root,
                    package,
                    other_clip,
                    other_mode,
                    animation_name=other_name,
                    fragment=other_fragment,
                    reverse_playback=_effective_fragment_reverse_playback(other_fragment),
                    fps_policy=fps_policy,
                )

    updated = _apply_animation_mode_for_clip(
        context,
        package_root,
        package,
        clip,
        normalized_mode,
        animation_name=animation_name,
        fragment=fragment,
        reverse_playback=reverse_playback,
        fps_policy=fps_policy,
    )

    mode_map[animation_name] = normalized_mode
    package_root[_ANIMATION_MODES_PROP] = json.dumps(mode_map, separators=(",", ":"), sort_keys=True)

    # Extend the scene frame range to cover the applied NLA strips. Socpak
    # door/elevator clips run hundreds of frames (e.g. 648, 1200) past Blender's
    # default 250-frame range, so without this the big mechanisms look frozen at
    # default playback. Best-effort: never shrinks the range (see
    # extend_frame_end) and is skipped when the scene exposes no frame_end.
    scene = getattr(context, "scene", None)
    current_end = getattr(scene, "frame_end", None)
    if isinstance(current_end, (int, float)):
        strip_ends = [
            strip.frame_end
            for obj in _iter_package_objects(package_root)
            if getattr(obj, "animation_data", None) is not None
            for track in obj.animation_data.nla_tracks
            for strip in track.strips
        ]
        scene.frame_end = extend_frame_end(current_end, strip_ends)

    return updated


def insert_animation_clip_at_frame(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    animation_name: str,
    frame: float | None = None,
    fps_policy: str = FPS_POLICY_ADAPT_SCENE,
) -> int:
    """Insert *animation_name* onto the NLA timeline at *frame*.

    If *frame* is ``None`` the current scene frame is used (identical to the
    UI "Insert" button behaviour).  Returns the number of objects updated.
    Uses the same internal path as ``apply_animation_mode_to_package_root``
    with ``mode='action'`` so all instance tracking, NLA strip creation, and
    multi-clip switching logic applies.
    """
    if frame is not None:
        context.scene.frame_set(int(round(float(frame))))
    return apply_animation_mode_to_package_root(
        context,
        package_root,
        animation_name,
        "action",
        fps_policy=fps_policy,
    )


def _apply_animation_mode_for_clip(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    package: PackageBundle,
    clip: dict[str, Any],
    mode: str,
    animation_name: str | None = None,
    fragment: dict[str, Any] | None = None,
    reverse_playback: bool = False,
    fps_policy: str = FPS_POLICY_ADAPT_SCENE,
) -> int:
    normalized_mode = mode.strip().lower()
    if normalized_mode == "none":
        return _restore_bind_pose(package_root)
    if normalized_mode in {"snap_first", "snap_last"}:
        frame_index = 0 if normalized_mode == "snap_first" else -1
        endpoint_policy = _fragment_endpoint_policy(fragment, normalized_mode) or _snap_endpoint_policy(
            str(clip.get("name", "")), normalized_mode
        )
        sample_frame_index = (-1 if frame_index == 0 else 0) if reverse_playback and endpoint_policy == "literal" else frame_index
        cyclic_target_frame = _clip_cyclic_transition_target_frame(clip)
        target_frame = cyclic_target_frame if endpoint_policy == "transition_end" else None
        updated = _apply_animation_pose(
            package_root,
            clip,
            sample_frame_index,
            endpoint_policy,
            target_frame=target_frame,
            anchor_frame=cyclic_target_frame,
        )
        if updated == 0:
            paired = _paired_clip_for_snap(package, clip, frame_index)
            if paired is not None:
                paired_clip, paired_frame_index = paired
                paired_fragment = paired_clip.get("source_fragment") if isinstance(paired_clip, dict) else None
                if not isinstance(paired_fragment, dict):
                    paired_fragment = None
                paired_policy = _fragment_endpoint_policy(paired_fragment, normalized_mode) or _snap_endpoint_policy(
                    str(paired_clip.get("name", "")), normalized_mode
                )
                paired_reverse = _effective_fragment_reverse_playback(paired_fragment)
                paired_sample_frame_index = (
                    (-1 if paired_frame_index == 0 else 0)
                    if paired_reverse and paired_policy == "literal"
                    else paired_frame_index
                )
                paired_cyclic_target_frame = _clip_cyclic_transition_target_frame(paired_clip)
                paired_target_frame = (
                    paired_cyclic_target_frame if paired_policy == "transition_end" else None
                )
                updated = _apply_animation_pose(
                    package_root,
                    paired_clip,
                    paired_sample_frame_index,
                    paired_policy,
                    target_frame=paired_target_frame,
                    anchor_frame=paired_cyclic_target_frame,
                )
        return updated
    if normalized_mode == "action":
        return _insert_animation_action(
            context,
            package_root,
            clip,
            animation_name=animation_name,
            reverse_playback=reverse_playback,
            fps_policy=fps_policy,
        )
    raise RuntimeError(f"Unsupported animation mode: {mode}")


def _clip_bone_hashes(clip: dict[str, Any]) -> set[str]:
    bones = clip.get("bones")
    if not isinstance(bones, dict):
        return set()
    return {str(key) for key in bones.keys() if isinstance(key, str)}


def _animation_clips(package: PackageBundle) -> list[dict[str, Any]]:
    raw = package.scene.root_entity.raw
    clips = raw.get("animations") if isinstance(raw, dict) else None
    if not isinstance(clips, list):
        return []
    result: list[dict[str, Any]] = []
    for clip in clips:
        if isinstance(clip, dict):
            result.append(clip)
    return result


def _strip_animation_prefix(name: str) -> str:
    normalized = name.strip()
    if normalized.lower().startswith("animations/"):
        return normalized[len("animations/") :]
    return normalized


def _entity_name_prefix(package: PackageBundle) -> str:
    """Derive a lowercase entity name prefix for stripping from clip names.

    Returns e.g. ``"my_ship"`` from
    ``entity_name="EntityClassDefinition.My_Ship"``.
    """
    try:
        entity_name = package.scene.root_entity.entity_name
    except AttributeError:
        return ""
    if not entity_name:
        return ""
    if "." in entity_name:
        entity_name = entity_name.rsplit(".", 1)[-1]
    return entity_name.lower()


def _animation_display_name(
    clip: dict[str, Any], entity_prefix: str | None = None
) -> str:
    """Return a human-readable display name for an animation clip.

    Checks localization keys first.  When the fallback raw clip name is used
    and *entity_prefix* is provided, the prefix (e.g. ``"my_ship_"``) is
    stripped and the remainder is title-cased (e.g. ``"my_ship_vtol_deploy"``
    → ``"Vtol Deploy"``).
    """
    for key in ("localized_name", "display_name", "label", "title", "ui_name"):
        value = clip.get(key)
        if isinstance(value, str):
            text = value.strip()
            if text:
                return text

    localization = clip.get("localization")
    if isinstance(localization, dict):
        for key in ("localized_name", "display_name", "label", "title", "ui_name"):
            value = localization.get(key)
            if isinstance(value, str):
                text = value.strip()
                if text:
                    return text

    raw_name = str(clip.get("name", "")).strip()
    shortened = _strip_animation_prefix(raw_name)
    filename = Path(shortened).name if shortened else ""
    base = filename or shortened or raw_name
    if entity_prefix:
        prefix_with_sep = entity_prefix.rstrip("_") + "_"
        if base.lower().startswith(prefix_with_sep):
            base = base[len(prefix_with_sep):]
        return _humanize_fragment_part(base)
    return base


def _hydrate_animation_clip(package: PackageBundle, clip: dict[str, Any]) -> dict[str, Any]:
    """Load the per-clip sidecar JSON on demand and merge `bones` into ``clip``.

    Phase 35 split full clip bodies out of ``scene.json`` into separate
    ``Packages/<entity>/animations/<clip>.json`` files. Index records in
    ``scene.json`` carry only ``name``, ``fps``, ``frame_count``,
    ``fragments`` and a ``sidecar`` reference; the heavy ``bones`` payload
    lives in the sidecar. This helper lazy-loads the sidecar the first time
    a clip is actually used and stores the result in-place on the clip
    dict so subsequent lookups are O(1).

    No-op if ``bones`` is already present (legacy/inline exports) or the
    sidecar reference is missing/unresolvable.
    """
    if not isinstance(clip, dict):
        return clip
    if isinstance(clip.get("bones"), dict):
        return clip
    sidecar_rel = clip.get("sidecar")
    if not isinstance(sidecar_rel, str) or not sidecar_rel.strip():
        return clip
    package_dir = package.scene_path.parent
    candidate = package_dir / sidecar_rel
    if not candidate.is_file():
        resolved = package.resolve_path(sidecar_rel)
        if resolved is None:
            return clip
        candidate = resolved
    try:
        with candidate.open("r", encoding="utf-8") as fh:
            payload = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return clip
    if not isinstance(payload, dict):
        return clip
    bones = payload.get("bones")
    if isinstance(bones, dict):
        clip["bones"] = bones
    # Sidecar may also carry richer fragments / time arrays; only set keys
    # that aren't already in the index record so the index stays
    # authoritative for summary metadata.
    for key, value in payload.items():
        if key in ("name", "fps", "frame_count", "fragments", "sidecar"):
            continue
        clip.setdefault(key, value)
    return clip


def _find_animation_selection(package: PackageBundle, animation_name: str) -> tuple[dict[str, Any], dict[str, Any] | None] | None:
    target = animation_name.strip()
    if not target:
        return None
    parsed_fragment = _parse_fragment_animation_key(target)
    if parsed_fragment is not None:
        fragment_index, clip_name = parsed_fragment
        for clip in _animation_clips(package):
            if str(clip.get("name", "")).strip() != clip_name:
                continue
            fragments = clip.get("fragments")
            _hydrate_animation_clip(package, clip)
            if isinstance(fragments, list) and 0 <= fragment_index < len(fragments):
                fragment = fragments[fragment_index]
                if isinstance(fragment, dict):
                    return clip, fragment
            return clip, None
    for clip in _animation_clips(package):
        if str(clip.get("name", "")).strip() == target:
            _hydrate_animation_clip(package, clip)
            return clip, None
    return None


def _find_animation_clip(package: PackageBundle, animation_name: str) -> dict[str, Any] | None:
    selection = _find_animation_selection(package, animation_name)
    return selection[0] if selection is not None else None


def _paired_clip_for_snap(
    package: PackageBundle,
    clip: dict[str, Any],
    frame_index: int,
) -> tuple[dict[str, Any], int] | None:
    name = str(clip.get("name", "")).strip()
    if not name:
        return None

    candidates: list[tuple[str, int]] = []
    def _append_pair(base: str, from_suffix: str, to_suffix: str) -> None:
        if base.endswith(from_suffix):
            alt_name = f"{base[:-len(from_suffix)]}{to_suffix}"
            candidates.append((alt_name, 0 if frame_index == -1 else -1))

    _append_pair(name, "_retract.caf", "_deploy.caf")
    _append_pair(name, "_deploy.caf", "_retract.caf")
    _append_pair(name, "_close.caf", "_open.caf")
    _append_pair(name, "_open.caf", "_close.caf")
    _append_pair(name, "_retract", "_deploy")
    _append_pair(name, "_deploy", "_retract")
    _append_pair(name, "_close", "_open")
    _append_pair(name, "_open", "_close")

    for alt_name, alt_frame in candidates:
        alt_clip = _find_animation_clip(package, alt_name)
        if alt_clip is not None:
            return alt_clip, alt_frame
    return None


def _snap_endpoint_policy(animation_name: str, mode: str) -> str:
    # The exporter (Phase 24B) now reverses clips whose internal direction
    # disagrees with the chrparams event-name semantic, so snap modes can
    # apply the literal first/last keyframe. Previously this function used
    # "most_bind_error"/"least_bind_error" heuristics to compensate for
    # reversed clips, which is now redundant and would re-flip corrected
    # clips to the wrong endpoint. Keep the signature for callers but
    # always return literal.
    del animation_name, mode
    return "literal"


def _channel_times(channel: dict[str, Any], key: str, count: int) -> list[float]:
    raw = channel.get(key)
    if isinstance(raw, list) and len(raw) == count:
        times: list[float] = []
        for value in raw:
            if not isinstance(value, (int, float)):
                break
            times.append(float(value))
        if len(times) == count:
            return times
    return [float(index) for index in range(count)]


def _sample_nearest_time(values: list[Any], times: list[float], item_len: int, target_frame: float) -> list[Any] | None:
    candidates: list[tuple[float, list[Any]]] = []
    for index, value in enumerate(values):
        if isinstance(value, list) and len(value) >= item_len and index < len(times):
            candidates.append((abs(times[index] - target_frame), value))
    if not candidates:
        return None
    return min(candidates, key=lambda item: item[0])[1]


def _rotation_distance(a: list[Any], b: list[Any]) -> float:
    dot = abs(float(a[0]) * float(b[0]) + float(a[1]) * float(b[1]) + float(a[2]) * float(b[2]) + float(a[3]) * float(b[3]))
    dot = max(0.0, min(1.0, dot))
    return 2.0 * math.acos(dot)


def _position_distance(a: list[Any], b: list[Any]) -> float:
    dx = float(a[0]) - float(b[0])
    dy = float(a[1]) - float(b[1])
    dz = float(a[2]) - float(b[2])
    return (dx * dx + dy * dy + dz * dz) ** 0.5


def _vec_distance_sq(a: tuple[float, float, float], b: tuple[float, float, float]) -> float:
    dx = float(a[0]) - float(b[0])
    dy = float(a[1]) - float(b[1])
    dz = float(a[2]) - float(b[2])
    return dx * dx + dy * dy + dz * dz


def _quat_mul(a: tuple[float, float, float, float], b: tuple[float, float, float, float]) -> tuple[float, float, float, float]:
    aw, ax, ay, az = a
    bw, bx, by, bz = b
    return (
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    )


def _quat_conj(q: tuple[float, float, float, float]) -> tuple[float, float, float, float]:
    return (q[0], -q[1], -q[2], -q[3])


def _quat_align(reference: tuple[float, float, float, float], q: tuple[float, float, float, float]) -> tuple[float, float, float, float]:
    """Return q (or -q) whichever has positive dot with reference (canonicalize hemisphere)."""
    if reference[0] * q[0] + reference[1] * q[1] + reference[2] * q[2] + reference[3] * q[3] < 0.0:
        return (-q[0], -q[1], -q[2], -q[3])
    return q


_ANIMATION_SAMPLE_DECODERS = ("identity", "source")
_BIND_POSITION_EPSILON_METERS = 0.01
_BIND_ROTATION_EPSILON_RADIANS = 0.025


def _quat_distance_sq(
    a: tuple[float, float, float, float],
    b: tuple[float, float, float, float],
) -> float:
    aligned = _quat_align(b, a)
    return (
        (aligned[0] - b[0]) ** 2
        + (aligned[1] - b[1]) ** 2
        + (aligned[2] - b[2]) ** 2
        + (aligned[3] - b[3]) ** 2
    )


def _bind_equivalent_position(
    sample: tuple[float, float, float],
    bind: tuple[float, float, float],
) -> tuple[float, float, float]:
    if _vec_distance_sq(sample, bind) <= (
        _BIND_POSITION_EPSILON_METERS * _BIND_POSITION_EPSILON_METERS
    ):
        return bind
    return sample


def _bind_equivalent_rotation(
    sample: tuple[float, float, float, float],
    bind: tuple[float, float, float, float],
) -> tuple[float, float, float, float]:
    dot = abs(
        sample[0] * bind[0]
        + sample[1] * bind[1]
        + sample[2] * bind[2]
        + sample[3] * bind[3]
    )
    dot = max(0.0, min(1.0, dot))
    if 2.0 * math.acos(dot) <= _BIND_ROTATION_EPSILON_RADIANS:
        return bind
    return sample


def _positive_speed_fragment(fragment: dict[str, Any]) -> bool:
    animations = fragment.get("animations")
    if not isinstance(animations, list):
        return True
    saw_animation = False
    for animation in animations:
        if not isinstance(animation, dict):
            continue
        saw_animation = True
        speed = animation.get("speed", 1.0)
        if not isinstance(speed, (int, float)) or float(speed) >= 0.0:
            return True
    return not saw_animation


def _clip_has_transition_fragment(clip: dict[str, Any]) -> bool:
    fragments = clip.get("fragments")
    if not isinstance(fragments, list):
        return False
    transition_tags = {"open", "close", "deploy", "retract", "extend", "stow", "unstow"}
    non_transition_tags = {"compress", "loop"}
    for fragment in fragments:
        if not isinstance(fragment, dict) or not _positive_speed_fragment(fragment):
            continue
        raw_tags: list[Any] = []
        for key in ("frag_tags", "tags"):
            value = fragment.get(key)
            if isinstance(value, list):
                raw_tags.extend(value)
            elif isinstance(value, str):
                raw_tags.append(value)
        tags = {str(tag).strip().lower() for tag in raw_tags if str(tag).strip()}
        if tags & transition_tags and not tags <= non_transition_tags:
            return True
    return False


def _series_cyclic_target_time(
    values: list[Any],
    times: list[float],
    item_len: int,
    distance: Callable[[list[Any], list[Any]], float],
    threshold: float,
) -> tuple[bool, float | None]:
    valid: list[tuple[list[Any], float]] = [
        (value, times[index])
        for index, value in enumerate(values)
        if isinstance(value, list) and len(value) >= item_len and index < len(times)
    ]
    if len(valid) < 3:
        return False, None
    first = valid[0][0]
    last = valid[-1][0]
    distances = [(distance(first, value), time) for value, time in valid]
    max_distance, target_time = max(distances, key=lambda item: item[0])
    if max_distance <= threshold:
        return False, None
    endpoint_distance = distance(first, last)
    if endpoint_distance <= max(max_distance * 0.4, threshold):
        return True, target_time
    return False, target_time


def _clip_start_rotation(clip: dict[str, Any]) -> tuple[float, float, float, float] | None:
    """Return the clip's DBA-metadata `start_rotation` as a Blender wxyz
    quaternion tuple, or None when the field is absent (CAF-only clips)
    or malformed.

    The exporter (`crates/starbreaker-3d/src/animation.rs::clip_to_json`)
    already converts the on-disk CryEngine xyzw quat into Blender wxyz
    convention (see `cry_xyzw_to_blender_wxyz`), so consumers here just
    need to validate the shape and coerce to floats.
    """

    raw = clip.get("start_rotation")
    if not isinstance(raw, (list, tuple)) or len(raw) < 4:
        return None
    try:
        return (float(raw[0]), float(raw[1]), float(raw[2]), float(raw[3]))
    except (TypeError, ValueError):
        return None


def _clip_start_position(clip: dict[str, Any]) -> tuple[float, float, float] | None:
    """Return the clip's DBA-metadata `start_position` as a Blender Z-up
    XYZ tuple, or None when the field is absent (CAF-only clips) or
    malformed.

    The exporter applies the same CryEngine→Blender axis swap used for
    sample positions (`(cx, cy, cz) → (cx, -cz, cy)`), so the value is
    already in Blender frame and can be used directly as the anchor in
    `bind + (sample - anchor)`.
    """

    raw = clip.get("start_position")
    if not isinstance(raw, (list, tuple)) or len(raw) < 3:
        return None
    try:
        return (float(raw[0]), float(raw[1]), float(raw[2]))
    except (TypeError, ValueError):
        return None


def _clip_cyclic_transition_target_frame(clip: dict[str, Any]) -> float | None:
    if not _clip_has_transition_fragment(clip):
        return None
    position_moving = 0
    position_targets: list[float] = []
    rotation_moving = 0
    rotation_targets: list[float] = []
    for channel_variants in _normalized_bone_channels(clip).values():
        if not channel_variants:
            continue
        channel = channel_variants[0]
        positions = channel.get("position")
        if isinstance(positions, list):
            position_times = _channel_times(channel, "position_time", len(positions))
            is_cyclic, target = _series_cyclic_target_time(
                positions, position_times, 3, _position_distance, 0.05
            )
            if target is not None:
                position_moving += 1
                if is_cyclic:
                    position_targets.append(target)
        rotations = channel.get("rotation")
        if isinstance(rotations, list):
            rotation_times = _channel_times(channel, "rotation_time", len(rotations))
            is_cyclic, target = _series_cyclic_target_time(
                rotations, rotation_times, 4, _rotation_distance, 0.03
            )
            if target is not None:
                rotation_moving += 1
                if is_cyclic:
                    rotation_targets.append(target)

    if position_moving > 0:
        moving_series = position_moving
        cyclic_targets = position_targets
    else:
        moving_series = rotation_moving
        cyclic_targets = rotation_targets

    if moving_series == 0 or len(cyclic_targets) / moving_series < 0.5:
        return None
    cyclic_targets.sort()
    return cyclic_targets[len(cyclic_targets) // 2]


def _object_source_name_candidates(obj: bpy.types.Object) -> list[str]:
    names: list[str] = []
    seen_names: set[str] = set()

    def _add_name(value: Any) -> None:
        if not isinstance(value, str):
            return
        name = value.strip()
        if not name or name in seen_names:
            return
        seen_names.add(name)
        names.append(name)

    _add_name(obj.get(PROP_SOURCE_NODE_NAME))
    _add_name(obj.name)

    # Native .blend linked hierarchies can prefix source animation node names
    # with parent path + instance indices, e.g. geo_nose_167_anim_wing_top_left.
    # Hash suffixes after numeric path tokens so those objects still match the
    # source CAF/DBA bone hashes without asset-specific name branches.
    for raw_name in list(names):
        parts = raw_name.split("_")
        for index, part in enumerate(parts[:-1]):
            if not part.isdigit():
                continue
            _add_name("_".join(parts[index + 1:]))
        while len(parts) > 1 and parts[-1].isdigit():
            parts = parts[:-1]
            _add_name("_".join(parts))

    return names


def _object_bone_hash(obj: bpy.types.Object) -> str:
    return _object_bone_hash_keys(obj)[0]


def _object_bone_hash_keys(obj: bpy.types.Object) -> list[str]:
    import zlib

    keys: list[str] = []
    seen_keys: set[str] = set()
    for name in _object_source_name_candidates(obj):
        digest = zlib.crc32(name.encode("utf-8")) & 0xFFFFFFFF
        key = f"0x{digest:08X}"
        if key in seen_keys:
            continue
        seen_keys.add(key)
        keys.append(key)
    return keys


def _canonical_bone_hash_key(value: Any) -> str | None:
    if isinstance(value, str):
        text = value.strip()
        if not text:
            return None
        try:
            parsed = int(text, 16) if text.lower().startswith("0x") else int(text)
        except ValueError:
            return text
        return f"0x{parsed & 0xFFFFFFFF:08X}"
    if isinstance(value, int):
        return f"0x{value & 0xFFFFFFFF:08X}"
    return None


def _normalized_bone_channels(clip: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    bones = clip.get("bones")
    if not isinstance(bones, dict):
        return {}
    normalized: dict[str, list[dict[str, Any]]] = {}
    for raw_key, raw_channel in bones.items():
        channels: list[dict[str, Any]] = []
        if isinstance(raw_channel, dict):
            channels.append(raw_channel)
        elif isinstance(raw_channel, list):
            channels.extend(c for c in raw_channel if isinstance(c, dict))
        if not channels:
            continue
        key = _canonical_bone_hash_key(raw_key)
        if key is None:
            continue
        normalized[key] = channels
    return normalized


def _provenance_tokens(value: Any) -> set[str]:
    if not isinstance(value, str) or not value:
        return set()
    text = value.replace("\\", "/").lower()
    stem = Path(text).stem
    return {token for token in re.split(r"[^a-z0-9]+", stem) if len(token) > 1}


def _asset_stem(value: Any) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    stem = Path(value.replace("\\", "/").lower()).stem
    while True:
        stripped = re.sub(r"_(?:lod|tex|mip)\d+$", "", stem)
        if stripped == stem:
            return stem
        stem = stripped


def _asset_stem_score(asset_path: Any, channel_skeleton: str) -> int:
    asset_stem = _asset_stem(asset_path)
    skeleton_stem = _asset_stem(channel_skeleton)
    if not asset_stem or not skeleton_stem:
        return 0
    if asset_stem == skeleton_stem:
        return 14
    if asset_stem.startswith(f"{skeleton_stem}_"):
        return 10
    return 0


_DIRECTION_TOKENS = {"front", "rear", "left", "right", "upper", "lower", "top", "bottom"}


def _object_provenance_tokens(obj: bpy.types.Object, *, include_self: bool = True) -> set[str]:
    tokens: set[str] = set()
    current = obj if include_self else obj.parent
    visited = 0
    while current is not None and visited < 16:
        visited += 1
        for value in (
            current.get(PROP_SOURCE_NODE_NAME),
            current.get(PROP_TEMPLATE_PATH),
            current.name,
        ):
            tokens.update(_provenance_tokens(value))
        current = current.parent
    return tokens


def _channel_variant_score(obj: bpy.types.Object, channel: dict[str, Any]) -> int:
    obj_source = str(obj.get(PROP_SOURCE_NODE_NAME, "") or "").strip().lower()
    obj_template = str(obj.get(PROP_TEMPLATE_PATH, "") or "").strip().lower()
    obj_mesh_asset = str(obj.get(PROP_MESH_ASSET, "") or "").strip().lower()
    channel_source = str(channel.get("source_node_name", "") or "").strip().lower()
    channel_skeleton = str(channel.get("source_skeleton_path", "") or "").strip().lower()

    score = 0
    obj_sources = [name.lower() for name in _object_source_name_candidates(obj)]
    if channel_source:
        for source in obj_sources:
            if source == channel_source:
                score += 6
                break
            if source.endswith(channel_source) or channel_source.endswith(source):
                score += 4
                break

    if obj_template and channel_skeleton:
        template_tokens = _provenance_tokens(obj_template)
        skeleton_tokens = _provenance_tokens(channel_skeleton)
        overlap = template_tokens.intersection(skeleton_tokens)
        score += min(len(overlap), 4)

        template_dirs = template_tokens.intersection(_DIRECTION_TOKENS)
        skeleton_dirs = skeleton_tokens.intersection(_DIRECTION_TOKENS)
        if template_dirs and skeleton_dirs:
            direction_overlap = template_dirs.intersection(skeleton_dirs)
            if direction_overlap:
                score += 6 + len(direction_overlap)
            else:
                score -= 6

    if channel_skeleton:
        score += _asset_stem_score(obj_mesh_asset, channel_skeleton)
        score += _asset_stem_score(obj_template, channel_skeleton)

        object_tokens = _object_provenance_tokens(obj)
        skeleton_tokens = _provenance_tokens(channel_skeleton)
        overlap = object_tokens.intersection(skeleton_tokens)
        score += min(len(overlap), 4)

        direction_tokens = _object_provenance_tokens(obj, include_self=False) or object_tokens
        object_dirs = direction_tokens.intersection(_DIRECTION_TOKENS)
        skeleton_dirs = skeleton_tokens.intersection(_DIRECTION_TOKENS)
        if object_dirs and skeleton_dirs:
            direction_overlap = object_dirs.intersection(skeleton_dirs)
            if direction_overlap:
                score += 6 + len(direction_overlap)
            else:
                score -= 6

    if obj_source and channel_skeleton and obj_source in channel_skeleton:
        score += 2
    return score


def _select_channel_variant_for_object(
    obj: bpy.types.Object,
    channels: list[dict[str, Any]],
) -> dict[str, Any] | None:
    if not channels:
        return None
    if len(channels) == 1:
        return channels[0]
    scored = [(index, _channel_variant_score(obj, channel)) for index, channel in enumerate(channels)]
    best_index, _best_score = max(scored, key=lambda item: (item[1], -item[0]))
    return channels[best_index]


def _position_track_matches_bind(
    bind_loc: tuple[float, float, float],
    positions: list[Any],
    *,
    decoder: str = "identity",
    tol: float = 0.15,
) -> bool:
    tol_sq = tol * tol
    for sample in positions:
        if not isinstance(sample, list) or len(sample) < 3:
            continue
        decoded = _decode_animation_position(sample, decoder)
        dist_sq = (
            (decoded[0] - bind_loc[0]) ** 2
            + (decoded[1] - bind_loc[1]) ** 2
            + (decoded[2] - bind_loc[2]) ** 2
        )
        if dist_sq <= tol_sq:
            return True
    return False


def _shared_hash_position_policy(
    groups: dict[str, list[tuple[bpy.types.Object, dict[str, Any], dict[str, Any]]]],
    _legacy_bones: Any | None = None,
    *,
    decoder: str = "identity",
) -> dict[int, bool]:
    """Return per-object position eligibility for duplicate-hash channels.

    Absolute parent-local position tracks work for a shared hash only when the
    receiving object's bind pose is compatible with at least one sampled
    position. If a hash is shared across multiple instances and only a subset of
    those binds match the track, suppress position on the incompatible objects
    while still allowing rotation.
    """

    policy: dict[int, bool] = {}
    for key, entries in groups.items():
        if len(entries) <= 1:
            continue
        matches: list[bool] = []
        for entry in entries:
            if len(entry) == 3:
                _obj, bind_data, channel = entry
            elif len(entry) == 2:
                _obj, bind_data = entry
                if isinstance(_legacy_bones, dict):
                    channel = _legacy_bones.get(key)
                else:
                    channel = None
            else:
                matches.append(False)
                continue
            if not isinstance(channel, dict):
                matches.append(False)
                continue
            positions = channel.get("position")
            if not isinstance(positions, list) or not positions:
                matches.append(True)
                continue
            bind_location = bind_data.get("location")
            if not isinstance(bind_location, list) or len(bind_location) < 3:
                matches.append(False)
                continue
            bind_loc = (float(bind_location[0]), float(bind_location[1]), float(bind_location[2]))
            matches.append(_position_track_matches_bind(bind_loc, positions, decoder=decoder))

        if not any(matches) or all(matches):
            continue

        for entry, matched in zip(entries, matches):
            if len(entry) >= 1:
                obj = entry[0]
            else:
                continue
            policy[id(obj)] = matched
    return policy


def _animation_sample_decoder_score(
    candidates: list[tuple[bpy.types.Object, str, dict[str, Any], dict[str, Any]]],
    decoder: str,
) -> tuple[float, int]:
    score = 0.0
    evidence = 0
    for _obj, _key, bind_data, channel in candidates:
        bind_location = bind_data.get("location")
        positions = channel.get("position")
        if (
            isinstance(bind_location, list)
            and len(bind_location) >= 3
            and isinstance(positions, list)
        ):
            bind_loc = (float(bind_location[0]), float(bind_location[1]), float(bind_location[2]))
            samples = [
                sample
                for sample in (positions[:1] + positions[-1:])
                if isinstance(sample, list) and len(sample) >= 3
            ]
            if samples:
                best = min(
                    _vec_distance_sq(_decode_animation_position(sample, decoder), bind_loc)
                    for sample in samples
                )
                score += min(best, 1.0)
                evidence += 1

        bind_rotation = bind_data.get("rotation_quaternion")
        rotations = channel.get("rotation")
        if (
            isinstance(bind_rotation, list)
            and len(bind_rotation) >= 4
            and isinstance(rotations, list)
        ):
            bind_rot = (
                float(bind_rotation[0]),
                float(bind_rotation[1]),
                float(bind_rotation[2]),
                float(bind_rotation[3]),
            )
            samples = [
                sample
                for sample in (rotations[:1] + rotations[-1:])
                if isinstance(sample, list) and len(sample) >= 4
            ]
            if samples:
                best = min(
                    _quat_distance_sq(_decode_animation_rotation(sample, decoder), bind_rot)
                    for sample in samples
                )
                score += min(best * 4.0, 1.0)
                evidence += 1

    return score, evidence


def _select_animation_sample_decoder(
    candidates: list[tuple[bpy.types.Object, str, dict[str, Any], dict[str, Any]]],
) -> str:
    scored = [
        (_animation_sample_decoder_score(candidates, decoder), decoder)
        for decoder in _ANIMATION_SAMPLE_DECODERS
    ]
    scored = [item for item in scored if item[0][1] > 0]
    if not scored:
        return "identity"
    scored.sort(key=lambda item: (item[0][0] / item[0][1], item[0][0], item[1] != "identity"))
    best_score, best_decoder = scored[0]
    identity = next((score for score, decoder in scored if decoder == "identity"), None)
    if identity is not None:
        best_average = best_score[0] / best_score[1]
        identity_average = identity[0] / identity[1]
        if abs(identity_average - best_average) <= 1.0e-8:
            return "identity"
    return best_decoder


def _animation_bone_candidates(
    package_root: bpy.types.Object,
    bones: dict[str, list[dict[str, Any]]],
) -> tuple[list[tuple[bpy.types.Object, str, dict[str, Any], dict[str, Any]]], dict[int, bool], str]:
    candidates: list[tuple[bpy.types.Object, str, dict[str, Any], dict[str, Any]]] = []
    groups: dict[str, list[tuple[bpy.types.Object, dict[str, Any], dict[str, Any]]]] = {}

    for obj in _iter_candidate_bone_objects(package_root):
        key = None
        variants = None
        for candidate_key in _object_bone_hash_keys(obj):
            key = _canonical_bone_hash_key(candidate_key) or candidate_key
            variants = bones.get(key)
            if isinstance(variants, list) and variants:
                break
        if key is None or not isinstance(variants, list) or not variants:
            continue
        channel = _select_channel_variant_for_object(obj, variants)
        if not isinstance(channel, dict):
            continue
        _store_bind_pose_once(obj)
        bind_data = _bind_pose_payload(obj)
        if bind_data is None:
            continue

        candidates.append((obj, key, bind_data, channel))
        groups.setdefault(key, []).append((obj, bind_data, channel))

    decoder = _select_animation_sample_decoder(candidates)
    return candidates, _shared_hash_position_policy(groups, decoder=decoder), decoder


def _iter_candidate_bone_objects(package_root: bpy.types.Object) -> list[bpy.types.Object]:
    return [obj for obj in _iter_package_objects(package_root) if obj.type in {"EMPTY", "MESH"}]


def _store_bind_pose_once(obj: bpy.types.Object) -> None:
    if isinstance(obj.get(_ANIMATION_BIND_TRS_PROP), str):
        return
    parent_distance = None
    if obj.parent is not None:
        parent_distance = float((obj.matrix_world.translation - obj.parent.matrix_world.translation).length)
    payload = {
        "location": [float(v) for v in obj.location],
        "rotation_mode": str(obj.rotation_mode),
        "rotation_quaternion": [float(v) for v in obj.rotation_quaternion],
        "parent_distance": parent_distance,
    }
    obj[_ANIMATION_BIND_TRS_PROP] = json.dumps(payload, separators=(",", ":"))


def _bind_pose_payload(obj: bpy.types.Object) -> dict[str, Any] | None:
    payload = obj.get(_ANIMATION_BIND_TRS_PROP)
    if not isinstance(payload, str) or not payload:
        return None
    try:
        data = json.loads(payload)
    except json.JSONDecodeError:
        return None
    return data if isinstance(data, dict) else None


def _restore_object_bind_pose(obj: bpy.types.Object, data: dict[str, Any]) -> None:
    location = data.get("location")
    rotation_mode = data.get("rotation_mode")
    rotation_quaternion = data.get("rotation_quaternion")
    if isinstance(location, list) and len(location) >= 3:
        obj.location = (float(location[0]), float(location[1]), float(location[2]))
    if isinstance(rotation_mode, str):
        obj.rotation_mode = rotation_mode
    if isinstance(rotation_quaternion, list) and len(rotation_quaternion) >= 4:
        obj.rotation_mode = "QUATERNION"
        obj.rotation_quaternion = (
            float(rotation_quaternion[0]),
            float(rotation_quaternion[1]),
            float(rotation_quaternion[2]),
            float(rotation_quaternion[3]),
        )


def _is_parent_distance_outlier(obj: bpy.types.Object, data: dict[str, Any]) -> bool:
    if obj.parent is None:
        return False
    bind_distance_raw = data.get("parent_distance")
    if not isinstance(bind_distance_raw, (int, float)):
        return False

    bind_distance = float(bind_distance_raw)
    current_distance = float((obj.matrix_world.translation - obj.parent.matrix_world.translation).length)

    if bind_distance <= 1e-5:
        return current_distance > 0.25

    ratio = current_distance / bind_distance
    return abs(current_distance - bind_distance) > 0.75 and (ratio > 2.5 or ratio < 0.4)


def _apply_candidate_transform(
    obj: bpy.types.Object,
    bind_data: dict[str, Any],
    rotation_sample: list[Any] | None,
    position_sample: list[Any] | None,
    *,
    rotation_order: str,
    use_position: bool,
    decoder: str | None,
) -> None:
    _restore_object_bind_pose(obj, bind_data)

    if rotation_sample is not None and len(rotation_sample) >= 4:
        obj.rotation_mode = "QUATERNION"
        if rotation_order == "xyzw":
            obj.rotation_quaternion = (
                float(rotation_sample[3]),
                float(rotation_sample[0]),
                float(rotation_sample[1]),
                float(rotation_sample[2]),
            )
        else:
            obj.rotation_quaternion = (
                float(rotation_sample[0]),
                float(rotation_sample[1]),
                float(rotation_sample[2]),
                float(rotation_sample[3]),
            )

    if use_position and decoder is not None and position_sample is not None and len(position_sample) >= 3:
        obj.location = _decode_animation_position(position_sample, decoder)


def _candidate_parent_distance_error(obj: bpy.types.Object, bind_data: dict[str, Any]) -> float:
    if obj.parent is None:
        return 0.0
    bind_distance_raw = bind_data.get("parent_distance")
    if not isinstance(bind_distance_raw, (int, float)):
        return 0.0
    bind_distance = float(bind_distance_raw)
    current_distance = float((obj.matrix_world.translation - obj.parent.matrix_world.translation).length)
    return abs(current_distance - bind_distance)


def _apply_best_channel_transform(
    obj: bpy.types.Object,
    bind_data: dict[str, Any],
    channel: dict[str, Any],
    frame_index: int,
    endpoint_policy: str,
    target_frame: float | None = None,
    anchor_frame: float | None = None,
    allow_rotation: bool = True,
    allow_position: bool = True,
    sample_decoder: str = "identity",
) -> None:
    _restore_object_bind_pose(obj, bind_data)

    rotations = channel.get("rotation")
    positions = channel.get("position")

    bind_location = bind_data.get("location", obj.location)
    bind_loc = (float(bind_location[0]), float(bind_location[1]), float(bind_location[2]))

    bind_quaternion = bind_data.get("rotation_quaternion", obj.rotation_quaternion)
    bind_rot = (
        float(bind_quaternion[0]),
        float(bind_quaternion[1]),
        float(bind_quaternion[2]),
        float(bind_quaternion[3]),
    )

    def _select_sample(values: list[Any], item_len: int) -> list[Any] | None:
        valid: list[list[Any]] = [v for v in values if isinstance(v, list) and len(v) >= item_len]
        if not valid:
            return None
        # Phase 53: data-backed sample selection. The exporter classifies
        # each bone as Additive vs Override (see classify_bone_blend_modes
        # in starbreaker-3d), and the addon composes
        # `result = bind · (start⁻¹ · sample)` for Additive bones using
        # the clip's DBA-metadata `start_rotation` / `start_position`
        # (whitepaper §14.6) as the anchor. The selector here only
        # decides WHICH sample to read; cyclic-target frames are handled
        # upstream by `_clip_cyclic_transition_target_frame`.
        if endpoint_policy == "transition_start":
            return valid[0]
        if endpoint_policy == "transition_end":
            return valid[-1]
        # Default ("literal" + any unrecognised string): first at
        # frame_index=0, last otherwise. Matches engine playback.
        return valid[0] if frame_index == 0 else valid[-1]

    rotation_sample: list[Any] | None = None
    if isinstance(rotations, list) and rotations:
        if target_frame is not None:
            rotation_sample = _sample_nearest_time(
                rotations, _channel_times(channel, "rotation_time", len(rotations)), 4, target_frame
            )
        else:
            rotation_sample = _select_sample(rotations, 4)

    position_sample: list[Any] | None = None
    if isinstance(positions, list) and positions:
        if target_frame is not None:
            position_sample = _sample_nearest_time(
                positions, _channel_times(channel, "position_time", len(positions)), 3, target_frame
            )
        else:
            position_sample = _select_sample(positions, 3)

    if allow_rotation and rotation_sample is not None:
        obj.rotation_mode = "QUATERNION"
        rot_sample_q = (
            *_decode_animation_rotation(rotation_sample, sample_decoder),
        )
        rot_sample_q = _bind_equivalent_rotation(rot_sample_q, bind_rot)
        blend_mode = str(channel.get("blend_mode") or "").lower()
        if blend_mode == "override":
            # Override mode (Phase 38): the exporter classified this bone's
            # CHR-bind as outside the AABB of all CAF position samples, which
            # means the clip authors meant the channel to *replace* the bind
            # pose rather than ride on top of it. Use the sampled rotation
            # verbatim — no anchor-relative composition.
            obj.rotation_quaternion = rot_sample_q
        # For transition_start/transition_end: CAF clips store absolute
        # bone rotations in parent-local space, not deltas. Using verbatim
        # sample is correct for all current cases (cyclic clips have
        # first=last=bind so verbatim is trivially bind; non-cyclic clips
        # author the deployed/stowed state directly as the sample value).
        # Bind-compose (`bind @ first⁻¹ @ sample`) was incorrect: for
        # non-cyclic deploy clips where last≈bind it doubles the rotation,
        # and the hemisphere-alignment step introduces additional sign errors
        # when first_rot.w < 0 (e.g. DRAK Clipper Foot_joint_anim).
        obj.rotation_quaternion = rot_sample_q

    if allow_position and position_sample is not None and isinstance(positions, list) and positions:
        sample_decoded = _bind_equivalent_position(_decode_animation_position(position_sample, sample_decoder), bind_loc)
        blend_mode = str(channel.get("blend_mode") or "").lower()
        if blend_mode == "override":
            # Override mode (Phase 38): use the sampled position verbatim.
            # The CHR-bind is outside the AABB of CAF samples, so anchor-
            # relative composition would land the bone in the wrong place
            # (canonical example: Scorpius BONE_Front_Landing_Gear_Foot).
            obj.location = sample_decoded
            return
        # Per-bone position anchor. The DBA `start_position` field
        # is plumbed through scene.json (whitepaper §14.6) but is
        # *not* used here: it is a clip-root-frame value, not in the
        # same coordinate space as per-bone parent-local samples.
        # Anchor selection picks whichever of {first sample,
        # anchor_frame sample} sits nearer to bind — that is the
        # endpoint the engine treats as the bone's resting state in
        # clip-frame, with the other endpoint being the "moved"
        # pose. The same shape was used pre-Phase-53; only the
        # rotation pathway needed the heuristic-removal that Phase
        # 52 / Phase 53 landed.
        valid_positions: list[list[Any]] = [v for v in positions if isinstance(v, list) and len(v) >= 3]
        if valid_positions:
            first_decoded = _bind_equivalent_position(_decode_animation_position(valid_positions[0], sample_decoder), bind_loc)
            if anchor_frame is not None:
                anchor_target = _sample_nearest_time(
                    positions, _channel_times(channel, "position_time", len(positions)), 3, anchor_frame
                )
                if anchor_target is not None:
                    anchor_target_decoded = _bind_equivalent_position(
                        _decode_animation_position(anchor_target, sample_decoder),
                        bind_loc,
                    )
                    d_first = (
                        (first_decoded[0] - bind_loc[0]) ** 2
                        + (first_decoded[1] - bind_loc[1]) ** 2
                        + (first_decoded[2] - bind_loc[2]) ** 2
                    )
                    d_target = (
                        (anchor_target_decoded[0] - bind_loc[0]) ** 2
                        + (anchor_target_decoded[1] - bind_loc[1]) ** 2
                        + (anchor_target_decoded[2] - bind_loc[2]) ** 2
                    )
                    anchor_decoded = first_decoded if d_first <= d_target else anchor_target_decoded
                else:
                    anchor_decoded = first_decoded
                obj.location = (
                    bind_loc[0] + (sample_decoded[0] - anchor_decoded[0]),
                    bind_loc[1] + (sample_decoded[1] - anchor_decoded[1]),
                    bind_loc[2] + (sample_decoded[2] - anchor_decoded[2]),
                )
            else:
                # Non-cyclic clips (no cyclic-anchor frame) store absolute
                # parent-local bone positions. Apply verbatim — no bind-compose.
                # The bind-compose was wrong for bones shared across multiple
                # instances that have different bind positions (e.g. DRAK Clipper
                # side swingarms whose bind ≠ channel endpoints).
                obj.location = sample_decoded
        else:
            obj.location = sample_decoded


def _restore_bind_pose(package_root: bpy.types.Object) -> int:
    restored = 0
    for obj in _iter_candidate_bone_objects(package_root):
        data = _bind_pose_payload(obj)
        if data is None:
            continue
        _restore_object_bind_pose(obj, data)
        restored += 1
    return restored


def _apply_animation_pose(
    package_root: bpy.types.Object,
    clip: dict[str, Any],
    frame_index: int,
    endpoint_policy: str = "literal",
    target_frame: float | None = None,
    anchor_frame: float | None = None,
) -> int:
    bones = _normalized_bone_channels(clip)
    if not bones:
        return 0
    candidates, position_policy, sample_decoder = _animation_bone_candidates(package_root, bones)
    updated = 0
    for obj, key, bind_data, channel in candidates:
        allow_position = position_policy.get(id(obj), True)

        _apply_best_channel_transform(
            obj,
            bind_data,
            channel,
            frame_index,
            endpoint_policy,
            target_frame,
            anchor_frame,
            # Duplicate-hash disambiguation only suppresses incompatible
            # position tracks; rotation remains valid and must still apply.
            allow_rotation=True,
            allow_position=allow_position,
            sample_decoder=sample_decoder,
        )
        updated += 1
    return updated


def _action_fcurves(action: Any) -> list[Any]:
    """Return all fcurves on `action`, supporting both the legacy
    `Action.fcurves` collection (Blender ≤4.3) and the layered
    `action.layers[*].strips[*].channelbag(slot).fcurves` storage
    introduced in Blender 4.4 (and now exclusive in 5.1+).

    Returns an empty list if neither storage is reachable.
    """

    legacy = getattr(action, "fcurves", None)
    if legacy is not None:
        try:
            return list(legacy)
        except Exception:
            return []
    out: list[Any] = []
    layers = getattr(action, "layers", None) or []
    slots = getattr(action, "slots", None) or []
    for layer in layers:
        strips = getattr(layer, "strips", None) or []
        for strip in strips:
            for slot in slots:
                try:
                    channelbag = strip.channelbag(slot)
                except Exception:
                    continue
                if channelbag is None:
                    continue
                cb_fcurves = getattr(channelbag, "fcurves", None)
                if cb_fcurves is None:
                    continue
                try:
                    out.extend(cb_fcurves)
                except Exception:
                    continue
    return out


def _action_groups_collection(action: Any) -> Any:
    """Return a groups-like collection (with `.get(name)` and
    `.new(name)`) for `action`, supporting both legacy Actions
    (Blender ≤4.3) and the layered-action API (Blender 4.4+ /
    5.1+). Returns None if no channelbag is reachable yet.

    On layered Actions a channelbag for the first available slot is
    used; the collection only exists once at least one keyframe has
    been inserted via `obj.keyframe_insert`, so callers must defer
    grouping until after their keyframe pass.
    """

    legacy = getattr(action, "groups", None)
    if legacy is not None:
        return legacy
    layers = getattr(action, "layers", None) or []
    slots = getattr(action, "slots", None) or []
    for layer in layers:
        strips = getattr(layer, "strips", None) or []
        for strip in strips:
            for slot in slots:
                try:
                    channelbag = strip.channelbag(slot, ensure=True)
                except TypeError:
                    try:
                        channelbag = strip.channelbag(slot)
                    except Exception:
                        continue
                except Exception:
                    continue
                if channelbag is None:
                    continue
                cb_groups = getattr(channelbag, "groups", None)
                if cb_groups is not None:
                    return cb_groups
    return None


def _insert_animation_action(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    clip: dict[str, Any],
    animation_name: str | None = None,
    reverse_playback: bool = False,
    fps_policy: str = FPS_POLICY_ADAPT_SCENE,
) -> int:
    bones = _normalized_bone_channels(clip)
    if not bones:
        return 0
    name = animation_name or str(clip.get("name", "animation")) or "animation"
    visible_name = _animation_insert_label(animation_name, clip)
    trim_frame = _clip_cyclic_transition_target_frame(clip)
    fps_reconciliation = reconcile_animation_fps(context.scene, clip.get("fps"), fps_policy)
    if fps_reconciliation.mismatch:
        print(describe_reconciliation(name, fps_reconciliation))

    frame_offset = float(getattr(context.scene, "frame_current", 1.0))
    duration_frames = _clip_duration_frames(clip, fps_reconciliation.frame_scale)
    existing_instances = _load_animation_instances(package_root)
    overlap_warnings = _animation_overlap_warnings(
        name,
        frame_offset,
        duration_frames,
        _clip_bone_hashes(clip),
        existing_instances,
    )
    for warning in overlap_warnings:
        print(f"[StarBreaker] WARNING: {warning}")
    instance_id = uuid.uuid4().hex
    instance = {
        "id": instance_id,
        "animation_name": name,
        "start_frame": frame_offset,
        "duration_frames": duration_frames,
        "driven_hashes": sorted(_clip_bone_hashes(clip)),
    }

    candidates, position_policy, sample_decoder = _animation_bone_candidates(package_root, bones)
    updated = 0
    # Multi-clip mode: when prior instances exist, clear live action from all
    # objects so NLA is the sole driver.  This covers the case where the first
    # insert left anim.action set (single-clip Dopesheet mode) and a second
    # insert is now being added.
    if existing_instances:
        for obj in _iter_package_objects(package_root):
            animation_data = getattr(obj, "animation_data", None)
            if animation_data is not None and getattr(animation_data, "action", None) is not None:
                try:
                    animation_data.action = None
                except Exception:
                    pass
    for obj, key, bind_data, channel in candidates:
        _restore_object_bind_pose(obj, bind_data)
        obj.rotation_mode = "QUATERNION"
        obj.animation_data_create()

        # Phase 24C: each animated object gets its own Action, named after
        # the clip + bone, and grouped by the bone's display name so the
        # Dope Sheet Action editor shows clean per-bone groups. The Action
        # is pushed onto a per-clip NLA track so multiple clips coexist on
        # the timeline without overwriting each other.
        action_name = f"SB_{package_root.name}_{visible_name}_{instance_id}_{obj.name}"
        action = bpy.data.actions.new(name=action_name)
        try:
            action[_ANIMATION_INSTANCE_ID_PROP] = instance_id
            action[_ANIMATION_INSTANCE_NAME_PROP] = name
        except Exception:
            pass
        # Temporarily set anim.action to the new action so keyframe_insert()
        # writes into the correct target.  After all keyframes are done the
        # live action pointer is cleared (NLA strips drive playback).
        obj.animation_data.action = action
        group_name = obj.name
        # Phase 39: defer group creation until after keyframes are
        # inserted. On Blender 5.1+ a freshly-created Action has no
        # layers/strips/slots/channelbags until the first keyframe is
        # inserted, and the legacy `Action.groups` collection has been
        # removed. Looking up `action.groups` before keyframes exist
        # would raise AttributeError mid-loop and abort all subsequent
        # bones (this is the regression that left only the first bone
        # animated when running Insert Action on Wings Deploy).

        rotations = channel.get("rotation") if isinstance(channel.get("rotation"), list) else []
        positions = channel.get("position") if isinstance(channel.get("position"), list) else []
        rotation_times = _channel_times(channel, "rotation_time", len(rotations))
        position_times = _channel_times(channel, "position_time", len(positions))
        channel_times = [*rotation_times, *position_times]
        duration_frame = trim_frame if trim_frame is not None else max(channel_times, default=0.0)
        allow_position = position_policy.get(id(obj), True)

        def _action_frame(sample_time: float) -> float:
            local_time = duration_frame - sample_time if reverse_playback else sample_time
            return frame_offset + (local_time * fps_reconciliation.frame_scale)

        if allow_position and positions:
            bind_location = bind_data.get("location", obj.location)
            bind = (float(bind_location[0]), float(bind_location[1]), float(bind_location[2]))

            # Per-bone first-sample anchor. The DBA `start_position`
            # field is plumbed through scene.json (whitepaper §14.6)
            # but is *not* used here: it is a clip-root-frame value,
            # not directly compatible with per-bone parent-local
            # samples. The bind-distance first/last heuristic that
            # lived here previously is retired — it masked the same
            # cyclic-channel ambiguity the snap-pose path suffered
            # from (Phase 52 evidence).
            first = positions[0] if isinstance(positions[0], list) and len(positions[0]) >= 3 else None
            anchor: tuple[float, float, float] | None = None
            if first is not None:
                anchor = _bind_equivalent_position(_decode_animation_position(first, sample_decoder), bind)

            for index, sample in enumerate(positions):
                sample_time = position_times[index] if index < len(position_times) else float(index)
                if trim_frame is not None and sample_time > trim_frame:
                    continue
                if isinstance(sample, list) and len(sample) >= 3:
                    sample_decoded = _bind_equivalent_position(_decode_animation_position(sample, sample_decoder), bind)
                    if trim_frame is not None and anchor is not None:
                        # Cyclic clip: use bind-delta to anchor the animation
                        # at the bind pose (e.g. Scorpius landing_gear_deploy).
                        obj.location = (
                            bind[0] + (sample_decoded[0] - anchor[0]),
                            bind[1] + (sample_decoded[1] - anchor[1]),
                            bind[2] + (sample_decoded[2] - anchor[2]),
                        )
                    else:
                        # Non-cyclic clip: CAF stores absolute parent-local
                        # positions — apply verbatim.
                        obj.location = sample_decoded
                    obj.keyframe_insert(data_path="location", frame=_action_frame(sample_time))

        # Phase 47.3: align each rotation sample to the previous *keyed*
        # sample's hemisphere so per-component LINEAR interpolation stays
        # on the short arc. Without this, a sign flip between consecutive
        # source samples (q vs -q — same rotation) makes Blender lerp
        # through ~0 at the midpoint, producing a spurious 180° "inversion"
        # frame between the two keys (observed on Scorpius
        # `landing_gear_extend` BONE_Front_Landing_Gear_Foot frames 37→39).
        prev_keyed_quat: tuple[float, float, float, float] | None = None
        bind_rotation = bind_data.get("rotation_quaternion", obj.rotation_quaternion)
        bind_rot = (
            float(bind_rotation[0]),
            float(bind_rotation[1]),
            float(bind_rotation[2]),
            float(bind_rotation[3]),
        )
        if rotations:
            for index, sample in enumerate(rotations):
                sample_time = rotation_times[index] if index < len(rotation_times) else float(index)
                if trim_frame is not None and sample_time > trim_frame:
                    continue
                if isinstance(sample, list) and len(sample) >= 4:
                    sample_q = _bind_equivalent_rotation(_decode_animation_rotation(sample, sample_decoder), bind_rot)
                    if prev_keyed_quat is not None:
                        sample_q = _quat_align(prev_keyed_quat, sample_q)
                    obj.rotation_quaternion = sample_q
                    obj.keyframe_insert(data_path="rotation_quaternion", frame=_action_frame(sample_time))
                    prev_keyed_quat = sample_q

        # Phase 24C / Phase 39: assign all fcurves on this action to the
        # bone's group so the Action editor renders a single collapsible
        # group per bone. On Blender 5.1+ both `Action.groups` and
        # `Action.fcurves` are removed in favor of the layered-action
        # API (`action.layers[*].strips[*].channelbag(slot)`); the
        # helpers `_action_groups_collection` and `_action_fcurves`
        # transparently support both storage models.
        groups_collection = _action_groups_collection(action)
        bone_group = None
        if groups_collection is not None:
            try:
                bone_group = groups_collection.get(group_name)
            except Exception:
                bone_group = None
            if bone_group is None:
                try:
                    bone_group = groups_collection.new(group_name)
                except Exception:
                    bone_group = None
        if bone_group is not None:
            for fcurve in _action_fcurves(action):
                try:
                    if fcurve.group is None:
                        fcurve.group = bone_group
                except Exception:
                    continue

        # Phase 46: force LINEAR interpolation on every inserted keyframe
        # to faithfully reproduce CryEngine's runtime playback. CryEngine
        # interpolates compressed channels linearly (position lerp, rotation
        # quaternion slerp) between adjacent keys; Blender's
        # keyframe_insert() default depends on the user pref and is often
        # BEZIER, which adds easing that the engine does not produce.
        # CONSTANT (also a possible default) produces visible step-judder.
        # LINEAR matches engine semantics exactly: per-component linear
        # interpolation on quaternion fcurves approximates slerp closely
        # enough that the cumulative rotation remains correct, and the
        # source data is dense enough (~24-35 keys for a 75-frame clip)
        # that residual lerp-vs-slerp drift is invisible.
        for fcurve in _action_fcurves(action):
            try:
                for keyframe in fcurve.keyframe_points:
                    keyframe.interpolation = "LINEAR"
                fcurve.update()
            except Exception:
                continue

        # Phase 24C / Phase 46: push the per-object action onto a per-clip
        # NLA track so the clip is visible as a movable "block" in the NLA
        # editor (entire clips can be slid to a different start frame by
        # selecting all the per-bone strips on the same NLA track and
        # pressing G). The strip is muted so the live action drives
        # playback (avoiding double-evaluation), and the live action keeps
        # `anim.action = action` so the Dope Sheet / Action Editor shows
        # the keyframes immediately for whichever bone object is selected.
        # Users wanting NLA-only playback can mute the action and unmute
        # the strip via the NLA editor.
        anim = obj.animation_data
        try:
            frame_range_low, frame_range_high = action.frame_range
            has_range = float(frame_range_high) > float(frame_range_low)
        except Exception:
            has_range = False
        if anim is not None and has_range:
            track_name = f"{name}@{int(round(frame_offset))}"
            track = anim.nla_tracks.get(track_name)
            if track is None:
                track = anim.nla_tracks.new()
                track.name = track_name
            try:
                strip_start = int(round(float(action.frame_range[0])))
            except Exception:
                strip_start = int(round(float(frame_offset)))
            strip = None
            try:
                strip = track.strips.new(name=visible_name, start=strip_start, action=action)
                strip.name = visible_name
                strip.extrapolation = 'HOLD_FORWARD'
                try:
                    strip[_ANIMATION_INSTANCE_ID_PROP] = instance_id
                    strip[_ANIMATION_INSTANCE_NAME_PROP] = name
                except Exception:
                    pass
            except Exception:
                # Strip may already exist at that frame; reuse the
                # most-recently-added strip on this track.
                if track.strips:
                    strip = track.strips[-1]
            # Phase 62: NLA strips drive playback for multi-insert scenarios.
            # Leave strips unmuted so Blender's NLA evaluates them at their
            # scheduled frame ranges.  When multiple clips are inserted on
            # separate non-overlapping tracks, the NLA plays each clip at
            # the right time without blending artifacts.
            # For single-clip mode (no prior instances), keep anim.action set
            # so keyframes are visible in the Action Editor / Dopesheet.
            # For multi-clip mode, clear anim.action so NLA drives playback.
            if anim is not None and existing_instances:
                anim.action = None

        updated += 1

    if updated > 0:
        existing_instances.append(instance)
        _store_animation_instances(package_root, existing_instances)

    return updated


def _animation_position_to_blender_local(sample: list[Any]) -> tuple[float, float, float]:
    # Backwards-compatible default decoder retained for older call sites.
    return _decode_animation_position(sample, "legacy")


def _decode_animation_position(sample: list[Any], decoder: str) -> tuple[float, float, float]:
    x = float(sample[0])
    y = float(sample[1])
    z = float(sample[2])
    if decoder == "source":
        # Native .blend hierarchy nodes can retain source/CryEngine local axes
        # while sidecars remain authored in the exported Blender-frame contract.
        return (x, z, -y)
    if decoder == "legacy":
        # Legacy export decode: [x, z, -y] -> (x, y, z_blender)
        return (x, -z, y)
    if decoder == "swizzled":
        # Alternate export decode: [cry_y, -cry_z, cry_x] -> (cry_x, -cry_z, cry_y)
        return (z, y, x)
    # "identity": already-authored Blender XYZ.
    return (x, y, z)


def _decode_animation_rotation(sample: list[Any], decoder: str) -> tuple[float, float, float, float]:
    w = float(sample[0])
    x = float(sample[1])
    y = float(sample[2])
    z = float(sample[3])
    if decoder == "source":
        return (w, x, z, -y)
    return (w, x, y, z)


def _select_position_decoder(
    positions: list[Any],
    bind_location: Any,
    frame_index: int | None = None,
) -> str | None:
    if not isinstance(positions, list) or not positions:
        return None

    valid_samples = [sample for sample in positions if isinstance(sample, list) and len(sample) >= 3]
    if not valid_samples:
        return None

    bind = (float(bind_location[0]), float(bind_location[1]), float(bind_location[2]))
    candidates = ("legacy", "swizzled", "identity", "source")

    if frame_index is None:
        anchor_samples = [valid_samples[0], valid_samples[-1]]
    else:
        anchor_samples = [valid_samples[0] if frame_index == 0 else valid_samples[-1]]

    def _distance_sq(loc: tuple[float, float, float]) -> float:
        dx = loc[0] - bind[0]
        dy = loc[1] - bind[1]
        dz = loc[2] - bind[2]
        return dx * dx + dy * dy + dz * dz

    scored = [
        (
            min(_distance_sq(_decode_animation_position(sample, decoder)) for sample in anchor_samples),
            decoder,
        )
        for decoder in candidates
    ]
    scored.sort(key=lambda item: item[0])

    # If even the closest decode is far from bind pose, treat translation keys
    # as unreliable for this channel and keep bind translation.
    if scored[0][0] > 0.25:  # 0.5m squared
        return None
    return scored[0][1]
