"""Palette-centric helpers pulled out of the monolithic runtime module.

Extracted in Phase 7.3. Pure functions over ``PaletteRecord``.
"""

from __future__ import annotations

import hashlib
import json
import math

from ..manifest import PaletteRecord
from ..palette import (
    palette_color,
    palette_decal_color,
    palette_decal_texture,
    palette_finish_glossiness,
    palette_finish_specular,
)


def _palette_decal_or_fallback(
    palette: PaletteRecord | None,
    decal_channel: str,
) -> tuple[float, float, float]:
    return palette_decal_color(palette, decal_channel) or (1.0, 1.0, 1.0)


def _palette_channel_has_iridescence(
    palette: PaletteRecord | None,
    channel_name: str,
) -> bool:
    if palette is None:
        return False
    channel = channel_name.lower()
    if channel not in {"primary", "secondary", "tertiary"}:
        return False
    facing = palette_finish_specular(palette, channel)
    if facing is None:
        return False
    grazing = palette_color(palette, channel)
    specular_chroma = max(facing) - min(facing)
    if specular_chroma < 0.10:
        return False
    color_distance = math.sqrt(
        (facing[0] - grazing[0]) ** 2
        + (facing[1] - grazing[1]) ** 2
        + (facing[2] - grazing[2]) ** 2
    )
    return color_distance >= 0.12


def _palette_has_iridescence(palette: PaletteRecord | None) -> bool:
    """Return True when a palette's tertiary finish encodes visible angle-shift iridescence.

    Runtime hard-surface iridescence uses the tertiary finish specular as the
    facing endpoint and the tertiary base color as the grazing endpoint. Treat a
    palette as iridescent when that tertiary specular is visibly chromatic and
    meaningfully separated from the tertiary base color.
    """
    return any(
        _palette_channel_has_iridescence(palette, channel_name)
        for channel_name in ("primary", "secondary", "tertiary")
    )


def _hard_surface_palette_iridescence_channel(
    palette: PaletteRecord | None,
    material_channel: str | None,
    *,
    authored_angle_shift: bool,
) -> str | None:
    """Pick the palette channel that should enable HardSurface iridescence.

    Ordinary palette-routed paints should only turn iridescence on when the
    material's own routed palette channel is visibly angle-shift. A tertiary
    fallback is reserved for explicitly authored angle-shift materials so a
    neutral primary paint inside an iridescent-capable palette does not become
    metallic by accident.
    """

    normalized_channel = str(material_channel or "").lower()
    if normalized_channel in {"primary", "secondary", "tertiary"}:
        if _palette_channel_has_iridescence(palette, normalized_channel):
            return normalized_channel
    if authored_angle_shift and _palette_channel_has_iridescence(palette, "tertiary"):
        return "tertiary"
    return None


def _palette_group_signature(palette: PaletteRecord) -> str:
    payload = {
        'schema': 'palette_group_v5',
        'id': palette.id,
        'primary': palette_color(palette, 'primary'),
        'secondary': palette_color(palette, 'secondary'),
        'tertiary': palette_color(palette, 'tertiary'),
        'glass': palette_color(palette, 'glass'),
        'decal_red': palette_decal_color(palette, 'red'),
        'decal_green': palette_decal_color(palette, 'green'),
        'decal_blue': palette_decal_color(palette, 'blue'),
        'decal_texture': palette_decal_texture(palette),
        'primary_spec': palette_finish_specular(palette, 'primary'),
        'secondary_spec': palette_finish_specular(palette, 'secondary'),
        'tertiary_spec': palette_finish_specular(palette, 'tertiary'),
        'primary_gloss': palette_finish_glossiness(palette, 'primary'),
        'secondary_gloss': palette_finish_glossiness(palette, 'secondary'),
        'tertiary_gloss': palette_finish_glossiness(palette, 'tertiary'),
    }
    return hashlib.sha1(json.dumps(payload, sort_keys=True).encode('utf-8')).hexdigest()
