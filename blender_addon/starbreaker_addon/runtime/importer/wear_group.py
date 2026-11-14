"""Node graph of the ``StarBreaker Wear Input`` group (engine HardSurface wear).

Mirrors ``runtime.wear_utils`` node for node: vertex alpha wear, the
primary layer blend-map threshold (``CalcBlendAlpha``) and the shading-model
remap (``Layer_RemapWear``). The damage output keeps COLOR_0.B gated by
``Use Damage``.
"""

from __future__ import annotations

from typing import Any

from ..wear_utils import VERTEX_WEAR_SCALE
from ..node_utils import _output_socket

WEAR_INPUT_SIGNATURE = "wear_input_v2"
WEAR_INPUT_SOCKETS = [
    ("Blend Map", "NodeSocketColor"),
    ("Blend Falloff", "NodeSocketFloat"),
    ("Wear Blend Base", "NodeSocketFloat"),
    ("Instance Wear", "NodeSocketFloat"),
    ("Use Vertex Colors", "NodeSocketFloat"),
    ("Remap Start", "NodeSocketFloat"),
    ("Use Damage", "NodeSocketFloat"),
]
WEAR_OUTPUT_SOCKETS = [
    ("Wear Factor", "NodeSocketFloat"),
    ("Damage Factor", "NodeSocketFloat"),
]
WEAR_MATH_NODE_COUNT = 22

_MAX_SLOPE_RATIO = 1.0 / 255.0


def build_wear_input_graph(nodes: Any, links: Any, group_input: Any, group_output: Any) -> None:
    column = [0]

    def math(operation: str, *operands: Any, clamp: bool = False, label: str = "") -> Any:
        node = nodes.new("ShaderNodeMath")
        node.operation = operation
        node.use_clamp = clamp
        node.label = label
        node.location = (-400 + 160 * (column[0] // 4), 200 - 140 * (column[0] % 4))
        column[0] += 1
        for index, operand in enumerate(operands):
            if isinstance(operand, (int, float)):
                node.inputs[index].default_value = float(operand)
            else:
                links.new(operand, node.inputs[index])
        return node.outputs[0]

    def socket(name: str) -> Any:
        return _output_socket(group_input, name)

    vertex_color = nodes.new("ShaderNodeVertexColor")
    vertex_color.location = (-720, 120)
    vertex_color.layer_name = "Color"
    separate = nodes.new("ShaderNodeSeparateColor")
    separate.location = (-520, -260)
    links.new(vertex_color.outputs["Color"], separate.inputs[0])

    # wearAmt = saturate(max(WearBlendBase, instWear) - 0.95 * vertex alpha)
    vertex_alpha = math("MULTIPLY", vertex_color.outputs["Alpha"], socket("Use Vertex Colors"))
    vertex_wear = math("MULTIPLY", vertex_alpha, VERTEX_WEAR_SCALE, label="CalculateVertexWear")
    object_wear = math("MAXIMUM", socket("Wear Blend Base"), socket("Instance Wear"))
    amount = math("SUBTRACT", object_wear, vertex_wear, clamp=True, label="wearAmt")

    # CalcBlendInAlpha
    # LayerAttribs_ReadBlendMap samples the blend texture's .x (red).
    blend_split = nodes.new("ShaderNodeSeparateColor")
    blend_split.location = (-720, 320)
    links.new(socket("Blend Map"), blend_split.inputs[0])
    blend = _output_socket(blend_split, "Red")
    blend_in = math("MULTIPLY_ADD", blend, 1.0 - _MAX_SLOPE_RATIO - 0.001, _MAX_SLOPE_RATIO + 0.001)
    soft_slope = math("MULTIPLY", amount, 2.0, clamp=True, label="softSlope")
    hard_slope = math("DIVIDE", 1.0, math("MAXIMUM", soft_slope, _MAX_SLOPE_RATIO), label="hardSlope")
    soft_alpha = math("MULTIPLY_ADD", blend_in, soft_slope, math("SUBTRACT", 1.0, soft_slope))
    hard_alpha = math("MULTIPLY", blend_in, hard_slope)
    in_alpha = math(
        "MULTIPLY_ADD",
        math("SUBTRACT", soft_alpha, hard_alpha),
        socket("Blend Falloff"),
        hard_alpha,
        clamp=True,
        label="CalcBlendInAlpha",
    )

    # CalcBlendOutAlpha = lerp(1, blend^2, saturate(2 * wearAmt - 1))
    out_t = math("MULTIPLY_ADD", amount, 2.0, -1.0, clamp=True)
    out_alpha = math(
        "MULTIPLY_ADD", math("MULTIPLY_ADD", blend, blend, -1.0), out_t, 1.0, label="CalcBlendOutAlpha"
    )
    wear_alpha = math("SUBTRACT", 1.0, math("MULTIPLY", in_alpha, out_alpha), label="CalcBlendAlpha")

    # Layer_RemapWear
    start = socket("Remap Start")
    remapped = math(
        "DIVIDE",
        math("SUBTRACT", wear_alpha, start),
        math("SUBTRACT", 1.0, start),
        clamp=True,
        label="Layer_RemapWear",
    )
    damage = math("MULTIPLY", _output_socket(separate, "Blue"), socket("Use Damage"), clamp=True)

    links.new(remapped, group_output.inputs["Wear Factor"])
    links.new(damage, group_output.inputs["Damage Factor"])
