"""Authored light dimensions/shadows, including old manifests and explicit zeroes."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

PATH = Path(__file__).resolve().parents[1] / "starbreaker_addon/runtime/importer/entity_lights.py"
SPEC = importlib.util.spec_from_file_location("light_fields_under_test", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class AuthoredLightFieldsTests(unittest.TestCase):
    def test_area_containment_requires_shadows(self):
        for shadow in (False, True, None):
            data = SimpleNamespace(type="POINT", use_shadow=False)
            MODULE.apply_authored_light_fields(data, SimpleNamespace(raw={
                "affects_this_area_only": True, "shadow_casting": shadow}))
            self.assertTrue(data.use_shadow)

    def test_point_authored_zero_radius_and_disabled_shadow(self):
        data = SimpleNamespace(type="POINT", use_shadow=True, shadow_soft_size=0.4)
        MODULE.apply_authored_light_fields(data, SimpleNamespace(raw={
            "bulb_radius_m": 0.0, "shadow_casting": False}))
        self.assertEqual(data.shadow_soft_size, 0.0)
        self.assertFalse(data.use_shadow)

    def test_area_dimensions_do_not_use_attenuation_radius(self):
        data = SimpleNamespace(type="AREA", shape="DISK", size=99, size_y=99, use_shadow=False)
        MODULE.apply_authored_light_fields(data, SimpleNamespace(raw={
            "plane_width_m": 2.25, "plane_height_m": 3.75, "shadow_casting": True}))
        self.assertEqual((data.shape, data.size, data.size_y), ("RECTANGLE", 2.25, 3.75))
        self.assertTrue(data.use_shadow)

    def test_absent_and_null_fields_preserve_legacy_mapping(self):
        for raw in ({}, {"plane_width_m": None, "plane_height_m": None, "bulb_radius_m": None}):
            data = SimpleNamespace(type="AREA", shape="RECTANGLE", size=7, size_y=8,
                                   shadow_soft_size=0.4, use_shadow=True)
            before = vars(data).copy()
            MODULE.apply_authored_light_fields(data, SimpleNamespace(raw=raw))
            self.assertEqual(vars(data), before)
