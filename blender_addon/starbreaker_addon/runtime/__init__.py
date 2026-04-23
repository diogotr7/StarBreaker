"""StarBreaker runtime package.

Phase 7 migration complete. The original ``_legacy.py`` monolith has been
decomposed into themed submodules; this ``__init__`` re-exports the
stable public surface consumed by ``ui.py`` and tests.
"""

from __future__ import annotations

from .constants import (
    GLTF_LIGHT_BASIS_CORRECTION,
    GLTF_PBR_WATTS_TO_LUMENS,
    LIGHT_CANDELA_TO_WATT,
    LIGHT_VISUAL_GAIN,
    MATERIAL_IDENTITY_SCHEMA,
    NON_COLOR_INPUT_KEYWORDS,
    PACKAGE_ROOT_PREFIX,
    PROP_ENTITY_NAME,
    PROP_EXPORT_ROOT,
    PROP_IMPORTED_SLOT_MAP,
    PROP_INSTANCE_JSON,
    PROP_MATERIAL_IDENTITY,
    PROP_MATERIAL_SIDECAR,
    PROP_MESH_ASSET,
    PROP_MISSING_ASSET,
    PROP_PACKAGE_NAME,
    PROP_PACKAGE_ROOT,
    PROP_PAINT_VARIANT_SIDECAR,
    PROP_PALETTE_ID,
    PROP_PALETTE_SCOPE,
    PROP_SCENE_PATH,
    PROP_SHADER_FAMILY,
    PROP_SOURCE_NODE_NAME,
    PROP_SUBMATERIAL_JSON,
    PROP_SURFACE_SHADER_MODE,
    PROP_TEMPLATE_KEY,
    PROP_TEMPLATE_PATH,
    SCENE_AXIS_CONVERSION,
    SCENE_AXIS_CONVERSION_INV,
    SCENE_WEAR_STRENGTH_PROP,
    SURFACE_SHADER_MODE_GLASS,
    SURFACE_SHADER_MODE_PRINCIPLED,
    TEMPLATE_COLLECTION_NAME,
)
from .importer import PackageImporter
from .package_ops import (
    apply_livery_to_package_root,
    apply_livery_to_selected_package,
    apply_paint_to_package_root,
    apply_paint_to_selected_package,
    apply_palette_to_package_root,
    apply_palette_to_selected_package,
    dump_selected_metadata,
    exterior_palette_ids,
    find_package_root,
    import_package,
)

