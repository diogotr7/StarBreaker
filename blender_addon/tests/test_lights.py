"""Regression tests for light import mapping (Phase 5).

Loads ``runtime/importer/utils.py`` as a standalone module with a stubbed
``bpy`` so the tests can run outside Blender. Avoids triggering
``starbreaker_addon/__init__.py`` (which needs a real bpy build).
"""

from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path
from types import SimpleNamespace

ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


def _load_utils() -> tuple[types.ModuleType, types.ModuleType]:
    if "bpy" not in sys.modules:
        sys.modules["bpy"] = types.ModuleType("bpy")
    if "mathutils" not in sys.modules:
        mathutils = types.ModuleType("mathutils")

        class _Stub:  # Minimal stand-ins; constants.py only stores instances.
            def __init__(self, *args, **kwargs):
                pass

            def inverted(self):
                return self

        mathutils.Matrix = _Stub
        mathutils.Quaternion = _Stub
        sys.modules["mathutils"] = mathutils

    def _load(name: str, path: Path) -> types.ModuleType:
        spec = importlib.util.spec_from_file_location(name, str(path))
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
        return module

    constants = _load(
        "sb_lights_test_constants",
        ADDON_ROOT / "runtime" / "constants.py",
    )
    parent_pkg = types.ModuleType("sb_lights_test_runtime")
    parent_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
    sys.modules["sb_lights_test_runtime"] = parent_pkg
    sys.modules["sb_lights_test_runtime.constants"] = constants

    importer_pkg = types.ModuleType("sb_lights_test_runtime.importer")
    importer_pkg.__path__ = [str(ADDON_ROOT / "runtime" / "importer")]
    sys.modules["sb_lights_test_runtime.importer"] = importer_pkg

    utils_src = (ADDON_ROOT / "runtime" / "importer" / "utils.py").read_text()
    # Rewrite relative imports to absolute paths under our shim package root.
    utils_src = utils_src.replace("from ...", "from sb_lights_test_addon.")
    utils_src = utils_src.replace("from ..constants import", "from sb_lights_test_runtime.constants import")
    utils_src = utils_src.replace("from ..", "from sb_lights_test_runtime.")
    # Stub modules referenced via the shim addon root (manifest, palette, etc.).
    addon_pkg = types.ModuleType("sb_lights_test_addon")
    addon_pkg.__path__ = [str(ADDON_ROOT)]
    sys.modules["sb_lights_test_addon"] = addon_pkg
    for sub in ("manifest", "palette", "templates", "material_contract"):
        stub = types.ModuleType(f"sb_lights_test_addon.{sub}")
        # Populate placeholder names used in utils.py import lists.
        for name in (
            "MaterialSidecar",
            "PackageBundle",
            "PaletteRecord",
            "SubmaterialRecord",
            "resolved_palette_id",
            "managed_material_runtime_graph_is_sane",
            "ShaderGroupContract",
            "ContractInput",
        ):
            setattr(stub, name, type(name, (), {}))
        sys.modules[f"sb_lights_test_addon.{sub}"] = stub
    # Stub ``..package_ops`` (referenced from utils.py).
    package_ops_stub = types.ModuleType("sb_lights_test_runtime.package_ops")
    package_ops_stub._scene_instance_from_object = lambda *a, **k: None
    package_ops_stub._string_prop = lambda *a, **k: None
    sys.modules["sb_lights_test_runtime.package_ops"] = package_ops_stub
    utils_module = types.ModuleType("sb_lights_test_runtime.importer.utils")
    utils_module.__file__ = str(ADDON_ROOT / "runtime" / "importer" / "utils.py")
    exec(compile(utils_src, utils_module.__file__, "exec"), utils_module.__dict__)
    return utils_module, constants


class LightMappingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.utils, cls.constants = _load_utils()

    def test_type_omni_is_point(self):
        light = SimpleNamespace(light_type="Omni", inner_angle=None, outer_angle=None)
        self.assertEqual(self.utils._blender_light_type(light), "POINT")

    def test_type_softomni_is_point(self):
        light = SimpleNamespace(light_type="SoftOmni", inner_angle=None, outer_angle=None)
        self.assertEqual(self.utils._blender_light_type(light), "POINT")

    def test_type_projector_is_spot(self):
        light = SimpleNamespace(light_type="Projector", inner_angle=30.0, outer_angle=45.0)
        self.assertEqual(self.utils._blender_light_type(light), "SPOT")

    def test_type_directional_is_sun(self):
        light = SimpleNamespace(light_type="Directional", inner_angle=None, outer_angle=None)
        self.assertEqual(self.utils._blender_light_type(light), "SUN")

    def test_type_falls_back_to_spot_when_angles_present(self):
        light = SimpleNamespace(light_type="", inner_angle=0.0, outer_angle=30.0)
        self.assertEqual(self.utils._blender_light_type(light), "SPOT")

    def test_energy_lumens_to_watts_point(self):
        self.assertAlmostEqual(
            self.utils._light_energy_to_blender(200.0, "POINT"),
            200.0 / self.constants.LUMENS_PER_WATT_WHITE,
            places=6,
        )

    def test_energy_lumens_to_watts_spot(self):
        self.assertAlmostEqual(
            self.utils._light_energy_to_blender(400.0, "SPOT"),
            400.0 / self.constants.LUMENS_PER_WATT_WHITE,
            places=6,
        )

    def test_energy_sun_uses_photopic_peak(self):
        self.assertAlmostEqual(
            self.utils._light_energy_to_blender(683.0, "SUN"),
            683.0 / self.constants.GLTF_PBR_WATTS_TO_LUMENS,
            places=6,
        )

    def test_negative_intensity_clamps_to_zero(self):
        self.assertEqual(self.utils._light_energy_to_blender(-42.0, "POINT"), 0.0)

    def test_efficacy_constant_sane(self):
        self.assertGreaterEqual(self.constants.LUMENS_PER_WATT_WHITE, 80.0)
        self.assertLessEqual(self.constants.LUMENS_PER_WATT_WHITE, 200.0)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
