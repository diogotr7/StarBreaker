from __future__ import annotations

import json
from pathlib import Path
import time

import bpy
import blf
import gpu
import mathutils
from bpy.app.handlers import persistent
from bpy.props import BoolProperty, EnumProperty, FloatProperty, IntProperty, StringProperty
from bpy.types import Operator, Panel
from bpy_extras.io_utils import ImportHelper
from gpu_extras.batch import batch_for_shader

from .manifest import PackageBundle
from .palette import resolved_palette_id
from .palette import paint_list_canonical_id
from .runtime import (
    DECAL_OFFSET_EXTERNAL_DEFAULT,
    DECAL_OFFSET_INTERNAL_DEFAULT,
    POM_DETAIL_DEFAULT,
    POM_DETAIL_ITEMS,
    PROP_DECAL_OFFSET_EXTERNAL,
    PROP_DECAL_OFFSET_INTERNAL,
    PROP_ENTITY_NAME,
    PROP_MATERIAL_SIDECAR,
    PROP_PACKAGE_NAME,
    PROP_PACKAGE_ROOT,
    PROP_PALETTE_ID,
    PROP_SCENE_PATH,
    PROP_SHADER_FAMILY,
    PROP_SURFACE_SHADER_MODE,
    PROP_TEMPLATE_KEY,
    SCENE_POM_DETAIL_PROP,
    SCENE_ENGINE_GLOW_PROP,
    SCENE_SHARED_GLOW_PROP,
    TEMPLATE_COLLECTION_NAME,
    apply_pom_detail_mode,
    SCENE_WEAR_STRENGTH_PROP,
    animation_overlap_warnings,
    apply_animation_mode_to_package_root,
    apply_decal_offsets_to_package_root,
    apply_engine_glow_to_package_root,
    apply_shared_glow_to_package_root,
    apply_light_state,
    apply_livery_to_selected_package,
    apply_paint_to_selected_package,
    apply_palette_to_selected_package,
    available_package_animation_items,
    package_animation_diagnostics,
    available_light_state_names,
    engine_glow_control_enabled,
    engine_glow_strength,
    shared_glow_control_enabled,
    shared_glow_strength,
    decal_offset_control_enabled,
    dirty_package_material_objects,
    dump_selected_metadata,
    exterior_palette_ids,
    find_package_root,
    import_package,
    delete_animation_instance,
    set_animation_instance_muted,
    solo_animation_instance,
    package_animation_instances,
    package_animation_mode_map,
    refresh_materials_for_package_root,
    resolve_package_scene_path,
    update_animation_instance_start_frame,
)


_PAINT_ITEMS_CACHE: list[tuple[str, str, str]] = []
_PALETTE_ITEMS_CACHE: list[tuple[str, str, str]] = []
_LIVERY_ITEMS_CACHE: list[tuple[str, str, str]] = []
_ANIMATION_MODE_ITEMS: tuple[tuple[str, str, str], ...] = (
    ("none", "None", "Leave current transforms (restore bind pose if available)"),
    ("snap_first", "First", "Apply first keyframe pose"),
    ("snap_last", "Last", "Apply last keyframe pose"),
    (
        "action",
        "Insert",
        "Insert full keyframes as Blender Action, anchored at the current timeline frame",
    ),
)
_ANIMATION_FPS_POLICY_ITEMS: tuple[tuple[str, str, str], ...] = (
    (
        "adapt_scene",
        "Adapt To Scene FPS",
        "Keep scene FPS; scale keyframe times by scene_fps/clip_fps",
    ),
    (
        "match_scene_to_clip",
        "Match Scene To Clip FPS",
        "Set scene FPS to clip FPS before inserting keys",
    ),
)
_SCENE_ANIMATION_FPS_POLICY_PROP = "starbreaker_animation_fps_policy"
_IMPORT_PROGRESS_ACTIVE_PROP = "starbreaker_import_progress_active"
_IMPORT_PROGRESS_VALUE_PROP = "starbreaker_import_progress_value"
_IMPORT_PROGRESS_DESCRIPTION_PROP = "starbreaker_import_progress_description"
_IMPORT_PROGRESS_INDETERMINATE_PROP = "starbreaker_import_progress_indeterminate"
_IMPORT_PROGRESS_TEXT_ONLY_PROP = "starbreaker_import_progress_text_only"
_IMPORT_PROGRESS_LAST_UPDATE = 0.0
_IMPORT_PROGRESS_DRAW_HANDLER = None
_AUTO_MATERIAL_REFRESH_TOKEN = 0
_CONTROL_PROP_SYNCING = False


def _progress_fraction(value: float) -> float:
    return max(0.0, min(1.0, float(value)))


def _tag_view3d_redraws(context: bpy.types.Context) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if screen is None:
        return
    for area in screen.areas:
        if area.type == "VIEW_3D":
            area.tag_redraw()


def _force_view3d_perspective(context: bpy.types.Context) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if window is None or screen is None:
        return
    for area in screen.areas:
        if area.type != "VIEW_3D":
            continue
        region = next((region for region in area.regions if region.type == "WINDOW"), None)
        if region is None:
            continue
        space = getattr(area, "spaces", None)
        active_space = space.active if space is not None else None
        region_3d = getattr(active_space, "region_3d", None)
        if region_3d is None:
            continue
        with context.temp_override(window=window, area=area, region=region):
            region_3d.view_perspective = "PERSP"


def _set_active_object(context: bpy.types.Context, obj: bpy.types.Object | None) -> None:
    view_layer = getattr(context, "view_layer", None)
    if view_layer is not None and obj is not None:
        view_layer.objects.active = obj


def _object_parent_depth(obj: object) -> int:
    depth = 0
    parent = getattr(obj, "parent", None)
    while parent is not None:
        depth += 1
        parent = getattr(parent, "parent", None)
    return depth


def _loaded_package_roots(
    objects: object,
    *,
    max_depth: int = 2,
) -> list[bpy.types.Object]:
    roots: list[bpy.types.Object] = []
    for obj in objects:
        get_prop = getattr(obj, "get", None)
        if not callable(get_prop) or not bool(get_prop(PROP_PACKAGE_ROOT)):
            continue
        if _object_parent_depth(obj) > max_depth:
            continue
        roots.append(obj)
    roots.sort(key=lambda candidate: (_object_parent_depth(candidate), getattr(candidate, "name", "")))
    return roots


def _open_view3d_sidebar(context: bpy.types.Context) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if screen is None:
        return
    for area in screen.areas:
        if area.type != "VIEW_3D":
            continue
        spaces = getattr(area, "spaces", None)
        active_space = spaces.active if spaces is not None else None
        if active_space is not None and hasattr(active_space, "show_region_ui"):
            active_space.show_region_ui = True
        ui_region = next((region for region in area.regions if region.type == "UI"), None)
        if ui_region is not None and hasattr(ui_region, "active_panel_category"):
            try:
                ui_region.active_panel_category = "StarBreaker"
            except Exception:
                pass
        area.tag_redraw()


def _focus_loaded_package_root(context: bpy.types.Context, package_root: bpy.types.Object) -> None:
    for obj in getattr(context, "selected_objects", []):
        try:
            obj.select_set(False)
        except Exception:
            pass
    try:
        package_root.select_set(True)
    except Exception:
        pass
    _set_active_object(context, package_root)
    _sync_scene_package_controls(context, package_root)
    _open_view3d_sidebar(context)
    _tag_view3d_redraws(context)


def _view3d_sidebar_categories(context: bpy.types.Context) -> list[str | None]:
    categories: list[str | None] = []
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if screen is None:
        return categories
    for area in screen.areas:
        if area.type != "VIEW_3D":
            continue
        ui_region = next((region for region in area.regions if region.type == "UI"), None)
        categories.append(getattr(ui_region, "active_panel_category", None) if ui_region is not None else None)
    return categories


def _deferred_focus_loaded_package_root(
    package_root_name: str,
    retries_remaining: int = 5,
) -> None:
    package_root = bpy.data.objects.get(package_root_name)
    if package_root is None:
        return None
    get_prop = getattr(package_root, "get", None)
    if not callable(get_prop) or not bool(get_prop(PROP_PACKAGE_ROOT)):
        return None
    context = bpy.context
    _focus_loaded_package_root(context, package_root)
    categories = _view3d_sidebar_categories(context)
    if categories and any(category != "StarBreaker" for category in categories):
        if retries_remaining > 0:
            try:
                bpy.app.timers.register(
                    lambda root_name=package_root_name, retries=retries_remaining - 1: _deferred_focus_loaded_package_root(
                        root_name,
                        retries,
                    ),
                    first_interval=0.5,
                    persistent=False,
                )
            except Exception:
                pass
    return None


def _collapse_template_cache_outliner(context: bpy.types.Context) -> None:
    if bpy.data.collections.get(TEMPLATE_COLLECTION_NAME) is None:
        return
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if window is None or screen is None:
        return
    for area in screen.areas:
        if area.type != "OUTLINER":
            continue
        region = next((region for region in area.regions if region.type == "WINDOW"), None)
        if region is None:
            continue
        with context.temp_override(window=window, area=area, region=region):
            for _ in range(8):
                try:
                    bpy.ops.outliner.show_one_level(open=False)
                except RuntimeError:
                    break


