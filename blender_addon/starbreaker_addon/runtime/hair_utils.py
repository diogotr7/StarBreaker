"""HairPBR parameter resolution (no ``bpy``), per ``HairPBR.cfxb``.

``%HAIR_CAP`` (scalp / brow-base caps under hair) reads ``$Diffuse``
(TexSlot1) as a ``ColorMap`` with coverage in **alpha**:
``hairColor = clamp(ColorMap.rgb · BaseTintColor · MatDifColor, 0.0001, 0.99)``,
``hairCapAlpha = ColorMap.a · (1 − (1 − saturate(N·V))^16)``, shaded
diffuse-only — a thin, usually near-black root layer over the skin.

Fibres (``%HAIR_CARDS`` / ``%HAIR_COAT``) have no colour texture: colour is
``GetHairColor(melanin, BaseMelaninRedness) · BaseTintColor`` (an absorption
model, :func:`hair_color`) with per-strand melanin from the ID map
(``$Specular`` = TexSlot4 ``.x``), then the dye layer. :func:`hair_params`
resolves the ``.mtl`` values over the shader defaults and the CHF dye slot
bound to this submaterial (``applies_to``; colours are sRGB there).
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any

from ..manifest import SubmaterialRecord
from .record_utils import _optional_float_public_param, _triplet_from_any
from .skin_utils import srgb_to_linear

HAIR_FAMILY = "HairPBR"
HAIR_CAP_TOKEN = "HAIR_CAP"
HAIR_FIBRE_TOKENS = ("HAIR_CARDS", "HAIR_COAT")

# GetHairColor absorption constants.
_SIGMA_EU = tuple(c * 1.2 for c in (0.419, 0.697, 1.37))
_SIGMA_PHEO = tuple((c * 2.4) ** 1.35 for c in (0.187, 0.4, 1.05))
_COLOR_BETA = 0.3

_SMOOTHNESS_DEFAULT = 0.65  # HairPBR.cfxb `Smoothness` default
_BASE_TINT_DEFAULT = (1.0, 1.0, 1.0)


@dataclass(frozen=True)
class HairCapParams:
    color_scale: tuple[float, float, float]  # BaseTintColor · MatDifColor
    color_min: float = 0.0001
    color_max: float = 0.99
    grazing_exponent: float = 16.0


def is_hair_cap(submaterial: SubmaterialRecord) -> bool:
    return submaterial.shader_family == HAIR_FAMILY and HAIR_CAP_TOKEN in submaterial.decoded_feature_flags.tokens


def _authored_diffuse(submaterial: SubmaterialRecord) -> tuple[float, float, float]:
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") == "Diffuse":
            return _triplet_from_any(attribute.get("value")) or (1.0, 1.0, 1.0)
    return (1.0, 1.0, 1.0)


def hair_cap_params(submaterial: SubmaterialRecord) -> HairCapParams:
    tint = _triplet_from_any(submaterial.public_params.get("BaseTintColor")) or _BASE_TINT_DEFAULT
    diffuse = _authored_diffuse(submaterial)
    return HairCapParams(color_scale=tuple(t * d for t, d in zip(tint, diffuse)))


def _absorption_denominator(beta: float) -> float:
    return ((((0.245 * beta + 5.574) * beta - 10.73) * beta + 2.532) * beta - 0.215) * beta + 5.969


def melanin_coefficients(redness: float) -> tuple[float, float, float]:
    """``k`` with ``GetHairColor(m, redness) = exp(-sqrt(q) · k)``, ``q = -ln(max(1 - m², 1e-4))``."""
    denom = _absorption_denominator(_COLOR_BETA)
    return tuple(
        math.sqrt((1.0 - redness) * eu + redness * pheo) * denom for eu, pheo in zip(_SIGMA_EU, _SIGMA_PHEO)
    )


def melanin_quantity(melanin: float) -> float:
    return -math.log(max(1.0 - melanin * melanin, 0.0001))


def hair_color(melanin: float, redness: float) -> tuple[float, float, float]:
    """``GetHairColor`` (reference implementation of the node graph)."""
    root = math.sqrt(melanin_quantity(melanin))
    return tuple(math.exp(-root * k) for k in melanin_coefficients(redness))


def is_hair_fibre(submaterial: SubmaterialRecord) -> bool:
    tokens = submaterial.decoded_feature_flags.tokens
    return (
        submaterial.shader_family == HAIR_FAMILY
        and HAIR_CAP_TOKEN not in tokens
        and any(t in tokens for t in HAIR_FIBRE_TOKENS)
    )


@dataclass(frozen=True)
class HairParams:
    cards: bool  # %HAIR_CARDS (else %HAIR_COAT)
    id_map: bool  # %CARD_ID_MAP
    melanin: float
    melanin_redness: float
    melanin_variation: float
    tint: tuple[float, float, float]
    dye_amount: float
    dye_color: tuple[float, float, float]
    dye_fadeout: float
    dye_pigment_variation: float
    dye_shift: float
    smoothness: float
    specular_variation: float
    specular_root_occlusion: float
    ambient_occlusion: float
    opacity_gamma: float
    diffuse: tuple[float, float, float]


def _normalized_mtl(path: str | None) -> str:
    value = (path or "").replace("\\", "/").lower()
    return value[5:] if value.startswith("data/") else value


def bound_chf_override(
    character: dict[str, Any] | None,
    source_material_path: str | None,
    submaterial_index: int,
) -> dict[str, Any] | None:
    """The CHF customization bound (``applies_to``) to this submaterial."""
    if not isinstance(character, dict):
        return None
    wanted = _normalized_mtl(source_material_path)
    for material in character.get("customization") or []:
        target = material.get("applies_to") or {}
        if _normalized_mtl(target.get("mtl_path")) != wanted or int(target.get("submaterial_index", -1)) != submaterial_index:
            continue
        subs = material.get("submaterials") or []
        return subs[0] if subs else None
    return None


def hair_params(
    submaterial: SubmaterialRecord,
    source_material_path: str | None,
    character: dict[str, Any] | None,
) -> HairParams:
    tokens = submaterial.decoded_feature_flags.tokens
    override = bound_chf_override(character, source_material_path, submaterial.index) or {}
    chf_params = override.get("params") or {}
    chf_colors = override.get("colors") or {}

    def scalar(name: str, default: float) -> float:
        if name in chf_params:
            return float(chf_params[name])
        value = _optional_float_public_param(submaterial, name)
        return default if value is None else value

    def colour(name: str, default: tuple[float, float, float]) -> tuple[float, float, float]:
        chf = chf_colors.get(name)
        if isinstance(chf, (list, tuple)) and len(chf) >= 3:
            return tuple(srgb_to_linear(float(c)) for c in chf[:3])
        return _triplet_from_any(submaterial.public_params.get(name)) or default

    return HairParams(
        cards="HAIR_CARDS" in tokens,
        id_map="CARD_ID_MAP" in tokens,
        melanin=scalar("BaseMelanin", 0.5),
        melanin_redness=scalar("BaseMelaninRedness", 0.5),
        melanin_variation=scalar("BaseMelaninVariation", 0.25),
        tint=colour("BaseTintColor", (1.0, 1.0, 1.0)),
        dye_amount=scalar("DyeAmount", 0.0),
        dye_color=colour("DyeColor", (1.0, 1.0, 1.0)),
        dye_fadeout=scalar("DyeFadeout", 0.0),
        dye_pigment_variation=scalar("DyePigmentVariation", 0.0),
        dye_shift=scalar("DyeShift", 0.0),
        smoothness=scalar("Smoothness", _SMOOTHNESS_DEFAULT),
        specular_variation=scalar("SpecularVariation", 0.25),
        specular_root_occlusion=scalar("SpecularRootOcclusion", 0.0),
        ambient_occlusion=scalar("AmbientOcclusion", 2.0),
        opacity_gamma=scalar("OpacityMapGamma", 1.0),
        diffuse=_authored_diffuse(submaterial),
    )


# HairShading constants (HairPBR.cfxb).
_HAIR_ALPHA = math.radians(2.5)  # cuticle tilt `Alpha`
_HAIR_F0 = 0.045  # Hair_F
_HAIR_BACKLIGHTING = 0.1
_HAIR_GLINT = 0.05  # base glintStrength (the cards' noise sparkle is not modelled)


@dataclass(frozen=True)
class HairLobe:
    """One HairShading lobe as a Cycles Hair BSDF.

    ``energy`` integrates the shader's lobe at peak alignment
    (``∫ Hair_g dθ = π · e^Dp`` times its azimuthal / Fresnel factors);
    ``offset`` is the longitudinal shift (radians); the Gaussian width is
    ``roughness² · roughness_scale``; ``tint`` is ``none`` / ``color``
    (hairColor) / ``sqrt`` (hairColor^0.5).
    """

    name: str
    energy: float
    offset: float
    roughness_scale: float
    tint: str
    transmission: bool


def hair_lobes() -> list[HairLobe]:
    absorbed = (1.0 - _HAIR_F0) ** 2
    return [
        # R: Mp(β=r², θ−2α) · cosHalfPhi·(0.2+glint) · F; × reflectance
        HairLobe("R", math.pi * (0.2 + _HAIR_GLINT) * _HAIR_F0, -2.0 * _HAIR_ALPHA, 1.0, "none", False),
        # TT: Mp_Np(β=r²/2, θ+α, Dp=−3.65cosφ−3.98) · hc^(0.5/cosθ) · (1−f)² · Backlighting
        HairLobe("TT", math.pi * math.exp(3.65 - 3.98) * absorbed * _HAIR_BACKLIGHTING, _HAIR_ALPHA, 0.5, "sqrt", True),
        # TRT: Mp_Np(β=2r², θ+4α, Dp=17cosφ/2−16.78) · (0.8+4·glint) · hc^(1/cosθ) · (1−f)²·f
        HairLobe(
            "TRT",
            math.pi * math.exp(17.0 - 16.78) * (0.8 + 4.0 * _HAIR_GLINT) * absorbed * _HAIR_F0,
            4.0 * _HAIR_ALPHA,
            2.0,
            "color",
            False,
        ),
    ]
