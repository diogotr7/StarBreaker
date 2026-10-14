"""HumanSkin_V2 material builder mixin for :class:`PackageImporter`.

Rebuilds CryEngine's ``HumanSkin_V2`` ``GetMaterialAttribs`` (decoded from
the shader cache) as a node graph, in shader order:

1. albedo ``TexSlot1`` + ``%WRINKLE_BLENDING_2`` diffuse deltas
   (``TexSlot3`` atlas, weights ``sqrt(sat(w / max(1, Σw)))``), × ``MatDifColor``;
   normal ``TexSlot2`` + wrinkle normal deltas (``TexSlot8`` atlas). Wrinkle
   weights come from the 2-panel ``TexSlot11`` mask and 24 ``wrinkle_NN``
   Value nodes (keyed by expression animation);
2. ``%TONE_ADJUSTMENT`` rebase onto ``FinalSkinTone`` (``TexSlot7`` mask:
   r = apply, g = detail, b = micro-normal scale);
3. CHF blemishes (freckles / sun spots: ``TexSlot5`` mask, ``TexSlot10`` ID);
4. reflectance ``TexSlot4 · MatSpecColor``; smoothness
   ``Shininess/255 · gloss(TexSlot2 alpha) · 0.8``;
5. micro-detail normal (``uv · 65 · (1, 0.25) · MicroDetailTiling``) and its
   smoothness adjustment;
6. ``LIGHTINGMODEL_SKIN`` → Principled random-walk subsurface, widened by the
   ``TexSlot12`` transmittance (knobs in ``runtime/constants.py``).

Parameters (incl. CHF overrides) are resolved by
:func:`..skin_utils.human_skin_params`. Not yet modelled: makeup / tattoo
layers (``HumanSkinMakeup``), ``%DECAL_MAP``.
"""

from __future__ import annotations

from typing import Any

import bpy

from ..constants import SKIN_SUBSURFACE_RADIUS, SKIN_SUBSURFACE_SCALE, SKIN_TRANSMITTANCE_SCALE_GAIN
from ..node_utils import _input_socket, _output_socket
from ..record_utils import _submaterial_texture_reference
from ..skin_utils import LUMINANCE, HumanSkinParams, human_skin_params, tone_constants
from .node_math import NodeMath, Operand, Socket
from ...manifest import MaterialSidecar, SubmaterialRecord

# CalculateSkinBlemishes melanin endpoints and luminance range.
_PALE_MELANIN = (0.53947961, 0.15896086, 0.068478175)
_DARK_MELANIN = (0.0043914425, 0.0030352699, 0.0018211621)
_BLEMISH_MIN_LUMINANCE = 0.004
_BLEMISH_MAX_LUMINANCE = 0.4

