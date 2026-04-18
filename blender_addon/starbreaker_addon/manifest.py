from __future__ import annotations

from dataclasses import dataclass, field
import json
from pathlib import Path
from typing import Any, Mapping

JsonDict = dict[str, Any]
Color3 = tuple[float, float, float]
Vec3 = tuple[float, float, float]
Vec4 = tuple[float, float, float, float]
Matrix4 = tuple[tuple[float, float, float, float], ...]


def _load_json(path: Path) -> JsonDict:
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"Expected JSON object in {path}")
    return data


def _as_dict(value: Any) -> JsonDict:
    if isinstance(value, Mapping):
        return dict(value)
    return {}


def _as_str(value: Any) -> str | None:
    if value is None:
        return None
    return str(value)


def _as_bool(value: Any, default: bool = False) -> bool:
    if isinstance(value, bool):
        return value
    return default


def _as_float(value: Any, default: float = 0.0) -> float:
    if value is None:
        return default
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def _float_tuple(value: Any, length: int) -> tuple[float, ...]:
    if not isinstance(value, (list, tuple)):
        return tuple(0.0 for _ in range(length))
    items = list(value)[:length]
    while len(items) < length:
        items.append(0.0)
    return tuple(_as_float(item) for item in items)


def _matrix4(value: Any) -> Matrix4:
    rows = []
    if isinstance(value, (list, tuple)):
        rows = list(value)[:4]
    while len(rows) < 4:
        rows.append((0.0, 0.0, 0.0, 0.0))
    return tuple(tuple(float(item) for item in _float_tuple(row, 4)) for row in rows)  # type: ignore[return-value]


def _normalize_relative_path(path: str | None) -> str | None:
    if path is None:
        return None
    return path.replace("\\", "/").lstrip("/")


def _relative_path(path: str | None) -> Path | None:
    normalized = _normalize_relative_path(path)
    if normalized is None:
        return None
    return Path(normalized)


def _candidate_relative_paths(path: str | None) -> list[str]:
    normalized = _normalize_relative_path(path)
    if normalized is None:
        return []
    candidates = [normalized]
    for prefix in ("materials/", "meshes/", "textures/"):
        if normalized.lower().startswith(prefix):
            stripped = normalized[len(prefix):]
            if stripped.lower().startswith("data/"):
                candidates.append(stripped)
    return candidates


@dataclass(frozen=True)
class PaletteChannel:
    index: int
    name: str

    @classmethod
    def from_value(cls, value: Any) -> PaletteChannel | None:
        data = _as_dict(value)
        if not data:
            return None
        return cls(index=int(data.get("index", 0)), name=str(data.get("name", "")))


@dataclass(frozen=True)
class LayerChannelBinding:
    index: int
    channel: PaletteChannel

    @classmethod
    def from_value(cls, value: Any) -> LayerChannelBinding | None:
        data = _as_dict(value)
        channel = PaletteChannel.from_value(data.get("channel"))
        if channel is None:
            return None
        return cls(index=int(data.get("index", 0)), channel=channel)


@dataclass(frozen=True)
class PaletteRouting:
    material_channel: PaletteChannel | None
    layer_channels: list[LayerChannelBinding]

    @classmethod
    def from_value(cls, value: Any) -> PaletteRouting:
        data = _as_dict(value)
        layer_channels = [
            binding
            for binding in (LayerChannelBinding.from_value(item) for item in data.get("layer_channels", []))
            if binding is not None
        ]
        return cls(
            material_channel=PaletteChannel.from_value(data.get("material_channel")),
            layer_channels=layer_channels,
        )