def _frame_package_root_three_quarter(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
) -> None:
    window = getattr(context, "window", None)
    screen = getattr(window, "screen", None)
    if window is None or screen is None:
        return

    mesh_candidates: list[tuple[bpy.types.Object, float]] = []
    for obj in package_root.children_recursive:
        if obj.type != "MESH":
            continue
        if not bool(getattr(getattr(obj, "data", None), "polygons", ())):
            continue
        if bool(getattr(obj, "hide_viewport", False)):
            continue
        if not obj.visible_get(view_layer=context.view_layer):
            continue
        corners = [obj.matrix_world @ mathutils.Vector(corner) for corner in obj.bound_box]
        if not corners:
            continue
        min_corner = mathutils.Vector(
            (
                min(corner.x for corner in corners),
                min(corner.y for corner in corners),
                min(corner.z for corner in corners),
            )
        )
        max_corner = mathutils.Vector(
            (
                max(corner.x for corner in corners),
                max(corner.y for corner in corners),
                max(corner.z for corner in corners),
            )
        )
        diagonal = (max_corner - min_corner).length
        if diagonal > 0.0:
            mesh_candidates.append((obj, diagonal))

    if not mesh_candidates:
        return

    largest_diagonal = max(diagonal for _, diagonal in mesh_candidates)
    cutoff = largest_diagonal * 0.08
    focus_objects = [obj for obj, diagonal in mesh_candidates if diagonal >= cutoff]
    if not focus_objects:
        focus_objects = [obj for obj, _ in mesh_candidates]

    world_corners: list[mathutils.Vector] = []
    for obj in focus_objects:
        bound_box = getattr(obj, "bound_box", None)
        if not bound_box:
            continue
        matrix_world = obj.matrix_world
        for corner in bound_box:
            world_corners.append(matrix_world @ mathutils.Vector(corner))
    if not world_corners:
        return

    sorted_x = sorted(corner.x for corner in world_corners)
    sorted_y = sorted(corner.y for corner in world_corners)
    sorted_z = sorted(corner.z for corner in world_corners)
    corner_count = len(world_corners)
    low_index = max(0, int(corner_count * 0.02))
    high_index = min(corner_count - 1, int(corner_count * 0.98))

    min_corner = mathutils.Vector(
        (
            sorted_x[low_index],
            sorted_y[low_index],
            sorted_z[low_index],
        )
    )
    max_corner = mathutils.Vector(
        (
            sorted_x[high_index],
            sorted_y[high_index],
            sorted_z[high_index],
        )
    )
    focus_center = (min_corner + max_corner) * 0.5
    diagonal = (max_corner - min_corner).length
    if diagonal <= 1e-6:
        diagonal = 1.0

    # Baseline direction tuned from an approved manual viewport angle.
    view_direction = mathutils.Vector((-0.735, -0.487, -0.650)).normalized()
    target_rotation = view_direction.to_track_quat("-Z", "Y")

    prioritized_areas: list[bpy.types.Area] = []
    active_area = getattr(context, "area", None)
    if active_area is not None and active_area.type == "VIEW_3D":
        prioritized_areas.append(active_area)
    prioritized_areas.extend(
        area for area in screen.areas if area.type == "VIEW_3D" and area is not active_area
    )

    for area in prioritized_areas:
        region = next((region for region in area.regions if region.type == "WINDOW"), None)
        if region is None:
            continue
        with context.temp_override(window=window, area=area, region=region):
            bpy.ops.object.select_all(action="DESELECT")
            for obj in focus_objects:
                obj.select_set(True)
            _set_active_object(context, focus_objects[0])
            space = getattr(area, "spaces", None)
            active_space = space.active if space is not None else None
            region_3d = getattr(active_space, "region_3d", None)
            if region_3d is not None:
                region_3d.view_perspective = "PERSP"
                for _ in range(3):
                    if region_3d.view_perspective == "PERSP":
                        break
                    try:
                        bpy.ops.view3d.view_persportho()
                    except RuntimeError:
                        break
                    region_3d.view_perspective = "PERSP"
                region_3d.view_rotation = target_rotation
                region_3d.view_location = focus_center

                lens_mm = float(getattr(active_space, "lens", 50.0))
                lens_scale = max(lens_mm, 1.0) / 50.0
                # Deterministic fit: scales with lens so 100mm does not over-zoom.
                region_3d.view_distance = max(diagonal * 0.95 * lens_scale, diagonal * 0.78)


def _finalize_import_view(context: bpy.types.Context, package_root: bpy.types.Object) -> None:
    _collapse_template_cache_outliner(context)
    _frame_package_root_three_quarter(context, package_root)
    _force_view3d_perspective(context)
    bpy.ops.object.select_all(action="DESELECT")
    package_root.select_set(True)
    _set_active_object(context, package_root)


def _auto_apply_landing_gear_snap_last(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
) -> None:
    scene_path = package_root.get(PROP_SCENE_PATH)
    if not isinstance(scene_path, str) or not scene_path:
        return

    try:
        package = PackageBundle.load(scene_path)
    except Exception:
        return

    try:
        animation_items = available_package_animation_items(package)
    except Exception:
        return
    if not animation_items:
        return

    def _animation_score(name: str, display_name: str) -> int:
        text = f"{name} {display_name}".lower()
        has_landing_gear = "landing" in text and "gear" in text
        if not has_landing_gear:
            return -1

        score = 0
        if "retract" in text:
            score += 100
        if "stow" in text or "stowed" in text:
            score += 80
        if "close" in text or "closed" in text:
            score += 60
        if "up" in text:
            score += 40

        if "deploy" in text or "deployed" in text:
            score -= 60
        if "extend" in text or "extended" in text:
            score -= 60
        if "open" in text or "opened" in text:
            score -= 40
        if "down" in text:
            score -= 20
        return score

    best_name: str | None = None
    best_score = -1
    for animation_name, animation_display_name in animation_items:
        score = _animation_score(animation_name, animation_display_name)
        if score > best_score:
            best_score = score
            best_name = animation_name

    if best_name is None or best_score < 0:
        return

    try:
        apply_animation_mode_to_package_root(context, package_root, best_name, "snap_last")
    except Exception:
        return


def _update_pom_detail(_: bpy.types.ID, context: bpy.types.Context) -> None:
    scene = getattr(context, "scene", None)
    if scene is None:
        return
    try:
        apply_pom_detail_mode(getattr(scene, SCENE_POM_DETAIL_PROP, POM_DETAIL_DEFAULT))
    except Exception:
        return
    _tag_view3d_redraws(context)


def _sync_scene_package_controls(context: bpy.types.Context, package_root: bpy.types.Object) -> None:
    scene = getattr(context, "scene", None)
    if scene is None:
        return
    updates: list[tuple[str, float]] = []
    if engine_glow_control_enabled(package_root):
        updates.append((SCENE_ENGINE_GLOW_PROP, engine_glow_strength(package_root)))
    if shared_glow_control_enabled(package_root):
        updates.append((SCENE_SHARED_GLOW_PROP, shared_glow_strength(package_root)))
    if not updates:
        return

    pending: list[tuple[str, float]] = []
    for prop_name, value in updates:
        try:
            current = float(getattr(scene, prop_name))
        except Exception:
            current = None
        if current is not None and abs(current - float(value)) <= 1.0e-6:
            continue
        pending.append((prop_name, float(value)))
    if not pending:
        return

    def _apply() -> None:
        global _CONTROL_PROP_SYNCING
        _CONTROL_PROP_SYNCING = True
        try:
            for prop_name, value in pending:
                try:
                    setattr(scene, prop_name, value)
                except Exception:
                    continue
        finally:
            _CONTROL_PROP_SYNCING = False
        return None

    try:
        bpy.app.timers.register(_apply, first_interval=0.0, persistent=False)
    except Exception:
        pass


def _update_engine_glow(_: bpy.types.ID, context: bpy.types.Context) -> None:
    if _CONTROL_PROP_SYNCING:
        return
    scene = getattr(context, "scene", None)
    if scene is None:
        return
    package_root = _package_root_from_context(context)
    if package_root is None or not engine_glow_control_enabled(package_root):
        return
    try:
        apply_engine_glow_to_package_root(package_root, float(getattr(scene, SCENE_ENGINE_GLOW_PROP, 3.0)))
    except Exception:
        return
    _tag_view3d_redraws(context)


def _update_shared_glow(_: bpy.types.ID, context: bpy.types.Context) -> None:
    if _CONTROL_PROP_SYNCING:
        return
    scene = getattr(context, "scene", None)
    if scene is None:
        return
    package_root = _package_root_from_context(context)
    if package_root is None or not shared_glow_control_enabled(package_root):
        return
    try:
        apply_shared_glow_to_package_root(package_root, float(getattr(scene, SCENE_SHARED_GLOW_PROP, 0.0)))
    except Exception:
        return
    _tag_view3d_redraws(context)


def _update_decal_offset(root: bpy.types.ID, context: bpy.types.Context) -> None:
    if not bool(getattr(root, "get", lambda _key: False)(PROP_PACKAGE_ROOT)):
        return
    try:
        apply_decal_offsets_to_package_root(
            root,
            float(getattr(root, PROP_DECAL_OFFSET_EXTERNAL, DECAL_OFFSET_EXTERNAL_DEFAULT)),
            float(getattr(root, PROP_DECAL_OFFSET_INTERNAL, DECAL_OFFSET_INTERNAL_DEFAULT)),
        )
    except Exception:
        return
    _tag_view3d_redraws(context)


