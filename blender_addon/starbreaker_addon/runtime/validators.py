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
MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES: frozenset[str] = frozenset({
    "ShaderNodeOutputMaterial",
    "ShaderNodeTexImage",
    "ShaderNodeGroup",
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
    "StarBreaker Runtime LayerSurface.",
    "StarBreaker Runtime HardSurface.",
    "StarBreaker Runtime Glass.",
    "StarBreaker Runtime NoDraw.",
    "StarBreaker Runtime Screen.",
    "StarBreaker Runtime Effect.",
    "StarBreaker Runtime LayeredInputs.",
    "StarBreaker Runtime Principled.",
    "StarBreaker Runtime HardSurface Stencil.",
    "StarBreaker Runtime Channel Split.",
    "StarBreaker Runtime Smoothness To Roughness.",
    "StarBreaker Runtime Color To Luma.",
    "StarBreaker Runtime Shadowless Wrapper.",
    "StarBreaker Wear Input.",
    "StarBreaker Iridescence Input.",
)


def _purge_orphaned_runtime_groups() -> int:
    removed = 0
    for group in list(bpy.data.node_groups):
        if group.users > 0:
            continue
        name = group.name
        if any(name.startswith(prefix) for prefix in _RUNTIME_GROUP_PREFIXES):
            bpy.data.node_groups.remove(group)
            removed += 1
    return removed