@dataclass(frozen=True)
class FeatureFlags:
    tokens: list[str]
    has_decal: bool
    has_iridescence: bool
    has_parallax_occlusion_mapping: bool
    has_stencil_map: bool
    has_vertex_colors: bool

    @classmethod
    def from_value(cls, value: Any) -> FeatureFlags:
        data = _as_dict(value)
        return cls(
            tokens=[str(token) for token in data.get("tokens", [])],
            has_decal=_as_bool(data.get("has_decal")),
            has_iridescence=_as_bool(data.get("has_iridescence")),
            has_parallax_occlusion_mapping=_as_bool(data.get("has_parallax_occlusion_mapping")),
            has_stencil_map=_as_bool(data.get("has_stencil_map")),
            has_vertex_colors=_as_bool(data.get("has_vertex_colors")),
        )


@dataclass(frozen=True)
class TextureReference:
    role: str
    source_path: str | None
    export_path: str | None
    export_kind: str
    slot: str | None = None
    is_virtual: bool = False
    texture_identity: str | None = None
    alpha_semantic: str | None = None
    derived_from_texture_identity: str | None = None
    derived_from_semantic: str | None = None
    texture_transform: JsonDict | None = None

    @classmethod
    def from_value(cls, value: Any) -> TextureReference:
        data = _as_dict(value)
        return cls(
            role=str(data.get("role", "")),
            source_path=_normalize_relative_path(_as_str(data.get("source_path"))),
            export_path=_normalize_relative_path(_as_str(data.get("export_path"))),
            export_kind=str(data.get("export_kind", "source")),
            slot=_as_str(data.get("slot")),
            is_virtual=_as_bool(data.get("is_virtual")),
            texture_identity=_as_str(data.get("texture_identity")),
            alpha_semantic=_as_str(data.get("alpha_semantic")),
            derived_from_texture_identity=_as_str(data.get("derived_from_texture_identity")),
            derived_from_semantic=_as_str(data.get("derived_from_semantic")),
            texture_transform=_as_dict(data.get("texture_transform")) or None,
        )


@dataclass(frozen=True)
class LayerManifestEntry:
    index: int
    source_material_path: str | None
    diffuse_export_path: str | None
    normal_export_path: str | None
    roughness_export_path: str | None
    palette_channel: PaletteChannel | None
    tint_color: Color3 | None
    uv_tiling: float | None

    @classmethod
    def from_value(cls, value: Any) -> LayerManifestEntry:
        data = _as_dict(value)
        tint = data.get("tint_color")
        tint_color = None
        if isinstance(tint, (list, tuple)):
            tint_color = _float_tuple(tint, 3)  # type: ignore[assignment]
        uv_tiling = data.get("uv_tiling")
        return cls(
            index=int(data.get("index", 0)),
            source_material_path=_normalize_relative_path(_as_str(data.get("source_material_path"))),
            diffuse_export_path=_normalize_relative_path(_as_str(data.get("diffuse_export_path"))),
            normal_export_path=_normalize_relative_path(_as_str(data.get("normal_export_path"))),
            roughness_export_path=_normalize_relative_path(_as_str(data.get("roughness_export_path"))),
            palette_channel=PaletteChannel.from_value(data.get("palette_channel")),
            tint_color=tint_color,
            uv_tiling=float(uv_tiling) if uv_tiling is not None else None,
        )


@dataclass(frozen=True)
class SubmaterialRecord:
    index: int
    submaterial_name: str
    blender_material_name: str | None
    shader: str
    shader_family: str
    activation_state: str
    activation_reason: str
    decoded_feature_flags: FeatureFlags
    direct_textures: list[TextureReference]
    derived_textures: list[TextureReference]
    texture_slots: list[TextureReference]
    layer_manifest: list[LayerManifestEntry]
    palette_routing: PaletteRouting
    public_params: JsonDict
    variant_membership: JsonDict
    virtual_inputs: list[str]
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> SubmaterialRecord:
        data = _as_dict(value)
        activation = _as_dict(data.get("activation_state"))
        return cls(
            index=int(data.get("index", 0)),
            submaterial_name=str(data.get("submaterial_name", "")),
            blender_material_name=_as_str(data.get("blender_material_name")),
            shader=str(data.get("shader", "")),
            shader_family=str(data.get("shader_family", "")),
            activation_state=str(activation.get("state", "active")),
            activation_reason=str(activation.get("reason", "visible")),
            decoded_feature_flags=FeatureFlags.from_value(data.get("decoded_feature_flags")),
            direct_textures=[TextureReference.from_value(item) for item in data.get("direct_textures", [])],
            derived_textures=[TextureReference.from_value(item) for item in data.get("derived_textures", [])],
            texture_slots=[TextureReference.from_value(item) for item in data.get("texture_slots", [])],
            layer_manifest=[LayerManifestEntry.from_value(item) for item in data.get("layer_manifest", [])],
            palette_routing=PaletteRouting.from_value(data.get("palette_routing")),
            public_params=_as_dict(data.get("public_params")),
            variant_membership=_as_dict(data.get("variant_membership")),
            virtual_inputs=[str(item) for item in data.get("virtual_inputs", [])],
            raw=data,
        )