def _draw_import_progress_overlay() -> None:
    context = bpy.context
    region = getattr(context, "region", None)
    if region is None:
        return
    window_manager = context.window_manager
    if not bool(getattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False)):
        return

    fraction = _progress_fraction(getattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0))
    description = getattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, "Preparing import")
    indeterminate = bool(getattr(window_manager, _IMPORT_PROGRESS_INDETERMINATE_PROP, False))
    text_only = bool(getattr(window_manager, _IMPORT_PROGRESS_TEXT_ONLY_PROP, False))

    panel_width = min(480.0, max(region.width - 80.0, 320.0))
    panel_height = 64.0 if text_only else 96.0
    panel_x = (region.width - panel_width) * 0.5
    panel_y = region.height * 0.12
    padding = 16.0
    bar_height = 24.0
    bar_width = panel_width - (padding * 2.0) - 72.0
    bar_x = panel_x + padding
    bar_y = panel_y + panel_height - padding - bar_height - 10.0

    shader = gpu.shader.from_builtin("UNIFORM_COLOR")

    def draw_rect(x: float, y: float, width: float, height: float, color: tuple[float, float, float, float]) -> None:
        vertices = ((x, y), (x + width, y), (x + width, y + height), (x, y + height))
        indices = ((0, 1, 2), (0, 2, 3))
        batch = batch_for_shader(shader, "TRIS", {"pos": vertices}, indices=indices)
        shader.bind()
        shader.uniform_float("color", color)
        batch.draw(shader)

    gpu.state.blend_set("ALPHA")
    draw_rect(panel_x, panel_y, panel_width, panel_height, (0.05, 0.07, 0.09, 0.88))
    draw_rect(panel_x + 1.0, panel_y + 1.0, panel_width - 2.0, panel_height - 2.0, (0.10, 0.12, 0.15, 0.92))
    if not text_only:
        draw_rect(bar_x, bar_y, bar_width, bar_height, (0.18, 0.21, 0.25, 1.0))
        if not indeterminate and fraction > 0.0:
            draw_rect(bar_x, bar_y, bar_width * fraction, bar_height, (0.23, 0.62, 0.86, 1.0))
    gpu.state.blend_set("NONE")

    font_id = 0
    blf.color(font_id, 0.96, 0.97, 0.98, 1.0)
    if not text_only:
        try:
            blf.size(font_id, 14.0)
        except TypeError:
            blf.size(font_id, 14, 72)
        blf.position(font_id, bar_x + bar_width + 16.0, bar_y + 4.0, 0)
        blf.draw(font_id, "..." if indeterminate else f"{int(round(fraction * 100.0))}%")

    try:
        blf.size(font_id, 13.0)
    except TypeError:
        blf.size(font_id, 13, 72)
    description_y = panel_y + padding if not text_only else panel_y + ((panel_height - 13.0) * 0.5)
    blf.position(font_id, bar_x, description_y, 0)
    blf.draw(font_id, description)


def _ensure_import_progress_overlay() -> None:
    global _IMPORT_PROGRESS_DRAW_HANDLER
    if _IMPORT_PROGRESS_DRAW_HANDLER is not None:
        return
    _IMPORT_PROGRESS_DRAW_HANDLER = bpy.types.SpaceView3D.draw_handler_add(
        _draw_import_progress_overlay,
        (),
        "WINDOW",
        "POST_PIXEL",
    )


def _remove_import_progress_overlay() -> None:
    global _IMPORT_PROGRESS_DRAW_HANDLER
    if _IMPORT_PROGRESS_DRAW_HANDLER is None:
        return
    bpy.types.SpaceView3D.draw_handler_remove(_IMPORT_PROGRESS_DRAW_HANDLER, "WINDOW")
    _IMPORT_PROGRESS_DRAW_HANDLER = None


def _begin_import_progress(
    context: bpy.types.Context,
    description: str,
    *,
    indeterminate: bool = False,
    text_only: bool = False,
) -> None:
    global _IMPORT_PROGRESS_LAST_UPDATE
    window_manager = context.window_manager
    setattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, True)
    setattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0)
    setattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, description)
    setattr(window_manager, _IMPORT_PROGRESS_INDETERMINATE_PROP, indeterminate)
    setattr(window_manager, _IMPORT_PROGRESS_TEXT_ONLY_PROP, text_only)
    _IMPORT_PROGRESS_LAST_UPDATE = 0.0
    _ensure_import_progress_overlay()
    _tag_view3d_redraws(context)
    try:
        window_manager.progress_begin(0, 100)
    except Exception:
        pass


def _update_import_progress(
    context: bpy.types.Context,
    fraction: float,
    description: str,
    *,
    force: bool = False,
    redraw: bool = True,
) -> None:
    global _IMPORT_PROGRESS_LAST_UPDATE
    now = time.monotonic()
    if not force and now - _IMPORT_PROGRESS_LAST_UPDATE < 0.5:
        return
    window_manager = context.window_manager
    clamped = _progress_fraction(fraction)
    setattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, clamped)
    setattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, description)
    setattr(window_manager, _IMPORT_PROGRESS_INDETERMINATE_PROP, False)
    _IMPORT_PROGRESS_LAST_UPDATE = now
    try:
        window_manager.progress_update(int(round(clamped * 100.0)))
    except Exception:
        pass
    if redraw:
        try:
            bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=1)
        except Exception:
            pass
    _tag_view3d_redraws(context)


def _end_import_progress(context: bpy.types.Context, description: str) -> None:
    window_manager = context.window_manager
    _update_import_progress(context, 1.0, description, force=True)
    setattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False)
    setattr(window_manager, _IMPORT_PROGRESS_INDETERMINATE_PROP, False)
    setattr(window_manager, _IMPORT_PROGRESS_TEXT_ONLY_PROP, False)
    _tag_view3d_redraws(context)
    try:
        window_manager.progress_end()
    except Exception:
        pass


def _init_instance_start_props(
    context: bpy.types.Context,
    package_root: bpy.types.Object,  # noqa: ARG001 — kept for future use
    instances: list[dict],
) -> None:
    """Initialise inline-frame scene props for all tracked animation instances.

    Must be called from an operator execute context, never from a draw callback.
    """
    for instance in instances:
        instance_id = str(instance.get("id", ""))
        if not instance_id:
            continue
        start = int(round(float(instance.get("start_frame", 1.0))))
        key = f"starbreaker_anim_start_{instance_id}"
        if context.scene.get(key) is None:
            context.scene[key] = start


def _schedule_init_scene_prop(key: str, value: int) -> None:
    """Defer a scene custom-property write to outside the draw callback.

    Writing to ID classes during a draw call raises a Blender context error.
    Using a zero-interval timer pushes the write to the next event loop tick.
    """
    def _init() -> None:
        try:
            scene = bpy.context.scene
            if key not in scene:
                scene[key] = value
        except Exception:
            pass
        return None

    try:
        bpy.app.timers.register(_init, first_interval=0.0, persistent=False)
    except Exception:
        pass


def _package_root_from_context(context: bpy.types.Context) -> bpy.types.Object | None:
    package_root = find_package_root(context.active_object)
    if package_root is not None:
        return package_root
    for obj in context.selected_objects:
        package_root = find_package_root(obj)
        if package_root is not None:
            return package_root
    return None


def _collection_instance_objects(package_root: bpy.types.Object) -> list[bpy.types.Object]:
    candidates = [package_root, *list(getattr(package_root, "children_recursive", ()))]
    return [
        obj
        for obj in candidates
        if getattr(obj, "instance_collection", None) is not None
    ]


def _linked_object_data_objects(package_root: bpy.types.Object) -> list[bpy.types.Object]:
    candidates = [package_root, *list(getattr(package_root, "children_recursive", ()))]
    return [
        obj
        for obj in candidates
        if getattr(getattr(obj, "data", None), "library", None) is not None
    ]


def _make_package_collection_instances_real(
    context: bpy.types.Context,
    package_root: bpy.types.Object,
    *,
    chunk_size: int = 100,
) -> int:
    total_processed = 0
    while True:
        current_instances = _collection_instance_objects(package_root)
        if not current_instances:
            return total_processed

        batch_size = len(current_instances)
        for chunk_start in range(0, batch_size, chunk_size):
            chunk = current_instances[chunk_start : chunk_start + chunk_size]
            if not chunk:
                continue
            bpy.ops.object.select_all(action="DESELECT")
            first_instance = None
            for obj in chunk:
                obj.select_set(True)
                if first_instance is None:
                    first_instance = obj
            if first_instance is None:
                continue

            view_layer = getattr(context, "view_layer", None)
            objects = getattr(view_layer, "objects", None)
            if objects is not None:
                objects.active = first_instance

            bpy.ops.object.duplicates_make_real(use_base_parent=True, use_hierarchy=True)
            for obj in chunk:
                obj.instance_type = "NONE"
                obj.instance_collection = None

        total_processed += batch_size


def _make_package_linked_object_data_local(package_root: bpy.types.Object) -> int:
    localized_data: dict[int, bpy.types.ID] = {}
    localized_count = 0
    for obj in _linked_object_data_objects(package_root):
        data = getattr(obj, "data", None)
        if data is None:
            continue
        pointer = data.as_pointer() if hasattr(data, "as_pointer") else id(data)
        local_data = localized_data.get(pointer)
        if local_data is None:
            local_data = data.copy()
            localized_data[pointer] = local_data
            localized_count += 1
        obj.data = local_data
    return localized_count


