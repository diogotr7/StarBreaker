"""Layer detail-map gloss and normals, as Star Engine computes them (bpy-free).

Sources (decoded engine shaders):

- ``common.cfib`` ``GetDetailMapGrad``: the detail texel is read ``.garb``,
  so normal x = G, normal y = A, diffuse = R, gloss = B.
  ``UnpackDetailMap``: x/y = min(2v - 256/255, 1); diffuse/gloss = 2v - 1.
- ``layerLib.cfib`` ``LayerAttribs_ReadDetail``: x/y *= DetailBump,
  gloss *= DetailGloss. ``LayerAttribs_DetailOverlay``: smoothness +=
  smoothness * gloss; normal = AddDetailNormals(base, ReconstructNormalZ(xy)).
- ``NormalsCommon.cfib`` ``ReconstructNormalZ`` / ``AddDetailNormals``
  (a reoriented normal blend).

These are the reference implementation the LayerSurface group is tested against.
"""

from __future__ import annotations

import math

DETAIL_NORMAL_MIDPOINT = (128.0 / 255.0) * 2.0  # UnpackDetailMap fMidPoint
RECONSTRUCT_EPSILON = 1e-6  # ReconstructNormalZ

Vec3 = tuple[float, float, float]


def detail_smoothness(smoothness: float, detail_blue: float, detail_gloss: float) -> float:
    value = smoothness * (1.0 + (2.0 * detail_blue - 1.0) * detail_gloss)
    return max(0.0, min(1.0, value))


def reconstruct_normal_z(x: float, y: float) -> Vec3:
    return (x, y, math.sqrt(max(RECONSTRUCT_EPSILON, 1.0 - (x * x + y * y))))


def detail_normal(detail_green: float, detail_alpha: float, detail_bump: float) -> Vec3:
    x = min(2.0 * detail_green - DETAIL_NORMAL_MIDPOINT, 1.0) * detail_bump
    y = min(2.0 * detail_alpha - DETAIL_NORMAL_MIDPOINT, 1.0) * detail_bump
    return reconstruct_normal_z(x, y)


def add_detail_normals(base: Vec3 | list[float], detail: Vec3) -> Vec3:
    bx, by, bz = base[0], base[1], base[2] + 1.0
    dx, dy, dz = -detail[0], -detail[1], detail[2]
    d = bx * dx + by * dy + bz * dz
    rx, ry, rz = bx * d - dx * bz, by * d - dy * bz, bz * d - dz * bz
    length = math.sqrt(rx * rx + ry * ry + rz * rz)
    return (rx / length, ry / length, rz / length)
