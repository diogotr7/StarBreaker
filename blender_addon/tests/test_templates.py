from __future__ import annotations

from pathlib import Path
import sys
import unittest


ADDON_ROOT = Path(__file__).resolve().parents[1]
STARBREAKER_ROOT = ADDON_ROOT.parent
REPO_ROOT = STARBREAKER_ROOT.parent

sys.path.insert(0, str(ADDON_ROOT))

from starbreaker_addon.manifest import FeatureFlags, MaterialSidecar, PaletteRouting, SubmaterialRecord
from starbreaker_addon.templates import (
    active_submaterials,
    has_virtual_input,
    material_palette_channels,
    representative_textures,
    template_plan_for_submaterial,
)


ARGO_EXTERIOR = REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/ARGO/MOLE/argo_mole_exterior.materials.json"
ARGO_INTERIOR = REPO_ROOT / "ships/Data/Objects/Spaceships/Ships/ARGO/MOLE/argo_mole_interior.materials.json"
COMPONENT_MASTER = REPO_ROOT / "ships/Data/Materials/vehicles/components/component_master_01.materials.json"


def synthetic_submaterial(shader_family: str, *, tokens: list[str] | None = None, active: bool = True) -> SubmaterialRecord:
    return SubmaterialRecord(
        index=0,
        submaterial_name=f"synthetic_{shader_family.lower()}",
        blender_material_name=None,
        shader=shader_family,
        shader_family=shader_family,
        activation_state="active" if active else "inactive",
        activation_reason="visible" if active else "nodraw",
        decoded_feature_flags=FeatureFlags(
            tokens=tokens or [],
            has_decal="DECAL" in (tokens or []),
            has_iridescence=False,
            has_parallax_occlusion_mapping="PARALLAX_OCCLUSION_MAPPING" in (tokens or []),
            has_stencil_map="STENCIL_MAP" in (tokens or []),
            has_vertex_colors=False,
        ),
        direct_textures=[],
        derived_textures=[],
        texture_slots=[],
        layer_manifest=[],
        palette_routing=PaletteRouting(material_channel=None, layer_channels=[]),
        public_params={},
        variant_membership={},
        virtual_inputs=[],
        raw={},
    )


class TemplateTests(unittest.TestCase):
    def test_fixture_submaterials_map_to_expected_template_families(self) -> None:
        exterior = MaterialSidecar.from_file(ARGO_EXTERIOR)
        pom = next(submaterial for submaterial in exterior.submaterials if submaterial.submaterial_name == "pom_decals")
        self.assertEqual(template_plan_for_submaterial(pom).template_key, "decal_stencil")

        interior = MaterialSidecar.from_file(ARGO_INTERIOR)
        screen = next(submaterial for submaterial in interior.submaterials if submaterial.shader_family == "DisplayScreen")
        self.assertEqual(template_plan_for_submaterial(screen).template_key, "screen_hud")
        self.assertTrue(has_virtual_input(screen, "$RenderToTexture"))

        component = MaterialSidecar.from_file(COMPONENT_MASTER)
        layered = next(
            submaterial
            for submaterial in component.submaterials
            if submaterial.shader_family == "LayerBlend_V2"
            and any(layer.palette_channel is not None for layer in submaterial.layer_manifest)
        )
        self.assertEqual(template_plan_for_submaterial(layered).template_key, "layered_wear")
        self.assertTrue(material_palette_channels(layered))

    def test_synthetic_support_covers_biology_hair_and_effect_templates(self) -> None:
        self.assertEqual(template_plan_for_submaterial(synthetic_submaterial("HumanSkin_V2")).template_key, "biological")
        self.assertEqual(template_plan_for_submaterial(synthetic_submaterial("HairPBR")).template_key, "hair")
        self.assertEqual(template_plan_for_submaterial(synthetic_submaterial("Hologram")).template_key, "effects")

    def test_representative_textures_pick_exportable_maps(self) -> None:
        component = MaterialSidecar.from_file(COMPONENT_MASTER)
        hard_surface = next(submaterial for submaterial in component.submaterials if submaterial.shader_family == "HardSurface")
        textures = representative_textures(hard_surface)
        self.assertIsNotNone(textures["base_color"])
        self.assertIsNotNone(textures["normal"])

    def test_active_submaterials_filter_hidden_entries(self) -> None:
        active = synthetic_submaterial("HardSurface")
        inactive = synthetic_submaterial("NoDraw", active=False)
        self.assertEqual(active_submaterials([active, inactive]), [active])


if __name__ == "__main__":
    unittest.main()