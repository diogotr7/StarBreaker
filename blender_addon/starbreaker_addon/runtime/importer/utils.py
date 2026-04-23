"""Module-level helper functions used by PackageImporter and mixins.

Extracted from ``runtime/_legacy.py`` as part of Phase 7.5g.
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
from typing import Any

import bpy
from mathutils import Matrix, Quaternion

from ..constants import (
    GLTF_LIGHT_BASIS_CORRECTION,
    GLTF_PBR_WATTS_TO_LUMENS,
    MATERIAL_IDENTITY_SCHEMA,
    NON_COLOR_INPUT_KEYWORDS,
    PROP_IMPORTED_SLOT_MAP,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_PALETTE_SCOPE,
    PROP_SOURCE_NODE_NAME,
    PROP_SUBMATERIAL_JSON,
    SCENE_AXIS_CONVERSION,
    SCENE_AXIS_CONVERSION_INV,
)
from ..package_ops import _scene_instance_from_object, _string_prop
from ...manifest import MaterialSidecar, PackageBundle, PaletteRecord, SubmaterialRecord
from ...material_contract import ContractInput


def _material_identity(
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
    palette_scope: str,
) -> str:
    payload = {
        "schema": MATERIAL_IDENTITY_SCHEMA,
        "material_sidecar": _canonical_material_sidecar_path(sidecar_path, sidecar),
        "submaterial": submaterial.raw,
        "palette_scope": palette_scope,
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.blake2s(encoded, digest_size=16).hexdigest()


def _material_name(
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    material_identity: str,
) -> str:
    preferred_name = submaterial.blender_material_name or _derived_material_name(sidecar_path, sidecar, submaterial)
    existing = bpy.data.materials.get(preferred_name)
    if existing is None:
        return preferred_name

    existing_identity = existing.get(PROP_MATERIAL_IDENTITY)
    if isinstance(existing_identity, str) and existing_identity == material_identity:
        return preferred_name
    return f"{preferred_name}#{material_identity[:8]}"


def _canonical_material_sidecar_path(sidecar_path: str, sidecar: MaterialSidecar) -> str:
    return sidecar.normalized_export_relative_path or sidecar_path or sidecar.source_material_path or "material"


def _material_is_compatible(
    material: bpy.types.Material,
    package: PackageBundle,
    sidecar_path: str,
    sidecar: MaterialSidecar,
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
    palette_scope: str,
) -> bool:
    existing_sidecar_path = _string_prop(material, PROP_MATERIAL_SIDECAR)
    canonical_sidecar_path = _canonical_material_sidecar_path(sidecar_path, sidecar)
    if existing_sidecar_path is None or existing_sidecar_path not in {sidecar_path, canonical_sidecar_path}:
        return False

    payload = material.get(PROP_SUBMATERIAL_JSON)
    if not isinstance(payload, str):
        return False
    try:
        existing_submaterial = json.loads(payload)
    except json.JSONDecodeError:
        return False
    if existing_submaterial != submaterial.raw:
        return False

    if not _managed_material_runtime_graph_is_sane(material):
        return False

    return _string_prop(material, PROP_PALETTE_SCOPE) == palette_scope


def _managed_material_runtime_graph_is_sane(material: bpy.types.Material) -> bool:
    node_tree = material.node_tree
    if node_tree is None:
        return False

    hard_surface_nodes = [
        node
        for node in node_tree.nodes
        if node.bl_idname == "ShaderNodeGroup"
        and getattr(getattr(node, "node_tree", None), "name", "").startswith("StarBreaker Runtime HardSurface")
    ]
    if not hard_surface_nodes:
        return True

    for node in hard_surface_nodes:
        linked_inputs = {
            link.to_socket.name
            for link in node_tree.links
            if link.to_node == node
        }
        if not {
            "Primary Color",
            "Primary Alpha",
            "Primary Roughness",
        }.issubset(linked_inputs):
            return False
        if not any(
            link.from_node == node
            and link.from_socket.name == "Shader"
            and link.to_node.bl_idname == "ShaderNodeOutputMaterial"
            and link.to_socket.name == "Surface"
            for link in node_tree.links
        ):
            return False

    return True


def _derived_material_name(sidecar_path: str, sidecar: MaterialSidecar, submaterial: SubmaterialRecord) -> str:
    normalized_path = sidecar.normalized_export_relative_path or sidecar_path or sidecar.source_material_path or "material"
    sidecar_name = Path(normalized_path).name
    if sidecar_name.endswith(".materials.json"):
        sidecar_name = sidecar_name[: -len(".materials.json")]
    elif sidecar_name.endswith(".json"):
        sidecar_name = sidecar_name[: -len(".json")]

    submaterial_name = submaterial.submaterial_name or f"slot_{submaterial.index}"
    return f"{sidecar_name}:{submaterial_name}"


def _safe_identifier(value: str) -> str:
    safe = "".join(character if character.isalnum() else "_" for character in value)
    return safe.strip("_") or "value"


def _palette_group_name(package_name: str, palette_scope: str) -> str:
    return f"StarBreaker Palette {package_name} {_safe_identifier(palette_scope)}"


def _canonical_source_name(name: str) -> str:
    if len(name) > 4 and name[-4] == "." and name[-3:].isdigit():
        return name[:-4]
    return name


def _scene_position_to_blender(position: tuple[float, float, float]) -> tuple[float, float, float]:
    return (position[0], -position[2], position[1])


def _scene_matrix_to_blender(matrix_rows: Any) -> Matrix:
    matrix = Matrix(matrix_rows).transposed()
    return SCENE_AXIS_CONVERSION @ matrix @ SCENE_AXIS_CONVERSION_INV


def _scene_quaternion_to_blender(rotation: tuple[float, float, float, float]) -> Quaternion:
    if all(abs(component) <= 1e-8 for component in rotation):
        return Quaternion((1.0, 0.0, 0.0, 0.0))
    matrix = Quaternion(rotation).to_matrix().to_4x4()
    return (SCENE_AXIS_CONVERSION @ matrix @ SCENE_AXIS_CONVERSION_INV).to_quaternion().normalized()


def _scene_light_quaternion_to_blender(rotation: tuple[float, float, float, float]) -> Quaternion:
    return (_scene_quaternion_to_blender(rotation) @ GLTF_LIGHT_BASIS_CORRECTION).normalized()


def _blender_light_type(light: Any) -> str:
    light_type = str(getattr(light, "light_type", "") or "").strip().lower()
    if light_type in {"directional", "sun"}:
        return "SUN"
    if light_type in {"projector", "spot"}:
        return "SPOT"
    if light_type in {"omni", "point"}:
        return "POINT"
    if (light.inner_angle or 0.0) > 0.0 or (light.outer_angle or 0.0) > 0.0:
        return "SPOT"
    return "POINT"


def _light_energy_to_blender(intensity: float, blender_light_type: str) -> float:
    intensity = max(float(intensity), 0.0)
    if blender_light_type == "SUN":
        return intensity / GLTF_PBR_WATTS_TO_LUMENS
    return intensity * 4.0 * math.pi / GLTF_PBR_WATTS_TO_LUMENS


def _is_axis_conversion_root(obj: bpy.types.Object) -> bool:
    source_name = _canonical_source_name(str(obj.get(PROP_SOURCE_NODE_NAME, obj.name) or ""))
    return obj.data is None and source_name == "CryEngine_Z_up"


def _should_neutralize_axis_root(obj: bpy.types.Object, mesh_asset: str) -> bool:
    """Phase 12: neutralize any ``CryEngine_Z_up`` template root when the
    instance is parented to a hardpoint.

    Historically this additionally required ``_has_non_identity_descendants``
    to be true, which meant templates whose axis-conversion empty had
    only identity-transformed descendants (e.g. ``geo_vtol_fan`` inside
    ``rsi_aurora_mk2_fan_vtol.glb``) kept the 90° Z-up→Y-up rotation
    baked into their top-level transform even though the parent
    hardpoint already provides the correct orientation. For
    hardpoint-attached instances the axis conversion is always
    redundant — the scene matrix conversion
    (``_scene_matrix_to_blender``) has already brought the hardpoint
    into Blender space — so we strip it unconditionally.
    """
    return _is_axis_conversion_root(obj)


def _slot_mapping_for_object(obj: bpy.types.Object) -> list[int | None] | None:
    data = getattr(obj, "data", None)
    if data is None:
        return None
    mapping_raw = data.get(PROP_IMPORTED_SLOT_MAP)
    if not isinstance(mapping_raw, str) or not mapping_raw:
        return None
    try:
        parsed = json.loads(mapping_raw)
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, list):
        return None
    mapping: list[int | None] = []
    for value in parsed:
        if value is None:
            mapping.append(None)
            continue
        try:
            mapping.append(int(value))
        except (TypeError, ValueError):
            mapping.append(None)
    return mapping


def _slot_mapping_source_sidecar_path(obj: bpy.types.Object, current_sidecar_path: str) -> str:
    instance = _scene_instance_from_object(obj)
    if instance is not None and instance.material_sidecar:
        return instance.material_sidecar
    return current_sidecar_path


def _unique_submaterials_by_name(sidecar: MaterialSidecar) -> dict[str, SubmaterialRecord]:
    grouped: dict[str, list[SubmaterialRecord]] = {}
    for submaterial in sidecar.submaterials:
        name = submaterial.submaterial_name.strip()
        if not name:
            continue
        grouped.setdefault(name, []).append(submaterial)
    return {
        name: submaterials[0]
        for name, submaterials in grouped.items()
        if len(submaterials) == 1
    }


def _remapped_submaterial_for_slot(
    source_submaterial: SubmaterialRecord | None,
    fallback_index: int,
    target_submaterials_by_index: dict[int, SubmaterialRecord],
    target_submaterials_by_name: dict[str, SubmaterialRecord],
) -> SubmaterialRecord | None:
    if source_submaterial is not None:
        source_name = source_submaterial.submaterial_name.strip()
        if source_name:
            remapped = target_submaterials_by_name.get(source_name)
            if remapped is not None:
                return remapped
    return target_submaterials_by_index.get(fallback_index)


def _imported_slot_mapping_from_materials(materials: Any) -> list[int | None] | None:
    mapping: list[int | None] = []
    has_explicit_mapping = False
    for material in materials:
        submaterial_index = _imported_submaterial_index(material)
        if submaterial_index is not None:
            has_explicit_mapping = True
        mapping.append(submaterial_index)
    if not has_explicit_mapping:
        return None
    return mapping


def _imported_submaterial_index(material: bpy.types.Material | None) -> int | None:
    if material is None:
        return None
    semantic = material.get("semantic")
    if hasattr(semantic, "to_dict"):
        semantic = semantic.to_dict()
    if not isinstance(semantic, dict):
        return None
    material_set_identity = semantic.get("material_set_identity")
    if hasattr(material_set_identity, "to_dict"):
        material_set_identity = material_set_identity.to_dict()
    if not isinstance(material_set_identity, dict):
        return None
    submaterial_index = material_set_identity.get("submaterial_index")
    try:
        return int(submaterial_index)
    except (TypeError, ValueError):
        return None


def _contract_input_uses_color(contract_input: ContractInput) -> bool:
    semantic = (contract_input.semantic or contract_input.name).lower()
    return not any(keyword in semantic for keyword in NON_COLOR_INPUT_KEYWORDS)
