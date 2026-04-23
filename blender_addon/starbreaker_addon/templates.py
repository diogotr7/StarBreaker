from __future__ import annotations

from dataclasses import dataclass

from .manifest import PaletteChannel, SubmaterialRecord, TextureReference


@dataclass(frozen=True)
class MaterialTemplatePlan:
    template_key: str
    label: str
    blend_method: str
    shadow_method: str
    cycles_first: bool
    uses_palette: bool
    uses_layers: bool
    uses_transmission: bool
    uses_emission: bool
    uses_alpha: bool


BASE_COLOR_ROLES = ("base_color", "alternate_base_color", "diffuse", "decal_sheet")
NORMAL_ROLES = ("normal_gloss",)
ROUGHNESS_ROLES = ("roughness",)
MASK_ROLES = ("wear_mask", "pattern_mask", "stencil", "tint_palette_decal", "hal_control")
HEIGHT_ROLES = ("height",)
OPACITY_ROLES = ("opacity",)


def active_submaterials(submaterials: list[SubmaterialRecord]) -> list[SubmaterialRecord]:
    return [submaterial for submaterial in submaterials if submaterial.activation_state == "active"]


def template_plan_for_submaterial(submaterial: SubmaterialRecord) -> MaterialTemplatePlan:
    family = submaterial.shader_family
    flags = submaterial.decoded_feature_flags

    if family == "NoDraw" or submaterial.activation_state != "active":
        return MaterialTemplatePlan("nodraw", "NoDraw", "CLIP", "NONE", False, False, False, False, False, True)
    if family in {"Layer", "LayerBlend_V2"}:
        return MaterialTemplatePlan("layered_wear", "Layered Wear", "OPAQUE", "OPAQUE", True, True, True, False, False, False)
    if family == "MeshDecal" or flags.has_decal or (flags.has_stencil_map and family != "HardSurface"):
        return MaterialTemplatePlan("decal_stencil", "Decal Or Stencil", "CLIP", "NONE", True, True, False, False, False, True)
    if flags.has_parallax_occlusion_mapping:
        return MaterialTemplatePlan("parallax_pom", "Parallax Or POM", "BLEND", "NONE", True, True, False, False, True, True)
    if family in {"DisplayScreen", "Monitor", "UIPlane"}:
        return MaterialTemplatePlan("screen_hud", "Screen Or HUD", "BLEND", "HASHED", True, False, False, False, True, True)
    if family in {"HumanSkin_V2", "Eye", "Organic"}:
        return MaterialTemplatePlan("biological", "Biological", "OPAQUE", "OPAQUE", True, True, False, False, False, False)
    if family == "HairPBR":
        return MaterialTemplatePlan("hair", "Hair", "HASHED", "HASHED", True, True, False, False, False, True)
    if family in {"Hologram", "HologramCIG", "Shield_Holo", "UIMesh"}:
        return MaterialTemplatePlan("effects", "Effects", "BLEND", "NONE", True, False, False, False, True, True)
    if family == "GlassPBR":
        return MaterialTemplatePlan("physical_surface", "Physical Surface", "BLEND", "HASHED", True, True, False, True, False, True)
    if family == "Illum":
        return MaterialTemplatePlan("physical_surface", "Physical Surface", "OPAQUE", "OPAQUE", True, True, False, False, True, False)
    return MaterialTemplatePlan("physical_surface", "Physical Surface", "OPAQUE", "OPAQUE", True, True, False, False, False, False)


def _all_texture_candidates(submaterial: SubmaterialRecord) -> list[TextureReference]:
    return [*submaterial.texture_slots, *submaterial.direct_textures, *submaterial.derived_textures]


def first_texture_export(submaterial: SubmaterialRecord, roles: tuple[str, ...]) -> str | None:
    for texture in _all_texture_candidates(submaterial):
        if texture.role in roles and texture.export_path:
            return texture.export_path
    return None


def representative_textures(submaterial: SubmaterialRecord) -> dict[str, str | None]:
    layer_base = next((layer.diffuse_export_path for layer in submaterial.layer_manifest if layer.diffuse_export_path), None)
    layer_normal = next((layer.normal_export_path for layer in submaterial.layer_manifest if layer.normal_export_path), None)
    layer_roughness = next((layer.roughness_export_path for layer in submaterial.layer_manifest if layer.roughness_export_path), None)
    return {
        "base_color": first_texture_export(submaterial, BASE_COLOR_ROLES) or layer_base,
        "normal": first_texture_export(submaterial, NORMAL_ROLES) or layer_normal,
        "roughness": first_texture_export(submaterial, ROUGHNESS_ROLES) or layer_roughness,
        "mask": first_texture_export(submaterial, MASK_ROLES),
        "height": first_texture_export(submaterial, HEIGHT_ROLES),
        "opacity": first_texture_export(submaterial, OPACITY_ROLES),
    }


def smoothness_texture_reference(submaterial: SubmaterialRecord) -> TextureReference | None:
    for texture in _all_texture_candidates(submaterial):
        if texture.alpha_semantic == "smoothness" and texture.export_path:
            return texture
    for layer in submaterial.layer_manifest:
        for texture in layer.texture_slots:
            if texture.alpha_semantic == "smoothness" and texture.export_path:
                return texture
    return None


def material_palette_channels(submaterial: SubmaterialRecord) -> list[PaletteChannel]:
    channels: list[PaletteChannel] = []
    if submaterial.palette_routing.material_channel is not None:
        channels.append(submaterial.palette_routing.material_channel)
    for binding in submaterial.palette_routing.layer_channels:
        if all((existing.index, existing.name) != (binding.channel.index, binding.channel.name) for existing in channels):
            channels.append(binding.channel)
    for layer in submaterial.layer_manifest:
        if layer.palette_channel is not None and all(
            (existing.index, existing.name) != (layer.palette_channel.index, layer.palette_channel.name)
            for existing in channels
        ):
            channels.append(layer.palette_channel)
    return channels


def has_virtual_input(submaterial: SubmaterialRecord, input_name: str) -> bool:
    return input_name in submaterial.virtual_inputs
