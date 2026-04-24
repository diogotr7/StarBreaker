from __future__ import annotations

from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent

sys.path.insert(0, str(ADDON_ROOT))


if "starbreaker_addon" not in sys.modules:
    package = types.ModuleType("starbreaker_addon")
    package.__path__ = [str(ADDON_ROOT / "starbreaker_addon")]
    sys.modules["starbreaker_addon"] = package

if "starbreaker_addon.runtime" not in sys.modules:
    runtime_package = types.ModuleType("starbreaker_addon.runtime")
    runtime_package.__path__ = [str(ADDON_ROOT / "starbreaker_addon" / "runtime")]
    sys.modules["starbreaker_addon.runtime"] = runtime_package


if "mathutils" not in sys.modules:
    mathutils = types.ModuleType("mathutils")

    class Matrix(tuple):
        def __new__(cls, rows):
            return tuple.__new__(cls, rows)

        def inverted(self):
            return self

    class Quaternion(tuple):
        def __new__(cls, values):
            return tuple.__new__(cls, values)

    class Euler(tuple):
        def __new__(cls, values, order='XYZ'):
            return tuple.__new__(cls, values)

    mathutils.Matrix = Matrix
    mathutils.Quaternion = Quaternion
    mathutils.Euler = Euler
    sys.modules["mathutils"] = mathutils


if "bpy" not in sys.modules:
    bpy = types.ModuleType("bpy")
    bpy.types = types.SimpleNamespace(Material=object, Node=object, ShaderNodeTree=object)
    bpy.data = types.SimpleNamespace(node_groups=[], images=[])
    sys.modules["bpy"] = bpy

from starbreaker_addon.manifest import PackageBundle
from starbreaker_addon.palette import (
    available_livery_ids,
    available_palette_ids,
    default_palette_id,
    livery_applies_to_instance,
    paint_list_canonical_id,
    palette_color,
    palette_finish_glossiness,
    palette_finish_specular,
    palette_for_id,
    palette_id_for_livery_instance,
    palette_signature_for_submaterial,
    resolved_palette_id,
)
from starbreaker_addon.runtime.constants import PROP_PALETTE_ID
from starbreaker_addon.runtime.importer.orchestration import OrchestrationMixin
from starbreaker_addon.runtime.palette_utils import (
    _hard_surface_palette_iridescence_channel,
    _palette_channel_has_iridescence,
    _palette_has_iridescence,
)


def _existing_scene(*relative_paths: str) -> Path:
    candidates = [REPO_ROOT / relative_path for relative_path in relative_paths]
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return candidates[0]


ARGO_SCENE = _existing_scene(
    "ships/Packages/ARGO MOLE/scene.json",
    "ships/Packages/ARGO MOLE_LOD0_TEX0/scene.json",
)
VULTURE_SCENE = _existing_scene(
    "ships/Packages/Drake Vulture/scene.json",
    "ships/Packages/DRAK Vulture_LOD0_TEX0/scene.json",
)
AURORA_SCENE = _existing_scene(
    "ships/Packages/RSI Aurora Mk2/scene.json",
    "ships/Packages/RSI Aurora Mk2_LOD0_TEX0/scene.json",
)


@unittest.skipUnless(
    ARGO_SCENE.is_file(),
    f"ARGO MOLE fixture not present at {ARGO_SCENE}; skipping palette tests",
)
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

    def test_resolved_palette_id_maps_mole_paint_variant_alias(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)

        self.assertEqual(
            resolved_palette_id(package, "palette/mole_bronze_black_brown"),
            "palette/argo_mole_july_bronze_black_brown",
        )

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


