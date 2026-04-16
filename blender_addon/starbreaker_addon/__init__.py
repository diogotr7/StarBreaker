bl_info = {
    "name": "StarBreaker Decomposed Import",
    "author": "GitHub Copilot",
    "version": (0, 1, 0),
    "blender": (4, 2, 0),
    "location": "View3D > Sidebar > StarBreaker",
    "description": "Import StarBreaker decomposed export packages and rebuild template-driven materials",
    "category": "Import-Export",
}

try:
    from .ui import register, unregister
except ModuleNotFoundError as exc:
    if exc.name != "bpy":
        raise

    def register() -> None:
        raise RuntimeError("The StarBreaker Blender add-on can only be registered inside Blender")

    def unregister() -> None:
        return None


__all__ = ["bl_info", "register", "unregister"]