def _prune_empty_mesh_material_slots(package_root: bpy.types.Object) -> int:
    """Detach material slots from geometry-less meshes under ``package_root``.

    A mesh with zero polygons (e.g. a seat-access interaction proxy the
    exporter writes with no geometry) shades nothing, yet it still carries a
    placeholder material with no node tree. Blender's FBX exporter wraps every
    material slot on a selected object and crashes on ``node_tree is None``.
    Detaching the material is a structural cleanup keyed on geometry (polygon
    count), not on any asset name, so it generalises to every package.
    """
    cleared = 0
    candidates = [package_root, *list(getattr(package_root, "children_recursive", ()))]
    for obj in candidates:
        if getattr(obj, "type", None) != "MESH":
            continue
        data = getattr(obj, "data", None)
        polygons = getattr(data, "polygons", None) if data is not None else None
        if polygons is None or len(polygons) != 0:
            continue
        for slot in getattr(obj, "material_slots", ()):
            if getattr(slot, "material", None) is not None:
                slot.material = None
                cleared += 1
    return cleared


def _detach_unresolved_empty_materials(package_root: bpy.types.Object) -> list[str]:
    """Last-resort guard: detach any remaining mesh-slot material that has no
    shader node tree, returning ``"<object> / <material>"`` descriptions.

    The material refresh and :func:`_prune_empty_mesh_material_slots` resolve
    the two known categories of node-tree-less materials. Anything that still
    reaches FBX export with ``node_tree is None`` would crash Blender's FBX
    exporter (it wraps every material slot). This guard detaches such a slot so
    the export stays usable, and the caller reports a WARNING naming each one.
    A non-empty result is NOT routine: it means a material the refresh should
    have rebuilt did not resolve, and the named materials are the lead to that
    root cause — this guard keeps the export working, it does not excuse the gap.
    """
    detached: list[str] = []
    candidates = [package_root, *list(getattr(package_root, "children_recursive", ()))]
    for obj in candidates:
        if getattr(obj, "type", None) != "MESH":
            continue
        for slot in getattr(obj, "material_slots", ()):
            material = getattr(slot, "material", None)
            if material is None:
                continue
            if getattr(material, "node_tree", None) is None:
                detached.append(f"{obj.name} / {getattr(material, 'name', '?')}")
                slot.material = None
    return detached


def _selected_package(context: bpy.types.Context) -> PackageBundle | None:
    package_root = _package_root_from_context(context)
    if package_root is None:
        return None
    try:
        return PackageBundle.load(str(resolve_package_scene_path(package_root)))
    except Exception:
        return None


def _humanize_identifier(value: str) -> str:
    parts = [part for part in value.replace("-", "_").split("_") if part]
    words: list[str] = []
    for part in parts:
        lowered = part.lower()
        if lowered == "mk2":
            words.append("Mk2")
        elif lowered == "rsi":
            words.append("RSI")
        else:
            words.append(part.capitalize())
    return " ".join(words) if words else value


def _palette_display_name(palette_id: str, source_name: str | None, display_name: str | None) -> str:
    display_value = (display_name or "").strip()
    if display_value:
        return display_value
    source_key = (source_name or "").strip()
    if source_key:
        return _humanize_identifier(source_key)
    return _humanize_identifier(palette_id.split("/", 1)[-1])


def _paint_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    global _PAINT_ITEMS_CACHE
    package = _selected_package(context)
    if package is None:
        _PAINT_ITEMS_CACHE = [("", "No imported package", "Import a StarBreaker package first")]
        return _PAINT_ITEMS_CACHE
    available_ids = exterior_palette_ids(package)
    deduped_items: dict[str, tuple[str, str, str]] = {}
    for palette_id in available_ids:
        paint_variant = package.paints.get(palette_id)
        palette_entry = package.palettes.get(palette_id)
        if paint_variant is None and palette_entry is None:
            continue
        source_name = (
            (paint_variant.display_name if paint_variant else None)
            or (palette_entry.source_name if palette_entry else None)
            or palette_id
        )
        display_name_str = (
            (paint_variant.display_name if paint_variant else None)
            or (palette_entry.display_name if palette_entry else None)
        )
        description = (
            (paint_variant.subgeometry_tag if paint_variant else None)
            or source_name
        )
        item = (
            palette_id,
            _palette_display_name(palette_id, source_name, display_name_str),
            description,
        )
        canonical_id = paint_list_canonical_id(package, palette_id) or palette_id
        existing = deduped_items.get(canonical_id)
        if existing is not None and paint_variant is None:
            continue
        deduped_items[canonical_id] = item
    items = sorted(deduped_items.values(), key=lambda item: item[1])
    _PAINT_ITEMS_CACHE = items
    return _PAINT_ITEMS_CACHE


def _palette_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    global _PALETTE_ITEMS_CACHE
    package = _selected_package(context)
    if package is None:
        _PALETTE_ITEMS_CACHE = [("", "No imported package", "Import a StarBreaker package first")]
        return _PALETTE_ITEMS_CACHE
    _PALETTE_ITEMS_CACHE = [
        (
            palette_id,
            _palette_display_name(
                palette_id,
                package.palettes[palette_id].source_name,
                package.palettes[palette_id].display_name,
            ),
            package.palettes[palette_id].source_name or palette_id,
        )
        for palette_id in sorted(package.palettes.keys())
    ]
    return _PALETTE_ITEMS_CACHE


def _first_valid_item_id(items: list[tuple[str, str, str]]) -> str:
    for item_id, _, _ in items:
        if item_id:
            return item_id
    return ""


def _livery_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    global _LIVERY_ITEMS_CACHE
    package = _selected_package(context)
    if package is None:
        _LIVERY_ITEMS_CACHE = [("", "No imported package", "Import a StarBreaker package first")]
        return _LIVERY_ITEMS_CACHE
    _LIVERY_ITEMS_CACHE = [
        (livery_id, livery_id, package.liveries[livery_id].palette_source_name or livery_id)
        for livery_id in sorted(package.liveries.keys())
    ]
    return _LIVERY_ITEMS_CACHE


class STARBREAKER_OT_import_decomposed_package(Operator, ImportHelper):
    bl_idname = "starbreaker.import_decomposed_package"
    bl_label = "Import StarBreaker Package"
    bl_options = {"REGISTER", "UNDO"}

    _timer: bpy.types.Timer | None = None
    _started: bool = False

    filter_glob: StringProperty(default="scene.json;*.json", options={"HIDDEN"})
    prefer_cycles: BoolProperty(
        name="Prefer Cycles",
        description="Switch the active scene to Cycles before import",
        default=True,
    )
    palette_id_override: StringProperty(
        name="Initial Palette ID",
        description="Optional palette override applied during import to avoid rebuilding the package a second time",
        default="",
    )

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_name = Path(self.filepath).parent.name
        _begin_import_progress(context, f"Preparing {package_name}")
        self._started = False
        self._timer = context.window_manager.event_timer_add(0.01, window=context.window)
        context.window_manager.modal_handler_add(self)
        return {"RUNNING_MODAL"}

    def modal(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if event.type != "TIMER" or self._started:
            return {"PASS_THROUGH"}
        self._started = True
        package_name = Path(self.filepath).parent.name
        try:
            package_root = import_package(
                context,
                self.filepath,
                prefer_cycles=self.prefer_cycles,
                palette_id=self.palette_id_override.strip() or None,
                progress_callback=lambda fraction, description: _update_import_progress(
                    context,
                    fraction,
                    description,
                ),
            )
        except Exception as exc:
            _end_import_progress(context, f"Import failed: {exc}")
            self.cancel(context)
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        prefs = _get_prefs()
        if _should_apply_landing_gear(prefs):
            _auto_apply_landing_gear_snap_last(context, package_root)
        if _should_change_viewport(prefs):
            _finalize_import_view(context, package_root)
        _end_import_progress(context, f"Imported {package_root.get(PROP_PACKAGE_NAME, package_name)}")
        self.cancel(context)
        self.report({"INFO"}, f"Imported {package_root.get(PROP_PACKAGE_NAME, package_name)}")
        return {"FINISHED"}

    def cancel(self, context: bpy.types.Context) -> None:
        if self._timer is not None:
            context.window_manager.event_timer_remove(self._timer)
            self._timer = None


class STARBREAKER_OT_import_progress_popup(Operator):
    bl_idname = "starbreaker.import_progress_popup"
    bl_label = "StarBreaker Import Progress"
    bl_options = {"INTERNAL"}

    _timer: bpy.types.Timer | None = None
    _started: bool = False

    filepath: StringProperty(options={"HIDDEN"})
    package_name: StringProperty(options={"HIDDEN"})
    prefer_cycles: BoolProperty(options={"HIDDEN"}, default=True)
    palette_id_override: StringProperty(options={"HIDDEN"}, default="")

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        _begin_import_progress(context, f"Preparing {self.package_name or Path(self.filepath).parent.name}")
        self._started = False
        self._timer = context.window_manager.event_timer_add(0.01, window=context.window)
        context.window_manager.modal_handler_add(self)
        return context.window_manager.invoke_popup(self, width=420)

    def modal(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if event.type == "TIMER":
            if not self._started:
                self._started = True
                try:
                    package_root = import_package(
                        context,
                        self.filepath,
                        prefer_cycles=self.prefer_cycles,
                        palette_id=self.palette_id_override.strip() or None,
                        progress_callback=lambda fraction, description: _update_import_progress(
                            context,
                            fraction,
                            description,
                        ),
                    )
                except Exception as exc:
                    _end_import_progress(context, f"Import failed: {exc}")
                    self.cancel(context)
                    self.report({"ERROR"}, str(exc))
                    return {"CANCELLED"}
                prefs = _get_prefs()
                if _should_apply_landing_gear(prefs):
                    _auto_apply_landing_gear_snap_last(context, package_root)
                if _should_change_viewport(prefs):
                    _finalize_import_view(context, package_root)
                _end_import_progress(
                    context,
                    f"Imported {package_root.get(PROP_PACKAGE_NAME, self.package_name or Path(self.filepath).parent.name)}",
                )
                self.cancel(context)
                self.report(
                    {"INFO"},
                    f"Imported {package_root.get(PROP_PACKAGE_NAME, self.package_name or Path(self.filepath).parent.name)}",
                )
                return {"FINISHED"}
            if not getattr(context.window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False):
                self.cancel(context)
                return {"FINISHED"}
            if context.window.screen is not None:
                for area in context.window.screen.areas:
                    area.tag_redraw()
        return {"PASS_THROUGH"}

    def cancel(self, context: bpy.types.Context) -> None:
        if self._timer is not None:
            context.window_manager.event_timer_remove(self._timer)
            self._timer = None

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout
        window_manager = context.window_manager
        fraction = _progress_fraction(getattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0))
        description = getattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, "Preparing import")
        text_only = bool(getattr(window_manager, _IMPORT_PROGRESS_TEXT_ONLY_PROP, False))

        if text_only:
            layout.label(text=description)
            return

        row = layout.row(align=True)
        bar = row.row()
        if hasattr(bar, "progress"):
            bar.progress(factor=fraction, type="BAR", text="")
        else:
            bar.prop(window_manager, _IMPORT_PROGRESS_VALUE_PROP, text="", slider=True)
        percent = row.row()
        percent.alignment = "RIGHT"
        percent.label(text=f"{int(round(fraction * 100.0))}%")
        layout.label(text=description)

