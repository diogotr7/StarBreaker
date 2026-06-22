"""Unit tests for starbreaker_addon.runtime.importer.utils helpers.

Covers the nearest-index duplicate-name selection logic added in Phase 57
(``_remapped_submaterial_for_slot``) and the helper builders
``_submaterials_by_name`` / ``_unique_submaterials_by_name``.

These tests run without a live Blender process (``bpy`` is stubbed out).
"""

from __future__ import annotations

import sys
import types
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))

# ---------------------------------------------------------------------------
# Stub bpy and mathutils so that utils.py can be imported outside Blender.
# ---------------------------------------------------------------------------

if "mathutils" not in sys.modules:
    mathutils = types.ModuleType("mathutils")

    class _Matrix(tuple):
        def __new__(cls, rows):
            return tuple.__new__(cls, rows)

        def inverted(self):
            return self

    class _Quaternion(tuple):
        def __new__(cls, values):
            return tuple.__new__(cls, values)

    mathutils.Matrix = _Matrix
    mathutils.Quaternion = _Quaternion
    sys.modules["mathutils"] = mathutils

if "bpy" not in sys.modules:
    bpy = types.ModuleType("bpy")
    bpy.types = types.SimpleNamespace(
        Context=object,
        Material=object,
        NodeLinks=object,
        Nodes=object,
        Object=object,
        ShaderNodeTexImage=object,
    )
    bpy.data = types.SimpleNamespace(node_groups=[], images=[])
    sys.modules["bpy"] = bpy

if "numpy" not in sys.modules:
    numpy = types.ModuleType("numpy")
    numpy.float32 = float
    numpy.empty = lambda length, dtype=None: [0.0] * length
    sys.modules["numpy"] = numpy

# Ensure package hierarchy is registered.
for _pkg in ("starbreaker_addon", "starbreaker_addon.runtime", "starbreaker_addon.runtime.importer"):
    if _pkg not in sys.modules:
        _mod = types.ModuleType(_pkg)
        _parts = _pkg.replace("starbreaker_addon", str(ADDON_ROOT / "starbreaker_addon"))
        _mod.__path__ = [str(ADDON_ROOT / Path(*_pkg.split(".")))]
        sys.modules[_pkg] = _mod

