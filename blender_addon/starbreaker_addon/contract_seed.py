from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

if __package__ in {None, ""}:
    import sys

    PACKAGE_ROOT = Path(__file__).resolve().parents[1]
    if str(PACKAGE_ROOT) not in sys.path:
        sys.path.insert(0, str(PACKAGE_ROOT))

    from starbreaker_addon.contract_naming import (
        known_texture_slots,
        palette_input_name,
        public_param_input_name,
        texture_input_name,
        texture_input_semantic,
        virtual_input_name,
    )
    from starbreaker_addon.material_contract import TemplateContract
    from starbreaker_addon.shader_inventory import ShaderInventory
else:
    from .contract_naming import (
        known_texture_slots,
        palette_input_name,
        public_param_input_name,
        texture_input_name,
        texture_input_semantic,
        virtual_input_name,
    )
    from .material_contract import TemplateContract
    from .shader_inventory import ShaderInventory


NON_IDENTIFIER_CHARS = re.compile(r"[^0-9A-Za-z_]+")


def _sanitized_shader_family(shader_family: str) -> str:
    sanitized = NON_IDENTIFIER_CHARS.sub("_", shader_family).strip("_")
    return sanitized or "Unknown"


def group_name_for_shader_family(shader_family: str, version: int = 1) -> str:
    return f"SB_{_sanitized_shader_family(shader_family)}_v{version}"


def merged_texture_slots(shader_family: str, inventory_slots: list[str]) -> list[str]:
    return sorted({*inventory_slots, *known_texture_slots(shader_family)})


def build_seed_contract(inventory: ShaderInventory, source_label: str | None = None) -> TemplateContract:
    groups = []
    for entry in inventory.families:
        texture_slots = merged_texture_slots(entry.shader_family, entry.texture_slots)
        inputs = [
            {
                "name": texture_input_name(entry.shader_family, slot),
                "socket_type": "NodeSocketColor",
                "semantic": texture_input_semantic(entry.shader_family, slot),
                "source_slot": slot,
                "required": False,
            }
            for slot in texture_slots
        ]
        groups.append(
            {
                "name": group_name_for_shader_family(entry.shader_family),
                "shader_families": [entry.shader_family],
                "version": 1,
                "shader_output": "Shader",
                "inputs": inputs,
                "metadata": {
                    "status": "seed",
                    "texture_slots": texture_slots,
                    "texture_roles": entry.texture_roles,
                    "public_params": entry.public_params,
                    "proposed_public_inputs": [public_param_input_name(name) for name in entry.public_params],
                    "palette_channels": entry.palette_channels,
                    "proposed_palette_inputs": [palette_input_name(name) for name in entry.palette_channels],
                    "virtual_inputs": entry.virtual_inputs,
                    "proposed_virtual_inputs": [virtual_input_name(name) for name in entry.virtual_inputs],
                    "max_layer_count": entry.max_layer_count,
                    "submaterial_count": entry.submaterial_count,
                    "sample_sidecars": entry.sample_sidecars,
                    "sample_submaterials": entry.sample_submaterials,
                    "note": "Seed contract generated from sidecar inventory plus the documented slot naming rules. These inputs are authoring proposals and still need Blender node-group implementation.",
                },
            }
        )
    generated_from = source_label or inventory.export_root or "shader_inventory"
    return TemplateContract.from_value(
        {
            "schema_version": 1,
            "generated_from": generated_from,
            "groups": groups,
            "metadata": {
                "status": "seed",
                "seed_source": "shader_inventory",
                "sidecar_count": inventory.sidecar_count,
                "note": "This is a generated authoring seed, not the final contract exported from the bundled .blend library.",
            },
        }
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Generate a seed material template contract from shader inventory.")
    parser.add_argument("root", type=Path, help="Export root or folder to scan for *.materials.json sidecars")
    parser.add_argument("--output", type=Path, required=True, help="Output path for the generated seed contract JSON")
    parser.add_argument(
        "--source-label",
        default=None,
        help="Optional human-readable source label written into generated_from",
    )
    args = parser.parse_args(argv)

    inventory = ShaderInventory.from_export_root(args.root)
    contract = build_seed_contract(inventory, source_label=args.source_label)
    payload = json.dumps(contract.to_dict(), indent=2, sort_keys=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(payload + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
