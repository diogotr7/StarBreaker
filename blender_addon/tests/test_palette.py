from __future__ import annotations

from pathlib import Path
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent

sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.manifest import PackageBundle
from starbreaker_addon.palette import (
    available_livery_ids,
    available_palette_ids,
    default_palette_id,
    livery_applies_to_instance,
    palette_color,
    palette_for_id,
    palette_id_for_livery_instance,
)


ARGO_SCENE = REPO_ROOT / "ships/Packages/Argo MOLE/scene.json"


class PaletteTests(unittest.TestCase):
    def test_available_ids_are_loaded_from_fixture_manifests(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        self.assertIn("palette/argo_mole", available_palette_ids(package))
        self.assertIn("palette/default", available_livery_ids(package))

    def test_default_palette_prefers_explicit_default(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        self.assertEqual(default_palette_id(package), "palette/default")

    def test_livery_matching_uses_entity_and_sidecar_identity(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        livery = package.liveries["palette/argo_mole"]
        child = package.scene.children[0]
        self.assertTrue(livery_applies_to_instance(livery, child, child.material_sidecar))

    def test_livery_can_override_instance_palette(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        child = package.scene.children[0]
        palette_id = palette_id_for_livery_instance(package, "palette/default", child, child.material_sidecar)
        self.assertEqual(palette_id, child.palette_id)

        palette_id = palette_id_for_livery_instance(package, "palette/argo_mole", child, child.material_sidecar)
        self.assertEqual(palette_id, "palette/argo_mole")

    def test_palette_color_returns_named_channels(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        palette = palette_for_id(package, "palette/argo_mole")
        self.assertIsNotNone(palette)
        self.assertEqual(palette_color(palette, "glass"), palette.glass)


if __name__ == "__main__":
    unittest.main()
