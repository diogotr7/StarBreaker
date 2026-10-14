"""Small node-math builder shared by the shader-family builders.

:class:`NodeMath` wraps a material's ``nodes`` / ``links``; every helper
creates the node(s) for one operation and returns the output socket. Operands
may be sockets or constants (floats / tuples), so builders read like the HLSL
they mirror.
"""

from __future__ import annotations

from typing import Any

import bpy

from ..node_utils import _output_socket

Socket = Any
Operand = Any  # a socket or a constant


class NodeMath:
    """Small node-math builder: every helper returns an output socket."""

    def __init__(self, nodes: bpy.types.Nodes, links: bpy.types.NodeLinks) -> None:
        self.nodes = nodes
        self.links = links

    def _feed(self, socket: Socket, value: Operand) -> None:
        if isinstance(value, (int, float)):
            socket.default_value = float(value)
        elif isinstance(value, tuple):
            socket.default_value = value
        else:
            self.links.new(value, socket)

    def math(self, op: str, a: Operand, b: Operand = None, *, clamp: bool = False) -> Socket:
        node = self.nodes.new("ShaderNodeMath")
        node.operation = op
        node.use_clamp = clamp
        self._feed(node.inputs[0], a)
        if b is not None:
            self._feed(node.inputs[1], b)
        return node.outputs[0]

    def lerp(self, a: Operand, b: Operand, t: Operand) -> Socket:
        node = self.nodes.new("ShaderNodeMapRange")
        node.clamp = False
        node.inputs["From Min"].default_value = 0.0
        node.inputs["From Max"].default_value = 1.0
        self._feed(node.inputs["Value"], t)
        self._feed(node.inputs["To Min"], a)
        self._feed(node.inputs["To Max"], b)
        return node.outputs[0]

    def vec(self, op: str, a: Operand, b: Operand = None) -> Socket:
        node = self.nodes.new("ShaderNodeVectorMath")
        node.operation = op
        self._feed(node.inputs[0], a)
        if b is not None:
            self._feed(node.inputs[1], b)
        return _output_socket(node, "Value") if op == "DOT_PRODUCT" else node.outputs[0]

    def scale(self, v: Socket, s: Operand) -> Socket:
        node = self.nodes.new("ShaderNodeVectorMath")
        node.operation = "SCALE"
        self.links.new(v, node.inputs[0])
        self._feed(node.inputs["Scale"], s)
        return node.outputs[0]

    def vlerp(self, a: Operand, b: Operand, t: Operand) -> Socket:
        """``a + (b - a) * t`` on vectors / colours."""
        return self.vec("ADD", a, self.scale(self.vec("SUBTRACT", b, a), t))

    def saturate3(self, v: Socket) -> Socket:
        return self.vec("MINIMUM", self.vec("MAXIMUM", v, (0.0, 0.0, 0.0)), (1.0, 1.0, 1.0))

    def split(self, v: Operand) -> tuple[Socket, Socket, Socket]:
        node = self.nodes.new("ShaderNodeSeparateXYZ")
        self._feed(node.inputs[0], v)
        return node.outputs[0], node.outputs[1], node.outputs[2]

    def join(self, x: Operand, y: Operand, z: Operand) -> Socket:
        node = self.nodes.new("ShaderNodeCombineXYZ")
        for socket, value in zip(node.inputs, (x, y, z)):
            self._feed(socket, value)
        return node.outputs[0]

    def value(self, name: str, default: float = 0.0) -> Socket:
        node = self.nodes.new("ShaderNodeValue")
        node.name = node.label = name
        node.outputs[0].default_value = default
        return node.outputs[0]


    def srgb_encode(self, c: Operand) -> Socket:
        """Linear → sRGB transfer (recovers raw bytes from a BYTE_COLOR attribute)."""
        low = self.math("MULTIPLY", c, 12.92)
        high = self.math("SUBTRACT", self.math("MULTIPLY", self.math("POWER", c, 1.0 / 2.4), 1.055), 0.055)
        return self.lerp(high, low, self.math("LESS_THAN", c, 0.0031308))

    def smoothstep(self, edge0: float, edge1: float, x: Operand) -> Socket:
        node = self.nodes.new("ShaderNodeMapRange")
        node.interpolation_type = "SMOOTHSTEP"
        node.clamp = True
        node.inputs["From Min"].default_value = edge0
        node.inputs["From Max"].default_value = edge1
        node.inputs["To Min"].default_value = 0.0
        node.inputs["To Max"].default_value = 1.0
        self._feed(node.inputs["Value"], x)
        return node.outputs[0]
