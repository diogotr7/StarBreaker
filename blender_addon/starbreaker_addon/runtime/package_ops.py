"""Public entry points and package-lifecycle helpers.

Extracted in Phase 7.4. These are the functions the rest of the add-on
(``ui.py``, operators) calls into. They orchestrate
:class:`PackageImporter` (which still lives in ``_legacy.py`` for now).

``PackageImporter`` is imported lazily inside each function to avoid a
circular import between this module and ``_legacy``.
"""

from __future__ import annotations

import json
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Callable

import bpy

from ..manifest import PackageBundle, SceneInstanceRecord
from ..palette import palette_id_for_livery_instance, resolved_palette_id
from .constants import (
    PROP_INSTANCE_JSON,
    PROP_LIGHT_ACTIVE_STATE,
    PROP_LIGHT_STATES_JSON,
    PROP_MATERIAL_SIDECAR,
    PROP_PACKAGE_ROOT,
    PROP_PAINT_VARIANT_SIDECAR,
    PROP_PALETTE_ID,
    PROP_SCENE_PATH,
    PROP_SOURCE_NODE_NAME,
    PROP_SUBMATERIAL_JSON,
)
from .validators import _purge_orphaned_file_backed_images, _purge_orphaned_runtime_groups


def import_package(
    context: bpy.types.Context,
    scene_path: str | Path,
    prefer_cycles: bool = True,
    palette_id: str | None = None,
    progress_callback: Callable[[float, str], None] | None = None,
) -> bpy.types.Object:
    from .importer import PackageImporter

    package = PackageBundle.load(scene_path)
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
    importer = PackageImporter(context, package, package_root=package_root)
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

    importer = PackageImporter(context, package, package_root=package_root)
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
    importer = PackageImporter(context, package, package_root=package_root)
    applied = 0
    with _suspend_heavy_viewports(context), _temporary_object_mode(context):
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
        if not switched or active_object is None or view_layer is None:
            return
        try:
            if view_layer.objects.active is not active_object:
                view_layer.objects.active = active_object
            _mode_set(original_mode)
        except Exception:
            pass


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


def _apply_state_to_light(light: bpy.types.Light, state_name: str) -> bool:
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


def apply_light_state(state_name: str) -> int:
    """Switch every StarBreaker light in the current .blend to the named
    state. Lights that lack the requested state keep their current values.
    Returns the number of lights that were updated."""
    updated = 0
    for light in _iter_starbreaker_lights():
        if _apply_state_to_light(light, state_name):
            updated += 1
    return updated


_ANIMATION_MODES_PROP = "starbreaker_animation_modes"
_ANIMATION_BIND_TRS_PROP = "starbreaker_animation_bind_trs"


def available_package_animation_names(package: PackageBundle) -> list[str]:
    """Return animation names exported on the package root entity."""
    return [name for name, _ in available_package_animation_items(package)]


def available_package_animation_items(package: PackageBundle) -> list[tuple[str, str]]:
    """Return ``(clip_name, display_name)`` pairs for exported animations.

    ``clip_name`` is the canonical sidecar key used for lookups. ``display_name``
    prefers localized metadata when present, then falls back to a shortened path.
    """
    items: list[tuple[str, str]] = []
    for clip in _animation_clips(package):
        clip_name = str(clip.get("name", "")).strip()
        if not clip_name:
            continue
        items.append((clip_name, _animation_display_name(clip)))
    return items


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
        channel_hashes = [str(key) for key in bones.keys() if isinstance(key, str)]

    hash_to_objects: dict[str, list[str]] = {}
    for obj in _iter_candidate_bone_objects(package_root):
        bone_hash = _object_bone_hash(obj)
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
) -> int:
    """Apply one animation in one of: none, snap_first, snap_last, action."""
    package = _load_package_from_root(package_root)
    clip = _find_animation_clip(package, animation_name)
    if clip is None:
        raise RuntimeError(f"Animation '{animation_name}' not found in package sidecar")

    normalized_mode = mode.strip().lower()
    if normalized_mode not in {"none", "snap_first", "snap_last", "action"}:
        raise RuntimeError(f"Unsupported animation mode: {mode}")

    updated = 0
    if normalized_mode == "none":
        updated = _restore_bind_pose(package_root)
    elif normalized_mode in {"snap_first", "snap_last"}:
        frame_index = 0 if normalized_mode == "snap_first" else -1
        updated = _apply_animation_pose(package_root, clip, frame_index)
        if updated == 0:
            paired = _paired_clip_for_snap(package, clip, frame_index)
            if paired is not None:
                paired_clip, paired_frame_index = paired
                updated = _apply_animation_pose(package_root, paired_clip, paired_frame_index)
    else:
        updated = _insert_animation_action(context, package_root, clip)

    mode_map = package_animation_mode_map(package_root)
    mode_map[animation_name] = normalized_mode
    package_root[_ANIMATION_MODES_PROP] = json.dumps(mode_map, separators=(",", ":"), sort_keys=True)
    return updated


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


def _animation_display_name(clip: dict[str, Any]) -> str:
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
    return filename or shortened or raw_name


def _find_animation_clip(package: PackageBundle, animation_name: str) -> dict[str, Any] | None:
    target = animation_name.strip()
    if not target:
        return None
    for clip in _animation_clips(package):
        if str(clip.get("name", "")).strip() == target:
            return clip
    return None


