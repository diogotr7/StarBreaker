from __future__ import annotations

from pathlib import Path
import time

import bpy
import blf
import gpu
from bpy.props import BoolProperty, EnumProperty, FloatProperty, StringProperty
from bpy.types import Operator, Panel
from bpy_extras.io_utils import ImportHelper
from gpu_extras.batch import batch_for_shader

from .manifest import PackageBundle
from .palette import resolved_palette_id
from .palette import paint_list_canonical_id
from .runtime import (
    PROP_ENTITY_NAME,
    PROP_MATERIAL_SIDECAR,
    PROP_PACKAGE_NAME,
    PROP_PALETTE_ID,
    PROP_SCENE_PATH,
    PROP_SHADER_FAMILY,
    PROP_SURFACE_SHADER_MODE,
    PROP_TEMPLATE_KEY,
    SCENE_WEAR_STRENGTH_PROP,
    apply_light_state,
    apply_livery_to_selected_package,
    apply_paint_to_selected_package,
    apply_palette_to_selected_package,
    available_light_state_names,
    dump_selected_metadata,
    exterior_palette_ids,
    find_package_root,
    import_package,
)


_PAINT_ITEMS_CACHE: list[tuple[str, str, str]] = []
_PALETTE_ITEMS_CACHE: list[tuple[str, str, str]] = []
_LIVERY_ITEMS_CACHE: list[tuple[str, str, str]] = []
_IMPORT_PROGRESS_ACTIVE_PROP = "starbreaker_import_progress_active"
_IMPORT_PROGRESS_VALUE_PROP = "starbreaker_import_progress_value"
_IMPORT_PROGRESS_DESCRIPTION_PROP = "starbreaker_import_progress_description"
_IMPORT_PROGRESS_LAST_UPDATE = 0.0
_IMPORT_PROGRESS_DRAW_HANDLER = None


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

    panel_width = min(480.0, max(region.width - 80.0, 320.0))
    panel_height = 96.0
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
    draw_rect(bar_x, bar_y, bar_width, bar_height, (0.18, 0.21, 0.25, 1.0))
    if fraction > 0.0:
        draw_rect(bar_x, bar_y, bar_width * fraction, bar_height, (0.23, 0.62, 0.86, 1.0))
    gpu.state.blend_set("NONE")

    font_id = 0
    try:
        blf.size(font_id, 14.0)
    except TypeError:
        blf.size(font_id, 14, 72)
    blf.color(font_id, 0.96, 0.97, 0.98, 1.0)
    blf.position(font_id, bar_x + bar_width + 16.0, bar_y + 4.0, 0)
    blf.draw(font_id, f"{int(round(fraction * 100.0))}%")

    try:
        blf.size(font_id, 13.0)
    except TypeError:
        blf.size(font_id, 13, 72)
    blf.position(font_id, bar_x, panel_y + padding, 0)
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


def _begin_import_progress(context: bpy.types.Context, description: str) -> None:
    global _IMPORT_PROGRESS_LAST_UPDATE
    window_manager = context.window_manager
    setattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, True)
    setattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, 0.0)
    setattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, description)
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
) -> None:
    global _IMPORT_PROGRESS_LAST_UPDATE
    now = time.monotonic()
    if not force and now - _IMPORT_PROGRESS_LAST_UPDATE < 0.5:
        return
    window_manager = context.window_manager
    clamped = _progress_fraction(fraction)
    setattr(window_manager, _IMPORT_PROGRESS_VALUE_PROP, clamped)
    setattr(window_manager, _IMPORT_PROGRESS_DESCRIPTION_PROP, description)
    _IMPORT_PROGRESS_LAST_UPDATE = now
    try:
        window_manager.progress_update(int(round(clamped * 100.0)))
    except Exception:
        pass
    try:
        bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=1)
    except Exception:
        pass
    _tag_view3d_redraws(context)


def _end_import_progress(context: bpy.types.Context, description: str) -> None:
    window_manager = context.window_manager
    _update_import_progress(context, 1.0, description, force=True)
    setattr(window_manager, _IMPORT_PROGRESS_ACTIVE_PROP, False)
    _tag_view3d_redraws(context)
    try:
        window_manager.progress_end()
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