from starbreaker_addon.manifest import MaterialSidecar, SubmaterialRecord
from starbreaker_addon.material_contract import ContractInput, ShaderGroupContract
from starbreaker_addon.runtime.importer.materials import MaterialsMixin
from starbreaker_addon.runtime.record_utils import _mesh_decal_authored_emission_strength
from starbreaker_addon.templates import screen_effects_apply_crt
from starbreaker_addon.runtime.importer.utils import (
    _canonical_source_name,
    _material_identity,
    _managed_material_runtime_graph_is_sane,
    _remapped_submaterial_for_slot,
    _slot_mapping_from_slot_names,
    _submaterials_by_name,
    _unique_submaterials_by_name,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_submaterial(index: int, name: str) -> SubmaterialRecord:
    """Build a minimal SubmaterialRecord stub for use in unit tests."""
    return SubmaterialRecord.from_value({"index": index, "submaterial_name": name})


def _make_sidecar(*entries: tuple[int, str]) -> MaterialSidecar:
    """Build a MaterialSidecar whose submaterials are the given (index, name) pairs."""
    submaterials = [_make_submaterial(idx, name) for idx, name in entries]
    return MaterialSidecar(
        geometry_path=None,
        normalized_export_relative_path=None,
        source_material_path=None,
        palette_contract={},
        submaterials=submaterials,
        raw={},
    )


class _FakeImageNode:
    def __init__(self, path: str | None):
        self.path = path
        self.outputs = _FakeOutputs(path)


class _FakeOutputs(dict):
    def __init__(self, path: str | None):
        super().__init__(
            {
                "Color": f"{path}:Color",
                "Alpha": f"{path}:Alpha",
                0: f"{path}:Color",
            }
        )


class _MaterialSocketProbe(MaterialsMixin):
    def __init__(self) -> None:
        self.image_paths: list[tuple[str | None, bool]] = []

    def _image_node(self, nodes, image_path, *, x, y, is_color, **_kwargs):
        self.image_paths.append((image_path, is_color))
        if image_path is None:
            return None
        return _FakeImageNode(image_path)


class _FakeGroupTree(dict):
    def __init__(self, name: str, signature: str, nodes: list[object] | None = None) -> None:
        super().__init__(starbreaker_runtime_built_signature=signature)
        self.name = name
        self.nodes = nodes or []


class _FakeGroupNode:
    bl_idname = "ShaderNodeGroup"

    def __init__(self, node_tree: _FakeGroupTree, inputs: tuple[str, ...] = ()) -> None:
        self.node_tree = node_tree
        self.inputs = {name: object() for name in inputs}


class _FakeNodeTree:
    def __init__(self, nodes: list[object]) -> None:
        self.nodes = nodes
        self.links = []


class _FakeManagedMaterial(dict):
    def __init__(self, template_key: str, node_tree: _FakeNodeTree | None) -> None:
        super().__init__(starbreaker_template_key=template_key)
        self.node_tree = node_tree


class _FakeMaterial(dict):
    def __init__(self, name: str) -> None:
        super().__init__()
        self.name = name


class _FakeMaterialSlots(list):
    pass


class _FakeMesh(dict):
    def __init__(self, names: list[str]) -> None:
        super().__init__()
        self.materials = _FakeMaterialSlots(_FakeMaterial(name) for name in names)

    def as_pointer(self) -> int:
        return 1


class _FakeMeshObject:
    type = "MESH"

    def __init__(self, names: list[str]) -> None:
        self.data = _FakeMesh(names)


# ---------------------------------------------------------------------------
# Tests for _submaterials_by_name
# ---------------------------------------------------------------------------

class TestCanonicalSourceName(unittest.TestCase):
    def test_strips_blender_numeric_suffix(self) -> None:
        self.assertEqual(_canonical_source_name("paint.001"), "paint")

    def test_native_blend_material_slot_name_maps_to_submaterial_name(self) -> None:
        self.assertEqual(
            _canonical_source_name("rsi_aurora_mk2_mtl_Decal_POM_07"),
            "Decal_POM",
        )

    def test_native_blend_material_slot_preserves_internal_numbers(self) -> None:
        self.assertEqual(
            _canonical_source_name("ship_mtl_screen_1x1_10"),
            "screen_1x1",
        )

    def test_decomposed_host_variant_maps_to_submaterial_name(self) -> None:
        self.assertEqual(
            _canonical_source_name(
                "esmg_fps_volt_quartz_collector_03_mat:pom_mat__host_primary"
            ),
            "pom_mat",
        )

    def test_decomposed_hashed_host_variant_maps_to_submaterial_name(self) -> None:
        self.assertEqual(
            _canonical_source_name(
                "drak_clipper_ext:Decal_POM_A#03e817cc__host_tertiary"
            ),
            "Decal_POM_A",
        )

    def test_decomposed_rgb_host_variant_strips_blender_numeric_suffix(self) -> None:
        self.assertEqual(
            _canonical_source_name("drak_vulture:pom_decals__host_rgb_1a2b3c.001"),
            "pom_decals",
        )


class TestManagedScreenMaterialGraph(unittest.TestCase):
    def test_screen_material_requires_current_runtime_group_signature(self) -> None:
        runtime_group = _FakeGroupTree("StarBreaker Runtime Screen", "screen_v1")
        material = _FakeManagedMaterial(
            "screen_hud",
            _FakeNodeTree([_FakeGroupNode(runtime_group, inputs=("Base Color",))]),
        )

        self.assertFalse(_managed_material_runtime_graph_is_sane(material))

    def test_screen_material_requires_material_level_pixelate_group(self) -> None:
        runtime_group = _FakeGroupTree("StarBreaker Runtime Screen", "screen_v4")
        material = _FakeManagedMaterial(
            "screen_hud",
            _FakeNodeTree([
                _FakeGroupNode(
                    runtime_group,
                    inputs=("Base Color", "Emission Strength", "Use CRT", "Vector", "X pixels", "Y pixels"),
                )
            ]),
        )

        self.assertFalse(_managed_material_runtime_graph_is_sane(material))

    def test_screen_material_accepts_current_crt_runtime_group(self) -> None:
        runtime_group = _FakeGroupTree(
            "StarBreaker Runtime Screen",
            "screen_v4",
            nodes=[
                _FakeGroupNode(_FakeGroupTree("RGB_grid", "")),
            ],
        )
        material = _FakeManagedMaterial(
            "screen_hud",
            _FakeNodeTree([
                _FakeGroupNode(
                    runtime_group,
                    inputs=("Base Color", "Emission Strength", "Use CRT", "Vector", "X pixels", "Y pixels"),
                ),
                _FakeGroupNode(_FakeGroupTree("Pixelate", "")),
            ]),
        )

        self.assertTrue(_managed_material_runtime_graph_is_sane(material))


# ---------------------------------------------------------------------------
# Tests for _submaterials_by_name
# ---------------------------------------------------------------------------

class TestSubmaterialsByName(unittest.TestCase):
    def test_groups_duplicate_names(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"), (2, "paint"))
        grouped = _submaterials_by_name(sidecar)
        self.assertIn("paint", grouped)
        self.assertEqual(len(grouped["paint"]), 2)
        self.assertEqual([r.index for r in grouped["paint"]], [0, 2])

    def test_single_entry_per_unique_name(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"))
        grouped = _submaterials_by_name(sidecar)
        self.assertEqual(len(grouped["paint"]), 1)
        self.assertEqual(len(grouped["glass"]), 1)

    def test_skips_blank_names(self) -> None:
        sidecar = _make_sidecar((0, ""), (1, "  "), (2, "metal"))
        grouped = _submaterials_by_name(sidecar)
        self.assertNotIn("", grouped)
        self.assertNotIn("  ", grouped)
        self.assertIn("metal", grouped)

    def test_native_blend_slot_names_map_to_sparse_sidecar_indices(self) -> None:
        sidecar = _make_sidecar(
            (1, "poms"),
            (2, "decals"),
            (3, "glow_mat"),
            (17, "stencil_decal"),
        )
        mapping = _slot_mapping_from_slot_names(
            [
                "bsnp_fps_behr_p6lr_mat_mtl_poms_00",
                "bsnp_fps_behr_p6lr_mat_mtl_decals_01",
                "bsnp_fps_behr_p6lr_mat_mtl_glow_mat_02",
                "bsnp_fps_behr_p6lr_mat_mtl_stencil_decal_014",
            ],
            _unique_submaterials_by_name(sidecar),
            _submaterials_by_name(sidecar),
        )
        self.assertEqual(mapping, [1, 2, 3, 17])

    def test_clear_template_material_bindings_preserves_native_blend_slot_names(self) -> None:
        obj = _FakeMeshObject(
            [
                "bsnp_fps_behr_p6lr_mat_mtl_poms_00",
                "bsnp_fps_behr_p6lr_mat_mtl_stencil_decal_014",
            ]
        )

        MaterialsMixin._clear_template_material_bindings(object(), [obj])

        self.assertEqual(
            obj.data.get("starbreaker_imported_slot_names"),
            '["bsnp_fps_behr_p6lr_mat_mtl_poms_00", "bsnp_fps_behr_p6lr_mat_mtl_stencil_decal_014"]',
        )
        self.assertEqual(list(obj.data.materials), [None, None])


# ---------------------------------------------------------------------------
# Tests for _unique_submaterials_by_name
# ---------------------------------------------------------------------------

class TestUniqueSubmaterialsByName(unittest.TestCase):
    def test_excludes_duplicate_names(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"), (2, "paint"))
        unique = _unique_submaterials_by_name(sidecar)
        self.assertNotIn("paint", unique)
        self.assertIn("glass", unique)

    def test_includes_unique_names(self) -> None:
        sidecar = _make_sidecar((0, "paint"), (1, "glass"))
        unique = _unique_submaterials_by_name(sidecar)
        self.assertIn("paint", unique)
        self.assertIn("glass", unique)

    def test_returns_first_record_for_unique(self) -> None:
        sidecar = _make_sidecar((3, "metal"))
        unique = _unique_submaterials_by_name(sidecar)
        self.assertEqual(unique["metal"].index, 3)


# ---------------------------------------------------------------------------
# Tests for _remapped_submaterial_for_slot  (Phase 57 fix)
# ---------------------------------------------------------------------------

class TestRemappedSubmaterialForSlot(unittest.TestCase):

    def _slot(
        self,
        source: SubmaterialRecord | None,
        fallback: int,
        by_index: dict[int, SubmaterialRecord],
        by_name: dict[str, SubmaterialRecord],
        by_name_all: dict[str, list[SubmaterialRecord]] | None = None,
    ) -> SubmaterialRecord | None:
        return _remapped_submaterial_for_slot(source, fallback, by_index, by_name, by_name_all)

    def test_unique_name_match_takes_priority_over_index(self) -> None:
        source = _make_submaterial(0, "glass")
        target_0 = _make_submaterial(0, "metal")
        target_1 = _make_submaterial(1, "glass")
        by_index = {0: target_0, 1: target_1}
        by_name = {"glass": target_1}
        result = self._slot(source, 0, by_index, by_name)
        self.assertIs(result, target_1)

    def test_falls_back_to_index_when_name_absent(self) -> None:
        source = _make_submaterial(0, "nonexistent")
        target_5 = _make_submaterial(5, "something_else")
        by_index = {5: target_5}
        by_name: dict[str, SubmaterialRecord] = {}
        result = self._slot(source, 5, by_index, by_name)
        self.assertIs(result, target_5)

    def test_none_source_falls_back_to_index(self) -> None:
        target_2 = _make_submaterial(2, "paint")
        by_index = {2: target_2}
        result = self._slot(None, 2, by_index, {})
        self.assertIs(result, target_2)

    def test_duplicate_names_picks_nearest_by_index(self) -> None:
        """Phase 57 fix: when a name maps to multiple submaterials, pick the one
        whose index is closest to fallback_index."""
        dup_near = _make_submaterial(3, "paint")
        dup_far = _make_submaterial(9, "paint")
        source = _make_submaterial(4, "paint")  # source index doesn't matter; fallback=4
        by_index: dict[int, SubmaterialRecord] = {}
        by_name: dict[str, SubmaterialRecord] = {}   # name absent from unique map (duplicate)
        by_name_all = {"paint": [dup_near, dup_far]}
        result = self._slot(source, 4, by_index, by_name, by_name_all)
        # |3-4| = 1 < |9-4| = 5 → should pick dup_near
        self.assertIs(result, dup_near)

    def test_duplicate_names_picks_nearest_when_far_is_closer(self) -> None:
        dup_near = _make_submaterial(1, "paint")
        dup_far = _make_submaterial(10, "paint")
        source = _make_submaterial(0, "paint")
        by_name_all = {"paint": [dup_near, dup_far]}
        result = self._slot(source, 9, {}, {}, by_name_all)
        # |1-9| = 8 vs |10-9| = 1 → should pick dup_far
        self.assertIs(result, dup_far)

    def test_by_name_all_not_queried_when_unique_match_exists(self) -> None:
        """If by_name has a unique match, by_name_all should be irrelevant."""
        unique = _make_submaterial(2, "paint")
        dup_a = _make_submaterial(0, "paint")
        dup_b = _make_submaterial(5, "paint")
        source = _make_submaterial(0, "paint")
        by_name = {"paint": unique}
        by_name_all = {"paint": [dup_a, dup_b]}
        result = self._slot(source, 0, {}, by_name, by_name_all)
        self.assertIs(result, unique)

    def test_blank_source_name_falls_back_to_index(self) -> None:
        source = _make_submaterial(0, "   ")  # blank name after strip
        target_7 = _make_submaterial(7, "anything")
        by_index = {7: target_7}
        result = self._slot(source, 7, by_index, {})
        self.assertIs(result, target_7)

    def test_returns_none_when_index_also_missing(self) -> None:
        source = _make_submaterial(0, "ghost")
        result = self._slot(source, 99, {}, {})
        self.assertIsNone(result)


# ---------------------------------------------------------------------------
# Tests for MeshDecal emission gating
# ---------------------------------------------------------------------------

class TestMeshDecalEmissionStrength(unittest.TestCase):
    def test_linked_mesh_decal_requires_explicit_glow_signal(self) -> None:
        linked = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "authored_attributes": [
                    {"name": "Emissive", "value": "1,1,1"},
                ],
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "source_path": "Data/Textures/vehicles/manufacturer/CRUS/Glows/crus_glows_diff.tif",
                        "export_path": "Data/Textures/vehicles/manufacturer/CRUS/Glows/crus_glows_diff_TEX0.png",
                    }
                ],
            }
        )
        self.assertEqual(_mesh_decal_authored_emission_strength(linked), 0.0)

    def test_unlinked_mesh_decal_keeps_authored_glow(self) -> None:
        unlinked = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "authored_attributes": [
                    {"name": "Emissive", "value": "1,1,1"},
                    {"name": "Glow", "value": "0.25"},
                ],
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "base_color",
                        "source_path": "Data/Textures/vehicles/manufacturer/CRUS/Glows/crus_glows_diff.tif",
                        "export_path": "Data/Textures/vehicles/manufacturer/CRUS/Glows/crus_glows_diff_TEX0.png",
                    }
                ],
            }
        )
        self.assertAlmostEqual(_mesh_decal_authored_emission_strength(unlinked), 0.25)


    def test_mesh_decal_emissive_texture_still_emits(self) -> None:
        decal = SubmaterialRecord.from_value({"shader_family": "MeshDecal"})
        self.assertEqual(
            _mesh_decal_authored_emission_strength(
                decal,
                emissive_texture_path="Data/Textures/test_emissive.png",
            ),
            1.0,
        )


