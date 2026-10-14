"""HairPBR material builder mixin for :class:`PackageImporter`.

Mirrors ``HairPBR.cfxb`` (parameters in :mod:`..hair_utils`):

* ``%HAIR_CAP`` (scalp / brow-base caps): colour = ``clamp(TexSlot1.rgb ·
  BaseTintColor · MatDifColor)``, coverage = ``TexSlot1.a · (1 − facing^16)``,
  diffuse-only — a thin root layer over the skin. TexSlot1 is read
  channel-packed (RGB colour, alpha coverage).
* fibres (``%HAIR_CARDS`` / ``%HAIR_COAT``): per-pixel ``GetHairColor`` from
  melanin (+ per-strand variation from the TexSlot4 ID map) × BaseTintColor,
  the dye layer (per-strand threshold, hair-length fade from the raw vertex
  colour), vertex-colour AO, reflectance / roughness from Smoothness,
  SpecularVariation and root occlusion, and the dithered opacity as alpha
  (cards ``opac^(2/3)``, coat ``opac^(0.8/γ)``). Shading = diffuse at the
  HairPBR wrap energy (``HAIR_DIFFUSE_SCALE``) + the shader's R / TT / TRT
  lobes as Cycles Hair BSDFs along the fibre (card V) axis, energies and
  shifts from :func:`..hair_utils.hair_lobes`. Not modelled: per-card glint
  noise, the engine noise texture for cards without an ID map (uses id 0.5),
  ``globalScattering`` (multiple-scattering shadow tint), the ``DyeColorMap``
  (unbound → white, as in all current hair ``.mtl``s).
"""

from __future__ import annotations

from typing import Any

import bpy

from ..constants import HAIR_DIFFUSE_SCALE
from ..hair_utils import HairParams, hair_cap_params, hair_lobes, hair_params, melanin_coefficients
from ..node_utils import _input_socket, _output_socket
from ..record_utils import _submaterial_texture_reference
from ..skin_utils import LUMINANCE
from ...manifest import MaterialSidecar, SubmaterialRecord
from .node_math import NodeMath, Operand, Socket