class SkinMixin:
    """Builds ``HumanSkin_V2`` submaterials (see module docs)."""

    def _package_character(self) -> dict[str, Any] | None:
        package = getattr(self, "package", None)
        root = getattr(getattr(package, "scene", None), "root_entity", None)
        character = getattr(root, "raw", {}).get("character") if root is not None else None
        return character if isinstance(character, dict) else None

    def _build_human_skin_material(
        self,
        material: bpy.types.Material,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        plan: Any,
    ) -> None:
        params = human_skin_params(submaterial, sidecar.source_material_path, self._package_character())
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()
        g = NodeMath(nodes, links)

        uv_node = nodes.new("ShaderNodeUVMap")
        u, v, _ = g.split(uv_node.outputs[0])

        def slot_path(slot: str) -> str | None:
            ref = _submaterial_texture_reference(submaterial, slots=(slot,))
            return ref.export_path if ref is not None else None

        def tex(
            path: str | None,
            *,
            color: bool,
            vector: Socket | None = None,
            interpolation: str = "Linear",
            extension: str = "REPEAT",
        ) -> Any:
            node = self._image_node(nodes, path, x=-1200, y=0, is_color=color)
            if node is None:
                return None
            node.interpolation = interpolation
            node.extension = extension
            if node.image is not None and not color:
                node.image.alpha_mode = "CHANNEL_PACKED"
            links.new(vector if vector is not None else uv_node.outputs[0], node.inputs[0])
            return node

        def color_of(node: Any) -> Socket:
            return _output_socket(node, "Color")

        def alpha_of(node: Any) -> Socket:
            return _output_socket(node, "Alpha")

        # 1. albedo / normal (+ wrinkles)
        base = tex(slot_path("TexSlot1"), color=True)
        albedo: Operand = color_of(base) if base is not None else (1.0, 1.0, 1.0)
        ddna = tex(slot_path("TexSlot2"), color=False)
        normal_enc: Operand = color_of(ddna) if ddna is not None else (0.5, 0.5, 1.0)
        gloss: Operand = alpha_of(ddna) if ddna is not None else 1.0

        weights = self._skin_wrinkle_weights(g, params, slot_path("TexSlot11"), u, v, tex, color_of, alpha_of)
        material["starbreaker_wrinkle_slot_count"] = len(params.wrinkle_slots) if weights else 0
        if weights is not None:
            w1, w2 = weights

            def tile(index: int) -> Socket:
                return g.join(g.math("ADD", g.math("MULTIPLY", u, 1.0 / 3.0), index / 3.0), v, 0.0)

            atlas_diffuse = [tex(slot_path("TexSlot3"), color=True, vector=tile(i)) for i in range(3)]
            if base is not None and all(atlas_diffuse):
                total = g.math("MAXIMUM", g.math("ADD", w1, w2), 1.0)
                wd = [g.math("SQRT", g.math("DIVIDE", w, total, clamp=True)) for w in (w1, w2)]
                d0 = color_of(atlas_diffuse[0])
                blended = albedo
                for k, w in zip((1, 2), wd):
                    blended = g.vec("ADD", blended, g.scale(g.vec("SUBTRACT", color_of(atlas_diffuse[k]), d0), w))
                albedo = g.saturate3(blended)
            atlas_normal = [tex(slot_path("TexSlot8"), color=False, vector=tile(i)) for i in range(3)]
            if ddna is not None and all(atlas_normal):
                n0 = color_of(atlas_normal[0])
                for k, w in zip((1, 2), (w1, w2)):
                    normal_enc = g.vec("ADD", normal_enc, g.scale(g.vec("SUBTRACT", color_of(atlas_normal[k]), n0), w))
        if params.diffuse_color != (1.0, 1.0, 1.0):
            albedo = g.vec("MULTIPLY", albedo, params.diffuse_color)

        # 2. tone adjustment
        tone_mask = tex(slot_path("TexSlot7"), color=False) if params.tone_adjustment else None
        micro_scale: Operand = 1.0
        if tone_mask is not None:
            mask_r, mask_g, mask_b = g.split(color_of(tone_mask))
            micro_scale = mask_b
            albedo = self._skin_tone_adjustment(g, params, albedo, mask_r, mask_g)

        # 3. blemishes
        if params.blemishes:
            albedo = self._skin_blemishes(g, params, albedo, tex, color_of)

        # 4. reflectance + smoothness
        spec = tex(slot_path("TexSlot4"), color=True)
        reflectance: Operand = (
            g.vec("MULTIPLY", color_of(spec), params.specular_color) if spec is not None else params.specular_color
        )
        smoothness = g.math("MULTIPLY", gloss, params.smoothness_scale * 0.8)

        # 5. micro detail
        enc_x, enc_y, _ = g.split(normal_enc)
        nx = g.math("SUBTRACT", g.math("MULTIPLY", enc_x, 2.0), 1.0)
        ny = g.math("SUBTRACT", g.math("MULTIPLY", enc_y, 2.0), 1.0)
        micro_path = slot_path("TexSlot6")
        if micro_path and params.micro_detail_tiling > 0.0:
            tiling = 65.0 * params.micro_detail_tiling
            micro_uv = g.join(g.math("MULTIPLY", u, tiling), g.math("MULTIPLY", v, tiling * 0.25), 0.0)
            micro = tex(micro_path, color=False, vector=micro_uv)
            if micro is not None:
                # CryEngine detail-map layout: normal X in alpha, Y in green.
                dx = g.math("SUBTRACT", g.math("MULTIPLY", alpha_of(micro), 2.0), 1.0)
                dy = g.math("SUBTRACT", g.math("MULTIPLY", g.split(color_of(micro))[1], 2.0), 1.0)
                nx = g.math("ADD", nx, g.math("MULTIPLY", dx, micro_scale))
                ny = g.math("ADD", ny, g.math("MULTIPLY", dy, micro_scale))
                peak = g.math("MAXIMUM", g.math("ABSOLUTE", dx), g.math("ABSOLUTE", dy))
                blend = g.math("MULTIPLY", g.math("POWER", peak, 2.0), 40.0, clamp=True)
                smoothness = g.lerp(
                    g.math("MULTIPLY", smoothness, 0.95),
                    g.math("MINIMUM", g.math("MULTIPLY", smoothness, 1.15), 0.95),
                    blend,
                )

        # 6. surface
        principled = self._create_surface_bsdf(nodes)
        base_color = _input_socket(principled, "Base Color")
        if isinstance(albedo, tuple):
            base_color.default_value = (*albedo, 1.0)
        else:
            links.new(albedo, base_color)
        links.new(g.math("SUBTRACT", 1.0, smoothness, clamp=True), _input_socket(principled, "Roughness"))
        _input_socket(principled, "IOR").default_value = 1.5
        f0 = g.vec("DOT_PRODUCT", reflectance, LUMINANCE)
        links.new(g.math("DIVIDE", f0, 0.08), _input_socket(principled, "Specular IOR Level", "Specular"))

        nz = g.math(
            "SQRT",
            g.math("MAXIMUM", g.math("SUBTRACT", 1.0, g.math("ADD", g.math("POWER", nx, 2.0), g.math("POWER", ny, 2.0))), 0.0),
        )
        encoded = g.join(*(g.math("ADD", g.math("MULTIPLY", c, 0.5), 0.5) for c in (nx, ny, nz)))
        normal_map = nodes.new("ShaderNodeNormalMap")
        links.new(encoded, _input_socket(normal_map, "Color"))
        links.new(_output_socket(normal_map, "Normal"), _input_socket(principled, "Normal"))

        _input_socket(principled, "Subsurface Weight", "Subsurface").default_value = 1.0
        radius = _input_socket(principled, "Subsurface Radius")
        if radius is not None:
            radius.default_value = SKIN_SUBSURFACE_RADIUS
        scale = _input_socket(principled, "Subsurface Scale")
        translucency = tex(slot_path("TexSlot12"), color=False)
        if scale is not None:
            if translucency is not None and params.translucency > 0.0:
                transmittance = g.math("MULTIPLY", g.split(color_of(translucency))[0], params.translucency * 0.25)
                widen = g.math("ADD", 1.0, g.math("MULTIPLY", transmittance, SKIN_TRANSMITTANCE_SCALE_GAIN))
                links.new(g.math("MULTIPLY", widen, SKIN_SUBSURFACE_SCALE), scale)
            else:
                scale.default_value = SKIN_SUBSURFACE_SCALE

        output = nodes.new("ShaderNodeOutputMaterial")
        self._wire_surface_shader_to_output(nodes, links, principled.outputs[0], output, plan, submaterial)
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    @staticmethod
    def _skin_wrinkle_weights(
        g: NodeMath,
        params: HumanSkinParams,
        mask_path: str | None,
        u: Socket,
        v: Socket,
        tex: Any,
        color_of: Any,
        alpha_of: Any,
    ) -> tuple[Socket, Socket] | None:
        """Per-tile wrinkle weights (tile 1 = stretch, tile 2 = compress).

        Each of the 24 slots reads one 64-level band of one mask channel
        (``DecodeWrinkleMaskChannel``) and adds ``ramp · wrinkle_NN`` to its
        tile's accumulator.
        """
        if not params.wrinkle_slots or not mask_path:
            return None
        panels = []
        for p in range(2):
            vector = g.join(g.math("ADD", g.math("MULTIPLY", u, 0.5), 0.5 * p), v, 0.0)
            node = tex(mask_path, color=False, vector=vector, interpolation="Closest", extension="CLIP")
            if node is None:
                return None
            panels.append([*g.split(color_of(node)), alpha_of(node)])
        acc: dict[int, Socket | None] = {1: None, 2: None}
        for i, slot in enumerate(params.wrinkle_slots):
            weight = g.value(f"wrinkle_{i:02d}")
            level = g.math("MULTIPLY", panels[slot.panel][slot.channel], 255.0)
            ramp = g.math("MULTIPLY", g.math("SUBTRACT", level, 64.0 * slot.band), 1.0 / 63.0, clamp=True)
            contribution = g.math("MULTIPLY", ramp, weight)
            if slot.band < 3:
                contribution = g.math("MULTIPLY", contribution, g.math("LESS_THAN", level, 64.0 * slot.band + 63.5))
            previous = acc.get(slot.tile)
            acc[slot.tile] = contribution if previous is None else g.math("ADD", previous, contribution)
        return (acc[1] if acc[1] is not None else 0.0, acc[2] if acc[2] is not None else 0.0)

    @staticmethod
    def _skin_tone_adjustment(g: NodeMath, params: HumanSkinParams, albedo: Socket, mask_r: Socket, mask_g: Socket) -> Socket:
        source = params.source_average
        final = params.final_tone
        detail_scale, gb_desaturate = tone_constants(source, final)
        dr, dg, db = g.split(g.vec("ABSOLUTE", g.vec("SUBTRACT", albedo, source)))
        col_delta = g.math("MAXIMUM", g.math("MAXIMUM", dr, dg), db)
        amount = g.lerp(
            params.source_detail,
            min(1.0, params.source_detail),
            g.math("MULTIPLY", col_delta, 4.0, clamp=True),
        )
        amount = g.math("MULTIPLY", g.math("MULTIPLY", amount, detail_scale), mask_g)
        safe_source = tuple(max(c, 1e-6) for c in source)
        cr, cg, cb = g.split(g.vec("DIVIDE", albedo, safe_source))
        if gb_desaturate > 0.0:
            mixed = g.math("ADD", g.math("MULTIPLY", cg, 0.7), g.math("MULTIPLY", cb, 0.3))
            cg, cb = g.lerp(cg, mixed, gb_desaturate), g.lerp(cb, mixed, gb_desaturate)
        adjusted = g.join(
            *(g.math("MULTIPLY", g.math("POWER", c, amount), tone, clamp=True) for c, tone in zip((cr, cg, cb), final))
        )
        return g.vlerp(albedo, adjusted, mask_r)

    @staticmethod
    def _skin_blemishes(g: NodeMath, params: HumanSkinParams, albedo: Socket, tex: Any, color_of: Any) -> Socket:
        mask = tex(params.override_textures["TexSlot5"], color=False)
        ids = tex(params.override_textures["TexSlot10"], color=False)
        if mask is None or ids is None:
            return albedo
        lum = g.vec("DOT_PRODUCT", albedo, LUMINANCE)
        normalized = g.math(
            "DIVIDE",
            g.math("SUBTRACT", lum, _BLEMISH_MIN_LUMINANCE),
            _BLEMISH_MAX_LUMINANCE - _BLEMISH_MIN_LUMINANCE,
            clamp=True,
        )
        inverse = g.math("SUBTRACT", 1.0, normalized)
        mask_channels = g.split(color_of(mask))
        id_channels = g.split(color_of(ids))
        layers = ((0, params.freckles_amount, params.freckles_opacity), (1, params.sunspots_amount, params.sunspots_opacity))
        for channel, amount, opacity in layers:
            if opacity <= 0.0:
                continue
            blemish_id = id_channels[channel]
            hit = g.math("SUBTRACT", 1.0, g.math("LESS_THAN", amount, blemish_id))  # step(id, amount)
            blend = g.math("MULTIPLY", g.math("MULTIPLY", hit, mask_channels[channel]), opacity)
            offset = g.math("MULTIPLY", g.math("SUBTRACT", 1.0, blemish_id), 0.2)
            melanin = g.math("MINIMUM", g.math("ADD", inverse, offset), 1.0)
            colour = g.vlerp(_PALE_MELANIN, _DARK_MELANIN, melanin)
            albedo = g.vlerp(albedo, colour, blend)
        return albedo
