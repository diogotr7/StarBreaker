"""Material lifecycle, low-level node helpers, and texture/socket utilities.

Extracted from ``runtime/_legacy.py``. Owns the bulk of ``PackageImporter``
helpers: ``material_for_submaterial`` + ``_reusable_material``, generic
node graph plumbing (mix/multiply/add color & value sockets, normal/bump,
image/tiled-image nodes, mask/alpha sockets), texture-reference lookups
(``_texture_path_for_slot``, ``_texture_reference_for_contract_input``,
``_smoothness_texture_reference``, ``_texture_alpha_socket``,
``_texture_export_path``, ``_texture_path_for_contract_input``),
contract-driven socket sources (``_contract_input_source_socket``,
``_roughness_group_source_socket``, ``_color_source_socket``,
``_alpha_source_socket``, ``_source_slot_alpha_socket``,
``_specular_socket_for_texture_path``, ``_roughness_socket_for_texture_reference``,
``_texture_reference_uses_packed_roughness_green``,
``_illum_emission_strength``, ``_create_surface_bsdf``,
``_value_socket``/``_value_color_socket``, ``_invert_value_socket``,
``_image_node``, ``_apply_material_node_layout``, ``_configure_material``,
``_link_color_output``, ``_clear_template_material_bindings``,
``_purge_unused_materials``, ``_set_socket_default``).

Cross-mixin calls (palette, decals, layers, builders, groups,
orchestration) resolve via composed MRO. Module-level helpers
(``_material_identity``, ``_material_name``, ``_material_is_compatible``,
``_derived_material_name``, ``_imported_slot_mapping_from_materials``,
``_contract_input_uses_color``) and the ``MATERIAL_NODE_LAYOUT`` constant
still living in ``_legacy.py`` are pulled in lazily via ``_legacy_attr``.
"""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any

import bpy

from ...manifest import (
    MaterialSidecar,
    PaletteRecord,
    SubmaterialRecord,
    TextureReference,
)
from ...material_contract import ContractInput, ShaderGroupContract
from ...templates import (
    material_palette_channels,
    representative_textures,
    smoothness_texture_reference,
    template_plan_for_submaterial,
)
from ..constants import (
    NON_COLOR_INPUT_KEYWORDS,
    PROP_ASSEMBLY_KIND,
    PROP_IMPORTED_SLOT_MAP,
    PROP_IMPORTED_SLOT_NAMES,
    PROP_MATERIAL_IDENTITY,
    PROP_TEMPLATE_KEY,
)
from ..node_utils import _input_socket, _output_socket, _refresh_group_node_sockets
from ..record_utils import (
    _float_authored_attribute,
    _uses_vertex_color_tint,
    _layer_texture_reference,
    _mesh_decal_authored_emission_strength,
    _matching_texture_reference,
    _mean_triplet,
    _public_param_triplet,
    _is_virtual_tint_palette_stencil_decal,
    _routes_virtual_tint_palette_decal_alpha_to_decal_source,
    _routes_virtual_tint_palette_decal_to_decal_source,
    _submaterial_texture_reference,
    _suppresses_virtual_tint_palette_stencil_input,
)
from .types import MATERIAL_NODE_LAYOUT, LayerSurfaceSockets
from .utils import (
    _contract_input_uses_color,
    _derived_material_name,
    _imported_slot_mapping_from_materials,
    _material_identity,
    _material_is_compatible,
    _material_name,
)


_MESH_DECAL_CONTROL_ONLY_POM_TEXTURE_INPUTS = {
    "TexSlot1": "TexSlot1_DecalSource",
    "TexSlot3": "TexSlot3_NormalGloss",
    "TexSlot4": "TexSlot4_Height",
}
_CONTROL_ONLY_POM_RELIEF_MODE = "control_only_pom_relief_v1"


def _material_datablock_is_valid(material: bpy.types.Material | None) -> bool:
    if material is None:
        return False
    try:
        material.name
    except ReferenceError:
        return False
    return True


