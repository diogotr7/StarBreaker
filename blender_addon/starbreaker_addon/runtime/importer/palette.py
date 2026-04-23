"""Palette wiring helpers for ``PackageImporter``.

Extracted from ``runtime/_legacy.py``. Owns:

* The per-package ``StarBreaker Palette ...`` group builder, its
  decal-tint adaptor sub-group, and the per-material instance node.
* Per-channel socket lookups (``_palette_color_socket`` etc.) used by
  every other builder/mixin to source palette outputs.
* Post-build palette rewiring helpers
  (``_apply_palette_to_material``, ``_rewire_layer_palette_channels``,
  layer/hard-surface/virtual-tint palette default updaters,
  ``_apply_material_palette_tint``).

All cross-mixin calls (``self._image_node``,
``self._virtual_tint_palette_decal_defaults``, ``self._palette_scope``,
``self._multiply_color_socket``, ``self._set_socket_default``) resolve via
the composed ``PackageImporter`` MRO; they live in other mixins that the
class also inherits.

Module-level helpers still living in ``_legacy.py``
(``_palette_group_name``, ``_safe_identifier``,
``_palette_decal_or_fallback``, ``_palette_group_signature``,
``_palette_has_iridescence``) are pulled in via the lazy
``_legacy_attr`` shim so this module loads cleanly even when
``_legacy.py`` is being initialised at import time.
"""

from __future__ import annotations

import json
from typing import Any

import bpy

from ...manifest import PaletteRecord, SubmaterialRecord
from ...palette import (
    palette_color,
    palette_decal_texture,
    palette_finish_glossiness,
    palette_finish_specular,
)
from ..constants import PROP_PALETTE_ID, PROP_SUBMATERIAL_JSON
from ..node_utils import _input_socket, _output_socket, _refresh_group_node_sockets
from ..palette_utils import (
    _palette_decal_or_fallback,
    _palette_group_signature,
    _palette_has_iridescence,
)
from .types import LayerSurfaceSockets
from .utils import _palette_group_name, _safe_identifier


