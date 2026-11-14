"""HardSurface wear, as Star Engine computes it (bpy-free).

Sources (decoded engine shaders):

- ``HardSurface.cfxb`` ``GetLayerAttribs``: vertex wear is the vertex colour
  alpha (``VertexData_FromVertColor``), scaled by ``CalculateVertexWear``
  (x 0.95); ``wearAmt = saturate(max(WearBlendBase, instWear) - vertexWear)``.
- ``CalcAlphas_BlendBased``: the primary layer's blend map (``Layer.cfxb``
  ``LayerBlendSampler`` = TexSlot8, sampled at the layer tiling) thresholds
  ``wearAmt`` via ``CalcBlendAlpha_FromLayer`` with the layer's
  ``WearDirtBlendFalloff`` as softness, then ``Layer_RemapWear`` for Paint,
  ClearCoat Paint and Cloth shading models.

``engine_wear_alpha`` / ``remap_wear`` / ``wear_amount`` are the reference
implementation the node group is tested against.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

VERTEX_WEAR_SCALE = 0.95  # layerLib CalculateVertexWear
_MAX_SLOPE_RATIO = 1.0 / 255.0  # layerLib CalcBlendInAlpha

# Layer.cfxb parameter defaults, used when a layer does not author them.
LAYER_DEFAULT_WEAR_FALLOFF = 0.5
LAYER_DEFAULT_SHADING_MODEL = 0

# layerLib SHADING_* and layerLibConstants remap starts.
SHADING_PAINT = 2
SHADING_CLEARCOAT_PAINT = 3
SHADING_CLOTH = 4
PAINT_PRIMER_START = 0.5
CLOTH_COMPLEMENT_START = 0.75


def _saturate(value: float) -> float:
    return max(0.0, min(1.0, value))


def wear_amount(wear_blend_base: float, instance_wear: float, vertex_alpha: float) -> float:
    return _saturate(max(wear_blend_base, instance_wear) - vertex_alpha * VERTEX_WEAR_SCALE)


def engine_wear_alpha(blend_map: float, amount: float, softness: float) -> float:
    """layerLib ``CalcBlendAlpha``: ``1 - InAlpha * OutAlpha``."""
    remapped = blend_map * (1.0 - _MAX_SLOPE_RATIO - 0.001) + _MAX_SLOPE_RATIO + 0.001
    soft_slope = _saturate(2.0 * amount)
    hard_slope = 1.0 / max(soft_slope, _MAX_SLOPE_RATIO)
    soft_alpha = remapped * soft_slope + 1.0 - soft_slope
    hard_alpha = remapped * hard_slope
    in_alpha = _saturate(hard_alpha + (soft_alpha - hard_alpha) * softness)
    out_alpha = 1.0 + (blend_map * blend_map - 1.0) * _saturate(2.0 * amount - 1.0)
    return 1.0 - in_alpha * out_alpha


def remap_wear(wear_alpha: float, start: float) -> float:
    """layerLib ``Layer_RemapWear``; ``start`` 0 leaves the value unchanged."""
    return _saturate((wear_alpha - start) / (1.0 - start))


def remap_start_for_shading_model(shading_model: int) -> float:
    if shading_model in (SHADING_PAINT, SHADING_CLEARCOAT_PAINT):
        return 1.0 - PAINT_PRIMER_START
    if shading_model == SHADING_CLOTH:
        return 1.0 - CLOTH_COMPLEMENT_START
    return 0.0


@dataclass(frozen=True)
class LayerWearParams:
    blend_map_path: str
    tiling: float
    falloff: float
    remap_start: float


def _authored_param(layer: Any, name: str) -> float | None:
    for param in (layer.resolved_material or {}).get("authored_public_params", []):
        if param.get("name") != name:
            continue
        try:
            return float(param.get("value"))
        except (TypeError, ValueError):
            return None
    return None


def primary_layer_wear_params(layer: Any) -> LayerWearParams | None:
    """Wear inputs from a primary ``LayerManifestEntry``; None without a blend map."""
    if layer is None:
        return None
    blend_path = next(
        (
            getattr(slot, "export_path", None)
            for slot in layer.texture_slots
            if getattr(slot, "slot", None) == "TexSlot8" and getattr(slot, "export_path", None)
        ),
        None,
    )
    if blend_path is None:
        return None
    falloff = _authored_param(layer, "WearDirtBlendFalloff")
    shading_model = _authored_param(layer, "ShadingModel")
    return LayerWearParams(
        blend_map_path=blend_path,
        tiling=layer.uv_tiling if layer.uv_tiling is not None else 1.0,
        falloff=LAYER_DEFAULT_WEAR_FALLOFF if falloff is None else falloff,
        remap_start=remap_start_for_shading_model(
            LAYER_DEFAULT_SHADING_MODEL if shading_model is None else int(shading_model)
        ),
    )
