"""HumanSkin_V2 parameter resolution (no ``bpy``).

Resolves everything the skin node graph needs from a submaterial record and,
for CHF character packages, the root instance's ``character`` metadata
(``scene.json`` → ``root_entity.character``, written by
``starbreaker chf to-package``):

* shader constants from the ``.mtl`` (``MatDifColor``, ``MatSpecColor`` /
  ``Shininess``, ``SourceAverageColor``, ``FinalSkinTone``, ``SourceDetail``,
  ``TranslucencyMultiplier``, ``MicroDetailTiling``),
* CHF overrides for the matching customized submaterial: ``BodyColor``
  (sRGB) → ``FinalSkinTone`` (linear), blemish textures (``TexSlot5`` mask,
  ``TexSlot10`` ID) and ``Freckles*`` / ``SunSpots*`` params,
* the constant terms of the ``%TONE_ADJUSTMENT`` rebase
  (:func:`tone_constants`),
* the ``%WRINKLE_BLENDING_2`` slot layout.

Ground truth: ``HumanSkin_V2.cfxb`` ``GetMaterialAttribs`` decoded from
``Engine/ShaderCache_D3D11.pak``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from ..manifest import SubmaterialRecord
from .record_utils import _optional_float_public_param, _triplet_from_any

HUMAN_SKIN_FAMILY = "HumanSkin_V2"

# CryEngine GetLuminance (Rec. 709).
LUMINANCE = (0.2126, 0.7152, 0.0722)


def srgb_to_linear(value: float) -> float:
    if value <= 0.04045:
        return value / 12.92
    return ((value + 0.055) / 1.055) ** 2.4


def luminance(rgb: tuple[float, float, float]) -> float:
    return sum(c * w for c, w in zip(rgb, LUMINANCE))


def tone_constants(
    source_average: tuple[float, float, float],
    final_tone: tuple[float, float, float],
) -> tuple[float, float]:
    """Pixel-independent terms of the ``%TONE_ADJUSTMENT`` rebase.

    Returns ``(detail_scale, gb_desaturate)``: the factor ``detailAmount`` is
    multiplied by (from ``colDelta2 = lum(FinalSkinTone) - lum(SourceAverageColor)``)
    and the ``details.gb`` desaturation lerp factor
    ``saturate(4 * |SourceAverageColor.g - FinalSkinTone.g|)``.
    """
    delta = luminance(final_tone) - luminance(source_average)
    if delta < 0:
        detail_scale = 1.0 + min(1.0, 2.0 * abs(delta))
    else:
        detail_scale = 1.0 / (1.0 + min(1.0, delta))
    gb_desaturate = min(1.0, 4.0 * abs(source_average[1] - final_tone[1]))
    return detail_scale, gb_desaturate


@dataclass(frozen=True)
class WrinkleSlot:
    panel: int
    channel: int
    band: int
    tile: int


@dataclass
class HumanSkinParams:
    diffuse_color: tuple[float, float, float] = (1.0, 1.0, 1.0)
    specular_color: tuple[float, float, float] = (0.04, 0.04, 0.04)
    smoothness_scale: float = 1.0
    source_average: tuple[float, float, float] = (0.5, 0.5, 0.5)
    final_tone: tuple[float, float, float] = (0.5, 0.5, 0.5)
    source_detail: float = 1.0
    translucency: float = 0.0
    micro_detail_tiling: float = 1.0
    tone_adjustment: bool = False
    wrinkle_blending: bool = False
    freckles_amount: float = 0.5
    freckles_opacity: float = 0.0
    sunspots_amount: float = 0.5
    sunspots_opacity: float = 0.0
    # package-relative PNG paths for CHF override textures (TexSlotN → path)
    override_textures: dict[str, str] = field(default_factory=dict)
    wrinkle_slots: list[WrinkleSlot] = field(default_factory=list)

    @property
    def blemishes(self) -> bool:
        return (
            self.freckles_opacity + self.sunspots_opacity > 0.0
            and "TexSlot5" in self.override_textures
            and "TexSlot10" in self.override_textures
        )


def _authored_triplet(submaterial: SubmaterialRecord, name: str) -> tuple[float, float, float] | None:
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") == name:
            return _triplet_from_any(attribute.get("value"))
    return None


def _authored_float(submaterial: SubmaterialRecord, name: str) -> float | None:
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") == name:
            try:
                return float(attribute.get("value"))
            except (TypeError, ValueError):
                return None
    return None


def _normalized_material_path(path: str | None) -> str:
    value = (path or "").replace("\\", "/").lower()
    return value[5:] if value.startswith("data/") else value


def chf_submaterial_overrides(
    character: dict[str, Any] | None,
    source_material_path: str | None,
    submaterial_name: str,
) -> dict[str, Any] | None:
    """The CHF customization entry for this ``.mtl`` + submaterial, if any.

    Matched structurally: the customized material's resolved ``mtl_path``
    equals the sidecar's source ``.mtl`` and the submaterial names agree.
    """
    if not isinstance(character, dict):
        return None
    wanted = _normalized_material_path(source_material_path)
    for material in character.get("customization") or []:
        if _normalized_material_path(material.get("mtl_path")) != wanted:
            continue
        for sub in material.get("submaterials") or []:
            if str(sub.get("name", "")).lower() == submaterial_name.lower():
                return sub
    return None


def human_skin_params(
    submaterial: SubmaterialRecord,
    source_material_path: str | None,
    character: dict[str, Any] | None,
) -> HumanSkinParams:
    params = HumanSkinParams()
    params.diffuse_color = _authored_triplet(submaterial, "Diffuse") or params.diffuse_color
    params.specular_color = _authored_triplet(submaterial, "Specular") or params.specular_color
    shininess = _authored_float(submaterial, "Shininess")
    if shininess is not None:
        params.smoothness_scale = max(0.0, min(1.0, shininess / 255.0))

    def triplet(name: str, default: tuple[float, float, float]) -> tuple[float, float, float]:
        return _triplet_from_any(submaterial.public_params.get(name)) or default

    def scalar(name: str, default: float) -> float:
        value = _optional_float_public_param(submaterial, name)
        return default if value is None else value

    params.source_average = triplet("SourceAverageColor", params.source_average)
    params.final_tone = triplet("FinalSkinTone", params.final_tone)
    params.source_detail = scalar("SourceDetail", params.source_detail)
    params.translucency = scalar("TranslucencyMultiplier", params.translucency)
    params.micro_detail_tiling = scalar("MicroDetailTiling", params.micro_detail_tiling)
    tokens = set(submaterial.decoded_feature_flags.tokens)
    params.tone_adjustment = "TONE_ADJUSTMENT" in tokens
    params.wrinkle_blending = "WRINKLE_BLENDING_2" in tokens

    override = chf_submaterial_overrides(character, source_material_path, submaterial.submaterial_name)
    if override is not None:
        body = (override.get("colors") or {}).get("BodyColor")
        if isinstance(body, (list, tuple)) and len(body) >= 3:
            params.final_tone = tuple(srgb_to_linear(float(c)) for c in body[:3])
        chf_params = override.get("params") or {}
        params.freckles_amount = float(chf_params.get("FrecklesAmount", params.freckles_amount))
        params.freckles_opacity = float(chf_params.get("FrecklesOpacity", params.freckles_opacity))
        params.sunspots_amount = float(chf_params.get("SunSpotsAmount", params.sunspots_amount))
        params.sunspots_opacity = float(chf_params.get("SunSpotsOpacity", params.sunspots_opacity))
        params.override_textures = {
            str(slot): str(path) for slot, path in (override.get("texture_exports") or {}).items()
        }

    if params.wrinkle_blending and isinstance(character, dict):
        slots = (character.get("wrinkles") or {}).get("slots") or []
        params.wrinkle_slots = [
            WrinkleSlot(int(s["panel"]), int(s["channel"]), int(s["band"]), int(s["tile"])) for s in slots
        ]
    return params