def _paired_clip_for_snap(
    package: PackageBundle,
    clip: dict[str, Any],
    frame_index: int,
) -> tuple[dict[str, Any], int] | None:
    name = str(clip.get("name", "")).strip()
    if not name:
        return None

    candidates: list[tuple[str, int]] = []
    if name.endswith("_retract.caf"):
        alt_name = f"{name[:-len('_retract.caf')]}_deploy.caf"
        candidates.append((alt_name, 0 if frame_index == -1 else -1))
    if name.endswith("_deploy.caf"):
        alt_name = f"{name[:-len('_deploy.caf')]}_retract.caf"
        candidates.append((alt_name, 0 if frame_index == -1 else -1))
    if name.endswith("_close.caf"):
        alt_name = f"{name[:-len('_close.caf')]}_open.caf"
        candidates.append((alt_name, 0 if frame_index == -1 else -1))
    if name.endswith("_open.caf"):
        alt_name = f"{name[:-len('_open.caf')]}_close.caf"
        candidates.append((alt_name, 0 if frame_index == -1 else -1))

    for alt_name, alt_frame in candidates:
        alt_clip = _find_animation_clip(package, alt_name)
        if alt_clip is not None:
            return alt_clip, alt_frame
    return None


def _object_bone_hash(obj: bpy.types.Object) -> str:
    import zlib

    source_name = str(obj.get(PROP_SOURCE_NODE_NAME, obj.name) or "")
    digest = zlib.crc32(source_name.encode("utf-8")) & 0xFFFFFFFF
    return f"0x{digest:08X}"


def _iter_candidate_bone_objects(package_root: bpy.types.Object) -> list[bpy.types.Object]:
    return [obj for obj in _iter_package_objects(package_root) if obj.type in {"EMPTY", "MESH"}]


def _store_bind_pose_once(obj: bpy.types.Object) -> None:
    if isinstance(obj.get(_ANIMATION_BIND_TRS_PROP), str):
        return
    payload = {
        "location": [float(v) for v in obj.location],
        "rotation_mode": str(obj.rotation_mode),
        "rotation_quaternion": [float(v) for v in obj.rotation_quaternion],
    }
    obj[_ANIMATION_BIND_TRS_PROP] = json.dumps(payload, separators=(",", ":"))


def _restore_bind_pose(package_root: bpy.types.Object) -> int:
    restored = 0
    for obj in _iter_candidate_bone_objects(package_root):
        payload = obj.get(_ANIMATION_BIND_TRS_PROP)
        if not isinstance(payload, str) or not payload:
            continue
        try:
            data = json.loads(payload)
        except json.JSONDecodeError:
            continue
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
        restored += 1
    return restored


def _apply_animation_pose(package_root: bpy.types.Object, clip: dict[str, Any], frame_index: int) -> int:
    bones = clip.get("bones")
    if not isinstance(bones, dict):
        return 0
    updated = 0
    for obj in _iter_candidate_bone_objects(package_root):
        key = _object_bone_hash(obj)
        channel = bones.get(key)
        if not isinstance(channel, dict):
            continue
        _store_bind_pose_once(obj)

        rotations = channel.get("rotation")
        positions = channel.get("position")
        if isinstance(rotations, list) and rotations:
            sample = rotations[0] if frame_index == 0 else rotations[-1]
            if isinstance(sample, list) and len(sample) >= 4:
                obj.rotation_mode = "QUATERNION"
                obj.rotation_quaternion = (
                    float(sample[0]),
                    float(sample[1]),
                    float(sample[2]),
                    float(sample[3]),
                )
        if isinstance(positions, list) and positions:
            sample = positions[0] if frame_index == 0 else positions[-1]
            if isinstance(sample, list) and len(sample) >= 3:
                obj.location = (float(sample[0]), float(sample[1]), float(sample[2]))
        updated += 1
    return updated


def _insert_animation_action(
    _context: bpy.types.Context,
    package_root: bpy.types.Object,
    clip: dict[str, Any],
) -> int:
    bones = clip.get("bones")
    if not isinstance(bones, dict):
        return 0
    name = str(clip.get("name", "animation")) or "animation"
    action_name = f"SB_{package_root.name}_{name}"
    action = bpy.data.actions.get(action_name)
    if action is None:
        action = bpy.data.actions.new(name=action_name)
    else:
        while action.fcurves:
            action.fcurves.remove(action.fcurves[0])

    updated = 0
    for obj in _iter_candidate_bone_objects(package_root):
        key = _object_bone_hash(obj)
        channel = bones.get(key)
        if not isinstance(channel, dict):
            continue
        _store_bind_pose_once(obj)
        obj.rotation_mode = "QUATERNION"
        obj.animation_data_create()
        obj.animation_data.action = action

        rotations = channel.get("rotation") if isinstance(channel.get("rotation"), list) else []
        positions = channel.get("position") if isinstance(channel.get("position"), list) else []

        for index, sample in enumerate(positions):
            if isinstance(sample, list) and len(sample) >= 3:
                obj.location = (float(sample[0]), float(sample[1]), float(sample[2]))
                obj.keyframe_insert(data_path="location", frame=index)

        for index, sample in enumerate(rotations):
            if isinstance(sample, list) and len(sample) >= 4:
                obj.rotation_quaternion = (
                    float(sample[0]),
                    float(sample[1]),
                    float(sample[2]),
                    float(sample[3]),
                )
                obj.keyframe_insert(data_path="rotation_quaternion", frame=index)

        updated += 1

    return updated
