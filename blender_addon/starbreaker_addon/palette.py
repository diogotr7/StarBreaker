from __future__ import annotations

from .manifest import LiveryRecord, PackageBundle, PaletteRecord, SceneInstanceRecord, SubmaterialRecord
from .templates import material_palette_channels


def _palette_id_candidates(palette_id: str | None) -> tuple[str, ...]:
    if not palette_id:
        return ()
    candidates = [palette_id]
    if palette_id.startswith("paint/"):
        suffix = palette_id[len("paint/"):]
        candidates.append(f"palette/{suffix}")
        if suffix.startswith("paint_"):
            candidates.append(f"palette/{suffix[len('paint_'):]}")
    # Preserve order while removing duplicates.
    return tuple(dict.fromkeys(candidates))


def available_palette_ids(package: PackageBundle) -> list[str]:
    return sorted(package.palettes.keys())


def available_livery_ids(package: PackageBundle) -> list[str]:
    return sorted(package.liveries.keys())


def default_palette_id(package: PackageBundle) -> str | None:
    if "palette/default" in package.palettes:
        return "palette/default"
    if package.scene.root_entity.palette_id in package.palettes:
        return package.scene.root_entity.palette_id
    return next(iter(sorted(package.palettes.keys())), None)


def resolved_palette_id(
    package: PackageBundle,
    palette_id: str | None,
    inherited_palette_id: str | None = None,
) -> str | None:
    for candidate in _palette_id_candidates(palette_id):
        if candidate in package.palettes:
            return candidate
    for candidate in _palette_id_candidates(inherited_palette_id):
        if candidate in package.palettes:
            return candidate
    return default_palette_id(package)


def palette_for_id(
    package: PackageBundle,
    palette_id: str | None,
    inherited_palette_id: str | None = None,
) -> PaletteRecord | None:
    resolved = resolved_palette_id(package, palette_id, inherited_palette_id)
    if resolved is None:
        return None
    return package.palettes.get(resolved)


def palette_color(palette: PaletteRecord | None, channel_name: str | None) -> tuple[float, float, float]:
    if palette is None:
        return (1.0, 1.0, 1.0)
    if channel_name == "primary":
        return palette.primary
    if channel_name == "secondary":
        return palette.secondary
    if channel_name == "tertiary":
        return palette.tertiary
    if channel_name == "glass":
        return palette.glass
    return palette.primary


def palette_finish_channel(palette: PaletteRecord | None, channel_name: str | None) -> dict[str, object] | None:
    if palette is None:
        return None
    finish = palette.raw.get("finish")
    if not isinstance(finish, dict):
        return None
    if channel_name not in {"primary", "secondary", "tertiary", "glass"}:
        channel_name = "primary"
    channel = finish.get(channel_name)
    if not isinstance(channel, dict):
        return None
    return channel


def palette_finish_specular(palette: PaletteRecord | None, channel_name: str | None) -> tuple[float, float, float] | None:
    channel = palette_finish_channel(palette, channel_name)
    if channel is None:
        return None
    specular = channel.get("specular")
    if not isinstance(specular, (list, tuple)) or len(specular) < 3:
        return None
    try:
        return (float(specular[0]), float(specular[1]), float(specular[2]))
    except (TypeError, ValueError):
        return None


def palette_finish_glossiness(palette: PaletteRecord | None, channel_name: str | None) -> float | None:
    channel = palette_finish_channel(palette, channel_name)
    if channel is None:
        return None
    glossiness = channel.get("glossiness")
    if glossiness is None:
        return None
    try:
        return float(glossiness)
    except (TypeError, ValueError):
        return None


def palette_decal_color(palette: PaletteRecord | None, channel_name: str | None) -> tuple[float, float, float] | None:
    if palette is None:
        return None
    if channel_name == "red":
        return palette.decal_red
    if channel_name == "green":
        return palette.decal_green
    if channel_name == "blue":
        return palette.decal_blue
    return None


def palette_decal_texture(palette: PaletteRecord | None) -> str | None:
    if palette is None:
        return None
    return palette.decal_texture


def palette_signature_for_submaterial(
    submaterial: SubmaterialRecord,
    palette: PaletteRecord | None,
) -> dict[str, dict[str, object] | None] | None:
    signature: dict[str, dict[str, object] | None] = {}
    for channel in material_palette_channels(submaterial):
        if channel.name in signature:
            continue
        if palette is None:
            signature[channel.name] = None
            continue
        signature[channel.name] = {
            "color": palette_color(palette, channel.name),
            "specular": palette_finish_specular(palette, channel.name),
            "glossiness": palette_finish_glossiness(palette, channel.name),
        }
    return signature or None


def livery_for_id(package: PackageBundle, livery_id: str | None) -> LiveryRecord | None:
    if livery_id is None:
        return None
    return package.liveries.get(livery_id)


def livery_applies_to_instance(
    livery: LiveryRecord,
    instance: SceneInstanceRecord,
    material_sidecar_path: str | None,
) -> bool:
    if instance.entity_name in livery.entity_names:
        return True
    if material_sidecar_path is not None and material_sidecar_path in livery.material_sidecars:
        return True
    return False


def palette_id_for_livery_instance(
    package: PackageBundle,
    livery_id: str | None,
    instance: SceneInstanceRecord,
    material_sidecar_path: str | None,
) -> str | None:
    livery = livery_for_id(package, livery_id)
    if livery is None:
        return instance.palette_id
    if livery_applies_to_instance(livery, instance, material_sidecar_path):
        return livery.palette_id or instance.palette_id
    return instance.palette_id
