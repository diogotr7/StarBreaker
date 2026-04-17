# Blender Material Template Authoring Workflow

## Goal

This workflow keeps `material_templates.blend` editable in Blender while preserving a machine-readable contract beside it.

The current Phase 1 tooling lives in:

- `blender_addon/scripts/build_material_template_library.py`
- `blender_addon/scripts/export_material_template_contract.py`
- `blender_addon/scripts/validate_material_template_library.py`
- `blender_addon/scripts/author_phase2_core_groups.py`

## Compatibility Rule

Author and save `material_templates.blend` in the current Blender LTS release.

Do not make a 5.x-only file the canonical source if the add-on still needs LTS compatibility.

## Canonical Files

- library file:
  `blender_addon/starbreaker_addon/resources/material_templates.blend`
- generated contract:
  `blender_addon/starbreaker_addon/resources/material_template_contract.json`

The `.blend` file is the canonical source.
The JSON contract is derived from it.

## Safe Editing Loop

1. Open `material_templates.blend` in Blender LTS.
2. Edit only the `SB_*_v1` node groups or their shared helper groups.
3. Preserve the existing top-level group names and the `TexSlotN_*`, `Param_*`, `Palette_*`, and `Virtual_*` interface naming rules unless the contract is being intentionally revised.
4. Save the `.blend` file.
5. Run the export script from Blender to regenerate `material_template_contract.json`.
6. Run the validation script from Blender to confirm no required groups or sockets were lost.

## Script Expectations

### Build Script

`build_material_template_library.py` creates a starter library from the current contract and writes it with `bpy.data.libraries.write`.

Use it when bootstrapping or re-creating the library structure from scratch.

### Export Script

`export_material_template_contract.py` reads the live `SB_*` node groups in the open `.blend` file and writes the generated contract JSON beside the library.

This is the source-of-truth refresh step after interface edits.

### Validation Script

`validate_material_template_library.py` compares the live `SB_*` groups in the open `.blend` against the generated contract file and fails if required groups or sockets are missing or renamed.

Run it after export and before relying on the library in runtime code.

### Phase 2 Authoring Script

`author_phase2_core_groups.py` applies the current starter node graphs for the core authored families in the open library file.

Right now that covers:

- `SB_HardSurface_v1`
- `SB_GlassPBR_v1`
- `SB_Illum_v1`
- `SB_NoDraw_v1`
- `SB_Unknown_v1`

This script is the repeatable starting point for the first material-library internals. Run it before manual tuning if the core groups need to be reset to the current baseline.

The current runtime importer consumes the exported contract and instantiates these bundled groups for the authored core families instead of rebuilding those materials entirely with ad hoc node trees.

For the current core groups, explicit `Palette_*` sockets are now part of the library interface where the contract metadata says palette routing is required. The runtime connects those sockets directly instead of baking palette tint into the texture input before the group.

## Editing Rules

- Do not silently rename top-level shader groups.
- Do not remove or rename interface sockets without re-exporting and validating the contract.
- Keep the original `TexSlotN` prefix in texture inputs.
- Keep public parameters and palette inputs traceable to their original exported identities.
- If a family needs to split into variants, add new explicit groups rather than overloading one ambiguous interface.

## Current Status

Phase 1 currently has:

- a checked-in `material_templates.blend`
- a Blender-driven contract export path
- a Blender-driven validation path

The next step after Phase 1 is authoring real shader logic inside the groups while keeping the contract and validation loop intact.