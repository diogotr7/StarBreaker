"""Top-level constants used across the StarBreaker runtime.

These names were formerly module-level in ``runtime.py``. They are imported
back into :mod:`._legacy` (and eventually into the split modules) so any
``from .runtime import PROP_*`` keeps working.
"""

from __future__ import annotations

import math

from mathutils import Matrix, Quaternion


PROP_PACKAGE_ROOT = "starbreaker_package_root"
PROP_SCENE_PATH = "starbreaker_scene_path"
PROP_EXPORT_ROOT = "starbreaker_export_root"
PROP_PACKAGE_NAME = "starbreaker_package_name"
PROP_ENTITY_NAME = "starbreaker_entity_name"
PROP_INSTANCE_JSON = "starbreaker_instance_json"
PROP_MESH_ASSET = "starbreaker_mesh_asset"
PROP_MATERIAL_SIDECAR = "starbreaker_material_sidecar"
PROP_PALETTE_ID = "starbreaker_palette_id"
PROP_PALETTE_SCOPE = "starbreaker_palette_scope"
PROP_SHADER_FAMILY = "starbreaker_shader_family"
PROP_TEMPLATE_KEY = "starbreaker_template_key"
PROP_SUBMATERIAL_JSON = "starbreaker_submaterial_json"
PROP_MATERIAL_IDENTITY = "starbreaker_material_identity"
PROP_IMPORTED_SLOT_MAP = "starbreaker_imported_slot_map"
PROP_TEMPLATE_PATH = "starbreaker_template_path"
# Records the active paint variant's exterior material sidecar on the package root.
# Set when a paint variant with a different material file is applied; used by
# _effective_exterior_material_sidecars() so that subsequent palette changes still
# reach the newly-built materials.
PROP_PAINT_VARIANT_SIDECAR = "starbreaker_paint_variant_sidecar"
PROP_SOURCE_NODE_NAME = "starbreaker_source_node_name"
PROP_MISSING_ASSET = "starbreaker_missing_asset"
PROP_SURFACE_SHADER_MODE = "starbreaker_surface_shader_mode"
SCENE_WEAR_STRENGTH_PROP = "starbreaker_wear_strength"
SURFACE_SHADER_MODE_PRINCIPLED = "principled_first"
SURFACE_SHADER_MODE_GLASS = "glass_bsdf"

PACKAGE_ROOT_PREFIX = "StarBreaker"
TEMPLATE_COLLECTION_NAME = "StarBreaker Template Cache"
GLTF_PBR_WATTS_TO_LUMENS = 683.0
# Luminous efficacy of a broadband white LED (~100-160 lm/W). Used to convert
# Star Citizen light intensity values (authored in lumens) into Blender's
# radiant-flux Watts for Point/Spot/Area lights. See
# ``docs/StarBreaker/lights-research.md``.
LUMENS_PER_WATT_WHITE = 120.0
SCENE_AXIS_CONVERSION = Matrix(
    (
        (1.0, 0.0, 0.0, 0.0),
        (0.0, 0.0, -1.0, 0.0),
        (0.0, 1.0, 0.0, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    )
)
SCENE_AXIS_CONVERSION_INV = SCENE_AXIS_CONVERSION.inverted()
GLTF_LIGHT_BASIS_CORRECTION = Quaternion((math.sqrt(0.5), math.sqrt(0.5), 0.0, 0.0))
NON_COLOR_INPUT_KEYWORDS = ("normal", "roughness", "gloss", "mask", "height", "specular", "opacity", "id_map")
MATERIAL_IDENTITY_SCHEMA = "runtime_material_v10"
