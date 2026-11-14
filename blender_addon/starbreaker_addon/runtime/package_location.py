"""Locate a package's StarBreaker export root at runtime, without stored paths.

Package roots carry ``starbreaker_scene_path`` relative to the export root (the
folder holding ``Packages/`` and the shared ``Data/`` tree) and never an
absolute export root: machine-specific paths are banned in exports and in the
user's .blend files. The export root is instead derived from Blender's own
library bookkeeping, which stays valid wherever the host .blend is saved
(Blender re-relativises library paths when a package is appended or the host
file moves).

Every linked scene-instance mesh object records ``starbreaker_mesh_asset``, the
export-root-relative path of the .blend its mesh data is linked from. Stripping
that suffix from the library's absolute path yields the export root exactly.

Public API:

- ``export_root_from_linked_meshes(package_root, abspath)`` — the single export
  root of a package root, or ``PackageLocationError`` when its linked meshes
  name none or several roots.
- ``PackageLocationError`` — raised for any root that cannot be located.

The module is bpy-free; callers pass ``bpy.path.abspath`` in.
"""

from __future__ import annotations

import os
from pathlib import Path, PurePath, PurePosixPath
from typing import Any, Callable

from .constants import PROP_MESH_ASSET


class PackageLocationError(RuntimeError):
    """The export root of a StarBreaker package cannot be determined."""


AbsPath = Callable[..., str]


def _library_abspath(library: Any, abspath: AbsPath) -> str:
    """Absolute path of a library; indirect libraries are relative to their parent."""

    parent = getattr(library, "parent", None)
    if parent is not None:
        return abspath(library.filepath, library=parent)
    return abspath(library.filepath)


def export_root_from_library_path(library_path: str, mesh_asset: str) -> Path:
    """Strip the export-root-relative ``mesh_asset`` suffix off ``library_path``."""

    library_parts = PurePath(os.path.normpath(library_path)).parts
    asset_parts = PurePosixPath(mesh_asset.replace("\\", "/")).parts
    count = len(asset_parts)
    suffix = library_parts[-count:] if 0 < count < len(library_parts) else ()
    if not suffix or os.path.normcase(os.path.join(*suffix)) != os.path.normcase(os.path.join(*asset_parts)):
        raise PackageLocationError(
            f"Linked library '{library_path}' does not end with its mesh asset '{mesh_asset}'; "
            "the export folder layout was changed after export"
        )
    return Path(*library_parts[:-count])


def export_root_from_linked_meshes(package_root: Any, abspath: AbsPath) -> Path:
    """Return the one export root the package root's linked meshes come from."""

    roots: dict[str, tuple[Path, str]] = {}
    for obj in (package_root, *getattr(package_root, "children_recursive", ())):
        library = getattr(getattr(obj, "data", None), "library", None)
        if library is None:
            continue
        mesh_asset = obj.get(PROP_MESH_ASSET)
        if not isinstance(mesh_asset, str) or not mesh_asset:
            continue
        library_path = _library_abspath(library, abspath)
        export_root = export_root_from_library_path(library_path, mesh_asset)
        roots.setdefault(os.path.normcase(str(export_root)), (export_root, os.path.normpath(library_path)))

    name = getattr(package_root, "name", "?")
    if not roots:
        raise PackageLocationError(
            f"Package '{name}' has no mesh linked from a StarBreaker export, so its export root "
            "(the folder holding Packages/ and Data/) cannot be located"
        )
    if len(roots) > 1:
        listed = "; ".join(f"{root} (e.g. {library})" for root, library in roots.values())
        raise PackageLocationError(
            f"Package '{name}' links meshes from more than one export root: {listed}. "
            "Relink its libraries so they all point into one export root"
        )
    return next(iter(roots.values()))[0]