class STARBREAKER_OT_refresh_materials(Operator):
    bl_idname = "starbreaker.refresh_materials"
    bl_label = "Refresh Materials"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = "Rebuild StarBreaker materials without reimporting meshes, empties, or lights"

    package_root_name: StringProperty(options={"HIDDEN"}, default="")  # type: ignore[assignment]
    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return _package_root_from_context(context) is not None or any(
            bool(obj.get(PROP_PACKAGE_ROOT)) for obj in bpy.data.objects
        )

    def _package_root(self, context: bpy.types.Context) -> bpy.types.Object | None:
        if self.package_root_name:
            candidate = bpy.data.objects.get(self.package_root_name)
            if candidate is not None and bool(candidate.get(PROP_PACKAGE_ROOT)):
                return candidate
        return _package_root_from_context(context)

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = self._package_root(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}
        palette_id = package_root.get(PROP_PALETTE_ID, "")
        try:
            applied = refresh_materials_for_package_root(
                context,
                package_root,
                palette_id if isinstance(palette_id, str) and palette_id else None,
            )
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        self.report({"INFO"}, f"Refreshed {applied} material slot group(s)")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        return self.execute(context)


class STARBREAKER_OT_apply_paint(Operator):
    bl_idname = "starbreaker.apply_paint"
    bl_label = "Apply Paint"
    bl_options = {"REGISTER", "UNDO"}

    paint_id: EnumProperty(name="Paint", items=_paint_items)

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if not self.paint_id:
            self.report({"ERROR"}, "No paint selected")
            return {"CANCELLED"}
        apply_paint_to_selected_package(context, self.paint_id)
        self.report({"INFO"}, f"Applied paint {self.paint_id}")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if not self.paint_id:
            package_root = _package_root_from_context(context)
            current_palette_id = package_root.get(PROP_PALETTE_ID, "") if package_root is not None else ""
            item_ids = _paint_items(self, context)
            valid_ids = {item_id for item_id, _, _ in item_ids if item_id}
            selected_paint_id = ""
            if isinstance(current_palette_id, str) and current_palette_id in valid_ids:
                selected_paint_id = current_palette_id
            else:
                selected_paint_id = _first_valid_item_id(item_ids)
            if not selected_paint_id:
                self.report({"ERROR"}, "No paint options are available for this package")
                return {"CANCELLED"}
            self.paint_id = selected_paint_id
        return context.window_manager.invoke_props_dialog(self)


class STARBREAKER_OT_apply_palette(Operator):
    bl_idname = "starbreaker.apply_palette"
    bl_label = "Apply Palette"
    bl_options = {"REGISTER", "UNDO"}

    palette_id: EnumProperty(name="Palette", items=_palette_items)

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if not self.palette_id:
            self.report({"ERROR"}, "No palette selected")
            return {"CANCELLED"}
        apply_palette_to_selected_package(context, self.palette_id)
        self.report({"INFO"}, f"Applied palette {self.palette_id}")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if not self.palette_id:
            package_root = _package_root_from_context(context)
            current_palette_id = package_root.get(PROP_PALETTE_ID, "") if package_root is not None else ""
            item_ids = _palette_items(self, context)
            valid_ids = {item_id for item_id, _, _ in item_ids if item_id}
            if isinstance(current_palette_id, str) and current_palette_id in valid_ids:
                self.palette_id = current_palette_id
            else:
                self.palette_id = _first_valid_item_id(item_ids)
        return context.window_manager.invoke_props_dialog(self)


class STARBREAKER_OT_apply_livery(Operator):
    bl_idname = "starbreaker.apply_livery"
    bl_label = "Apply Livery"
    bl_options = {"REGISTER", "UNDO"}

    livery_id: EnumProperty(name="Livery", items=_livery_items)

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if not self.livery_id:
            self.report({"ERROR"}, "No livery selected")
            return {"CANCELLED"}
        applied = apply_livery_to_selected_package(context, self.livery_id)
        self.report({"INFO"}, f"Updated {applied} material slots")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
        if not self.livery_id:
            self.livery_id = _first_valid_item_id(_livery_items(self, context))
        return context.window_manager.invoke_props_dialog(self)


class STARBREAKER_OT_switch_light_state(Operator):
    bl_idname = "starbreaker.switch_light_state"
    bl_label = "Switch Light State"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = (
        "Switch every imported StarBreaker light to the named CryEngine "
        "authored state (defaultState, auxiliaryState, emergencyState, "
        "cinematicState). Lights that lack the requested state keep their "
        "current values."
    )

    state_name: StringProperty(name="State")  # type: ignore[assignment]
    include_strobe: BoolProperty(name="Include Strobe", default=True)  # type: ignore[assignment]

    def execute(self, context: bpy.types.Context) -> set[str]:
        name = (self.state_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No state name provided")
            return {"CANCELLED"}
        count = apply_light_state(name, include_strobe=bool(self.include_strobe))
        self.report({"INFO"}, f"Applied '{name}' to {count} light(s)")
        return {"FINISHED"}


class STARBREAKER_OT_dump_metadata(Operator):
    bl_idname = "starbreaker.dump_metadata"
    bl_label = "Dump Metadata"
    bl_options = {"REGISTER"}

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return context.active_object is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        try:
            text_names = dump_selected_metadata(context)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        if not text_names:
            self.report({"WARNING"}, "No StarBreaker metadata found on the current selection")
            return {"CANCELLED"}
        self.report({"INFO"}, f"Created {len(text_names)} text datablocks")
        return {"FINISHED"}


class STARBREAKER_OT_make_instances_real(Operator):
    bl_idname = "starbreaker.make_instances_real"
    bl_label = "Make Instances Real"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = "Makes instances real, useful for 3d printing or exporting"

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return _package_root_from_context(context) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        if getattr(context, "mode", "OBJECT") != "OBJECT":
            self.report({"ERROR"}, "Switch to Object Mode before making instances real")
            return {"CANCELLED"}

        package_root = _package_root_from_context(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}

        realized_instances = _make_package_collection_instances_real(context, package_root)
        localized_data = _make_package_linked_object_data_local(package_root)
        pruned_slots = _prune_empty_mesh_material_slots(package_root)
        unresolved_materials = _detach_unresolved_empty_materials(package_root)
        if (
            realized_instances == 0
            and localized_data == 0
            and pruned_slots == 0
            and not unresolved_materials
        ):
            self.report(
                {"INFO"},
                "No collection instances, linked object data, or empty-mesh materials found under the selected StarBreaker package",
            )
            return {"FINISHED"}

        _focus_loaded_package_root(context, package_root)
        self.report(
            {"INFO"},
            (
                f"Made {realized_instances} instance container(s) real, "
                f"localized {localized_data} linked datablock(s), and "
                f"pruned {pruned_slots} empty-mesh material slot(s)"
            ),
        )
        if unresolved_materials:
            preview = ", ".join(unresolved_materials[:5])
            if len(unresolved_materials) > 5:
                preview = f"{preview}, +{len(unresolved_materials) - 5} more"
            self.report(
                {"WARNING"},
                (
                    f"Detached {len(unresolved_materials)} material(s) with no shader "
                    f"node tree to keep FBX export working — these did not rebuild and "
                    f"should be investigated: {preview}"
                ),
            )
        return {"FINISHED"}


class STARBREAKER_OT_apply_animation_mode(Operator):
    bl_idname = "starbreaker.apply_animation_mode"
    bl_label = "Apply Animation Mode"
    bl_options = {"REGISTER", "UNDO"}

    animation_name: StringProperty(name="Animation")  # type: ignore[assignment]
    mode: EnumProperty(name="Mode", items=_ANIMATION_MODE_ITEMS)  # type: ignore[assignment]
    fps_policy: EnumProperty(  # type: ignore[assignment]
        name="FPS Policy",
        items=_ANIMATION_FPS_POLICY_ITEMS,
        default="adapt_scene",
    )

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}
        name = (self.animation_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No animation selected")
            return {"CANCELLED"}
        package = _selected_package(context)
        warnings: list[str] = []
        if package is not None and self.mode == "action":
            warnings = animation_overlap_warnings(
                package_root,
                name,
                float(getattr(context.scene, "frame_current", 1.0)),
                context_scene=context.scene,
                fps_policy=self.fps_policy,
                package=package,
            )
        try:
            updated = apply_animation_mode_to_package_root(
                context,
                package_root,
                name,
                self.mode,
                fps_policy=self.fps_policy,
            )
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        if warnings:
            head = "; ".join(warnings[:2])
            if len(warnings) > 2:
                head = f"{head}; +{len(warnings) - 2} more overlap(s)"
            self.report({"WARNING"}, head)
        # After inserting, make sure the inline-frame scene props are
        # pre-initialised so that the panel draw never needs to mutate
        # ID data (which Blender forbids during draw callbacks).
        if self.mode == "action" and package is not None:
            try:
                _init_instance_start_props(
                    context,
                    package_root,
                    package_animation_instances(package_root, package),
                )
            except Exception:
                pass
        self.report({"INFO"}, f"{name}: {self.mode} ({updated} object(s) updated)")
        return {"FINISHED"}


class STARBREAKER_OT_edit_animation_instance(Operator):
    bl_idname = "starbreaker.apply_animation_instance_start_frame"
    bl_label = "Apply Animation Instance Start Frame"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = "Apply the inline start-frame value for this animation instance"

    instance_id: StringProperty(name="Instance Id")  # type: ignore[assignment]
    scene_prop_key: StringProperty(name="Scene Prop Key")  # type: ignore[assignment]

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        package = _selected_package(context)
        if package_root is None or package is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}

        raw_value = context.scene.get(self.scene_prop_key, None)
        if raw_value is None:
            self.report({"ERROR"}, "Missing inline frame value")
            return {"CANCELLED"}
        try:
            start_frame = float(raw_value)
        except Exception:
            self.report({"ERROR"}, "Inline frame value must be numeric")
            return {"CANCELLED"}

        updated = update_animation_instance_start_frame(
            package_root,
            self.instance_id,
            start_frame,
            package=package,
        )
        if not updated:
            self.report({"ERROR"}, "Animation instance not found")
            return {"CANCELLED"}
        self.report({"INFO"}, f"Moved to frame {int(round(start_frame))}")
        return {"FINISHED"}


