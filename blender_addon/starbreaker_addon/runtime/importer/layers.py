"""Layer/wear/iridescence/detail/stencil-overlay helpers for ``PackageImporter``.

Extracted from ``runtime/_legacy.py``. Owns:

* Manifest layer surface group connection
  (``_connect_manifest_layer_surface_group``, ``_connect_layer_surface_group``,
  ``_link_group_input``).
* Detail texture wiring (``_detail_texture_channels``, ``_apply_detail_color``,
  ``_apply_detail_gloss``).
* HardSurface stencil overlay (``_hard_surface_stencil_overlay_sockets``).
* Wear/damage factor sockets (``_wear_strength``,
  ``_layered_wear_factor_socket``, ``_layered_damage_factor_socket``,
  ``_mix_layered_base_color``, ``_mix_layered_roughness``,
  ``_layered_wear_layer``, ``_layer_color_socket``, ``_layer_roughness_socket``).
* HardSurface angle/iridescence helpers (``_hard_surface_angle_factor_socket``,
  ``_iridescence_ramp_color_socket``).

All cross-mixin calls resolve via composed MRO; module-level helpers and
dataclasses still living in ``_legacy.py`` (``LayerSurfaceSockets``,
``StencilOverlaySockets``, ``SocketRef``) are pulled in via the lazy
``_legacy_attr`` shim.
"""

from __future__ import annotations

from typing import Any

import bpy

from ...manifest import LayerManifestEntry, PaletteRecord, SubmaterialRecord
from ...palette import palette_finish_glossiness, palette_finish_specular
from ...templates import representative_textures
from ..constants import SCENE_WEAR_STRENGTH_PROP
from ..node_utils import _input_socket, _output_socket, _refresh_group_node_sockets
from ..record_utils import (
    _authored_attribute_triplet,
    _float_layer_public_param,
    _float_public_param,
    _layer_snapshot_float,
    _layer_snapshot_triplet,
    _layer_texture_reference,
    _mean_triplet,
    _optional_float_public_param,
    _public_param_triplet,
    _resolved_submaterial_palette_color,
    _submaterial_texture_reference,
)
from .types import LayerSurfaceSockets, SocketRef, StencilOverlaySockets