@unittest.skipUnless(
    VULTURE_SCENE.is_file(),
    f"Drake Vulture fixture not present at {VULTURE_SCENE}; skipping Vulture palette tests",
)
class VulturePaletteTests(unittest.TestCase):
    def test_resolved_palette_id_preserves_existing_vulture_paint_variant(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)

        self.assertEqual(
            resolved_palette_id(package, "palette/vulture_carnival_pink_black"),
            "palette/vulture_carnival_pink_black",
        )

    def test_iridescence_detects_tertiary_specular_highlight(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)
        palette = palette_for_id(package, "palette/drak_vulture_carnival_pink_black")

        self.assertIsNotNone(palette)
        self.assertTrue(_palette_has_iridescence(palette))

    def test_iridescence_ignores_grayscale_tertiary_specular(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)
        palette = palette_for_id(package, "palette/drak_vulture_assembly_red_white")

        self.assertIsNotNone(palette)
        self.assertFalse(_palette_has_iridescence(palette))

    def test_iridescence_detects_primary_specular_highlight(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)
        palette = palette_for_id(package, "palette/vulture_ghoulish_green")

        self.assertIsNotNone(palette)
        self.assertTrue(_palette_channel_has_iridescence(palette, "primary"))

    def test_paint_list_keeps_unresolved_hidden_paint_distinct(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)

        self.assertEqual(
            paint_list_canonical_id(package, "palette/vulture_ghoulish_green"),
            "palette/vulture_ghoulish_green",
        )

    def test_paint_list_prefers_vulture_paint_id_over_duplicate_palette_aliases(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)

        self.assertEqual(
            paint_list_canonical_id(package, "palette/drak_vulture_carnival_pink_black"),
            "palette/vulture_carnival_pink_black",
        )
        self.assertEqual(
            paint_list_canonical_id(package, "palette/drak_vulture_assembly_red_white"),
            "palette/vulture_assembly_red_white",
        )


@unittest.skipUnless(
    AURORA_SCENE.is_file(),
    f"RSI Aurora Mk2 fixture not present at {AURORA_SCENE}; skipping Aurora palette tests",
)
class AuroraPaletteTests(unittest.TestCase):
    def test_hard_surface_iridescence_stays_off_for_primary_paint_when_only_tertiary_is_chromatic(self) -> None:
        package = PackageBundle.load(AURORA_SCENE)
        palette = palette_for_id(package, "palette/rsi_aurora_mk2")

        self.assertIsNotNone(palette)
        self.assertFalse(_palette_channel_has_iridescence(palette, "primary"))
        self.assertTrue(_palette_channel_has_iridescence(palette, "tertiary"))
        self.assertIsNone(
            _hard_surface_palette_iridescence_channel(
                palette,
                "primary",
                authored_angle_shift=False,
            )
        )

    def test_hard_surface_authored_angle_shift_can_fall_back_to_tertiary_palette_channel(self) -> None:
        package = PackageBundle.load(AURORA_SCENE)
        palette = palette_for_id(package, "palette/rsi_aurora_mk2")

        self.assertIsNotNone(palette)
        self.assertEqual(
            _hard_surface_palette_iridescence_channel(
                palette,
                "primary",
                authored_angle_shift=True,
            ),
            "tertiary",
        )


class PaletteOverrideImporterUnderTest(OrchestrationMixin):
    def __init__(self, package: PackageBundle, *, package_root=None, import_palette_override: str | None = None):
        self.package = package
        self.package_root = package_root
        self.import_palette_override = import_palette_override


@unittest.skipUnless(
    VULTURE_SCENE.is_file(),
    f"Drake Vulture fixture not present at {VULTURE_SCENE}; skipping palette override tests",
)
class PaletteOverrideRoutingTests(unittest.TestCase):
    def test_exterior_override_replaces_root_default_palette(self) -> None:
        package = PackageBundle.load(VULTURE_SCENE)
        importer = PaletteOverrideImporterUnderTest(
            package,
            package_root={PROP_PALETTE_ID: "palette/drak_vulture_carnival_pink_black"},
            import_palette_override="palette/drak_vulture_carnival_pink_black",
        )

        self.assertEqual(
            importer._effective_palette_id(package.scene.root_entity.palette_id),
            "palette/drak_vulture_carnival_pink_black",
        )


if __name__ == "__main__":
    unittest.main()
