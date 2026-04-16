from __future__ import annotations

from pathlib import Path

import bpy
from bpy.props import BoolProperty, EnumProperty, StringProperty
from bpy.types import Operator, Panel
from bpy_extras.io_utils import ImportHelper

from .manifest import PackageBundle
from .runtime import (
    PROP_ENTITY_NAME,
    PROP_MATERIAL_SIDECAR,
    PROP_PACKAGE_NAME,
    PROP_PALETTE_ID,
    PROP_SCENE_PATH,
    PROP_SHADER_FAMILY,
    PROP_TEMPLATE_KEY,
    apply_livery_to_selected_package,
    apply_palette_to_selected_package,
    dump_selected_metadata,
    find_package_root,
    import_package,
)


def _selected_package(context: bpy.types.Context) -> PackageBundle | None:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        return None
    scene_path = package_root.get(PROP_SCENE_PATH)
    if not isinstance(scene_path, str) or not scene_path:
        return None
    try:
        return PackageBundle.load(scene_path)
    except Exception:
        return None


def _palette_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    package = _selected_package(context)
    if package is None:
        return [("", "No imported package", "Import a StarBreaker package first")]
    return [
        (palette_id, palette_id, package.palettes[palette_id].source_name or palette_id)
        for palette_id in sorted(package.palettes.keys())
    ]


def _livery_items(_: bpy.types.Operator, context: bpy.types.Context) -> list[tuple[str, str, str]]:
    package = _selected_package(context)
    if package is None:
        return [("", "No imported package", "Import a StarBreaker package first")]
    return [
        (livery_id, livery_id, package.liveries[livery_id].palette_source_name or livery_id)
        for livery_id in sorted(package.liveries.keys())
    ]


class STARBREAKER_OT_import_decomposed_package(Operator, ImportHelper):
    bl_idname = "starbreaker.import_decomposed_package"
    bl_label = "Import StarBreaker Package"
    bl_options = {"REGISTER", "UNDO"}

    filter_glob: StringProperty(default="scene.json;*.json", options={"HIDDEN"})
    prefer_cycles: BoolProperty(
        name="Prefer Cycles",
        description="Switch the active scene to Cycles before import",
        default=True,
    )

    def execute(self, context: bpy.types.Context) -> set[str]:
        try:
            package_root = import_package(context, self.filepath, prefer_cycles=self.prefer_cycles)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        self.report({"INFO"}, f"Imported {package_root.get(PROP_PACKAGE_NAME, Path(self.filepath).parent.name)}")
        return {"FINISHED"}


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
        applied = apply_palette_to_selected_package(context, self.palette_id)
        self.report({"INFO"}, f"Updated {applied} material slots")
        return {"FINISHED"}

    def invoke(self, context: bpy.types.Context, event: bpy.types.Event) -> set[str]:
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
        return context.window_manager.invoke_props_dialog(self)


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
        package_root = find_package_root(obj)
        if package_root is None:
            return

        package = _selected_package(context)
        info = layout.box()
        info.label(text=f"Package: {package_root.get(PROP_PACKAGE_NAME, '')}")
        info.label(text=f"Entity: {obj.get(PROP_ENTITY_NAME, obj.name) if obj else ''}")
        info.label(text=f"Palette: {obj.get(PROP_PALETTE_ID, '') if obj else ''}")
        if obj is not None:
            material_sidecar = obj.get(PROP_MATERIAL_SIDECAR)
            if isinstance(material_sidecar, str) and material_sidecar:
                info.label(text=f"Sidecar: {Path(material_sidecar).name}")

        actions = layout.row(align=True)
        actions.operator(STARBREAKER_OT_apply_palette.bl_idname, icon="COLOR")
        actions.operator(STARBREAKER_OT_apply_livery.bl_idname, icon="MATERIAL")
        layout.operator(STARBREAKER_OT_dump_metadata.bl_idname, icon="TEXT")

        if package is not None:
            available = layout.box()
            available.label(text=f"Palettes: {', '.join(sorted(package.palettes.keys()))}")
            available.label(text=f"Liveries: {', '.join(sorted(package.liveries.keys()))}")

        if obj is not None and obj.active_material is not None:
            material = obj.active_material
            material_box = layout.box()
            material_box.label(text=f"Shader: {material.get(PROP_SHADER_FAMILY, '')}")
            material_box.label(text=f"Template: {material.get(PROP_TEMPLATE_KEY, '')}")


CLASSES = [
    STARBREAKER_OT_import_decomposed_package,
    STARBREAKER_OT_apply_palette,
    STARBREAKER_OT_apply_livery,
    STARBREAKER_OT_dump_metadata,
    STARBREAKER_PT_tools,
]


def register() -> None:
    for cls in CLASSES:
        bpy.utils.register_class(cls)


def unregister() -> None:
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)
