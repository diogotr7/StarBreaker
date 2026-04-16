from __future__ import annotations

from .manifest import LiveryRecord, PackageBundle, PaletteRecord, SceneInstanceRecord


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


def palette_for_id(package: PackageBundle, palette_id: str | None) -> PaletteRecord | None:
    if palette_id and palette_id in package.palettes:
        return package.palettes[palette_id]
    fallback = default_palette_id(package)
    if fallback is None:
        return None
    return package.palettes.get(fallback)


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
