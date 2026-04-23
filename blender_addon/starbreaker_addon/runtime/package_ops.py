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
from typing import Any

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
    PROP_SUBMATERIAL_JSON,
)
from .validators import _purge_orphaned_file_backed_images, _purge_orphaned_runtime_groups


def import_package(
    context: bpy.types.Context,
    scene_path: str | Path,
    prefer_cycles: bool = True,
    palette_id: str | None = None,
) -> bpy.types.Object:
    from .importer import PackageImporter

    package = PackageBundle.load(scene_path)
    importer = PackageImporter(context, package)
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
    with _suspend_heavy_viewports(context):
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

    if target_sidecar is None:
        # No paint-variant sidecar for this palette: fast palette-only path.
        return apply_palette_to_package_root(context, package_root, palette_id)

    # Determine which objects are currently exterior so we know what to rebuild.
    # We check against both the original livery sidecars AND any previously-active
    # paint variant sidecar so that consecutive paint switches work correctly.
    effective_exterior = _effective_exterior_material_sidecars(package, package_root)
    base_exterior = _exterior_material_sidecars(package)
    check_sidecars = effective_exterior or base_exterior

    importer = PackageImporter(context, package, package_root=package_root)
    applied = 0
    with _suspend_heavy_viewports(context):
        for obj in _iter_package_objects(package_root):
            if obj.type != "MESH":
                continue
            obj_sidecar = _string_prop(obj, PROP_MATERIAL_SIDECAR)
            if check_sidecars is not None and (obj_sidecar is None or obj_sidecar not in check_sidecars):
                continue
            # Point the object at the new sidecar then rebuild.
            obj[PROP_MATERIAL_SIDECAR] = target_sidecar
            applied += importer.rebuild_object_materials(obj, palette_id)

    # Record the active paint variant sidecar so palette-only changes still work.
    package_root[PROP_PAINT_VARIANT_SIDECAR] = target_sidecar
    package_root[PROP_PALETTE_ID] = palette_id
    _purge_orphaned_runtime_groups()
    _purge_orphaned_file_backed_images()
    return applied


def apply_livery_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, livery_id: str) -> int:
    from .importer import PackageImporter

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
    from ..constants import (
        GLTF_PBR_WATTS_TO_LUMENS,
        LIGHT_CANDELA_TO_WATT,
        LIGHT_VISUAL_GAIN,
    )

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

    intensity_cd = float(state.get("intensity_cd") or 0.0)
    temperature = float(state.get("temperature") or 6500.0)
    use_temperature = bool(state.get("use_temperature"))
    color = state.get("color") or [1.0, 1.0, 1.0]
    if not (isinstance(color, (list, tuple)) and len(color) >= 3):
        color = [1.0, 1.0, 1.0]

    if light.type == "SUN":
        light.energy = intensity_cd / GLTF_PBR_WATTS_TO_LUMENS
    else:
        light.energy = intensity_cd * LIGHT_CANDELA_TO_WATT * LIGHT_VISUAL_GAIN

    if use_temperature:
        # Blender has no built-in Kelvin → RGB for a Light datablock. Keep the
        # authored fallback colour; the exporter writes a RGB triple derived
        # from the temperature when ``useTemperature="1"``.
        pass
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
