from __future__ import annotations

from pathlib import Path
import sys
import types
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]

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

    mathutils.Matrix = Matrix
    mathutils.Quaternion = Quaternion
    sys.modules["mathutils"] = mathutils


from starbreaker_addon.runtime.constants import POM_DETAIL_DEFAULT, POM_DETAIL_ITEMS, pom_detail_settings


class PomDetailTests(unittest.TestCase):
    def test_medium_is_the_default_profile(self) -> None:
        self.assertEqual(POM_DETAIL_DEFAULT, "MEDIUM")
        self.assertEqual([item[0] for item in POM_DETAIL_ITEMS], ["LOW", "MEDIUM", "HIGH"])

    def test_profile_settings_map_to_expected_layers_and_scale_multiplier(self) -> None:
        # multiplier = layers**2 / 40**2 to compensate for delta =
        # parallax_dir * Scale / Layers**2 inside each POM root.
        self.assertEqual(pom_detail_settings("LOW"), (20, 0.25))
        self.assertEqual(pom_detail_settings("MEDIUM"), (50, 1.5625))
        self.assertEqual(pom_detail_settings("HIGH"), (100, 6.25))

    def test_unknown_mode_falls_back_to_medium(self) -> None:
        self.assertEqual(pom_detail_settings("unexpected"), (50, 1.5625))


if __name__ == "__main__":
    unittest.main()