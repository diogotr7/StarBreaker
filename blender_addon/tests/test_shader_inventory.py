from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent

sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.shader_inventory import ShaderInventory, material_sidecar_paths


ARGO_SIDECARE_DIR = REPO_ROOT / "ships/Data/objects/spaceships/ships/argo/mole"
ARGO_EXTERIOR = ARGO_SIDECARE_DIR / "argo_mole_exterior.materials.json"
ARGO_INTERIOR = ARGO_SIDECARE_DIR / "argo_mole_interior.materials.json"


_requires_argo_sidecars = unittest.skipUnless(
    ARGO_EXTERIOR.is_file() and ARGO_INTERIOR.is_file(),
    "ARGO MOLE sidecar fixtures not present; skipping shader inventory test",
)


class ShaderInventoryTests(unittest.TestCase):
    @_requires_argo_sidecars
    def test_material_sidecar_paths_discovers_fixture_sidecars(self) -> None:
        paths = material_sidecar_paths(ARGO_SIDECARE_DIR)
        self.assertIn(ARGO_EXTERIOR, paths)
        self.assertIn(ARGO_INTERIOR, paths)

    @_requires_argo_sidecars
    def test_inventory_summarizes_fixture_shader_families(self) -> None:
        inventory = ShaderInventory.from_sidecar_paths(
            [ARGO_EXTERIOR, ARGO_INTERIOR],
            export_root=REPO_ROOT / "ships",
        )
        hard_surface = inventory.family("HardSurface")
        glass = inventory.family("GlassPBR")
        ui_plane = inventory.family("UIPlane")

        self.assertIsNotNone(hard_surface)
        self.assertIsNotNone(glass)
        self.assertIsNotNone(ui_plane)
        self.assertIn("TexSlot1", hard_surface.texture_slots)
        self.assertIn("TexSlot3", hard_surface.texture_slots)
        self.assertIn("primary", hard_surface.palette_channels)
        self.assertIn("secondary", hard_surface.palette_channels)
        self.assertGreaterEqual(hard_surface.submaterial_count, 1)

    @_requires_argo_sidecars
    def test_inventory_to_dict_is_json_ready(self) -> None:
        inventory = ShaderInventory.from_sidecar_paths([ARGO_EXTERIOR], export_root=REPO_ROOT / "ships")
        payload = inventory.to_dict()
        self.assertEqual(payload["sidecar_count"], 1)
        self.assertTrue(any(entry["shader_family"] == "HardSurface" for entry in payload["families"]))

    @_requires_argo_sidecars
    def test_inventory_preserves_screen_families_from_fixture_sidecar(self) -> None:
        inventory = ShaderInventory.from_sidecar_paths([ARGO_INTERIOR], export_root=REPO_ROOT / "ships")
        monitor = inventory.family("DisplayScreen")

        self.assertIsNotNone(monitor)
        self.assertEqual(monitor.shaders, ["DisplayScreen"])
        self.assertIn("TexSlot9", monitor.texture_slots)

    def test_shader_inventory_script_help_runs_from_file_path(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ADDON_ROOT / "starbreaker_addon" / "shader_inventory.py"), "--help"],
            capture_output=True,
            text=True,
            cwd=STARBREAKER_ROOT,
            check=False,
        )

        self.assertEqual(result.returncode, 0, msg=result.stderr)
        self.assertIn("Summarize shader-family usage", result.stdout)


if __name__ == "__main__":
    unittest.main()