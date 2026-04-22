"""Submaterial / layer / texture-reference record helpers.

Extracted in Phase 7.3. Pure functions over ``SubmaterialRecord``,
``LayerManifestEntry``, ``TextureReference``, and ``ContractInput``.
"""

from __future__ import annotations

from typing import Any

from ..manifest import (
    LayerManifestEntry,
    PaletteRecord,
    SubmaterialRecord,
    TextureReference,
)
from ..material_contract import ContractInput
from ..palette import palette_color


def _mean_triplet(value: tuple[float, float, float] | None) -> float | None:
    if value is None:
        return None
    return sum(value) / 3.0


def _triplet_from_value(value: Any) -> tuple[float, float, float] | None:
    if not isinstance(value, (list, tuple)) or len(value) < 3:
        return None
    try:
        return (float(value[0]), float(value[1]), float(value[2]))
    except (TypeError, ValueError):
        return None


def _triplet_from_string(value: Any) -> tuple[float, float, float] | None:
    if not isinstance(value, str):
        return None
    parts = [part.strip() for part in value.split(",")]
    if len(parts) < 3:
        return None
    try:
        return (float(parts[0]), float(parts[1]), float(parts[2]))
    except (TypeError, ValueError):
        return None


def _triplet_from_any(value: Any) -> tuple[float, float, float] | None:
    return _triplet_from_value(value) or _triplet_from_string(value)


def _optional_float_public_param(submaterial: SubmaterialRecord, *names: str) -> float | None:
    for name in names:
        value = submaterial.public_params.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return None


def _resolve_public_param_default(
    submaterial: SubmaterialRecord, lowercase_name: str
) -> float | tuple[float, float, float, float] | None:
    """Resolve an authored public param for a ``public_param_<name>`` semantic.

    The contract semantic is lowercased by convention; this helper matches it
    case-insensitively against the submaterial's public-params dict and returns
    a value suitable for assignment to a ``NodeSocketFloat.default_value`` or
    ``NodeSocketColor.default_value`` (RGBA tuple). Returns ``None`` when the
    param is absent or unparseable, so the socket retains its authored default.
    """
    if not lowercase_name:
        return None
    lookup = {key.lower(): value for key, value in submaterial.public_params.items()}
    raw = lookup.get(lowercase_name)
    if raw is None:
        return None
    try:
        return float(raw)
    except (TypeError, ValueError):
        pass
    triplet = _triplet_from_any(raw)
    if triplet is not None:
        return (triplet[0], triplet[1], triplet[2], 1.0)
    return None


def _authored_attribute_string(submaterial: SubmaterialRecord, *names: str) -> str | None:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        value = attribute.get("value")
        if isinstance(value, str) and value:
            return value
    return None


def _matching_texture_reference(
    textures: list[TextureReference],
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    for texture in textures:
        if slots and texture.slot not in slots:
            continue
        if roles and texture.role not in roles:
            continue
        if alpha_semantic is not None and texture.alpha_semantic != alpha_semantic:
            continue
        if texture.export_path:
            return texture
    for texture in textures:
        if slots and texture.slot not in slots:
            continue
        if roles and texture.role not in roles:
            continue
        if alpha_semantic is not None and texture.alpha_semantic != alpha_semantic:
            continue
        return texture
    return None


def _submaterial_texture_reference(
    submaterial: SubmaterialRecord,
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    return _matching_texture_reference(
        [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures],
        slots=slots,
        roles=roles,
        alpha_semantic=alpha_semantic,
    )


def _layer_texture_reference(
    layer: LayerManifestEntry,
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    return _matching_texture_reference(layer.texture_slots, slots=slots, roles=roles, alpha_semantic=alpha_semantic)


def _uses_virtual_tint_palette_decal(submaterial: SubmaterialRecord) -> bool:
    texture = _submaterial_texture_reference(submaterial, slots=("TexSlot7",), roles=("tint_palette_decal",))
    return texture is not None and bool(texture.is_virtual)


def _is_virtual_tint_palette_stencil_decal(submaterial: SubmaterialRecord) -> bool:
    if submaterial.shader_family != "MeshDecal" or not _uses_virtual_tint_palette_decal(submaterial):
        return False
    string_gen_mask = (_authored_attribute_string(submaterial, "StringGenMask") or "").upper()
    if "STENCIL_MAP" in string_gen_mask:
        return True
    if any(
        name in submaterial.public_params
        for name in ("StencilOpacity", "StencilDiffuseBreakup", "StencilTiling", "StencilTintOverride")
    ):
        return True
    lowered_name = (submaterial.submaterial_name or "").lower()
    return "_stencil" in lowered_name or "branding" in lowered_name


def _routes_virtual_tint_palette_decal_to_decal_source(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot1" and (contract_input.semantic or contract_input.name).lower() == "decal_source"


def _suppresses_virtual_tint_palette_stencil_input(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot7" and (contract_input.semantic or contract_input.name).lower() in {
        "stencil_source",
        "stencil_source_alpha",
    }


def _routes_virtual_tint_palette_decal_alpha_to_decal_source(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot1" and (contract_input.semantic or contract_input.name).lower() == "decal_source_alpha"


def _public_param_triplet(submaterial: SubmaterialRecord, *names: str) -> tuple[float, float, float] | None:
    for name in names:
        triplet = _triplet_from_any(submaterial.public_params.get(name))
        if triplet is not None:
            return triplet
    return None


def _authored_attribute_triplet(submaterial: SubmaterialRecord, *names: str) -> tuple[float, float, float] | None:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        triplet = _triplet_from_any(attribute.get("value"))
        if triplet is not None:
            return triplet
    return None


def _resolved_submaterial_palette_color(
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
) -> tuple[float, float, float] | None:
    if palette is None:
        return None
    channel = submaterial.palette_routing.material_channel
    if channel is not None:
        return palette_color(palette, channel.name)
    if submaterial.shader_family == "GlassPBR":
        return palette_color(palette, "glass")
    return None


def _float_layer_public_param(layer: LayerManifestEntry, *names: str) -> float:
    wanted = set(names)
    for param in layer.resolved_material.get("authored_public_params", []):
        if param.get("name") not in wanted:
            continue
        try:
            return float(param.get("value"))
        except (TypeError, ValueError):
            continue
    return 0.0


def _layer_snapshot_triplet(layer: LayerManifestEntry, name: str) -> tuple[float, float, float] | None:
    return _triplet_from_value(layer.layer_snapshot.get(name))


def _layer_snapshot_float(layer: LayerManifestEntry, name: str) -> float:
    value = layer.layer_snapshot.get(name)
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def _float_public_param(submaterial: SubmaterialRecord, *names: str) -> float:
    for name in names:
        value = submaterial.public_params.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return 0.0


def _float_authored_attribute(submaterial: SubmaterialRecord, *names: str) -> float:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        value = attribute.get("value")
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return 0.0


def _hard_surface_angle_shift_enabled(submaterial: SubmaterialRecord) -> bool:
    if submaterial.decoded_feature_flags.has_iridescence:
        return True
    strength = _optional_float_public_param(submaterial, "IridescenceStrength")
    if strength is None or strength <= 0.0:
        return False
    thickness_u = _optional_float_public_param(submaterial, "IridescenceThicknessU")
    thickness_v = _optional_float_public_param(submaterial, "IridescenceThicknessV")
    has_thickness = (thickness_u is not None and thickness_u > 0.0) or (thickness_v is not None and thickness_v > 0.0)
    has_support_texture = any(texture.slot == "TexSlot10" and bool(texture.export_path) for texture in submaterial.texture_slots)
    return has_thickness or has_support_texture
