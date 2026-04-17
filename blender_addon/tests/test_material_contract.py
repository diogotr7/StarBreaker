from __future__ import annotations

from pathlib import Path
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.material_contract import (
    TemplateContract,
    bundled_template_contract_path,
    bundled_template_library_path,
    load_bundled_template_contract,
)


class MaterialContractTests(unittest.TestCase):
    def test_bundled_resource_paths_point_into_resources_directory(self) -> None:
        self.assertEqual(bundled_template_contract_path().parent.name, "resources")
        self.assertEqual(bundled_template_library_path().parent.name, "resources")
        self.assertEqual(bundled_template_contract_path().name, "material_template_contract.json")
        self.assertEqual(bundled_template_library_path().name, "material_templates.blend")

    def test_bundled_contract_loads(self) -> None:
        contract = load_bundled_template_contract()
        self.assertEqual(contract.schema_version, 1)
        self.assertEqual(contract.metadata.get("status"), "blend_export")
        self.assertGreaterEqual(len(contract.groups), 1)
        self.assertTrue(str(contract.generated_from).endswith("material_templates.blend"))
        hard_surface = contract.group_for_shader_family("HardSurface")
        monitor = contract.group_for_shader_family("Monitor")
        nodraw = contract.group_for_shader_family("NoDraw")
        self.assertIsNotNone(hard_surface)
        self.assertIsNotNone(monitor)
        self.assertIsNotNone(nodraw)
        self.assertEqual(hard_surface.name, "SB_HardSurface_v1")
        self.assertTrue(hard_surface.inputs)
        self.assertEqual(hard_surface.inputs[0].name, "TexSlot1_BaseColor")
        self.assertEqual(hard_surface.metadata.get("status"), "seed")

    def test_bundled_library_contains_core_groups_and_verified_inputs(self) -> None:
        payload = bundled_template_library_path().read_bytes()

        self.assertIn(b"SB_NoDraw_v1", payload)
        self.assertIn(b"TexSlot10_IridescenceColor", payload)
        self.assertIn(b"TexSlot15_CondensationNormal", payload)

    def test_group_lookup_matches_shader_family(self) -> None:
        contract = TemplateContract.from_value(
            {
                "schema_version": 1,
                "groups": [
                    {
                        "name": "SB_HardSurface_v1",
                        "shader_families": ["HardSurface"],
                        "version": 1,
                        "shader_output": "Shader",
                        "inputs": [
                            {
                                "name": "TexSlot1_BaseColor",
                                "socket_type": "NodeSocketColor",
                                "semantic": "base_color",
                                "source_slot": "TexSlot1",
                                "required": True,
                            }
                        ],
                    }
                ],
            }
        )
        group = contract.group_for_shader_family("HardSurface")
        self.assertIsNotNone(group)
        self.assertEqual(group.name, "SB_HardSurface_v1")
        self.assertEqual(group.inputs[0].source_slot, "TexSlot1")

    def test_contract_serializes_back_to_plain_json(self) -> None:
        contract = TemplateContract.from_value(
            {
                "schema_version": 1,
                "generated_from": "seed",
                "groups": [
                    {
                        "name": "SB_UIPlane_v1",
                        "shader_families": ["UIPlane"],
                        "version": 1,
                        "shader_output": "Shader",
                        "inputs": [],
                        "metadata": {"status": "seed"},
                    }
                ],
                "metadata": {"status": "seed"},
            }
        )
        payload = contract.to_dict()
        self.assertEqual(payload["generated_from"], "seed")
        self.assertEqual(payload["groups"][0]["name"], "SB_UIPlane_v1")
        self.assertEqual(payload["groups"][0]["metadata"]["status"], "seed")


if __name__ == "__main__":
    unittest.main()