def _selected_package(context: bpy.types.Context) -> PackageBundle | None:
    package_root = _package_root_from_context(context)
    if package_root is None:
        return None
    scene_path = package_root.get(PROP_SCENE_PATH)
    if not isinstance(scene_path, str) or not scene_path:
        return None
    try:
        return PackageBundle.load(scene_path)
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
            if isinstance(current_palette_id, str) and current_palette_id in valid_ids:
                self.paint_id = current_palette_id
            else:
                self.paint_id = _first_valid_item_id(item_ids)
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

    def execute(self, context: bpy.types.Context) -> set[str]:
        name = (self.state_name or "").strip()
        if not name:
            self.report({"ERROR"}, "No state name provided")
            return {"CANCELLED"}
        count = apply_light_state(name)
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


class STARBREAKER_PT_tools(Panel):
    bl_label = "StarBreaker"
    bl_idname = "STARBREAKER_PT_tools"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "StarBreaker"

    def draw(self, context: bpy.types.Context) -> None:
        layout = self.layout
        layout.operator(STARBREAKER_OT_import_decomposed_package.bl_idname, icon="IMPORT")

        obj = context.active_object
        package_root = _package_root_from_context(context)
        if package_root is None:
            return

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
        layout.operator(STARBREAKER_OT_dump_metadata.bl_idname, icon="TEXT")

        tuning = layout.box()
        tuning.label(text="Layered Wear")
        tuning.prop(context.scene, SCENE_WEAR_STRENGTH_PROP, slider=True)

        if package is not None:
            available = layout.box()
            available.label(text=f"Palettes: {', '.join(sorted(package.palettes.keys()))}")
            available.label(text=f"Liveries: {', '.join(sorted(package.liveries.keys()))}")

        if obj is not None and obj.active_material is not None:
            material = obj.active_material
            material_box = layout.box()
            material_box.label(text=f"Shader: {material.get(PROP_SHADER_FAMILY, '')}")
            material_box.label(text=f"Template: {material.get(PROP_TEMPLATE_KEY, '')}")
            material_box.label(text=f"Surface: {material.get(PROP_SURFACE_SHADER_MODE, '')}")

        # Phase 28: light state switcher. Show a row of buttons when the
        # current .blend has any imported lights with authored states.
        state_names = available_light_state_names()
        if state_names:
            light_box = layout.box()
            light_box.label(text="Light States")
            row = light_box.row(align=True)
            _SHORT = {
                "defaultState": "Default",
                "auxiliaryState": "Auxiliary",
                "emergencyState": "Emergency",
                "cinematicState": "Cinematic",
                "offState": "Off",
            }
            for name in state_names:
                op = row.operator(
                    STARBREAKER_OT_switch_light_state.bl_idname,
                    text=_SHORT.get(name, name),
                )
                op.state_name = name


CLASSES = [
    STARBREAKER_OT_import_decomposed_package,
    STARBREAKER_OT_import_progress_popup,
    STARBREAKER_OT_apply_paint,
    STARBREAKER_OT_apply_palette,
    STARBREAKER_OT_apply_livery,
    STARBREAKER_OT_switch_light_state,
    STARBREAKER_OT_dump_metadata,
    STARBREAKER_PT_tools,
]


def register() -> None:
    setattr(bpy.types.WindowManager, _IMPORT_PROGRESS_ACTIVE_PROP, BoolProperty(default=False))
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_VALUE_PROP,
        FloatProperty(default=0.0, min=0.0, max=1.0),
    )
    setattr(
        bpy.types.WindowManager,
        _IMPORT_PROGRESS_DESCRIPTION_PROP,
        StringProperty(default="Preparing import"),
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
    for cls in CLASSES:
        bpy.utils.register_class(cls)


def unregister() -> None:
    _remove_import_progress_overlay()
    for cls in reversed(CLASSES):
        try:
            bpy.utils.unregister_class(cls)
        except RuntimeError:
            pass
    for prop_name in (
        _IMPORT_PROGRESS_ACTIVE_PROP,
        _IMPORT_PROGRESS_VALUE_PROP,
        _IMPORT_PROGRESS_DESCRIPTION_PROP,
    ):
        if hasattr(bpy.types.WindowManager, prop_name):
            delattr(bpy.types.WindowManager, prop_name)
    if hasattr(bpy.types.Scene, SCENE_WEAR_STRENGTH_PROP):
        delattr(bpy.types.Scene, SCENE_WEAR_STRENGTH_PROP)
