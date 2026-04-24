"""Decal/stencil and shadow-wiring helpers for ``PackageImporter``.

Extracted from ``runtime/_legacy.py``. Owns:

* Virtual-tint palette decal sources for stencil/decal shaders
  (``_virtual_tint_palette_decal_sockets``,
  ``_virtual_tint_palette_decal_defaults``).
* Surface→Output shadow wrapper wiring
  (``_plan_casts_no_shadows``, ``_wire_surface_shader_to_output``,
  ``_shadowless_surface_output``).
* Primary-surface-node introspection used by the export pipeline
  (``_primary_surface_node``, ``_is_shadow_wrapper_mix``).

Cross-mixin calls (``self._palette_group_node``, ``self._palette_color_socket``,
``self._ensure_runtime_shadowless_wrapper_group``) resolve via composed MRO.
"""

from __future__ import annotations

from typing import Any

import bpy

from ...manifest import PaletteRecord, SubmaterialRecord
from ...palette import palette_decal_texture
from ..node_utils import _input_socket, _output_socket, _refresh_group_node_sockets
from ..record_utils import (
    _authored_attribute_triplet,
    _optional_float_public_param,
    _public_param_triplet,
    _resolved_submaterial_palette_color,
)
from .types import LayerSurfaceSockets


class DecalsMixin:
    """Decal/stencil sources and shadow-wrapper wiring for ``PackageImporter``."""

    def _has_palette_decal_texture(self, palette: PaletteRecord | None) -> bool:
        return (
            palette is not None
            and self.package.resolve_path(palette_decal_texture(palette)) is not None
        )

    # ------------------------------------------------------------------
    # Virtual-tint palette decal sources
    # ------------------------------------------------------------------
    def _virtual_tint_palette_decal_sockets(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        has_decal_texture = self._has_palette_decal_texture(palette)
        fallback_color, alpha = self._virtual_tint_palette_decal_defaults(
            submaterial,
            palette,
            has_decal_texture=has_decal_texture,
        )
        if has_decal_texture:
            group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
            color_socket = _output_socket(group_node, "Decal Color")
            alpha_socket = _output_socket(group_node, "Decal Alpha")
            if color_socket is not None and alpha_socket is not None:
                if abs(alpha - 1.0) < 1e-6:
                    return LayerSurfaceSockets(color=color_socket, alpha=alpha_socket)
                alpha_multiply = next(
                    (
                        node
                        for node in nodes
                        if node.bl_idname == "ShaderNodeMath"
                        and getattr(node, "name", "")
                        == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA_MULTIPLY"
                    ),
                    None,
                )
                if alpha_multiply is None:
                    alpha_multiply = nodes.new("ShaderNodeMath")
                    alpha_multiply.name = (
                        "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA_MULTIPLY"
                    )
                    alpha_multiply.label = "StarBreaker Virtual Tint Palette Decal Alpha"
                    alpha_multiply.operation = "MULTIPLY"
                alpha_multiply.location = (x + 220, y - 140)
                alpha_multiply.inputs[1].default_value = alpha
                nodes.id_data.links.new(alpha_socket, alpha_multiply.inputs[0])
                return LayerSurfaceSockets(color=color_socket, alpha=alpha_multiply.outputs[0])

        palette_color_socket = None
        if palette is not None:
            palette_channel = submaterial.palette_routing.material_channel
            channel_name = (
                palette_channel.name
                if palette_channel is not None
                else ("glass" if submaterial.shader_family == "GlassPBR" else None)
            )
            if channel_name is not None:
                palette_color_socket = self._palette_color_socket(
                    nodes, palette, channel_name, x=x, y=y
                )

        color_node = next(
            (
                node
                for node in nodes
                if node.bl_idname == "ShaderNodeRGB"
                and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"
            ),
            None,
        )
        alpha_node = next(
            (
                node
                for node in nodes
                if node.bl_idname == "ShaderNodeValue"
                and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"
            ),
            None,
        )

        if color_node is None:
            color_node = nodes.new("ShaderNodeRGB")
            color_node.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"
            color_node.label = "StarBreaker Virtual Tint Palette Decal"
        color_node.location = (x, y)

        if alpha_node is None:
            alpha_node = nodes.new("ShaderNodeValue")
            alpha_node.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"
            alpha_node.label = "StarBreaker Virtual Tint Palette Decal Alpha"
        alpha_node.location = (x, y - 140)

        if palette_color_socket is not None:
            alpha_node.outputs[0].default_value = alpha
            return LayerSurfaceSockets(color=palette_color_socket, alpha=alpha_node.outputs[0])

        color_node.outputs[0].default_value = (*fallback_color, 1.0)
        alpha_node.outputs[0].default_value = alpha
        return LayerSurfaceSockets(color=color_node.outputs[0], alpha=alpha_node.outputs[0])

    def _virtual_tint_palette_decal_defaults(
        self,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        has_decal_texture: bool = True,
    ) -> tuple[tuple[float, float, float], float]:
        color = (
            _public_param_triplet(
                submaterial,
                "StencilDiffuseColor1",
                "StencilDiffuse1",
                "StencilTintColor",
                "TintColor",
                "StencilDiffuseColor",
            )
            or _resolved_submaterial_palette_color(submaterial, palette)
            or _authored_attribute_triplet(submaterial, "Diffuse")
            or (1.0, 1.0, 1.0)
        )
        alpha = _optional_float_public_param(
            submaterial, "StencilOpacity", "DecalDiffuseOpacity", "DecalAlphaMult"
        )
        if alpha is None:
            flags = submaterial.decoded_feature_flags
            is_decal_surface = (
                submaterial.shader_family == "MeshDecal"
                or flags.has_decal
                or flags.has_stencil_map
            )
            if is_decal_surface and not has_decal_texture:
                alpha = 0.0
            else:
                alpha = 0.85 if submaterial.shader_family == "MeshDecal" else 0.5
        return color, max(0.0, min(1.0, alpha))

    # ------------------------------------------------------------------
    # Shadow-ray wrapper wiring
    # ------------------------------------------------------------------
    def _primary_surface_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        output: bpy.types.Node | None,
    ) -> bpy.types.Node | None:
        if output is None:
            return None

        surface_input = _input_socket(output, "Surface")
        if surface_input is None or not surface_input.is_linked:
            return None

        primary_node = next(
            (
                link.from_node
                for link in links
                if link.to_node == output and link.to_socket == surface_input
            ),
            None,
        )
        if primary_node is None:
            return None

        if self._is_shadow_wrapper_mix(primary_node):
            shader_input = primary_node.inputs[2]
            if shader_input.is_linked:
                return shader_input.links[0].from_node

        return primary_node

    def _is_shadow_wrapper_mix(self, node: bpy.types.Node | None) -> bool:
        if node is None or node.bl_idname != "ShaderNodeMixShader":
            return False

        factor_input = node.inputs[0]
        transparent_input = node.inputs[1]
        shader_input = node.inputs[2]
        if (
            not factor_input.is_linked
            or not transparent_input.is_linked
            or not shader_input.is_linked
        ):
            return False

        return (
            factor_input.links[0].from_node.bl_idname == "ShaderNodeLightPath"
            and transparent_input.links[0].from_node.bl_idname == "ShaderNodeBsdfTransparent"
        )

    def _plan_casts_no_shadows(
        self, plan: Any, submaterial: SubmaterialRecord | None = None
    ) -> bool:
        template_key = getattr(plan, "template_key", "")
        # ``decal_stencil`` is always a thin overlay — never cast shadows.
        if template_key == "decal_stencil":
            return True
        if submaterial is not None and submaterial.shader_family == "MeshDecal":
            return True
        # ``parallax_pom`` is used both for genuine POM decals (thin
        # overlays that should let host shadows through) and for opaque
        # HardSurface panels that happen to use POM for surface detail.
        # Only the former should be shadowless; opaque POM panels that
        # self-shadow properly render much darker and closer to the
        # in-game look (otherwise shadow rays pass through and Cycles
        # GI lifts the surface to a uniform mid-grey even when the
        # palette tint is near-black).
        if template_key == "parallax_pom" and submaterial is not None:
            flags = submaterial.decoded_feature_flags
            if flags.has_decal or flags.has_stencil_map:
                return True
            if submaterial.shader_family == "MeshDecal":
                return True
            return False
        # Legacy behaviour for callers that invoke without a submaterial
        # (early bring-up paths) — preserve the historical default.
        if template_key == "parallax_pom":
            return True
        return False

    def _wire_surface_shader_to_output(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        surface_shader: Any,
        output_node: bpy.types.Node,
        plan: Any,
        submaterial: SubmaterialRecord | None = None,
    ) -> None:
        """Link *surface_shader* to *output_node*'s Surface socket.

        If ``_plan_casts_no_shadows(plan, submaterial)`` is True, insert the
        shared ``StarBreaker Runtime Shadowless Wrapper`` group so the
        surface becomes invisible to shadow rays while preserving top-level
        graph hygiene. Otherwise link directly.
        """
        if surface_shader is None:
            return
        if self._plan_casts_no_shadows(plan, submaterial):
            wrapper = nodes.new("ShaderNodeGroup")
            wrapper.node_tree = self._ensure_runtime_shadowless_wrapper_group()
            _refresh_group_node_sockets(wrapper)
            wrapper.location = (output_node.location.x - 180, output_node.location.y - 140)
            wrapper.label = "StarBreaker Shadowless"
            links.new(surface_shader, wrapper.inputs["Shader"])
            links.new(wrapper.outputs["Shader"], output_node.inputs[0])
        else:
            links.new(surface_shader, output_node.inputs[0])

    def _shadowless_surface_output(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        surface_shader: Any,
    ) -> Any:
        light_path = nodes.new("ShaderNodeLightPath")
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        mix = nodes.new("ShaderNodeMixShader")
        shadow_ray = _output_socket(light_path, "Is Shadow Ray")
        if shadow_ray is None:
            return surface_shader
        links.new(shadow_ray, mix.inputs[0])
        links.new(surface_shader, mix.inputs[1])
        links.new(transparent.outputs[0], mix.inputs[2])
        return mix.outputs[0]