@dataclass(frozen=True)
class MaterialSidecar:
    geometry_path: str | None
    normalized_export_relative_path: str | None
    source_material_path: str | None
    palette_contract: JsonDict
    submaterials: list[SubmaterialRecord]
    raw: JsonDict

    @classmethod
    def from_file(cls, path: Path) -> MaterialSidecar:
        return cls.from_value(_load_json(path))

    @classmethod
    def from_value(cls, value: Any) -> MaterialSidecar:
        data = _as_dict(value)
        return cls(
            geometry_path=_normalize_relative_path(_as_str(data.get("geometry_path"))),
            normalized_export_relative_path=_normalize_relative_path(_as_str(data.get("normalized_export_relative_path"))),
            source_material_path=_normalize_relative_path(_as_str(data.get("source_material_path"))),
            palette_contract=_as_dict(data.get("palette_contract")),
            submaterials=[SubmaterialRecord.from_value(item) for item in data.get("submaterials", [])],
            raw=data,
        )


@dataclass(frozen=True)
class PaletteRecord:
    id: str
    source_name: str | None
    primary: Color3
    secondary: Color3
    tertiary: Color3
    glass: Color3
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> PaletteRecord:
        data = _as_dict(value)
        return cls(
            id=str(data.get("id", "")),
            source_name=_as_str(data.get("source_name")),
            primary=_float_tuple(data.get("primary"), 3),  # type: ignore[arg-type]
            secondary=_float_tuple(data.get("secondary"), 3),  # type: ignore[arg-type]
            tertiary=_float_tuple(data.get("tertiary"), 3),  # type: ignore[arg-type]
            glass=_float_tuple(data.get("glass"), 3),  # type: ignore[arg-type]
            raw=data,
        )


@dataclass(frozen=True)
class LiveryRecord:
    id: str
    palette_id: str | None
    palette_source_name: str | None
    entity_names: list[str]
    material_sidecars: list[str]
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> LiveryRecord:
        data = _as_dict(value)
        return cls(
            id=str(data.get("id", "")),
            palette_id=_as_str(data.get("palette_id")),
            palette_source_name=_as_str(data.get("palette_source_name")),
            entity_names=[str(item) for item in data.get("entity_names", [])],
            material_sidecars=[
                normalized
                for normalized in (_normalize_relative_path(_as_str(item)) for item in data.get("material_sidecars", []))
                if normalized is not None
            ],
            raw=data,
        )


@dataclass(frozen=True)
class LightRecord:
    name: str
    color: Color3
    light_type: str | None
    intensity: float
    radius: float
    position: Vec3
    rotation: Vec4
    inner_angle: float
    outer_angle: float
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> LightRecord:
        data = _as_dict(value)
        return cls(
            name=str(data.get("name", "")),
            color=_float_tuple(data.get("color"), 3),  # type: ignore[arg-type]
            light_type=_as_str(data.get("light_type")),
            intensity=_as_float(data.get("intensity")),
            radius=_as_float(data.get("radius")),
            position=_float_tuple(data.get("position"), 3),  # type: ignore[arg-type]
            rotation=_float_tuple(data.get("rotation"), 4),  # type: ignore[arg-type]
            inner_angle=_as_float(data.get("inner_angle")),
            outer_angle=_as_float(data.get("outer_angle")),
            raw=data,
        )