class STARBREAKER_OT_delete_animation_instance(Operator):
    """Delete this animation instance without confirmation"""

    bl_idname = "starbreaker.delete_animation_instance"
    bl_label = "Delete Instance"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = "Remove this animation instance and its NLA strips"

    instance_id: StringProperty(name="Instance Id")  # type: ignore[assignment]

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        package = _selected_package(context)
        if package_root is None or package is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}
        deleted = delete_animation_instance(package_root, self.instance_id, package=package)
        if not deleted:
            self.report({"ERROR"}, "Animation instance not found")
            return {"CANCELLED"}
        self.report({"INFO"}, "Deleted animation instance")
        return {"FINISHED"}


def _nla_track_mute_state(package_root: bpy.types.Object, track_name: str) -> bool:
    """Return True when the named NLA track is muted on any object in the package."""
    for obj in [package_root, *package_root.children_recursive]:
        if not obj.animation_data:
            continue
        for track in obj.animation_data.nla_tracks:
            if track.name == track_name:
                return track.mute
    return False


def _set_nla_track_mute(
    package_root: bpy.types.Object, track_name: str, mute: bool
) -> None:
    """Set .mute on every NLA track named *track_name* across the package."""
    for obj in [package_root, *package_root.children_recursive]:
        if not obj.animation_data:
            continue
        for track in obj.animation_data.nla_tracks:
            if track.name == track_name:
                track.mute = mute


class STARBREAKER_OT_mute_animation_instance(Operator):
    """Toggle mute on the NLA strips for this animation instance"""

    bl_idname = "starbreaker.mute_animation_instance"
    bl_label = "Toggle Mute"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = "Mute or unmute this animation instance's NLA strips"

    instance_id: StringProperty(name="Instance Id")  # type: ignore[assignment]

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        package = _selected_package(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}
        if package is None:
            self.report({"ERROR"}, "No package selected")
            return {"CANCELLED"}
        muted = set_animation_instance_muted(package_root, self.instance_id, None, package=package)
        if muted is None:
            self.report({"ERROR"}, "Animation instance not found")
            return {"CANCELLED"}
        self.report({"INFO"}, "Muted animation instance" if muted else "Unmuted animation instance")
        return {"FINISHED"}


class STARBREAKER_OT_solo_animation_instance(Operator):
    """Solo this animation instance — mute all others"""

    bl_idname = "starbreaker.solo_animation_instance"
    bl_label = "Solo Instance"
    bl_options = {"REGISTER", "UNDO"}
    bl_description = "Unmute this instance and mute all other animation instances"

    instance_id: StringProperty(name="Instance Id")  # type: ignore[assignment]

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return find_package_root(context.active_object) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        package = _selected_package(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}
        if package is None:
            self.report({"ERROR"}, "No package selected")
            return {"CANCELLED"}
        if not solo_animation_instance(package_root, self.instance_id, package=package):
            self.report({"ERROR"}, "Animation instance not found")
            return {"CANCELLED"}
        self.report({"INFO"}, "Soloed animation instance")
        return {"FINISHED"}


class STARBREAKER_OT_dump_animation_diagnostics(Operator):
    bl_idname = "starbreaker.dump_animation_diagnostics"
    bl_label = "Animation Diagnostics"
    bl_options = {"REGISTER"}
    bl_description = "Dump hash/object matching diagnostics for one animation"

    animation_name: StringProperty(name="Animation")  # type: ignore[assignment]

    @classmethod
    def poll(cls, context: bpy.types.Context) -> bool:
        return _package_root_from_context(context) is not None

    def execute(self, context: bpy.types.Context) -> set[str]:
        package_root = _package_root_from_context(context)
        if package_root is None:
            self.report({"ERROR"}, "Select an imported StarBreaker object first")
            return {"CANCELLED"}

        package = _selected_package(context)
        if package is None:
            self.report({"ERROR"}, "Unable to load package from selected object")
            return {"CANCELLED"}

        name = (self.animation_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No animation selected")
            return {"CANCELLED"}

        try:
            diagnostics = package_animation_diagnostics(package, package_root, name)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}

        text_name = f"starbreaker_anim_diag_{Path(name).stem}.json"
        text = bpy.data.texts.get(text_name)
        if text is None:
            text = bpy.data.texts.new(text_name)
        else:
            text.clear()
        text.from_string(json.dumps(diagnostics, indent=2, sort_keys=True))

        self.report(
            {"INFO"},
            (
                f"{name}: {diagnostics['matched_object_count']} objects, "
                f"{diagnostics['unmatched_hash_count']} unmatched hashes "
                f"(saved to {text.name})"
            ),
        )
        return {"FINISHED"}


