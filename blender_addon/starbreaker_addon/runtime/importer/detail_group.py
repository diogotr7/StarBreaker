"""Detail-map gloss and normal sub-graphs of the LayerSurface group.

Mirrors ``runtime.detail_utils`` node for node (engine
``LayerAttribs_ReadDetail`` / ``LayerAttribs_DetailOverlay``,
``ReconstructNormalZ``, ``AddDetailNormals``).
"""

from __future__ import annotations

from typing import Any

from ..detail_utils import DETAIL_NORMAL_MIDPOINT, RECONSTRUCT_EPSILON
from ..node_utils import _output_socket

DETAIL_NORMAL_FLAT = 128.0 / 255.0


class _Math:
    def __init__(self, nodes: Any, links: Any, *, x: int, y: int) -> None:
        self.nodes, self.links = nodes, links
        self.x, self.y, self.count = x, y, 0

    def __call__(self, operation: str, *operands: Any, clamp: bool = False, label: str = "") -> Any:
        node = self.nodes.new("ShaderNodeMath")
        node.operation = operation
        node.use_clamp = clamp
        node.label = label
        node.location = (self.x + 150 * (self.count // 5), self.y - 120 * (self.count % 5))
        self.count += 1
        for index, operand in enumerate(operands):
            if isinstance(operand, (int, float)):
                node.inputs[index].default_value = float(operand)
            else:
                self.links.new(operand, node.inputs[index])
        return node.outputs[0]

    def reconstruct_z(self, x: Any, y: Any) -> Any:
        length_sq = self("MULTIPLY_ADD", x, x, self("MULTIPLY", y, y))
        return self("SQRT", self("MAXIMUM", self("SUBTRACT", 1.0, length_sq), RECONSTRUCT_EPSILON))


def build_detail_gloss_roughness(nodes: Any, links: Any, group_input: Any, roughness: Any) -> Any:
    """Roughness after the detail gloss, applied in smoothness space.

    The add-on maps smoothness to roughness as sqrt(1 - s) (DDNA Roughness
    group), so s = 1 - r^2, s *= 1 + (2B - 1) * DetailGloss, r = sqrt(1 - s).
    """
    math = _Math(nodes, links, x=80, y=-200)
    smoothness = math("SUBTRACT", 1.0, math("MULTIPLY", roughness, roughness))
    signed = math("MULTIPLY_ADD", _output_socket(group_input, "Detail Gloss Mask"), 2.0, -1.0)
    scale = math("MULTIPLY_ADD", signed, _output_socket(group_input, "Detail Gloss Strength"), 1.0)
    detailed = math("MULTIPLY", smoothness, scale, clamp=True, label="Detail smoothness")
    return math("SQRT", math("SUBTRACT", 1.0, detailed), label="Roughness")


def build_detail_normal_color(nodes: Any, links: Any, group_input: Any) -> Any:
    """Normal-map colour of the base normal with the detail normal added (RNM)."""
    math = _Math(nodes, links, x=-1100, y=-560)
    base_split = nodes.new("ShaderNodeSeparateColor")
    base_split.location = (-1300, -560)
    links.new(_output_socket(group_input, "Normal Color"), base_split.inputs[0])
    bx = math("MULTIPLY_ADD", _output_socket(base_split, "Red"), 2.0, -1.0)
    by = math("MULTIPLY_ADD", _output_socket(base_split, "Green"), 2.0, -1.0)
    bz_plus_one = math("ADD", math.reconstruct_z(bx, by), 1.0)

    bump = _output_socket(group_input, "Detail Bump Strength")

    def unpack(name: str) -> Any:
        signed = math("MULTIPLY_ADD", _output_socket(group_input, name), 2.0, -DETAIL_NORMAL_MIDPOINT)
        return math("MULTIPLY", math("MINIMUM", signed, 1.0), bump)

    dx = unpack("Detail Normal X")
    dy = unpack("Detail Normal Y")
    dz = math.reconstruct_z(dx, dy)

    # AddDetailNormals with vBase.z += 1 and vDetail.xy negated.
    dot = math(
        "MULTIPLY_ADD", bz_plus_one, dz, math("SUBTRACT", 0.0, math("MULTIPLY_ADD", bx, dx, math("MULTIPLY", by, dy)))
    )
    rx = math("MULTIPLY_ADD", bx, dot, math("MULTIPLY", dx, bz_plus_one))
    ry = math("MULTIPLY_ADD", by, dot, math("MULTIPLY", dy, bz_plus_one))
    rz = math("MULTIPLY", bz_plus_one, math("SUBTRACT", dot, dz))

    combine = nodes.new("ShaderNodeCombineXYZ")
    combine.location = (math.x + 150 * (math.count // 5 + 1), -560)
    links.new(rx, combine.inputs[0])
    links.new(ry, combine.inputs[1])
    links.new(rz, combine.inputs[2])
    normalize = nodes.new("ShaderNodeVectorMath")
    normalize.operation = "NORMALIZE"
    normalize.location = (combine.location[0] + 160, -560)
    links.new(combine.outputs[0], normalize.inputs[0])
    encode = nodes.new("ShaderNodeVectorMath")
    encode.operation = "MULTIPLY_ADD"
    encode.label = "Encode normal colour"
    encode.location = (combine.location[0] + 320, -560)
    links.new(normalize.outputs[0], encode.inputs[0])
    encode.inputs[1].default_value = (0.5, 0.5, 0.5)
    encode.inputs[2].default_value = (0.5, 0.5, 0.5)
    return encode.outputs[0]
