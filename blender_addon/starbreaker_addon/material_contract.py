from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
from typing import Any, Mapping

JsonDict = dict[str, Any]

SHADER_FAMILY_ALIASES: dict[str, tuple[str, ...]] = {
    "Monitor": ("Unknown",),
}

RESOURCES_DIR = Path(__file__).with_name("resources")
BUNDLED_TEMPLATE_LIBRARY_NAME = "material_templates.blend"
BUNDLED_TEMPLATE_CONTRACT_NAME = "material_template_contract.json"


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


def bundled_template_resource_dir() -> Path:
    return RESOURCES_DIR


def bundled_template_library_path() -> Path:
    return bundled_template_resource_dir() / BUNDLED_TEMPLATE_LIBRARY_NAME


def bundled_template_contract_path() -> Path:
    return bundled_template_resource_dir() / BUNDLED_TEMPLATE_CONTRACT_NAME


@dataclass(frozen=True)
class ContractInput:
    name: str
    socket_type: str
    semantic: str | None
    source_slot: str | None
    required: bool
    default_value: Any
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> ContractInput:
        data = _as_dict(value)
        return cls(
            name=str(data.get("name", "")),
            socket_type=str(data.get("socket_type", "NodeSocketColor")),
            semantic=_as_str(data.get("semantic")),
            source_slot=_as_str(data.get("source_slot")),
            required=_as_bool(data.get("required"), default=False),
            default_value=data.get("default_value"),
            raw=data,
        )

    def to_dict(self) -> JsonDict:
        return {
            "name": self.name,
            "socket_type": self.socket_type,
            "semantic": self.semantic,
            "source_slot": self.source_slot,
            "required": self.required,
            "default_value": self.default_value,
        }


@dataclass(frozen=True)
class ShaderGroupContract:
    name: str
    shader_families: list[str]
    version: int
    shader_output: str
    inputs: list[ContractInput]
    metadata: JsonDict
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> ShaderGroupContract:
        data = _as_dict(value)
        return cls(
            name=str(data.get("name", "")),
            shader_families=[str(item) for item in data.get("shader_families", [])],
            version=int(data.get("version", 1)),
            shader_output=str(data.get("shader_output", "Shader")),
            inputs=[ContractInput.from_value(item) for item in data.get("inputs", [])],
            metadata=_as_dict(data.get("metadata")),
            raw=data,
        )

    def supports_shader_family(self, shader_family: str) -> bool:
        return shader_family in self.shader_families

    def to_dict(self) -> JsonDict:
        return {
            "name": self.name,
            "shader_families": self.shader_families,
            "version": self.version,
            "shader_output": self.shader_output,
            "inputs": [item.to_dict() for item in self.inputs],
            "metadata": self.metadata,
        }


@dataclass(frozen=True)
class TemplateContract:
    schema_version: int
    generated_from: str | None
    groups: list[ShaderGroupContract]
    metadata: JsonDict
    raw: JsonDict

    @classmethod
    def from_value(cls, value: Any) -> TemplateContract:
        data = _as_dict(value)
        return cls(
            schema_version=int(data.get("schema_version", 1)),
            generated_from=_as_str(data.get("generated_from")),
            groups=[ShaderGroupContract.from_value(item) for item in data.get("groups", [])],
            metadata=_as_dict(data.get("metadata")),
            raw=data,
        )

    @classmethod
    def from_file(cls, path: Path) -> TemplateContract:
        with path.open("r", encoding="utf-8") as handle:
            return cls.from_value(json.load(handle))

    def group_for_shader_family(self, shader_family: str) -> ShaderGroupContract | None:
        candidates = [shader_family, *SHADER_FAMILY_ALIASES.get(shader_family, ())]
        for candidate in candidates:
            for group in self.groups:
                if group.supports_shader_family(candidate):
                    return group
        return None

    def to_dict(self) -> JsonDict:
        return {
            "schema_version": self.schema_version,
            "generated_from": self.generated_from,
            "groups": [group.to_dict() for group in self.groups],
            "metadata": self.metadata,
        }


def load_bundled_template_contract(path: Path | None = None) -> TemplateContract:
    contract_path = path or bundled_template_contract_path()
    return TemplateContract.from_file(contract_path)

