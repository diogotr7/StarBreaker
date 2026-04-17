from __future__ import annotations

from dataclasses import dataclass
import re


NON_ALNUM = re.compile(r"[^0-9A-Za-z]+")


@dataclass(frozen=True)
class TextureInputRule:
    suffix: str
    semantic: str


TEXTURE_INPUT_RULES: dict[str, dict[str, TextureInputRule]] = {
    "DisplayScreen": {
        "TexSlot2": TextureInputRule("NormalGloss", "normal_gloss"),
        "TexSlot3": TextureInputRule("CrackNormalGloss", "crack_normal_gloss"),
        "TexSlot6": TextureInputRule("ScreenSurfaceMask", "screen_surface_mask"),
        "TexSlot9": TextureInputRule("ScreenSource", "screen_source"),
        "TexSlot10": TextureInputRule("CrackMask", "crack_mask"),
        "TexSlot11": TextureInputRule("Dirt", "dirt"),
        "TexSlot15": TextureInputRule("CondensationNormal", "condensation_normal"),
        "TexSlot16": TextureInputRule("DisplayMask", "display_mask"),
        "TexSlot17": TextureInputRule("PixelLayout", "pixel_layout"),
    },
    "GlassPBR": {
        "TexSlot2": TextureInputRule("NormalGloss", "normal_gloss"),
        "TexSlot4": TextureInputRule("TintColor", "tint_color"),
        "TexSlot6": TextureInputRule("WearGloss", "wear_gloss"),
        "TexSlot11": TextureInputRule("Dirt", "dirt"),
        "TexSlot15": TextureInputRule("CondensationNormal", "condensation_normal"),
        "TexSlot16": TextureInputRule("PatternMask", "pattern_mask"),
    },
    "HardSurface": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
        "TexSlot3": TextureInputRule("NormalGloss", "normal_gloss"),
        "TexSlot6": TextureInputRule("Displacement", "displacement"),
        "TexSlot10": TextureInputRule("IridescenceColor", "iridescence_color"),
        "TexSlot14": TextureInputRule("Emissive", "emissive"),
    },
    "Illum": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
        "TexSlot2": TextureInputRule("NormalGlossPrimary", "normal_gloss_primary"),
        "TexSlot3": TextureInputRule("NormalGlossSecondary", "normal_gloss_secondary"),
        "TexSlot4": TextureInputRule("Specular", "specular"),
        "TexSlot6": TextureInputRule("DetailAux", "detail_aux"),
        "TexSlot8": TextureInputRule("Height", "height"),
        "TexSlot9": TextureInputRule("BaseColorSecondary", "base_color_secondary"),
        "TexSlot10": TextureInputRule("SpecularSecondary", "specular_secondary"),
        "TexSlot11": TextureInputRule("HeightSecondary", "height_secondary"),
        "TexSlot12": TextureInputRule("BlendMask", "blend_mask"),
        "TexSlot13": TextureInputRule("DetailSecondary", "detail_secondary"),
        "TexSlot17": TextureInputRule("SubsurfaceMask", "subsurface_mask"),
    },
    "Layer": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
        "TexSlot2": TextureInputRule("NormalGloss", "normal_gloss"),
    },
    "LayerBlend_V2": {
        "TexSlot3": TextureInputRule("NormalGloss", "normal_gloss"),
        "TexSlot11": TextureInputRule("WearMask", "wear_mask"),
        "TexSlot12": TextureInputRule("BlendMask", "blend_mask"),
        "TexSlot13": TextureInputRule("HalControl", "hal_control"),
    },
    "Eye": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
        "TexSlot2": TextureInputRule("IrisNormal", "iris_normal"),
        "TexSlot3": TextureInputRule("CorneaNormal", "cornea_normal"),
        "TexSlot8": TextureInputRule("Height", "height"),
    },
    "Hair": {
        "TexSlot1": TextureInputRule("StrandColor", "strand_color"),
    },
    "HairPBR": {
        "TexSlot1": TextureInputRule("OpacityMask", "opacity_mask"),
        "TexSlot4": TextureInputRule("IdMap", "id_map"),
    },
    "HumanSkin_V2": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
        "TexSlot2": TextureInputRule("NormalGloss", "normal_gloss"),
        "TexSlot3": TextureInputRule("WrinkleColor", "wrinkle_color"),
        "TexSlot4": TextureInputRule("Specular", "specular"),
        "TexSlot6": TextureInputRule("SkinMicroDetail", "skin_micro_detail"),
        "TexSlot8": TextureInputRule("WrinkleNormal", "wrinkle_normal"),
        "TexSlot11": TextureInputRule("WrinkleMask", "wrinkle_mask"),
        "TexSlot12": TextureInputRule("Transmission", "transmission"),
    },
    "MeshDecal": {
        "TexSlot1": TextureInputRule("DecalSource", "decal_source"),
        "TexSlot2": TextureInputRule("Specular", "specular"),
        "TexSlot3": TextureInputRule("NormalGloss", "normal_gloss"),
        "TexSlot4": TextureInputRule("Height", "height"),
        "TexSlot5": TextureInputRule("BreakupMask", "breakup_mask"),
        "TexSlot6": TextureInputRule("TintMask", "tint_mask"),
        "TexSlot7": TextureInputRule("StencilSource", "stencil_source"),
        "TexSlot8": TextureInputRule("GrimeBreakup", "grime_breakup"),
    },
    "Monitor": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
    },
    "UIPlane": {
        "TexSlot9": TextureInputRule("ScreenSource", "screen_source"),
        "TexSlot17": TextureInputRule("PixelLayout", "pixel_layout"),
    },
    "Unknown": {
        "TexSlot1": TextureInputRule("BaseColor", "base_color"),
    },
}


def _pascal_case(value: str) -> str:
    if not value:
        return value
    words = [part for part in NON_ALNUM.split(value) if part]
    if not words:
        return value
    return "".join(word[:1].upper() + word[1:] for word in words)


def texture_input_rule(shader_family: str, slot: str) -> TextureInputRule | None:
    return TEXTURE_INPUT_RULES.get(shader_family, {}).get(slot)


def known_texture_slots(shader_family: str) -> list[str]:
    return sorted(TEXTURE_INPUT_RULES.get(shader_family, {}))


def texture_input_name(shader_family: str, slot: str) -> str:
    rule = texture_input_rule(shader_family, slot)
    suffix = rule.suffix if rule is not None else "Aux"
    return f"{slot}_{suffix}"


def texture_input_semantic(shader_family: str, slot: str) -> str | None:
    rule = texture_input_rule(shader_family, slot)
    return None if rule is None else rule.semantic


def public_param_input_name(name: str) -> str:
    return f"Param_{_pascal_case(name)}"


def palette_input_name(name: str) -> str:
    return f"Palette_{_pascal_case(name)}"


def virtual_input_name(name: str) -> str:
    return f"Virtual_{_pascal_case(name.lstrip('$'))}"
