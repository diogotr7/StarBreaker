from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
from typing import Any
import uuid

import bpy
from mathutils import Euler, Matrix, Quaternion

from .manifest import LayerManifestEntry, MaterialSidecar, PackageBundle, PaletteRecord, SceneInstanceRecord, SubmaterialRecord, TextureReference
from .material_contract import ContractInput, ShaderGroupContract, TemplateContract, bundled_template_library_path, load_bundled_template_contract
from .palette import (
    palette_color,
    palette_decal_color,
    palette_decal_texture,
    palette_finish_glossiness,
    palette_finish_specular,
    palette_for_id,
    palette_id_for_livery_instance,
    resolved_palette_id,
)
from .templates import (
    has_virtual_input,
    material_palette_channels,
    representative_textures,
    smoothness_texture_reference,
    template_plan_for_submaterial,
)

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


@dataclass(frozen=True)
class ImportedTemplate:
    mesh_asset: str
    root_names: list[str]


@dataclass(frozen=True)
class MaterialNodeLayout:
    texture_x: float = -300.0
    texture_start_y: float = 160.0
    texture_vertical_step: float = 260.0
    texture_width: float = 300.0
    primary_x: float = 200.0
    primary_y: float = -120.0
    group_width: float = 460.0
    output_x: float = 780.0
    output_y: float = -120.0
    shadow_mix_x: float = 500.0
    shadow_mix_y: float = -120.0
    shadow_transparent_x: float = 260.0
    shadow_transparent_y: float = -300.0
    shadow_light_path_x: float = 260.0
    shadow_light_path_y: float = -480.0


MATERIAL_NODE_LAYOUT = MaterialNodeLayout()


@dataclass
class LayerSurfaceSockets:
    color: Any | None = None
    alpha: Any | None = None
    normal: Any | None = None
    roughness: Any | None = None
    specular: Any | None = None
    specular_tint: Any | None = None
    metallic: Any | None = None


@dataclass
class StencilOverlaySockets:
    color: Any | None = None
    color_factor: Any | None = None
    factor: Any | None = None
    roughness: Any | None = None
    specular: Any | None = None
    specular_tint: Any | None = None


@dataclass(frozen=True)
class SocketRef:
    node: Any
    name: str
    is_output: bool = True


def import_package(
    context: bpy.types.Context,
    scene_path: str | Path,
    prefer_cycles: bool = True,
    palette_id: str | None = None,
) -> bpy.types.Object:
    package = PackageBundle.load(scene_path)
    importer = PackageImporter(context, package)
    with _suspend_heavy_viewports(context):
        root = importer.import_scene(prefer_cycles=prefer_cycles, palette_id=palette_id)
    _purge_orphaned_runtime_groups()
    return root


#: Allowed ``bl_idname`` values for nodes at the top level of a built material.
#:
#: Phase 6 of the blender-exporter plan constrains material top-level node
#: trees to the orchestration layer only: palette groups, image textures,
#: layer / shader helper groups, and the material output. Anything else
#: belongs inside an owned group. See ``docs/StarBreaker/todo.md`` Phase 6
#: for the authoritative definition.
MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES: frozenset[str] = frozenset({
    "ShaderNodeOutputMaterial",
    "ShaderNodeTexImage",
    "ShaderNodeGroup",
})


def _material_top_level_violations(
    material: "bpy.types.Material",
    *,
    extra_allowed: frozenset[str] | None = None,
) -> list[tuple[str, str]]:
    """Return ``[(node_name, bl_idname), ...]`` for nodes that break the Phase 6 rule.

    The rule is documented in :data:`MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES`:
    material top-level trees may only contain palette / layer / shader group
    nodes, image texture nodes, and the material output. Callers that still
    have known-deferred helper types at top level can pass them through
    ``extra_allowed`` to silence those during targeted validation.
    """
    if material is None or material.node_tree is None:
        return []
    allowed = MATERIAL_TOP_LEVEL_ALLOWED_BL_IDNAMES
    if extra_allowed:
        allowed = allowed | extra_allowed
    return [
        (node.name, node.bl_idname)
        for node in material.node_tree.nodes
        if node.bl_idname not in allowed
    ]


def _assert_material_top_level_clean(
    material: "bpy.types.Material",
    *,
    extra_allowed: frozenset[str] | None = None,
) -> None:
    """Raise ``AssertionError`` when ``material``'s top level violates Phase 6.

    Used from the unittest harness and may be called from runtime build paths
    under debug flags. See :func:`_material_top_level_violations` for the rule.
    """
    violations = _material_top_level_violations(material, extra_allowed=extra_allowed)
    if not violations:
        return
    detail = ", ".join(f"{name}:{bl_idname}" for name, bl_idname in violations)
    raise AssertionError(
        f"Material {material.name!r} violates top-level hygiene: {detail}"
    )


def _purge_orphaned_runtime_groups() -> int:
    removed = 0
    for group in list(bpy.data.node_groups):
        if group.users > 0:
            continue
        name = group.name
        if (name.startswith("StarBreaker Runtime LayerSurface.") or
                name.startswith("StarBreaker Runtime HardSurface.") or
                name.startswith("StarBreaker Runtime Glass.") or
                name.startswith("StarBreaker Runtime NoDraw.") or
                name.startswith("StarBreaker Runtime Screen.") or
                name.startswith("StarBreaker Runtime Effect.") or
                name.startswith("StarBreaker Runtime LayeredInputs.") or
                name.startswith("StarBreaker Runtime Principled.") or
                name.startswith("StarBreaker Runtime HardSurface Stencil.") or
                name.startswith("StarBreaker Wear Input.") or
                name.startswith("StarBreaker Iridescence Input.")):
            bpy.data.node_groups.remove(group)
            removed += 1
    return removed


def find_package_root(obj: bpy.types.Object | None) -> bpy.types.Object | None:
    current = obj
    while current is not None:
        if bool(current.get(PROP_PACKAGE_ROOT)):
            return current
        current = current.parent
    return None


def _exterior_material_sidecars(package: PackageBundle) -> set[str] | None:
    """Return the set of material sidecar paths from the exterior livery group.

    The exterior group is the one whose material_sidecars include the root entity's
    sidecar.  Returns None if livery data is absent or unresolvable (caller falls back
    to applying to all materials).
    """
    if not package.liveries:
        return None
    root_sidecar = package.scene.root_entity.material_sidecar
    if not root_sidecar:
        return None
    for livery in package.liveries.values():
        if root_sidecar in livery.material_sidecars:
            return set(livery.material_sidecars)
    return None


def _effective_exterior_material_sidecars(
    package: PackageBundle,
    package_root: bpy.types.Object | None,
) -> set[str] | None:
    """Return the exterior sidecar set, extended with any active paint variant sidecar.

    When a paint variant with a different material file is active, its sidecar is stored
    on the package root object.  This helper ensures that palette-change operations also
    reach materials that were rebuilt from that variant sidecar.
    """
    base = _exterior_material_sidecars(package)
    paint_sidecar = _string_prop(package_root, PROP_PAINT_VARIANT_SIDECAR) if package_root is not None else None
    if paint_sidecar is None:
        return base
    if base is None:
        return {paint_sidecar}
    return base | {paint_sidecar}


def exterior_palette_ids(package: PackageBundle) -> list[str]:
    """Return palette IDs applicable to the exterior livery group.

    Includes both palette-based IDs (from palettes.json) and paint-variant IDs
    (from paints.json), minus any IDs that are interior-only.
    """
    all_ids = set(package.palettes.keys()) | set(package.paints.keys())
    if not all_ids:
        return []
    if not package.liveries:
        return sorted(all_ids)
    exterior_sidecars = _exterior_material_sidecars(package)
    if exterior_sidecars is None:
        return sorted(all_ids)
    interior_only_palette_ids: set[str] = set()
    for livery in package.liveries.values():
        if not set(livery.material_sidecars).intersection(exterior_sidecars):
            if livery.palette_id:
                interior_only_palette_ids.add(livery.palette_id)
    return sorted(pid for pid in all_ids if pid not in interior_only_palette_ids)


def _paint_variant_for_palette_id(package: PackageBundle, palette_id: str | None) -> Any | None:
    if not palette_id:
        return None
    direct = package.paints.get(palette_id)
    if direct is not None:
        return direct
    canonical_id = resolved_palette_id(package, palette_id)
    if canonical_id is None:
        return None
    for candidate_id, variant in package.paints.items():
        if resolved_palette_id(package, candidate_id) == canonical_id:
            return variant
    return None


def apply_palette_to_selected_package(context: bpy.types.Context, palette_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_palette_to_package_root(context, package_root, palette_id)


def apply_paint_to_selected_package(context: bpy.types.Context, palette_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_paint_to_package_root(context, package_root, palette_id)


def apply_livery_to_selected_package(context: bpy.types.Context, livery_id: str) -> int:
    package_root = find_package_root(context.active_object)
    if package_root is None:
        raise RuntimeError("Select an imported StarBreaker object first")
    return apply_livery_to_package_root(context, package_root, livery_id)


def dump_selected_metadata(context: bpy.types.Context) -> list[str]:
    obj = context.active_object
    if obj is None:
        raise RuntimeError("Select an imported StarBreaker object first")

    text_names: list[str] = []
    instance_json = obj.get(PROP_INSTANCE_JSON)
    if isinstance(instance_json, str):
        text = bpy.data.texts.new(f"starbreaker_instance_{obj.name}.json")
        text.from_string(json.dumps(json.loads(instance_json), indent=2, sort_keys=True))
        text_names.append(text.name)

    material = obj.active_material
    if material is not None:
        submaterial_json = material.get(PROP_SUBMATERIAL_JSON)
        if isinstance(submaterial_json, str):
            text = bpy.data.texts.new(f"starbreaker_material_{material.name}.json")
            text.from_string(json.dumps(json.loads(submaterial_json), indent=2, sort_keys=True))
            text_names.append(text.name)

    return text_names


def apply_palette_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, palette_id: str) -> int:
    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root)
    with _suspend_heavy_viewports(context):
        return importer.apply_palette_to_package_root(package_root, palette_id)


def apply_paint_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, palette_id: str) -> int:
    """Switch to the paint variant whose palette_id matches, rebuilding exterior materials
    from the variant's material sidecar when it differs from the current one.

    Falls back to a fast palette-only update when no matching paint variant is found
    or when the variant does not carry a different material sidecar.
    """
    package = _load_package_from_root(package_root)
    variant = package.paints.get(palette_id)
    target_sidecar = variant.exterior_material_sidecar if variant is not None else None

    if target_sidecar is None:
        # No paint-variant sidecar for this palette: fast palette-only path.
        return apply_palette_to_package_root(context, package_root, palette_id)

    # Determine which objects are currently exterior so we know what to rebuild.
    # We check against both the original livery sidecars AND any previously-active
    # paint variant sidecar so that consecutive paint switches work correctly.
    effective_exterior = _effective_exterior_material_sidecars(package, package_root)
    base_exterior = _exterior_material_sidecars(package)
    check_sidecars = effective_exterior or base_exterior

    importer = PackageImporter(context, package, package_root=package_root)
    applied = 0
    with _suspend_heavy_viewports(context):
        for obj in _iter_package_objects(package_root):
            if obj.type != "MESH":
                continue
            obj_sidecar = _string_prop(obj, PROP_MATERIAL_SIDECAR)
            if check_sidecars is not None and (obj_sidecar is None or obj_sidecar not in check_sidecars):
                continue
            # Point the object at the new sidecar then rebuild.
            obj[PROP_MATERIAL_SIDECAR] = target_sidecar
            applied += importer.rebuild_object_materials(obj, palette_id)

    # Record the active paint variant sidecar so palette-only changes still work.
    package_root[PROP_PAINT_VARIANT_SIDECAR] = target_sidecar
    package_root[PROP_PALETTE_ID] = palette_id
    return applied


def apply_livery_to_package_root(context: bpy.types.Context, package_root: bpy.types.Object, livery_id: str) -> int:
    package = _load_package_from_root(package_root)
    importer = PackageImporter(context, package, package_root=package_root)
    applied = 0
    with _suspend_heavy_viewports(context):
        for obj in _iter_package_objects(package_root):
            instance = _scene_instance_from_object(obj)
            if instance is None:
                continue
            effective_palette_id = palette_id_for_livery_instance(
                package,
                livery_id,
                instance,
                _string_prop(obj, PROP_MATERIAL_SIDECAR),
            )
            applied += importer.rebuild_object_materials(obj, effective_palette_id)
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
        root_palette_id = palette_id_for_livery_instance(
            package,
            livery_id,
            package.scene.root_entity,
            package.scene.root_entity.material_sidecar,
        )
        package_root[PROP_PALETTE_ID] = resolved_palette_id(
            package,
            root_palette_id,
            package.scene.root_entity.palette_id,
        ) or ""
    _purge_orphaned_runtime_groups()
    return applied


@contextmanager
def _suspend_heavy_viewports(context: bpy.types.Context):
    window_manager = getattr(context, "window_manager", None)
    if window_manager is None:
        yield
        return

    suspended: list[tuple[Any, str]] = []
    try:
        for window in window_manager.windows:
            screen = getattr(window, "screen", None)
            if screen is None:
                continue
            for area in screen.areas:
                if area.type != "VIEW_3D":
                    continue
                space = area.spaces.active
                shading = getattr(space, "shading", None)
                shading_type = getattr(shading, "type", None)
                if shading is None or shading_type not in {"RENDERED", "MATERIAL"}:
                    continue
                suspended.append((shading, shading_type))
                shading.type = "SOLID"
        yield
    finally:
        for shading, shading_type in suspended:
            try:
                shading.type = shading_type
            except Exception:
                continue


