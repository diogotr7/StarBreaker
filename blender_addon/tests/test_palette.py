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
    palette_finish_glossiness,
    palette_finish_specular,
    palette_for_id,
    palette_id_for_livery_instance,
    palette_signature_for_submaterial,
    resolved_palette_id,
)


ARGO_SCENE = REPO_ROOT / "ships/Packages/ARGO MOLE/scene.json"


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

    def test_palette_finish_preserves_specular_data(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        palette = palette_for_id(package, "palette/argo_mole")

        self.assertEqual(
            palette_finish_specular(palette, "primary"),
            (0.04373502731323242, 0.04373502731323242, 0.04373502731323242),
        )
        self.assertIsNone(palette_finish_glossiness(palette, "primary"))

    def test_null_child_palette_inherits_package_root_palette(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        child = next(scene_child for scene_child in package.scene.children if scene_child.palette_id is None)
        self.assertIsNone(child.palette_id)
        self.assertEqual(
            resolved_palette_id(package, child.palette_id, package.scene.root_entity.palette_id),
            "palette/argo_mole",
        )

        inherited_palette = palette_for_id(package, child.palette_id, package.scene.root_entity.palette_id)
        self.assertIsNotNone(inherited_palette)
        self.assertEqual(inherited_palette.id, "palette/argo_mole")

    def test_palette_signature_reuses_same_glass_color_across_palettes(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        sidecar = package.load_material_sidecar("Data/objects/spaceships/ships/argo/mole/argo_mole_interior.materials.json")
        self.assertIsNotNone(sidecar)

        glass = next(submaterial for submaterial in sidecar.submaterials if submaterial.submaterial_name == "glass_interior_canopy")
        default_palette = palette_for_id(package, "palette/default")
        argo_palette = palette_for_id(package, "palette/argo_mole")

        self.assertEqual(
            palette_signature_for_submaterial(glass, default_palette),
            palette_signature_for_submaterial(glass, argo_palette),
        )

    def test_palette_signature_keeps_distinct_primary_colors_split(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        sidecar = package.load_material_sidecar("Data/objects/spaceships/ships/argo/mole/argo_mole_exterior.materials.json")
        self.assertIsNotNone(sidecar)

        paint = next(
            submaterial
            for submaterial in sidecar.submaterials
            if submaterial.palette_routing.material_channel is not None
            and submaterial.palette_routing.material_channel.name == "primary"
        )
        default_palette = palette_for_id(package, "palette/default")
        argo_palette = palette_for_id(package, "palette/argo_mole")

        self.assertNotEqual(
            palette_signature_for_submaterial(paint, default_palette),
            palette_signature_for_submaterial(paint, argo_palette),
        )


if __name__ == "__main__":
    unittest.main()
