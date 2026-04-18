from __future__ import annotations

from pathlib import Path
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent

sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.manifest import LightRecord, MaterialSidecar, PackageBundle, TextureReference, infer_export_root


ARGO_SCENE = REPO_ROOT / "ships/Packages/ARGO MOLE/scene.json"
ARGO_INTERIOR = REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/ARGO/MOLE/argo_mole_interior.materials.json"
COMPONENT_MASTER = REPO_ROOT / "ships/Data/Materials/vehicles/components/component_master_01.materials.json"


class ManifestTests(unittest.TestCase):
    def test_export_root_inference_matches_fixture_layout(self) -> None:
        export_root = infer_export_root(ARGO_SCENE, "Packages/ARGO MOLE")
        self.assertEqual(export_root, REPO_ROOT / "ships")

    def test_package_bundle_loads_real_fixture_manifests(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        self.assertEqual(package.package_name, "ARGO MOLE")
        self.assertEqual(package.scene.root_entity.entity_name, "EntityClassDefinition.ARGO_MOLE")
        self.assertGreater(len(package.scene.children), 10)
        self.assertIn("palette/argo_mole", package.palettes)
        self.assertIn("palette/default", package.liveries)

    def test_package_bundle_resolves_and_caches_material_sidecars(self) -> None:
        package = PackageBundle.load(ARGO_SCENE)
        sidecar = package.load_material_sidecar("Data/objects/spaceships/ships/argo/mole/argo_mole_interior.materials.json")
        self.assertIsNotNone(sidecar)
        second = package.load_material_sidecar("Data/objects/spaceships/ships/argo/mole/argo_mole_interior.materials.json")
        self.assertIs(sidecar, second)

        cargo_pod = package.resolve_path("Data/Objects/Spaceships/Ships/MISC/Prospector/MISC_Prospector_Cargo_Pod_Collapsed.glb")
        self.assertIsNotNone(cargo_pod)
        self.assertTrue(cargo_pod.is_file())

    def test_material_sidecar_preserves_layer_and_virtual_input_contract(self) -> None:
        interior = MaterialSidecar.from_file(ARGO_INTERIOR)
        self.assertIsNotNone(interior.source_material_path)
        self.assertTrue(interior.submaterials)
        ui_plane = next(submaterial for submaterial in interior.submaterials if submaterial.shader_family == "UIPlane")
        self.assertEqual(ui_plane.submaterial_name, "rtt_hud")
        self.assertIn("$RenderToTexture", ui_plane.virtual_inputs)

        component = MaterialSidecar.from_file(COMPONENT_MASTER)
        layered = next(
            submaterial
            for submaterial in component.submaterials
            if submaterial.shader_family == "LayerBlend_V2"
            and any(layer.palette_channel is not None for layer in submaterial.layer_manifest)
        )
        self.assertTrue(layered.layer_manifest)
        palette_layer = next(layer for layer in layered.layer_manifest if layer.palette_channel is not None)
        self.assertEqual(palette_layer.palette_channel.name, "primary")

    def test_light_record_preserves_type_for_decomposed_runtime(self) -> None:
        light = LightRecord.from_value(
            {
                "name": "Light-1",
                "color": [1.0, 0.5, 0.25],
                "light_type": "Projector",
                "intensity": 123.0,
                "radius": 7.5,
                "position": [1.0, 2.0, 3.0],
                "rotation": [1.0, 0.0, 0.0, 0.0],
                "inner_angle": 18.0,
                "outer_angle": 24.0,
            }
        )
        self.assertEqual(light.light_type, "Projector")
        self.assertEqual(light.outer_angle, 24.0)

    def test_texture_reference_preserves_ddna_smoothness_markers(self) -> None:
        texture = TextureReference.from_value(
            {
                "role": "normal_gloss",
                "source_path": "Data/Objects/Ships/Test/hull_ddna.dds",
                "export_path": "Data/Objects/Ships/Test/hull_ddna.png",
                "export_kind": "source",
                "texture_identity": "ddna_normal",
                "alpha_semantic": "smoothness",
            }
        )

        self.assertEqual(texture.texture_identity, "ddna_normal")
        self.assertEqual(texture.alpha_semantic, "smoothness")

    def test_layer_manifest_preserves_texture_slots(self) -> None:
        component = MaterialSidecar.from_file(COMPONENT_MASTER)
        layered = next(
            submaterial
            for submaterial in component.submaterials
            if submaterial.shader_family == "LayerBlend_V2"
            and any(layer.texture_slots for layer in submaterial.layer_manifest)
        )
        layer = next(layer for layer in layered.layer_manifest if layer.texture_slots)
        smoothness_texture = next(texture for texture in layer.texture_slots if texture.alpha_semantic == "smoothness")
        self.assertEqual(smoothness_texture.texture_identity, "ddna_normal")

    def test_layer_manifest_preserves_resolved_layer_details(self) -> None:
        exterior = MaterialSidecar.from_file(REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/ARGO/MOLE/argo_mole_exterior.materials.json")
        layered = next(
            submaterial
            for submaterial in exterior.submaterials
            if submaterial.submaterial_name == "paint_primary_orange_low"
        )
        primary = layered.layer_manifest[0]

        self.assertEqual(primary.name, "Primary")
        self.assertAlmostEqual(primary.gloss_mult or 0.0, 0.7699999809265137)
        self.assertEqual(primary.palette_channel.name, "primary")
        self.assertEqual(primary.layer_snapshot["shader"], "Layer")
        self.assertTrue(primary.resolved_material["authored_public_params"])

    def test_texture_reference_preserves_structured_texture_transform(self) -> None:
        texture = TextureReference.from_value(
            {
                "role": "normal_gloss",
                "source_path": "Data/libs/materials/metal/test_layer_ddna.dds",
                "export_path": "Data/libs/materials/metal/test_layer.normal.png",
                "export_kind": "normal_from_ddna",
                "texture_transform": {
                    "attributes": {
                        "TileU": 2,
                        "TileV": 3,
                    },
                    "scale": [2.0, 3.0],
                },
            }
        )

        self.assertEqual(texture.texture_transform["scale"], [2.0, 3.0])
        self.assertEqual(texture.texture_transform["attributes"]["TileU"], 2)


if __name__ == "__main__":
    unittest.main()
