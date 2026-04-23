"""Shared dataclasses + constants for the importer mixins.

Extracted from ``runtime/_legacy.py`` as part of Phase 7.5g so the mixins
can import these types directly without pulling the full ``_legacy``
module (and the circular-import shim that went with it).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import bpy


@dataclass(frozen=True)
class ImportedTemplate:
    mesh_asset: str
    root_names: list[str]


@dataclass(frozen=True)
class MaterialNodeLayout:
    texture_x: float = -300.0
    texture_start_y: float = 160.0
    texture_vertical_step: float = 260.0
    texture_width: float = 300.0
    primary_x: float = 200.0
    primary_y: float = -120.0
    group_width: float = 460.0
    output_x: float = 780.0
    output_y: float = -120.0
    shadow_mix_x: float = 500.0
    shadow_mix_y: float = -120.0
    shadow_transparent_x: float = 260.0
    shadow_transparent_y: float = -300.0
    shadow_light_path_x: float = 260.0
    shadow_light_path_y: float = -480.0


MATERIAL_NODE_LAYOUT = MaterialNodeLayout()


@dataclass
class LayerSurfaceSockets:
    color: Any | None = None
    alpha: Any | None = None
    normal: Any | None = None
    roughness: Any | None = None
    specular: Any | None = None
    specular_tint: Any | None = None
    metallic: Any | None = None


@dataclass
class StencilOverlaySockets:
    color: Any | None = None
    color_factor: Any | None = None
    factor: Any | None = None
    roughness: Any | None = None
    specular: Any | None = None
    specular_tint: Any | None = None


@dataclass(frozen=True)
class SocketRef:
    node: Any
    name: str
    is_output: bool = True


BITANGENT_SIGN_ATTRIBUTE = "starbreaker_bitangent_sign"


def _bake_bitangent_sign_attribute(mesh: bpy.types.Mesh) -> bool:
    """Bake per-corner MikkTSpace bitangent sign into a float attribute.

    The POM ``tangent_space`` group multiplies its bitangent projection by
    this attribute to compensate for UV-mirrored regions, where a shared
    tangent direction produces an inverted bitangent. Meshes without a UV
    map, or without loops, are skipped silently.
    """
    if mesh is None or not getattr(mesh, "loops", None):
        return False
    if not mesh.uv_layers:
        return False
    try:
        mesh.calc_tangents()
    except Exception:
        return False
    existing = mesh.attributes.get(BITANGENT_SIGN_ATTRIBUTE)
    if existing is not None:
        mesh.attributes.remove(existing)
    attr = mesh.attributes.new(BITANGENT_SIGN_ATTRIBUTE, "FLOAT", "CORNER")
    for idx, loop in enumerate(mesh.loops):
        attr.data[idx].value = loop.bitangent_sign
    try:
        mesh.free_tangents()
    except Exception:
        pass
    return True