class HairMixin:
    """Builds ``HairPBR`` caps and fibres (see module docs)."""

    def _build_hair_cap_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        plan: Any,
    ) -> None:
        params = hair_cap_params(submaterial)
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        principled = self._create_surface_bsdf(nodes)
        _input_socket(principled, "Specular IOR Level", "Specular").default_value = 0.0  # diffuse-only
        base_color = _input_socket(principled, "Base Color")
        alpha = _input_socket(principled, "Alpha")

        ref = _submaterial_texture_reference(submaterial, slots=("TexSlot1",))
        color_map = self._image_node(nodes, ref.export_path if ref else None, x=-700, y=0, is_color=True)
        if color_map is None:
            base_color.default_value = (
                *(min(max(c, params.color_min), params.color_max) * HAIR_DIFFUSE_SCALE for c in params.color_scale),
                1.0,
            )
        else:
            if color_map.image is not None:
                color_map.image.alpha_mode = "CHANNEL_PACKED"
            color = self._hair_vector_math(nodes, links, "MULTIPLY", _output_socket(color_map, "Color"), params.color_scale)
            color = self._hair_vector_math(nodes, links, "MAXIMUM", color, (params.color_min,) * 3)
            color = self._hair_vector_math(nodes, links, "MINIMUM", color, (params.color_max,) * 3)
            color = self._hair_vector_math(nodes, links, "MULTIPLY", color, (HAIR_DIFFUSE_SCALE,) * 3)
            links.new(color, base_color)

            # hairCapAlpha = ColorMap.a * (1 - (1 - saturate(N.V))^16)
            layer_weight = nodes.new("ShaderNodeLayerWeight")
            layer_weight.inputs["Blend"].default_value = 0.5  # Facing = 1 - |N.V|
            fade = nodes.new("ShaderNodeMath")
            fade.operation = "POWER"
            links.new(_output_socket(layer_weight, "Facing"), fade.inputs[0])
            fade.inputs[1].default_value = params.grazing_exponent
            visible = nodes.new("ShaderNodeMath")
            visible.operation = "SUBTRACT"
            visible.inputs[0].default_value = 1.0
            links.new(fade.outputs[0], visible.inputs[1])
            coverage = nodes.new("ShaderNodeMath")
            coverage.operation = "MULTIPLY"
            coverage.use_clamp = True
            links.new(_output_socket(color_map, "Alpha"), coverage.inputs[0])
            links.new(visible.outputs[0], coverage.inputs[1])
            links.new(coverage.outputs[0], alpha)

        output = nodes.new("ShaderNodeOutputMaterial")
        self._wire_surface_shader_to_output(nodes, links, principled.outputs[0], output, plan, submaterial)
        self._configure_material(material, blend_method="HASHED", shadow_method="HASHED")

    @staticmethod
    def _hair_vector_math(nodes: Any, links: Any, op: str, a: Any, b: tuple[float, float, float]) -> Any:
        node = nodes.new("ShaderNodeVectorMath")
        node.operation = op
        links.new(a, node.inputs[0])
        node.inputs[1].default_value = b
        return node.outputs[0]

    def _build_hair_fibre_material(
        self,
        material: bpy.types.Material,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        plan: Any,
    ) -> None:
        params = hair_params(submaterial, sidecar.source_material_path, self._package_character())
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()
        g = NodeMath(nodes, links)

        def tex(slot: str) -> Any:
            ref = _submaterial_texture_reference(submaterial, slots=(slot,))
            node = self._image_node(nodes, ref.export_path if ref else None, x=-1200, y=0, is_color=False)
            if node is not None and node.image is not None:
                node.image.alpha_mode = "CHANNEL_PACKED"
            return node

        # strand ID: TexSlot4 .x with %CARD_ID_MAP; coat: 0
        hair_id: Operand = 0.0
        if params.cards:
            id_map = tex("TexSlot4") if params.id_map else None
            hair_id = g.split(_output_socket(id_map, "Color"))[0] if id_map is not None else 0.5

        # raw vertex colour: x = hair length (m), y / z = AO
        hair_length: Operand = 0.015
        occlusion: Operand = 1.0
        if params.cards:
            attribute = nodes.new("ShaderNodeVertexColor")
            attribute.layer_name = "Color"
            raw = [g.srgb_encode(c) for c in g.split(_output_socket(attribute, "Color"))]
            hair_length = g.math("MAXIMUM", raw[0], 0.001)
            occlusion = g.math("MAXIMUM", g.math("MAXIMUM", raw[1], raw[2]), 1e-5)

        color = self._hair_fibre_color(g, params, hair_id, hair_length)
        if params.cards and params.ambient_occlusion > 0.0:
            exponent = g.math(
                "MULTIPLY",
                g.math("SUBTRACT", 1.5, g.math("MULTIPLY", g.vec("DOT_PRODUCT", color, LUMINANCE), 0.75)),
                params.ambient_occlusion,
            )
            color = g.scale(color, g.math("POWER", occlusion, exponent))

        # reflectance / roughness
        reflectance: Operand = min(1.0, max(0.0, params.smoothness * 10.0))
        roughness: Operand = 0.15 + (1.0 - params.smoothness) * 0.85
        if params.cards:
            root = g.lerp(
                1.0 - 0.66 * params.specular_root_occlusion,
                1.0,
                g.math("POWER", g.math("DIVIDE", hair_length, 0.08, clamp=True), 2.0),
            )
            variation = g.math("POWER", g.math("ADD", hair_id, 1.0 - params.specular_variation ** 0.5 * 0.7), 2.0)
            reflectance = g.math("MULTIPLY", g.math("MULTIPLY", reflectance, root), g.math("MINIMUM", variation, 1.0))
            roughness = g.lerp(roughness, max(roughness - 0.1, 0.15), g.smoothstep(0.7, 0.8, hair_id))

        surface = self._hair_fibre_shading(g, color, reflectance, roughness)

        opacity = tex("TexSlot1")
        if opacity is not None:
            coverage = g.split(_output_socket(opacity, "Color"))[0]
            exponent = 2.0 / 3.0 if params.cards else 0.8 / max(params.opacity_gamma, 1e-3)
            mix = nodes.new("ShaderNodeMixShader")
            links.new(g.math("POWER", coverage, exponent), mix.inputs[0])
            links.new(nodes.new("ShaderNodeBsdfTransparent").outputs[0], mix.inputs[1])
            links.new(surface, mix.inputs[2])
            surface = mix.outputs[0]

        output = nodes.new("ShaderNodeOutputMaterial")
        self._wire_surface_shader_to_output(nodes, links, surface, output, plan, submaterial)
        self._configure_material(material, blend_method="HASHED", shadow_method="HASHED")

    @staticmethod
    def _hair_fibre_shading(g: NodeMath, color: Socket, reflectance: Operand, roughness: Operand) -> Socket:
        """Diffuse (HairPBR wrap energy) + the R / TT / TRT lobes as Hair BSDFs.

        The fibre axis is the card V direction: ``cross(N, dP/du)``.
        """
        nodes, links = g.nodes, g.links
        tangent = nodes.new("ShaderNodeTangent")
        tangent.direction_type = "UV_MAP"
        geometry = nodes.new("ShaderNodeNewGeometry")
        fibre = g.vec(
            "NORMALIZE",
            g.vec("CROSS_PRODUCT", _output_socket(geometry, "Normal"), _output_socket(tangent, "Tangent")),
        )
        diffuse = nodes.new("ShaderNodeBsdfDiffuse")
        links.new(g.scale(color, HAIR_DIFFUSE_SCALE), diffuse.inputs["Color"])
        surface = diffuse.outputs[0]
        beta = g.math("POWER", roughness, 2.0)
        sqrt_color = g.join(*(g.math("SQRT", c) for c in g.split(color)))
        for lobe in hair_lobes():
            if lobe.tint == "none":
                weight = g.math("MULTIPLY", reflectance, lobe.energy)
                lobe_color = g.join(weight, weight, weight)
            else:
                lobe_color = g.scale(color if lobe.tint == "color" else sqrt_color, lobe.energy)
            bsdf = nodes.new("ShaderNodeBsdfHair")
            bsdf.component = "Transmission" if lobe.transmission else "Reflection"
            links.new(lobe_color, bsdf.inputs["Color"])
            bsdf.inputs["Offset"].default_value = lobe.offset
            links.new(
                g.math("MINIMUM", g.math("MAXIMUM", g.math("MULTIPLY", beta, lobe.roughness_scale), 0.001), 1.0),
                bsdf.inputs["RoughnessU"],
            )
            bsdf.inputs["RoughnessV"].default_value = 1.0  # azimuthal spread (cosHalfPhi falloff)
            links.new(fibre, bsdf.inputs["Tangent"])
            add = nodes.new("ShaderNodeAddShader")
            links.new(surface, add.inputs[0])
            links.new(bsdf.outputs[0], add.inputs[1])
            surface = add.outputs[0]
        return surface

    @staticmethod
    def _hair_fibre_color(g: NodeMath, params: HairParams, hair_id: Operand, hair_length: Operand) -> Socket:
        """``GetHairColor(melanin) · BaseTintColor``, dye layer, ``· MatDifColor``, clamped."""
        offset = g.math("SUBTRACT", hair_id, 0.5) if params.id_map else g.math("POWER", hair_id, 2.0)
        scale = params.melanin_variation * 0.5 if params.id_map else params.melanin_variation
        melanin = g.math("ADD", params.melanin, g.math("MULTIPLY", offset, scale), clamp=True)
        one_minus = g.math("MAXIMUM", g.math("SUBTRACT", 1.0, g.math("POWER", melanin, 2.0)), 1e-4)
        quantity = g.math("MULTIPLY", g.math("LOGARITHM", one_minus, 2.718281828459045), -1.0)
        root = g.math("SQRT", quantity)
        channels = [
            g.math("MULTIPLY", g.math("EXPONENT", g.math("MULTIPLY", root, -k)), tint)
            for k, tint in zip(melanin_coefficients(params.melanin_redness), params.tint)
        ]

        if params.dye_amount > 0.0:
            # DyeColorMap unbound -> white: colorMap.rgb = 1, colorMap.a = 1.
            per_hair = g.math("SQRT", g.math("MULTIPLY", g.math("SUBTRACT", params.dye_amount, hair_id), 2.0, clamp=True))
            blend = g.lerp(per_hair, 1.0, params.dye_amount ** 2)
            shift = g.smoothstep(
                0.0, max(params.dye_fadeout, 0.001), g.math("SUBTRACT", hair_length, params.dye_shift, clamp=True)
            )
            dyed = []
            for channel, dye in zip(channels, params.dye_color):
                pigment = g.math(
                    "ADD",
                    max(dye, 0.0) ** 0.5,
                    g.math("MULTIPLY", g.math("SUBTRACT", 0.5, hair_id), 0.75 * params.dye_pigment_variation),
                    clamp=True,
                )
                dyed_channel = g.lerp(channel, g.math("POWER", pigment, 2.0), blend)
                dyed.append(g.lerp(channel, dyed_channel, shift))
            channels = dyed

        return g.join(
            *(
                g.math("MINIMUM", g.math("MAXIMUM", g.math("MULTIPLY", c, d), 0.0001), 0.99)
                for c, d in zip(channels, params.diffuse)
            )
        )
