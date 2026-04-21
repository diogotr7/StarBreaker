"""Low-level Blender node helpers used throughout the runtime.

Extracted from ``runtime.py`` in Phase 7. Pure helpers that do not depend
on ``PackageImporter`` state.
"""

from __future__ import annotations

from typing import Any


def _input_socket(node: Any, *names: str) -> Any:
    for name in names:
        socket = node.inputs.get(name)
        if socket is not None:
            return socket
    if getattr(node, "bl_idname", "") == "ShaderNodeGroup":
        node_tree = getattr(node, "node_tree", None)
        if node_tree is not None:
            try:
                node.node_tree = node_tree
            except Exception:
                return None
            for name in names:
                socket = node.inputs.get(name)
                if socket is not None:
                    return socket
    return None


def _output_socket(node: Any, *names: str) -> Any:
    for name in names:
        socket = node.outputs.get(name)
        if socket is not None:
            return socket
    if getattr(node, "bl_idname", "") == "ShaderNodeGroup":
        node_tree = getattr(node, "node_tree", None)
        if node_tree is not None:
            try:
                node.node_tree = node_tree
            except Exception:
                return None
            for name in names:
                socket = node.outputs.get(name)
                if socket is not None:
                    return socket
    return None


def _set_group_input_default(group_input_node: Any, socket_name: str, value: Any) -> None:
    """Set the default value for a named output socket on a NodeGroupInput node.

    Used inside ``_ensure_runtime_*_group`` builders to seed identity defaults
    so callers may leave sockets unlinked without changing the composed
    behaviour.
    """
    if group_input_node is None:
        return
    socket = group_input_node.outputs.get(socket_name)
    if socket is None:
        return
    try:
        socket.default_value = value
    except Exception:
        pass


def _refresh_group_node_sockets(node: Any) -> None:
    if getattr(node, "bl_idname", "") != "ShaderNodeGroup":
        return
    node_tree = getattr(node, "node_tree", None)
    if node_tree is None:
        return
    try:
        node.node_tree = node_tree
    except Exception:
        return