class TestScreenEffects(unittest.TestCase):
    def test_screen_effects_apply_crt_reads_sidecar_flag(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "UIPlane",
                "screen_effects": {"apply_crt": True},
            }
        )

        self.assertTrue(screen_effects_apply_crt(submaterial))

    def test_screen_effects_apply_crt_defaults_false(self) -> None:
        submaterial = SubmaterialRecord.from_value({"shader_family": "UIPlane"})

        self.assertFalse(screen_effects_apply_crt(submaterial))


class TestMaterialAlphaSources(unittest.TestCase):
    def test_stencil_decal_alpha_prefers_texslot7_stencil_texture(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_stencil_map": True, "tokens": ["STENCIL_MAP"]},
                "texture_slots": [
                    {
                        "slot": "TexSlot1",
                        "role": "opacity",
                        "export_path": "Data/Textures/test/transparent_opacity.png",
                    },
                    {
                        "slot": "TexSlot7",
                        "role": "stencil",
                        "export_path": "Data/Textures/test/stencil.png",
                    },
                ],
            }
        )
        probe = _MaterialSocketProbe()

        alpha = probe._alpha_source_socket(
            object(),
            submaterial,
            {"opacity": "Data/Textures/test/transparent_opacity.png", "base_color": None},
            x=0,
            y=0,
        )

        self.assertEqual(alpha, "Data/Textures/test/stencil.png:Alpha")
        self.assertEqual(probe.image_paths, [("Data/Textures/test/stencil.png", True)])

    def test_non_stencil_decal_alpha_uses_generic_opacity(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_decal": True, "tokens": ["DECAL"]},
                "texture_slots": [
                    {
                        "slot": "TexSlot7",
                        "role": "stencil",
                        "export_path": "Data/Textures/test/stencil.png",
                    },
                ],
            }
        )
        probe = _MaterialSocketProbe()

        alpha = probe._alpha_source_socket(
            object(),
            submaterial,
            {"opacity": "Data/Textures/test/opacity.png", "base_color": None},
            x=0,
            y=0,
        )

        self.assertEqual(alpha, "Data/Textures/test/opacity.png:Color")
        self.assertEqual(probe.image_paths, [("Data/Textures/test/opacity.png", False)])

    def test_stencil_decal_contract_does_not_fallback_without_adaptor(self) -> None:
        submaterial = SubmaterialRecord.from_value(
            {
                "shader_family": "MeshDecal",
                "decoded_feature_flags": {"has_stencil_map": True, "tokens": ["STENCIL_MAP"]},
                "texture_slots": [
                    {
                        "slot": "TexSlot7",
                        "role": "stencil",
                        "export_path": "Data/Textures/test/stencil.png",
                    },
                ],
            }
        )
        probe = _MaterialSocketProbe()
        group_contract = ShaderGroupContract(
            name="SB_MeshDecal_v1",
            shader_families=["MeshDecal"],
            version=1,
            shader_output="Shader",
            inputs=[],
            metadata={},
            raw={},
        )

        color = probe._contract_input_source_socket(
            object(),
            submaterial,
            None,
            group_contract,
            ContractInput(
                name="TexSlot1_DecalSource",
                socket_type="NodeSocketColor",
                semantic="decal_source",
                source_slot="TexSlot1",
                required=False,
                default_value=None,
                raw={},
            ),
            x=0,
            y=0,
        )
        alpha = probe._contract_input_source_socket(
            object(),
            submaterial,
            None,
            group_contract,
            ContractInput(
                name="TexSlot1_DecalSource_alpha",
                socket_type="NodeSocketFloat",
                semantic="decal_source_alpha",
                source_slot="TexSlot1",
                required=False,
                default_value=0.0,
                raw={},
            ),
            x=0,
            y=0,
        )

        self.assertIsNone(color)
        self.assertIsNone(alpha)


