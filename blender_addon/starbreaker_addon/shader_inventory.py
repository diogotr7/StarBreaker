from __future__ import annotations

from dataclasses import dataclass
import argparse
import json
from pathlib import Path
from typing import Any

if __package__ in {None, ""}:
    import sys

    PACKAGE_ROOT = Path(__file__).resolve().parents[1]
    if str(PACKAGE_ROOT) not in sys.path:
        sys.path.insert(0, str(PACKAGE_ROOT))

    from starbreaker_addon.manifest import MaterialSidecar, SubmaterialRecord
else:
    from .manifest import MaterialSidecar, SubmaterialRecord


def material_sidecar_paths(root: Path) -> list[Path]:
    return sorted(path for path in root.rglob("*.materials.json") if path.is_file())


def _relative_path(path: Path, root: Path | None) -> str:
    if root is None:
        return path.as_posix()
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def _palette_channel_names(submaterial: SubmaterialRecord) -> set[str]:
    channels: set[str] = set()
    if submaterial.palette_routing.material_channel is not None:
        channels.add(submaterial.palette_routing.material_channel.name)
    for binding in submaterial.palette_routing.layer_channels:
        channels.add(binding.channel.name)
    for layer in submaterial.layer_manifest:
        if layer.palette_channel is not None:
            channels.add(layer.palette_channel.name)
    return channels


@dataclass(frozen=True)
class ShaderFamilyInventoryEntry:
    shader_family: str
    shaders: list[str]
    texture_slots: list[str]
    texture_roles: list[str]
    public_params: list[str]
    palette_channels: list[str]
    virtual_inputs: list[str]
    max_layer_count: int
    submaterial_count: int
    sample_sidecars: list[str]
    sample_submaterials: list[str]

    def to_dict(self) -> dict[str, Any]:
        return {
            "shader_family": self.shader_family,
            "shaders": self.shaders,
            "texture_slots": self.texture_slots,
            "texture_roles": self.texture_roles,
            "public_params": self.public_params,
            "palette_channels": self.palette_channels,
            "virtual_inputs": self.virtual_inputs,
            "max_layer_count": self.max_layer_count,
            "submaterial_count": self.submaterial_count,
            "sample_sidecars": self.sample_sidecars,
            "sample_submaterials": self.sample_submaterials,
        }


@dataclass(frozen=True)
class ShaderInventory:
    export_root: str | None
    sidecar_count: int
    families: list[ShaderFamilyInventoryEntry]

    @classmethod
    def from_sidecar_paths(cls, sidecar_paths: list[Path], export_root: Path | None = None) -> ShaderInventory:
        accumulators: dict[str, dict[str, Any]] = {}
        for sidecar_path in sidecar_paths:
            sidecar = MaterialSidecar.from_file(sidecar_path)
            sidecar_name = _relative_path(sidecar_path, export_root)
            for submaterial in sidecar.submaterials:
                family = submaterial.shader_family or "Unknown"
                entry = accumulators.setdefault(
                    family,
                    {
                        "shaders": set(),
                        "texture_slots": set(),
                        "texture_roles": set(),
                        "public_params": set(),
                        "palette_channels": set(),
                        "virtual_inputs": set(),
                        "max_layer_count": 0,
                        "submaterial_count": 0,
                        "sample_sidecars": [],
                        "sample_submaterials": [],
                    },
                )
                entry["shaders"].add(submaterial.shader)
                entry["texture_slots"].update(texture.slot for texture in submaterial.texture_slots if texture.slot)
                entry["texture_roles"].update(texture.role for texture in submaterial.texture_slots if texture.role)
                entry["public_params"].update(submaterial.public_params.keys())
                entry["palette_channels"].update(_palette_channel_names(submaterial))
                entry["virtual_inputs"].update(submaterial.virtual_inputs)
                entry["max_layer_count"] = max(entry["max_layer_count"], len(submaterial.layer_manifest))
                entry["submaterial_count"] += 1
                if sidecar_name not in entry["sample_sidecars"]:
                    entry["sample_sidecars"].append(sidecar_name)
                if submaterial.submaterial_name and submaterial.submaterial_name not in entry["sample_submaterials"]:
                    entry["sample_submaterials"].append(submaterial.submaterial_name)

        families = [
            ShaderFamilyInventoryEntry(
                shader_family=shader_family,
                shaders=sorted(value["shaders"]),
                texture_slots=sorted(value["texture_slots"]),
                texture_roles=sorted(value["texture_roles"]),
                public_params=sorted(value["public_params"]),
                palette_channels=sorted(value["palette_channels"]),
                virtual_inputs=sorted(value["virtual_inputs"]),
                max_layer_count=int(value["max_layer_count"]),
                submaterial_count=int(value["submaterial_count"]),
                sample_sidecars=sorted(value["sample_sidecars"]),
                sample_submaterials=sorted(value["sample_submaterials"]),
            )
            for shader_family, value in sorted(accumulators.items())
        ]
        return cls(
            export_root=export_root.as_posix() if export_root is not None else None,
            sidecar_count=len(sidecar_paths),
            families=families,
        )

    @classmethod
    def from_export_root(cls, export_root: Path) -> ShaderInventory:
        return cls.from_sidecar_paths(material_sidecar_paths(export_root), export_root=export_root)

    def family(self, shader_family: str) -> ShaderFamilyInventoryEntry | None:
        for entry in self.families:
            if entry.shader_family == shader_family:
                return entry
        return None

    def to_dict(self) -> dict[str, Any]:
        return {
            "export_root": self.export_root,
            "sidecar_count": self.sidecar_count,
            "families": [entry.to_dict() for entry in self.families],
        }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Summarize shader-family usage from exported StarBreaker material sidecars.")
    parser.add_argument("root", type=Path, help="Export root or folder to scan for *.materials.json sidecars")
    parser.add_argument("--output", type=Path, help="Optional output path for the generated JSON summary")
    args = parser.parse_args(argv)

    inventory = ShaderInventory.from_export_root(args.root)
    payload = json.dumps(inventory.to_dict(), indent=2, sort_keys=True)
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(payload + "\n", encoding="utf-8")
    else:
        print(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
