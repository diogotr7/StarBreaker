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
_FRAGMENT_ANIMATION_PREFIX = "fragment:"


def available_package_animation_names(package: PackageBundle) -> list[str]:
    """Return animation names exported on the package root entity."""
    return [name for name, _ in available_package_animation_items(package)]


def available_package_animation_items(package: PackageBundle) -> list[tuple[str, str]]:
    """Return ``(clip_name, display_name)`` pairs for exported animations.

    ``clip_name`` is the canonical sidecar key used for lookups. ``display_name``
    prefers localized metadata when present, then falls back to a shortened path.
    """
    clips = _animation_clips(package)
    preferred_exact_names = {
        str(clip.get("name", "")).strip()
        for clip in clips
        if _is_preferred_package_animation_name(str(clip.get("name", "")).strip())
    }

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
        items.append((clip_name, _animation_display_name(clip)))
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


def _fragment_endpoint_policy(fragment: dict[str, Any] | None, mode: str) -> str | None:
    """Map a Mannequin fragment + snap mode to a transition state policy.

    Each Mannequin transition fragment references a single CryEngine clip and
    plays it either forward (``speed >= 0``) or in reverse (``speed < 0``).
    The clip itself encodes a transition from one steady state ("start") to
    another ("end"). We resolve start/end purely in clip-time:

    * ``start`` = first clip sample (clip-time = 0).
    * ``end`` = the per-channel "other endpoint": the last sample for
      non-cyclic channels, or the mid-clip extreme for cyclic channels
      (those whose first and last samples coincide, e.g. Scorpius front
      landing-gear which is bound in the stowed pose and arcs back to it).

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

    reverse = not _positive_speed_fragment(fragment)
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
) -> int:
    """Apply one animation in one of: none, snap_first, snap_last, action."""
    package = _load_package_from_root(package_root)
    selection = _find_animation_selection(package, animation_name)
    if selection is None:
        raise RuntimeError(f"Animation '{animation_name}' not found in package sidecar")
    clip, fragment = selection
    reverse_playback = _fragment_reverse_playback(fragment)

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
                    fragment=other_fragment,
                    reverse_playback=_fragment_reverse_playback(other_fragment),
                )

    updated = _apply_animation_mode_for_clip(
        context,
        package_root,
        package,
        clip,
        normalized_mode,
        fragment=fragment,
        reverse_playback=reverse_playback,
    )

    mode_map[animation_name] = normalized_mode
    package_root[_ANIMATION_MODES_PROP] = json.dumps(mode_map, separators=(",", ":"), sort_keys=True)
    return updated


def _apply_animation_mode_for_clip(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    package: PackageBundle,
    clip: dict[str, Any],
    mode: str,
    fragment: dict[str, Any] | None = None,
    reverse_playback: bool = False,
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
        target_frame = cyclic_target_frame if normalized_mode == "snap_last" and not reverse_playback else None
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
                paired_policy = _snap_endpoint_policy(str(paired_clip.get("name", "")), normalized_mode)
                paired_cyclic_target_frame = _clip_cyclic_transition_target_frame(paired_clip)
                paired_target_frame = paired_cyclic_target_frame if normalized_mode == "snap_last" else None
                updated = _apply_animation_pose(
                    package_root,
                    paired_clip,
                    paired_frame_index,
                    paired_policy,
                    target_frame=paired_target_frame,
                    anchor_frame=paired_cyclic_target_frame,
                )
        return updated
    if normalized_mode == "action":
        return _insert_animation_action(context, package_root, clip, reverse_playback=reverse_playback)
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


def _clip_cyclic_transition_target_frame(clip: dict[str, Any]) -> float | None:
    if not _clip_has_transition_fragment(clip):
        return None
    position_moving = 0
    position_targets: list[float] = []
    rotation_moving = 0
    rotation_targets: list[float] = []
    for channel in _normalized_bone_channels(clip).values():
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


def _object_bone_hash(obj: bpy.types.Object) -> str:
    import zlib

    source_name = str(obj.get(PROP_SOURCE_NODE_NAME, obj.name) or "")
    digest = zlib.crc32(source_name.encode("utf-8")) & 0xFFFFFFFF
    return f"0x{digest:08X}"


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


def _normalized_bone_channels(clip: dict[str, Any]) -> dict[str, dict[str, Any]]:
    bones = clip.get("bones")
    if not isinstance(bones, dict):
        return {}
    normalized: dict[str, dict[str, Any]] = {}
    for raw_key, channel in bones.items():
        if not isinstance(channel, dict):
            continue
        key = _canonical_bone_hash_key(raw_key)
        if key is None:
            continue
        normalized[key] = channel
    return normalized


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

    def _rotation_score(sample: list[Any]) -> float:
        w = float(sample[0])
        x = float(sample[1])
        y = float(sample[2])
        z = float(sample[3])
        dot = abs(bind_rot[0] * w + bind_rot[1] * x + bind_rot[2] * y + bind_rot[3] * z)
        dot = max(0.0, min(1.0, dot))
        return 2.0 * math.acos(dot)

    def _position_score(sample: list[Any]) -> float:
        decoded = _decode_animation_position(sample, "identity")
        dx = decoded[0] - bind_loc[0]
        dy = decoded[1] - bind_loc[1]
        dz = decoded[2] - bind_loc[2]
        return (dx * dx + dy * dy + dz * dz) ** 0.5

    def _channel_other_endpoint(
        valid: list[list[Any]],
        item_len: int,
        distance: Callable[[list[Any], list[Any]], float],
        threshold: float,
    ) -> list[Any]:
        """Return the channel's "opposite state" sample.

        For a non-cyclic series the last sample is the opposite of the first.
        For a cyclic series (first ≈ last) the opposite state lives mid-clip:
        find the sample most distant from the first/last endpoint.
        """

        if len(valid) < 2:
            return valid[-1] if valid else []
        first = valid[0]
        last = valid[-1]
        if distance(first, last) > threshold:
            return last
        # Cyclic: pick the sample most distant from the shared endpoint.
        return max(valid, key=lambda value: distance(first, value))

    def _select_sample(values: list[Any], item_len: int, scorer: Callable[[list[Any]], float]) -> list[Any] | None:
        valid: list[list[Any]] = [v for v in values if isinstance(v, list) and len(v) >= item_len]
        if not valid:
            return None
        if endpoint_policy == "literal":
            return valid[0] if frame_index == 0 else valid[-1]
        if endpoint_policy == "transition_start":
            return valid[0]
        if endpoint_policy == "transition_end":
            distance = _position_distance if item_len == 3 else _rotation_distance
            threshold = 0.05 if item_len == 3 else 0.03
            return _channel_other_endpoint(valid, item_len, distance, threshold)
        if endpoint_policy == "least_bind_error":
            endpoint_candidates = [valid[0], valid[-1]] if len(valid) > 1 else valid
            scored = [(scorer(v), v) for v in endpoint_candidates]
            return min(scored, key=lambda item: item[0])[1]
        if endpoint_policy == "most_bind_error":
            scored = [(scorer(v), v) for v in valid]
            return max(scored, key=lambda item: item[0])[1]
        return valid[0] if frame_index == 0 else valid[-1]

    rotation_sample: list[Any] | None = None
    if isinstance(rotations, list) and rotations:
        if target_frame is not None:
            rotation_sample = _sample_nearest_time(
                rotations, _channel_times(channel, "rotation_time", len(rotations)), 4, target_frame
            )
        else:
            rotation_sample = _select_sample(rotations, 4, _rotation_score)

    position_sample: list[Any] | None = None
    if isinstance(positions, list) and positions:
        if target_frame is not None:
            position_sample = _sample_nearest_time(
                positions, _channel_times(channel, "position_time", len(positions)), 3, target_frame
            )
        else:
            position_sample = _select_sample(positions, 3, _position_score)

    if rotation_sample is not None:
        obj.rotation_mode = "QUATERNION"
        rot_sample_q = (
            float(rotation_sample[0]),
            float(rotation_sample[1]),
            float(rotation_sample[2]),
            float(rotation_sample[3]),
        )
        blend_mode = str(channel.get("blend_mode") or "").lower()
        if blend_mode == "override":
            # Override mode (Phase 38): the exporter classified this bone's
            # CHR-bind as outside the AABB of all CAF position samples, which
            # means the clip authors meant the channel to *replace* the bind
            # pose rather than ride on top of it. Use the sampled rotation
            # verbatim — no anchor-relative composition.
            obj.rotation_quaternion = rot_sample_q
        elif endpoint_policy in {"transition_start", "transition_end"} and isinstance(rotations, list) and rotations:
            # Anchor-relative composition (matches the position pathway).
            # Clip channels are stored in a coordinate frame that has a fixed
            # offset from the imported rest pose; the offset cancels out by
            # composing `bind ⋅ (anchor⁻¹ ⋅ sample)`. The clip's two channel
            # "states" are valid[0] and the per-channel opposite endpoint
            # (last sample, or rotation-extreme mid-clip sample for cyclic
            # channels). The state nearer to bind acts as the anchor.
            valid_rots: list[list[Any]] = [v for v in rotations if isinstance(v, list) and len(v) >= 4]
            if valid_rots:
                other_rot = _channel_other_endpoint(valid_rots, 4, _rotation_distance, 0.03)
                anchor_candidates = [valid_rots[0], other_rot]
                anchor_rot_list = min(anchor_candidates, key=_rotation_score)
                anchor_q = (
                    float(anchor_rot_list[0]),
                    float(anchor_rot_list[1]),
                    float(anchor_rot_list[2]),
                    float(anchor_rot_list[3]),
                )
                rot_sample_q = _quat_align(anchor_q, rot_sample_q)
                delta = _quat_mul(_quat_conj(anchor_q), rot_sample_q)
                rot_sample_q = _quat_mul(bind_rot, delta)
        obj.rotation_quaternion = rot_sample_q

    if position_sample is not None and isinstance(positions, list) and positions:
        sample_decoded = _decode_animation_position(position_sample, "identity")
        blend_mode = str(channel.get("blend_mode") or "").lower()
        if blend_mode == "override":
            # Override mode (Phase 38): use the sampled position verbatim.
            # The CHR-bind is outside the AABB of CAF samples, so anchor-
            # relative composition would land the bone in the wrong place
            # (canonical example: Scorpius BONE_Front_Landing_Gear_Foot).
            obj.location = sample_decoded
            return
        # Anchor-relative composition (mirrors rotation pathway). Clip
        # positions are in a fixed-offset coordinate frame relative to
        # bind; composing `bind + (sample - anchor)` cancels the offset.
        # Anchor candidates: valid[0] and the per-channel opposite endpoint
        # (last sample, or mid-clip extreme for cyclic position channels).
        # The candidate nearest to bind is the anchor.
        valid_positions: list[list[Any]] = [v for v in positions if isinstance(v, list) and len(v) >= 3]
        if valid_positions:
            if anchor_frame is not None:
                anchor_target = _sample_nearest_time(
                    positions, _channel_times(channel, "position_time", len(positions)), 3, anchor_frame
                )
                anchor_samples = [valid_positions[0], anchor_target or position_sample]
            elif endpoint_policy in {"transition_start", "transition_end"}:
                other = _channel_other_endpoint(
                    valid_positions, 3, _position_distance, 0.05
                )
                anchor_samples = [valid_positions[0], other]
            else:
                anchor_samples = [valid_positions[0], valid_positions[-1]]

            def _dist_sq(sample: list[Any]) -> float:
                decoded = _decode_animation_position(sample, "identity")
                return (
                    (decoded[0] - bind_loc[0]) ** 2
                    + (decoded[1] - bind_loc[1]) ** 2
                    + (decoded[2] - bind_loc[2]) ** 2
                )

            anchor_decoded = _decode_animation_position(min(anchor_samples, key=_dist_sq), "identity")
            obj.location = (
                bind_loc[0] + (sample_decoded[0] - anchor_decoded[0]),
                bind_loc[1] + (sample_decoded[1] - anchor_decoded[1]),
                bind_loc[2] + (sample_decoded[2] - anchor_decoded[2]),
            )
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
    updated = 0
    for obj in _iter_candidate_bone_objects(package_root):
        key = _canonical_bone_hash_key(_object_bone_hash(obj)) or _object_bone_hash(obj)
        channel = bones.get(key)
        if not isinstance(channel, dict):
            continue
        _store_bind_pose_once(obj)
        bind_data = _bind_pose_payload(obj)
        if bind_data is None:
            continue

        _apply_best_channel_transform(
            obj, bind_data, channel, frame_index, endpoint_policy, target_frame, anchor_frame
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
    reverse_playback: bool = False,
) -> int:
    bones = _normalized_bone_channels(clip)
    if not bones:
        return 0
    name = str(clip.get("name", "animation")) or "animation"
    trim_frame = _clip_cyclic_transition_target_frame(clip)

    # Phase 46.2: anchor inserted keyframes at frame 1 by default. Earlier
    # versions used the current scene playhead so multiple action-mode
    # clips could chain naturally on the timeline, but in practice the
    # per-clip NLA strip (added in Phase 46) is the proper UI gesture for
    # time-shifting blocks: the user grabs the strip and drags it. Using
    # the playhead as an implicit anchor surprised users who scrubbed the
    # timeline and then re-applied a clip and saw it land at frame 76 (or
    # wherever they had stopped).
    frame_offset = 1

    updated = 0
    for obj in _iter_candidate_bone_objects(package_root):
        key = _canonical_bone_hash_key(_object_bone_hash(obj)) or _object_bone_hash(obj)
        channel = bones.get(key)
        if not isinstance(channel, dict):
            continue
        _store_bind_pose_once(obj)
        bind_data = _bind_pose_payload(obj)
        if bind_data is None:
            continue
        obj.rotation_mode = "QUATERNION"
        obj.animation_data_create()

        # Phase 24C: each animated object gets its own Action, named after
        # the clip + bone, and grouped by the bone's display name so the
        # Dope Sheet Action editor shows clean per-bone groups. The Action
        # is pushed onto a per-clip NLA track so multiple clips coexist on
        # the timeline without overwriting each other.
        action_name = f"SB_{package_root.name}_{name}_{obj.name}"
        existing = bpy.data.actions.get(action_name)
        if existing is not None:
            bpy.data.actions.remove(existing, do_unlink=True)
        action = bpy.data.actions.new(name=action_name)
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

        def _action_frame(sample_time: float) -> float:
            local_time = duration_frame - sample_time if reverse_playback else sample_time
            return frame_offset + local_time

        if positions:
            bind_location = bind_data.get("location", obj.location)
            bind = (float(bind_location[0]), float(bind_location[1]), float(bind_location[2]))

            first = positions[0] if isinstance(positions[0], list) and len(positions[0]) >= 3 else None
            last = (
                _sample_nearest_time(positions, position_times, 3, trim_frame)
                if trim_frame is not None
                else positions[-1] if isinstance(positions[-1], list) and len(positions[-1]) >= 3 else None
            )
            anchor: tuple[float, float, float] | None = None
            if first is not None and last is not None:
                first_decoded = _decode_animation_position(first, "identity")
                last_decoded = _decode_animation_position(last, "identity")
                first_dist_sq = (
                    (first_decoded[0] - bind[0]) ** 2
                    + (first_decoded[1] - bind[1]) ** 2
                    + (first_decoded[2] - bind[2]) ** 2
                )
                last_dist_sq = (
                    (last_decoded[0] - bind[0]) ** 2
                    + (last_decoded[1] - bind[1]) ** 2
                    + (last_decoded[2] - bind[2]) ** 2
                )
                anchor = first_decoded if first_dist_sq <= last_dist_sq else last_decoded

            if anchor is not None:
                for index, sample in enumerate(positions):
                    sample_time = position_times[index] if index < len(position_times) else float(index)
                    if trim_frame is not None and sample_time > trim_frame:
                        continue
                    if isinstance(sample, list) and len(sample) >= 3:
                        sample_decoded = _decode_animation_position(sample, "identity")
                        obj.location = (
                            bind[0] + (sample_decoded[0] - anchor[0]),
                            bind[1] + (sample_decoded[1] - anchor[1]),
                            bind[2] + (sample_decoded[2] - anchor[2]),
                        )
                        obj.keyframe_insert(data_path="location", frame=_action_frame(sample_time))

        # Phase 47.3: align each rotation sample to the previous *keyed*
        # sample's hemisphere so per-component LINEAR interpolation stays
        # on the short arc. Without this, a sign flip between consecutive
        # source samples (q vs -q — same rotation) makes Blender lerp
        # through ~0 at the midpoint, producing a spurious 180° "inversion"
        # frame between the two keys (observed on Scorpius
        # `landing_gear_extend` BONE_Front_Landing_Gear_Foot frames 37→39).
        prev_keyed_quat: tuple[float, float, float, float] | None = None
        for index, sample in enumerate(rotations):
            sample_time = rotation_times[index] if index < len(rotation_times) else float(index)
            if trim_frame is not None and sample_time > trim_frame:
                continue
            if isinstance(sample, list) and len(sample) >= 4:
                sample_q = (
                    float(sample[0]),
                    float(sample[1]),
                    float(sample[2]),
                    float(sample[3]),
                )
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
            track = anim.nla_tracks.get(name)
            if track is None:
                track = anim.nla_tracks.new()
                track.name = name
            try:
                strip_start = int(action.frame_range[0])
            except Exception:
                strip_start = int(frame_offset)
            strip = None
            try:
                strip = track.strips.new(name=name, start=strip_start, action=action)
                strip.name = name
            except Exception:
                # Strip may already exist at that frame; reuse the
                # most-recently-added strip on this track.
                if track.strips:
                    strip = track.strips[-1]
            # Phase 46: mute the NLA strip so the live action drives
            # playback (no double-evaluation). Apply mute regardless of
            # whether the strip was freshly created or pre-existing from
            # a prior import — Phase 46's first-pass version only set
            # mute on freshly-created strips and left re-imports playing
            # the strip at full strength alongside the live action.
            if strip is not None:
                try:
                    strip.mute = True
                except Exception:
                    pass
            # Phase 46: keep anim.action set so the keyframes are visible
            # in the Dope Sheet / Action Editor for the selected object.
            # The NLA strip above is muted, so there's no double-eval.
            anim.action = action

        updated += 1

    return updated


def _animation_position_to_blender_local(sample: list[Any]) -> tuple[float, float, float]:
    # Backwards-compatible default decoder retained for older call sites.
    return _decode_animation_position(sample, "legacy")


def _decode_animation_position(sample: list[Any], decoder: str) -> tuple[float, float, float]:
    x = float(sample[0])
    y = float(sample[1])
    z = float(sample[2])
    if decoder == "legacy":
        # Legacy export decode: [x, z, -y] -> (x, y, z_blender)
        return (x, -z, y)
    if decoder == "swizzled":
        # Alternate export decode: [cry_y, -cry_z, cry_x] -> (cry_x, -cry_z, cry_y)
        return (z, y, x)
    # "identity": already-authored Blender XYZ.
    return (x, y, z)


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
    candidates = ("legacy", "swizzled", "identity")

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