def _illum_payload_is_local_opacity_decal(payload: dict[str, Any]) -> bool:
    if str(payload.get("shader_family", "")).strip() != "Illum":
        return False
    flags = payload.get("decoded_feature_flags") or {}
    tokens = {
        str(token).strip().upper()
        for token in flags.get("tokens", []) or []
        if str(token).strip()
    }
    if not bool(flags.get("has_decal")) or "DECAL_OPACITY_MAP" not in tokens:
        return False
    if bool(flags.get("has_parallax_occlusion_mapping")):
        return False
    virtual_inputs = {
        str(value).strip().lower()
        for value in payload.get("virtual_inputs", []) or []
        if str(value).strip()
    }
    if virtual_inputs.intersection({"$tintpalettedecal", "$rendertotexture"}):
        return False
    for texture in payload.get("texture_slots", []) or []:
        if not isinstance(texture, dict):
            continue
        role = str(texture.get("role", "")).strip().lower()
        if role == "tint_palette_decal" and bool(texture.get("is_virtual")):
            return False
    return True


def _mesh_decal_pom_payload_is_control_only(payload: dict[str, Any]) -> bool:
    if str(payload.get("shader_family", "")).strip() != "MeshDecal":
        return False
    flags = payload.get("decoded_feature_flags") or {}
    if not bool(flags.get("has_parallax_occlusion_mapping")):
        return False
    if bool(flags.get("has_decal")) or bool(flags.get("has_stencil_map")):
        return False
    tokens = {
        str(token).strip().upper()
        for token in flags.get("tokens", []) or []
        if str(token).strip()
    }
    if tokens.intersection({"DECAL", "STENCIL_MAP", "DECAL_OPACITY_MAP"}):
        return False
    visible_roles = {
        "decal_sheet",
        "opacity",
        "stencil",
        "tint_mask",
        "tint_palette_decal",
        "render_to_texture",
    }
    for texture in payload.get("texture_slots", []) or []:
        if not isinstance(texture, dict):
            continue
        role = str(texture.get("role", "")).strip().lower()
        if role in visible_roles:
            return False
        slot = str(texture.get("slot", "")).strip().lower()
        if slot in {"texslot5", "texslot6", "texslot7", "texslot8"}:
            return False
    virtual_inputs = {
        str(value).strip().lower()
        for value in payload.get("virtual_inputs", []) or []
        if str(value).strip()
    }
    return not virtual_inputs.intersection({"$tintpalettedecal", "$rendertotexture"})


def _texture_reference_uses_packed_roughness_green(texture: Any) -> bool:
    """Return true when a sidecar texture stores roughness in glTF MR green."""

    if texture is None:
        return False
    if getattr(texture, "role", None) != "roughness":
        return False
    if getattr(texture, "packed_texture_format", None) != "gltf_metallic_roughness":
        return False
    value_channel = getattr(texture, "value_channel", None)
    return value_channel is None or str(value_channel).lower() == "g"


