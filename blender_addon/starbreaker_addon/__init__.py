bl_info = {
    "name": "StarBreaker Decomposed Import",
    "author": "GitHub Copilot",
    "version": (0, 2, 2),
    "blender": (4, 2, 0),
    "location": "View3D > Sidebar > StarBreaker",
    "description": "Import StarBreaker decomposed export packages and rebuild template-driven materials",
    "category": "Import-Export",
}

# Installer-facing semantic version string.
VERSION = "0.2.2+addon.7"

try:
    from .ui import register, unregister
except ModuleNotFoundError as exc:
    if exc.name != "bpy":
        raise

    def register() -> None:
        raise RuntimeError("The StarBreaker Blender add-on can only be registered inside Blender")

    def unregister() -> None:
        return None


__all__ = ["bl_info", "VERSION", "register", "unregister"]