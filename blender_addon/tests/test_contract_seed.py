from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent

sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.contract_seed import build_seed_contract, group_name_for_shader_family
from starbreaker_addon.shader_inventory import ShaderInventory


ARGO_SIDECAR_DIR = REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/ARGO/MOLE"
ARGO_EXTERIOR = ARGO_SIDECAR_DIR / "argo_mole_exterior.materials.json"
ARGO_INTERIOR = ARGO_SIDECAR_DIR / "argo_mole_interior.materials.json"


class ContractSeedTests(unittest.TestCase):
    def test_group_name_uses_stable_shader_family_prefix(self) -> None:
        self.assertEqual(group_name_for_shader_family("HardSurface"), "SB_HardSurface_v1")
        self.assertEqual(group_name_for_shader_family("LayerBlend_V2"), "SB_LayerBlend_V2_v1")
        self.assertEqual(group_name_for_shader_family("UI Plane"), "SB_UI_Plane_v1")

    def test_seed_contract_uses_inventory_metadata_without_guessing_inputs(self) -> None:
        inventory = ShaderInventory.from_sidecar_paths(
            [ARGO_EXTERIOR, ARGO_INTERIOR],
            export_root=REPO_ROOT / "ships",
        )
        contract = build_seed_contract(inventory, source_label="ships/Data")
        hard_surface = contract.group_for_shader_family("HardSurface")

        self.assertEqual(contract.metadata.get("status"), "seed")
        self.assertEqual(contract.generated_from, "ships/Data")
        self.assertIsNotNone(hard_surface)
        self.assertEqual(hard_surface.name, "SB_HardSurface_v1")
        self.assertEqual(hard_surface.inputs[0].name, "TexSlot1_BaseColor")
        self.assertEqual(hard_surface.inputs[0].semantic, "base_color")
        self.assertEqual(hard_surface.metadata.get("status"), "seed")
        self.assertIn("TexSlot1", hard_surface.metadata.get("texture_slots", []))
        self.assertIn("primary", hard_surface.metadata.get("palette_channels", []))
        self.assertIn("Palette_Primary", hard_surface.metadata.get("proposed_palette_inputs", []))

    def test_seed_contract_includes_documented_slots_not_present_in_fixture_inventory(self) -> None:
        inventory = ShaderInventory.from_sidecar_paths(
            [ARGO_EXTERIOR, ARGO_INTERIOR],
            export_root=REPO_ROOT / "ships",
        )
        contract = build_seed_contract(inventory, source_label="ships/Data")
        hard_surface = contract.group_for_shader_family("HardSurface")
        glass = contract.group_for_shader_family("GlassPBR")

        self.assertIsNotNone(hard_surface)
        self.assertIsNotNone(glass)
        self.assertIn("TexSlot10_IridescenceColor", [item.name for item in hard_surface.inputs])
        self.assertIn("TexSlot15_CondensationNormal", [item.name for item in glass.inputs])

    def test_contract_seed_script_help_runs_from_file_path(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ADDON_ROOT / "starbreaker_addon" / "contract_seed.py"), "--help"],
            capture_output=True,
            text=True,
            cwd=STARBREAKER_ROOT,
            check=False,
        )

        self.assertEqual(result.returncode, 0, msg=result.stderr)
        self.assertIn("Generate a seed material template contract", result.stdout)



if __name__ == "__main__":
    unittest.main()