class MaterialsMixin:
    """Material lifecycle + node/socket utilities for ``PackageImporter``."""

    _LOCALIZED_COPY_PROP = "starbreaker_localized_copy"

    def material_for_submaterial(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        ui_image_path: str | None = None,
    ) -> bpy.types.Material:
        palette_scope = self._palette_scope(palette)
        identity_cache = getattr(self, "material_identity_cache", None)
        expected_template_key = template_plan_for_submaterial(submaterial).template_key
        effective_ui_image_path = ui_image_path if expected_template_key == "screen_hud" else None
        identity_key = (
            sidecar_path,
            int(submaterial.index),
            palette_scope,
            effective_ui_image_path or "",
        )
        cache_key = identity_cache.get(identity_key) if identity_cache is not None else None
        if cache_key is None:
            cache_key = _material_identity(sidecar_path, sidecar, submaterial, palette, palette_scope)
            if effective_ui_image_path:
                cache_key = f"{cache_key}|ui:{effective_ui_image_path}"
            if identity_cache is not None:
                identity_cache[identity_key] = cache_key
        cached = self.material_cache.get(cache_key)
        if cached is not None:
            if _material_datablock_is_valid(cached):
                return cached
            self.material_cache.pop(cache_key, None)
            self.material_identity_index.pop(cache_key, None)

        reusable = self._reusable_material(sidecar_path, sidecar, submaterial, palette, palette_scope, cache_key)
        if reusable is not None:
            reusable, localized_copy = self._local_editable_material(reusable, cache_key)
            existing_identity = reusable.get(PROP_MATERIAL_IDENTITY)
            existing_template_key = reusable.get(PROP_TEMPLATE_KEY)
            if localized_copy:
                self._build_managed_material(
                    reusable,
                    sidecar_path,
                    sidecar,
                    submaterial,
                    palette,
                    cache_key,
                    ui_image_path=effective_ui_image_path,
                )
                try:
                    del reusable[self._LOCALIZED_COPY_PROP]
                except Exception:
                    pass
                self.material_cache[cache_key] = reusable
                self.material_identity_index[cache_key] = reusable
                return reusable
            if (
                isinstance(existing_identity, str)
                and existing_identity == cache_key
                and existing_template_key == expected_template_key
                and (
                    self._material_needs_mesh_decal_emission_refresh(reusable, submaterial)
                    or self._material_needs_illum_decal_alpha_mode_refresh(reusable, submaterial)
                )
            ):
                self._build_managed_material(
                    reusable,
                    sidecar_path,
                    sidecar,
                    submaterial,
                    palette,
                    cache_key,
                    ui_image_path=effective_ui_image_path,
                )
                self.material_cache[cache_key] = reusable
                self.material_identity_index[cache_key] = reusable
                return reusable
            if (
                isinstance(existing_identity, str)
                and existing_identity == cache_key
                and existing_template_key == expected_template_key
            ):
                self.material_cache[cache_key] = reusable
                self.material_identity_index[cache_key] = reusable
                return reusable
            if isinstance(existing_identity, str) and existing_identity != cache_key:
                self._build_managed_material(
                    reusable,
                    sidecar_path,
                    sidecar,
                    submaterial,
                    palette,
                    cache_key,
                    ui_image_path=effective_ui_image_path,
                )
                self.material_cache[cache_key] = reusable
                self.material_identity_index[cache_key] = reusable
                return reusable
            self._build_managed_material(
                reusable,
                sidecar_path,
                sidecar,
                submaterial,
                palette,
                cache_key,
                ui_image_path=effective_ui_image_path,
            )
            self.material_cache[cache_key] = reusable
            self.material_identity_index[cache_key] = reusable
            return reusable

        material_name = _material_name(sidecar_path, sidecar, submaterial, cache_key)
        material = bpy.data.materials.new(material_name)
        self._build_managed_material(
            material,
            sidecar_path,
            sidecar,
            submaterial,
            palette,
            cache_key,
            ui_image_path=effective_ui_image_path,
        )
        self.material_cache[cache_key] = material
        self.material_identity_index[cache_key] = material
        return material



    def _local_editable_material(
        self,
        material: bpy.types.Material,
        material_identity: str,
    ) -> tuple[bpy.types.Material, bool]:
        if getattr(material, "library", None) is None:
            return material, False

        self._ensure_material_identity_index()
        indexed_material = self.material_identity_index.get(material_identity)
        if indexed_material is not None and not _material_datablock_is_valid(indexed_material):
            self.material_identity_index.pop(material_identity, None)
            indexed_material = None
        if indexed_material is not None and getattr(indexed_material, "library", None) is None:
            return indexed_material, False

        for candidate in bpy.data.materials:
            if candidate is material:
                continue
            if getattr(candidate, "library", None) is not None:
                continue
            if candidate.get(PROP_MATERIAL_IDENTITY) == material_identity:
                self.material_identity_index[material_identity] = candidate
                return candidate, False

        local_material = material.copy()
        try:
            local_material.name = material.name
        except Exception:
            pass
        local_material[PROP_MATERIAL_IDENTITY] = material_identity
        local_material[self._LOCALIZED_COPY_PROP] = True
        self.material_identity_index[material_identity] = local_material
        return local_material, True



    def _material_needs_mesh_decal_emission_refresh(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
    ) -> bool:
        group_contract = self._group_contract_for_submaterial(submaterial)
        if group_contract is None or group_contract.name != "SB_MeshDecal_v1":
            return False

        node_tree = getattr(material, "node_tree", None)
        if node_tree is None:
            return True

        expected_use_vertex_colors = _uses_vertex_color_tint(submaterial)
        if expected_use_vertex_colors and not any(node.bl_idname == "ShaderNodeVertexColor" for node in node_tree.nodes):
            return True

        if any(node.bl_idname in {"ShaderNodeAddShader", "ShaderNodeEmission"} for node in node_tree.nodes):
            return True

        required_texture_inputs = (
            self._mesh_decal_pom_required_texture_inputs(submaterial)
            if self._package_uses_fps_weapon_pom_material_logic()
            else set()
        )
        if (
            required_texture_inputs
            and material.get("starbreaker_mesh_decal_material_mode") != _CONTROL_ONLY_POM_RELIEF_MODE
        ):
            return True

        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeGroup":
                continue
            group_tree = getattr(node, "node_tree", None)
            if group_tree is None or group_tree.name != "SB_MeshDecal_v1":
                continue
            if group_tree.get("starbreaker_mesh_decal_emission_patch_version") != 7:
                return True
            for input_name in required_texture_inputs:
                texture_socket = _input_socket(node, input_name)
                if texture_socket is None or not getattr(texture_socket, "links", ()):
                    return True
            emission_strength_socket = _input_socket(node, "Emission Strength")
            if emission_strength_socket is None:
                return True
            use_vert_col_socket = _input_socket(node, "Use Vert Col")
            if use_vert_col_socket is None:
                return True
            expected_emission_strength = self._mesh_decal_emission_strength(submaterial)
            if abs(float(getattr(emission_strength_socket, "default_value", 0.0)) - expected_emission_strength) > 1e-6:
                return True
            if bool(getattr(use_vert_col_socket, "default_value", False)) != expected_use_vertex_colors:
                return True
            return False
        return True


    def _package_uses_fps_weapon_pom_material_logic(self) -> bool:
        """Return whether FPS weapon-only MeshDecal POM refresh rules apply."""

        rebind_checker = getattr(self, "_package_uses_fps_weapon_pom_rebind", None)
        if callable(rebind_checker):
            try:
                return bool(rebind_checker())
            except TypeError:
                pass

        package = getattr(self, "package", None)
        scene = getattr(package, "scene", None)
        raw = getattr(scene, "raw", None)
        if isinstance(raw, dict):
            assembly_kind = str(raw.get("assembly_kind", "") or "").strip().lower()
            if assembly_kind:
                return assembly_kind == "fps_weapon"

        root = getattr(self, "package_root", None)
        if root is None or not hasattr(root, "get"):
            return False
        assembly_kind = str(root.get(PROP_ASSEMBLY_KIND, "") or "").strip().lower()
        return assembly_kind == "fps_weapon"


    @staticmethod
    def _mesh_decal_pom_required_texture_inputs(submaterial: SubmaterialRecord) -> set[str]:
        if submaterial.shader_family != "MeshDecal":
            return set()
        if not submaterial.decoded_feature_flags.has_parallax_occlusion_mapping:
            return set()
        raw = getattr(submaterial, "raw", None)
        if not isinstance(raw, dict) or not _mesh_decal_pom_payload_is_control_only(raw):
            return set()
        required: set[str] = set()
        for texture in submaterial.texture_slots:
            input_name = _MESH_DECAL_CONTROL_ONLY_POM_TEXTURE_INPUTS.get(texture.slot)
            if input_name is None:
                continue
            if not texture.export_path:
                continue
            if texture.is_virtual:
                continue
            required.add(input_name)
        return required


    @staticmethod
    def _mesh_decal_pom_requires_decal_source_texture(submaterial: SubmaterialRecord) -> bool:
        return "TexSlot1_DecalSource" in MaterialsMixin._mesh_decal_pom_required_texture_inputs(submaterial)


    def _material_needs_illum_decal_alpha_mode_refresh(
        self,
        material: bpy.types.Material,
        submaterial: SubmaterialRecord,
    ) -> bool:
        if not self._package_uses_fps_weapon_pom_material_logic():
            return False
        if not self._illum_submaterial_requires_premul_alpha(submaterial):
            return False

        node_tree = getattr(material, "node_tree", None)
        if node_tree is None:
            return True

        expected_names = self._illum_opacity_decal_texture_names(submaterial)
        found = False
        for node in node_tree.nodes:
            if node.bl_idname != "ShaderNodeTexImage":
                continue
            image = getattr(node, "image", None)
            if image is None:
                continue
            if expected_names:
                filepath = str(getattr(image, "filepath", "") or "")
                if Path(filepath).name.lower() not in expected_names:
                    continue
            found = True
            return getattr(image, "alpha_mode", None) != "PREMUL"
        return not found


    @staticmethod
    def _illum_submaterial_requires_premul_alpha(submaterial: SubmaterialRecord) -> bool:
        payload = getattr(submaterial, "raw", None)
        return isinstance(payload, dict) and _illum_payload_is_local_opacity_decal(payload)


    @staticmethod
    def _illum_opacity_decal_texture_names(submaterial: SubmaterialRecord) -> set[str]:
        names: set[str] = set()
        for texture in [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures]:
            if texture.is_virtual:
                continue
            if texture.slot != "TexSlot1" and texture.role not in {"base_color", "diffuse"}:
                continue
            if not texture.export_path:
                continue
            names.add(Path(texture.export_path).name.lower())
        return names


    def _reusable_material(
        self,
        sidecar_path: str,
        sidecar: MaterialSidecar,
        submaterial: SubmaterialRecord,
        palette: PaletteRecord | None,
        palette_scope: str,
        material_identity: str,
    ) -> bpy.types.Material | None:
        ui_image_path = None
        if material_identity and "|ui:" in material_identity:
            ui_image_path = material_identity.split("|ui:", 1)[1]
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
            ui_image_path=ui_image_path,
        ):
            return preferred

        self._ensure_material_identity_index()
        indexed_material = self.material_identity_index.get(material_identity)
        if indexed_material is not None and not _material_datablock_is_valid(indexed_material):
            self.material_identity_index.pop(material_identity, None)
            indexed_material = None
        if indexed_material is not None and _material_is_compatible(
            indexed_material,
            self.package,
            sidecar_path,
            sidecar,
            submaterial,
            palette,
            palette_scope,
            ui_image_path=ui_image_path,
        ):
            return indexed_material
        return None



    def _set_socket_default(self, socket: Any, value: Any) -> None:
        if socket is not None and hasattr(socket, "default_value"):
            socket.default_value = value



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
        if _texture_reference_uses_packed_roughness_green(texture):
            roughness = self._metallic_roughness_green_socket(nodes, texture.export_path, x=x, y=y)
            if roughness is not None:
                return roughness, False
        if texture.alpha_semantic == "smoothness":
            smoothness = self._texture_alpha_socket(nodes, texture.export_path, x=x, y=y, is_color=False)
            if smoothness is not None:
                return smoothness, True
        image_node = self._image_node(nodes, texture.export_path, x=x, y=y, is_color=False)
        if image_node is None:
            return None, False
        return image_node.outputs[0], False

    def _roughness_socket_for_layer_surface(
        self,
        nodes: bpy.types.Nodes,
        texture: TextureReference | None,
        *,
        x: int,
        y: int,
    ) -> Any:
        roughness, is_smoothness = self._roughness_socket_for_texture_reference(
            nodes, texture, x=x, y=y
        )
        if roughness is not None and is_smoothness:
            return self._invert_value_socket(nodes, roughness, x=x + 180, y=y)
        return roughness



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
        group_node = nodes.new("ShaderNodeGroup")
        group_node.node_tree = self._ensure_runtime_color_to_luma_group()
        group_node.location = (x + 180, y)
        group_node.label = "StarBreaker Color To Luma"
        image_node.id_data.links.new(image_node.outputs[0], group_node.inputs["Color"])
        return group_node.outputs["Luma"]



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
        if group_contract.name == "SB_MeshDecal_v1":
            if (
                template_plan_for_submaterial(submaterial).template_key == "decal_stencil"
                and submaterial.decoded_feature_flags.has_stencil_map
                and contract_input.name == "TexSlot7_StencilSource_alpha"
            ):
                return None
            stencil_source = self._mesh_decal_stencil_contract_source(
                nodes,
                submaterial,
                contract_input,
                x=x,
                y=y,
            )
            if stencil_source is not None:
                return stencil_source
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
                color_socket = image_node.outputs[0]
            else:
                color_socket = self._color_source_socket(nodes, submaterial, palette, image_path, x=x, y=y)
            return color_socket
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
            roughness_socket = self._metallic_roughness_green_socket(
                nodes,
                image_path,
                x=x,
                y=y,
            )
            if roughness_socket is not None:
                return roughness_socket

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


    def _metallic_roughness_green_socket(
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
        image_node.label = "METALLIC ROUGHNESS"

        separate = nodes.new("ShaderNodeSeparateColor")
        separate.location = (x + 180, y)
        if hasattr(separate, "mode"):
            separate.mode = "RGB"
        image_node.id_data.links.new(image_node.outputs[0], separate.inputs[0])
        return _output_socket(separate, "Green")



    def _smoothness_texture_reference(self, submaterial: SubmaterialRecord) -> TextureReference | None:
        return smoothness_texture_reference(submaterial)



    def _authored_emissive_triplet(self, submaterial: SubmaterialRecord) -> tuple[float, float, float] | None:
        for attribute in submaterial.raw.get("authored_attributes", []):
            if attribute.get("name") != "Emissive":
                continue
            value = attribute.get("value")
            if isinstance(value, (list, tuple)) and len(value) >= 3:
                try:
                    return (float(value[0]), float(value[1]), float(value[2]))
                except (TypeError, ValueError):
                    return None
            if isinstance(value, str):
                parts = [part.strip() for part in value.split(",")]
                if len(parts) >= 3:
                    try:
                        return (float(parts[0]), float(parts[1]), float(parts[2]))
                    except (TypeError, ValueError):
                        return None
            return None
        return None



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



    def _invert_value_socket(self, nodes: bpy.types.Nodes, source_socket: Any, *, x: int, y: int) -> Any:
        group_node = nodes.new("ShaderNodeGroup")
        group_node.location = (x, y)
        group_node.node_tree = self._ensure_runtime_ddna_roughness_group()
        group_node.label = "StarBreaker DDNA Roughness"
        group_node.id_data.links.new(source_socket, group_node.inputs["Smoothness"])
        return group_node.outputs["Roughness"]



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



    def _create_surface_bsdf(self, nodes: bpy.types.Nodes) -> bpy.types.ShaderNodeBsdfPrincipled:
        principled = nodes.new("ShaderNodeBsdfPrincipled")
        principled.location = (420, 0)
        principled.label = "StarBreaker Surface"
        return principled



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
        stencil_alpha = self._stencil_alpha_source_socket(nodes, submaterial, x=x, y=y)
        if stencil_alpha is not None:
            return stencil_alpha

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

    def _stencil_alpha_source_socket(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> Any:
        if template_plan_for_submaterial(submaterial).template_key != "decal_stencil":
            return None
        if not submaterial.decoded_feature_flags.has_stencil_map:
            return None
        stencil_ref = _submaterial_texture_reference(
            submaterial,
            slots=("TexSlot7",),
            roles=("stencil",),
        )
        if stencil_ref is None:
            return None
        return self._texture_alpha_socket(nodes, stencil_ref.export_path, x=x, y=y, is_color=True)

    def _mesh_decal_stencil_contract_source(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        contract_input: ContractInput,
        *,
        x: int,
        y: int,
    ) -> Any:
        if template_plan_for_submaterial(submaterial).template_key != "decal_stencil":
            return None
        if not submaterial.decoded_feature_flags.has_stencil_map:
            return None
        if contract_input.name not in {"TexSlot1_DecalSource", "TexSlot1_DecalSource_alpha"}:
            return None
        stencil_sockets = self._mesh_decal_stencil_adaptor_sockets(
            nodes,
            submaterial,
            x=x,
            y=y,
        )
        if stencil_sockets is None:
            return None
        if contract_input.name == "TexSlot1_DecalSource":
            return stencil_sockets.color
        return stencil_sockets.alpha

    def _mesh_decal_stencil_adaptor_sockets(
        self,
        nodes: bpy.types.Nodes,
        submaterial: SubmaterialRecord,
        *,
        x: int,
        y: int,
    ) -> LayerSurfaceSockets | None:
        stencil_ref = _submaterial_texture_reference(
            submaterial,
            slots=("TexSlot7",),
            roles=("stencil",),
        )
        if stencil_ref is None:
            return None
        ensure_adaptor = getattr(self, "_ensure_tint_decal_adaptor_group", None)
        if not callable(ensure_adaptor):
            return None

        existing = next(
            (
                node
                for node in nodes
                if node.bl_idname == "ShaderNodeGroup"
                and getattr(node, "name", "") == "STARBREAKER_STENCIL_TINT_DECAL_ADAPTOR"
            ),
            None,
        )
        if existing is not None:
            color_socket = _output_socket(existing, "Color")
            alpha_socket = _output_socket(existing, "Alpha")
            if color_socket is not None and alpha_socket is not None:
                return LayerSurfaceSockets(color=color_socket, alpha=alpha_socket)

        image_node = self._image_node(nodes, stencil_ref.export_path, x=x, y=y, is_color=True)
        if image_node is None:
            return None
        adaptor = nodes.new("ShaderNodeGroup")
        adaptor.name = "STARBREAKER_STENCIL_TINT_DECAL_ADAPTOR"
        adaptor.label = "StarBreaker Stencil Tint Decal"
        adaptor.location = (x + 240, y)
        adaptor.node_tree = ensure_adaptor()

        red = _public_param_triplet(
            submaterial,
            "StencilDiffuseColor1",
            "StencilDiffuse1",
            "StencilTintColor",
            "TintColor",
            "StencilDiffuseColor",
        ) or (1.0, 1.0, 1.0)
        green = _public_param_triplet(submaterial, "StencilDiffuseColor2", "StencilDiffuse2") or (1.0, 1.0, 1.0)
        blue = _public_param_triplet(submaterial, "StencilDiffuseColor3", "StencilDiffuse3") or (1.0, 1.0, 1.0)
        adaptor.inputs["Decal Red Tint"].default_value = (*red, 1.0)
        adaptor.inputs["Decal Green Tint"].default_value = (*green, 1.0)
        adaptor.inputs["Decal Blue Tint"].default_value = (*blue, 1.0)
        nodes.id_data.links.new(image_node.outputs[0], adaptor.inputs["Image"])

        color_socket = _output_socket(adaptor, "Color")
        alpha_socket = _output_socket(adaptor, "Alpha")
        if color_socket is None or alpha_socket is None:
            return None
        return LayerSurfaceSockets(color=color_socket, alpha=alpha_socket)



    def _illum_emission_strength(self, submaterial: SubmaterialRecord) -> float:
        emissive = self._authored_emissive_triplet(submaterial)
        emissive_mean = _mean_triplet(emissive) if emissive is not None else None
        if emissive_mean is not None and emissive_mean > 0.0:
            return emissive_mean

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



    def _mesh_decal_emission_strength(self, submaterial: SubmaterialRecord) -> float:
        return _mesh_decal_authored_emission_strength(
            submaterial,
            emissive_texture_path=self._texture_export_path(submaterial, "emissive"),
        )



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
        alpha_mode: str | None = None,
    ) -> bpy.types.ShaderNodeTexImage | None:
        resolved = self.package.resolve_path(image_path)
        if resolved is None or not resolved.is_file():
            return None
        resolved_str = str(resolved)
        if not self._package_uses_fps_weapon_pom_material_logic():
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
            node.image = bpy.data.images.load(resolved_str, check_existing=True)
            self._refresh_image_source(node.image, resolved_str)
            if not is_color and node.image is not None and hasattr(node.image, "colorspace_settings"):
                node.image.colorspace_settings.name = "Non-Color"
            return node

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
                self._set_image_alpha_mode(image, alpha_mode)
                return existing
            if self._image_matches_color_space(image, is_color=is_color):
                existing.location = (x, y)
                self._set_image_alpha_mode(image, alpha_mode)
                return existing
        node = nodes.new("ShaderNodeTexImage")
        node.location = (x, y)
        node.image = self._load_image_for_color_space(resolved_str, is_color=is_color)
        self._set_image_alpha_mode(node.image, alpha_mode)
        return node


    @staticmethod
    def _image_matches_color_space(image: Any, *, is_color: bool) -> bool:
        color_space = getattr(getattr(image, "colorspace_settings", None), "name", "")
        return color_space != "Non-Color" if is_color else color_space == "Non-Color"


    @staticmethod
    def _image_file_path(image: Any) -> str:
        try:
            return bpy.path.abspath(image.filepath, library=image.library)
        except Exception:
            return str(getattr(image, "filepath", "") or "")

    @staticmethod
    def _refresh_image_source(image: Any, resolved_path: str) -> None:
        """Reload a reused image datablock when its source file changed on disk and
        stamp the source mtime so re-exports pick up fresh pixels. Inert on objects
        that don't expose Blender ID-property access (e.g. test stubs)."""
        if image is None:
            return
        try:
            token = str(Path(resolved_path).stat().st_mtime_ns)
        except OSError:
            return
        getter = getattr(image, "get", None)
        current = getter("starbreaker_source_mtime_ns", "") if callable(getter) else ""
        if current and str(current) != token:
            reload_fn = getattr(image, "reload", None)
            if callable(reload_fn):
                reload_fn()
        try:
            # Blender ID properties use C-backed ints and can overflow on ns timestamps.
            image["starbreaker_source_mtime_ns"] = token
        except TypeError:
            pass


    @classmethod
    def _load_image_for_color_space(cls, resolved_path: str, *, is_color: bool) -> Any:
        for image in getattr(bpy.data, "images", []) or []:
            if cls._image_file_path(image) != resolved_path:
                continue
            if cls._image_matches_color_space(image, is_color=is_color):
                cls._refresh_image_source(image, resolved_path)
                return image

        image = bpy.data.images.load(resolved_path, check_existing=False)
        cls._refresh_image_source(image, resolved_path)
        color_settings = getattr(image, "colorspace_settings", None)
        if color_settings is not None:
            try:
                color_settings.name = "sRGB" if is_color else "Non-Color"
            except Exception:
                if not is_color:
                    try:
                        color_settings.name = "Non-Color"
                    except Exception:
                        pass
        return image


    @staticmethod
    def _set_image_alpha_mode(image: Any, alpha_mode: str | None) -> None:
        if image is None or not alpha_mode:
            return
        if not hasattr(image, "alpha_mode"):
            return
        try:
            image.alpha_mode = alpha_mode
        except Exception:
            return



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



    def _configure_material(self, material: bpy.types.Material, *, blend_method: str, shadow_method: str) -> None:
        if hasattr(material, "blend_method"):
            try:
                material.blend_method = blend_method
            except (AttributeError, TypeError):
                # Blender 4.2+ made ``blend_method`` read-only; the
                # modern property is ``surface_render_method`` below.
                pass
        if hasattr(material, "surface_render_method"):
            # Blender 5.x unified mapping:
            #   OPAQUE / CLIP / HASHED -> DITHERED
            #   BLEND                  -> BLENDED
            method = "BLENDED" if blend_method == "BLEND" else "DITHERED"
            try:
                material.surface_render_method = method
            except (AttributeError, TypeError):
                pass
        if hasattr(material, "shadow_method"):
            try:
                material.shadow_method = shadow_method
            except (AttributeError, TypeError):
                pass
        material.use_backface_culling = False



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
            else:
                slot_names = [
                    material.name if material is not None and getattr(material, "name", None) else None
                    for material in materials
                ]
                if any(name is not None for name in slot_names):
                    obj.data[PROP_IMPORTED_SLOT_NAMES] = json.dumps(slot_names)
            for index in range(len(materials)):
                materials[index] = None



    def _purge_unused_materials(self, materials: list[bpy.types.Material]) -> None:
        # Phase B perf fix: defer the actual ``bpy.data.materials.remove``
        # to a single ``bpy.data.batch_remove`` at the end of the import.
        # The per-template path used to call ``remove`` once per orphan
        # which dominated 8s of wall on the Clipper. The orphan set is
        # re-checked for ``users == 0`` at flush time so any intervening
        # rebind still keeps the material alive.
        pending = getattr(self, "_pending_orphan_materials", None)
        if pending is None:
            for material in materials:
                if material is not None and material.users == 0:
                    bpy.data.materials.remove(material)
            return
        for material in materials:
            if material is not None and material.users == 0:
                pending.add(material)



    def _link_color_output(self, output: Any, input_socket: Any) -> None:
        output.node.id_data.links.new(output, input_socket)
