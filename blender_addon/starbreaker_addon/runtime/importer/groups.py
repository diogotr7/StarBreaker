"""Group-builder mixin for :class:`PackageImporter`.

Extracted in Phase 7.5 from ``runtime/_legacy.py``. Contains the 16
``_ensure_runtime_*_group`` methods plus the two helpers that manage the
lifecycle of shared runtime shader groups (``_begin_runtime_shared_group``,
``_invalidate_runtime_group_if_unexpected``).

Each ``_ensure_*`` method creates (or reuses) a ``bpy.types.ShaderNodeTree``
for a specific part of the material authoring contract. They operate
purely on ``bpy.data`` node groups; they do not touch per-material node
trees. The mixin is stateless beyond what :class:`PackageImporter`
initialises (``runtime_shared_groups_ready`` etc.).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import bpy

from ..constants import POM_DETAIL_DEFAULT, SCENE_POM_DETAIL_PROP, pom_detail_settings

from ..node_utils import (
    _input_socket,
    _output_socket,
    _refresh_group_node_sockets,
    _set_group_input_default,
)

_POM_LIBRARY_PATH = Path(__file__).resolve().parent.parent.parent / "resources" / "pom_library.blend"
_POM_LIBRARY_ROOT_GROUPS = (
    "POM_Vector",
    "POM_10x_Layer_Steps",
    "POM_parallax",
    "POM_disp",
    "HeightMap",
    "Clamp",
    "tangent_space",
)
_POM_DETAIL_GROUP_NAME = "StarBreaker Runtime POM Detail"
_POM_DETAIL_GROUP_SIGNATURE = "pom_detail_v3"
_POM_DETAIL_CONTROL_NODE_NAME = "StarBreaker POM Detail"
_POM_DETAIL_SCALE_NODE_NAME = "StarBreaker POM Detail Scale"
_POM_DETAIL_LAYERS_NODE_NAME = "StarBreaker Detail Layers"
_POM_DETAIL_SCALE_MULTIPLIER_NODE_NAME = "StarBreaker Detail Scale Multiplier"


def _ensure_runtime_pom_detail_group() -> bpy.types.ShaderNodeTree:
    group_tree = bpy.data.node_groups.get(_POM_DETAIL_GROUP_NAME)
    if group_tree is None:
        group_tree = bpy.data.node_groups.new(_POM_DETAIL_GROUP_NAME, "ShaderNodeTree")
    if (
        group_tree.get("starbreaker_runtime_signature") == _POM_DETAIL_GROUP_SIGNATURE
        and group_tree.get("starbreaker_runtime_built_signature") == _POM_DETAIL_GROUP_SIGNATURE
        and group_tree.nodes.get(_POM_DETAIL_LAYERS_NODE_NAME) is not None
        and group_tree.nodes.get(_POM_DETAIL_SCALE_MULTIPLIER_NODE_NAME) is not None
    ):
        return group_tree

    group_tree.use_fake_user = False
    group_tree.nodes.clear()
    for item in list(group_tree.interface.items_tree):
        group_tree.interface.remove(item)
    group_tree.interface.new_socket(name="Layers", in_out="OUTPUT", socket_type="NodeSocketFloat")
    group_tree.interface.new_socket(name="Layers Vector", in_out="OUTPUT", socket_type="NodeSocketVector")
    group_tree.interface.new_socket(name="Scale Multiplier", in_out="OUTPUT", socket_type="NodeSocketFloat")

    nodes = group_tree.nodes
    links = group_tree.links
    group_output = nodes.new("NodeGroupOutput")
    group_output.location = (240, 0)

    detail_layers = nodes.new("ShaderNodeValue")
    detail_layers.name = _POM_DETAIL_LAYERS_NODE_NAME
    detail_layers.label = "Detail Layers"
    detail_layers.location = (-320, 80)

    scale_multiplier = nodes.new("ShaderNodeValue")
    scale_multiplier.name = _POM_DETAIL_SCALE_MULTIPLIER_NODE_NAME
    scale_multiplier.label = "Scale Multiplier"
    scale_multiplier.location = (-320, -60)

    layers_vector = nodes.new("ShaderNodeCombineXYZ")
    layers_vector.location = (-40, 120)

    links.new(detail_layers.outputs[0], layers_vector.inputs[0])
    links.new(detail_layers.outputs[0], layers_vector.inputs[1])
    links.new(detail_layers.outputs[0], layers_vector.inputs[2])
    links.new(detail_layers.outputs[0], group_output.inputs["Layers"])
    links.new(layers_vector.outputs[0], group_output.inputs["Layers Vector"])
    links.new(scale_multiplier.outputs[0], group_output.inputs["Scale Multiplier"])

    group_tree["starbreaker_runtime_signature"] = _POM_DETAIL_GROUP_SIGNATURE
    group_tree["starbreaker_runtime_built_signature"] = _POM_DETAIL_GROUP_SIGNATURE
    return group_tree


def _configure_runtime_pom_detail_group(mode: str) -> bpy.types.ShaderNodeTree:
    group_tree = _ensure_runtime_pom_detail_group()
    layers, scale_multiplier = pom_detail_settings(mode)
    detail_layers = group_tree.nodes.get(_POM_DETAIL_LAYERS_NODE_NAME)
    if detail_layers is not None:
        detail_layers.outputs[0].default_value = float(layers)
    multiplier_node = group_tree.nodes.get(_POM_DETAIL_SCALE_MULTIPLIER_NODE_NAME)
    if multiplier_node is not None:
        multiplier_node.outputs[0].default_value = float(scale_multiplier)
    group_tree["starbreaker_pom_detail_mode"] = str(mode or POM_DETAIL_DEFAULT).upper()
    return group_tree


def _ensure_pom_detail_control_on_tree(
    group_tree: bpy.types.ShaderNodeTree,
    mode: str,
) -> bool:
    """Install direct internal Value/Math nodes inside a top-level POM root.

    We do not rely on a shared helper node group because routing values
    through a nested group + implicit socket conversions caused the
    parallax to evaluate to zero (div-by-zero on Vector Math.002 input[1]
    was also observed). Instead each top-level ``StarBreaker POM [...]``
    root now owns two Value nodes (``Detail Layers`` and ``Scale
    Multiplier``) plus an internal Multiply math node for the scale.
    ``apply_pom_detail_mode`` only has to update the two Value defaults.
    """

    if (
        group_tree is None
        or not group_tree.name.startswith("StarBreaker POM [")
        or " / " in group_tree.name
    ):
        return False

    nodes = group_tree.nodes
    links = group_tree.links
    layers, scale_multiplier = pom_detail_settings(mode)

    changed = False

    # Remove legacy "Layers" interface input socket so external 40.0 default
    # cannot override our internal value through Group Input.x nodes.
    legacy_layers_item = next(
        (
            item
            for item in list(group_tree.interface.items_tree)
            if getattr(item, "in_out", None) == "INPUT" and getattr(item, "name", None) == "Layers"
        ),
        None,
    )
    if legacy_layers_item is not None:
        group_tree.interface.remove(legacy_layers_item)
        changed = True

    # Remove the old shared-helper control node if still present (deprecated).
    stale_control = nodes.get(_POM_DETAIL_CONTROL_NODE_NAME)
    if stale_control is not None and stale_control.bl_idname == "ShaderNodeGroup":
        nodes.remove(stale_control)
        changed = True

    # Internal Value node: number of POM layers (num_layers / denominator).
    layers_node = nodes.get(_POM_DETAIL_LAYERS_NODE_NAME)
    if layers_node is None or layers_node.bl_idname != "ShaderNodeValue":
        if layers_node is not None:
            nodes.remove(layers_node)
        layers_node = nodes.new("ShaderNodeValue")
        layers_node.name = _POM_DETAIL_LAYERS_NODE_NAME
        layers_node.label = "Detail Layers"
        layers_node.location = (-420, 220)
        changed = True
    if float(layers_node.outputs[0].default_value) != float(layers):
        layers_node.outputs[0].default_value = float(layers)
        changed = True

    # Internal Value node: scale multiplier (layers / 40) to keep perceived depth constant.
    mult_node = nodes.get(_POM_DETAIL_SCALE_MULTIPLIER_NODE_NAME)
    if mult_node is None or mult_node.bl_idname != "ShaderNodeValue":
        if mult_node is not None:
            nodes.remove(mult_node)
        mult_node = nodes.new("ShaderNodeValue")
        mult_node.name = _POM_DETAIL_SCALE_MULTIPLIER_NODE_NAME
        mult_node.label = "Scale Multiplier"
        mult_node.location = (-420, 80)
        changed = True
    if float(mult_node.outputs[0].default_value) != float(scale_multiplier):
        mult_node.outputs[0].default_value = float(scale_multiplier)
        changed = True

    # Internal Multiply math: Scale * multiplier -> feeds Math.003 input[0].
    scale_math = nodes.get(_POM_DETAIL_SCALE_NODE_NAME)
    if scale_math is None or scale_math.bl_idname != "ShaderNodeMath":
        if scale_math is not None:
            nodes.remove(scale_math)
        scale_math = nodes.new("ShaderNodeMath")
        scale_math.name = _POM_DETAIL_SCALE_NODE_NAME
        scale_math.label = "POM Detail Scale"
        scale_math.operation = "MULTIPLY"
        scale_math.location = (-200, 160)
        changed = True

    # Find a Group Input node that exposes "Scale".
    source_scale_output = None
    for node in nodes:
        if node.bl_idname == "NodeGroupInput":
            out = _output_socket(node, "Scale")
            if out is not None:
                source_scale_output = out
                break
    if source_scale_output is None:
        return changed

    def _ensure_direct_link(output_socket, input_socket) -> None:
        nonlocal changed
        if output_socket is None or input_socket is None:
            return
        for link in list(input_socket.links):
            if (
                link.from_socket.as_pointer() == output_socket.as_pointer()
                and link.to_socket.as_pointer() == input_socket.as_pointer()
            ):
                return
            links.remove(link)
            changed = True
        links.new(output_socket, input_socket)
        changed = True

    # Wire scale_math inputs: [0]=source Scale, [1]=multiplier Value.
    _ensure_direct_link(source_scale_output, scale_math.inputs[0])
    _ensure_direct_link(mult_node.outputs[0], scale_math.inputs[1])

    # Math.003 (DIVIDE): [0]=scale*multiplier, [1]=layers. (Unchanged from the
    # previous approach — it was already correct.)
    layer_divide = nodes.get("Math.003")
    if layer_divide is not None and len(layer_divide.inputs) > 1:
        _ensure_direct_link(scale_math.outputs[0], layer_divide.inputs[0])
        _ensure_direct_link(layers_node.outputs[0], layer_divide.inputs[1])

    # Vector Math.002 (DIVIDE): [0]=delta numerator (leave original link
    # intact!), [1]=layers (float -> implicit vector). The previous code
    # wrote layers to input[0], clobbering the numerator -> flat POM.
    vector_math = nodes.get("Vector Math.002")
    if vector_math is not None and len(vector_math.inputs) > 1:
        _ensure_direct_link(layers_node.outputs[0], vector_math.inputs[1])

    # Group.001 first POM_10x_Layer_Steps: num_layers=layers, Scale=raw Scale.
    pom_group = nodes.get("Group.001")
    if pom_group is not None:
        _ensure_direct_link(layers_node.outputs[0], _input_socket(pom_group, "num layers"))
        _ensure_direct_link(source_scale_output, _input_socket(pom_group, "Scale"))

    return changed


def apply_pom_detail_mode(mode: str) -> int:
    # Keep the shared detail group datablock around for back-compat / deletion
    # of its runtime signature, but do not wire it into any material.
    _configure_runtime_pom_detail_group(mode)
    updated = 0
    for group_tree in bpy.data.node_groups:
        if _ensure_pom_detail_control_on_tree(group_tree, mode):
            updated += 1
    return updated


class GroupsMixin:
    def _current_pom_detail_mode(self) -> str:
        scene = getattr(getattr(self, "context", None), "scene", None)
        return str(getattr(scene, SCENE_POM_DETAIL_PROP, POM_DETAIL_DEFAULT) or POM_DETAIL_DEFAULT)

    def _begin_runtime_shared_group(
        self,
        group_name: str,
        *,
        signature: str,
        inputs: list[tuple[str, str]],
        outputs: list[tuple[str, str]],
    ) -> tuple[bpy.types.ShaderNodeTree, bpy.types.Node, bpy.types.Node]:
        group_tree = bpy.data.node_groups.get(group_name)
        if group_tree is None:
            group_tree = bpy.data.node_groups.new(group_name, "ShaderNodeTree")
        existing_signature = group_tree.get("starbreaker_runtime_signature")
        built_signature = group_tree.get("starbreaker_runtime_built_signature")
        group_input = next((node for node in group_tree.nodes if node.bl_idname == "NodeGroupInput"), None)
        group_output = next((node for node in group_tree.nodes if node.bl_idname == "NodeGroupOutput"), None)
        if (
            existing_signature == signature
            and built_signature == signature
            and group_input is not None
            and group_output is not None
        ):
            return group_tree, group_input, group_output
        group_tree.use_fake_user = False
        group_tree.nodes.clear()
        for item in list(group_tree.interface.items_tree):
            group_tree.interface.remove(item)
        for socket_name, socket_type in inputs:
            sock = group_tree.interface.new_socket(name=socket_name, in_out="INPUT", socket_type=socket_type)
            if "Normal" in socket_name and hasattr(sock, "default_value"):
                if socket_type == "NodeSocketColor":
                    sock.default_value = (0xBC / 255, 0xBC / 255, 1.0, 1.0)
                elif socket_type == "NodeSocketVector":
                    sock.default_value = (0xBC / 255, 0xBC / 255, 1.0)
        for socket_name, socket_type in outputs:
            group_tree.interface.new_socket(name=socket_name, in_out="OUTPUT", socket_type=socket_type)

        group_input = group_tree.nodes.new("NodeGroupInput")
        group_input.location = (-980, 0)
        group_output = group_tree.nodes.new("NodeGroupOutput")
        group_output.location = (980, 0)
        group_tree["starbreaker_runtime_signature"] = signature
        group_tree["starbreaker_runtime_built_signature"] = ""
        return group_tree, group_input, group_output

    def _invalidate_runtime_group_if_unexpected(
        self,
        group_name: str,
        signature: str,
        expected_node_counts: dict[str, int],
    ) -> None:
        group_tree = bpy.data.node_groups.get(group_name)
        if group_tree is None or group_tree.get("starbreaker_runtime_signature") != signature:
            return
        for bl_idname, expected_count in expected_node_counts.items():
            actual_count = sum(1 for node in group_tree.nodes if node.bl_idname == bl_idname)
            if actual_count != expected_count:
                group_tree["starbreaker_runtime_built_signature"] = ""
                return

    def _ensure_runtime_layer_surface_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime LayerSurface",
            "layer_surface_v4",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeNormalMap": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime LayerSurface",
            signature="layer_surface_v4",
            inputs=[
                ("Base Color", "NodeSocketColor"),
                ("Base Alpha", "NodeSocketFloat"),
                ("Palette Color", "NodeSocketColor"),
                ("Tint Color", "NodeSocketColor"),
                ("Detail Color Mask", "NodeSocketFloat"),
                ("Detail Height Mask", "NodeSocketFloat"),
                ("Detail Gloss Mask", "NodeSocketFloat"),
                ("Detail Diffuse Strength", "NodeSocketFloat"),
                ("Detail Gloss Strength", "NodeSocketFloat"),
                ("Detail Bump Strength", "NodeSocketFloat"),
                ("Normal Color", "NodeSocketColor"),
                ("Roughness Source", "NodeSocketFloat"),
                ("Roughness Source Is Smoothness", "NodeSocketBool"),
                ("Palette Glossiness", "NodeSocketFloat"),
                ("Specular Value", "NodeSocketFloat"),
                ("Palette Specular", "NodeSocketFloat"),
                ("Metallic", "NodeSocketFloat"),
                ("Specular Color", "NodeSocketColor"),
            ],
            outputs=[
                ("Color", "NodeSocketColor"),
                ("Alpha", "NodeSocketFloat"),
                ("Roughness", "NodeSocketFloat"),
                ("Specular", "NodeSocketFloat"),
                ("Normal", "NodeSocketVector"),
                ("Metallic", "NodeSocketFloat"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "layer_surface_v4":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        tint = nodes.new("ShaderNodeMixRGB")
        tint.location = (-720, 280)
        tint.blend_type = "MULTIPLY"
        tint.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Base Color"), tint.inputs[1])
        links.new(_output_socket(group_input, "Tint Color"), tint.inputs[2])

        palette_mix = nodes.new("ShaderNodeMixRGB")
        palette_mix.location = (-520, 280)
        palette_mix.blend_type = "MULTIPLY"
        palette_mix.inputs[0].default_value = 1.0
        links.new(tint.outputs[0], palette_mix.inputs[1])
        links.new(_output_socket(group_input, "Palette Color"), palette_mix.inputs[2])

        detail_gray = nodes.new("ShaderNodeValToRGB")
        detail_gray.location = (-720, 40)
        links.new(_output_socket(group_input, "Detail Color Mask"), detail_gray.inputs[0])

        white = nodes.new("ShaderNodeRGB")
        white.location = (-720, -120)
        white.outputs[0].default_value = (1.0, 1.0, 1.0, 1.0)

        detail_mix = nodes.new("ShaderNodeMixRGB")
        detail_mix.location = (-520, 40)
        detail_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Detail Diffuse Strength"), detail_mix.inputs[0])
        links.new(white.outputs[0], detail_mix.inputs[1])
        links.new(detail_gray.outputs[0], detail_mix.inputs[2])

        final_color = nodes.new("ShaderNodeMixRGB")
        final_color.location = (-320, 220)
        final_color.blend_type = "MULTIPLY"
        final_color.inputs[0].default_value = 1.0
        links.new(palette_mix.outputs[0], final_color.inputs[1])
        links.new(detail_mix.outputs[0], final_color.inputs[2])

        roughness_invert = nodes.new("ShaderNodeMath")
        roughness_invert.location = (-720, -300)
        roughness_invert.operation = "SUBTRACT"
        roughness_invert.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Roughness Source"), roughness_invert.inputs[1])

        roughness_source = nodes.new("ShaderNodeMix")
        roughness_source.location = (-520, -300)
        if hasattr(roughness_source, "data_type"):
            roughness_source.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Roughness Source Is Smoothness"), roughness_source.inputs[0])
        links.new(_output_socket(group_input, "Roughness Source"), roughness_source.inputs[2])
        links.new(roughness_invert.outputs[0], roughness_source.inputs[3])

        palette_gloss_factor = nodes.new("ShaderNodeMath")
        palette_gloss_factor.location = (-720, -180)
        palette_gloss_factor.operation = "SUBTRACT"
        palette_gloss_factor.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Palette Glossiness"), palette_gloss_factor.inputs[1])

        roughness_base = nodes.new("ShaderNodeMath")
        roughness_base.location = (-320, -240)
        roughness_base.operation = "MULTIPLY"
        links.new(roughness_source.outputs[0], roughness_base.inputs[0])
        links.new(palette_gloss_factor.outputs[0], roughness_base.inputs[1])

        detail_gloss = nodes.new("ShaderNodeMix")
        detail_gloss.location = (-120, -240)
        if hasattr(detail_gloss, "data_type"):
            detail_gloss.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Detail Gloss Strength"), detail_gloss.inputs[0])
        detail_gloss.inputs[2].default_value = 1.0
        links.new(_output_socket(group_input, "Detail Gloss Mask"), detail_gloss.inputs[3])

        roughness = nodes.new("ShaderNodeMath")
        roughness.location = (80, -240)
        roughness.operation = "MULTIPLY"
        links.new(roughness_base.outputs[0], roughness.inputs[0])
        links.new(detail_gloss.outputs[0], roughness.inputs[1])

        specular = nodes.new("ShaderNodeMath")
        specular.location = (-320, -420)
        specular.operation = "ADD"
        specular.use_clamp = True
        links.new(_output_socket(group_input, "Specular Value"), specular.inputs[0])
        links.new(_output_socket(group_input, "Palette Specular"), specular.inputs[1])

        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.location = (-520, -620)
        links.new(_output_socket(group_input, "Normal Color"), _input_socket(normal_map, "Color"))

        bump = nodes.new("ShaderNodeBump")
        bump.location = (-320, -620)
        links.new(_output_socket(group_input, "Detail Bump Strength"), bump.inputs[0])
        links.new(_output_socket(group_input, "Detail Height Mask"), bump.inputs[2])
        links.new(_output_socket(normal_map, "Normal"), bump.inputs[3])

        metallic_color_mix = nodes.new("ShaderNodeMixRGB")
        metallic_color_mix.location = (-120, 220)
        metallic_color_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Metallic"), metallic_color_mix.inputs[0])
        links.new(final_color.outputs[0], metallic_color_mix.inputs[1])
        links.new(_output_socket(group_input, "Specular Color"), metallic_color_mix.inputs[2])

        links.new(metallic_color_mix.outputs[0], group_output.inputs["Color"])
        links.new(_output_socket(group_input, "Base Alpha"), group_output.inputs["Alpha"])
        links.new(roughness.outputs[0], group_output.inputs["Roughness"])
        links.new(specular.outputs[0], group_output.inputs["Specular"])
        links.new(bump.outputs[0], group_output.inputs["Normal"])
        links.new(_output_socket(group_input, "Metallic"), group_output.inputs["Metallic"])
        group_tree["starbreaker_runtime_built_signature"] = "layer_surface_v4"
        return group_tree

    def _ensure_runtime_hard_surface_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime HardSurface",
            "hard_surface_v31",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime HardSurface",
            signature="hard_surface_v31",
            inputs=[
                ("Top Base Color", "NodeSocketColor"),
                ("Top Alpha", "NodeSocketFloat"),
                ("Primary Color", "NodeSocketColor"),
                ("Primary Alpha", "NodeSocketFloat"),
                ("Primary Roughness", "NodeSocketFloat"),
                ("Primary Specular", "NodeSocketFloat"),
                ("Primary Specular Tint", "NodeSocketColor"),
                ("Primary Metallic", "NodeSocketFloat"),
                ("Primary Normal", "NodeSocketVector"),
                ("Secondary Color", "NodeSocketColor"),
                ("Secondary Alpha", "NodeSocketFloat"),
                ("Secondary Roughness", "NodeSocketFloat"),
                ("Secondary Specular", "NodeSocketFloat"),
                ("Secondary Specular Tint", "NodeSocketColor"),
                ("Secondary Metallic", "NodeSocketFloat"),
                ("Secondary Normal", "NodeSocketVector"),
                ("Iridescence Facing Color", "NodeSocketColor"),
                ("Iridescence Grazing Color", "NodeSocketColor"),
                ("Iridescence Ramp Color", "NodeSocketColor"),
                ("Iridescence Ramp Weight", "NodeSocketFloat"),
                ("Iridescence Strength", "NodeSocketFloat"),
                ("Iridescence Factor", "NodeSocketFloat"),
                ("Wear Factor", "NodeSocketFloat"),
                ("Damage Factor", "NodeSocketFloat"),
                ("Stencil Color", "NodeSocketColor"),
                ("Stencil Color Factor", "NodeSocketFloat"),
                ("Stencil Factor", "NodeSocketFloat"),
                ("Stencil Roughness", "NodeSocketFloat"),
                ("Stencil Specular", "NodeSocketFloat"),
                ("Stencil Specular Tint", "NodeSocketColor"),
                ("Macro Normal Color", "NodeSocketColor"),
                ("Macro Normal Strength", "NodeSocketFloat"),
                ("Displacement Height", "NodeSocketFloat"),
                ("Displacement Strength", "NodeSocketFloat"),
                ("Emission Color", "NodeSocketColor"),
                ("Emission Strength", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "hard_surface_v31":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        damage_invert = nodes.new("ShaderNodeMath")
        damage_invert.location = (-980, 120)
        damage_invert.operation = "SUBTRACT"
        damage_invert.use_clamp = True
        damage_invert.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Damage Factor"), damage_invert.inputs[1])

        effective_wear_factor = nodes.new("ShaderNodeMath")
        effective_wear_factor.location = (-820, 120)
        effective_wear_factor.operation = "MULTIPLY"
        effective_wear_factor.use_clamp = True
        links.new(_output_socket(group_input, "Wear Factor"), effective_wear_factor.inputs[0])
        links.new(damage_invert.outputs[0], effective_wear_factor.inputs[1])

        color_mix = nodes.new("ShaderNodeMixRGB")
        color_mix.location = (-700, 260)
        color_mix.blend_type = "MIX"
        links.new(effective_wear_factor.outputs[0], color_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Color"), color_mix.inputs[1])
        links.new(_output_socket(group_input, "Secondary Color"), color_mix.inputs[2])

        final_color = nodes.new("ShaderNodeMixRGB")
        final_color.location = (-500, 260)
        final_color.blend_type = "MULTIPLY"
        final_color.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Top Base Color"), final_color.inputs[1])
        links.new(color_mix.outputs[0], final_color.inputs[2])

        angle_factor = self._hard_surface_angle_factor_socket(nodes, links, x=-720, y=520)

        iridescence_color = nodes.new("ShaderNodeMixRGB")
        iridescence_color.location = (-500, 520)
        iridescence_color.blend_type = "MIX"
        links.new(_output_socket(group_input, "Iridescence Facing Color"), iridescence_color.inputs[1])
        links.new(_output_socket(group_input, "Iridescence Grazing Color"), iridescence_color.inputs[2])
        links.new(angle_factor, iridescence_color.inputs[0])

        iridescence_source = nodes.new("ShaderNodeMixRGB")
        iridescence_source.location = (-280, 520)
        iridescence_source.blend_type = "SCREEN"
        links.new(_output_socket(group_input, "Iridescence Ramp Weight"), iridescence_source.inputs[0])
        links.new(iridescence_color.outputs[0], iridescence_source.inputs[1])
        links.new(_output_socket(group_input, "Iridescence Ramp Color"), iridescence_source.inputs[2])

        iridescence_strength_mix = nodes.new("ShaderNodeMath")
        iridescence_strength_mix.location = (-80, 360)
        iridescence_strength_mix.operation = "MULTIPLY"
        iridescence_strength_mix.use_clamp = True
        iridescence_strength_mix.inputs[1].default_value = 1.0
        links.new(_output_socket(group_input, "Iridescence Strength"), iridescence_strength_mix.inputs[0])

        iridescence_consumer_factor = nodes.new("ShaderNodeMath")
        iridescence_consumer_factor.location = (120, 340)
        iridescence_consumer_factor.operation = "MULTIPLY"
        iridescence_consumer_factor.use_clamp = True
        links.new(_output_socket(group_input, "Iridescence Factor"), iridescence_consumer_factor.inputs[0])
        links.new(iridescence_strength_mix.outputs[0], iridescence_consumer_factor.inputs[1])

        body_iridescence_factor = nodes.new("ShaderNodeMath")
        body_iridescence_factor.location = (120, 420)
        body_iridescence_factor.operation = "MULTIPLY"
        body_iridescence_factor.use_clamp = True
        links.new(iridescence_consumer_factor.outputs[0], body_iridescence_factor.inputs[0])
        body_iridescence_factor.inputs[1].default_value = 0.65

        body_iridescence_source = nodes.new("ShaderNodeMixRGB")
        body_iridescence_source.location = (-60, 620)
        body_iridescence_source.blend_type = "MIX"
        links.new(_output_socket(group_input, "Iridescence Ramp Weight"), body_iridescence_source.inputs[0])
        links.new(iridescence_color.outputs[0], body_iridescence_source.inputs[1])
        links.new(_output_socket(group_input, "Iridescence Ramp Color"), body_iridescence_source.inputs[2])

        body_iridescence_channels = nodes.new("ShaderNodeSeparateColor")
        body_iridescence_channels.location = (140, 660)
        if hasattr(body_iridescence_channels, "mode"):
            body_iridescence_channels.mode = "RGB"
        links.new(body_iridescence_source.outputs[0], body_iridescence_channels.inputs[0])

        body_iridescence_max_rg = nodes.new("ShaderNodeMath")
        body_iridescence_max_rg.location = (320, 720)
        body_iridescence_max_rg.operation = "MAXIMUM"
        links.new(body_iridescence_channels.outputs[0], body_iridescence_max_rg.inputs[0])
        links.new(body_iridescence_channels.outputs[1], body_iridescence_max_rg.inputs[1])

        body_iridescence_max_rgb = nodes.new("ShaderNodeMath")
        body_iridescence_max_rgb.location = (500, 720)
        body_iridescence_max_rgb.operation = "MAXIMUM"
        links.new(body_iridescence_max_rg.outputs[0], body_iridescence_max_rgb.inputs[0])
        links.new(body_iridescence_channels.outputs[2], body_iridescence_max_rgb.inputs[1])

        body_iridescence_safe_max = nodes.new("ShaderNodeMath")
        body_iridescence_safe_max.location = (680, 720)
        body_iridescence_safe_max.operation = "MAXIMUM"
        links.new(body_iridescence_max_rgb.outputs[0], body_iridescence_safe_max.inputs[0])
        body_iridescence_safe_max.inputs[1].default_value = 0.001

        body_iridescence_red = nodes.new("ShaderNodeMath")
        body_iridescence_red.location = (860, 780)
        body_iridescence_red.operation = "DIVIDE"
        links.new(body_iridescence_channels.outputs[0], body_iridescence_red.inputs[0])
        links.new(body_iridescence_safe_max.outputs[0], body_iridescence_red.inputs[1])

        body_iridescence_green = nodes.new("ShaderNodeMath")
        body_iridescence_green.location = (860, 660)
        body_iridescence_green.operation = "DIVIDE"
        links.new(body_iridescence_channels.outputs[1], body_iridescence_green.inputs[0])
        links.new(body_iridescence_safe_max.outputs[0], body_iridescence_green.inputs[1])

        body_iridescence_blue = nodes.new("ShaderNodeMath")
        body_iridescence_blue.location = (860, 540)
        body_iridescence_blue.operation = "DIVIDE"
        links.new(body_iridescence_channels.outputs[2], body_iridescence_blue.inputs[0])
        links.new(body_iridescence_safe_max.outputs[0], body_iridescence_blue.inputs[1])

        body_iridescence_tint = nodes.new("ShaderNodeCombineColor")
        body_iridescence_tint.location = (1040, 660)
        if hasattr(body_iridescence_tint, "mode"):
            body_iridescence_tint.mode = "RGB"
        links.new(body_iridescence_red.outputs[0], body_iridescence_tint.inputs[0])
        links.new(body_iridescence_green.outputs[0], body_iridescence_tint.inputs[1])
        links.new(body_iridescence_blue.outputs[0], body_iridescence_tint.inputs[2])

        body_iridescence_tinted_base = nodes.new("ShaderNodeMixRGB")
        body_iridescence_tinted_base.location = (1220, 560)
        body_iridescence_tinted_base.blend_type = "MULTIPLY"
        body_iridescence_tinted_base.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Top Base Color"), body_iridescence_tinted_base.inputs[1])
        links.new(body_iridescence_tint.outputs[0], body_iridescence_tinted_base.inputs[2])

        body_iridescence_mix = nodes.new("ShaderNodeMixRGB")
        body_iridescence_mix.location = (1400, 560)
        body_iridescence_mix.blend_type = "MIX"
        links.new(body_iridescence_factor.outputs[0], body_iridescence_mix.inputs[0])
        links.new(final_color.outputs[0], body_iridescence_mix.inputs[1])
        links.new(body_iridescence_tinted_base.outputs[0], body_iridescence_mix.inputs[2])

        stencil_mix = nodes.new("ShaderNodeMixRGB")
        stencil_mix.location = (1600, 460)
        stencil_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Stencil Color Factor"), stencil_mix.inputs[0])
        links.new(body_iridescence_mix.outputs[0], stencil_mix.inputs[1])
        links.new(_output_socket(group_input, "Stencil Color"), stencil_mix.inputs[2])

        alpha_mix = nodes.new("ShaderNodeMix")
        alpha_mix.location = (-700, 80)
        if hasattr(alpha_mix, "data_type"):
            alpha_mix.data_type = "FLOAT"
        links.new(effective_wear_factor.outputs[0], alpha_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Alpha"), alpha_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Alpha"), alpha_mix.inputs[3])

        alpha_mul = nodes.new("ShaderNodeMath")
        alpha_mul.location = (-500, 80)
        alpha_mul.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Top Alpha"), alpha_mul.inputs[0])
        links.new(alpha_mix.outputs[0], alpha_mul.inputs[1])

        roughness_mix = nodes.new("ShaderNodeMix")
        roughness_mix.location = (-700, -100)
        if hasattr(roughness_mix, "data_type"):
            roughness_mix.data_type = "FLOAT"
        links.new(effective_wear_factor.outputs[0], roughness_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Roughness"), roughness_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Roughness"), roughness_mix.inputs[3])

        stencil_roughness_mix = nodes.new("ShaderNodeMix")
        stencil_roughness_mix.location = (-480, -100)
        if hasattr(stencil_roughness_mix, "data_type"):
            stencil_roughness_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Stencil Factor"), stencil_roughness_mix.inputs[0])
        links.new(roughness_mix.outputs[0], stencil_roughness_mix.inputs[2])
        links.new(_output_socket(group_input, "Stencil Roughness"), stencil_roughness_mix.inputs[3])

        specular_mix = nodes.new("ShaderNodeMix")
        specular_mix.location = (-700, -280)
        if hasattr(specular_mix, "data_type"):
            specular_mix.data_type = "FLOAT"
        links.new(effective_wear_factor.outputs[0], specular_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Specular"), specular_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Specular"), specular_mix.inputs[3])

        stencil_specular_mix = nodes.new("ShaderNodeMix")
        stencil_specular_mix.location = (-480, -280)
        if hasattr(stencil_specular_mix, "data_type"):
            stencil_specular_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Stencil Factor"), stencil_specular_mix.inputs[0])
        links.new(specular_mix.outputs[0], stencil_specular_mix.inputs[2])
        links.new(_output_socket(group_input, "Stencil Specular"), stencil_specular_mix.inputs[3])

        specular_tint_mix = nodes.new("ShaderNodeMixRGB")
        specular_tint_mix.location = (-700, -420)
        specular_tint_mix.blend_type = "MIX"
        links.new(effective_wear_factor.outputs[0], specular_tint_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Specular Tint"), specular_tint_mix.inputs[1])
        links.new(_output_socket(group_input, "Secondary Specular Tint"), specular_tint_mix.inputs[2])

        iridescence_specular_tint_mix = nodes.new("ShaderNodeMixRGB")
        iridescence_specular_tint_mix.location = (-590, -520)
        iridescence_specular_tint_mix.blend_type = "MIX"
        links.new(iridescence_consumer_factor.outputs[0], iridescence_specular_tint_mix.inputs[0])
        links.new(specular_tint_mix.outputs[0], iridescence_specular_tint_mix.inputs[1])
        links.new(iridescence_source.outputs[0], iridescence_specular_tint_mix.inputs[2])

        stencil_specular_tint_mix = nodes.new("ShaderNodeMixRGB")
        stencil_specular_tint_mix.location = (-480, -420)
        stencil_specular_tint_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Stencil Factor"), stencil_specular_tint_mix.inputs[0])
        links.new(iridescence_specular_tint_mix.outputs[0], stencil_specular_tint_mix.inputs[1])
        links.new(_output_socket(group_input, "Stencil Specular Tint"), stencil_specular_tint_mix.inputs[2])

        normal_mix = nodes.new("ShaderNodeMix")
        normal_mix.location = (-700, -500)
        if hasattr(normal_mix, "data_type"):
            normal_mix.data_type = "VECTOR"
        links.new(effective_wear_factor.outputs[0], normal_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Normal"), normal_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Normal"), normal_mix.inputs[3])

        macro_normal = nodes.new("ShaderNodeNormalMap")
        macro_normal.location = (-500, -680)
        strength_input = _input_socket(macro_normal, "Strength")
        if strength_input is not None:
            links.new(_output_socket(group_input, "Macro Normal Strength"), strength_input)
        links.new(_output_socket(group_input, "Macro Normal Color"), _input_socket(macro_normal, "Color"))

        normal_add = nodes.new("ShaderNodeVectorMath")
        normal_add.location = (-300, -520)
        normal_add.operation = "ADD"
        links.new(normal_mix.outputs[0], normal_add.inputs[0])
        links.new(_output_socket(macro_normal, "Normal"), normal_add.inputs[1])

        normal_normalize = nodes.new("ShaderNodeVectorMath")
        normal_normalize.location = (-100, -520)
        normal_normalize.operation = "NORMALIZE"
        links.new(normal_add.outputs[0], normal_normalize.inputs[0])

        bump = nodes.new("ShaderNodeBump")
        bump.location = (100, -520)
        links.new(_output_socket(group_input, "Displacement Strength"), bump.inputs[0])
        links.new(_output_socket(group_input, "Displacement Height"), bump.inputs[2])
        links.new(normal_normalize.outputs[0], bump.inputs[3])

        principled = self._create_surface_bsdf(nodes)
        principled.location = (320, 40)
        links.new(stencil_mix.outputs[0], _input_socket(principled, "Base Color"))
        links.new(alpha_mul.outputs[0], _input_socket(principled, "Alpha"))
        links.new(stencil_roughness_mix.outputs[0], _input_socket(principled, "Roughness"))
        metallic_layer_mix = nodes.new("ShaderNodeMix")
        metallic_layer_mix.location = (-700, -600)
        if hasattr(metallic_layer_mix, "data_type"):
            metallic_layer_mix.data_type = "FLOAT"
        links.new(effective_wear_factor.outputs[0], metallic_layer_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Metallic"), metallic_layer_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Metallic"), metallic_layer_mix.inputs[3])
        # Phase 8: Iridescence Factor is already a clamped 0/1 float driven
        # from the material builder; the previous MapRange (0..1 -> 0..1,
        # clamp=True) was an identity no-op. Feed it directly into the
        # metallic MAX.
        metallic_max = nodes.new("ShaderNodeMath")
        metallic_max.location = (260, -180)
        metallic_max.operation = "MAXIMUM"
        links.new(metallic_layer_mix.outputs[0], metallic_max.inputs[0])
        links.new(_output_socket(group_input, "Iridescence Factor"), metallic_max.inputs[1])
        metallic_input = _input_socket(principled, "Metallic")
        if metallic_input is not None:
            links.new(metallic_max.outputs[0], metallic_input)
        specular_input = _input_socket(principled, "Specular IOR Level", "Specular")
        if specular_input is not None:
            links.new(stencil_specular_mix.outputs[0], specular_input)
        specular_tint_input = _input_socket(principled, "Specular Tint")
        if specular_tint_input is not None:
            links.new(stencil_specular_tint_mix.outputs[0], specular_tint_input)
        coat_weight_input = _input_socket(principled, "Coat Weight")
        if coat_weight_input is not None:
            links.new(iridescence_consumer_factor.outputs[0], coat_weight_input)
        coat_roughness_input = _input_socket(principled, "Coat Roughness")
        if coat_roughness_input is not None:
            coat_roughness_input.default_value = 0.08
        coat_tint_input = _input_socket(principled, "Coat Tint")
        if coat_tint_input is not None:
            links.new(iridescence_source.outputs[0], coat_tint_input)
        normal_input = _input_socket(principled, "Normal")
        if normal_input is not None:
            links.new(bump.outputs[0], normal_input)
        emission_color = _input_socket(principled, "Emission Color", "Emission")
        emission_strength = _input_socket(principled, "Emission Strength")
        if emission_color is not None:
            links.new(_output_socket(group_input, "Emission Color"), emission_color)
        if emission_strength is not None:
            links.new(_output_socket(group_input, "Emission Strength"), emission_strength)

        links.new(principled.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "hard_surface_v31"
        return group_tree

    def _ensure_runtime_wear_input_group(self) -> bpy.types.ShaderNodeTree:
        """Helper group for wear/damage vertex-color routing.

        Inputs:
            Wear Mask           float  (from optional mask texture; used when Use Vertex Colors = 0)
            Use Vertex Colors   float  (0 = mask-based, 1 = vertex-color COLOR_0.R invert)
            Wear Base           float  (multiplier, from WearBlendBase/DamagePerObjectWear)
            Wear Strength       float  (global wear multiplier)
            Use Damage          float  (0 = force damage 0, 1 = pass COLOR_0.B through)

        Outputs:
            Wear Factor    float
            Damage Factor  float
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Wear Input",
            "wear_input_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeVertexColor": 1,
                "ShaderNodeSeparateColor": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Wear Input",
            signature="wear_input_v1",
            inputs=[
                ("Wear Mask", "NodeSocketFloat"),
                ("Use Vertex Colors", "NodeSocketFloat"),
                ("Wear Base", "NodeSocketFloat"),
                ("Wear Strength", "NodeSocketFloat"),
                ("Use Damage", "NodeSocketFloat"),
            ],
            outputs=[
                ("Wear Factor", "NodeSocketFloat"),
                ("Damage Factor", "NodeSocketFloat"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "wear_input_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        # Vertex color source: COLOR_0, separate into R/G/B.
        vc_node = nodes.new("ShaderNodeVertexColor")
        vc_node.location = (-720, 120)
        vc_node.layer_name = "Color"
        separate = nodes.new("ShaderNodeSeparateColor")
        separate.location = (-520, 120)
        links.new(vc_node.outputs[0], separate.inputs[0])

        # Invert R (Aurora COLOR_0 red: 1.0 = pristine paint).
        invert = nodes.new("ShaderNodeMath")
        invert.location = (-320, 200)
        invert.operation = "SUBTRACT"
        invert.use_clamp = True
        invert.inputs[0].default_value = 1.0
        links.new(_output_socket(separate, "Red"), invert.inputs[1])

        # Pick vertex-based or mask-based source.
        select_source = nodes.new("ShaderNodeMix")
        select_source.location = (-120, 120)
        if hasattr(select_source, "data_type"):
            select_source.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Use Vertex Colors"), select_source.inputs[0])
        links.new(_output_socket(group_input, "Wear Mask"), select_source.inputs[2])
        links.new(invert.outputs[0], select_source.inputs[3])

        # Multiply by Wear Base.
        mul_base = nodes.new("ShaderNodeMath")
        mul_base.location = (100, 120)
        mul_base.operation = "MULTIPLY"
        mul_base.use_clamp = True
        links.new(select_source.outputs[0], mul_base.inputs[0])
        links.new(_output_socket(group_input, "Wear Base"), mul_base.inputs[1])

        # Multiply by Wear Strength.
        mul_strength = nodes.new("ShaderNodeMath")
        mul_strength.location = (300, 120)
        mul_strength.operation = "MULTIPLY"
        mul_strength.use_clamp = True
        links.new(mul_base.outputs[0], mul_strength.inputs[0])
        links.new(_output_socket(group_input, "Wear Strength"), mul_strength.inputs[1])

        # Damage path: COLOR_0.B gated by Use Damage.
        damage_gate = nodes.new("ShaderNodeMath")
        damage_gate.location = (-320, -200)
        damage_gate.operation = "MULTIPLY"
        damage_gate.use_clamp = True
        links.new(_output_socket(separate, "Blue"), damage_gate.inputs[0])
        links.new(_output_socket(group_input, "Use Damage"), damage_gate.inputs[1])

        links.new(mul_strength.outputs[0], group_output.inputs["Wear Factor"])
        links.new(damage_gate.outputs[0], group_output.inputs["Damage Factor"])
        group_tree["starbreaker_runtime_built_signature"] = "wear_input_v1"
        return group_tree

    def _ensure_runtime_iridescence_input_group(self) -> bpy.types.ShaderNodeTree:
        """Helper group for HardSurface angle-based iridescence sampling.

        Inputs:
            Thickness U   float  (scales angle factor along X; clamped 0..1)
            Thickness V   float  (static Y coordinate; clamped 0..1)

        Outputs:
            Angle Factor  float   (0..1, from LayerWeight Facing via MapRange)
            Ramp UV       vector  (feed into TexSlot10 ramp image node Vector input)
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Iridescence Input",
            "iridescence_input_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeLayerWeight": 1,
                "ShaderNodeMapRange": 1,
                "ShaderNodeCombineXYZ": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Iridescence Input",
            signature="iridescence_input_v1",
            inputs=[
                ("Thickness U", "NodeSocketFloat"),
                ("Thickness V", "NodeSocketFloat"),
            ],
            outputs=[
                ("Angle Factor", "NodeSocketFloat"),
                ("Ramp UV", "NodeSocketVector"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "iridescence_input_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        layer_weight = nodes.new("ShaderNodeLayerWeight")
        layer_weight.location = (-520, 120)
        blend_input = _input_socket(layer_weight, "Blend")
        if blend_input is not None:
            blend_input.default_value = 0.3

        angle_factor = nodes.new("ShaderNodeMapRange")
        angle_factor.location = (-320, 120)
        angle_factor.clamp = True
        angle_factor.inputs[1].default_value = 0.0
        angle_factor.inputs[2].default_value = 0.2
        angle_factor.inputs[3].default_value = 0.0
        angle_factor.inputs[4].default_value = 1.0
        links.new(_output_socket(layer_weight, "Facing"), angle_factor.inputs[0])

        scale_x = nodes.new("ShaderNodeMath")
        scale_x.location = (-120, -40)
        scale_x.operation = "MULTIPLY"
        scale_x.use_clamp = True
        links.new(angle_factor.outputs[0], scale_x.inputs[0])
        links.new(_output_socket(group_input, "Thickness U"), scale_x.inputs[1])

        combine = nodes.new("ShaderNodeCombineXYZ")
        combine.location = (100, 0)
        links.new(scale_x.outputs[0], combine.inputs[0])
        links.new(_output_socket(group_input, "Thickness V"), combine.inputs[1])

        links.new(angle_factor.outputs[0], group_output.inputs["Angle Factor"])
        links.new(combine.outputs[0], group_output.inputs["Ramp UV"])
        group_tree["starbreaker_runtime_built_signature"] = "iridescence_input_v1"
        return group_tree

    def _ensure_runtime_nodraw_group(self) -> bpy.types.ShaderNodeTree:
        """Thin wrapper around BsdfTransparent so nodraw materials keep their top level clean."""
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime NoDraw",
            "nodraw_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfTransparent": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime NoDraw",
            signature="nodraw_v1",
            inputs=[],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "nodraw_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (0, 0)
        links.new(transparent.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "nodraw_v1"
        return group_tree

    def _ensure_runtime_glass_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap BsdfGlass + NormalMap inside a reusable shader group.

        Inputs:
            Base Color      color
            Roughness       float
            IOR             float   (default 1.05)
            Normal Color    color   (raw image color; internal NormalMap applies)
            Normal Strength float   (default 0.25)
            Use Normal      float   (0 to ignore normal map; 1 to apply)
        Outputs:
            Shader          shader

        Implementation notes. Aurora-class ships stack 10+ interior cabin
        glass panes behind the exterior canopy. With plain BsdfGlass on
        every pane the canopy refraction ray transmits through each
        interior pane in turn, multiplying the Base Color tint on every
        hop so the cockpit reads near-black. To avoid Beer-Lambert
        compounding without hiding interior geometry, the group mixes
        BsdfGlass with a pure Transparent BSDF via ``Is Camera Ray``:
        primary camera rays see proper glass (refraction, tint,
        roughness); transmission / shadow / diffuse / glossy rays see
        the pane as a clear window so light and refracted views pass
        through unattenuated.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Glass",
            "glass_v2",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfGlass": 1,
                "ShaderNodeBsdfTransparent": 1,
                "ShaderNodeLightPath": 1,
                "ShaderNodeMixShader": 1,
                "ShaderNodeNormalMap": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Glass",
            signature="glass_v2",
            inputs=[
                ("Base Color", "NodeSocketColor"),
                ("Roughness", "NodeSocketFloat"),
                ("IOR", "NodeSocketFloat"),
                ("Normal Color", "NodeSocketColor"),
                ("Normal Strength", "NodeSocketFloat"),
                ("Use Normal", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "glass_v2":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.location = (-320, -200)
        links.new(_output_socket(group_input, "Normal Color"), _input_socket(normal_map, "Color"))
        links.new(_output_socket(group_input, "Normal Strength"), _input_socket(normal_map, "Strength"))

        # Mix between "no normal" (geometry) and mapped normal via Use Normal toggle.
        geometry = nodes.new("ShaderNodeNewGeometry")
        geometry.location = (-320, -420)
        normal_mix = nodes.new("ShaderNodeMix")
        normal_mix.location = (-120, -300)
        if hasattr(normal_mix, "data_type"):
            normal_mix.data_type = "VECTOR"
        links.new(_output_socket(group_input, "Use Normal"), normal_mix.inputs[0])
        links.new(_output_socket(geometry, "Normal"), normal_mix.inputs[4])
        links.new(_output_socket(normal_map, "Normal"), normal_mix.inputs[5])

        glass = nodes.new("ShaderNodeBsdfGlass")
        glass.location = (120, 0)
        glass.label = "StarBreaker Glass"
        links.new(_output_socket(group_input, "Base Color"), _input_socket(glass, "Color"))
        links.new(_output_socket(group_input, "Roughness"), _input_socket(glass, "Roughness"))
        links.new(_output_socket(group_input, "IOR"), _input_socket(glass, "IOR"))
        links.new(normal_mix.outputs[1], _input_socket(glass, "Normal"))

        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (120, -180)
        transparent.label = "StarBreaker Glass Non-Camera Passthrough"
        # Pure white so refracted / transmitted views through stacked
        # interior glass panes aren't darkened by Beer-Lambert tint.
        transparent.inputs["Color"].default_value = (1.0, 1.0, 1.0, 1.0)

        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (120, 220)

        mix_shader = nodes.new("ShaderNodeMixShader")
        mix_shader.location = (340, 60)
        # fac=0 -> Transparent (non-camera rays), fac=1 -> Glass (camera rays)
        links.new(_output_socket(light_path, "Is Camera Ray"), mix_shader.inputs[0])
        links.new(transparent.outputs[0], mix_shader.inputs[1])
        links.new(glass.outputs[0], mix_shader.inputs[2])

        links.new(mix_shader.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "glass_v2"
        return group_tree

    def _ensure_runtime_screen_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap Emission + Transparent + MixShader (optional checker fallback) into a shader group."""
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Screen",
            "screen_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeEmission": 1,
                "ShaderNodeBsdfTransparent": 1,
                "ShaderNodeMixShader": 1,
                "ShaderNodeTexChecker": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Screen",
            signature="screen_v1",
            inputs=[
                ("Base Color", "NodeSocketColor"),
                ("Emission Strength", "NodeSocketFloat"),
                ("Mix Factor", "NodeSocketFloat"),
                ("Use Checker", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "screen_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        # Procedural checker fallback, selected via Use Checker.
        checker = nodes.new("ShaderNodeTexChecker")
        checker.location = (-520, 220)
        checker_mix = nodes.new("ShaderNodeMixRGB")
        checker_mix.location = (-320, 120)
        checker_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Use Checker"), checker_mix.inputs[0])
        links.new(_output_socket(group_input, "Base Color"), checker_mix.inputs[1])
        links.new(_output_socket(checker, "Color"), checker_mix.inputs[2])

        emission = nodes.new("ShaderNodeEmission")
        emission.location = (-100, 120)
        links.new(checker_mix.outputs[0], _input_socket(emission, "Color"))
        links.new(_output_socket(group_input, "Emission Strength"), _input_socket(emission, "Strength"))

        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (-100, -80)

        mix = nodes.new("ShaderNodeMixShader")
        mix.location = (120, 40)
        links.new(_output_socket(group_input, "Mix Factor"), mix.inputs[0])
        links.new(transparent.outputs[0], mix.inputs[1])
        links.new(emission.outputs[0], mix.inputs[2])

        links.new(mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "screen_v1"
        return group_tree

    def _ensure_runtime_effect_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap Emission + Transparent + MixShader into an Effect shader group."""
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Effect",
            "effect_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeEmission": 1,
                "ShaderNodeBsdfTransparent": 1,
                "ShaderNodeMixShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Effect",
            signature="effect_v1",
            inputs=[
                ("Base Color", "NodeSocketColor"),
                ("Emission Strength", "NodeSocketFloat"),
                ("Mix Factor", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "effect_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        emission = nodes.new("ShaderNodeEmission")
        emission.location = (-100, 120)
        links.new(_output_socket(group_input, "Base Color"), _input_socket(emission, "Color"))
        links.new(_output_socket(group_input, "Emission Strength"), _input_socket(emission, "Strength"))

        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (-100, -80)

        mix = nodes.new("ShaderNodeMixShader")
        mix.location = (120, 40)
        links.new(_output_socket(group_input, "Mix Factor"), mix.inputs[0])
        links.new(transparent.outputs[0], mix.inputs[1])
        links.new(emission.outputs[0], mix.inputs[2])

        links.new(mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "effect_v1"
        return group_tree

    def _ensure_runtime_layered_inputs_group(self) -> bpy.types.ShaderNodeTree:
        """Helper group for layered-wear base color / roughness composition.

        Inputs:
            Base Image        color   (primary diffuse image; default white)
            Base Palette      color   (palette channel for primary; default white = pass-through)
            Layer Image       color   (wear-layer diffuse image; default white)
            Layer Tint        color   (per-layer tint color; default white = pass-through)
            Layer Palette     color   (palette channel for wear layer; default white)
            Wear Factor       float   (0 = pure base, 1 = pure layer; default 0)
            Base Roughness    float   (default 0.45)
            Layer Roughness   float   (default 0.45)

        Outputs:
            Color         color
            Roughness     float

        Multiplicative composition: the ``* default white`` inputs are identity
        when unused, so callers only need to wire sockets that are actually
        present per-material.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime LayeredInputs",
            "layered_inputs_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeMixRGB": 4,
                "ShaderNodeMix": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime LayeredInputs",
            signature="layered_inputs_v1",
            inputs=[
                ("Base Image", "NodeSocketColor"),
                ("Base Palette", "NodeSocketColor"),
                ("Layer Image", "NodeSocketColor"),
                ("Layer Tint", "NodeSocketColor"),
                ("Layer Palette", "NodeSocketColor"),
                ("Wear Factor", "NodeSocketFloat"),
                ("Base Roughness", "NodeSocketFloat"),
                ("Layer Roughness", "NodeSocketFloat"),
            ],
            outputs=[
                ("Color", "NodeSocketColor"),
                ("Roughness", "NodeSocketFloat"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "layered_inputs_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        # Default identity values so disconnected sockets pass through.
        _set_group_input_default(group_input, "Base Image", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Base Palette", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Layer Image", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Layer Tint", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Layer Palette", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Wear Factor", 0.0)
        _set_group_input_default(group_input, "Base Roughness", 0.45)
        _set_group_input_default(group_input, "Layer Roughness", 0.45)

        # base_final = Base Image * Base Palette
        base_mult = nodes.new("ShaderNodeMixRGB")
        base_mult.location = (-420, 260)
        base_mult.blend_type = "MULTIPLY"
        base_mult.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Base Image"), base_mult.inputs[1])
        links.new(_output_socket(group_input, "Base Palette"), base_mult.inputs[2])

        # layer_tinted = Layer Image * Layer Tint
        layer_tint_mult = nodes.new("ShaderNodeMixRGB")
        layer_tint_mult.location = (-420, 40)
        layer_tint_mult.blend_type = "MULTIPLY"
        layer_tint_mult.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Layer Image"), layer_tint_mult.inputs[1])
        links.new(_output_socket(group_input, "Layer Tint"), layer_tint_mult.inputs[2])

        # layer_final = layer_tinted * Layer Palette
        layer_palette_mult = nodes.new("ShaderNodeMixRGB")
        layer_palette_mult.location = (-220, 40)
        layer_palette_mult.blend_type = "MULTIPLY"
        layer_palette_mult.inputs[0].default_value = 1.0
        links.new(layer_tint_mult.outputs[0], layer_palette_mult.inputs[1])
        links.new(_output_socket(group_input, "Layer Palette"), layer_palette_mult.inputs[2])

        # out_color = mix(base_final, layer_final, Wear Factor)
        wear_color_mix = nodes.new("ShaderNodeMixRGB")
        wear_color_mix.location = (20, 160)
        wear_color_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Wear Factor"), wear_color_mix.inputs[0])
        links.new(base_mult.outputs[0], wear_color_mix.inputs[1])
        links.new(layer_palette_mult.outputs[0], wear_color_mix.inputs[2])

        # out_rough = mix(Base Roughness, Layer Roughness, Wear Factor)
        wear_rough_mix = nodes.new("ShaderNodeMix")
        wear_rough_mix.location = (20, -120)
        if hasattr(wear_rough_mix, "data_type"):
            wear_rough_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Wear Factor"), wear_rough_mix.inputs[0])
        links.new(_output_socket(group_input, "Base Roughness"), wear_rough_mix.inputs[2])
        links.new(_output_socket(group_input, "Layer Roughness"), wear_rough_mix.inputs[3])

        links.new(wear_color_mix.outputs[0], group_output.inputs["Color"])
        links.new(wear_rough_mix.outputs[0], group_output.inputs["Roughness"])
        group_tree["starbreaker_runtime_built_signature"] = "layered_inputs_v1"
        return group_tree

    def _ensure_runtime_principled_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap Principled BSDF + NormalMap + Bump into a shader group.

        The shadowless behaviour is provided separately by
        :meth:`_ensure_runtime_shadowless_wrapper_group`, which the caller
        wraps around this group's output only when the material is supposed
        to cast no shadows. Unconditionally embedding the shadowless
        ``MixShader`` / ``LightPath`` / ``Transparent`` chain here inflates
        Cycles kernel memory for every non-shadowless material, so it lives
        outside.

        Inputs:
            Base Color          color
            Roughness           float
            Metallic            float   (default 0)
            Normal Color        color   (raw image color; default (0.5,0.5,1,1))
            Normal Strength     float   (default 1.0)
            Use Normal          float   (0 = geometry normal, 1 = normal map)
            Height              float   (default 0)
            Bump Strength       float   (default 0.02)
            Use Bump            float   (0 = skip bump, 1 = apply bump)
            Alpha               float   (default 1)
            Emission Color      color   (default black)
            Emission Strength   float   (default 0)

        Outputs:
            Shader              shader
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Principled",
            "principled_v2",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeNormalMap": 1,
                "ShaderNodeBump": 1,
                "ShaderNodeNewGeometry": 1,
                "ShaderNodeMix": 2,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Principled",
            signature="principled_v2",
            inputs=[
                ("Base Color", "NodeSocketColor"),
                ("Roughness", "NodeSocketFloat"),
                ("Metallic", "NodeSocketFloat"),
                ("Normal Color", "NodeSocketColor"),
                ("Normal Strength", "NodeSocketFloat"),
                ("Use Normal", "NodeSocketFloat"),
                ("Height", "NodeSocketFloat"),
                ("Bump Strength", "NodeSocketFloat"),
                ("Use Bump", "NodeSocketFloat"),
                ("Alpha", "NodeSocketFloat"),
                ("Emission Color", "NodeSocketColor"),
                ("Emission Strength", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "principled_v2":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        _set_group_input_default(group_input, "Base Color", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Roughness", 0.45)
        _set_group_input_default(group_input, "Metallic", 0.0)
        _set_group_input_default(group_input, "Normal Color", (0.5, 0.5, 1.0, 1.0))
        _set_group_input_default(group_input, "Normal Strength", 1.0)
        _set_group_input_default(group_input, "Use Normal", 0.0)
        _set_group_input_default(group_input, "Height", 0.0)
        _set_group_input_default(group_input, "Bump Strength", 0.02)
        _set_group_input_default(group_input, "Use Bump", 0.0)
        _set_group_input_default(group_input, "Alpha", 1.0)
        _set_group_input_default(group_input, "Emission Color", (0.0, 0.0, 0.0, 1.0))
        _set_group_input_default(group_input, "Emission Strength", 0.0)

        # Normal map chain: NormalMap driven by Normal Color + Strength.
        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.location = (-620, -40)
        links.new(_output_socket(group_input, "Normal Color"), _input_socket(normal_map, "Color"))
        links.new(_output_socket(group_input, "Normal Strength"), _input_socket(normal_map, "Strength"))

        # Fallback geometry normal.
        geometry = nodes.new("ShaderNodeNewGeometry")
        geometry.location = (-620, -260)

        # Toggle between geometry and normal map via Use Normal.
        normal_toggle = nodes.new("ShaderNodeMix")
        normal_toggle.location = (-420, -140)
        if hasattr(normal_toggle, "data_type"):
            normal_toggle.data_type = "VECTOR"
        links.new(_output_socket(group_input, "Use Normal"), normal_toggle.inputs[0])
        links.new(_output_socket(geometry, "Normal"), normal_toggle.inputs[4])
        links.new(_output_socket(normal_map, "Normal"), normal_toggle.inputs[5])

        # Bump node: feeds off the toggled normal vector.
        bump = nodes.new("ShaderNodeBump")
        bump.location = (-200, -180)
        links.new(_output_socket(group_input, "Height"), _input_socket(bump, "Height"))
        links.new(_output_socket(group_input, "Bump Strength"), _input_socket(bump, "Strength"))
        links.new(normal_toggle.outputs[1], _input_socket(bump, "Normal"))

        # Toggle between "no bump" (normal_toggle output) and bump output.
        bump_toggle = nodes.new("ShaderNodeMix")
        bump_toggle.location = (0, -120)
        if hasattr(bump_toggle, "data_type"):
            bump_toggle.data_type = "VECTOR"
        links.new(_output_socket(group_input, "Use Bump"), bump_toggle.inputs[0])
        links.new(normal_toggle.outputs[1], bump_toggle.inputs[4])
        links.new(_output_socket(bump, "Normal"), bump_toggle.inputs[5])

        # Principled BSDF.
        principled = nodes.new("ShaderNodeBsdfPrincipled")
        principled.location = (220, 0)
        principled.label = "StarBreaker Surface"
        links.new(_output_socket(group_input, "Base Color"), _input_socket(principled, "Base Color"))
        links.new(_output_socket(group_input, "Roughness"), _input_socket(principled, "Roughness"))
        links.new(_output_socket(group_input, "Metallic"), _input_socket(principled, "Metallic"))
        alpha_input = _input_socket(principled, "Alpha")
        if alpha_input is not None:
            links.new(_output_socket(group_input, "Alpha"), alpha_input)
        emission_color_input = _input_socket(principled, "Emission Color", "Emission")
        if emission_color_input is not None:
            links.new(_output_socket(group_input, "Emission Color"), emission_color_input)
        emission_strength_input = _input_socket(principled, "Emission Strength")
        if emission_strength_input is not None:
            links.new(_output_socket(group_input, "Emission Strength"), emission_strength_input)
        links.new(bump_toggle.outputs[1], _input_socket(principled, "Normal"))

        links.new(principled.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "principled_v2"
        return group_tree

    def _ensure_runtime_hardsurface_stencil_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap the HardSurface stencil overlay chain into a fixed-shape shader group.

        See :meth:`_hard_surface_stencil_overlay_sockets` for the caller-side
        contract. Inputs and outputs mirror the public params consumed by that
        helper; ``Mode`` selects between single-channel (0.0) and multi-channel
        (1.0) composition.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime HardSurface Stencil",
            "hardsurface_stencil_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                # 1 splitting the stencil color into RGB channel masks.
                "ShaderNodeSeparateColor": 1,
                # 3 RGBToBW: stencil luma, breakup luma, specular grayscale.
                "ShaderNodeRGBToBW": 3,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime HardSurface Stencil",
            signature="hardsurface_stencil_v1",
            inputs=[
                ("Stencil Color", "NodeSocketColor"),
                ("Stencil Alpha", "NodeSocketFloat"),
                ("Breakup Color", "NodeSocketColor"),
                ("Breakup Alpha", "NodeSocketFloat"),
                ("Breakup Strength", "NodeSocketFloat"),
                ("Breakup Enable", "NodeSocketFloat"),
                ("Stencil Opacity", "NodeSocketFloat"),
                ("Stencil Glossiness", "NodeSocketFloat"),
                ("Mode", "NodeSocketFloat"),
                ("Tint1", "NodeSocketColor"),
                ("Tint2", "NodeSocketColor"),
                ("Tint3", "NodeSocketColor"),
                ("Tint1 Enable", "NodeSocketFloat"),
                ("Tint2 Enable", "NodeSocketFloat"),
                ("Tint3 Enable", "NodeSocketFloat"),
                ("Specular1", "NodeSocketColor"),
                ("Specular2", "NodeSocketColor"),
                ("Specular3", "NodeSocketColor"),
                ("Specular1 Enable", "NodeSocketFloat"),
                ("Specular2 Enable", "NodeSocketFloat"),
                ("Specular3 Enable", "NodeSocketFloat"),
            ],
            outputs=[
                ("Color", "NodeSocketColor"),
                ("Color Factor", "NodeSocketFloat"),
                ("Factor", "NodeSocketFloat"),
                ("Roughness", "NodeSocketFloat"),
                ("Specular", "NodeSocketFloat"),
                ("Specular Tint", "NodeSocketColor"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "hardsurface_stencil_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        # Identity defaults.
        _set_group_input_default(group_input, "Stencil Color", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Stencil Alpha", 1.0)
        _set_group_input_default(group_input, "Breakup Color", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Breakup Alpha", 1.0)
        _set_group_input_default(group_input, "Breakup Strength", 0.0)
        _set_group_input_default(group_input, "Breakup Enable", 0.0)
        _set_group_input_default(group_input, "Stencil Opacity", 1.0)
        _set_group_input_default(group_input, "Stencil Glossiness", 0.0)
        _set_group_input_default(group_input, "Mode", 0.0)
        _set_group_input_default(group_input, "Tint1", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Tint2", (0.0, 0.0, 0.0, 1.0))
        _set_group_input_default(group_input, "Tint3", (0.0, 0.0, 0.0, 1.0))
        _set_group_input_default(group_input, "Tint1 Enable", 0.0)
        _set_group_input_default(group_input, "Tint2 Enable", 0.0)
        _set_group_input_default(group_input, "Tint3 Enable", 0.0)
        _set_group_input_default(group_input, "Specular1", (0.0, 0.0, 0.0, 1.0))
        _set_group_input_default(group_input, "Specular2", (0.0, 0.0, 0.0, 1.0))
        _set_group_input_default(group_input, "Specular3", (0.0, 0.0, 0.0, 1.0))
        _set_group_input_default(group_input, "Specular1 Enable", 0.0)
        _set_group_input_default(group_input, "Specular2 Enable", 0.0)
        _set_group_input_default(group_input, "Specular3 Enable", 0.0)

        def _mul_f(a, b, *, x, y):
            m = nodes.new("ShaderNodeMath")
            m.location = (x, y)
            m.operation = "MULTIPLY"
            m.use_clamp = False
            links.new(a, m.inputs[0])
            links.new(b, m.inputs[1])
            return m.outputs[0]

        def _add_f(a, b, *, x, y, clamp=False):
            m = nodes.new("ShaderNodeMath")
            m.location = (x, y)
            m.operation = "ADD"
            m.use_clamp = clamp
            links.new(a, m.inputs[0])
            links.new(b, m.inputs[1])
            return m.outputs[0]

        def _sub_f(a, b, *, x, y, clamp=True):
            m = nodes.new("ShaderNodeMath")
            m.location = (x, y)
            m.operation = "SUBTRACT"
            m.use_clamp = clamp
            links.new(a, m.inputs[0])
            links.new(b, m.inputs[1])
            return m.outputs[0]

        def _mix_f(a, b, factor, *, x, y):
            m = nodes.new("ShaderNodeMix")
            m.location = (x, y)
            if hasattr(m, "data_type"):
                m.data_type = "FLOAT"
            links.new(factor, m.inputs[0])
            links.new(a, m.inputs[2])
            links.new(b, m.inputs[3])
            return m.outputs[0]

        def _mix_c(a, b, factor, *, x, y, blend="MIX"):
            m = nodes.new("ShaderNodeMixRGB")
            m.location = (x, y)
            m.blend_type = blend
            links.new(factor, m.inputs[0])
            links.new(a, m.inputs[1])
            links.new(b, m.inputs[2])
            return m.outputs[0]

        def _mul_c(a, b, *, x, y):
            m = nodes.new("ShaderNodeMixRGB")
            m.location = (x, y)
            m.blend_type = "MULTIPLY"
            m.inputs[0].default_value = 1.0
            links.new(a, m.inputs[1])
            links.new(b, m.inputs[2])
            return m.outputs[0]

        def _add_c(a, b, *, x, y):
            m = nodes.new("ShaderNodeMixRGB")
            m.location = (x, y)
            m.blend_type = "ADD"
            m.inputs[0].default_value = 1.0
            links.new(a, m.inputs[1])
            links.new(b, m.inputs[2])
            return m.outputs[0]

        stencil_color = _output_socket(group_input, "Stencil Color")
        stencil_alpha = _output_socket(group_input, "Stencil Alpha")
        breakup_color = _output_socket(group_input, "Breakup Color")
        breakup_alpha = _output_socket(group_input, "Breakup Alpha")
        breakup_strength = _output_socket(group_input, "Breakup Strength")
        breakup_enable = _output_socket(group_input, "Breakup Enable")
        stencil_opacity = _output_socket(group_input, "Stencil Opacity")
        stencil_gloss = _output_socket(group_input, "Stencil Glossiness")
        mode = _output_socket(group_input, "Mode")

        # Split stencil RGB → channel masks, each multiplied by stencil alpha.
        separate = nodes.new("ShaderNodeSeparateColor")
        separate.location = (-1000, 200)
        if hasattr(separate, "mode"):
            separate.mode = "RGB"
        links.new(stencil_color, separate.inputs[0])
        r_raw = separate.outputs[0]
        g_raw = separate.outputs[1]
        b_raw = separate.outputs[2]
        m_r = _mul_f(r_raw, stencil_alpha, x=-800, y=280)
        m_g = _mul_f(g_raw, stencil_alpha, x=-800, y=160)
        m_b = _mul_f(b_raw, stencil_alpha, x=-800, y=40)

        # Luma mask from stencil color × alpha (used for single-channel factor).
        stencil_luma = nodes.new("ShaderNodeRGBToBW")
        stencil_luma.location = (-1000, -60)
        links.new(stencil_color, stencil_luma.inputs[0])
        stencil_mask = _mul_f(stencil_luma.outputs[0], stencil_alpha, x=-800, y=-80)

        # Multi-channel: per-channel enabled masks.
        e1 = _output_socket(group_input, "Tint1 Enable")
        e2 = _output_socket(group_input, "Tint2 Enable")
        e3 = _output_socket(group_input, "Tint3 Enable")
        tm_r = _mul_f(m_r, e1, x=-580, y=280)
        tm_g = _mul_f(m_g, e2, x=-580, y=160)
        tm_b = _mul_f(m_b, e3, x=-580, y=40)

        # Per-channel colored contributions: tint × mask.
        tint1 = _output_socket(group_input, "Tint1")
        tint2 = _output_socket(group_input, "Tint2")
        tint3 = _output_socket(group_input, "Tint3")
        # Build masked tint: mix(black, tint, tm).
        black = nodes.new("ShaderNodeRGB")
        black.location = (-580, -260)
        black.outputs[0].default_value = (0.0, 0.0, 0.0, 1.0)
        black_socket = black.outputs[0]
        masked_1 = _mix_c(black_socket, tint1, tm_r, x=-380, y=280)
        masked_2 = _mix_c(black_socket, tint2, tm_g, x=-380, y=160)
        masked_3 = _mix_c(black_socket, tint3, tm_b, x=-380, y=40)
        multi_color_12 = _add_c(masked_1, masked_2, x=-180, y=220)
        multi_color = _add_c(multi_color_12, masked_3, x=20, y=180)

        multi_factor_12 = _add_f(tm_r, tm_g, x=-180, y=60, clamp=True)
        multi_factor = _add_f(multi_factor_12, tm_b, x=20, y=40, clamp=True)

        # Single-channel: raw stencil × mix(white, Tint1, Tint1Enable).
        white = nodes.new("ShaderNodeRGB")
        white.location = (-580, 440)
        white.outputs[0].default_value = (1.0, 1.0, 1.0, 1.0)
        white_socket = white.outputs[0]
        tint1_gated = _mix_c(white_socket, tint1, e1, x=-380, y=440)
        single_color = _mul_c(stencil_color, tint1_gated, x=-180, y=400)

        # Mode mix.
        color_mode = _mix_c(single_color, multi_color, mode, x=220, y=300)
        factor_mode = _mix_f(stencil_mask, multi_factor, mode, x=220, y=120)

        # Breakup.
        breakup_luma = nodes.new("ShaderNodeRGBToBW")
        breakup_luma.location = (-1000, -260)
        links.new(breakup_color, breakup_luma.inputs[0])
        breakup_mask = _mul_f(breakup_luma.outputs[0], breakup_alpha, x=-800, y=-260)
        # breakup_factor = mix(1, breakup_mask, breakup_strength)
        one_const = nodes.new("ShaderNodeValue")
        one_const.location = (-800, -440)
        one_const.outputs[0].default_value = 1.0
        one_socket = one_const.outputs[0]
        breakup_blend = _mix_f(one_socket, breakup_mask, breakup_strength, x=-580, y=-260)
        # Apply only when BreakupEnable = 1.
        breakup_applied = _mix_f(one_socket, breakup_blend, breakup_enable, x=-380, y=-260)

        # factor_out = factor_mode × breakup_applied × opacity.
        factor_with_breakup = _mul_f(factor_mode, breakup_applied, x=420, y=80)
        factor_out = _mul_f(factor_with_breakup, stencil_opacity, x=620, y=80)

        # Roughness output = 1 - gloss.
        roughness_out = _sub_f(one_socket, stencil_gloss, x=420, y=-60, clamp=True)

        # Specular accumulation: sum(mask_i × enable_i × spec_i) then RGBToBW.
        spec1 = _output_socket(group_input, "Specular1")
        spec2 = _output_socket(group_input, "Specular2")
        spec3 = _output_socket(group_input, "Specular3")
        se1 = _output_socket(group_input, "Specular1 Enable")
        se2 = _output_socket(group_input, "Specular2 Enable")
        se3 = _output_socket(group_input, "Specular3 Enable")
        # channel_mask × specular_enable
        sm_r = _mul_f(m_r, se1, x=-580, y=-540)
        sm_g = _mul_f(m_g, se2, x=-580, y=-660)
        sm_b = _mul_f(m_b, se3, x=-580, y=-780)
        masked_s1 = _mix_c(black_socket, spec1, sm_r, x=-380, y=-540)
        masked_s2 = _mix_c(black_socket, spec2, sm_g, x=-380, y=-660)
        masked_s3 = _mix_c(black_socket, spec3, sm_b, x=-380, y=-780)
        spec_sum_12 = _add_c(masked_s1, masked_s2, x=-180, y=-580)
        spec_tint_socket = _add_c(spec_sum_12, masked_s3, x=20, y=-620)
        spec_gray = nodes.new("ShaderNodeRGBToBW")
        spec_gray.location = (220, -600)
        links.new(spec_tint_socket, spec_gray.inputs[0])
        spec_socket = spec_gray.outputs[0]

        # Wire outputs.
        links.new(color_mode, group_output.inputs["Color"])
        links.new(factor_out, group_output.inputs["Factor"])
        links.new(factor_out, group_output.inputs["Color Factor"])
        links.new(roughness_out, group_output.inputs["Roughness"])
        links.new(spec_socket, group_output.inputs["Specular"])
        links.new(spec_tint_socket, group_output.inputs["Specular Tint"])

        group_tree["starbreaker_runtime_built_signature"] = "hardsurface_stencil_v1"
        return group_tree

    def _ensure_runtime_channel_split_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap a SeparateColor + alpha passthrough into a shader group.

        Used by :meth:`_detail_texture_channels` so top-level material graphs
        only contain the image texture plus the group node, not a bare
        ``ShaderNodeSeparateColor``.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Channel Split",
            "channel_split_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeSeparateColor": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Channel Split",
            signature="channel_split_v1",
            inputs=[
                ("Color", "NodeSocketColor"),
                ("Alpha", "NodeSocketFloat"),
            ],
            outputs=[
                ("R", "NodeSocketFloat"),
                ("G", "NodeSocketFloat"),
                ("B", "NodeSocketFloat"),
                ("Alpha", "NodeSocketFloat"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "channel_split_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        _set_group_input_default(group_input, "Color", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Alpha", 1.0)

        separate = nodes.new("ShaderNodeSeparateColor")
        separate.location = (0, 0)
        if hasattr(separate, "mode"):
            separate.mode = "RGB"
        links.new(_output_socket(group_input, "Color"), separate.inputs[0])
        links.new(separate.outputs[0], group_output.inputs["R"])
        links.new(separate.outputs[1], group_output.inputs["G"])
        links.new(separate.outputs[2], group_output.inputs["B"])
        links.new(_output_socket(group_input, "Alpha"), group_output.inputs["Alpha"])

        group_tree["starbreaker_runtime_built_signature"] = "channel_split_v1"
        return group_tree

    def _ensure_runtime_smoothness_roughness_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap the (1 - smoothness) invert into a shader group.

        Used by :meth:`_invert_value_socket` (only caller) so layered_wear
        smoothness-to-roughness conversion keeps the top level clean.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Smoothness To Roughness",
            "smoothness_roughness_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeMath": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Smoothness To Roughness",
            signature="smoothness_roughness_v1",
            inputs=[
                ("Smoothness", "NodeSocketFloat"),
            ],
            outputs=[
                ("Roughness", "NodeSocketFloat"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "smoothness_roughness_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        _set_group_input_default(group_input, "Smoothness", 0.5)

        invert = nodes.new("ShaderNodeMath")
        invert.location = (0, 0)
        invert.operation = "SUBTRACT"
        invert.use_clamp = True
        invert.inputs[0].default_value = 1.0
        links.new(_output_socket(group_input, "Smoothness"), invert.inputs[1])
        links.new(invert.outputs[0], group_output.inputs["Roughness"])

        group_tree["starbreaker_runtime_built_signature"] = "smoothness_roughness_v1"
        return group_tree

    def _ensure_runtime_color_to_luma_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap a single ShaderNodeRGBToBW (color → grayscale) into a group.

        Used by :meth:`_specular_socket_for_texture_path` and the palette
        specular inline block in :meth:`_emit_layer_surface_input_block` so
        top-level material graphs stay free of bare ``ShaderNodeRGBToBW``.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Color To Luma",
            "color_to_luma_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeRGBToBW": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Color To Luma",
            signature="color_to_luma_v1",
            inputs=[
                ("Color", "NodeSocketColor"),
            ],
            outputs=[
                ("Luma", "NodeSocketFloat"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "color_to_luma_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        _set_group_input_default(group_input, "Color", (1.0, 1.0, 1.0, 1.0))

        rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
        rgb_to_bw.location = (0, 0)
        links.new(_output_socket(group_input, "Color"), rgb_to_bw.inputs[0])
        links.new(rgb_to_bw.outputs[0], group_output.inputs["Luma"])

        group_tree["starbreaker_runtime_built_signature"] = "color_to_luma_v1"
        return group_tree

    def _ensure_runtime_shadowless_wrapper_group(self) -> bpy.types.ShaderNodeTree:
        """Wrap a shader in a shadow-ray → transparent mix.

        Input ``Shader`` is passed through unchanged on camera / diffuse /
        glossy / transmission rays; on shadow rays it is replaced by a
        ``BsdfTransparent`` so the surface casts no shadow.

        This exists as a standalone wrapper so the main surface groups
        (``StarBreaker Runtime Principled``, ``StarBreaker Runtime
        HardSurface``) do not have to carry the ``LightPath`` /
        ``MixShader`` / ``BsdfTransparent`` chain unconditionally. Cycles
        must compile both branches of every ``MixShader`` (the factor is
        runtime-variable), so keeping the chain out of the always-
        instantiated surface groups avoids inflating kernel memory for the
        majority of materials that do cast shadows normally.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Shadowless Wrapper",
            "shadowless_wrapper_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeLightPath": 1,
                "ShaderNodeBsdfTransparent": 1,
                "ShaderNodeMixShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Shadowless Wrapper",
            signature="shadowless_wrapper_v1",
            inputs=[
                ("Shader", "NodeSocketShader"),
            ],
            outputs=[
                ("Shader", "NodeSocketShader"),
            ],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "shadowless_wrapper_v1":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (-200, -200)
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (0, -200)
        mix_shader = nodes.new("ShaderNodeMixShader")
        mix_shader.location = (200, 0)
        links.new(_output_socket(light_path, "Is Shadow Ray"), mix_shader.inputs[0])
        links.new(_output_socket(group_input, "Shader"), mix_shader.inputs[1])
        links.new(transparent.outputs[0], mix_shader.inputs[2])
        links.new(mix_shader.outputs[0], group_output.inputs["Shader"])

        group_tree["starbreaker_runtime_built_signature"] = "shadowless_wrapper_v1"
        return group_tree

    def _ensure_runtime_illum_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Illum",
            "illum_v4",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeEmission": 1,
                "ShaderNodeAddShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Illum",
            signature="illum_v4",
            inputs=[
                ("Primary Color", "NodeSocketColor"),
                ("Primary Alpha", "NodeSocketFloat"),
                ("Primary Roughness", "NodeSocketFloat"),
                ("Primary Specular", "NodeSocketFloat"),
                ("Primary Normal", "NodeSocketVector"),
                ("Secondary Color", "NodeSocketColor"),
                ("Secondary Alpha", "NodeSocketFloat"),
                ("Secondary Roughness", "NodeSocketFloat"),
                ("Secondary Specular", "NodeSocketFloat"),
                ("Secondary Normal", "NodeSocketVector"),
                ("Blend Mask", "NodeSocketFloat"),
                ("Primary Height", "NodeSocketFloat"),
                ("Secondary Height", "NodeSocketFloat"),
                ("POM Strength", "NodeSocketFloat"),
                ("Emission Strength", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "illum_v4":
            return group_tree
        nodes = group_tree.nodes
        links = group_tree.links

        color_mix = nodes.new("ShaderNodeMixRGB")
        color_mix.location = (-700, 260)
        color_mix.blend_type = "MIX"
        links.new(_output_socket(group_input, "Blend Mask"), color_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Color"), color_mix.inputs[1])
        links.new(_output_socket(group_input, "Secondary Color"), color_mix.inputs[2])

        alpha_mix = nodes.new("ShaderNodeMix")
        alpha_mix.location = (-700, 80)
        if hasattr(alpha_mix, "data_type"):
            alpha_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), alpha_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Alpha"), alpha_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Alpha"), alpha_mix.inputs[3])

        roughness_mix = nodes.new("ShaderNodeMix")
        roughness_mix.location = (-700, -100)
        if hasattr(roughness_mix, "data_type"):
            roughness_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), roughness_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Roughness"), roughness_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Roughness"), roughness_mix.inputs[3])

        specular_mix = nodes.new("ShaderNodeMix")
        specular_mix.location = (-700, -280)
        if hasattr(specular_mix, "data_type"):
            specular_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), specular_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Specular"), specular_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Specular"), specular_mix.inputs[3])

        normal_mix = nodes.new("ShaderNodeMix")
        normal_mix.location = (-700, -500)
        if hasattr(normal_mix, "data_type"):
            normal_mix.data_type = "VECTOR"
        links.new(_output_socket(group_input, "Blend Mask"), normal_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Normal"), normal_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Normal"), normal_mix.inputs[3])

        height_mix = nodes.new("ShaderNodeMix")
        height_mix.location = (-700, -680)
        if hasattr(height_mix, "data_type"):
            height_mix.data_type = "FLOAT"
        links.new(_output_socket(group_input, "Blend Mask"), height_mix.inputs[0])
        links.new(_output_socket(group_input, "Primary Height"), height_mix.inputs[2])
        links.new(_output_socket(group_input, "Secondary Height"), height_mix.inputs[3])

        bump = nodes.new("ShaderNodeBump")
        bump.location = (-500, -560)
        links.new(_output_socket(group_input, "POM Strength"), bump.inputs[0])
        links.new(height_mix.outputs[0], bump.inputs[2])
        links.new(normal_mix.outputs[0], bump.inputs[3])

        principled = self._create_surface_bsdf(nodes)
        principled.location = (-120, 40)
        links.new(color_mix.outputs[0], _input_socket(principled, "Base Color"))
        links.new(alpha_mix.outputs[0], _input_socket(principled, "Alpha"))
        links.new(roughness_mix.outputs[0], _input_socket(principled, "Roughness"))
        specular_input = _input_socket(principled, "Specular IOR Level", "Specular")
        if specular_input is not None:
            links.new(specular_mix.outputs[0], specular_input)
        normal_input = _input_socket(principled, "Normal")
        if normal_input is not None:
            links.new(bump.outputs[0], normal_input)

        emission = nodes.new("ShaderNodeEmission")
        emission.location = (-120, -220)
        links.new(color_mix.outputs[0], emission.inputs["Color"])
        links.new(_output_socket(group_input, "Emission Strength"), emission.inputs["Strength"])

        add_shader = nodes.new("ShaderNodeAddShader")
        add_shader.location = (120, -40)
        links.new(principled.outputs[0], add_shader.inputs[0])
        links.new(emission.outputs[0], add_shader.inputs[1])

        links.new(add_shader.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "illum_v4"
        return group_tree

    def _ensure_runtime_parallax_group(
        self,
        height_image: bpy.types.Image | None = None,
    ) -> bpy.types.ShaderNodeTree | None:
        """Phase 12 (POM plan, Phase 1 — revised): return a per-height-image
        copy of the production-quality POM pipeline authored in
        ``docs/StarBreaker/POM-test.blend``.

        A true parallax-occlusion effect requires a ray-march over the
        height field, which Blender's shader graph cannot express as a
        runtime loop — the reference file unrolls it as ~30 sampling
        steps via nested ``POM_Vector`` → ``POM_10x_Layer_Steps`` →
        ``POM_parallax`` → ``POM_disp`` groups. Because ``POM_disp``
        contains a ``ShaderNodeTexImage`` sampler and Blender shader
        groups cannot accept a sampler datablock as an *input*, every
        material that uses a different height map needs its own copy of
        ``POM_disp`` — and, transitively, its own copies of every group
        that references ``POM_disp`` up the chain.

        This helper appends the bundled ``POM_Vector`` + dependencies
        from ``starbreaker_addon/resources/pom_library.blend`` once per
        *height image*, renames the appended chain with a stable
        ``StarBreaker POM …`` prefix that encodes the image name, and
        patches the embedded ``HeightMap`` / ``POM_disp`` tex samplers
        to point at that image. Subsequent calls with the same image
        return the cached copy.

        Returns ``None`` when no height image is supplied (single-sample
        offset mapping is no longer used — silent no-op signals the
        caller to skip parallax wiring).
        """
        if height_image is None:
            return None
        image_key = height_image.name
        cached_name = f"StarBreaker POM [{image_key}]"
        cached = bpy.data.node_groups.get(cached_name)
        current_mode = self._current_pom_detail_mode()
        _configure_runtime_pom_detail_group(current_mode)
        if cached is not None:
            _ensure_pom_detail_control_on_tree(cached, current_mode)
            return cached

        library_path = _POM_LIBRARY_PATH
        if not library_path.is_file():
            return None

        # Snapshot existing group names so we can tell which ones were
        # created by this append (Blender auto-suffixes duplicates).
        before = {g.name for g in bpy.data.node_groups}
        with bpy.data.libraries.load(str(library_path), link=False) as (src, dst):
            # Appending the root also pulls in every group it references
            # transitively; request the full set explicitly so the
            # reference graph stays intact even when ``POM_Vector`` has
            # already been appended previously.
            dst.node_groups = [name for name in _POM_LIBRARY_ROOT_GROUPS if name in src.node_groups]
        added = [g for g in bpy.data.node_groups if g.name not in before]

        # Find the newly-appended ``POM_Vector`` — Blender will have
        # renamed it to ``POM_Vector.001`` etc. if the name collided.
        pom_vector_new = None
        for g in added:
            if g.name.startswith("POM_Vector"):
                pom_vector_new = g
                break
        if pom_vector_new is None:
            # No ``POM_Vector`` appended — treat as silent failure so
            # the caller falls back to no parallax wiring.
            for g in added:
                g.use_fake_user = False
                if g.users == 0:
                    bpy.data.node_groups.remove(g, do_unlink=True)
            return None

        # Patch every ``ShaderNodeTexImage`` inside the appended group
        # chain to point at ``height_image``. ``POM_disp`` and
        # ``HeightMap`` are the only groups in the reference pipeline
        # that carry samplers, but we walk the full appended set so a
        # future library refactor doesn't silently leave stale images
        # behind.
        for g in added:
            for n in g.nodes:
                if n.bl_idname == "ShaderNodeTexImage":
                    n.image = height_image

        # Phase 17 — share samplerless, reference-less helpers across
        # every POM instance. Blender shader groups cannot accept an
        # ImageTexture datablock as a socket input, so groups that
        # contain a sampler (``POM_disp``) — and any group that
        # references them transitively (``POM_parallax``,
        # ``POM_10x_Layer_Steps``, ``POM_Vector``) — must remain
        # per-image. Pure-math helpers like ``tangent_space`` and
        # ``Clamp`` hold no sampler and reference no other groups, so
        # we can collapse every copy back onto a canonical shared
        # version, dropping ~one node group per POM-using material.
        _CANONICAL_SHARED = {"tangent_space", "Clamp"}
        canonical_by_base: dict[str, bpy.types.ShaderNodeTree] = {}
        for existing in bpy.data.node_groups:
            base = existing.name.rsplit(".", 1)[0]
            if (
                base in _CANONICAL_SHARED
                and existing not in added
                and not any(n.bl_idname == "ShaderNodeTexImage" for n in existing.nodes)
                and not any(
                    n.bl_idname == "ShaderNodeGroup" and n.node_tree is not None
                    for n in existing.nodes
                )
            ):
                canonical_by_base.setdefault(base, existing)

        redundant: list[bpy.types.ShaderNodeTree] = []
        promoted: list[bpy.types.ShaderNodeTree] = []
        for new_group in list(added):
            base = new_group.name.rsplit(".", 1)[0]
            if base not in _CANONICAL_SHARED:
                continue
            if any(n.bl_idname == "ShaderNodeTexImage" for n in new_group.nodes):
                continue
            if any(
                n.bl_idname == "ShaderNodeGroup" and n.node_tree is not None
                for n in new_group.nodes
            ):
                continue
            canonical = canonical_by_base.get(base)
            if canonical is None:
                # First time we've seen this helper — promote the
                # newly-appended copy to the canonical and keep it
                # under its bare name (no ``StarBreaker POM [...]``
                # prefix) so subsequent imports find it on sight.
                new_group.name = base
                new_group.use_fake_user = True
                canonical_by_base[base] = new_group
                promoted.append(new_group)
                continue
            # Rewire every reference from the redundant copy to the
            # canonical, then mark it for removal.
            for g in added:
                for n in g.nodes:
                    if (
                        n.bl_idname == "ShaderNodeGroup"
                        and n.node_tree is new_group
                    ):
                        n.node_tree = canonical
            redundant.append(new_group)

        for new_group in redundant:
            added.remove(new_group)
            if new_group.users == 0:
                bpy.data.node_groups.remove(new_group, do_unlink=True)
        # Promoted canonicals keep their bare base name and must not be
        # rewritten by the prefix rename below.
        for new_group in promoted:
            if new_group in added:
                added.remove(new_group)

        # Rename the root (and keep the internal chain under a prefix
        # so it's obvious in the node-group browser which copies belong
        # to which image).
        pom_vector_new.name = cached_name
        pom_vector_new.use_fake_user = False
        prefix = f"StarBreaker POM [{image_key}] / "
        for g in added:
            if g is pom_vector_new:
                continue
            # Strip any ``.001``-style suffix Blender added.
            base = g.name.rsplit(".", 1)[0]
            # Only prefix copies we appended as part of the POM chain;
            # shared helpers that happened to already exist would have
            # been excluded by the ``before`` snapshot above.
            g.name = f"{prefix}{base}"
            g.use_fake_user = False

        _ensure_pom_detail_control_on_tree(pom_vector_new, current_mode)

        return pom_vector_new

    def _ensure_runtime_gobo_group(self) -> bpy.types.ShaderNodeTree:
        """Shared shader group for spot-light gobo (projector) textures.

        Exposes two inputs:

        - ``Gobo Image`` (NodeSocketColor): sampled texture colour.
        - ``Strength`` (NodeSocketFloat): emission strength multiplier.

        Output ``Shader`` is a ``ShaderNodeEmission`` driven by the gobo
        image colour. Intended to live inside a light's node tree; the
        caller (``create_light``) feeds the image via a
        ``ShaderNodeTexImage`` that samples ``Texture Coordinate > UV``.

        See ``docs/StarBreaker/lights-research.md``.
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Gobo",
            "gobo_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeEmission": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Gobo",
            signature="gobo_v1",
            inputs=[
                ("Gobo Image", "NodeSocketColor"),
                ("Strength", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "gobo_v1":
            return group_tree
        _set_group_input_default(group_input, "Gobo Image", (1.0, 1.0, 1.0, 1.0))
        _set_group_input_default(group_input, "Strength", 1.0)
        nodes = group_tree.nodes
        links = group_tree.links
        emission = nodes.new("ShaderNodeEmission")
        emission.location = (0, 0)
        links.new(group_input.outputs["Gobo Image"], emission.inputs["Color"])
        links.new(group_input.outputs["Strength"], emission.inputs["Strength"])
        links.new(emission.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "gobo_v1"
        return group_tree
