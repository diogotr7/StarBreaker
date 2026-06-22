from __future__ import annotations

from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))


for package_name in (
    "starbreaker_addon",
    "starbreaker_addon.runtime",
    "starbreaker_addon.runtime.importer",
):
    if package_name not in sys.modules:
        package = types.ModuleType(package_name)
        package.__path__ = [str(ADDON_ROOT / Path(*package_name.split(".")))]
        sys.modules[package_name] = package


from starbreaker_addon.runtime.importer.roughness import (
    combine_glossiness_signals,
    ddna_and_palette_gloss_to_roughness,
    ddna_smoothness_to_roughness,
    glossiness_to_perceptual_roughness,
    roughness_and_palette_gloss_to_roughness,
)


class DdnaRoughnessTests(unittest.TestCase):
    def test_zero_smoothness_maps_to_full_roughness(self) -> None:
        self.assertAlmostEqual(ddna_smoothness_to_roughness(0.0), 1.0)

    def test_full_smoothness_maps_to_zero_roughness(self) -> None:
        self.assertAlmostEqual(ddna_smoothness_to_roughness(1.0), 0.0)

    def test_mid_smoothness_uses_perceptual_invert(self) -> None:
        self.assertAlmostEqual(ddna_smoothness_to_roughness(0.25), 0.8660254037844386)

    def test_input_is_clamped_to_unit_interval(self) -> None:
        self.assertAlmostEqual(ddna_smoothness_to_roughness(-3.0), 1.0)
        self.assertAlmostEqual(ddna_smoothness_to_roughness(2.0), 0.0)

    def test_glossiness_to_perceptual_roughness_matches_ddna_helper(self) -> None:
        self.assertAlmostEqual(
            glossiness_to_perceptual_roughness(0.25),
            ddna_smoothness_to_roughness(0.25),
        )

    def test_combine_glossiness_signals_preserves_endpoint_behavior(self) -> None:
        self.assertAlmostEqual(combine_glossiness_signals(0.0, 0.0), 0.0)
        self.assertAlmostEqual(combine_glossiness_signals(1.0, 0.0), 1.0)
        self.assertAlmostEqual(combine_glossiness_signals(0.0, 1.0), 1.0)

    def test_ddna_and_palette_gloss_combine_before_one_remap(self) -> None:
        self.assertAlmostEqual(
            ddna_and_palette_gloss_to_roughness(0.25, 0.5),
            0.6123724356957945,
        )

    def test_derived_roughness_and_palette_gloss_do_not_remap_roughness_twice(self) -> None:
        self.assertAlmostEqual(
            roughness_and_palette_gloss_to_roughness(0.8660254037844386, 0.5),
            0.6123724356957945,
        )


if __name__ == "__main__":
    unittest.main()