class LayersMixin:
    """Layer/wear/detail/stencil/iridescence wiring for ``PackageImporter``."""

    # ------------------------------------------------------------------
    # Manifest layer surface group plumbing
    # ------------------------------------------------------------------
    def _connect_manifest_layer_surface_group(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        layer: LayerManifestEntry | None,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
        label: str,
        detail_slots: tuple[str, ...],
    ) -> Any:
        if layer is None:
            return LayerSurfaceSockets()

        base_texture = _layer_texture_reference(
            layer, slots=("TexSlot1",), roles=("base_color", "diffuse")
        )
        base_node = self._image_node(
            nodes,
            base_texture.export_path if base_texture is not None else None,
            x=x,
            y=y,
            is_color=True,
        )
        detail_ref = _layer_texture_reference(layer, slots=detail_slots)
        detail_channels = self._detail_texture_channels(
            nodes,
            detail_ref.export_path if detail_ref is not None else None,
            x=x,
            y=y - 420,
        )
        normal_ref = _layer_texture_reference(
            layer, roles=("normal_gloss",), alpha_semantic="smoothness"
        )
        normal_node = self._image_node(
            nodes,
            normal_ref.export_path if normal_ref is not None else None,
            x=x,
            y=y - 560,
            is_color=False,
        )
        roughness, roughness_is_smoothness = self._roughness_socket_for_texture_reference(
            nodes, normal_ref, x=x + 180, y=y - 560
        )
        layer_channel_name = (
            layer.palette_channel.name if layer.palette_channel is not None else None
        )
        return self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=base_node.outputs[0] if base_node is not None else None,
            base_alpha_socket=_output_socket(base_node, "Alpha") if base_node is not None else None,
            normal_color_socket=normal_node.outputs[0] if normal_node is not None else None,
            roughness_socket=roughness,
            roughness_source_is_smoothness=roughness_is_smoothness,
            detail_channels=detail_channels,
            detail_diffuse_strength=max(
                0.0, min(1.0, _float_layer_public_param(layer, "DetailDiffuse"))
            ),
            detail_gloss_strength=max(
                0.0, min(1.0, _float_layer_public_param(layer, "DetailGloss"))
            ),
            detail_bump_strength=max(0.0, _float_layer_public_param(layer, "DetailBump")),
            tint_color=layer.tint_color,
            palette=palette,
            palette_channel_name=layer_channel_name,
            palette_finish_channel_name=layer_channel_name,
            palette_glossiness=palette_finish_glossiness(palette, layer_channel_name),
            specular_value=_mean_triplet(_layer_snapshot_triplet(layer, "specular")) or 0.0,
            palette_specular_value=_mean_triplet(
                palette_finish_specular(palette, layer_channel_name)
            )
            or 0.0,
            metallic_value=_layer_snapshot_float(layer, "metallic"),
            specular_color=_layer_snapshot_triplet(layer, "specular"),
            x=x + 420,
            y=y - 120,
            label=label,
        )

    def _connect_layer_surface_group(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        *,
        base_color_socket: Any,
        base_alpha_socket: Any,
        normal_color_socket: Any,
        roughness_socket: Any,
        roughness_source_is_smoothness: bool,
        detail_channels: dict[str, Any] | None,
        detail_diffuse_strength: float,
        detail_gloss_strength: float,
        detail_bump_strength: float,
        tint_color: tuple[float, float, float] | None,
        palette: PaletteRecord | None,
        palette_channel_name: str | None,
        palette_finish_channel_name: str | None,
        palette_glossiness: float | None,
        specular_value: float,
        palette_specular_value: float,
        metallic_value: float,
        specular_color: tuple[float, float, float] | None,
        x: int,
        y: int,
        label: str,
    ) -> Any:
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_layer_surface_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = label

        self._set_socket_default(_input_socket(group_node, "Base Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(group_node, "Base Alpha"), 1.0)
        self._set_socket_default(_input_socket(group_node, "Palette Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(
            _input_socket(group_node, "Tint Color"),
            (*tint_color, 1.0) if tint_color is not None else (1.0, 1.0, 1.0, 1.0),
        )
        self._set_socket_default(
            _input_socket(group_node, "Detail Diffuse Strength"), detail_diffuse_strength
        )
        self._set_socket_default(
            _input_socket(group_node, "Detail Gloss Strength"), detail_gloss_strength
        )
        self._set_socket_default(
            _input_socket(group_node, "Detail Bump Strength"), detail_bump_strength
        )
        self._set_socket_default(_input_socket(group_node, "Normal Color"), (0.5, 0.5, 1.0, 1.0))
        self._set_socket_default(_input_socket(group_node, "Roughness Source"), 0.45)
        self._set_socket_default(
            _input_socket(group_node, "Roughness Source Is Smoothness"),
            roughness_source_is_smoothness,
        )
        self._set_socket_default(
            _input_socket(group_node, "Palette Glossiness"),
            max(0.0, min(1.0, palette_glossiness)) if palette_glossiness is not None else 0.0,
        )
        self._set_socket_default(_input_socket(group_node, "Specular Value"), specular_value)
        self._set_socket_default(
            _input_socket(group_node, "Palette Specular"), palette_specular_value
        )
        self._set_socket_default(_input_socket(group_node, "Metallic"), metallic_value)
        self._set_socket_default(
            _input_socket(group_node, "Specular Color"),
            (*specular_color, 1.0) if specular_color is not None else (0.04, 0.04, 0.04, 1.0),
        )
        if palette_finish_channel_name is not None:
            group_node["starbreaker_palette_finish_channel"] = palette_finish_channel_name

        palette_color_socket = None
        palette_gloss_socket = None
        palette_specular_socket = None
        palette_specular_tint_socket = None
        if palette is not None and palette_channel_name is not None:
            palette_color_socket = self._palette_color_socket(
                nodes, palette, palette_channel_name, x=x - 220, y=y - 160
            )
            finish_channel_name = palette_finish_channel_name or palette_channel_name
            palette_gloss_socket = self._palette_glossiness_socket(
                nodes, palette, finish_channel_name, x=x - 220, y=y - 320
            )
            palette_specular_color = self._palette_specular_socket(
                nodes, palette, finish_channel_name, x=x - 220, y=y - 480
            )
            if palette_specular_color is not None:
                palette_specular_tint_socket = palette_specular_color
                luma_group = nodes.new("ShaderNodeGroup")
                luma_group.node_tree = self._ensure_runtime_color_to_luma_group()
                luma_group.location = (x - 20, y - 480)
                luma_group.label = "StarBreaker Color To Luma"
                links.new(palette_specular_color, luma_group.inputs["Color"])
                palette_specular_socket = luma_group.outputs["Luma"]

        self._link_group_input(links, base_color_socket, group_node, "Base Color")
        self._link_group_input(links, base_alpha_socket, group_node, "Base Alpha")
        self._link_group_input(links, normal_color_socket, group_node, "Normal Color")
        self._link_group_input(links, roughness_socket, group_node, "Roughness Source")
        self._link_group_input(links, palette_color_socket, group_node, "Palette Color")
        self._link_group_input(links, palette_gloss_socket, group_node, "Palette Glossiness")
        self._link_group_input(links, palette_specular_socket, group_node, "Palette Specular")
        if detail_channels is not None:
            self._link_group_input(
                links, detail_channels.get("red"), group_node, "Detail Color Mask"
            )
            self._link_group_input(
                links, detail_channels.get("green"), group_node, "Detail Height Mask"
            )
            self._link_group_input(
                links, detail_channels.get("blue"), group_node, "Detail Gloss Mask"
            )

        return LayerSurfaceSockets(
            color=SocketRef(group_node, "Color"),
            alpha=SocketRef(group_node, "Alpha"),
            normal=SocketRef(group_node, "Normal"),
            roughness=SocketRef(group_node, "Roughness"),
            specular=SocketRef(group_node, "Specular"),
            specular_tint=(
                SocketRef(palette_specular_tint_socket.node, palette_specular_tint_socket.name)
                if palette_specular_tint_socket is not None
                else None
            ),
            metallic=SocketRef(group_node, "Metallic"),
        )

    def _link_group_input(
        self,
        links: bpy.types.NodeLinks,
        source_socket: Any,
        group_node: bpy.types.Node,
        socket_name: str,
    ) -> None:
        if source_socket is None:
            return
        if isinstance(source_socket, SocketRef):
            _refresh_group_node_sockets(source_socket.node)
            source_socket = (
                _output_socket(source_socket.node, source_socket.name)
                if source_socket.is_output
                else _input_socket(source_socket.node, source_socket.name)
            )
            if source_socket is None:
                return
        _refresh_group_node_sockets(group_node)
        target_socket = _input_socket(group_node, socket_name)
        if target_socket is None:
            return
        if not getattr(source_socket, "is_output", False) or getattr(
            target_socket, "is_output", False
        ):
            return
        try:
            links.new(source_socket, target_socket)
        except RuntimeError as exc:
            if "Same input/output direction of sockets" in str(exc):
                return
            raise

    # ------------------------------------------------------------------
    # Detail texture wiring
    # ------------------------------------------------------------------
    def _detail_texture_channels(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> dict[str, Any] | None:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x + 180, y)
        group_node.node_tree = self._ensure_runtime_channel_split_group()
        group_node.label = "StarBreaker Channel Split"
        links = image_node.id_data.links
        links.new(image_node.outputs[0], group_node.inputs["Color"])
        alpha_socket = _output_socket(image_node, "Alpha")
        if alpha_socket is not None:
            links.new(alpha_socket, group_node.inputs["Alpha"])
        return {
            "red": group_node.outputs["R"],
            "green": group_node.outputs["G"],
            "blue": group_node.outputs["B"],
            "alpha": group_node.outputs["Alpha"],
        }

    def _apply_detail_color(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        color_socket: Any,
        detail_channels: dict[str, Any] | None,
        *,
        strength: float,
        x: int,
        y: int,
    ) -> Any:
        if (
            color_socket is None
            or detail_channels is None
            or detail_channels.get("red") is None
            or strength <= 0.0
        ):
            return color_socket
        grayscale = nodes.new("ShaderNodeValToRGB")
        grayscale.location = (x, y)
        links.new(detail_channels["red"], grayscale.inputs[0])
        white = self._value_color_socket(nodes, (1.0, 1.0, 1.0, 1.0), x=x, y=y - 120)
        tint_mix = nodes.new("ShaderNodeMixRGB")
        tint_mix.location = (x + 180, y)
        tint_mix.blend_type = "MIX"
        tint_mix.inputs[0].default_value = max(0.0, min(1.0, strength))
        self._link_color_output(white, tint_mix.inputs[1])
        links.new(grayscale.outputs[0], tint_mix.inputs[2])
        return self._multiply_color_socket(
            nodes, links, color_socket, tint_mix.outputs[0], x=x + 360, y=y
        )

    def _apply_detail_gloss(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        roughness_socket: Any,
        detail_channels: dict[str, Any] | None,
        *,
        strength: float,
        x: int,
        y: int,
    ) -> Any:
        if (
            roughness_socket is None
            or detail_channels is None
            or detail_channels.get("blue") is None
            or strength <= 0.0
        ):
            return roughness_socket
        detail_value = nodes.new("ShaderNodeMix")
        detail_value.location = (x, y)
        if hasattr(detail_value, "data_type"):
            detail_value.data_type = "FLOAT"
        detail_value.inputs[0].default_value = max(0.0, min(1.0, strength))
        detail_value.inputs[2].default_value = 1.0
        links.new(detail_channels["blue"], detail_value.inputs[3])
        return self._multiply_value_socket(
            nodes, links, roughness_socket, detail_value.outputs[0], x=x + 180, y=y
        )

    # ------------------------------------------------------------------
    # HardSurface stencil overlay
    # ------------------------------------------------------------------
    def _hard_surface_stencil_overlay_sockets(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        if not submaterial.decoded_feature_flags.has_stencil_map:
            return StencilOverlaySockets()

        stencil_ref = _submaterial_texture_reference(
            submaterial,
            slots=("TexSlot7",),
            roles=("stencil", "stencil_source", "tint_palette_decal"),
        )
        if stencil_ref is None or stencil_ref.export_path is None:
            return StencilOverlaySockets()

        stencil_tiling = _optional_float_public_param(submaterial, "StencilTiling")
        stencil_uv_map = (
            "UVMap.001"
            if (_optional_float_public_param(submaterial, "UseUV2ForStencil") or 0.0) > 0.5
            else None
        )
        stencil_node = self._tiled_image_node(
            nodes,
            links,
            stencil_ref.export_path,
            x=x,
            y=y,
            is_color=True,
            tiling=stencil_tiling
            if stencil_tiling is not None and stencil_tiling > 0.0
            else 1.0,
            uv_map_name=stencil_uv_map,
        )
        if stencil_node is None:
            return StencilOverlaySockets()

        tint = _public_param_triplet(
            submaterial,
            "StencilDiffuseColor1",
            "StencilDiffuse1",
            "StencilTintColor",
            "TintColor",
            "StencilDiffuseColor",
        )
        tint_2 = _public_param_triplet(submaterial, "StencilDiffuseColor2", "StencilDiffuse2")
        tint_3 = _public_param_triplet(submaterial, "StencilDiffuseColor3", "StencilDiffuse3")
        specular_1 = _public_param_triplet(
            submaterial, "StencilSpecularColor1", "StencilSpecular1", "StencilSpecularColor"
        )
        specular_2 = _public_param_triplet(submaterial, "StencilSpecularColor2", "StencilSpecular2")
        specular_3 = _public_param_triplet(submaterial, "StencilSpecularColor3", "StencilSpecular3")
        multi_channel_stencil = any(
            value is not None for value in (tint_2, tint_3, specular_2, specular_3)
        )
        tint_override = _optional_float_public_param(submaterial, "StencilTintOverride") or 0.0
        stencil_glossiness = _optional_float_public_param(submaterial, "StencilGlossiness")
        opacity = _optional_float_public_param(submaterial, "StencilOpacity")

        breakup_node = None
        breakup_strength = 0.0
        breakup_ref = _submaterial_texture_reference(
            submaterial, slots=("TexSlot8",), roles=("breakup", "grime_breakup")
        )
        if breakup_ref is not None and breakup_ref.export_path is not None:
            breakup_tiling = _optional_float_public_param(submaterial, "StencilBreakupTiling")
            breakup_node = self._tiled_image_node(
                nodes,
                links,
                breakup_ref.export_path,
                x=x,
                y=y - 220,
                is_color=False,
                tiling=breakup_tiling
                if breakup_tiling is not None and breakup_tiling > 0.0
                else 1.0,
                uv_map_name=stencil_uv_map,
            )
            breakup_strength = max(
                0.0,
                min(
                    1.0,
                    _optional_float_public_param(
                        submaterial, "StencilDiffuseBreakup", "StencilGlossBreakup"
                    )
                    or 0.0,
                ),
            )

        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x + 420, y)
        group_node.node_tree = self._ensure_runtime_hardsurface_stencil_group()
        group_node.label = "StarBreaker HardSurface Stencil"

        links.new(stencil_node.outputs[0], group_node.inputs["Stencil Color"])
        stencil_alpha_socket = _output_socket(stencil_node, "Alpha")
        if stencil_alpha_socket is not None:
            links.new(stencil_alpha_socket, group_node.inputs["Stencil Alpha"])
        if breakup_node is not None:
            links.new(breakup_node.outputs[0], group_node.inputs["Breakup Color"])
            breakup_alpha_socket = _output_socket(breakup_node, "Alpha")
            if breakup_alpha_socket is not None:
                links.new(breakup_alpha_socket, group_node.inputs["Breakup Alpha"])
            group_node.inputs["Breakup Strength"].default_value = breakup_strength
            group_node.inputs["Breakup Enable"].default_value = (
                1.0 if breakup_strength > 0.0 else 0.0
            )
        else:
            group_node.inputs["Breakup Enable"].default_value = 0.0

        if opacity is not None:
            group_node.inputs["Stencil Opacity"].default_value = max(0.0, min(1.0, opacity))
        if stencil_glossiness is not None:
            group_node.inputs["Stencil Glossiness"].default_value = max(
                0.0, min(1.0, stencil_glossiness)
            )

        use_override = stencil_ref.is_virtual or tint_override > 0.0
        if use_override:
            group_node.inputs["Mode"].default_value = 0.0
            group_node.inputs["Tint1"].default_value = (
                *(tint if tint is not None else (1.0, 1.0, 1.0)),
                1.0,
            )
            group_node.inputs["Tint1 Enable"].default_value = 1.0
            group_node.inputs["Tint2 Enable"].default_value = 0.0
            group_node.inputs["Tint3 Enable"].default_value = 0.0
        elif multi_channel_stencil:
            group_node.inputs["Mode"].default_value = 1.0
            for slot, color in (("Tint1", tint), ("Tint2", tint_2), ("Tint3", tint_3)):
                enable_key = f"{slot} Enable"
                if color is not None and (_mean_triplet(color) or 0.0) > 0.01:
                    group_node.inputs[slot].default_value = (*color, 1.0)
                    group_node.inputs[enable_key].default_value = 1.0
                else:
                    group_node.inputs[enable_key].default_value = 0.0
        else:
            group_node.inputs["Mode"].default_value = 0.0
            group_node.inputs["Tint1 Enable"].default_value = 0.0
            group_node.inputs["Tint2 Enable"].default_value = 0.0
            group_node.inputs["Tint3 Enable"].default_value = 0.0

        for slot, color in (
            ("Specular1", specular_1),
            ("Specular2", specular_2),
            ("Specular3", specular_3),
        ):
            enable_key = f"{slot} Enable"
            if color is not None and (_mean_triplet(color) or 0.0) > 0.0:
                group_node.inputs[slot].default_value = (*color, 1.0)
                group_node.inputs[enable_key].default_value = 1.0
            else:
                group_node.inputs[enable_key].default_value = 0.0

        has_specular = any(
            c is not None and (_mean_triplet(c) or 0.0) > 0.0
            for c in (specular_1, specular_2, specular_3)
        )

        return StencilOverlaySockets(
            color=group_node.outputs["Color"],
            color_factor=group_node.outputs["Color Factor"],
            factor=group_node.outputs["Factor"],
            roughness=group_node.outputs["Roughness"] if stencil_glossiness is not None else None,
            specular=group_node.outputs["Specular"] if has_specular else None,
            specular_tint=group_node.outputs["Specular Tint"] if has_specular else None,
        )

    # ------------------------------------------------------------------
    # Wear / damage / iridescence / angle factor sockets
    # ------------------------------------------------------------------
    def _wear_strength(self) -> float:
        raw_value = getattr(self.context.scene, SCENE_WEAR_STRENGTH_PROP, 0.0)
        try:
            value = float(raw_value)
        except (TypeError, ValueError):
            value = 0.0
        return max(0.0, min(2.0, value))

    def _hard_surface_angle_factor_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        *,
        x: int,
        y: int,
    ) -> Any:
        layer_weight = nodes.new("ShaderNodeLayerWeight")
        layer_weight.location = (x, y)
        blend_input = _input_socket(layer_weight, "Blend")
        if blend_input is not None:
            blend_input.default_value = 0.3

        angle_factor = nodes.new("ShaderNodeMapRange")
        angle_factor.location = (x + 140, y + 100)
        angle_factor.clamp = True
        angle_factor.inputs[1].default_value = 0.0
        angle_factor.inputs[2].default_value = 0.2
        angle_factor.inputs[3].default_value = 0.0
        angle_factor.inputs[4].default_value = 1.0
        links.new(_output_socket(layer_weight, "Facing"), angle_factor.inputs[0])
        return angle_factor.outputs[0]

    def _iridescence_ramp_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        ramp_path = self._texture_path_for_slot(submaterial, "TexSlot10")
        if ramp_path is None:
            return None

        ramp_node = self._image_node(nodes, ramp_path, x=x + 360, y=y, is_color=True)
        if ramp_node is None:
            return None
        if hasattr(ramp_node, "extension"):
            ramp_node.extension = "EXTEND"

        thickness_u = _optional_float_public_param(submaterial, "IridescenceThicknessU")
        thickness_v = _optional_float_public_param(submaterial, "IridescenceThicknessV")

        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_iridescence_input_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x + 180, y)
        group_node.label = "StarBreaker Iridescence"
        self._set_socket_default(
            _input_socket(group_node, "Thickness U"),
            max(0.0, min(1.0, thickness_u if thickness_u is not None else 1.0)),
        )
        self._set_socket_default(
            _input_socket(group_node, "Thickness V"),
            max(0.0, min(1.0, thickness_v if thickness_v is not None else 0.5)),
        )

        ramp_uv = _output_socket(group_node, "Ramp UV")
        vector_input = _input_socket(ramp_node, "Vector")
        if ramp_uv is not None and vector_input is not None:
            links.new(ramp_uv, vector_input)
        return ramp_node.outputs[0]

    def _layered_wear_factor_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        textures = representative_textures(submaterial)
        has_vertex_colors = submaterial.decoded_feature_flags.has_vertex_colors
        wear_base = _float_public_param(submaterial, "WearBlendBase", "DamagePerObjectWear")
        mask_node = None
        if not has_vertex_colors:
            mask_node = self._image_node(nodes, textures["mask"], x=x - 220, y=y, is_color=False)
        if not has_vertex_colors and mask_node is None and wear_base <= 0.0:
            return None

        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_wear_input_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = "StarBreaker Wear"
        self._set_socket_default(_input_socket(group_node, "Wear Mask"), 0.0)
        self._set_socket_default(
            _input_socket(group_node, "Use Vertex Colors"), 1.0 if has_vertex_colors else 0.0
        )
        self._set_socket_default(
            _input_socket(group_node, "Wear Base"),
            max(0.0, wear_base if wear_base > 0.0 else 1.0),
        )
        self._set_socket_default(_input_socket(group_node, "Wear Strength"), self._wear_strength())
        self._set_socket_default(_input_socket(group_node, "Use Damage"), 0.0)

        if not has_vertex_colors and mask_node is not None:
            self._link_group_input(links, mask_node.outputs[0], group_node, "Wear Mask")

        return _output_socket(group_node, "Wear Factor")

    def _layered_damage_factor_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        if not submaterial.decoded_feature_flags.has_damage_map:
            return None
        if not submaterial.decoded_feature_flags.has_vertex_colors:
            return None

        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_wear_input_group()
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = "StarBreaker Damage"
        self._set_socket_default(_input_socket(group_node, "Wear Mask"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Use Vertex Colors"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Wear Base"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Wear Strength"), 0.0)
        self._set_socket_default(_input_socket(group_node, "Use Damage"), 1.0)
        return _output_socket(group_node, "Damage Factor")

    def _mix_layered_base_color(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        base_socket: Any,
        wear_factor_socket: Any,
    ) -> Any:
        layer_color = self._layer_color_socket(nodes, submaterial, palette, x=40, y=320)
        if wear_factor_socket is None:
            return base_socket or layer_color
        if base_socket is None:
            return layer_color
        if layer_color is None:
            return base_socket

        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (320, 160)
        mix.blend_type = "MIX"
        links.new(wear_factor_socket, mix.inputs[0])
        self._link_color_output(base_socket, mix.inputs[1])
        self._link_color_output(layer_color, mix.inputs[2])
        return mix.outputs[0]

    def _mix_layered_roughness(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        base_source: Any,
        wear_factor_socket: Any,
        *,
        default_value: float,
    ) -> Any:
        layer_source = self._layer_roughness_socket(nodes, submaterial, x=80, y=-500)
        if wear_factor_socket is None:
            return base_source or layer_source
        if base_source is None:
            base_source = self._value_socket(nodes, default_value, x=260, y=-120)
        if layer_source is None:
            return base_source

        mix = nodes.new("ShaderNodeMix")
        mix.location = (320, -260)
        if hasattr(mix, "data_type"):
            mix.data_type = "FLOAT"
        links.new(wear_factor_socket, mix.inputs[0])
        links.new(base_source, mix.inputs[2])
        links.new(layer_source, mix.inputs[3])
        return mix.outputs[0]

    def _layered_wear_layer(
        self, submaterial: SubmaterialRecord
    ) -> LayerManifestEntry | None:
        if len(submaterial.layer_manifest) > 1:
            return submaterial.layer_manifest[1]
        if submaterial.layer_manifest:
            return submaterial.layer_manifest[0]
        return None

    def _layer_color_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        wear_layer = self._layered_wear_layer(submaterial)
        layer = (
            wear_layer if wear_layer is not None and wear_layer.diffuse_export_path else None
        )
        if layer is None:
            layer = next(
                (item for item in submaterial.layer_manifest if item.diffuse_export_path), None
            )
        if layer is None:
            return None

        source = self._image_node(nodes, layer.diffuse_export_path, x=x, y=y, is_color=True)
        if source is None:
            return None
        output = source.outputs[0]

        if layer.tint_color is not None and any(
            abs(channel - 1.0) > 1e-6 for channel in layer.tint_color
        ):
            tint = nodes.new("ShaderNodeRGB")
            tint.location = (x, y - 160)
            tint.outputs[0].default_value = (*layer.tint_color, 1.0)
            mix = nodes.new("ShaderNodeMixRGB")
            mix.location = (x + 180, y)
            mix.inputs[0].default_value = 1.0
            self._link_color_output(output, mix.inputs[1])
            self._link_color_output(tint.outputs[0], mix.inputs[2])
            output = mix.outputs[0]

        if layer.palette_channel is not None and palette is not None:
            palette_socket = self._palette_color_socket(
                nodes, palette, layer.palette_channel.name, x=x, y=y - 320
            )
            mix = nodes.new("ShaderNodeMixRGB")
            mix.location = (x + 360, y)
            mix.blend_type = "MULTIPLY"
            mix.inputs[0].default_value = 1.0
            self._link_color_output(output, mix.inputs[1])
            self._link_color_output(palette_socket, mix.inputs[2])
            output = mix.outputs[0]

        return output

    def _layer_roughness_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        wear_layer = self._layered_wear_layer(submaterial)
        layer = None
        if wear_layer is not None and (
            wear_layer.roughness_export_path
            or any(
                texture.alpha_semantic == "smoothness" and texture.export_path
                for texture in wear_layer.texture_slots
            )
        ):
            layer = wear_layer
        if layer is None:
            layer = next(
                (
                    item
                    for item in submaterial.layer_manifest
                    if item.roughness_export_path
                    or any(
                        texture.alpha_semantic == "smoothness" and texture.export_path
                        for texture in item.texture_slots
                    )
                ),
                None,
            )
        if layer is None:
            return None
        if layer.roughness_export_path:
            image_node = self._image_node(
                nodes, layer.roughness_export_path, x=x, y=y, is_color=False
            )
            if image_node is not None:
                return image_node.outputs[0]
        smoothness_texture = next(
            (
                texture
                for texture in layer.texture_slots
                if texture.alpha_semantic == "smoothness" and texture.export_path
            ),
            None,
        )
        if smoothness_texture is None:
            return None
        smoothness_alpha = self._texture_alpha_socket(
            nodes,
            smoothness_texture.export_path,
            x=x,
            y=y,
            is_color=False,
        )
        if smoothness_alpha is None:
            return None
        return self._invert_value_socket(nodes, smoothness_alpha, x=x + 180, y=y)
