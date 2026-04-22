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

    Also propagates the default to the group tree's interface socket so that
    newly created instances of the group pick up the same default on their
    unlinked input sockets. Without this, interface sockets start at their
    socket-type zero (e.g. Alpha=0.0) even when the internal Group Input
    node outputs default to the intended identity value (e.g. Alpha=1.0),
    causing fully-transparent materials whenever the caller does not wire
    the socket.
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
    tree = getattr(group_input_node, "id_data", None)
    if tree is not None and hasattr(tree, "interface"):
        try:
            items = tree.interface.items_tree
        except Exception:
            items = ()
        for item in items:
            if (
                getattr(item, "in_out", None) == "INPUT"
                and getattr(item, "name", None) == socket_name
                and hasattr(item, "default_value")
            ):
                try:
                    item.default_value = value
                except Exception:
                    pass
                break


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