class STARBREAKER_PT_tools(Panel):
    bl_label = "StarBreaker"
    bl_idname = "STARBREAKER_PT_tools"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "StarBreaker"

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout

        obj = context.active_object
        package_root = _package_root_from_context(context)
        if package_root is None:
            layout.label(text="Open scene.blend or other")
            layout.label(text="StarBreaker exported .blend file")
            layout.label(text="for more options")
            return
        _sync_scene_package_controls(context, package_root)

        package = _selected_package(context)
        info = layout.box()
        info.label(text=f"Package: {package_root.get(PROP_PACKAGE_NAME, '')}")
        info.label(text=f"Entity: {obj.get(PROP_ENTITY_NAME, obj.name) if obj else ''}")
        info.label(text=f"Palette: {package_root.get(PROP_PALETTE_ID, '')}")
        if obj is not None:
            material_sidecar = obj.get(PROP_MATERIAL_SIDECAR)
            if isinstance(material_sidecar, str) and material_sidecar:
                info.label(text=f"Sidecar: {Path(material_sidecar).name}")

        actions = layout.row(align=True)
        actions.operator_menu_enum(STARBREAKER_OT_apply_paint.bl_idname, "paint_id", text="Apply Paint", icon="BRUSH_DATA")
        actions.operator(STARBREAKER_OT_refresh_materials.bl_idname, text="Refresh Materials", icon="MATERIAL")
        layout.operator(STARBREAKER_OT_dump_metadata.bl_idname, icon="TEXT")

        tuning = layout.box()
        tuning.prop(context.scene, SCENE_POM_DETAIL_PROP, text="POM Detail")
        tuning.label(text="Layered Wear")
        tuning.prop(context.scene, SCENE_WEAR_STRENGTH_PROP, slider=True)
        if decal_offset_control_enabled(package_root):
            tuning.label(text="Decal Offset")
            tuning.prop(package_root, PROP_DECAL_OFFSET_EXTERNAL, text="Exterior")
            tuning.prop(package_root, PROP_DECAL_OFFSET_INTERNAL, text="Interior")

        if obj is not None and obj.active_material is not None:
            material = obj.active_material
            material_box = layout.box()
            material_box.label(text=f"Shader: {material.get(PROP_SHADER_FAMILY, '')}")
            material_box.label(text=f"Template: {material.get(PROP_TEMPLATE_KEY, '')}")
            material_box.label(text=f"Surface: {material.get(PROP_SURFACE_SHADER_MODE, '')}")

        # Phase 28: light state switcher. Show a row of buttons when the
        # current .blend has any imported lights with authored states.
        state_names = available_light_state_names()
        if state_names or engine_glow_control_enabled(package_root) or shared_glow_control_enabled(package_root):
            light_box = layout.box()
            if state_names:
                light_box.label(text="Lighting Panel")
                _SHORT = {
                    "defaultState": "Default",
                    "auxiliaryState": "Auxiliary",
                    "emergencyState": "Emergency",
                    "cinematicState": "Cinematic",
                    "offState": "Off",
                }
                _ICONS = {
                    "defaultState": "CHECKMARK",
                    "auxiliaryState": "LIGHT",
                    "emergencyState": "ERROR",
                    "cinematicState": "RESTRICT_RENDER_OFF",
                    "offState": "HIDE_ON",
                    "emergencyState_strobe": "LIGHT_SUN",
                }
                button_specs: list[tuple[str, str, bool, str]] = []
                for name in state_names:
                    if name == "emergencyState":
                        button_specs.append(("Emergency", name, False, _ICONS["emergencyState"]))
                        button_specs.append(("Emergency + Strobe", name, True, _ICONS["emergencyState_strobe"]))
                    else:
                        button_specs.append((_SHORT.get(name, name), name, True, _ICONS.get(name, "NONE")))

                split = (len(button_specs) + 1) // 2
                top_row = light_box.row(align=True)
                bottom_row = light_box.row(align=True)
                for index, (label, state_name, include_strobe, icon_name) in enumerate(button_specs):
                    row = top_row if index < split else bottom_row
                    kwargs = {"text": label}
                    if icon_name != "NONE":
                        kwargs["icon"] = icon_name
                    op = row.operator(STARBREAKER_OT_switch_light_state.bl_idname, **kwargs)
                    op.state_name = state_name
                    op.include_strobe = include_strobe
            elif engine_glow_control_enabled(package_root) or shared_glow_control_enabled(package_root):
                light_box.label(text="Lighting Panel")
            if engine_glow_control_enabled(package_root):
                light_box.prop(context.scene, SCENE_ENGINE_GLOW_PROP, text="Engine Glow Strength", slider=True)
            if shared_glow_control_enabled(package_root):
                light_box.prop(context.scene, SCENE_SHARED_GLOW_PROP, text="Shared Glow", slider=True)

        tools_box = layout.box()
        tools_box.label(text="Tools")
        tools_box.operator(
            STARBREAKER_OT_make_instances_real.bl_idname,
            text="Make Instances Real",
            icon="OUTLINER_OB_EMPTY",
        )

        if package is not None:
            animation_items = available_package_animation_items(package)
            animation_box = layout.box()
            animation_box.label(text="Animations")
            animation_box.label(text=f"{len(animation_items)} clip(s)")
            if not animation_items:
                animation_box.label(text="No animations exported in this scene.json")
            else:
                animation_box.prop(context.scene, _SCENE_ANIMATION_FPS_POLICY_PROP, text="FPS")
                try:
                    mode_map = package_animation_mode_map(package_root)
                    instances = package_animation_instances(package_root, package, persist=False)
                    instances_by_animation: dict[str, list[dict[str, object]]] = {}
                    for instance in instances:
                        key = str(instance.get("animation_name", ""))
                        if not key:
                            continue
                        instances_by_animation.setdefault(key, []).append(instance)
                    for values in instances_by_animation.values():
                        values.sort(key=lambda item: float(item.get("start_frame", 1.0)))
                    fps_policy = getattr(context.scene, _SCENE_ANIMATION_FPS_POLICY_PROP, "adapt_scene")
                    for animation_name, animation_display_name in animation_items:
                        animation_box.label(text=animation_display_name)
                        current_mode = mode_map.get(animation_name, "none")
                        buttons_row = animation_box.row(align=True)
                        for mode_id, mode_label, _ in _ANIMATION_MODE_ITEMS:
                            op = buttons_row.operator(
                                STARBREAKER_OT_apply_animation_mode.bl_idname,
                                text=mode_label,
                                depress=(current_mode == mode_id),
                            )
                            op.animation_name = animation_name
                            op.mode = mode_id
                            op.fps_policy = fps_policy

                        instance_entries = instances_by_animation.get(animation_name, [])
                        if instance_entries:
                            for instance in instance_entries:
                                start = int(round(float(instance.get("start_frame", 1.0))))
                                instance_id = str(instance.get("id", ""))
                                track_name = f"{animation_name}@{start}"
                                is_muted = _nla_track_mute_state(package_root, track_name)

                                row = animation_box.row(align=True)
                                row.label(text=animation_display_name)

                                # Inline frame number control (replaces old dialog workflow).
                                scene_prop_key = f"starbreaker_anim_start_{instance_id}"
                                if context.scene.get(scene_prop_key) is None:
                                    # Writing to ID classes is forbidden during draw.
                                    # Defer the initialisation to a timer and show a
                                    # read-only label for this one redraw cycle.
                                    _schedule_init_scene_prop(scene_prop_key, int(start))
                                    row.label(text=f"@{start}")
                                else:
                                    row.prop(context.scene, f'["{scene_prop_key}"]', text="@")

                                apply_op = row.operator(
                                    STARBREAKER_OT_edit_animation_instance.bl_idname,
                                    text="",
                                    icon="CHECKMARK",
                                )
                                apply_op.instance_id = instance_id
                                apply_op.scene_prop_key = scene_prop_key

                                # Mute toggle
                                mute_op = row.operator(
                                    STARBREAKER_OT_mute_animation_instance.bl_idname,
                                    text="",
                                    icon="HIDE_ON" if is_muted else "HIDE_OFF",
                                    depress=is_muted,
                                )
                                mute_op.instance_id = instance_id

                                # Solo
                                solo_op = row.operator(
                                    STARBREAKER_OT_solo_animation_instance.bl_idname,
                                    text="",
                                    icon="EVENT_S",
                                )
                                solo_op.instance_id = instance_id

                                # Delete
                                del_op = row.operator(
                                    STARBREAKER_OT_delete_animation_instance.bl_idname,
                                    text="",
                                    icon="TRASH",
                                )
                                del_op.instance_id = instance_id
                except Exception as exc:
                    animation_box.label(text=f"Animation UI error: {exc}")


# ── Addon Preferences ────────────────────────────────────────────────────────

class STARBREAKER_AP_preferences(bpy.types.AddonPreferences):
    """User-configurable post-import behaviour for the StarBreaker addon."""

    bl_idname = "starbreaker_addon"

    viewport_change_after_import: BoolProperty(  # type: ignore[assignment]
        name="Adjust viewport after import",
        description=(
            "Frame the imported package in the 3D viewport and switch to "
            "perspective view after a successful import."
        ),
        default=True,
    )

    landing_gear_retract_after_import: BoolProperty(  # type: ignore[assignment]
        name="Retract landing gear after import",
        description=(
            "Automatically apply the best landing-gear retracted pose when a "
            "package is imported, so ships appear ready-for-flight by default."
        ),
        default=True,
    )

    auto_refresh_unloaded_materials_on_load: BoolProperty(  # type: ignore[assignment]
        name="Refresh unloaded materials after file load",
        description=(
            "Automatically rebuild package materials after opening a generated "
            ".blend file when placeholder or unloaded material slots still need refresh."
        ),
        default=True,
    )

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout
        layout.label(text="Post-import behaviour:")
        layout.prop(self, "landing_gear_retract_after_import")
        layout.prop(self, "viewport_change_after_import")
        layout.separator()
        layout.label(text="Post-open behaviour:")
        layout.prop(self, "auto_refresh_unloaded_materials_on_load")


def _get_prefs() -> STARBREAKER_AP_preferences | None:
    """Return the addon preferences object, or None when not available."""
    addon_entry = bpy.context.preferences.addons.get("starbreaker_addon")
    if addon_entry is None:
        return None
    return getattr(addon_entry, "preferences", None)


def _should_apply_landing_gear(prefs: object | None) -> bool:
    """Pure gate — return True when landing-gear retract should run after import."""
    if prefs is None:
        return True
    return bool(getattr(prefs, "landing_gear_retract_after_import", True))


def _should_change_viewport(prefs: object | None) -> bool:
    """Pure gate — return True when viewport framing should run after import."""
    if prefs is None:
        return True
    return bool(getattr(prefs, "viewport_change_after_import", True))


def _should_auto_refresh_unloaded_materials_on_load(prefs: object | None) -> bool:
    """Pure gate — return True when load-post should auto-refresh unloaded materials."""
    if prefs is None:
        return True
    return bool(getattr(prefs, "auto_refresh_unloaded_materials_on_load", True))


def _should_auto_refresh_package_root(obj: object, needs_material_refresh: object) -> bool:
    """Return True when load-post should rebuild materials for a package root."""
    get_prop = getattr(obj, "get", None)
    if not callable(get_prop) or not bool(get_prop(PROP_PACKAGE_ROOT)):
        return False
    if not callable(needs_material_refresh):
        return False
    return bool(needs_material_refresh(obj))


