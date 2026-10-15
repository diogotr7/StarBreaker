"""Texture image paths: store them host-relative, compare them by absolute file.

Machine-specific absolute paths are banned in the user's .blend files, so a
texture the material refresh loads is pointed at its ``//``-relative path from
the host .blend. Blender keeps such paths valid when the host is saved
elsewhere, and makes every path relative on the first save of an unsaved host.
A host on a different drive from the export root has no relative path to it;
the image keeps its absolute path then, as Blender does for its own data.

Because loaded images may now hold relative paths, code that matches an image
to a resolved file must compare normalised absolute paths (``same_image_file``)
rather than raw ``filepath`` strings.

Public API: ``host_relative_path`` (pure), ``image_file_path``,
``same_image_file``, ``store_host_relative``.
"""

from __future__ import annotations

import os
from typing import Any


def host_relative_path(path: str, host_blend: str) -> str | None:
    """``//``-relative form of ``path`` from the host .blend's folder, or None if there is none."""

    if not path or not host_blend:
        return None
    try:
        relative = os.path.relpath(path, os.path.dirname(host_blend))
    except ValueError:  # a different drive: no relative path exists
        return None
    return "//" + relative.replace(os.sep, "/")


def image_file_path(image: Any) -> str:
    """Normalised absolute path of an image's source file ('' when it has none)."""

    import bpy

    filepath = str(getattr(image, "filepath", "") or "")
    try:
        filepath = bpy.path.abspath(filepath, library=getattr(image, "library", None))
    except Exception:
        pass
    return os.path.normpath(filepath) if filepath else ""


def same_image_file(image: Any, path: str) -> bool:
    return os.path.normcase(image_file_path(image)) == os.path.normcase(os.path.normpath(path))


def store_host_relative(image: Any) -> None:
    """Point a local file image at its host-relative path when one exists."""

    if image is None or getattr(image, "library", None) is not None:
        return
    import bpy

    relative = host_relative_path(image_file_path(image), getattr(bpy.data, "filepath", ""))
    if relative is not None and getattr(image, "filepath_raw", None) != relative:
        image.filepath_raw = relative
