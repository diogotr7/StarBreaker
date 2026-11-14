"""Append a generated ``Packages/<pkg>/scene.blend`` into the current scene.

A package's scene.blend holds one top-level collection per package (the
package root object plus its instances, with child collections such as the
interior); mesh data stays linked from the shared ``Data/`` libraries. Appending
those collections again gives an independent copy of the package (Blender
suffixes names with ``.001``) that shares the linked mesh data.

Library paths are appended relative to the host .blend so the user's file
never records a machine-specific path; the export root is found again at run
time from those libraries (see ``package_location``).

Public API:

- ``package_collections(collections)`` — top-level collections that hold a
  package root (pure; raises ``PackageAppendError`` when there are none).
- ``package_roots(collections)`` — the package root objects inside them.
- ``append_package(context, blend_path)`` — append into the active collection
  and return the new package roots (requires bpy).
"""

from __future__ import annotations

from typing import Any, Iterable

from .constants import PROP_PACKAGE_ROOT


class PackageAppendError(RuntimeError):
    """The chosen .blend is not a StarBreaker package scene."""


def _key(collection: Any) -> int:
    # bpy hands out a new Python wrapper per access, so compare datablock pointers.
    as_pointer = getattr(collection, "as_pointer", None)
    return as_pointer() if callable(as_pointer) else id(collection)


def _is_package_root(obj: Any) -> bool:
    return bool(obj.get(PROP_PACKAGE_ROOT))


def package_collections(collections: Iterable[Any]) -> list[Any]:
    """Collections that hold a package root and are not nested in another one given."""

    collections = list(collections)
    nested = {_key(child) for collection in collections for child in collection.children}
    selected = [
        collection
        for collection in collections
        if _key(collection) not in nested and any(_is_package_root(obj) for obj in collection.all_objects)
    ]
    if not selected:
        names = ", ".join(collection.name for collection in collections) or "none"
        raise PackageAppendError(f"No StarBreaker package collection found (collections: {names})")
    return selected


def package_roots(collections: Iterable[Any]) -> list[Any]:
    return [obj for collection in collections for obj in collection.all_objects if _is_package_root(obj)]


def _descendant_collections(collections: Iterable[Any]) -> set[int]:
    seen: set[int] = set()
    stack = list(collections)
    while stack:
        collection = stack.pop()
        if _key(collection) in seen:
            continue
        seen.add(_key(collection))
        stack.extend(collection.children)
    return seen


def append_package(context: Any, blend_path: str) -> list[Any]:
    """Append the package collections of ``blend_path``; return their package roots."""

    import bpy

    # relative=True only takes effect once the host has a filepath; an unsaved
    # host gets every path made relative when it is first saved.
    with bpy.data.libraries.load(blend_path, link=False, relative=True) as (data_from, data_to):
        data_to.collections = list(data_from.collections)
    loaded = [collection for collection in data_to.collections if collection is not None]

    try:
        selected = package_collections(loaded)
    except PackageAppendError:
        for collection in loaded:
            bpy.data.collections.remove(collection)
        raise

    kept = _descendant_collections(selected)
    for collection in loaded:
        if _key(collection) not in kept:
            bpy.data.collections.remove(collection)

    layer_collection = getattr(context.view_layer, "active_layer_collection", None)
    target = layer_collection.collection if layer_collection is not None else context.scene.collection
    for collection in selected:
        target.children.link(collection)
    return package_roots(selected)