@dataclass(frozen=True)
class InteriorPlacementRecord:
    cgf_path: str | None
    material_path: str | None
    mesh_asset: str | None
    material_sidecar: str | None
    entity_class_guid: str | None
    transform: Matrix4
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> InteriorPlacementRecord:
        data = _as_dict(value)
        return cls(
            cgf_path=_normalize_relative_path(_as_str(data.get("cgf_path"))),
            material_path=_normalize_relative_path(_as_str(data.get("material_path"))),
            mesh_asset=_normalize_relative_path(_as_str(data.get("mesh_asset"))),
            material_sidecar=_normalize_relative_path(_as_str(data.get("material_sidecar"))),
            entity_class_guid=_as_str(data.get("entity_class_guid")),
            transform=_matrix4(data.get("transform")),
            raw=data,
        )


@dataclass(frozen=True)
class InteriorContainerRecord:
    name: str
    palette_id: str | None
    container_transform: Matrix4
    placements: list[InteriorPlacementRecord]
    lights: list[LightRecord]
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> InteriorContainerRecord:
        data = _as_dict(value)
        return cls(
            name=str(data.get("name", "")),
            palette_id=_as_str(data.get("palette_id")),
            container_transform=_matrix4(data.get("container_transform")),
            placements=[InteriorPlacementRecord.from_value(item) for item in data.get("placements", [])],
            lights=[LightRecord.from_value(item) for item in data.get("lights", [])],
            raw=data,
        )


@dataclass(frozen=True)
class SceneInstanceRecord:
    entity_name: str
    geometry_path: str | None
    material_path: str | None
    material_sidecar: str | None
    mesh_asset: str | None
    palette_id: str | None
    parent_entity_name: str | None = None
    parent_node_name: str | None = None
    no_rotation: bool = False
    offset_position: Vec3 = (0.0, 0.0, 0.0)
    offset_rotation: Vec3 = (0.0, 0.0, 0.0)
    raw: JsonDict = field(default_factory=dict, repr=False)

    @classmethod
    def from_value(cls, value: Any) -> SceneInstanceRecord:
        data = _as_dict(value)
        return cls(
            entity_name=str(data.get("entity_name", "")),
            geometry_path=_normalize_relative_path(_as_str(data.get("geometry_path"))),
            material_path=_normalize_relative_path(_as_str(data.get("material_path"))),
            material_sidecar=_normalize_relative_path(_as_str(data.get("material_sidecar"))),
            mesh_asset=_normalize_relative_path(_as_str(data.get("mesh_asset"))),
            palette_id=_as_str(data.get("palette_id")),
            parent_entity_name=_as_str(data.get("parent_entity_name")),
            parent_node_name=_as_str(data.get("parent_node_name")),
            no_rotation=_as_bool(data.get("no_rotation")),
            offset_position=_float_tuple(data.get("offset_position"), 3),  # type: ignore[arg-type]
            offset_rotation=_float_tuple(data.get("offset_rotation"), 3),  # type: ignore[arg-type]
            raw=data,
        )


@dataclass(frozen=True)
class PackageRule:
    package_dir: str
    shared_asset_root: str
    normalized_p4k_relative_paths: bool
    paths_are_relative_to_export_root: bool
    root: str
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> PackageRule:
        data = _as_dict(value)
        return cls(
            package_dir=str(data.get("package_dir", "Packages")),
            shared_asset_root=str(data.get("shared_asset_root", "Data")),
            normalized_p4k_relative_paths=_as_bool(data.get("normalized_p4k_relative_paths")),
            paths_are_relative_to_export_root=_as_bool(data.get("paths_are_relative_to_export_root"), True),
            root=str(data.get("root", "caller_selected_export_root")),
            raw=data,
        )