class PackageImporter:
    def __init__(
        self,
        context: bpy.types.Context,
        package: PackageBundle,
        package_root: bpy.types.Object | None = None,
    ) -> None:
        self.context = context
        self.package = package
        self.collection = self._ensure_collection(package.package_name)
        self.template_collection = self._ensure_template_collection()
        self.package_root = package_root
        self.exterior_material_sidecars = _exterior_material_sidecars(package)
        self.template_cache: dict[str, ImportedTemplate] = {}
        self.material_cache: dict[str, bpy.types.Material] = {}
        self.node_index_by_entity_name: dict[str, dict[str, bpy.types.Object]] = {}
        self.bundled_template_contract: TemplateContract | None = None
        self.import_palette_override: str | None = None
        self.import_paint_variant_sidecar: str | None = None
        self.runtime_shared_groups_ready = False
        self.material_identity_index: dict[str, bpy.types.Material] = {}
        self.material_identity_index_ready = False
        self.sidecar_submaterials_by_index: dict[str, dict[int, SubmaterialRecord]] = {}
        self.sidecar_submaterials_by_name: dict[str, dict[str, SubmaterialRecord]] = {}
        self.slot_mapping_cache: dict[int, list[int | None] | None] = {}

    def _ensure_runtime_shared_groups(self) -> None:
        if self.runtime_shared_groups_ready:
            return
        self._ensure_runtime_layer_surface_group()
        self._ensure_runtime_hard_surface_group()
        self._ensure_runtime_illum_group()
        self._ensure_runtime_wear_input_group()
        self._ensure_runtime_iridescence_input_group()
        self._ensure_runtime_nodraw_group()
        self._ensure_runtime_glass_group()
        self._ensure_runtime_screen_group()
        self._ensure_runtime_effect_group()
        self._ensure_runtime_layered_inputs_group()
        self._ensure_runtime_principled_group()
        self._ensure_runtime_hardsurface_stencil_group()
        self.runtime_shared_groups_ready = True

    def _ensure_material_identity_index(self) -> None:
        if self.material_identity_index_ready:
            return
        for material in bpy.data.materials:
            material_identity = material.get(PROP_MATERIAL_IDENTITY)
            if isinstance(material_identity, str) and material_identity:
                self.material_identity_index[material_identity] = material
        self.material_identity_index_ready = True

    def _submaterials_by_index(self, sidecar_path: str, sidecar: MaterialSidecar) -> dict[int, SubmaterialRecord]:
        canonical_path = _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_index.get(canonical_path)
        if cached is not None:
            return cached
        indexed = {submaterial.index: submaterial for submaterial in sidecar.submaterials}
        self.sidecar_submaterials_by_index[canonical_path] = indexed
        return indexed

    def _submaterials_by_unique_name(self, sidecar_path: str, sidecar: MaterialSidecar) -> dict[str, SubmaterialRecord]:
        canonical_path = _canonical_material_sidecar_path(sidecar_path, sidecar)
        cached = self.sidecar_submaterials_by_name.get(canonical_path)
        if cached is not None:
            return cached
        indexed = _unique_submaterials_by_name(sidecar)
        self.sidecar_submaterials_by_name[canonical_path] = indexed
        return indexed

    def _effective_palette_id(self, palette_id: str | None) -> str | None:
        inherited_palette_id = None
        if self.package_root is not None:
            inherited_palette_id = _string_prop(self.package_root, PROP_PALETTE_ID)
        return resolved_palette_id(
            self.package,
            self.import_palette_override or palette_id,
            inherited_palette_id or self.package.scene.root_entity.palette_id,
        )

    def import_scene(self, prefer_cycles: bool = True, palette_id: str | None = None) -> bpy.types.Object:
        if prefer_cycles and hasattr(self.context.scene.render, "engine"):
            self.context.scene.render.engine = "CYCLES"
            self._ensure_cycles_denoising_support()

        self._ensure_runtime_shared_groups()

        initial_palette_id = resolved_palette_id(
            self.package,
            palette_id,
            self.package.scene.root_entity.palette_id,
        )
        initial_paint_variant = _paint_variant_for_palette_id(self.package, palette_id)
        self.import_palette_override = initial_palette_id
        self.import_paint_variant_sidecar = (
            initial_paint_variant.exterior_material_sidecar
            if initial_paint_variant is not None
            else None
        )
        package_root = self.package_root or self._create_package_root(initial_palette_id)
        self.package_root = package_root
        if initial_palette_id is not None:
            package_root[PROP_PALETTE_ID] = initial_palette_id
        if self.import_paint_variant_sidecar is not None:
            package_root[PROP_PAINT_VARIANT_SIDECAR] = self.import_paint_variant_sidecar

        root_anchor, root_nodes = self.instantiate_scene_instance(self.package.scene.root_entity, parent=package_root)
        self.node_index_by_entity_name[self.package.scene.root_entity.entity_name] = self._index_nodes(root_nodes)
        root_anchor.parent = package_root
        scene_root_parent = self._scene_root_parent(root_nodes) or package_root

        for child in self.package.scene.children:
            parent_node = None
            if child.parent_entity_name:
                parent_node = self.node_index_by_entity_name.get(child.parent_entity_name, {}).get(child.parent_node_name or "")
            anchor, child_nodes = self.instantiate_scene_instance(child, parent=scene_root_parent, parent_node=parent_node)
            self.node_index_by_entity_name.setdefault(child.entity_name, {}).update(self._index_nodes(child_nodes))

        for interior in self.package.scene.interiors:
            self.import_interior_container(interior, scene_root_parent)

        return package_root

    def _effective_import_material_sidecar(self, sidecar_path: str | None) -> str | None:
        if sidecar_path is None:
            return None
        if self.import_paint_variant_sidecar is None:
            return sidecar_path
        if self.exterior_material_sidecars is None:
            return self.import_paint_variant_sidecar
        if sidecar_path in self.exterior_material_sidecars:
            return self.import_paint_variant_sidecar
        return sidecar_path

    def rebuild_object_materials(self, obj: bpy.types.Object, palette_id: str | None) -> int:
        self._ensure_runtime_shared_groups()
        if obj.type != "MESH":
            return 0
        sidecar_path = _string_prop(obj, PROP_MATERIAL_SIDECAR)
        if sidecar_path is None:
            return 0
        sidecar = self.package.load_material_sidecar(sidecar_path)
        if sidecar is None:
            return 0
        effective_palette_id = self._effective_palette_id(palette_id)
        palette = palette_for_id(self.package, effective_palette_id)
        applied = 0
        mesh_materials = getattr(obj.data, "materials", None)
        data = getattr(obj, "data", None)
        data_pointer = data.as_pointer() if data is not None else 0
        slot_mapping = self.slot_mapping_cache.get(data_pointer)
        if data_pointer not in self.slot_mapping_cache:
            slot_mapping = _slot_mapping_for_object(obj)
            self.slot_mapping_cache[data_pointer] = slot_mapping
        if slot_mapping is not None:
            if mesh_materials is not None:
                while len(mesh_materials) < len(slot_mapping):
                    mesh_materials.append(None)
            source_sidecar_path = _slot_mapping_source_sidecar_path(obj, sidecar_path)
            source_sidecar = self.package.load_material_sidecar(source_sidecar_path)
            if source_sidecar is None:
                source_sidecar = sidecar
            source_submaterials_by_index = self._submaterials_by_index(source_sidecar_path, source_sidecar)
            target_submaterials_by_index = self._submaterials_by_index(sidecar_path, sidecar)
            target_submaterials_by_name = self._submaterials_by_unique_name(sidecar_path, sidecar)
            for slot_index, mapped_index in enumerate(slot_mapping):
                fallback_index = mapped_index if mapped_index is not None else slot_index
                source_submaterial = source_submaterials_by_index.get(fallback_index)
                submaterial = _remapped_submaterial_for_slot(
                    source_submaterial,
                    fallback_index,
                    target_submaterials_by_index,
                    target_submaterials_by_name,
                )
                if submaterial is None:
                    print(
                        f"StarBreaker: missing sidecar submaterial index {mapped_index} for {obj.name}"
                    )
                    continue
                if slot_index >= len(obj.material_slots):
                    print(
                        f"StarBreaker: slot index {slot_index} exceeds material slot count for {obj.name}"
                    )
                    continue
                material = self.material_for_submaterial(sidecar_path, sidecar, submaterial, palette)
                slot = obj.material_slots[slot_index]
                slot.link = "OBJECT"
                slot.material = material
                applied += 1
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
            return applied
        for submaterial in sorted(sidecar.submaterials, key=lambda item: item.index):
            if mesh_materials is not None:
                while len(mesh_materials) <= submaterial.index:
                    mesh_materials.append(None)
            if submaterial.index >= len(obj.material_slots):
                print(
                    f"StarBreaker: submaterial index {submaterial.index} exceeds material slot count for {obj.name}"
                )
                continue
            material = self.material_for_submaterial(sidecar_path, sidecar, submaterial, palette)
            slot = obj.material_slots[submaterial.index]
            slot.link = "OBJECT"
            slot.material = material
            applied += 1
        if effective_palette_id is not None:
            obj[PROP_PALETTE_ID] = effective_palette_id
        return applied

    def apply_palette_to_package_root(self, package_root: bpy.types.Object, palette_id: str | None) -> int:
        effective_palette_id = self._effective_palette_id(palette_id)
        palette = palette_for_id(self.package, effective_palette_id)
        if palette is None:
            return 0

        self._ensure_runtime_shared_groups()
        self.package_root = package_root
        palette_group = self._ensure_palette_group(palette)
        if effective_palette_id is not None:
            package_root[PROP_PALETTE_ID] = effective_palette_id

        allowed_sidecars = _effective_exterior_material_sidecars(self.package, package_root)

        for material in bpy.data.materials:
            if material.node_tree is None:
                continue
            if not material.get(PROP_SUBMATERIAL_JSON):
                continue
            if allowed_sidecars is not None:
                mat_sidecar = _string_prop(material, PROP_MATERIAL_SIDECAR)
                if mat_sidecar is not None and mat_sidecar not in allowed_sidecars:
                    continue
            has_palette_node = any(
                n.bl_idname == "ShaderNodeGroup"
                and getattr(getattr(n, "node_tree", None), "name", "").startswith("StarBreaker Palette ")
                for n in material.node_tree.nodes
            )
            if has_palette_node:
                self._apply_palette_to_material(material, palette, palette_group)

        self.context.view_layer.update()
        return 0

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
            submaterial = SubmaterialRecord.from_value(json.loads(payload))
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
            current_source = palette_color_input.links[0].from_socket.name if palette_color_input.is_linked else None
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
        channel_name = group_node.get("starbreaker_palette_finish_channel")
        if not isinstance(channel_name, str) or channel_name not in {"primary", "secondary", "tertiary", "glass"}:
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
            (node for node in node_tree.nodes if node.bl_idname == "ShaderNodeRGB" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"),
            None,
        )
        alpha_node = next(
            (node for node in node_tree.nodes if node.bl_idname == "ShaderNodeValue" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"),
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

    def instantiate_scene_instance(
        self,
        record: SceneInstanceRecord,
        parent: bpy.types.Object,
        parent_node: bpy.types.Object | None = None,
    ) -> tuple[bpy.types.Object, list[bpy.types.Object]]:
        effective_palette_id = self._effective_palette_id(record.palette_id)
        anchor = bpy.data.objects.new(record.entity_name, None)
        anchor.empty_display_type = "PLAIN_AXES"
        self.collection.objects.link(anchor)

        target_parent = parent_node or parent
        anchor.parent = target_parent
        anchor.rotation_mode = "QUATERNION"
        anchor.location = _scene_position_to_blender(record.offset_position)
        desired_rotation = Euler(tuple(math.radians(value) for value in record.offset_rotation), "XYZ").to_quaternion()
        if parent_node is not None and record.no_rotation:
            anchor.rotation_quaternion = parent_node.matrix_world.to_quaternion().inverted() @ desired_rotation
        else:
            anchor.rotation_quaternion = desired_rotation

        try:
            template = self.ensure_template(record.mesh_asset)
        except RuntimeError:
            anchor.empty_display_type = "SPHERE"
            if record.mesh_asset is not None:
                anchor[PROP_MISSING_ASSET] = record.mesh_asset
            self._apply_instance_metadata([anchor], record, effective_palette_id)
            return anchor, [anchor]

        clones = self.instantiate_template(template, anchor, neutralize_axis_root=parent_node is not None)
        self._apply_instance_metadata([anchor, *clones], record, effective_palette_id)

        for clone in clones:
            self.rebuild_object_materials(clone, effective_palette_id)
        return anchor, clones

    def import_interior_container(self, interior: Any, package_root: bpy.types.Object) -> bpy.types.Object:
        anchor_name = interior.name if interior.name.startswith("interior_") else f"interior_{interior.name}"
        anchor = bpy.data.objects.new(anchor_name, None)
        anchor.empty_display_type = "CUBE"
        anchor.parent = package_root
        anchor.matrix_local = _scene_matrix_to_blender(interior.container_transform)
        self.collection.objects.link(anchor)

        for placement in interior.placements:
            instance = SceneInstanceRecord(
                entity_name=placement.entity_class_guid or Path(placement.cgf_path or "interior").stem,
                geometry_path=placement.cgf_path,
                material_path=placement.material_path,
                material_sidecar=placement.material_sidecar,
                mesh_asset=placement.mesh_asset,
                palette_id=interior.palette_id,
                raw=placement.raw,
            )
            effective_palette_id = self._effective_palette_id(instance.palette_id)
            placement_anchor = bpy.data.objects.new(instance.entity_name, None)
            placement_anchor.parent = anchor
            placement_anchor.matrix_local = _scene_matrix_to_blender(placement.transform)
            self.collection.objects.link(placement_anchor)

            try:
                template = self.ensure_template(instance.mesh_asset)
            except RuntimeError:
                placement_anchor.empty_display_type = "SPHERE"
                if instance.mesh_asset is not None:
                    placement_anchor[PROP_MISSING_ASSET] = instance.mesh_asset
                self._apply_instance_metadata([placement_anchor], instance, effective_palette_id)
                continue

            clones = self.instantiate_template(
                template,
                placement_anchor,
                neutralize_axis_root=True,
                force_neutralize_axis_root=True,
            )
            self._apply_instance_metadata([placement_anchor, *clones], instance, effective_palette_id)
            for clone in clones:
                self.rebuild_object_materials(clone, effective_palette_id)

        for light in interior.lights:
            self.create_light(light, anchor)

        return anchor

    def create_light(self, light: Any, parent: bpy.types.Object) -> bpy.types.Object:
        blender_light_type = _blender_light_type(light)
        light_data = bpy.data.lights.new(name=light.name or "StarBreaker Light", type=blender_light_type)
        light_data.energy = _light_energy_to_blender(light.intensity, blender_light_type)
        light_data.color = light.color
        if blender_light_type != "SUN" and hasattr(light_data, "cutoff_distance"):
            light_data.cutoff_distance = light.radius
        if blender_light_type == "SPOT" and hasattr(light_data, "spot_size"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            light_data.spot_size = math.radians(outer_angle) * 2.0
        if blender_light_type == "SPOT" and hasattr(light_data, "spot_blend"):
            outer_angle = max(light.outer_angle or 45.0, 0.01)
            inner_angle = min(light.inner_angle or 0.0, outer_angle)
            inner_ratio = min(max(inner_angle / outer_angle, 0.0), 1.0)
            light_data.spot_blend = 1.0 - inner_ratio

        light_object = bpy.data.objects.new(light.name or "StarBreaker Light", light_data)
        light_object.parent = parent
        light_object.location = _scene_position_to_blender(light.position)
        light_object.rotation_mode = "QUATERNION"
        light_object.rotation_quaternion = _scene_light_quaternion_to_blender(light.rotation)
        self.collection.objects.link(light_object)
        return light_object

    def ensure_template(self, mesh_asset: str | None) -> ImportedTemplate:
        if not mesh_asset:
            raise RuntimeError("Scene instance is missing mesh_asset")

        asset_path = self.package.resolve_path(mesh_asset)
        if asset_path is None or not asset_path.is_file():
            raise RuntimeError(f"Missing mesh asset: {mesh_asset}")
        asset_key = str(asset_path.resolve())

        cached = self.template_cache.get(asset_key)
        if cached is not None:
            return cached

        before = {obj.as_pointer() for obj in bpy.data.objects}
        result = bpy.ops.import_scene.gltf(filepath=str(asset_path), import_pack_images=False, merge_vertices=False)
        if "FINISHED" not in result:
            raise RuntimeError(f"Failed to import {asset_path}")

        imported = [obj for obj in bpy.data.objects if obj.as_pointer() not in before]
        imported_materials_by_pointer: dict[int, bpy.types.Material] = {}
        for obj in imported:
            for slot in getattr(obj, "material_slots", []):
                material = getattr(slot, "material", None)
                if material is not None:
                    imported_materials_by_pointer[material.as_pointer()] = material
        imported_materials = list(imported_materials_by_pointer.values())
        root_objects = self._root_objects(imported)
        for obj in imported:
            for collection in list(obj.users_collection):
                collection.objects.unlink(obj)
            self.template_collection.objects.link(obj)
            obj.hide_set(True)
            obj.hide_render = True
            obj[PROP_TEMPLATE_PATH] = mesh_asset
            obj[PROP_SOURCE_NODE_NAME] = _canonical_source_name(obj.name)

        self._clear_template_material_bindings(imported)
        self._purge_unused_materials(imported_materials)

        template = ImportedTemplate(mesh_asset=mesh_asset, root_names=[obj.name for obj in root_objects])
        self.template_cache[asset_key] = template
        return template

    def instantiate_template(
        self,
        template: ImportedTemplate,
        anchor: bpy.types.Object,
        neutralize_axis_root: bool = False,
        force_neutralize_axis_root: bool = False,
    ) -> list[bpy.types.Object]:
        clones: list[bpy.types.Object] = []
        mapping: dict[str, bpy.types.Object] = {}
        needs_view_layer_update = False
        for root_name in template.root_names:
            source = bpy.data.objects.get(root_name)
            if source is None:
                continue
            neutralize_root = neutralize_axis_root and (
                force_neutralize_axis_root or _should_neutralize_axis_root(source, template.mesh_asset)
            )
            clone = self._duplicate_object_tree(source, template.mesh_asset, mapping)
            clone.parent = anchor
            if neutralize_root:
                clone.matrix_local = Matrix.Identity(4)
                needs_view_layer_update = True
            clones.append(clone)
        if needs_view_layer_update:
            self.context.view_layer.update()
        return list(mapping.values()) or clones

    def _duplicate_object_tree(
        self,
        source: bpy.types.Object,
        mesh_asset: str,
        mapping: dict[str, bpy.types.Object],
    ) -> bpy.types.Object:
        clone = source.copy()
        if source.data is not None:
            clone.data = source.data
        clone.animation_data_clear()
        clone.hide_set(False)
        clone.hide_render = False
        clone[PROP_TEMPLATE_PATH] = mesh_asset
        clone[PROP_SOURCE_NODE_NAME] = source.get(PROP_SOURCE_NODE_NAME, source.name)
        self.collection.objects.link(clone)
        clone.matrix_basis = source.matrix_basis.copy()
        mapping[source.name] = clone

        for child in source.children:
            if child.get(PROP_TEMPLATE_PATH) != mesh_asset:
                continue
            child_clone = self._duplicate_object_tree(child, mesh_asset, mapping)
            child_clone.parent = clone
            child_clone.matrix_parent_inverse = child.matrix_parent_inverse.copy()
        return clone

    def material_for_submaterial(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
    ) -> bpy.types.Material:
        palette_scope = self._palette_scope()
        cache_key = _material_identity(sidecar_path, sidecar, submaterial, palette, palette_scope)
        cached = self.material_cache.get(cache_key)
        if cached is not None:
            return cached

        reusable = self._reusable_material(sidecar_path, sidecar, submaterial, palette, palette_scope, cache_key)
        if reusable is not None:
            existing_identity = reusable.get(PROP_MATERIAL_IDENTITY)
            if isinstance(existing_identity, str) and existing_identity == cache_key:
                self.material_cache[cache_key] = reusable
                self.material_identity_index[cache_key] = reusable
                return reusable
            self._build_managed_material(reusable, sidecar_path, sidecar, submaterial, palette, cache_key)
            self.material_cache[cache_key] = reusable
            self.material_identity_index[cache_key] = reusable
            return reusable

        material_name = _material_name(sidecar_path, sidecar, submaterial, cache_key)
        material = bpy.data.materials.new(material_name)
        self._build_managed_material(material, sidecar_path, sidecar, submaterial, palette, cache_key)
        self.material_cache[cache_key] = material
        self.material_identity_index[cache_key] = material
        return material

    def _reusable_material(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        palette_scope: str,
        material_identity: str,
    ) -> bpy.types.Material | None:
        preferred_name = submaterial.blender_material_name or _derived_material_name(sidecar_path, sidecar, submaterial)
        preferred = bpy.data.materials.get(preferred_name)
        if preferred is not None and _material_is_compatible(
            preferred,
            self.package,
            sidecar_path,
            sidecar,
            submaterial,
            palette,
            palette_scope,
        ):
            return preferred

        self._ensure_material_identity_index()
        indexed_material = self.material_identity_index.get(material_identity)
        if indexed_material is not None and _material_is_compatible(
            indexed_material,
            self.package,
            sidecar_path,
            sidecar,
            submaterial,
            palette,
            palette_scope,
        ):
            return indexed_material
        return None

    def _build_managed_material(
        self,
        material: bpy.types.Material,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        material_identity: str,
    ) -> None:
        palette_key = palette.id if palette is not None else "none"
        material.use_nodes = True
        plan = template_plan_for_submaterial(submaterial)
        surface_mode = SURFACE_SHADER_MODE_PRINCIPLED
        if submaterial.shader_family == "HardSurface":
            self._build_hard_surface_material(material, submaterial, palette, plan)
        elif submaterial.shader_family == "Illum":
            self._build_illum_material(material, submaterial, palette, plan)
        else:
            group_contract = None if plan.template_key == "layered_wear" else self._group_contract_for_submaterial(submaterial)
            if group_contract is not None and self._build_contract_group_material(material, submaterial, palette, plan, group_contract):
                if submaterial.shader_family == "GlassPBR":
                    surface_mode = SURFACE_SHADER_MODE_GLASS
            elif submaterial.shader_family == "GlassPBR":
                self._build_glass_material(material, submaterial, palette, plan)
                surface_mode = SURFACE_SHADER_MODE_GLASS
            elif plan.template_key == "nodraw":
                self._build_nodraw_material(material)
            elif plan.template_key == "screen_hud":
                self._build_screen_material(material, submaterial, palette, plan)
            elif plan.template_key == "effects":
                self._build_effect_material(material, submaterial, palette, plan)
            else:
                self._build_principled_material(material, submaterial, palette, plan)

        self._apply_material_node_layout(material)

        material[PROP_SHADER_FAMILY] = submaterial.shader_family
        material[PROP_TEMPLATE_KEY] = plan.template_key
        material[PROP_PALETTE_ID] = palette_key
        material[PROP_PALETTE_SCOPE] = self._palette_scope()
        material[PROP_MATERIAL_SIDECAR] = _canonical_material_sidecar_path(sidecar_path, sidecar)
        material[PROP_MATERIAL_IDENTITY] = material_identity
        material[PROP_SUBMATERIAL_JSON] = json.dumps(submaterial.raw, sort_keys=True)
        material[PROP_SURFACE_SHADER_MODE] = surface_mode

    def _palette_scope(self) -> str:
        package_root = self.package_root
        if package_root is None:
            return _safe_identifier(self.package.package_name)
        palette_scope = _string_prop(package_root, PROP_PALETTE_SCOPE)
        if palette_scope:
            return palette_scope
        palette_scope = uuid.uuid4().hex
        package_root[PROP_PALETTE_SCOPE] = palette_scope
        return palette_scope

    def _template_contract(self) -> TemplateContract:
        if self.bundled_template_contract is None:
            self.bundled_template_contract = load_bundled_template_contract()
        return self.bundled_template_contract

    def _group_contract_for_submaterial(self, submaterial: SubmaterialRecord) -> ShaderGroupContract | None:
        return self._template_contract().group_for_shader_family(submaterial.shader_family)

    def _ensure_contract_group(self, group_contract: ShaderGroupContract) -> bpy.types.ShaderNodeTree | None:
        group = bpy.data.node_groups.get(group_contract.name)
        if group is not None:
            return group
        library_path = bundled_template_library_path()
        if not library_path.is_file():
            return None
        with bpy.data.libraries.load(str(library_path), link=False) as (data_from, data_to):
            if group_contract.name not in data_from.node_groups:
                return None
            data_to.node_groups = [group_contract.name]
        return bpy.data.node_groups.get(group_contract.name)

    def _build_contract_group_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
        group_contract: ShaderGroupContract,
    ) -> bool:
        group_tree = self._ensure_contract_group(group_contract)
        if group_tree is None:
            return False

        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = group_tree
        group_node.location = (220, 0)

        shader_output = _output_socket(group_node, group_contract.shader_output)
        if shader_output is None:
            return False
        surface_shader = shader_output

        y = 280
        for contract_input in group_contract.inputs:
            target_socket = _input_socket(group_node, contract_input.name)
            if target_socket is None:
                continue
            semantic = (contract_input.semantic or contract_input.name).lower()
            if "disable" in semantic and "shadow" in semantic:
                if hasattr(target_socket, "default_value"):
                    target_socket.default_value = bool(self._plan_casts_no_shadows(plan, submaterial))
                source_socket = None
            elif semantic == "emission_strength" and hasattr(target_socket, "default_value"):
                target_socket.default_value = self._illum_emission_strength(submaterial)
                source_socket = None
            else:
                if (
                    group_contract.name == "SB_HardSurface_v1"
                    and semantic == "base_color"
                    and hasattr(target_socket, "default_value")
                ):
                    target_socket.default_value = (1.0, 1.0, 1.0, 1.0)
                elif (
                    group_contract.name == "SB_HardSurface_v1"
                    and semantic == "base_color_alpha"
                    and hasattr(target_socket, "default_value")
                ):
                    target_socket.default_value = 1.0
                elif ("alpha" in semantic or "opacity" in semantic) and hasattr(target_socket, "default_value"):
                    target_socket.default_value = 0.0
                source_socket = self._contract_input_source_socket(
                    nodes,
                    submaterial,
                    palette,
                    group_contract,
                    contract_input,
                    x=-220,
                    y=y,
                )
            if source_socket is not None:
                links.new(source_socket, target_socket)
            elif "normal" in semantic and hasattr(target_socket, "default_value"):
                target_socket.default_value = (0.5, 0.5, 1.0, 1.0)
            y -= 180

        group_handles_alpha = any(
            (contract_input.semantic or contract_input.name).lower() in {"alpha", "opacity"}
            or "alpha" in (contract_input.semantic or contract_input.name).lower()
            or "opacity" in (contract_input.semantic or contract_input.name).lower()
            for contract_input in group_contract.inputs
        )

        if plan.uses_alpha and not group_handles_alpha:
            alpha_source = self._alpha_source_socket(
                nodes,
                submaterial,
                representative_textures(submaterial),
                x=-220,
                y=y,
            )
            if alpha_source is not None:
                transparent = nodes.new("ShaderNodeBsdfTransparent")
                transparent.location = (400, -180)
                mix = nodes.new("ShaderNodeMixShader")
                mix.location = (560, 0)
                links.new(alpha_source, mix.inputs[0])
                links.new(transparent.outputs[0], mix.inputs[1])
                links.new(surface_shader, mix.inputs[2])
                surface_shader = mix.outputs[0]

        links.new(surface_shader, output.inputs[0])

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)
        return True

    def _build_hard_surface_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)

        top_base = _submaterial_texture_reference(submaterial, slots=("TexSlot1",), roles=("base_color", "diffuse"))
        top_base_node = self._image_node(nodes, top_base.export_path if top_base is not None else None, x=-720, y=520, is_color=True)
        top_base_color = top_base_node.outputs[0] if top_base_node is not None else None
        top_base_alpha = _output_socket(top_base_node, "Alpha") if top_base_node is not None else None
        material_channel = submaterial.palette_routing.material_channel.name if submaterial.palette_routing.material_channel is not None else None
        angle_shift_enabled = _hard_surface_angle_shift_enabled(submaterial) or (
            material_channel == "tertiary" and _palette_has_iridescence(palette)
        )

        primary_layer = submaterial.layer_manifest[0] if submaterial.layer_manifest else None
        secondary_layer = submaterial.layer_manifest[1] if len(submaterial.layer_manifest) > 1 else None
        primary = self._connect_manifest_layer_surface_group(
            nodes,
            links,
            submaterial,
            primary_layer,
            palette,
            x=-240,
            y=240,
            label="Primary Layer",
            detail_slots=("TexSlot7", "TexSlot13", "TexSlot6"),
        )
        secondary = self._connect_manifest_layer_surface_group(
            nodes,
            links,
            submaterial,
            secondary_layer,
            palette,
            x=-240,
            y=-120,
            label="Secondary Layer",
            detail_slots=("TexSlot7", "TexSlot13", "TexSlot6"),
        )
        wear_factor = self._layered_wear_factor_socket(nodes, links, submaterial, x=-720, y=-120)
        damage_factor = self._layered_damage_factor_socket(nodes, links, submaterial, x=-720, y=-240)
        iridescence_ramp_color = self._iridescence_ramp_color_socket(nodes, links, submaterial, x=-980, y=-1560)
        stencil = self._hard_surface_stencil_overlay_sockets(nodes, links, submaterial, x=-980, y=-1820)

        macro_normal_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot3",), roles=("normal_gloss",))
        macro_normal_node = self._image_node(
            nodes,
            macro_normal_ref.export_path if macro_normal_ref is not None else None,
            x=-720,
            y=-420,
            is_color=False,
        )
        displacement_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot6",), roles=("height", "displacement"))
        displacement_node = self._image_node(
            nodes,
            displacement_ref.export_path if displacement_ref is not None else None,
            x=-720,
            y=-720,
            is_color=False,
        )
        emissive_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot14",), roles=("emissive",))
        emissive_node = self._image_node(
            nodes,
            emissive_ref.export_path if emissive_ref is not None else None,
            x=-720,
            y=-1020,
            is_color=True,
        )

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_hard_surface_group()
        _refresh_group_node_sockets(shader_group)
        shader_group.location = (140, 0)
        shader_group.label = "StarBreaker HardSurface"
        self._set_socket_default(_input_socket(shader_group, "Top Base Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Top Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Primary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Roughness"), 0.45)
        self._set_socket_default(_input_socket(shader_group, "Primary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Specular Tint"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Primary Metallic"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Roughness"), 0.45)
        self._set_socket_default(_input_socket(shader_group, "Secondary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Specular Tint"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Metallic"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Normal"), (0.0, 0.0, 1.0))
        if angle_shift_enabled and palette is not None:
            facing_socket = self._palette_color_socket(nodes, palette, "tertiary", x=-720, y=-1320)
            grazing_socket = self._palette_specular_socket(nodes, palette, "tertiary", x=-720, y=-1320)
            self._link_group_input(links, facing_socket, shader_group, "Iridescence Facing Color")
            self._link_group_input(links, grazing_socket, shader_group, "Iridescence Grazing Color")
        else:
            self._set_socket_default(_input_socket(shader_group, "Iridescence Facing Color"), (0.0, 0.0, 0.0, 1.0))
            self._set_socket_default(_input_socket(shader_group, "Iridescence Grazing Color"), (0.0, 0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Iridescence Ramp Color"), (0.0, 0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Iridescence Ramp Weight"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Iridescence Strength"), 1.0)
        iridescence_active = angle_shift_enabled and (submaterial.decoded_feature_flags.has_iridescence or _palette_has_iridescence(palette))
        self._set_socket_default(_input_socket(shader_group, "Iridescence Factor"), 1.0 if iridescence_active else 0.0)
        self._set_socket_default(_input_socket(shader_group, "Stencil Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Stencil Color Factor"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Stencil Factor"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Stencil Roughness"), 0.45)
        self._set_socket_default(_input_socket(shader_group, "Stencil Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Stencil Specular Tint"), (1.0, 1.0, 1.0, 1.0))
        self._link_group_input(links, iridescence_ramp_color, shader_group, "Iridescence Ramp Color")
        if iridescence_ramp_color is not None:
            self._set_socket_default(_input_socket(shader_group, "Iridescence Ramp Weight"), 1.0)
        iridescence_strength = _optional_float_public_param(submaterial, "IridescenceStrength")
        if iridescence_strength is not None and iridescence_strength > 0.0:
            self._set_socket_default(_input_socket(shader_group, "Iridescence Strength"), iridescence_strength)
        self._set_socket_default(_input_socket(shader_group, "Wear Factor"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Damage Factor"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Macro Normal Color"), (0.5, 0.5, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Macro Normal Strength"), 0.4)
        self._set_socket_default(_input_socket(shader_group, "Displacement Strength"), 0.05)
        self._set_socket_default(_input_socket(shader_group, "Emission Color"), (0.0, 0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Emission Strength"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Disable Shadows"), self._plan_casts_no_shadows(plan))
        shader_group["starbreaker_angle_shift_enabled"] = angle_shift_enabled

        self._link_group_input(links, top_base_color, shader_group, "Top Base Color")
        self._link_group_input(links, top_base_alpha, shader_group, "Top Alpha")
        self._link_group_input(links, primary.color, shader_group, "Primary Color")
        self._link_group_input(links, primary.alpha, shader_group, "Primary Alpha")
        self._link_group_input(links, primary.roughness, shader_group, "Primary Roughness")
        self._link_group_input(links, primary.specular, shader_group, "Primary Specular")
        self._link_group_input(links, primary.specular_tint, shader_group, "Primary Specular Tint")
        self._link_group_input(links, primary.metallic, shader_group, "Primary Metallic")
        self._link_group_input(links, primary.normal, shader_group, "Primary Normal")
        self._link_group_input(links, secondary.color, shader_group, "Secondary Color")
        self._link_group_input(links, secondary.alpha, shader_group, "Secondary Alpha")
        self._link_group_input(links, secondary.roughness, shader_group, "Secondary Roughness")
        self._link_group_input(links, secondary.specular, shader_group, "Secondary Specular")
        self._link_group_input(links, secondary.specular_tint, shader_group, "Secondary Specular Tint")
        self._link_group_input(links, secondary.metallic, shader_group, "Secondary Metallic")
        self._link_group_input(links, secondary.normal, shader_group, "Secondary Normal")
        self._link_group_input(links, wear_factor, shader_group, "Wear Factor")
        self._link_group_input(links, damage_factor, shader_group, "Damage Factor")
        self._link_group_input(links, stencil.color, shader_group, "Stencil Color")
        self._link_group_input(links, stencil.color_factor, shader_group, "Stencil Color Factor")
        self._link_group_input(links, stencil.factor, shader_group, "Stencil Factor")
        self._link_group_input(links, stencil.roughness, shader_group, "Stencil Roughness")
        self._link_group_input(links, stencil.specular, shader_group, "Stencil Specular")
        self._link_group_input(links, stencil.specular_tint, shader_group, "Stencil Specular Tint")
        self._link_group_input(
            links,
            macro_normal_node.outputs[0] if macro_normal_node is not None else None,
            shader_group,
            "Macro Normal Color",
        )
        self._link_group_input(
            links,
            displacement_node.outputs[0] if displacement_node is not None else None,
            shader_group,
            "Displacement Height",
        )
        self._link_group_input(
            links,
            emissive_node.outputs[0] if emissive_node is not None else None,
            shader_group,
            "Emission Color",
        )
        if emissive_node is not None:
            self._set_socket_default(_input_socket(shader_group, "Emission Strength"), 1.0)

        surface_shader = _output_socket(shader_group, "Shader")
        if surface_shader is not None:
            links.new(surface_shader, output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_illum_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)

        blend_mask_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot12",), roles=("wear_mask", "pattern_mask", "blend_mask"))
        blend_mask_node = self._image_node(
            nodes,
            blend_mask_ref.export_path if blend_mask_ref is not None else None,
            x=-720,
            y=160,
            is_color=False,
        )
        blend_mask_socket = blend_mask_node.outputs[0] if blend_mask_node is not None else None

        material_channel = submaterial.palette_routing.material_channel.name if submaterial.palette_routing.material_channel is not None else None

        primary_color_node = self._image_node(
            nodes,
            self._texture_export_path(submaterial, "base_color", "diffuse") or self._texture_path_for_slot(submaterial, "TexSlot1"),
            x=-720,
            y=520,
            is_color=True,
        )
        decal_palette = self._palette_decal_sockets(
            nodes,
            links,
            palette,
            material_channel,
            x=-420,
            y=520,
        )
        primary_normal_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot2",), roles=("normal_gloss",))
        primary_normal_node = self._image_node(
            nodes,
            primary_normal_ref.export_path if primary_normal_ref is not None else None,
            x=-720,
            y=-140,
            is_color=False,
        )
        primary_detail = self._detail_texture_channels(nodes, self._texture_path_for_slot(submaterial, "TexSlot6"), x=-720, y=-420)
        primary_roughness, primary_roughness_is_smoothness = self._roughness_socket_for_texture_reference(nodes, primary_normal_ref, x=-460, y=-140)
        primary_specular = self._specular_socket_for_texture_path(nodes, self._texture_path_for_slot(submaterial, "TexSlot4"), x=-720, y=760)
        primary = self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=decal_palette.color if decal_palette.color is not None else (primary_color_node.outputs[0] if primary_color_node is not None else None),
            base_alpha_socket=decal_palette.alpha if decal_palette.alpha is not None else (_output_socket(primary_color_node, "Alpha") if primary_color_node is not None else None),
            normal_color_socket=primary_normal_node.outputs[0] if primary_normal_node is not None else None,
            roughness_socket=primary_roughness,
            roughness_source_is_smoothness=primary_roughness_is_smoothness,
            detail_channels=primary_detail,
            detail_diffuse_strength=0.35,
            detail_gloss_strength=0.35,
            detail_bump_strength=0.15,
            tint_color=None,
            palette=palette,
            palette_channel_name=material_channel,
            palette_finish_channel_name=material_channel,
            palette_glossiness=palette_finish_glossiness(palette, material_channel),
            specular_value=0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, material_channel)) or 0.0,
            metallic_value=0.0,
            specular_color=None,
            x=-180,
            y=220,
            label="Primary Layer",
        )

        secondary_color_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot9",), roles=("alternate_base_color", "base_color", "diffuse"))
        secondary_color_node = self._image_node(
            nodes,
            secondary_color_ref.export_path if secondary_color_ref is not None else None,
            x=-720,
            y=20,
            is_color=True,
        )
        secondary_normal_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot3",), roles=("normal_gloss",))
        secondary_normal_node = self._image_node(
            nodes,
            secondary_normal_ref.export_path if secondary_normal_ref is not None else None,
            x=-720,
            y=-700,
            is_color=False,
        )
        secondary_detail = self._detail_texture_channels(nodes, self._texture_path_for_slot(submaterial, "TexSlot13"), x=-720, y=-980)
        secondary_roughness, secondary_roughness_is_smoothness = self._roughness_socket_for_texture_reference(nodes, secondary_normal_ref, x=-460, y=-700)
        secondary_specular = self._specular_socket_for_texture_path(nodes, self._texture_path_for_slot(submaterial, "TexSlot10"), x=-720, y=980)
        secondary = self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=secondary_color_node.outputs[0] if secondary_color_node is not None else None,
            base_alpha_socket=_output_socket(secondary_color_node, "Alpha") if secondary_color_node is not None else None,
            normal_color_socket=secondary_normal_node.outputs[0] if secondary_normal_node is not None else None,
            roughness_socket=secondary_roughness,
            roughness_source_is_smoothness=secondary_roughness_is_smoothness,
            detail_channels=secondary_detail,
            detail_diffuse_strength=0.35,
            detail_gloss_strength=0.35,
            detail_bump_strength=0.15,
            tint_color=None,
            palette=palette,
            palette_channel_name=material_channel,
            palette_finish_channel_name=material_channel,
            palette_glossiness=palette_finish_glossiness(palette, material_channel),
            specular_value=0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, material_channel)) or 0.0,
            metallic_value=0.0,
            specular_color=None,
            x=-180,
            y=-140,
            label="Secondary Layer",
        )

        height_primary = self._mask_socket(nodes, self._texture_path_for_slot(submaterial, "TexSlot8"), x=-720, y=-1240)
        height_secondary = self._mask_socket(nodes, self._texture_path_for_slot(submaterial, "TexSlot11"), x=-720, y=-1400)

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_illum_group()
        shader_group.location = (140, 0)
        shader_group.label = "StarBreaker Illum"
        self._set_socket_default(_input_socket(shader_group, "Primary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Primary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Roughness"), 0.35)
        self._set_socket_default(_input_socket(shader_group, "Primary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Primary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Secondary Alpha"), 1.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Roughness"), 0.35)
        self._set_socket_default(_input_socket(shader_group, "Secondary Specular"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Secondary Normal"), (0.0, 0.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Blend Mask"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "POM Strength"), 0.0)
        self._set_socket_default(_input_socket(shader_group, "Emission Strength"), self._illum_emission_strength(submaterial))
        self._set_socket_default(_input_socket(shader_group, "Disable Shadows"), self._plan_casts_no_shadows(plan))

        self._link_group_input(links, primary.color, shader_group, "Primary Color")
        self._link_group_input(links, primary.alpha, shader_group, "Primary Alpha")
        self._link_group_input(links, primary.roughness, shader_group, "Primary Roughness")
        self._link_group_input(links, primary.specular, shader_group, "Primary Specular")
        self._link_group_input(links, primary.normal, shader_group, "Primary Normal")
        self._link_group_input(links, secondary.color, shader_group, "Secondary Color")
        self._link_group_input(links, secondary.alpha, shader_group, "Secondary Alpha")
        self._link_group_input(links, secondary.roughness, shader_group, "Secondary Roughness")
        self._link_group_input(links, secondary.specular, shader_group, "Secondary Specular")
        self._link_group_input(links, secondary.normal, shader_group, "Secondary Normal")
        self._link_group_input(links, blend_mask_socket, shader_group, "Blend Mask")
        if plan.template_key == "parallax_pom":
            self._link_group_input(links, height_primary, shader_group, "Primary Height")
            self._link_group_input(links, height_secondary, shader_group, "Secondary Height")
            self._set_socket_default(
                _input_socket(shader_group, "POM Strength"),
                max(0.03, min(0.2, _float_public_param(submaterial, "PomDisplacement", "HeightBias") or 0.08)),
            )

        surface_shader = _output_socket(shader_group, "Shader")
        if surface_shader is not None:
            links.new(surface_shader, output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _ensure_cycles_denoising_support(self) -> None:
        cycles = getattr(self.context.scene, "cycles", None)
        view_layer = getattr(self.context, "view_layer", None)
        view_layer_cycles = getattr(view_layer, "cycles", None) if view_layer is not None else None
        if cycles is None or view_layer_cycles is None:
            return
        if not getattr(cycles, "use_denoising", False):
            return
        if getattr(cycles, "denoiser", None) != "OPENIMAGEDENOISE":
            return
        if getattr(cycles, "denoising_input_passes", "RGB") == "RGB":
            return
        if hasattr(view_layer_cycles, "denoising_store_passes"):
            view_layer_cycles.denoising_store_passes = True

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
            "hard_surface_v29",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeMixShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime HardSurface",
            signature="hard_surface_v29",
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
                ("Disable Shadows", "NodeSocketBool"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "hard_surface_v29":
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
        iridescence_metallic_boost = nodes.new("ShaderNodeMapRange")
        iridescence_metallic_boost.location = (80, -180)
        iridescence_metallic_boost.clamp = True
        iridescence_metallic_boost.inputs[1].default_value = 0.0
        iridescence_metallic_boost.inputs[2].default_value = 1.0
        iridescence_metallic_boost.inputs[3].default_value = 0.0
        iridescence_metallic_boost.inputs[4].default_value = 1.0
        links.new(_output_socket(group_input, "Iridescence Factor"), iridescence_metallic_boost.inputs[0])
        metallic_max = nodes.new("ShaderNodeMath")
        metallic_max.location = (260, -180)
        metallic_max.operation = "MAXIMUM"
        links.new(metallic_layer_mix.outputs[0], metallic_max.inputs[0])
        links.new(iridescence_metallic_boost.outputs[0], metallic_max.inputs[1])
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

        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (520, -200)
        shadow_toggle = nodes.new("ShaderNodeMath")
        shadow_toggle.location = (700, -200)
        shadow_toggle.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Disable Shadows"), shadow_toggle.inputs[0])
        links.new(_output_socket(light_path, "Is Shadow Ray"), shadow_toggle.inputs[1])
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (700, -380)
        shadow_mix = nodes.new("ShaderNodeMixShader")
        shadow_mix.location = (900, -40)
        links.new(shadow_toggle.outputs[0], shadow_mix.inputs[0])
        links.new(principled.outputs[0], shadow_mix.inputs[1])
        links.new(transparent.outputs[0], shadow_mix.inputs[2])
        links.new(shadow_mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "hard_surface_v29"
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
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Glass",
            "glass_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfGlass": 1,
                "ShaderNodeNormalMap": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Glass",
            signature="glass_v1",
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
        if group_tree.get("starbreaker_runtime_built_signature") == "glass_v1":
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

        links.new(glass.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "glass_v1"
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
        """Wrap Principled BSDF + NormalMap + Bump + shadowless mix into a shader group.

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
            Shadowless          float   (0 = cast shadows, 1 = invisible to shadow rays)

        Outputs:
            Shader              shader
        """
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Principled",
            "principled_v1",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeNormalMap": 1,
                "ShaderNodeBump": 1,
                "ShaderNodeNewGeometry": 1,
                "ShaderNodeMix": 2,
                "ShaderNodeLightPath": 1,
                "ShaderNodeBsdfTransparent": 1,
                "ShaderNodeMixShader": 1,
                "ShaderNodeMath": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Principled",
            signature="principled_v1",
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
                ("Shadowless", "NodeSocketFloat"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "principled_v1":
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
        _set_group_input_default(group_input, "Shadowless", 0.0)

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

        # Shadowless branch: factor = Is Shadow Ray * Shadowless.
        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (220, -320)
        shadow_gate = nodes.new("ShaderNodeMath")
        shadow_gate.location = (420, -280)
        shadow_gate.operation = "MULTIPLY"
        shadow_gate.use_clamp = True
        links.new(_output_socket(light_path, "Is Shadow Ray"), shadow_gate.inputs[0])
        links.new(_output_socket(group_input, "Shadowless"), shadow_gate.inputs[1])

        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (420, -120)

        shadow_mix = nodes.new("ShaderNodeMixShader")
        shadow_mix.location = (620, -40)
        links.new(shadow_gate.outputs[0], shadow_mix.inputs[0])
        links.new(principled.outputs[0], shadow_mix.inputs[1])
        links.new(transparent.outputs[0], shadow_mix.inputs[2])

        links.new(shadow_mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "principled_v1"
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

    def _ensure_runtime_illum_group(self) -> bpy.types.ShaderNodeTree:
        self._invalidate_runtime_group_if_unexpected(
            "StarBreaker Runtime Illum",
            "illum_v3",
            {
                "NodeGroupInput": 1,
                "NodeGroupOutput": 1,
                "ShaderNodeBsdfPrincipled": 1,
                "ShaderNodeEmission": 1,
                "ShaderNodeAddShader": 1,
                "ShaderNodeMixShader": 1,
            },
        )
        group_tree, group_input, group_output = self._begin_runtime_shared_group(
            "StarBreaker Runtime Illum",
            signature="illum_v3",
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
                ("Disable Shadows", "NodeSocketBool"),
            ],
            outputs=[("Shader", "NodeSocketShader")],
        )
        if group_tree.get("starbreaker_runtime_built_signature") == "illum_v3":
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

        light_path = nodes.new("ShaderNodeLightPath")
        light_path.location = (320, -220)
        shadow_toggle = nodes.new("ShaderNodeMath")
        shadow_toggle.location = (500, -220)
        shadow_toggle.operation = "MULTIPLY"
        links.new(_output_socket(group_input, "Disable Shadows"), shadow_toggle.inputs[0])
        links.new(_output_socket(light_path, "Is Shadow Ray"), shadow_toggle.inputs[1])
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        transparent.location = (500, -400)
        shadow_mix = nodes.new("ShaderNodeMixShader")
        shadow_mix.location = (700, -40)
        links.new(shadow_toggle.outputs[0], shadow_mix.inputs[0])
        links.new(add_shader.outputs[0], shadow_mix.inputs[1])
        links.new(transparent.outputs[0], shadow_mix.inputs[2])
        links.new(shadow_mix.outputs[0], group_output.inputs["Shader"])
        group_tree["starbreaker_runtime_built_signature"] = "illum_v3"
        return group_tree

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
    ) -> LayerSurfaceSockets:
        if layer is None:
            return LayerSurfaceSockets()

        base_texture = _layer_texture_reference(layer, slots=("TexSlot1",), roles=("base_color", "diffuse"))
        base_node = self._image_node(nodes, base_texture.export_path if base_texture is not None else None, x=x, y=y, is_color=True)
        detail_ref = _layer_texture_reference(layer, slots=detail_slots)
        detail_channels = self._detail_texture_channels(nodes, detail_ref.export_path if detail_ref is not None else None, x=x, y=y - 420)
        normal_ref = _layer_texture_reference(layer, roles=("normal_gloss",), alpha_semantic="smoothness")
        normal_node = self._image_node(nodes, normal_ref.export_path if normal_ref is not None else None, x=x, y=y - 560, is_color=False)
        roughness, roughness_is_smoothness = self._roughness_socket_for_texture_reference(nodes, normal_ref, x=x + 180, y=y - 560)
        layer_channel_name = layer.palette_channel.name if layer.palette_channel is not None else None
        return self._connect_layer_surface_group(
            nodes,
            links,
            base_color_socket=base_node.outputs[0] if base_node is not None else None,
            base_alpha_socket=_output_socket(base_node, "Alpha") if base_node is not None else None,
            normal_color_socket=normal_node.outputs[0] if normal_node is not None else None,
            roughness_socket=roughness,
            roughness_source_is_smoothness=roughness_is_smoothness,
            detail_channels=detail_channels,
            detail_diffuse_strength=max(0.0, min(1.0, _float_layer_public_param(layer, "DetailDiffuse"))),
            detail_gloss_strength=max(0.0, min(1.0, _float_layer_public_param(layer, "DetailGloss"))),
            detail_bump_strength=max(0.0, _float_layer_public_param(layer, "DetailBump")),
            tint_color=layer.tint_color,
            palette=palette,
            palette_channel_name=layer_channel_name,
            palette_finish_channel_name=layer_channel_name,
            palette_glossiness=palette_finish_glossiness(palette, layer_channel_name),
            specular_value=_mean_triplet(_layer_snapshot_triplet(layer, "specular")) or 0.0,
            palette_specular_value=_mean_triplet(palette_finish_specular(palette, layer_channel_name)) or 0.0,
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
    ) -> LayerSurfaceSockets:
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
        self._set_socket_default(_input_socket(group_node, "Detail Diffuse Strength"), detail_diffuse_strength)
        self._set_socket_default(_input_socket(group_node, "Detail Gloss Strength"), detail_gloss_strength)
        self._set_socket_default(_input_socket(group_node, "Detail Bump Strength"), detail_bump_strength)
        self._set_socket_default(_input_socket(group_node, "Normal Color"), (0.5, 0.5, 1.0, 1.0))
        self._set_socket_default(_input_socket(group_node, "Roughness Source"), 0.45)
        self._set_socket_default(_input_socket(group_node, "Roughness Source Is Smoothness"), roughness_source_is_smoothness)
        self._set_socket_default(
            _input_socket(group_node, "Palette Glossiness"),
            max(0.0, min(1.0, palette_glossiness)) if palette_glossiness is not None else 0.0,
        )
        self._set_socket_default(_input_socket(group_node, "Specular Value"), specular_value)
        self._set_socket_default(_input_socket(group_node, "Palette Specular"), palette_specular_value)
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
            palette_color_socket = self._palette_color_socket(nodes, palette, palette_channel_name, x=x - 220, y=y - 160)
            finish_channel_name = palette_finish_channel_name or palette_channel_name
            palette_gloss_socket = self._palette_glossiness_socket(nodes, palette, finish_channel_name, x=x - 220, y=y - 320)
            palette_specular_color = self._palette_specular_socket(nodes, palette, finish_channel_name, x=x - 220, y=y - 480)
            if palette_specular_color is not None:
                palette_specular_tint_socket = palette_specular_color
                rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
                rgb_to_bw.location = (x - 20, y - 480)
                links.new(palette_specular_color, rgb_to_bw.inputs[0])
                palette_specular_socket = rgb_to_bw.outputs[0]

        self._link_group_input(links, base_color_socket, group_node, "Base Color")
        self._link_group_input(links, base_alpha_socket, group_node, "Base Alpha")
        self._link_group_input(links, normal_color_socket, group_node, "Normal Color")
        self._link_group_input(links, roughness_socket, group_node, "Roughness Source")
        self._link_group_input(links, palette_color_socket, group_node, "Palette Color")
        self._link_group_input(links, palette_gloss_socket, group_node, "Palette Glossiness")
        self._link_group_input(links, palette_specular_socket, group_node, "Palette Specular")
        if detail_channels is not None:
            self._link_group_input(links, detail_channels.get("red"), group_node, "Detail Color Mask")
            self._link_group_input(links, detail_channels.get("green"), group_node, "Detail Height Mask")
            self._link_group_input(links, detail_channels.get("blue"), group_node, "Detail Gloss Mask")

        return LayerSurfaceSockets(
            color=SocketRef(group_node, "Color"),
            alpha=SocketRef(group_node, "Alpha"),
            normal=SocketRef(group_node, "Normal"),
            roughness=SocketRef(group_node, "Roughness"),
            specular=SocketRef(group_node, "Specular"),
            specular_tint=(SocketRef(palette_specular_tint_socket.node, palette_specular_tint_socket.name) if palette_specular_tint_socket is not None else None),
            metallic=SocketRef(group_node, "Metallic"),
        )

    def _link_group_input(self, links: bpy.types.NodeLinks, source_socket: Any, group_node: bpy.types.Node, socket_name: str) -> None:
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
        if not getattr(source_socket, "is_output", False) or getattr(target_socket, "is_output", False):
            return
        try:
            links.new(source_socket, target_socket)
        except RuntimeError as exc:
            if "Same input/output direction of sockets" in str(exc):
                return
            raise

    def _set_socket_default(self, socket: Any, value: Any) -> None:
        if socket is not None and hasattr(socket, "default_value"):
            socket.default_value = value

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
        return self._multiply_color_socket(nodes, links, color_socket, palette_socket, x=x + 180, y=y)

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
        separate = nodes.new("ShaderNodeSeparateColor")
        separate.location = (x + 180, y)
        if hasattr(separate, "mode"):
            separate.mode = "RGB"
        image_node.id_data.links.new(image_node.outputs[0], separate.inputs[0])
        return {
            "red": _output_socket(separate, "Red", "R"),
            "green": _output_socket(separate, "Green", "G"),
            "blue": _output_socket(separate, "Blue", "B"),
            "alpha": _output_socket(image_node, "Alpha"),
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
        if color_socket is None or detail_channels is None or detail_channels.get("red") is None or strength <= 0.0:
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
        return self._multiply_color_socket(nodes, links, color_socket, tint_mix.outputs[0], x=x + 360, y=y)

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
        if roughness_socket is None or detail_channels is None or detail_channels.get("blue") is None or strength <= 0.0:
            return roughness_socket
        detail_value = nodes.new("ShaderNodeMix")
        detail_value.location = (x, y)
        if hasattr(detail_value, "data_type"):
            detail_value.data_type = "FLOAT"
        detail_value.inputs[0].default_value = max(0.0, min(1.0, strength))
        detail_value.inputs[2].default_value = 1.0
        links.new(detail_channels["blue"], detail_value.inputs[3])
        return self._multiply_value_socket(nodes, links, roughness_socket, detail_value.outputs[0], x=x + 180, y=y)

    def _roughness_socket_for_texture_reference(
        self,
        nodes: bpy.types.Nodes,
        texture: TextureReference | None,
        *,
        x: int,
        y: int,
    ) -> tuple[Any, bool]:
        if texture is None or texture.export_path is None:
            return None, False
        if texture.alpha_semantic == "smoothness":
            smoothness = self._texture_alpha_socket(nodes, texture.export_path, x=x, y=y, is_color=False)
            if smoothness is not None:
                return smoothness, True
        image_node = self._image_node(nodes, texture.export_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None, False
        return image_node.outputs[0], False

    def _specular_socket_for_texture_path(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
        rgb_to_bw.location = (x + 180, y)
        image_node.id_data.links.new(image_node.outputs[0], rgb_to_bw.inputs[0])
        return rgb_to_bw.outputs[0]

    def _mask_socket(self, nodes: bpy.types.Nodes, image_path: str | None, *, x: int, y: int) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        return image_node.outputs[0]

    def _tiled_image_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
        tiling: float = 1.0,
        uv_map_name: str | None = None,
    ) -> bpy.types.ShaderNodeTexImage | None:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=is_color)
        if image_node is None:
            return None
        if uv_map_name is None and math.isclose(tiling, 1.0, rel_tol=1e-6, abs_tol=1e-6):
            return image_node
        uv_source = None
        if uv_map_name:
            uv_map = nodes.new("ShaderNodeUVMap")
            uv_map.location = (x - 360, y)
            uv_map.uv_map = uv_map_name
            uv_source = _output_socket(uv_map, "UV")
        else:
            tex_coord = nodes.new("ShaderNodeTexCoord")
            tex_coord.location = (x - 360, y)
            uv_source = _output_socket(tex_coord, "UV")
        mapping = nodes.new("ShaderNodeMapping")
        mapping.location = (x - 180, y)
        scale_input = _input_socket(mapping, "Scale")
        if scale_input is not None and hasattr(scale_input, "default_value"):
            scale_input.default_value[0] = tiling
            scale_input.default_value[1] = tiling
            if len(scale_input.default_value) > 2:
                scale_input.default_value[2] = 1.0
        vector_input = _input_socket(mapping, "Vector")
        image_vector = _input_socket(image_node, "Vector")
        mapped_vector = _output_socket(mapping, "Vector")
        if uv_source is not None and vector_input is not None:
            links.new(uv_source, vector_input)
        if mapped_vector is not None and image_vector is not None:
            links.new(mapped_vector, image_vector)
        return image_node

    def _image_mask_socket_from_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        image_node: bpy.types.ShaderNodeTexImage | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if image_node is None:
            return None
        rgb_to_bw = nodes.new("ShaderNodeRGBToBW")
        rgb_to_bw.location = (x, y)
        links.new(image_node.outputs[0], rgb_to_bw.inputs[0])
        alpha_socket = _output_socket(image_node, "Alpha")
        if alpha_socket is None:
            return rgb_to_bw.outputs[0]
        return self._multiply_value_socket(nodes, links, rgb_to_bw.outputs[0], alpha_socket, x=x + 180, y=y)

    def _masked_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        mask_socket: Any,
        color_value: tuple[float, float, float],
        *,
        x: int,
        y: int,
    ) -> Any:
        if mask_socket is None:
            return None
        tint_socket = self._value_color_socket(nodes, (*color_value, 1.0), x=x, y=y)
        black_socket = self._value_color_socket(nodes, (0.0, 0.0, 0.0, 1.0), x=x, y=y - 120)
        return self._mix_color_socket(nodes, links, black_socket, tint_socket, mask_socket, x=x + 180, y=y - 40)

    def _add_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        add = nodes.new("ShaderNodeMixRGB")
        add.location = (x, y)
        add.blend_type = "ADD"
        add.inputs[0].default_value = 1.0
        self._link_color_output(socket_a, add.inputs[1])
        self._link_color_output(socket_b, add.inputs[2])
        return add.outputs[0]

    def _hard_surface_stencil_overlay_sockets(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> StencilOverlaySockets:
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
        stencil_uv_map = "UVMap.001" if (_optional_float_public_param(submaterial, "UseUV2ForStencil") or 0.0) > 0.5 else None
        stencil_node = self._tiled_image_node(
            nodes,
            links,
            stencil_ref.export_path,
            x=x,
            y=y,
            is_color=True,
            tiling=stencil_tiling if stencil_tiling is not None and stencil_tiling > 0.0 else 1.0,
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
        specular_1 = _public_param_triplet(submaterial, "StencilSpecularColor1", "StencilSpecular1", "StencilSpecularColor")
        specular_2 = _public_param_triplet(submaterial, "StencilSpecularColor2", "StencilSpecular2")
        specular_3 = _public_param_triplet(submaterial, "StencilSpecularColor3", "StencilSpecular3")
        multi_channel_stencil = any(value is not None for value in (tint_2, tint_3, specular_2, specular_3))
        tint_override = _optional_float_public_param(submaterial, "StencilTintOverride") or 0.0
        stencil_glossiness = _optional_float_public_param(submaterial, "StencilGlossiness")
        opacity = _optional_float_public_param(submaterial, "StencilOpacity")

        # Optional breakup image texture at top level.
        breakup_node = None
        breakup_strength = 0.0
        breakup_ref = _submaterial_texture_reference(submaterial, slots=("TexSlot8",), roles=("breakup", "grime_breakup"))
        if breakup_ref is not None and breakup_ref.export_path is not None:
            breakup_tiling = _optional_float_public_param(submaterial, "StencilBreakupTiling")
            breakup_node = self._tiled_image_node(
                nodes,
                links,
                breakup_ref.export_path,
                x=x,
                y=y - 220,
                is_color=False,
                tiling=breakup_tiling if breakup_tiling is not None and breakup_tiling > 0.0 else 1.0,
                uv_map_name=stencil_uv_map,
            )
            breakup_strength = max(
                0.0,
                min(1.0, _optional_float_public_param(submaterial, "StencilDiffuseBreakup", "StencilGlossBreakup") or 0.0),
            )

        # Instantiate the Runtime HardSurface Stencil group.
        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x + 420, y)
        group_node.node_tree = self._ensure_runtime_hardsurface_stencil_group()
        group_node.label = "StarBreaker HardSurface Stencil"

        # Wire image textures.
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
            group_node.inputs["Breakup Enable"].default_value = 1.0 if breakup_strength > 0.0 else 0.0
        else:
            group_node.inputs["Breakup Enable"].default_value = 0.0

        if opacity is not None:
            group_node.inputs["Stencil Opacity"].default_value = max(0.0, min(1.0, opacity))
        if stencil_glossiness is not None:
            group_node.inputs["Stencil Glossiness"].default_value = max(0.0, min(1.0, stencil_glossiness))

        # Decide mode + tint defaults.
        use_override = stencil_ref.is_virtual or tint_override > 0.0
        if use_override:
            group_node.inputs["Mode"].default_value = 0.0
            group_node.inputs["Tint1"].default_value = (*(tint if tint is not None else (1.0, 1.0, 1.0)), 1.0)
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

        # Specular tints.
        for slot, color in (("Specular1", specular_1), ("Specular2", specular_2), ("Specular3", specular_3)):
            enable_key = f"{slot} Enable"
            if color is not None and (_mean_triplet(color) or 0.0) > 0.0:
                group_node.inputs[slot].default_value = (*color, 1.0)
                group_node.inputs[enable_key].default_value = 1.0
            else:
                group_node.inputs[enable_key].default_value = 0.0

        has_specular = any(
            c is not None and (_mean_triplet(c) or 0.0) > 0.0 for c in (specular_1, specular_2, specular_3)
        )

        return StencilOverlaySockets(
            color=group_node.outputs["Color"],
            color_factor=group_node.outputs["Color Factor"],
            factor=group_node.outputs["Factor"],
            roughness=group_node.outputs["Roughness"] if stencil_glossiness is not None else None,
            specular=group_node.outputs["Specular"] if has_specular else None,
            specular_tint=group_node.outputs["Specular Tint"] if has_specular else None,
        )

    def _mix_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        factor_socket: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        if factor_socket is None:
            return socket_a
        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x, y)
        mix.blend_type = "MIX"
        links.new(factor_socket, mix.inputs[0])
        self._link_color_output(socket_a, mix.inputs[1])
        self._link_color_output(socket_b, mix.inputs[2])
        return mix.outputs[0]

    def _multiply_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x, y)
        mix.blend_type = "MULTIPLY"
        mix.inputs[0].default_value = 1.0
        self._link_color_output(socket_a, mix.inputs[1])
        self._link_color_output(socket_b, mix.inputs[2])
        return mix.outputs[0]

    def _mix_value_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        factor_socket: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        if factor_socket is None:
            return socket_a
        mix = nodes.new("ShaderNodeMix")
        mix.location = (x, y)
        if hasattr(mix, "data_type"):
            mix.data_type = "FLOAT"
        links.new(factor_socket, mix.inputs[0])
        links.new(socket_a, mix.inputs[2])
        links.new(socket_b, mix.inputs[3])
        return mix.outputs[0]

    def _multiply_value_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        multiply = nodes.new("ShaderNodeMath")
        multiply.location = (x, y)
        multiply.operation = "MULTIPLY"
        links.new(socket_a, multiply.inputs[0])
        links.new(socket_b, multiply.inputs[1])
        return multiply.outputs[0]

    def _add_clamped_value_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        add = nodes.new("ShaderNodeMath")
        add.location = (x, y)
        add.operation = "ADD"
        add.use_clamp = True
        links.new(socket_a, add.inputs[0])
        links.new(socket_b, add.inputs[1])
        return add.outputs[0]

    def _normal_from_color_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        color_socket: Any,
        *,
        x: int,
        y: int,
        strength: float,
    ) -> Any:
        if color_socket is None:
            return None
        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.location = (x, y)
        strength_input = _input_socket(normal_map, "Strength")
        if strength_input is not None:
            strength_input.default_value = strength
        links.new(color_socket, _input_socket(normal_map, "Color"))
        return _output_socket(normal_map, "Normal")

    def _bump_normal_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        height_socket: Any,
        base_normal_socket: Any,
        *,
        strength: float | None = None,
        strength_socket: Any = None,
        x: int,
        y: int,
    ) -> Any:
        if height_socket is None:
            return base_normal_socket
        bump = nodes.new("ShaderNodeBump")
        bump.location = (x, y)
        if strength_socket is not None:
            links.new(strength_socket, bump.inputs[0])
        elif strength is not None:
            bump.inputs[0].default_value = strength
        links.new(height_socket, bump.inputs[2])
        if base_normal_socket is not None:
            links.new(base_normal_socket, bump.inputs[3])
        return bump.outputs[0]

    def _combine_normal_socket(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        socket_a: Any,
        socket_b: Any,
        *,
        x: int,
        y: int,
    ) -> Any:
        if socket_a is None:
            return socket_b
        if socket_b is None:
            return socket_a
        add = nodes.new("ShaderNodeVectorMath")
        add.location = (x, y)
        add.operation = "ADD"
        links.new(socket_a, add.inputs[0])
        links.new(socket_b, add.inputs[1])
        normalize = nodes.new("ShaderNodeVectorMath")
        normalize.location = (x + 180, y)
        normalize.operation = "NORMALIZE"
        links.new(add.outputs[0], normalize.inputs[0])
        return normalize.outputs[0]

    def _texture_path_for_slot(self, submaterial: SubmaterialRecord, slot: str) -> str | None:
        texture = _submaterial_texture_reference(submaterial, slots=(slot,))
        return texture.export_path if texture is not None else None
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)
        return True

    def _contract_input_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        group_contract: ShaderGroupContract,
        contract_input: ContractInput,
        *,
        x: int,
        y: int,
    ) -> Any:
        if contract_input.name.startswith("Palette_"):
            if palette is None:
                return None
            channel_name = contract_input.name.removeprefix("Palette_").lower()
            used_channels = {channel.name.lower() for channel in material_palette_channels(submaterial)}
            if channel_name not in used_channels:
                return None
            return self._palette_color_socket(nodes, palette, channel_name, x=x, y=y)

        semantic = (contract_input.semantic or contract_input.name).lower()
        if _routes_virtual_tint_palette_decal_to_decal_source(submaterial, contract_input):
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).color
        if _suppresses_virtual_tint_palette_stencil_input(submaterial, contract_input):
            return None
        if contract_input.source_slot is not None and contract_input.name.lower().endswith("_alpha"):
            return self._source_slot_alpha_socket(nodes, submaterial, contract_input, palette, x=x, y=y)

        texture = self._texture_reference_for_contract_input(submaterial, contract_input)
        if texture is not None and texture.is_virtual and texture.role == "tint_palette_decal":
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).color

        if "alpha" in semantic or "opacity" in semantic:
            return self._alpha_source_socket(
                nodes,
                submaterial,
                representative_textures(submaterial),
                x=x,
                y=y,
            )

        if contract_input.source_slot is None and "roughness" in semantic:
            return self._roughness_group_source_socket(
                nodes,
                submaterial,
                representative_textures(submaterial)["roughness"],
                x=x,
                y=y,
            )

        image_path = texture.export_path if texture is not None else self._texture_path_for_contract_input(submaterial, contract_input)
        if _contract_input_uses_color(contract_input):
            if any(item.name.startswith("Palette_") for item in group_contract.inputs):
                image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=True)
                if image_node is None:
                    return None
                return image_node.outputs[0]
            return self._color_source_socket(nodes, submaterial, palette, image_path, x=x, y=y)
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None
        return image_node.outputs[0]

    def _roughness_group_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if image_path:
            image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=False)
            if image_node is not None:
                image_node.label = "METALLIC ROUGHNESS"

                separate = nodes.new("ShaderNodeSeparateColor")
                separate.location = (x + 180, y)
                if hasattr(separate, "mode"):
                    separate.mode = "RGB"
                image_node.id_data.links.new(image_node.outputs[0], separate.inputs[0])
                return _output_socket(separate, "Green")

        smoothness_texture = self._smoothness_texture_reference(submaterial)
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

    def _smoothness_texture_reference(self, submaterial: SubmaterialRecord) -> TextureReference | None:
        return smoothness_texture_reference(submaterial)

    def _texture_reference_for_contract_input(self, submaterial: SubmaterialRecord, contract_input: ContractInput) -> TextureReference | None:
        source_slot = contract_input.source_slot
        if source_slot is None:
            return None
        texture = _matching_texture_reference(
            [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures],
            slots=(source_slot,),
        )
        if texture is not None:
            return texture

        for layer in submaterial.layer_manifest:
            texture = _layer_texture_reference(layer, slots=(source_slot,))
            if texture is not None:
                return texture
        return None

    def _texture_alpha_socket(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
    ) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=is_color)
        if image_node is None:
            return None
        return _output_socket(image_node, "Alpha")

    def _virtual_tint_palette_decal_sockets(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> LayerSurfaceSockets:
        fallback_color, alpha = self._virtual_tint_palette_decal_defaults(submaterial, palette)
        if palette is not None and self.package.resolve_path(palette_decal_texture(palette)) is not None:
            group_node = self._palette_group_node(nodes, nodes.id_data.links, palette, x=x, y=y)
            color_socket = _output_socket(group_node, "Decal Color")
            alpha_socket = _output_socket(group_node, "Decal Alpha")
            if color_socket is not None and alpha_socket is not None:
                if abs(alpha - 1.0) < 1e-6:
                    return LayerSurfaceSockets(color=color_socket, alpha=alpha_socket)
                alpha_multiply = next(
                    (node for node in nodes if node.bl_idname == "ShaderNodeMath" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA_MULTIPLY"),
                    None,
                )
                if alpha_multiply is None:
                    alpha_multiply = nodes.new("ShaderNodeMath")
                    alpha_multiply.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA_MULTIPLY"
                    alpha_multiply.label = "StarBreaker Virtual Tint Palette Decal Alpha"
                    alpha_multiply.operation = "MULTIPLY"
                alpha_multiply.location = (x + 220, y - 140)
                alpha_multiply.inputs[1].default_value = alpha
                nodes.id_data.links.new(alpha_socket, alpha_multiply.inputs[0])
                return LayerSurfaceSockets(color=color_socket, alpha=alpha_multiply.outputs[0])

        palette_color_socket = None
        if palette is not None:
            palette_channel = submaterial.palette_routing.material_channel
            channel_name = palette_channel.name if palette_channel is not None else ("glass" if submaterial.shader_family == "GlassPBR" else None)
            if channel_name is not None:
                palette_color_socket = self._palette_color_socket(nodes, palette, channel_name, x=x, y=y)

        color_node = next(
            (node for node in nodes if node.bl_idname == "ShaderNodeRGB" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"),
            None,
        )
        alpha_node = next(
            (node for node in nodes if node.bl_idname == "ShaderNodeValue" and getattr(node, "name", "") == "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"),
            None,
        )

        if color_node is None:
            color_node = nodes.new("ShaderNodeRGB")
            color_node.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_COLOR"
            color_node.label = "StarBreaker Virtual Tint Palette Decal"
        color_node.location = (x, y)

        if alpha_node is None:
            alpha_node = nodes.new("ShaderNodeValue")
            alpha_node.name = "STARBREAKER_VIRTUAL_TINT_PALETTE_DECAL_ALPHA"
            alpha_node.label = "StarBreaker Virtual Tint Palette Decal Alpha"
        alpha_node.location = (x, y - 140)

        if palette_color_socket is not None:
            alpha_node.outputs[0].default_value = alpha
            return LayerSurfaceSockets(color=palette_color_socket, alpha=alpha_node.outputs[0])

        color_node.outputs[0].default_value = (*fallback_color, 1.0)
        alpha_node.outputs[0].default_value = alpha
        return LayerSurfaceSockets(color=color_node.outputs[0], alpha=alpha_node.outputs[0])

    def _virtual_tint_palette_decal_defaults(
        self,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
    ) -> tuple[tuple[float, float, float], float]:
        color = (
            _public_param_triplet(submaterial, "StencilDiffuseColor1", "StencilDiffuse1", "StencilTintColor", "TintColor", "StencilDiffuseColor")
            or _resolved_submaterial_palette_color(submaterial, palette)
            or _authored_attribute_triplet(submaterial, "Diffuse")
            or (1.0, 1.0, 1.0)
        )
        alpha = _optional_float_public_param(submaterial, "StencilOpacity", "DecalDiffuseOpacity", "DecalAlphaMult")
        if alpha is None:
            alpha = 0.85 if submaterial.shader_family == "MeshDecal" else 0.5
        return color, max(0.0, min(1.0, alpha))

    def _invert_value_socket(self, nodes: bpy.types.Nodes, source_socket: Any, *, x: int, y: int) -> Any:
        invert = nodes.new("ShaderNodeMath")
        invert.location = (x, y)
        invert.operation = "SUBTRACT"
        invert.inputs[0].default_value = 1.0
        invert.id_data.links.new(source_socket, invert.inputs[1])
        return invert.outputs[0]

    def _source_slot_alpha_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        contract_input: ContractInput,
        palette: PaletteRecord | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        if _routes_virtual_tint_palette_decal_alpha_to_decal_source(submaterial, contract_input):
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).alpha
        if _suppresses_virtual_tint_palette_stencil_input(submaterial, contract_input):
            return None
        texture = self._texture_reference_for_contract_input(submaterial, contract_input)
        if texture is None:
            return None
        if texture.is_virtual and texture.role == "tint_palette_decal":
            return self._virtual_tint_palette_decal_sockets(nodes, submaterial, palette, x=x, y=y).alpha
        return self._texture_alpha_socket(nodes, texture.export_path, x=x, y=y, is_color=True)

    def _texture_path_for_contract_input(self, submaterial: SubmaterialRecord, contract_input: ContractInput) -> str | None:
        texture = self._texture_reference_for_contract_input(submaterial, contract_input)
        return texture.export_path if texture is not None else None

    def _build_nodraw_material(self, material: bpy.types.Material) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()
        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (250, 0)
        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_nodraw_group()
        _refresh_group_node_sockets(shader_group)
        shader_group.location = (0, 0)
        shader_group.label = "StarBreaker NoDraw"
        surface = _output_socket(shader_group, "Shader")
        if surface is not None:
            links.new(surface, output.inputs[0])
        self._configure_material(material, blend_method="CLIP", shadow_method="NONE")

    def _build_screen_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (550, 0)

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_screen_group()
        _refresh_group_node_sockets(shader_group)
        shader_group.location = (250, 0)
        shader_group.label = "StarBreaker Screen"
        self._set_socket_default(_input_socket(shader_group, "Base Color"), (0.5, 0.5, 0.5, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Emission Strength"), 3.0)
        self._set_socket_default(_input_socket(shader_group, "Mix Factor"), 0.12)
        self._set_socket_default(_input_socket(shader_group, "Use Checker"), 0.0)

        image_path = representative_textures(submaterial)["base_color"]
        color_source = self._color_source_socket(nodes, submaterial, palette, image_path, x=0, y=0)
        if color_source is not None:
            self._link_group_input(links, color_source, shader_group, "Base Color")
        elif has_virtual_input(submaterial, "$RenderToTexture"):
            self._set_socket_default(_input_socket(shader_group, "Use Checker"), 1.0)

        surface = _output_socket(shader_group, "Shader")
        if surface is not None:
            links.new(surface, output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_effect_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (550, 0)

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_effect_group()
        _refresh_group_node_sockets(shader_group)
        shader_group.location = (250, 0)
        shader_group.label = "StarBreaker Effect"
        self._set_socket_default(_input_socket(shader_group, "Base Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Emission Strength"), 2.5)
        self._set_socket_default(_input_socket(shader_group, "Mix Factor"), 0.35)

        color_source = self._color_source_socket(nodes, submaterial, palette, representative_textures(submaterial)["base_color"], x=0, y=0)
        if color_source is not None:
            self._link_group_input(links, color_source, shader_group, "Base Color")

        surface = _output_socket(shader_group, "Shader")
        if surface is not None:
            links.new(surface, output.inputs[0])
        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_layered_wear_principled_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        """Clean-top-level layered wear Principled builder.

        Top level is restricted to Material Output, Palette group nodes,
        Image Texture nodes, Wear Input helper group, LayeredInputs helper
        group, and Principled shader group. All BSDF/NormalMap/Bump/MixRGB/
        Mix/RGB nodes that the legacy ``_build_principled_material`` emitted
        at the material top level live inside the two new shader groups;
        per-layer tint, shadowless / emission / alpha flags, and roughness
        defaults are seeded as group-input socket defaults.

        Residual top-level helpers (``SeparateColor`` from the metallic-
        roughness split in ``_roughness_group_source_socket`` and the
        ``Math`` invert in ``_layer_roughness_socket``) are intentionally
        left in place and covered by the deferred LayerSurface detail-channel
        refactor.
        """
        nodes = material.node_tree.nodes
        links = material.node_tree.links

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)

        principled_group = nodes.new("ShaderNodeGroup")
        principled_group.node_tree = self._ensure_runtime_principled_group()
        _refresh_group_node_sockets(principled_group)
        principled_group.location = (420, 0)
        principled_group.label = "StarBreaker Principled"

        layered_group = nodes.new("ShaderNodeGroup")
        layered_group.node_tree = self._ensure_runtime_layered_inputs_group()
        _refresh_group_node_sockets(layered_group)
        layered_group.location = (120, 0)
        layered_group.label = "StarBreaker LayeredInputs"

        textures = representative_textures(submaterial)

        # Base image (primary diffuse).
        base_image_node = self._image_node(
            nodes, textures["base_color"], x=-280, y=220, is_color=True
        )
        if base_image_node is not None:
            base_image_socket = _input_socket(layered_group, "Base Image")
            if base_image_socket is not None:
                links.new(base_image_node.outputs[0], base_image_socket)

        # Base palette channel (optional).
        channels = material_palette_channels(submaterial)
        active_channel = submaterial.palette_routing.material_channel or (
            channels[0] if channels else None
        )
        if active_channel is not None and palette is not None:
            base_palette_socket = self._palette_color_socket(
                nodes, palette, active_channel.name, x=-280, y=40
            )
            if base_palette_socket is not None:
                target = _input_socket(layered_group, "Base Palette")
                if target is not None:
                    self._link_color_output(base_palette_socket, target)

        # Wear layer (tint + palette + diffuse).
        wear_layer = self._layered_wear_layer(submaterial)
        if wear_layer is None:
            wear_layer = next(
                (layer for layer in submaterial.layer_manifest if layer.diffuse_export_path),
                None,
            )
        if wear_layer is not None:
            if wear_layer.diffuse_export_path:
                layer_image_node = self._image_node(
                    nodes, wear_layer.diffuse_export_path, x=-280, y=-140, is_color=True
                )
                if layer_image_node is not None:
                    target = _input_socket(layered_group, "Layer Image")
                    if target is not None:
                        links.new(layer_image_node.outputs[0], target)
            if wear_layer.tint_color is not None and any(
                abs(channel - 1.0) > 1e-6 for channel in wear_layer.tint_color
            ):
                tint_socket = _input_socket(layered_group, "Layer Tint")
                if tint_socket is not None:
                    tint_socket.default_value = (*wear_layer.tint_color, 1.0)
            if wear_layer.palette_channel is not None and palette is not None:
                layer_palette_socket = self._palette_color_socket(
                    nodes, palette, wear_layer.palette_channel.name, x=-280, y=-320
                )
                if layer_palette_socket is not None:
                    target = _input_socket(layered_group, "Layer Palette")
                    if target is not None:
                        self._link_color_output(layer_palette_socket, target)

        # Wear factor (Wear Input helper group — already wrapped).
        wear_factor_socket = self._layered_wear_factor_socket(
            nodes, links, submaterial, x=-60, y=-460
        )
        if wear_factor_socket is not None:
            target = _input_socket(layered_group, "Wear Factor")
            if target is not None:
                links.new(wear_factor_socket, target)

        # Roughness (base + wear layer).
        base_roughness_source = self._roughness_group_source_socket(
            nodes, submaterial, textures["roughness"], x=-280, y=-620
        )
        base_roughness_target = _input_socket(layered_group, "Base Roughness")
        if base_roughness_source is not None and base_roughness_target is not None:
            links.new(base_roughness_source, base_roughness_target)

        layer_roughness_source = self._layer_roughness_socket(
            nodes, submaterial, x=-280, y=-780
        )
        layer_roughness_target = _input_socket(layered_group, "Layer Roughness")
        if layer_roughness_source is not None and layer_roughness_target is not None:
            links.new(layer_roughness_source, layer_roughness_target)

        # LayeredInputs outputs → Principled group inputs.
        color_output = _output_socket(layered_group, "Color")
        roughness_output = _output_socket(layered_group, "Roughness")
        if color_output is not None:
            target = _input_socket(principled_group, "Base Color")
            if target is not None:
                links.new(color_output, target)
        if roughness_output is not None:
            target = _input_socket(principled_group, "Roughness")
            if target is not None:
                links.new(roughness_output, target)

        # Normal map.
        normal_path = textures["normal"]
        if normal_path:
            normal_node = self._image_node(
                nodes, normal_path, x=-280, y=-940, is_color=False
            )
            if normal_node is not None:
                target = _input_socket(principled_group, "Normal Color")
                if target is not None:
                    links.new(normal_node.outputs[0], target)
                use_normal = _input_socket(principled_group, "Use Normal")
                if use_normal is not None:
                    use_normal.default_value = 1.0

        # Height / bump.
        height_path = textures["height"]
        if height_path:
            height_node = self._image_node(
                nodes, height_path, x=-280, y=-1100, is_color=False
            )
            if height_node is not None:
                target = _input_socket(principled_group, "Height")
                if target is not None:
                    links.new(height_node.outputs[0], target)
                use_bump = _input_socket(principled_group, "Use Bump")
                if use_bump is not None:
                    use_bump.default_value = 1.0

        # Alpha.
        if plan.uses_alpha:
            alpha_source = self._alpha_source_socket(
                nodes, submaterial, textures, x=-280, y=-1260
            )
            if alpha_source is not None:
                target = _input_socket(principled_group, "Alpha")
                if target is not None:
                    links.new(alpha_source, target)

        # Emission.
        if plan.uses_emission:
            strength_socket = _input_socket(principled_group, "Emission Strength")
            if strength_socket is not None:
                strength_socket.default_value = 2.0
            if color_output is not None:
                target = _input_socket(principled_group, "Emission Color")
                if target is not None:
                    links.new(color_output, target)
            elif palette is not None and plan.uses_palette:
                emissive = self._palette_color_socket(
                    nodes, palette, "primary", x=-280, y=360
                )
                if emissive is not None:
                    target = _input_socket(principled_group, "Emission Color")
                    if target is not None:
                        self._link_color_output(emissive, target)

        # Shadowless.
        if self._plan_casts_no_shadows(plan):
            shadow_socket = _input_socket(principled_group, "Shadowless")
            if shadow_socket is not None:
                shadow_socket.default_value = 1.0

        shader_out = _output_socket(principled_group, "Shader")
        if shader_out is not None:
            links.new(shader_out, output.inputs[0])

        self._configure_material(
            material, blend_method=plan.blend_method, shadow_method=plan.shadow_method
        )

    def _build_principled_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        if plan.template_key == "layered_wear":
            self._build_layered_wear_principled_material(material, submaterial, palette, plan)
            return

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (700, 0)
        principled = self._create_surface_bsdf(nodes)
        surface_shader = principled.outputs[0]

        textures = representative_textures(submaterial)
        base_socket = self._color_source_socket(nodes, submaterial, palette, textures["base_color"], x=40, y=140)
        if base_socket is None and palette is not None and plan.uses_palette:
            primary = self._palette_color_socket(nodes, palette, "primary", x=80, y=120)
            base_socket = primary

        if base_socket is not None:
            links.new(base_socket, _input_socket(principled, "Base Color"))

        if plan.uses_alpha:
            alpha_socket = _input_socket(principled, "Alpha")
            alpha_source = self._alpha_source_socket(nodes, submaterial, textures, x=80, y=20)
            if alpha_socket is not None:
                if alpha_source is not None:
                    links.new(alpha_source, alpha_socket)
                elif plan.template_key == "hair":
                    alpha_socket.default_value = 0.85

        roughness_socket = _input_socket(principled, "Roughness")
        roughness_default = 0.45 if submaterial.shader_family != "GlassPBR" else 0.08
        roughness_source = self._roughness_group_source_socket(
            nodes,
            submaterial,
            textures["roughness"],
            x=80,
            y=-120,
        )
        if roughness_socket is not None:
            if roughness_source is not None:
                links.new(roughness_source, roughness_socket)
            else:
                roughness_socket.default_value = roughness_default

        normal_input = _input_socket(principled, "Normal")
        normal_node = self._image_node(nodes, textures["normal"], x=80, y=-280, is_color=False)
        bump_node = None
        if textures["height"] or plan.template_key == "parallax_pom":
            bump_node = nodes.new("ShaderNodeBump")
            bump_node.location = (240, -320)
            bump_input = _input_socket(bump_node, "Height")
            bump_input.default_value = 0.02
            height_node = self._image_node(nodes, textures["height"] or textures["mask"], x=40, y=-420, is_color=False)
            if height_node is not None:
                links.new(height_node.outputs[0], bump_input)
        if normal_node is not None:
            normal_map = nodes.new("ShaderNodeNormalMap")
            normal_map.location = (240, -220)
            links.new(normal_node.outputs[0], _input_socket(normal_map, "Color"))
            if bump_node is not None:
                links.new(_output_socket(normal_map, "Normal"), _input_socket(bump_node, "Normal"))
            elif normal_input is not None:
                links.new(_output_socket(normal_map, "Normal"), normal_input)
        if bump_node is not None and normal_input is not None:
            links.new(_output_socket(bump_node, "Normal"), normal_input)

        if plan.uses_transmission:
            transmission = _input_socket(principled, "Transmission Weight", "Transmission")
            if transmission is not None:
                transmission.default_value = 1.0
            ior_socket = _input_socket(principled, "IOR")
            if ior_socket is not None:
                ior_socket.default_value = 1.45
            alpha_socket = _input_socket(principled, "Alpha")
            if alpha_socket is not None:
                alpha_socket.default_value = 0.2

        if plan.uses_emission:
            emission_color = _input_socket(principled, "Emission Color", "Emission")
            if emission_color is not None:
                if base_socket is not None:
                    links.new(base_socket, emission_color)
                elif palette is not None and plan.uses_palette:
                    emissive = self._palette_color_socket(nodes, palette, "primary", x=80, y=300)
                    links.new(emissive, emission_color)
            emission_strength = _input_socket(principled, "Emission Strength")
            if emission_strength is not None:
                emission_strength.default_value = 2.0

        if plan.template_key == "biological":
            subsurface = _input_socket(principled, "Subsurface Weight", "Subsurface")
            if subsurface is not None:
                subsurface.default_value = 0.15

        if plan.template_key == "hair":
            anisotropic = _input_socket(principled, "Anisotropic")
            if anisotropic is not None:
                anisotropic.default_value = 0.4

        if self._plan_casts_no_shadows(plan):
            surface_shader = self._shadowless_surface_output(nodes, links, surface_shader)

        links.new(surface_shader, output.inputs[0])

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _build_glass_material(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        plan: Any,
    ) -> None:
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        nodes.clear()

        output = nodes.new("ShaderNodeOutputMaterial")
        output.location = (620, 0)

        shader_group = nodes.new("ShaderNodeGroup")
        shader_group.node_tree = self._ensure_runtime_glass_group()
        _refresh_group_node_sockets(shader_group)
        shader_group.location = (360, 0)
        shader_group.label = "StarBreaker Glass"
        self._set_socket_default(_input_socket(shader_group, "Base Color"), (1.0, 1.0, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Roughness"), 0.08)
        self._set_socket_default(_input_socket(shader_group, "IOR"), 1.05)
        self._set_socket_default(_input_socket(shader_group, "Normal Color"), (0.5, 0.5, 1.0, 1.0))
        self._set_socket_default(_input_socket(shader_group, "Normal Strength"), 0.25)
        self._set_socket_default(_input_socket(shader_group, "Use Normal"), 0.0)

        textures = representative_textures(submaterial)
        base_path = textures["base_color"]
        roughness_path = textures["roughness"] or self._texture_export_path(submaterial, "wear_gloss")
        normal_path = textures["normal"]

        base_socket = self._color_source_socket(nodes, submaterial, palette, base_path, x=40, y=140)
        if base_socket is None and palette is not None:
            base_socket = self._palette_color_socket(nodes, palette, "glass", x=80, y=120)
        if base_socket is None:
            base_socket = self._value_color_socket(nodes, (1.0, 1.0, 1.0, 1.0), x=80, y=120)
        if base_socket is not None:
            self._link_group_input(links, base_socket, shader_group, "Base Color")

        roughness_node = self._image_node(nodes, roughness_path, x=80, y=-120, is_color=False)
        if roughness_node is not None:
            self._link_group_input(links, roughness_node.outputs[0], shader_group, "Roughness")

        normal_node = self._image_node(nodes, normal_path, x=80, y=-280, is_color=False)
        if normal_node is not None:
            self._link_group_input(links, normal_node.outputs[0], shader_group, "Normal Color")
            self._set_socket_default(_input_socket(shader_group, "Use Normal"), 1.0)

        surface = _output_socket(shader_group, "Shader")
        if surface is not None:
            links.new(surface, output.inputs[0])

        self._configure_material(material, blend_method=plan.blend_method, shadow_method=plan.shadow_method)

    def _create_surface_bsdf(self, nodes: bpy.types.Nodes) -> bpy.types.ShaderNodeBsdfPrincipled:
        principled = nodes.new("ShaderNodeBsdfPrincipled")
        principled.location = (420, 0)
        principled.label = "StarBreaker Surface"
        return principled

    def _wear_strength(self) -> float:
        raw_value = getattr(self.context.scene, SCENE_WEAR_STRENGTH_PROP, 1.0)
        try:
            value = float(raw_value)
        except (TypeError, ValueError):
            value = 1.0
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
        self._set_socket_default(_input_socket(group_node, "Use Vertex Colors"), 1.0 if has_vertex_colors else 0.0)
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
        if "USE_DAMAGE_MAP" not in submaterial.decoded_feature_flags.tokens:
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

    def _layered_wear_layer(self, submaterial: SubmaterialRecord) -> LayerManifestEntry | None:
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
        layer = wear_layer if wear_layer is not None and wear_layer.diffuse_export_path else None
        if layer is None:
            layer = next((item for item in submaterial.layer_manifest if item.diffuse_export_path), None)
        if layer is None:
            return None

        source = self._image_node(nodes, layer.diffuse_export_path, x=x, y=y, is_color=True)
        if source is None:
            return None
        output = source.outputs[0]

        if layer.tint_color is not None and any(abs(channel - 1.0) > 1e-6 for channel in layer.tint_color):
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
            palette_socket = self._palette_color_socket(nodes, palette, layer.palette_channel.name, x=x, y=y - 320)
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
            or any(texture.alpha_semantic == "smoothness" and texture.export_path for texture in wear_layer.texture_slots)
        ):
            layer = wear_layer
        if layer is None:
            layer = next(
                (
                    item
                    for item in submaterial.layer_manifest
                    if item.roughness_export_path
                    or any(texture.alpha_semantic == "smoothness" and texture.export_path for texture in item.texture_slots)
                ),
                None,
            )
        if layer is None:
            return None
        if layer.roughness_export_path:
            image_node = self._image_node(nodes, layer.roughness_export_path, x=x, y=y, is_color=False)
            if image_node is not None:
                return image_node.outputs[0]
        smoothness_texture = next(
            (texture for texture in layer.texture_slots if texture.alpha_semantic == "smoothness" and texture.export_path),
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

    def _value_socket(self, nodes: bpy.types.Nodes, value: float, *, x: int, y: int) -> Any:
        node = nodes.new("ShaderNodeValue")
        node.location = (x, y)
        node.outputs[0].default_value = value
        return node.outputs[0]

    def _value_color_socket(self, nodes: bpy.types.Nodes, value: tuple[float, float, float, float], *, x: int, y: int) -> Any:
        node = nodes.new("ShaderNodeRGB")
        node.location = (x, y)
        node.outputs[0].default_value = value
        return node.outputs[0]

    def _texture_export_path(self, submaterial: SubmaterialRecord, *roles: str) -> str | None:
        for texture in [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures]:
            if texture.role in roles and texture.export_path:
                return texture.export_path
        return None

    def _alpha_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        textures: dict[str, str | None],
        *,
        x: int,
        y: int,
    ) -> Any:
        opacity_path = textures.get("opacity")
        if opacity_path:
            opacity_node = self._image_node(nodes, opacity_path, x=x, y=y, is_color=False)
            if opacity_node is not None:
                return opacity_node.outputs[0]

        alpha_image_path = (
            textures.get("base_color")
            or self._texture_export_path(submaterial, "decal_sheet", "diffuse", "alternate_base_color")
        )
        alpha_node = self._image_node(nodes, alpha_image_path, x=x, y=y, is_color=True)
        if alpha_node is None:
            return None
        return _output_socket(alpha_node, "Alpha")

    def _illum_emission_strength(self, submaterial: SubmaterialRecord) -> float:
        glow_value = _float_authored_attribute(submaterial, "Glow")
        if glow_value > 0.0:
            return glow_value

        if self._texture_export_path(submaterial, "emissive"):
            return 1.0

        material_name = " ".join(
            part.lower()
            for part in (submaterial.submaterial_name, submaterial.blender_material_name)
            if part
        )
        if "glow" in material_name or "emissive" in material_name:
            return 0.35
        return 0.0

    def _color_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        image_path: str | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        image_node = self._image_node(nodes, image_path, x=x, y=y, is_color=True)
        channels = material_palette_channels(submaterial)
        active_channel = submaterial.palette_routing.material_channel or (channels[0] if channels else None)

        if image_node is None and active_channel is None:
            return None
        if active_channel is None or palette is None:
            return image_node.outputs[0] if image_node is not None else None

        palette_socket = self._palette_color_socket(nodes, palette, active_channel.name, x=x, y=y - 180)
        if image_node is None:
            return palette_socket

        mix = nodes.new("ShaderNodeMixRGB")
        mix.location = (x + 180, y)
        mix.blend_type = "MULTIPLY"
        mix.inputs[0].default_value = 1.0
        mix.inputs[1].default_value = (1.0, 1.0, 1.0, 1.0)
        self._link_color_output(image_node.outputs[0], mix.inputs[1])
        self._link_color_output(palette_socket, mix.inputs[2])
        return mix.outputs[0]

    def _image_node(
        self,
        nodes: bpy.types.Nodes,
        image_path: str | None,
        *,
        x: int,
        y: int,
        is_color: bool,
        reuse_any_existing: bool = False,
    ) -> bpy.types.ShaderNodeTexImage | None:
        resolved = self.package.resolve_path(image_path)
        if resolved is None or not resolved.is_file():
            return None
        resolved_str = str(resolved)
        for existing in nodes:
            if existing.bl_idname != "ShaderNodeTexImage":
                continue
            image = getattr(existing, "image", None)
            if image is None:
                continue
            if bpy.path.abspath(image.filepath, library=image.library) != resolved_str:
                continue
            if reuse_any_existing:
                existing.location = (x, y)
                return existing
            color_space = getattr(getattr(image, "colorspace_settings", None), "name", "")
            if is_color and color_space != "Non-Color":
                existing.location = (x, y)
                return existing
            if not is_color and color_space == "Non-Color":
                existing.location = (x, y)
                return existing
        node = nodes.new("ShaderNodeTexImage")
        node.location = (x, y)
        node.image = bpy.data.images.load(str(resolved), check_existing=True)
        if not is_color and node.image is not None and hasattr(node.image, "colorspace_settings"):
            node.image.colorspace_settings.name = "Non-Color"
        return node

    def _apply_material_node_layout(self, material: bpy.types.Material) -> None:
        node_tree = material.node_tree
        if node_tree is None:
            return

        nodes = node_tree.nodes
        links = node_tree.links
        layout = MATERIAL_NODE_LAYOUT

        output = next((node for node in nodes if node.bl_idname == "ShaderNodeOutputMaterial"), None)
        if output is not None:
            output.location = (layout.output_x, layout.output_y)

        primary_node = self._primary_surface_node(nodes, links, output)
        if primary_node is not None:
            primary_node.location = (layout.primary_x, layout.primary_y)
            if primary_node.bl_idname == "ShaderNodeGroup":
                primary_node.width = layout.group_width

        shadow_mix = next((node for node in nodes if node.bl_idname == "ShaderNodeMixShader" and node != primary_node), None)
        if shadow_mix is not None:
            shadow_mix.location = (layout.shadow_mix_x, layout.shadow_mix_y)

        shadow_transparent = next((node for node in nodes if node.bl_idname == "ShaderNodeBsdfTransparent"), None)
        if shadow_transparent is not None:
            shadow_transparent.location = (layout.shadow_transparent_x, layout.shadow_transparent_y)

        shadow_light_path = next((node for node in nodes if node.bl_idname == "ShaderNodeLightPath"), None)
        if shadow_light_path is not None:
            shadow_light_path.location = (layout.shadow_light_path_x, layout.shadow_light_path_y)

        texture_nodes = [node for node in nodes if node.bl_idname == "ShaderNodeTexImage"]
        texture_nodes.sort(key=lambda node: (float(node.location.y), node.name), reverse=True)
        next_y = layout.texture_start_y
        for node in texture_nodes:
            node.location = (layout.texture_x, next_y)
            node.width = layout.texture_width
            next_y -= layout.texture_vertical_step

        palette_groups = [
            node
            for node in nodes
            if node.bl_idname == "ShaderNodeGroup"
            and node != primary_node
            and getattr(getattr(node, "node_tree", None), "name", "").startswith("StarBreaker Palette ")
        ]
        palette_groups.sort(key=lambda node: node.name)
        palette_y = 120.0
        for node in palette_groups:
            node.location = (layout.primary_x - 620.0, palette_y)
            node.width = 240.0
            palette_y -= 220.0

        layer_groups = [
            node
            for node in nodes
            if node.bl_idname == "ShaderNodeGroup"
            and node != primary_node
            and getattr(getattr(node, "node_tree", None), "name", "").startswith("StarBreaker Runtime LayerSurface")
        ]
        layer_groups.sort(key=lambda node: float(node.location.y), reverse=True)
        layer_y = 80.0
        for node in layer_groups:
            node.location = (layout.primary_x - 300.0, layer_y)
            node.width = 320.0
            layer_y -= 240.0

    def _primary_surface_node(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        output: bpy.types.Node | None,
    ) -> bpy.types.Node | None:
        if output is None:
            return None

        surface_input = _input_socket(output, "Surface")
        if surface_input is None or not surface_input.is_linked:
            return None

        primary_node = next(
            (link.from_node for link in links if link.to_node == output and link.to_socket == surface_input),
            None,
        )
        if primary_node is None:
            return None

        if self._is_shadow_wrapper_mix(primary_node):
            shader_input = primary_node.inputs[2]
            if shader_input.is_linked:
                return shader_input.links[0].from_node

        return primary_node

    def _is_shadow_wrapper_mix(self, node: bpy.types.Node | None) -> bool:
        if node is None or node.bl_idname != "ShaderNodeMixShader":
            return False

        factor_input = node.inputs[0]
        transparent_input = node.inputs[1]
        shader_input = node.inputs[2]
        if not factor_input.is_linked or not transparent_input.is_linked or not shader_input.is_linked:
            return False

        return (
            factor_input.links[0].from_node.bl_idname == "ShaderNodeLightPath"
            and transparent_input.links[0].from_node.bl_idname == "ShaderNodeBsdfTransparent"
        )

    def _plan_casts_no_shadows(self, plan: Any, submaterial: SubmaterialRecord | None = None) -> bool:
        if getattr(plan, "template_key", "") in {"decal_stencil", "parallax_pom"}:
            return True
        return submaterial is not None and submaterial.shader_family == "MeshDecal"

    def _shadowless_surface_output(
        self,
        nodes: bpy.types.Nodes,
        links: bpy.types.NodeLinks,
        surface_shader: Any,
    ) -> Any:
        light_path = nodes.new("ShaderNodeLightPath")
        transparent = nodes.new("ShaderNodeBsdfTransparent")
        mix = nodes.new("ShaderNodeMixShader")
        shadow_ray = _output_socket(light_path, "Is Shadow Ray")
        if shadow_ray is None:
            return surface_shader
        links.new(shadow_ray, mix.inputs[0])
        links.new(surface_shader, mix.inputs[1])
        links.new(transparent.outputs[0], mix.inputs[2])
        return mix.outputs[0]

    def _configure_material(self, material: bpy.types.Material, *, blend_method: str, shadow_method: str) -> None:
        if hasattr(material, "blend_method"):
            material.blend_method = blend_method
        if hasattr(material, "shadow_method"):
            material.shadow_method = shadow_method
        material.use_backface_culling = False

    def _apply_instance_metadata(
        self,
        objects: list[bpy.types.Object],
        record: SceneInstanceRecord,
        effective_palette_id: str | None,
    ) -> None:
        effective_material_sidecar = self._effective_import_material_sidecar(record.material_sidecar)
        serialized = json.dumps(record.raw or {
            "entity_name": record.entity_name,
            "mesh_asset": record.mesh_asset,
            "material_sidecar": effective_material_sidecar,
            "palette_id": record.palette_id,
        }, sort_keys=True)
        for obj in objects:
            obj[PROP_SCENE_PATH] = str(self.package.scene_path)
            obj[PROP_EXPORT_ROOT] = str(self.package.export_root)
            obj[PROP_PACKAGE_NAME] = self.package.package_name
            obj[PROP_ENTITY_NAME] = record.entity_name
            if record.mesh_asset is not None:
                obj[PROP_MESH_ASSET] = record.mesh_asset
            if effective_material_sidecar is not None:
                obj[PROP_MATERIAL_SIDECAR] = effective_material_sidecar
            if effective_palette_id is not None:
                obj[PROP_PALETTE_ID] = effective_palette_id
            obj[PROP_INSTANCE_JSON] = serialized

    def _create_package_root(self, palette_id: str | None = None) -> bpy.types.Object:
        package_root = bpy.data.objects.new(f"{PACKAGE_ROOT_PREFIX} {self.package.package_name}", None)
        package_root.empty_display_type = "ARROWS"
        package_root[PROP_PACKAGE_ROOT] = True
        package_root[PROP_SCENE_PATH] = str(self.package.scene_path)
        package_root[PROP_EXPORT_ROOT] = str(self.package.export_root)
        package_root[PROP_PACKAGE_NAME] = self.package.package_name
        package_root[PROP_PALETTE_ID] = palette_id or self.package.scene.root_entity.palette_id or ""
        package_root[PROP_PALETTE_SCOPE] = uuid.uuid4().hex
        self.collection.objects.link(package_root)
        return package_root

    def _ensure_collection(self, package_name: str) -> bpy.types.Collection:
        collection_name = f"StarBreaker {package_name}"
        collection = bpy.data.collections.get(collection_name)
        if collection is None:
            collection = bpy.data.collections.new(collection_name)
            self.context.scene.collection.children.link(collection)
        return collection

    def _ensure_template_collection(self) -> bpy.types.Collection:
        collection = bpy.data.collections.get(TEMPLATE_COLLECTION_NAME)
        if collection is None:
            collection = bpy.data.collections.new(TEMPLATE_COLLECTION_NAME)
            self.context.scene.collection.children.link(collection)
        collection.hide_viewport = True
        collection.hide_render = True
        return collection

    def _index_nodes(self, objects: list[bpy.types.Object]) -> dict[str, bpy.types.Object]:
        indexed: dict[str, bpy.types.Object] = {}
        for obj in objects:
            source_name = obj.get(PROP_SOURCE_NODE_NAME, obj.name)
            source_name_str = str(source_name)
            indexed[source_name_str] = obj
            indexed[_canonical_source_name(source_name_str)] = obj
        return indexed

    def _scene_root_parent(self, objects: list[bpy.types.Object]) -> bpy.types.Object | None:
        indexed = self._index_nodes(objects)
        return indexed.get("CryEngine_Z_up")

    def _clear_template_material_bindings(self, objects: list[bpy.types.Object]) -> None:
        seen_meshes: set[int] = set()
        for obj in objects:
            if obj.type != "MESH" or obj.data is None:
                continue
            pointer = obj.data.as_pointer()
            if pointer in seen_meshes:
                continue
            seen_meshes.add(pointer)
            materials = getattr(obj.data, "materials", None)
            if materials is None:
                continue
            slot_mapping = _imported_slot_mapping_from_materials(materials)
            if slot_mapping is not None:
                obj.data[PROP_IMPORTED_SLOT_MAP] = json.dumps(slot_mapping)
            for index in range(len(materials)):
                materials[index] = None

    def _purge_unused_materials(self, materials: list[bpy.types.Material]) -> None:
        for material in materials:
            if material.users == 0:
                bpy.data.materials.remove(material)

    def _root_objects(self, objects: list[bpy.types.Object]) -> list[bpy.types.Object]:
        imported_pointers = {obj.as_pointer() for obj in objects}
        return [obj for obj in objects if obj.parent is None or obj.parent.as_pointer() not in imported_pointers]

    def _link_color_output(self, output: Any, input_socket: Any) -> None:
        output.node.id_data.links.new(output, input_socket)

    def _ensure_tint_decal_adaptor_group(self) -> bpy.types.ShaderNodeTree:
        group_name = "SB_Tint_Decal_Adaptor"
        group = bpy.data.node_groups.get(group_name)
        if group is not None:
            existing_inputs = {
                item.name
                for item in group.interface.items_tree
                if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "INPUT"
            }
            existing_outputs = {
                item.name
                for item in group.interface.items_tree
                if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "OUTPUT"
            }
            expected_inputs = {"Image", "Decal Red Tint", "Decal Green Tint", "Decal Blue Tint"}
            expected_outputs = {"Color", "Alpha"}
            if expected_inputs.issubset(existing_inputs) and expected_outputs.issubset(existing_outputs):
                return group
        if group is None:
            group = bpy.data.node_groups.new(group_name, "ShaderNodeTree")
        for item in list(group.interface.items_tree):
            group.interface.remove(item)
        group.nodes.clear()
        group.interface.new_socket(name="Image", in_out="INPUT", socket_type="NodeSocketColor")
        for tint_name in ("Decal Red Tint", "Decal Green Tint", "Decal Blue Tint"):
            sock = group.interface.new_socket(name=tint_name, in_out="INPUT", socket_type="NodeSocketColor")
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
        group.links.new(_output_socket(separate_hsv, "Value", "V", "Blue"), group_output.inputs["Alpha"])

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
            if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "OUTPUT"
        }
        existing_inputs = {
            item.name
            for item in group.interface.items_tree
            if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "INPUT"
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
        }
        _stale_outputs = {"Palette Decal Color", "Palette Decal Alpha", "Iridescence Facing Color", "Iridescence Grazing Color", "Iridescence Strength"}
        if group.get("starbreaker_palette_signature") == group_signature and expected_inputs.issubset(existing_inputs) and expected_outputs.issubset(existing_outputs) and not _stale_outputs.intersection(existing_outputs):
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
        )
        for socket_name in (
            "Decal Color",
            "Decal Alpha",
        ):
            if socket_name not in existing_outputs:
                group.interface.new_socket(
                    name=socket_name,
                    in_out="OUTPUT",
                    socket_type="NodeSocketFloat" if socket_name.endswith("Alpha") else "NodeSocketColor",
                )
        for socket_name, _channel_name, _y in channel_specs:
            if socket_name not in existing_outputs:
                socket_type = "NodeSocketFloat" if "Glossiness" in socket_name else "NodeSocketColor"
                group.interface.new_socket(name=socket_name, in_out="OUTPUT", socket_type=socket_type)

        for item in list(group.interface.items_tree):
            if getattr(item, "item_type", None) == "SOCKET" and getattr(item, "in_out", None) == "OUTPUT" and item.name in _stale_outputs:
                group.interface.remove(item)

        group.nodes.clear()

        group_input = group.nodes.new("NodeGroupInput")
        group_input.location = (-900, -120)
        output = group.nodes.new("NodeGroupOutput")
        output.location = (520, -120)

        primary_color = (*_palette_decal_or_fallback(palette, "red"), 1.0)
        secondary_color = (*_palette_decal_or_fallback(palette, "green"), 1.0)
        tertiary_color = (*_palette_decal_or_fallback(palette, "blue"), 1.0)

        palette_decal_node = self._image_node(group.nodes, palette_decal_texture(palette), x=-900, y=-520, is_color=True)
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
                value.outputs[0].default_value = palette_finish_glossiness(palette, channel_name) or 0.0
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
                and getattr(getattr(node, "node_tree", None), "name", "") == _palette_group_name(self.package.package_name, self._palette_scope())
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
    ) -> LayerSurfaceSockets:
        if palette is None:
            return LayerSurfaceSockets()
        group_node = self._palette_group_node(nodes, links, palette, x=x, y=y)
        return LayerSurfaceSockets(
            color=_output_socket(group_node, "Decal Color"),
            alpha=_output_socket(group_node, "Decal Alpha"),
        )


def _load_package_from_root(package_root: bpy.types.Object) -> PackageBundle:
    scene_path = _string_prop(package_root, PROP_SCENE_PATH)
    if scene_path is None:
        raise RuntimeError("Selected object is missing StarBreaker scene metadata")
    return PackageBundle.load(scene_path)


def _scene_instance_from_object(obj: bpy.types.Object) -> SceneInstanceRecord | None:
    payload = obj.get(PROP_INSTANCE_JSON)
    if not isinstance(payload, str):
        return None
    try:
        return SceneInstanceRecord.from_value(json.loads(payload))
    except (json.JSONDecodeError, ValueError, TypeError):
        return None


def _iter_package_objects(package_root: bpy.types.Object) -> list[bpy.types.Object]:
    return [package_root, *package_root.children_recursive]


def _string_prop(obj: bpy.types.ID, name: str) -> str | None:
    value = obj.get(name)
    if isinstance(value, str) and value:
        return value
    return None


def _mean_triplet(value: tuple[float, float, float] | None) -> float | None:
    if value is None:
        return None
    return sum(value) / 3.0


def _palette_decal_or_fallback(
    palette: PaletteRecord | None,
    decal_channel: str,
) -> tuple[float, float, float]:
    return palette_decal_color(palette, decal_channel) or (1.0, 1.0, 1.0)


def _palette_has_iridescence(palette: PaletteRecord | None) -> bool:
    """Return True when a palette's finish specular produces visible angle-shift iridescence.

    Requires significant chroma (saturation) on both the facing (secondary) and
    grazing (tertiary) specular channels, plus a measurable color distance between them.
    """
    if palette is None:
        return False
    facing = palette_finish_specular(palette, "secondary") or palette_color(palette, "secondary")
    grazing = palette_finish_specular(palette, "tertiary") or palette_color(palette, "tertiary")
    facing_chroma = max(facing) - min(facing)
    grazing_chroma = max(grazing) - min(grazing)
    if min(facing_chroma, grazing_chroma) < 0.10:
        return False
    color_distance = math.sqrt(
        (facing[0] - grazing[0]) ** 2
        + (facing[1] - grazing[1]) ** 2
        + (facing[2] - grazing[2]) ** 2
    )
    return color_distance >= 0.25


def _palette_group_signature(palette: PaletteRecord) -> str:
    payload = {
        'schema': 'palette_group_v4',
        'id': palette.id,
        'primary': palette_color(palette, 'primary'),
        'secondary': palette_color(palette, 'secondary'),
        'tertiary': palette_color(palette, 'tertiary'),
        'glass': palette_color(palette, 'glass'),
        'decal_red': palette_decal_color(palette, 'red'),
        'decal_green': palette_decal_color(palette, 'green'),
        'decal_blue': palette_decal_color(palette, 'blue'),
        'decal_texture': palette_decal_texture(palette),
        'primary_spec': palette_finish_specular(palette, 'primary'),
        'secondary_spec': palette_finish_specular(palette, 'secondary'),
        'tertiary_spec': palette_finish_specular(palette, 'tertiary'),
        'primary_gloss': palette_finish_glossiness(palette, 'primary'),
        'secondary_gloss': palette_finish_glossiness(palette, 'secondary'),
        'tertiary_gloss': palette_finish_glossiness(palette, 'tertiary'),
    }
    return hashlib.sha1(json.dumps(payload, sort_keys=True).encode('utf-8')).hexdigest()


def _hard_surface_angle_shift_enabled(submaterial: SubmaterialRecord) -> bool:
    if submaterial.decoded_feature_flags.has_iridescence:
        return True
    strength = _optional_float_public_param(submaterial, "IridescenceStrength")
    if strength is None or strength <= 0.0:
        return False
    thickness_u = _optional_float_public_param(submaterial, "IridescenceThicknessU")
    thickness_v = _optional_float_public_param(submaterial, "IridescenceThicknessV")
    has_thickness = (thickness_u is not None and thickness_u > 0.0) or (thickness_v is not None and thickness_v > 0.0)
    has_support_texture = any(texture.slot == "TexSlot10" and bool(texture.export_path) for texture in submaterial.texture_slots)
    return has_thickness or has_support_texture


def _triplet_from_value(value: Any) -> tuple[float, float, float] | None:
    if not isinstance(value, (list, tuple)) or len(value) < 3:
        return None
    try:
        return (float(value[0]), float(value[1]), float(value[2]))
    except (TypeError, ValueError):
        return None


def _triplet_from_string(value: Any) -> tuple[float, float, float] | None:
    if not isinstance(value, str):
        return None
    parts = [part.strip() for part in value.split(",")]
    if len(parts) < 3:
        return None
    try:
        return (float(parts[0]), float(parts[1]), float(parts[2]))
    except (TypeError, ValueError):
        return None


def _triplet_from_any(value: Any) -> tuple[float, float, float] | None:
    return _triplet_from_value(value) or _triplet_from_string(value)


def _optional_float_public_param(submaterial: SubmaterialRecord, *names: str) -> float | None:
    for name in names:
        value = submaterial.public_params.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return None


def _authored_attribute_string(submaterial: SubmaterialRecord, *names: str) -> str | None:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        value = attribute.get("value")
        if isinstance(value, str) and value:
            return value
    return None


def _uses_virtual_tint_palette_decal(submaterial: SubmaterialRecord) -> bool:
    texture = _submaterial_texture_reference(submaterial, slots=("TexSlot7",), roles=("tint_palette_decal",))
    return texture is not None and bool(texture.is_virtual)


def _is_virtual_tint_palette_stencil_decal(submaterial: SubmaterialRecord) -> bool:
    if submaterial.shader_family != "MeshDecal" or not _uses_virtual_tint_palette_decal(submaterial):
        return False
    string_gen_mask = (_authored_attribute_string(submaterial, "StringGenMask") or "").upper()
    if "STENCIL_MAP" in string_gen_mask:
        return True
    if any(
        name in submaterial.public_params
        for name in ("StencilOpacity", "StencilDiffuseBreakup", "StencilTiling", "StencilTintOverride")
    ):
        return True
    lowered_name = (submaterial.submaterial_name or "").lower()
    return "_stencil" in lowered_name or "branding" in lowered_name


def _routes_virtual_tint_palette_decal_to_decal_source(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot1" and (contract_input.semantic or contract_input.name).lower() == "decal_source"


def _suppresses_virtual_tint_palette_stencil_input(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot7" and (contract_input.semantic or contract_input.name).lower() in {
        "stencil_source",
        "stencil_source_alpha",
    }


def _routes_virtual_tint_palette_decal_alpha_to_decal_source(
    submaterial: SubmaterialRecord,
    contract_input: ContractInput,
) -> bool:
    if not _is_virtual_tint_palette_stencil_decal(submaterial):
        return False
    return contract_input.source_slot == "TexSlot1" and (contract_input.semantic or contract_input.name).lower() == "decal_source_alpha"


def _public_param_triplet(submaterial: SubmaterialRecord, *names: str) -> tuple[float, float, float] | None:
    for name in names:
        triplet = _triplet_from_any(submaterial.public_params.get(name))
        if triplet is not None:
            return triplet
    return None


def _authored_attribute_triplet(submaterial: SubmaterialRecord, *names: str) -> tuple[float, float, float] | None:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        triplet = _triplet_from_any(attribute.get("value"))
        if triplet is not None:
            return triplet
    return None


def _resolved_submaterial_palette_color(
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
) -> tuple[float, float, float] | None:
    if palette is None:
        return None
    channel = submaterial.palette_routing.material_channel
    if channel is not None:
        return palette_color(palette, channel.name)
    if submaterial.shader_family == "GlassPBR":
        return palette_color(palette, "glass")
    return None


def _matching_texture_reference(
    textures: list[TextureReference],
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    for texture in textures:
        if slots and texture.slot not in slots:
            continue
        if roles and texture.role not in roles:
            continue
        if alpha_semantic is not None and texture.alpha_semantic != alpha_semantic:
            continue
        if texture.export_path:
            return texture
    for texture in textures:
        if slots and texture.slot not in slots:
            continue
        if roles and texture.role not in roles:
            continue
        if alpha_semantic is not None and texture.alpha_semantic != alpha_semantic:
            continue
        return texture
    return None


def _submaterial_texture_reference(
    submaterial: SubmaterialRecord,
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    return _matching_texture_reference(
        [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures],
        slots=slots,
        roles=roles,
        alpha_semantic=alpha_semantic,
    )


def _layer_texture_reference(
    layer: LayerManifestEntry,
    *,
    slots: tuple[str, ...] = (),
    roles: tuple[str, ...] = (),
    alpha_semantic: str | None = None,
) -> TextureReference | None:
    return _matching_texture_reference(layer.texture_slots, slots=slots, roles=roles, alpha_semantic=alpha_semantic)


def _float_layer_public_param(layer: LayerManifestEntry, *names: str) -> float:
    wanted = set(names)
    for param in layer.resolved_material.get("authored_public_params", []):
        if param.get("name") not in wanted:
            continue
        try:
            return float(param.get("value"))
        except (TypeError, ValueError):
            continue
    return 0.0


def _layer_snapshot_triplet(layer: LayerManifestEntry, name: str) -> tuple[float, float, float] | None:
    return _triplet_from_value(layer.layer_snapshot.get(name))


def _layer_snapshot_float(layer: LayerManifestEntry, name: str) -> float:
    value = layer.layer_snapshot.get(name)
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def _float_public_param(submaterial: SubmaterialRecord, *names: str) -> float:
    for name in names:
        value = submaterial.public_params.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return 0.0


def _float_authored_attribute(submaterial: SubmaterialRecord, *names: str) -> float:
    wanted = set(names)
    for attribute in submaterial.raw.get("authored_attributes", []):
        if attribute.get("name") not in wanted:
            continue
        value = attribute.get("value")
        try:
            return float(value)
        except (TypeError, ValueError):
            continue
    return 0.0


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
    return _is_axis_conversion_root(obj) and _has_non_identity_descendants(obj, mesh_asset)


def _has_non_identity_descendants(root: bpy.types.Object, mesh_asset: str) -> bool:
    stack = list(root.children)
    while stack:
        current = stack.pop()
        if current.get(PROP_TEMPLATE_PATH) != mesh_asset:
            continue
        if not _matrix_is_identityish(current.matrix_local):
            return True
        stack.extend(current.children)
    return False


def _matrix_is_identityish(matrix: Matrix, epsilon: float = 1e-4) -> bool:
    identity = Matrix.Identity(4)
    for row_index in range(4):
        for column_index in range(4):
            if abs(matrix[row_index][column_index] - identity[row_index][column_index]) > epsilon:
                return False
    return True


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

    Used inside `_ensure_runtime_*_group` builders to seed identity defaults so
    callers may leave sockets unlinked without changing the composed behavior.
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


def _contract_input_uses_color(contract_input: ContractInput) -> bool:
    semantic = (contract_input.semantic or contract_input.name).lower()
    return not any(keyword in semantic for keyword in NON_COLOR_INPUT_KEYWORDS)