# ---------------------------------------------------------------------------
# Tests for _material_identity  (Phase 57 cache-key dedup)
# ---------------------------------------------------------------------------

class TestMaterialIdentityCacheKey(unittest.TestCase):
    """Verify that _material_identity uses the canonical sidecar path, not the
    raw sidecar_path argument, so two callers with different path strings but
    the same underlying sidecar produce the same cache key (Phase 57 fix)."""

    def _make_minimal_sidecar(self, normalized_path: str | None) -> MaterialSidecar:
        return MaterialSidecar(
            geometry_path=None,
            normalized_export_relative_path=normalized_path,
            source_material_path="Data/hull.mtl",
            palette_contract={},
            submaterials=[],
            raw={},
        )

    def _make_submaterial_record(self) -> SubmaterialRecord:
        return SubmaterialRecord.from_value({
            "index": 0,
            "submaterial_name": "hull_panel",
        })

    def test_different_sidecar_paths_with_same_normalized_path_produce_same_key(self) -> None:
        """Two different sidecar_path values (e.g. hash-variant vs canonical)
        with the same normalized_export_relative_path must produce the same
        identity hash — the Phase 57 dedup fix."""
        sidecar = self._make_minimal_sidecar("Data/Objects/Ships/hull_TEX0.materials.json")
        submaterial = self._make_submaterial_record()
        key_a = _material_identity(
            "Data/Objects/Ships/hull_TEX0.materials.json",
            sidecar,
            submaterial,
            None,
            "none",
        )
        key_b = _material_identity(
            "Data/Objects/Ships/hull-7735c1b7.materials.json",  # hash-variant path
            sidecar,
            submaterial,
            None,
            "none",
        )
        self.assertEqual(key_a, key_b, "different sidecar_path strings must produce the same key when normalized_export_relative_path is set")

    def test_same_path_produces_same_key_deterministically(self) -> None:
        sidecar = self._make_minimal_sidecar("Data/Objects/Ships/hull_TEX0.materials.json")
        submaterial = self._make_submaterial_record()
        key1 = _material_identity("Data/Objects/Ships/hull_TEX0.materials.json", sidecar, submaterial, None, "none")
        key2 = _material_identity("Data/Objects/Ships/hull_TEX0.materials.json", sidecar, submaterial, None, "none")
        self.assertEqual(key1, key2)

    def test_different_submaterials_produce_different_keys(self) -> None:
        sidecar = self._make_minimal_sidecar("Data/Objects/Ships/hull_TEX0.materials.json")
        sub_a = SubmaterialRecord.from_value({"index": 0, "submaterial_name": "hull_panel"})
        sub_b = SubmaterialRecord.from_value({"index": 1, "submaterial_name": "glass_pane"})
        key_a = _material_identity("Data/hull.materials.json", sidecar, sub_a, None, "none")
        key_b = _material_identity("Data/hull.materials.json", sidecar, sub_b, None, "none")
        self.assertNotEqual(key_a, key_b)

    def test_different_palette_scopes_produce_different_keys(self) -> None:
        sidecar = self._make_minimal_sidecar("Data/Objects/Ships/hull_TEX0.materials.json")
        submaterial = self._make_submaterial_record()
        key_none = _material_identity("Data/hull.materials.json", sidecar, submaterial, None, "none")
        key_primary = _material_identity("Data/hull.materials.json", sidecar, submaterial, None, "primary")
        self.assertNotEqual(key_none, key_primary)

    def test_fallback_to_sidecar_path_when_no_normalized_path(self) -> None:
        """When normalized_export_relative_path is None, sidecar_path itself
        is used as the canonical key, so identical sidecar_path values still
        deduplicate correctly."""
        sidecar = self._make_minimal_sidecar(None)
        submaterial = self._make_submaterial_record()
        key1 = _material_identity("Data/hull.materials.json", sidecar, submaterial, None, "none")
        key2 = _material_identity("Data/hull.materials.json", sidecar, submaterial, None, "none")
        self.assertEqual(key1, key2)


if __name__ == "__main__":
    unittest.main()
