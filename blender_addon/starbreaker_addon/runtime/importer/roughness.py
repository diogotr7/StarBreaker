"""Shared roughness helpers for runtime material reconstruction.

DDNA alpha is treated as authored gloss/smoothness. The current approved VFL
hypothesis uses a perceptual invert so Blender's roughness input preserves the
correct endpoints while making the midrange rougher than a plain linear
invert.
"""

from __future__ import annotations

import math


def glossiness_to_perceptual_roughness(glossiness: float) -> float:
    """Convert a gloss/smoothness value into Blender perceptual roughness."""

    clamped = max(0.0, min(1.0, float(glossiness)))
    return math.sqrt(1.0 - clamped)


def combine_glossiness_signals(primary_glossiness: float, secondary_glossiness: float) -> float:
    """Combine two glossiness controls in gloss space.

    This preserves the old multiplicative intent in roughness-linear space:

        combined_linear_roughness = (1 - primary_gloss) * (1 - secondary_gloss)

    and then converts that back into a single glossiness signal so one final
    perceptual remap can be applied.
    """

    primary = max(0.0, min(1.0, float(primary_glossiness)))
    secondary = max(0.0, min(1.0, float(secondary_glossiness)))
    return 1.0 - ((1.0 - primary) * (1.0 - secondary))


def ddna_smoothness_to_roughness(smoothness: float) -> float:
    """Map DDNA smoothness into Blender roughness.

    CryEngine docs describe ``_ddna`` alpha as gloss. The live Clipper check
    rejected a capped legacy-style remap and the user confirmed that a
    perceptual invert better matches in-game appearance:

        roughness = sqrt(1 - glossiness)
    """

    return glossiness_to_perceptual_roughness(smoothness)


def ddna_and_palette_gloss_to_roughness(
    ddna_glossiness: float,
    palette_glossiness: float,
) -> float:
    """Combine DDNA gloss with palette gloss, then map once to roughness."""

    return glossiness_to_perceptual_roughness(
        combine_glossiness_signals(ddna_glossiness, palette_glossiness)
    )


def roughness_and_palette_gloss_to_roughness(
    roughness: float,
    palette_glossiness: float,
) -> float:
    """Combine an already-derived roughness value with palette glossiness."""

    clamped_roughness = max(0.0, min(1.0, float(roughness)))
    return clamped_roughness * glossiness_to_perceptual_roughness(palette_glossiness)