@dataclass(frozen=True)
class SceneManifest:
    children: list[SceneInstanceRecord]
    interiors: list[InteriorContainerRecord]
    package_rule: PackageRule
    root_entity: SceneInstanceRecord
    version: int
    raw: JsonDict

    @classmethod
    def from_file(cls, path: Path) -> SceneManifest:
        return cls.from_value(_load_json(path))

    @classmethod
    def from_value(cls, value: Any) -> SceneManifest:
        data = _as_dict(value)
        return cls(
            children=[SceneInstanceRecord.from_value(item) for item in data.get("children", [])],
            interiors=[InteriorContainerRecord.from_value(item) for item in data.get("interiors", [])],
            package_rule=PackageRule.from_value(data.get("package_rule")),
            root_entity=SceneInstanceRecord.from_value(data.get("root_entity")),
            version=int(data.get("version", 1)),
            raw=data,
        )


def infer_export_root(scene_path: Path, package_dir: str) -> Path:
    export_root = scene_path.resolve().parent
    package_parts = Path(package_dir).parts
    if package_parts and export_root.name.casefold() != package_parts[-1].casefold():
        export_root = export_root.parent
    for _ in package_parts:
        export_root = export_root.parent
    return export_root


@dataclass
class PackageBundle:
    export_root: Path
    scene_path: Path
    scene: SceneManifest
    palettes: dict[str, PaletteRecord]
    liveries: dict[str, LiveryRecord]
    _material_cache: dict[str, MaterialSidecar] = field(default_factory=dict, repr=False)
    _path_index: dict[str, Path] | None = field(default=None, repr=False)

    @classmethod
    def load(cls, scene_path: str | Path) -> PackageBundle:
        scene_path = Path(scene_path).resolve()
        if not scene_path.is_file():
            raise FileNotFoundError(f"Scene manifest not found: {scene_path}")
        scene = SceneManifest.from_file(scene_path)
        export_root = infer_export_root(scene_path, scene.package_rule.package_dir)

        palettes_path = scene_path.with_name("palettes.json")
        liveries_path = scene_path.with_name("liveries.json")
        if not palettes_path.is_file():
            raise FileNotFoundError(f"Required palettes manifest not found: {palettes_path}")
        if not liveries_path.is_file():
            raise FileNotFoundError(f"Required liveries manifest not found: {liveries_path}")

        palettes_data = _load_json(palettes_path)
        liveries_data = _load_json(liveries_path)

        palettes = {
            palette.id: palette
            for palette in (PaletteRecord.from_value(item) for item in palettes_data.get("palettes", []))
        }
        liveries = {
            livery.id: livery
            for livery in (LiveryRecord.from_value(item) for item in liveries_data.get("liveries", []))
        }
        return cls(export_root=export_root, scene_path=scene_path, scene=scene, palettes=palettes, liveries=liveries)

    @property
    def package_name(self) -> str:
        return self.scene_path.parent.name

    def resolve_path(self, relative_path: str | None) -> Path | None:
        path_index = self._build_path_index()
        for candidate in _candidate_relative_paths(relative_path):
            direct = self.export_root / Path(candidate)
            if direct.exists():
                return direct
            resolved = path_index.get(candidate.lower())
            if resolved is not None:
                return resolved
        return None

    def load_material_sidecar(self, relative_path: str | None) -> MaterialSidecar | None:
        normalized = _normalize_relative_path(relative_path)
        if normalized is None:
            return None
        cached = self._material_cache.get(normalized)
        if cached is not None:
            return cached
        resolved = self.resolve_path(normalized)
        if resolved is None or not resolved.is_file():
            return None
        sidecar = MaterialSidecar.from_file(resolved)
        self._material_cache[normalized] = sidecar
        return sidecar

    def _build_path_index(self) -> dict[str, Path]:
        if self._path_index is not None:
            return self._path_index
        self._path_index = {}
        for candidate in self.export_root.rglob("*"):
            if not candidate.is_file():
                continue
            relative = candidate.relative_to(self.export_root).as_posix().lower()
            self._path_index.setdefault(relative, candidate)
        return self._path_index
