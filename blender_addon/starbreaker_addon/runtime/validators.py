"""Top-level material graph hygiene validators (Phase 6 rule).

Extracted from ``runtime.py`` in Phase 7. The Phase 6 contract is:

    A material's top-level node tree may contain only Material Output,
    Image Texture, and Node Group nodes. Everything else belongs inside an
    owned group.

See ``docs/StarBreaker/todo.md`` Phase 6 for the authoritative definition.
"""

from __future__ import annotations

import bpy


#: Block bl_idnames allowed at the top level of any managed material.
#:
#: Image Texture nodes stay at top level so palette / livery switching
#: can locate and re-wire them. Mapping + Texture Coordinate nodes are
#: allowed at top level because they are orchestration for those image
#: textures (UV scale / offset from public-param tiling values), not
#: shader logic. All BSDFs, math, mixes, etc. belong inside a Node Group.
MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES: frozenset[str] = frozenset({
    "ShaderNodeOutputMaterial",
    "ShaderNodeTexImage",
    "ShaderNodeGroup",
    "ShaderNodeMapping",
    "ShaderNodeTexCoord",
})


def _material_top_level_violations(
    material: "bpy.types.Material",
    *,
    extra_allowed: frozenset[str] | None = None,
) -> list[tuple[str, str]]:
    """Return ``[(node_name, bl_idname), ...]`` for nodes that break the Phase 6 rule.

    Callers that still have known-deferred helper types at top level can pass
    them through ``extra_allowed`` to silence those during targeted validation.
    """
    if material is None or material.node_tree is None:
        return []
    allowed = MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES
    if extra_allowed:
        allowed = allowed | extra_allowed
    return [
        (node.name, node.bl_idname)
        for node in material.node_tree.nodes
        if node.bl_idname not in allowed
    ]


def _assert_material_top_level_clean(
    material: "bpy.types.Material",
    *,
    extra_allowed: frozenset[str] | None = None,
) -> None:
    """Raise ``AssertionError`` when ``material``'s top level violates Phase 6."""
    violations = _material_top_level_violations(material, extra_allowed=extra_allowed)
    if not violations:
        return
    detail = ", ".join(f"{name}:{bl_idname}" for name, bl_idname in violations)
    raise AssertionError(
        f"Material {material.name!r} violates top-level hygiene: {detail}"
    )


#: Name prefixes of runtime-owned node groups. Used by
#: :func:`_purge_orphaned_runtime_groups` to skip non-starbreaker groups.
_RUNTIME_GROUP_PREFIXES: tuple[str, ...] = (
    "StarBreaker Runtime LayerSurface",
    "StarBreaker Runtime HardSurface",
    "StarBreaker Runtime Glass",
    "StarBreaker Runtime NoDraw",
    "StarBreaker Runtime Screen",
    "StarBreaker Runtime Effect",
    "StarBreaker Runtime LayeredInputs",
    "StarBreaker Runtime Principled",
    "StarBreaker Runtime HardSurface Stencil",
    "StarBreaker Runtime Channel Split",
    "StarBreaker Runtime Smoothness To Roughness",
    "StarBreaker Runtime Color To Luma",
    "StarBreaker Runtime POM Detail",
    "StarBreaker Runtime Shadowless Wrapper",
    "StarBreaker Wear Input",
    "StarBreaker Iridescence Input",
    # Phase 12 POM: appended copies of the production POM pipeline
    # from resources/pom_library.blend. Each POM-flagged material
    # triggers a unique copy (keyed by height-image name) via
    # ``_ensure_runtime_parallax_group``; purging orphans here keeps
    # the blend file clean when those materials are rebuilt onto a
    # different paint or removed.
    "StarBreaker POM [",
)


#: ID-property stamps the addon writes onto every image datablock it loads.
#: Their presence is what marks an image as addon-owned and therefore safe to
#: purge when it falls to zero users.
_ADDON_IMAGE_STAMPS: tuple[str, ...] = (
    "starbreaker_source_mtime_ns",
    "starbreaker_gobo_mean_luminance",
)


def _batch_remove(ids: list) -> int:
    """Drop ``ids`` in one ``bpy.data.batch_remove`` call.

    Per-datablock ``.remove()`` re-runs Blender's internal user-remap
    bookkeeping every time; ``batch_remove`` does it once for the whole set.
    """
    if not ids:
        return 0
    bpy.data.batch_remove(ids=ids)
    return len(ids)


def _purge_orphaned_runtime_groups() -> int:
    return _batch_remove([
        group
        for group in bpy.data.node_groups
        if group.users == 0
        and any(group.name.startswith(prefix) for prefix in _RUNTIME_GROUP_PREFIXES)
    ])


def _purge_orphaned_managed_materials() -> int:
    return _batch_remove([
        material
        for material in bpy.data.materials
        if material.users == 0 and material.get("starbreaker_material_identity")
    ])


def _purge_orphaned_runtime_actions() -> int:
    return _batch_remove([
        action
        for action in bpy.data.actions
        if action.users == 0 and action.name.startswith("HeightMapAction")
    ])


def _purge_orphaned_file_backed_images() -> int:
    """Remove ``bpy.data.images`` datablocks with no users that were loaded
    from an on-disk file.

    After the per-material
    :meth:`~starbreaker_addon.runtime.importer.builders.BuildersMixin._sweep_unreachable_nodes`
    pass, any :class:`ShaderNodeTexImage` nodes that were never wired into
    a Material Output have been dropped. The images they referenced may
    have fallen to ``users == 0`` as a result; this helper frees their
    pixel data (and, transitively, the VRAM Cycles would have uploaded
    for them).

    Only file-backed images the addon itself loaded are considered — every
    such load stamps one of :data:`_ADDON_IMAGE_STAMPS` (see
    ``MaterialsMixin._refresh_image_source`` and the gobo loader in
    ``orchestration``). Without that restriction this purged *any* zero-user
    file-backed image, including ones the user had loaded by hand and merely
    left unassigned. Stampless datablocks like ``Render Result`` and
    ``Viewer Node`` (which always have ``users == 0``) are likewise left alone.
    """
    return _batch_remove([
        image
        for image in bpy.data.images
        if image.users == 0
        and image.filepath
        and any(image.get(stamp) is not None for stamp in _ADDON_IMAGE_STAMPS)
    ])