def _material_refresh_prompt_timer(token: int | None = None) -> float | None:
    if token is not None and token != _AUTO_MATERIAL_REFRESH_TOKEN:
        return None
    context = bpy.context
    prefs = _get_prefs()

    if not _should_auto_refresh_unloaded_materials_on_load(prefs):
        return None

    package_roots = _loaded_package_roots(bpy.data.objects, max_depth=2)
    if package_roots:
        _focus_loaded_package_root(context, package_roots[0])
        try:
            bpy.app.timers.register(
                lambda root_name=package_roots[0].name: _deferred_focus_loaded_package_root(root_name),
                first_interval=0.25,
                persistent=False,
            )
        except Exception:
            pass

    for obj in package_roots:
        get_prop = getattr(obj, "get", None)
        if not callable(get_prop) or not bool(get_prop(PROP_PACKAGE_ROOT)):
            continue
        dirty_objects = dirty_package_material_objects(obj)
        if not dirty_objects:
            continue
        palette_id = obj.get(PROP_PALETTE_ID, "")
        package_name = obj.get(PROP_PACKAGE_NAME, obj.name)
        _begin_import_progress(
            context,
            "Loading materials, please wait",
            indeterminate=True,
            text_only=True,
        )
        try:
            bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=1)
        except Exception:
            pass
        progress_callback = lambda fraction, description: _update_import_progress(
            context,
            fraction,
            "Loading materials, please wait",
            force=True,
            redraw=False,
        )
        applied = refresh_materials_for_package_root(
            context,
            obj,
            palette_id if isinstance(palette_id, str) and palette_id else None,
            only_unloaded=True,
            target_objects=dirty_objects,
            progress_callback=progress_callback,
            progress_interval_seconds=5.0,
        )
        _end_import_progress(
            context,
            f"Refreshed {applied} material slots for {package_name}",
        )
        return None
    return None


@persistent
def _starbreaker_load_post(_dummy: object) -> None:
    global _AUTO_MATERIAL_REFRESH_TOKEN
    _AUTO_MATERIAL_REFRESH_TOKEN += 1
    token = _AUTO_MATERIAL_REFRESH_TOKEN
    bpy.app.timers.register(
        lambda: _material_refresh_prompt_timer(token),
        first_interval=0.05,
        persistent=False,
    )


CLASSES = [
    STARBREAKER_AP_preferences,
    STARBREAKER_OT_refresh_materials,
    STARBREAKER_OT_apply_paint,
    STARBREAKER_OT_apply_palette,
    STARBREAKER_OT_apply_livery,
    STARBREAKER_OT_switch_light_state,
    STARBREAKER_OT_dump_metadata,
    STARBREAKER_OT_make_instances_real,
    STARBREAKER_OT_apply_animation_mode,
    STARBREAKER_OT_edit_animation_instance,
    STARBREAKER_OT_delete_animation_instance,
    STARBREAKER_OT_mute_animation_instance,
    STARBREAKER_OT_solo_animation_instance,
    STARBREAKER_OT_dump_animation_diagnostics,
    STARBREAKER_PT_tools,
]


def register() -> None:
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_ACTIVE_PROP,
        BoolProperty(name="StarBreaker Import Active", default=False),
    )
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_VALUE_PROP,
        FloatProperty(
            name="StarBreaker Import Progress",
            default=0.0,
            min=0.0,
            max=1.0,
        ),
    )
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_DESCRIPTION_PROP,
        StringProperty(name="StarBreaker Import Progress Description", default=""),
    )
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_INDETERMINATE_PROP,
        BoolProperty(name="StarBreaker Import Progress Indeterminate", default=False),
    )
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_TEXT_ONLY_PROP,
        BoolProperty(name="StarBreaker Import Progress Text Only", default=False),
    )
    setattr(
        bpy.types.Scene,
        SCENE_POM_DETAIL_PROP,
        EnumProperty(
            name="POM Detail",
            description=(
                "Global quality preset for StarBreaker parallax-occlusion materials. "
                "Updates the shared runtime POM detail group so imported POM materials "
                "change quality together without rewriting each material node tree."
            ),
            items=POM_DETAIL_ITEMS,
            default=POM_DETAIL_DEFAULT,
            update=_update_pom_detail,
        ),
    )
    setattr(
        bpy.types.Scene,
        SCENE_WEAR_STRENGTH_PROP,
        FloatProperty(
            name="Wear Strength",
            description=(
                "Scale layered wear contribution for imported StarBreaker "
                "layered materials. Default is 0 because vertex-colour-driven "
                "wear on ship hulls would otherwise blend the primary paint "
                "toward a worn-grey layer on every import, which does not "
                "match the default in-game appearance of a freshly spawned "
                "ship. Raise this slider to expose the authored wear layer."
            ),
            default=0.0,
            min=0.0,
            max=2.0,
            soft_min=0.0,
            soft_max=2.0,
        ),
    )
    setattr(
        bpy.types.Scene,
        SCENE_ENGINE_GLOW_PROP,
        FloatProperty(
            name="Engine Glow",
            description=(
                "Absolute emission strength for DataCore thruster glow materials exported in scene.json controls."
            ),
            default=3.0,
            min=0.0,
            max=200.0,
            soft_min=0.0,
            soft_max=200.0,
            update=_update_engine_glow,
        ),
    )
    setattr(
        bpy.types.Scene,
        SCENE_SHARED_GLOW_PROP,
        FloatProperty(
            name="Shared Glow",
            description=(
                "Additional emission strength added to imported glow-style MeshDecal materials."
            ),
            default=0.0,
            min=0.0,
            max=10.0,
            soft_min=0.0,
            soft_max=10.0,
            update=_update_shared_glow,
        ),
    )
    setattr(
        bpy.types.Object,
        PROP_DECAL_OFFSET_EXTERNAL,
        FloatProperty(
            name="Exterior Decal Offset",
            description=(
                "Offset strength for imported StarBreaker decal displacement modifiers "
                "on exterior ship-hull objects in the selected package root."
            ),
            default=DECAL_OFFSET_EXTERNAL_DEFAULT,
            min=0.0,
            soft_max=0.02,
            precision=4,
            update=_update_decal_offset,
        ),
    )
    setattr(
        bpy.types.Object,
        PROP_DECAL_OFFSET_INTERNAL,
        FloatProperty(
            name="Interior Decal Offset",
            description=(
                "Offset strength for imported StarBreaker decal displacement modifiers "
                "on interior and loadout objects in the selected package root."
            ),
            default=DECAL_OFFSET_INTERNAL_DEFAULT,
            min=0.0,
            soft_max=0.02,
            precision=4,
            update=_update_decal_offset,
        ),
    )
    setattr(
        bpy.types.Scene,
        _SCENE_ANIMATION_FPS_POLICY_PROP,
        EnumProperty(
            name="Animation FPS",
            description=(
                "How to reconcile clip FPS with scene FPS when inserting animation actions. "
                "Snap modes are unaffected."
            ),
            items=_ANIMATION_FPS_POLICY_ITEMS,
            default="adapt_scene",
        ),
    )
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    for handler in list(bpy.app.handlers.load_post):
        if (
            handler is not _starbreaker_load_post
            and getattr(handler, "__name__", "") == _starbreaker_load_post.__name__
        ):
            bpy.app.handlers.load_post.remove(handler)
    if _starbreaker_load_post not in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.append(_starbreaker_load_post)


def unregister() -> None:
    if _starbreaker_load_post in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.remove(_starbreaker_load_post)
    for cls in reversed(CLASSES):
        try:
            bpy.utils.unregister_class(cls)
        except RuntimeError:
            pass
    if hasattr(bpy.types.Scene, SCENE_POM_DETAIL_PROP):
        delattr(bpy.types.Scene, SCENE_POM_DETAIL_PROP)
    if hasattr(bpy.types.Scene, SCENE_WEAR_STRENGTH_PROP):
        delattr(bpy.types.Scene, SCENE_WEAR_STRENGTH_PROP)
    if hasattr(bpy.types.Scene, SCENE_ENGINE_GLOW_PROP):
        delattr(bpy.types.Scene, SCENE_ENGINE_GLOW_PROP)
    if hasattr(bpy.types.Scene, SCENE_SHARED_GLOW_PROP):
        delattr(bpy.types.Scene, SCENE_SHARED_GLOW_PROP)
    if hasattr(bpy.types.Object, PROP_DECAL_OFFSET_EXTERNAL):
        delattr(bpy.types.Object, PROP_DECAL_OFFSET_EXTERNAL)
    if hasattr(bpy.types.Object, PROP_DECAL_OFFSET_INTERNAL):
        delattr(bpy.types.Object, PROP_DECAL_OFFSET_INTERNAL)
    if hasattr(bpy.types.Scene, _SCENE_ANIMATION_FPS_POLICY_PROP):
        delattr(bpy.types.Scene, _SCENE_ANIMATION_FPS_POLICY_PROP)
    for prop_name in (
        _IMPORT_PROGRESS_ACTIVE_PROP,
        _IMPORT_PROGRESS_VALUE_PROP,
        _IMPORT_PROGRESS_DESCRIPTION_PROP,
        _IMPORT_PROGRESS_INDETERMINATE_PROP,
        _IMPORT_PROGRESS_TEXT_ONLY_PROP,
    ):
        if hasattr(bpy.types.WindowManager, prop_name):
            delattr(bpy.types.WindowManager, prop_name)