class PaletteMixin:
    """``PackageImporter`` mixin owning palette node groups and palette wiring."""

    # ------------------------------------------------------------------
    # Per-material palette application (post-build pass)
    # ------------------------------------------------------------------
    def _apply_palette_to_material(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
        palette_group: bpy.types.ShaderNodeTree,
    ) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        palette_node: bpy.types.Node | None = None
        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeGroup":
                continue
            node_tree_name = getattr(getattr(node, "node_tree", None), "name", "")
            if node_tree_name.startswith("StarBreaker Palette "):
                node.node_tree = palette_group
                node.label = f"StarBreaker Palette {palette.id}"
                palette_node = node
                continue
            if node_tree_name.startswith("StarBreaker Runtime LayerSurface"):
                self._update_layer_surface_palette_defaults(node, palette)
            if node_tree_name.startswith("StarBreaker Runtime HardSurface"):
                self._update_runtime_hard_surface_palette_defaults(node, palette)

        if palette_node is not None:
            self._rewire_layer_palette_channels(material, palette, palette_node)

        self._update_virtual_tint_palette_decal_nodes(material, palette)
        material[PROP_PALETTE_ID] = palette.id

    def _rewire_layer_palette_channels(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
        palette_node: bpy.types.Node,
    ) -> None:
        payload = material.get(PROP_SUBMATERIAL_JSON)
        if not isinstance(payload, str):
            return
        try:
            SubmaterialRecord.from_value(json.loads(payload))
        except Exception:
            return
        node_tree = material.node_tree
        if node_tree is None:
            return
        channel_socket_name = {
            "primary": "Primary",
            "secondary": "Secondary",
            "tertiary": "Tertiary",
            "glass": "Glass Color",
        }
        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeGroup":
                continue
            node_tree_name = getattr(getattr(node, "node_tree", None), "name", "")
            if not node_tree_name.startswith("StarBreaker Runtime LayerSurface"):
                continue
            finish_channel = node.get("starbreaker_palette_finish_channel")
            if not isinstance(finish_channel, str) or finish_channel not in channel_socket_name:
                continue
            target_socket_name = channel_socket_name.get(finish_channel, "Primary")
            palette_color_input = _input_socket(node, "Palette Color")
            if palette_color_input is None:
                continue
            current_source = (
                palette_color_input.links[0].from_socket.name
                if palette_color_input.is_linked
                else None
            )
            if current_source == target_socket_name:
                continue
            if palette_color_input.is_linked:
                node_tree.links.remove(palette_color_input.links[0])
            source_socket = _output_socket(palette_node, target_socket_name)
            if source_socket is not None:
                node_tree.links.new(source_socket, palette_color_input)

    def _update_layer_surface_palette_defaults(
        self,
        group_node: bpy.types.Node,
        palette: PaletteRecord,
    ) -> None:
        from ..record_utils import _mean_triplet

        channel_name = group_node.get("starbreaker_palette_finish_channel")
        if not isinstance(channel_name, str) or channel_name not in {
            "primary",
            "secondary",
            "tertiary",
            "glass",
        }:
            palette_color_input = _input_socket(group_node, "Palette Color")
            if palette_color_input is None or not palette_color_input.is_linked:
                return

            source_socket_name = palette_color_input.links[0].from_socket.name
            channel_name = {
                "Primary": "primary",
                "Secondary": "secondary",
                "Tertiary": "tertiary",
                "Glass Color": "glass",
            }.get(source_socket_name)
        if channel_name is None:
            return

        self._set_socket_default(
            _input_socket(group_node, "Palette Glossiness"),
            palette_finish_glossiness(palette, channel_name) or 0.0,
        )
        self._set_socket_default(
            _input_socket(group_node, "Palette Specular"),
            _mean_triplet(palette_finish_specular(palette, channel_name)) or 0.0,
        )

    def _update_runtime_hard_surface_palette_defaults(
        self,
        group_node: bpy.types.Node,
        palette: PaletteRecord,
    ) -> None:
        if not bool(group_node.get("starbreaker_angle_shift_enabled", False)):
            return
        iridescence_active = _palette_has_iridescence(palette)
        factor_socket = _input_socket(group_node, "Iridescence Factor")
        if factor_socket is not None and hasattr(factor_socket, "default_value"):
            factor_socket.default_value = 1.0 if iridescence_active else 0.0

    def _update_virtual_tint_palette_decal_nodes(
        self,
        material: bpy.types.Material,
        palette: PaletteRecord,
    ) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        color_node = next(
            (
                node
                for node in node_tree.nodes
                if node.bl_idname == "ShaderNodeRGB"
                and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"
            ),
            None,
        )
        alpha_node = next(
            (
                node
                for node in node_tree.nodes
                if node.bl_idname == "ShaderNodeValue"
                and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"
            ),
            None,
        )
        if color_node is None and alpha_node is None:
            return

        payload = material.get(PROP_SUBMATERIAL_JSON)
        if not isinstance(payload, str):
            return
        try:
            submaterial = SubmaterialRecord.from_value(json.loads(payload))
        except Exception:
            return

        color, alpha = self._virtual_tint_palette_decal_defaults(submaterial, palette)
        if color_node is not None:
            color_node.outputs[0].default_value = (*color, 1.0)
        if alpha_node is not None:
            alpha_node.outputs[0].default_value = alpha

    # ------------------------------------------------------------------
    # Per-material palette tint helper
    # ------------------------------------------------------------------
    def _apply_material_palette_tint(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        color_socket: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        channel = submaterial.palette_routing.material_channel
        if color_socket is None or channel is None or palette is None:
            return color_socket
        palette_socket = self._palette_color_socket(nodes, palette, channel.name, x=x, y=y)
        return self._multiply_color_socket(
            nodes, links, color_socket, palette_socket, x=x + 180, y=y
        )

    # ------------------------------------------------------------------
    # Palette node group: builder, instance, per-channel socket lookups
    # ------------------------------------------------------------------
    def _ensure_tint_decal_adaptor_group(self) -> bpy.types.ShaderNodeTree:
        group_name = "SB_Tint_Decal_Adaptor"
        group = bpy.data.node_groups.get(group_name)
        if group is not None:
            existing_inputs = {
                item.name
                for item in group.interface.items_tree
                if getattr(item, "item_type", None) == "SOCKET"
                and getattr(item, "in_out", None) == "INPUT"
            }
            existing_outputs = {
                item.name
                for item in group.interface.items_tree
                if getattr(item, "item_type", None) == "SOCKET"
                and getattr(item, "in_out", None) == "OUTPUT"
            }
            expected_inputs = {"Image", "Decal Red Tint", "Decal Green Tint", "Decal Blue Tint"}
            expected_outputs = {"Color", "Alpha"}
            if expected_inputs.issubset(existing_inputs) and expected_outputs.issubset(
                existing_outputs
            ):
                return group
        if group is None:
            group = bpy.data.node_groups.new(group_name, "ShaderNodeTree")
        for item in list(group.interface.items_tree):
            group.interface.remove(item)
        group.nodes.clear()
        group.interface.new_socket(name="Image", in_out="INPUT", socket_type="NodeSocketColor")
        for tint_name in ("Decal Red Tint", "Decal Green Tint", "Decal Blue Tint"):
            sock = group.interface.new_socket(
                name=tint_name, in_out="INPUT", socket_type="NodeSocketColor"
            )
            if hasattr(sock, "default_value"):
                sock.default_value = (1.0, 1.0, 1.0, 1.0)
        group.interface.new_socket(name="Color", in_out="OUTPUT", socket_type="NodeSocketColor")
        group.interface.new_socket(name="Alpha", in_out="OUTPUT", socket_type="NodeSocketFloat")

        group_input = group.nodes.new("NodeGroupInput")
        group_input.location = (-380, 200)
        group_output = group.nodes.new("NodeGroupOutput")
        group_output.location = (660, 140)

        separate_rgb = group.nodes.new("ShaderNodeSeparateColor")
        separate_rgb.location = (-140, 300)
        if hasattr(separate_rgb, "mode"):
            separate_rgb.mode = "RGB"
        group.links.new(group_input.outputs["Image"], separate_rgb.inputs[0])

        separate_hsv = group.nodes.new("ShaderNodeSeparateColor")
        separate_hsv.location = (460, 60)
        if hasattr(separate_hsv, "mode"):
            separate_hsv.mode = "HSV"
        group.links.new(group_input.outputs["Image"], separate_hsv.inputs[0])

        red_mix = group.nodes.new("ShaderNodeMix")
        red_mix.label = "Red Mix"
        red_mix.location = (60, 340)
        red_mix.data_type = "RGBA"
        red_mix.blend_type = "MIX"
        red_mix.clamp_factor = True
        red_mix.inputs[6].default_value = (0.0, 0.0, 0.0, 1.0)
        group.links.new(_output_socket(separate_rgb, "Red", "R"), red_mix.inputs[0])
        group.links.new(group_input.outputs["Decal Red Tint"], red_mix.inputs[7])

        green_mix = group.nodes.new("ShaderNodeMix")
        green_mix.label = "Green Mix"
        green_mix.location = (260, 320)
        green_mix.data_type = "RGBA"
        green_mix.blend_type = "MIX"
        green_mix.clamp_factor = True
        group.links.new(_output_socket(separate_rgb, "Green", "G"), green_mix.inputs[0])
        group.links.new(red_mix.outputs[2], green_mix.inputs[6])
        group.links.new(group_input.outputs["Decal Green Tint"], green_mix.inputs[7])

        blue_mix = group.nodes.new("ShaderNodeMix")
        blue_mix.label = "Blue Mix"
        blue_mix.location = (460, 300)
        blue_mix.data_type = "RGBA"
        blue_mix.blend_type = "MIX"
        blue_mix.clamp_factor = True
        group.links.new(_output_socket(separate_rgb, "Blue", "B"), blue_mix.inputs[0])
        group.links.new(green_mix.outputs[2], blue_mix.inputs[6])
        group.links.new(group_input.outputs["Decal Blue Tint"], blue_mix.inputs[7])

        group.links.new(blue_mix.outputs[2], group_output.inputs["Color"])
        group.links.new(
            _output_socket(separate_hsv, "Value", "V", "Blue"), group_output.inputs["Alpha"]
        )

        return group

    def _ensure_palette_group(self, palette: PaletteRecord) -> bpy.types.ShaderNodeTree:
        group_name = _palette_group_name(self.package.package_name, self._palette_scope())
        group_signature = _palette_group_signature(palette)
        group = bpy.data.node_groups.get(group_name)
        if group is None:
            group = bpy.data.node_groups.new(group_name, "ShaderNodeTree")

        existing_outputs = {
            item.name
            for item in group.interface.items_tree
            if getattr(item, "item_type", None) == "SOCKET"
            and getattr(item, "in_out", None) == "OUTPUT"
        }
        existing_inputs = {
            item.name
            for item in group.interface.items_tree
            if getattr(item, "item_type", None) == "SOCKET"
            and getattr(item, "in_out", None) == "INPUT"
        }

        expected_inputs: set[str] = set()
        expected_outputs = {
            "Decal Color",
            "Decal Alpha",
            "Primary",
            "Primary SpecColor",
            "Primary Glossiness",
            "Secondary",
            "Secondary SpecColor",
            "Secondary Glossiness",
            "Tertiary",
            "Tertiary SpecColor",
            "Tertiary Glossiness",
            "Glass Color",
            "Glass SpecColor",
            "Glass Glossiness",
        }
        _stale_outputs = {
            "Palette Decal Color",
            "Palette Decal Alpha",
            "Iridescence Facing Color",
            "Iridescence Grazing Color",
            "Iridescence Strength",
        }
        if (
            group.get("starbreaker_palette_signature") == group_signature
            and expected_inputs.issubset(existing_inputs)
            and expected_outputs.issubset(existing_outputs)
            and not _stale_outputs.intersection(existing_outputs)
        ):
            return group

        channel_specs = (
            ("Primary", "primary", 240),
            ("Primary SpecColor", "primary", 120),
            ("Primary Glossiness", "primary", 0),
            ("Secondary", "secondary", -140),
            ("Secondary SpecColor", "secondary", -260),
            ("Secondary Glossiness", "secondary", -380),
            ("Tertiary", "tertiary", -520),
            ("Tertiary SpecColor", "tertiary", -640),
            ("Tertiary Glossiness", "tertiary", -760),
            ("Glass Color", "glass", -900),
            ("Glass SpecColor", "glass", -1020),
            ("Glass Glossiness", "glass", -1140),
        )
        for socket_name in (
            "Decal Color",
            "Decal Alpha",
        ):
            if socket_name not in existing_outputs:
                group.interface.new_socket(
                    name=socket_name,
                    in_out="OUTPUT",
                    socket_type="NodeSocketFloat"
                    if socket_name.endswith("Alpha")
                    else "NodeSocketColor",
                )
        for socket_name, _channel_name, _y in channel_specs:
            if socket_name not in existing_outputs:
                socket_type = (
                    "NodeSocketFloat" if "Glossiness" in socket_name else "NodeSocketColor"
                )
                group.interface.new_socket(
                    name=socket_name, in_out="OUTPUT", socket_type=socket_type
                )

        for item in list(group.interface.items_tree):
            if (
                getattr(item, "item_type", None) == "SOCKET"
                and getattr(item, "in_out", None) == "OUTPUT"
                and item.name in _stale_outputs
            ):
                group.interface.remove(item)

        group.nodes.clear()

        group_input = group.nodes.new("NodeGroupInput")
        group_input.location = (-900, -120)
        output = group.nodes.new("NodeGroupOutput")
        output.location = (520, -120)

        primary_color = (*_palette_decal_or_fallback(palette, "red"), 1.0)
        secondary_color = (*_palette_decal_or_fallback(palette, "green"), 1.0)
        tertiary_color = (*_palette_decal_or_fallback(palette, "blue"), 1.0)

        palette_decal_node = self._image_node(
            group.nodes, palette_decal_texture(palette), x=-900, y=-520, is_color=True
        )
        if palette_decal_node is not None:
            adaptor_tree = self._ensure_tint_decal_adaptor_group()
            decal_converter = group.nodes.new("ShaderNodeGroup")
            decal_converter.name = "DecalConverter"
            decal_converter.node_tree = adaptor_tree
            decal_converter.location = (-420, -420)
            group.links.new(palette_decal_node.outputs[0], decal_converter.inputs["Image"])
            decal_converter.inputs["Decal Red Tint"].default_value = primary_color
            decal_converter.inputs["Decal Green Tint"].default_value = secondary_color
            decal_converter.inputs["Decal Blue Tint"].default_value = tertiary_color
            group.links.new(decal_converter.outputs["Color"], output.inputs["Decal Color"])
            group.links.new(decal_converter.outputs["Alpha"], output.inputs["Decal Alpha"])

        for socket_name, channel_name, y in channel_specs:
            if socket_name.endswith("SpecColor"):
                rgb = group.nodes.new("ShaderNodeRGB")
                rgb.location = (120, y)
                rgb.label = socket_name
                spec = palette_finish_specular(palette, channel_name) or (0.0, 0.0, 0.0)
                rgb.outputs[0].default_value = (*spec, 1.0)
                group.links.new(rgb.outputs[0], output.inputs[socket_name])
            elif socket_name.endswith("Glossiness"):
                value = group.nodes.new("ShaderNodeValue")
                value.location = (120, y)
                value.label = socket_name
                value.outputs[0].default_value = (
                    palette_finish_glossiness(palette, channel_name) or 0.0
                )
                group.links.new(value.outputs[0], output.inputs[socket_name])
            else:
                rgb = group.nodes.new("ShaderNodeRGB")
                rgb.location = (120, y)
                rgb.label = socket_name
                rgb.outputs[0].default_value = (*palette_color(palette, channel_name), 1.0)
                group.links.new(rgb.outputs[0], output.inputs[socket_name])

        group["starbreaker_palette_signature"] = group_signature
        return group

    def _palette_group_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        palette: PaletteRecord,
        *,
        x: int,
        y: int,
    ) -> bpy.types.Node:
        expected_name = f"STARBREAKER_PALETTE_{_safe_identifier(self._palette_scope()).upper()}"
        existing = next(
            (
                node
                for node in nodes
                if node.bl_idname == "ShaderNodeGroup"
                and getattr(node, "name", "") == expected_name
                and getattr(getattr(node, "node_tree", None), "name", "")
                == _palette_group_name(self.package.package_name, self._palette_scope())
            ),
            None,
        )
        group_node = existing or nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_palette_group(palette)
        _refresh_group_node_sockets(group_node)
        group_node.location = (x, y)
        group_node.label = "StarBreaker Palette"
        group_node.name = expected_name
        return group_node

    def _palette_color_socket(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> Any:
        group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
        socket_name = {
            "primary": "Primary",
            "secondary": "Secondary",
            "tertiary": "Tertiary",
            "glass": "Glass Color",
        }.get(channel_name, "Primary")
        return _output_socket(group_node, socket_name)

    def _palette_specular_socket(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> Any:
        group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
        socket_name = {
            "primary": "Primary SpecColor",
            "secondary": "Secondary SpecColor",
            "tertiary": "Tertiary SpecColor",
        }.get(channel_name)
        return _output_socket(group_node, socket_name) if socket_name is not None else None

    def _palette_glossiness_socket(
        self,
        nodes: bpy.types.Nodes,
        palette: PaletteRecord,
        channel_name: str,
        *,
        x: int,
        y: int,
    ) -> Any:
        group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
        socket_name = {
            "primary": "Primary Glossiness",
            "secondary": "Secondary Glossiness",
            "tertiary": "Tertiary Glossiness",
        }.get(channel_name)
        return _output_socket(group_node, socket_name) if socket_name is not None else None

    def _palette_decal_sockets(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        palette: PaletteRecord | None,
        channel_name: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if palette is None:
            return LayerSurfaceSockets()
        group_node = self._palette_group_node(nodes, links, palette, x=x, y=y)
        return LayerSurfaceSockets(
            color=_output_socket(group_node, "Decal Color"),
            alpha=_output_socket(group_node, "Decal Alpha"),
        )
