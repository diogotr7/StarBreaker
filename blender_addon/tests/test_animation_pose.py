"""Regression tests for the snap_first / snap_last pose-application math
used by `runtime.package_ops._apply_best_channel_transform`.

Pins:

* `_decode_animation_position("identity", …)` is a pass-through; the
  exporter writes Blender-frame XYZ already (per
  `crates/starbreaker-3d/src/animation.rs::clip_to_json`, which emits
  `(cry_y, -cry_z, cry_x)`).
* The endpoint-policy selector picks the first sample at frame_index=0
  and the last sample otherwise (literal mode).
* The position-track delta is computed against the keyframe whose
  decoded position is closest to the bone's bind location, then added
  to the bind location. A bone whose entire position track is constant
  must therefore stay at bind, regardless of the absolute offset
  between the clip's authored values and the bone's bind position.

This last property is the key contract the `wings_deploy` X-shape
investigation hinges on: rotators `Wing_Rotator_Top_Left` and
`Wing_Rotator_Bottom_Right` have constant position tracks in the
sidecar and therefore MUST land at bind position. If a future change
breaks that property, the X-shape failure mode reappears regardless
of any parent-frame composition fix.

See `docs/StarBreaker/animation-research.md` (Scorpius wing-deploy
kinematics → Deployed-pose verification in Blender) for the data the
constants below are pinned against.
"""

from __future__ import annotations

import json
import math
import sys
import types
import unittest
from pathlib import Path


ADDON_ROOT = Path(__file__).resolve().parent.parent / "starbreaker_addon"


class _StubObject:
    """Minimal stand-in for `bpy.types.Object` for pose-math tests."""

    def __init__(
        self,
        name: str,
        location=(0.0, 0.0, 0.0),
        rotation_quaternion=(1.0, 0.0, 0.0, 0.0),
    ) -> None:
        self.name = name
        self.location = tuple(float(v) for v in location)
        self.rotation_mode = "QUATERNION"
        self.rotation_quaternion = tuple(float(v) for v in rotation_quaternion)
        self._props: dict[str, object] = {}

    # dict-like custom-properties access mirroring the bpy API surface
    # the production code uses.
    def __getitem__(self, key: str) -> object:
        return self._props[key]

    def __setitem__(self, key: str, value: object) -> None:
        self._props[key] = value

    def __contains__(self, key: str) -> bool:
        return key in self._props

    def get(self, key: str, default: object = None) -> object:
        return self._props.get(key, default)


def _load_package_ops() -> types.ModuleType:
    bpy = sys.modules.get("bpy")
    if bpy is None:
        bpy = types.ModuleType("bpy")
        sys.modules["bpy"] = bpy
    bpy.types = types.SimpleNamespace(Context=object, Object=object, ID=object, Light=object)
    bpy.data = types.SimpleNamespace(objects=[], lights=[])

    mathutils = sys.modules.get("mathutils")
    if mathutils is None:
        mathutils = types.ModuleType("mathutils")
        sys.modules["mathutils"] = mathutils

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

    runtime_pkg = sys.modules.get("sb_anim_test_runtime")
    if runtime_pkg is None:
        runtime_pkg = types.ModuleType("sb_anim_test_runtime")
        runtime_pkg.__path__ = [str(ADDON_ROOT / "runtime")]
        sys.modules["sb_anim_test_runtime"] = runtime_pkg

    addon_pkg = sys.modules.get("sb_anim_test_addon")
    if addon_pkg is None:
        addon_pkg = types.ModuleType("sb_anim_test_addon")
        addon_pkg.__path__ = [str(ADDON_ROOT)]
        sys.modules["sb_anim_test_addon"] = addon_pkg

    manifest_stub = types.ModuleType("sb_anim_test_addon.manifest")
    manifest_stub.PackageBundle = type("PackageBundle", (), {"load": staticmethod(lambda p: None)})
    manifest_stub.SceneInstanceRecord = type("SceneInstanceRecord", (), {})
    sys.modules["sb_anim_test_addon.manifest"] = manifest_stub

    palette_stub = types.ModuleType("sb_anim_test_addon.palette")
    palette_stub.palette_id_for_livery_instance = lambda *a, **kw: None
    palette_stub.resolved_palette_id = lambda package, requested, inherited: requested or inherited
    sys.modules["sb_anim_test_addon.palette"] = palette_stub

    validators_stub = types.ModuleType("sb_anim_test_runtime.validators")
    validators_stub._purge_orphaned_file_backed_images = lambda: 0
    validators_stub._purge_orphaned_runtime_groups = lambda: 0
    sys.modules["sb_anim_test_runtime.validators"] = validators_stub

    importer_stub = types.ModuleType("sb_anim_test_runtime.importer")
    importer_stub.PackageImporter = type("PackageImporter", (), {})
    sys.modules["sb_anim_test_runtime.importer"] = importer_stub

    constants_path = ADDON_ROOT / "runtime" / "constants.py"
    constants = types.ModuleType("sb_anim_test_runtime.constants")
    constants.__file__ = str(constants_path)
    spec = __import__("importlib.util").util.spec_from_file_location(
        "sb_anim_test_runtime.constants", str(constants_path)
    )
    assert spec is not None and spec.loader is not None
    spec.loader.exec_module(constants)
    sys.modules["sb_anim_test_runtime.constants"] = constants

    source = (ADDON_ROOT / "runtime" / "package_ops.py").read_text()
    source = source.replace("from ..manifest import", "from sb_anim_test_addon.manifest import")
    source = source.replace("from ..palette import", "from sb_anim_test_addon.palette import")
    source = source.replace("from .constants import", "from sb_anim_test_runtime.constants import")
    source = source.replace("from .validators import", "from sb_anim_test_runtime.validators import")
    source = source.replace(
        "from .importer import PackageImporter",
        "from sb_anim_test_runtime.importer import PackageImporter",
    )
    module = types.ModuleType("sb_anim_test_runtime.package_ops")
    module.__file__ = str(ADDON_ROOT / "runtime" / "package_ops.py")
    module.__package__ = "sb_anim_test_runtime"
    sys.modules[module.__name__] = module
    exec(compile(source, module.__file__, "exec"), module.__dict__)
    return module


# Pinned values from the live sidecar
# `ships/Packages/RSI Scorpius_LOD0_TEX0/scene.json`,
# clip "wings_deploy" (verified 2026-04-27).
# Position values are already in Blender XYZ (cry_y, -cry_z, cry_x).
_TOP_LEFT_POS_FIRST = [0.023793935775756836, 0.8021461367607117, -1.3102056980133057]
_TOP_LEFT_POS_LAST = [0.023793935775756836, 0.8021461367607117, -1.3102056980133057]
_TOP_RIGHT_POS_FIRST = [-0.5459997653961182, 0.8021460771560669, 1.3102059364318848]
_TOP_RIGHT_POS_LAST = [0.023352086544036865, 0.8021460771560669, 1.6394926309585571]

# Bind positions in Blender local frame (NMC bone_to_world translation
# axis-swapped via (cry_y, -cry_z, cry_x); see
# docs/StarBreaker/animation-research.md → bind pose tables).
_TOP_LEFT_BIND = (-0.546, 0.802, -1.310)
_TOP_RIGHT_BIND = (-0.546, 0.802, 1.310)


class AnimationPoseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.package_ops = _load_package_ops()

    # ---- pure decoder ------------------------------------------------

    def test_decode_identity_is_passthrough(self) -> None:
        decoded = self.package_ops._decode_animation_position([1.5, -2.25, 3.0], "identity")
        self.assertEqual(decoded, (1.5, -2.25, 3.0))

    def test_decode_legacy_swaps_y_and_z(self) -> None:
        # Legacy decoder retained for older sidecars: (x, z, -y) → (x, -z, y)
        decoded = self.package_ops._decode_animation_position([1.0, 2.0, 3.0], "legacy")
        self.assertEqual(decoded, (1.0, -3.0, 2.0))

    # ---- _apply_best_channel_transform endpoint policy ---------------

    def _make_object(self, name: str, bind_loc: tuple[float, float, float]) -> _StubObject:
        return _StubObject(name=name, location=bind_loc)

    def _bind_data(self, bind_loc: tuple[float, float, float]) -> dict[str, object]:
        return {
            "location": list(bind_loc),
            "rotation_mode": "QUATERNION",
            "rotation_quaternion": [1.0, 0.0, 0.0, 0.0],
            "parent_distance": None,
        }

    def test_constant_position_track_keeps_bone_at_bind(self) -> None:
        """Wing_Rotator_Top_Left has a constant position track in the
        wings_deploy sidecar. The endpoint must land at bind position
        in Blender-local frame (delta-from-closed-frame logic). This
        property is independent of any future parent-frame composition
        fix; if it ever regresses, the X-shape collapses regardless of
        what the parent yaw does.
        """
        obj = self._make_object("Wing_Rotator_Top_Left", _TOP_LEFT_BIND)
        channel = {
            "rotation": [[0.999, 0.0, 0.044, 0.0], [0.866, 0.0, 0.500, 0.0]],
            "position": [_TOP_LEFT_POS_FIRST, _TOP_LEFT_POS_LAST],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(_TOP_LEFT_BIND),
            channel,
            frame_index=1,
            endpoint_policy="literal",
        )

        for axis, (got, want) in enumerate(zip(obj.location, _TOP_LEFT_BIND)):
            self.assertAlmostEqual(
                got, want, places=5,
                msg=f"axis {axis}: constant track must keep bone at bind",
            )

    def test_authored_position_track_lands_at_bind_plus_clip_delta(self) -> None:
        """Wing_Rotator_Top_Right's clip moves the bone +0.57 along
        Blender-X and +0.33 along Blender-Z between frame 0 and the
        last frame (in clip frame). The addon's delta-anchored apply
        must add that delta on top of the bone's bind position.
        """
        obj = self._make_object("Wing_Rotator_Top_Right", _TOP_RIGHT_BIND)
        channel = {
            "rotation": [[0.999, 0.0, -0.044, 0.0], [0.866, 0.0, -0.500, 0.0]],
            "position": [_TOP_RIGHT_POS_FIRST, _TOP_RIGHT_POS_LAST],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(_TOP_RIGHT_BIND),
            channel,
            frame_index=1,
            endpoint_policy="literal",
        )

        clip_dx = _TOP_RIGHT_POS_LAST[0] - _TOP_RIGHT_POS_FIRST[0]
        clip_dy = _TOP_RIGHT_POS_LAST[1] - _TOP_RIGHT_POS_FIRST[1]
        clip_dz = _TOP_RIGHT_POS_LAST[2] - _TOP_RIGHT_POS_FIRST[2]
        expected = (
            _TOP_RIGHT_BIND[0] + clip_dx,
            _TOP_RIGHT_BIND[1] + clip_dy,
            _TOP_RIGHT_BIND[2] + clip_dz,
        )
        for axis, (got, want) in enumerate(zip(obj.location, expected)):
            self.assertAlmostEqual(got, want, places=5, msg=f"axis {axis}")

    def test_override_blend_mode_uses_sample_verbatim(self) -> None:
        """Phase 38 override path. When the per-bone `blend_mode` is
        marked as `override` (CHR-bind sits outside the AABB of CAF
        position samples), the addon must use the sampled position
        verbatim and ignore the bind. The canonical real-world case
        is Scorpius `BONE_Front_Landing_Gear_Foot`, whose CHR-bind is
        ~1.81m off any clip sample. With the additive pathway that
        bone lands far off the gear; with the override pathway it
        lands at the sample exactly.
        """
        bind = (10.0, 0.0, 0.0)
        sample = [3.5, 1.25, -0.75]
        obj = self._make_object("BONE_Front_Landing_Gear_Foot", bind)
        channel = {
            "rotation": [[1.0, 0.0, 0.0, 0.0]],
            "position": [sample],
            "blend_mode": "override",
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(bind),
            channel,
            frame_index=0,
            endpoint_policy="transition_end",
        )
        for axis, (got, want) in enumerate(zip(obj.location, sample)):
            self.assertAlmostEqual(
                got, want, places=5,
                msg=f"axis {axis}: override mode must use sample verbatim",
            )

    # ---- Phase 39: layered-Action helpers ---------------------------

    def test_action_fcurves_handles_legacy_action(self) -> None:
        """Pre-Blender-4.4 Actions expose `Action.fcurves` directly.
        The helper must return that collection unchanged.
        """
        legacy_fcurves = ["fc_loc_x", "fc_loc_y", "fc_loc_z", "fc_quat_w"]
        action = types.SimpleNamespace(fcurves=legacy_fcurves)
        result = self.package_ops._action_fcurves(action)
        self.assertEqual(result, legacy_fcurves)

    def test_action_fcurves_walks_layered_channelbag(self) -> None:
        """Blender 5.1 Actions have no `Action.fcurves`; fcurves live on
        `action.layers[*].strips[*].channelbag(slot).fcurves`. The helper
        must enumerate them via the layered API.
        """
        slot = object()
        cb_fcurves = ["fc_loc_x", "fc_loc_y", "fc_quat_w"]
        channelbag = types.SimpleNamespace(fcurves=cb_fcurves)
        strip = types.SimpleNamespace(
            channelbag=lambda s, ensure=False: channelbag if s is slot else None
        )
        layer = types.SimpleNamespace(strips=[strip])
        action = types.SimpleNamespace(layers=[layer], slots=[slot])
        # Make sure attempting to access `.fcurves` does NOT yield the
        # legacy attribute.
        self.assertFalse(hasattr(action, "fcurves"))
        result = self.package_ops._action_fcurves(action)
        self.assertEqual(result, cb_fcurves)

    def test_action_groups_collection_returns_layered_channelbag_groups(self) -> None:
        """The Phase 39 regression: `Action.groups` is removed in Blender
        5.1 and grouping must come from the layered channelbag instead.
        Looking up `action.groups` directly used to abort the Insert
        Action loop after the first bone; the helper must transparently
        return the channelbag's groups collection.
        """

        class _Groups:
            def __init__(self) -> None:
                self._items: dict[str, object] = {}

            def get(self, name: str) -> object | None:
                return self._items.get(name)

            def new(self, name: str) -> object:
                grp = object()
                self._items[name] = grp
                return grp

        cb_groups = _Groups()
        slot = object()
        channelbag = types.SimpleNamespace(groups=cb_groups)
        strip = types.SimpleNamespace(
            channelbag=lambda s, ensure=False: channelbag if s is slot else None
        )
        layer = types.SimpleNamespace(strips=[strip])
        action = types.SimpleNamespace(layers=[layer], slots=[slot])
        self.assertFalse(hasattr(action, "groups"))
        result = self.package_ops._action_groups_collection(action)
        self.assertIs(result, cb_groups)
        # The helper must support the regular Insert-Action call sequence:
        # caller does `groups.get(name)` → falsy → `groups.new(name)`.
        self.assertIsNone(result.get("BoneA"))
        new_group = result.new("BoneA")
        self.assertIs(result.get("BoneA"), new_group)

    def test_action_groups_collection_returns_none_when_no_data(self) -> None:
        """A freshly-created layered Action with no keyframes inserted
        yet has no slots/channelbags. The helper must return None
        instead of raising — callers then skip grouping silently.
        """
        action = types.SimpleNamespace(layers=[], slots=[])
        self.assertIsNone(self.package_ops._action_groups_collection(action))

    def test_endpoint_policy_literal_picks_first_at_frame_zero(self) -> None:
        obj = self._make_object("Wing_Rotator_Top_Right", _TOP_RIGHT_BIND)
        channel = {
            "rotation": [[0.999, 0.0, -0.044, 0.0], [0.866, 0.0, -0.500, 0.0]],
            "position": [_TOP_RIGHT_POS_FIRST, _TOP_RIGHT_POS_LAST],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data(_TOP_RIGHT_BIND),
            channel,
            frame_index=0,
            endpoint_policy="literal",
        )
        # At frame 0 the literal policy picks pos[0]. With closed_sample
        # also = pos[0] (or any frame nearest bind), delta is zero, so
        # location should be at bind.
        for axis, (got, want) in enumerate(zip(obj.location, _TOP_RIGHT_BIND)):
            self.assertAlmostEqual(got, want, places=5, msg=f"axis {axis}")
        # Rotation at frame 0 is bind-equivalent (~5° tilt).
        self.assertAlmostEqual(obj.rotation_quaternion[0], 0.999, places=3)

    def test_rotation_sample_writes_quaternion_unchanged(self) -> None:
        obj = self._make_object("Wing_Mechanism_Top_Left", (0.0, 0.0, 0.0))
        channel = {
            "rotation": [[1.0, 0.0, 0.0, 0.0], [0.985, 0.174, 0.0, 0.0]],
            "position": [],
        }
        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=1,
            endpoint_policy="literal",
        )
        self.assertEqual(obj.rotation_mode, "QUATERNION")
        for got, want in zip(obj.rotation_quaternion, (0.985, 0.174, 0.0, 0.0)):
            self.assertAlmostEqual(got, want, places=5)

    def test_fragment_tagged_cyclic_clip_targets_mid_transition_time(self) -> None:
        clip = {
            "fragments": [{"frag_tags": ["Open"], "animations": [{"name": "canopy_open"}]}],
            "bones": {
                "0x00000001": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
                    "position_time": [0.0, 36.5, 75.0],
                },
                "0x00000002": {
                    "position": [[1.0, 0.0, 0.0], [1.0, -3.0, 0.0], [1.0, -0.1, 0.0]],
                    "position_time": [0.0, 36.5, 75.0],
                },
            },
        }

        self.assertEqual(self.package_ops._clip_cyclic_transition_target_frame(clip), 36.5)

    def test_cyclic_target_requires_source_fragment_metadata(self) -> None:
        clip = {
            "bones": {
                "0x00000001": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
                    "position_time": [0.0, 36.5, 75.0],
                }
            }
        }

        self.assertIsNone(self.package_ops._clip_cyclic_transition_target_frame(clip))

    def test_mixed_cyclic_clip_keeps_literal_endpoint(self) -> None:
        clip = {
            "fragments": [{"frag_tags": ["Deploy"], "animations": [{"name": "landing_gear_extend"}]}],
            "bones": {
                "0x00000001": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
                    "position_time": [0.0, 50.0, 100.0],
                },
                "0x00000002": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0]],
                    "position_time": [0.0, 50.0, 100.0],
                },
                "0x00000003": {
                    "position": [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0]],
                    "position_time": [0.0, 50.0, 100.0],
                },
            },
        }

        self.assertIsNone(self.package_ops._clip_cyclic_transition_target_frame(clip))

    def test_fragment_endpoint_policy_maps_state_tags_to_transition(self) -> None:
        deploy = {
            "fragment": "Landing_Gear",
            "frag_tags": ["Deploy"],
            "animations": [{"name": "landing_gear_extend"}],
        }
        retract = {
            "fragment": "Landing_Gear",
            "frag_tags": ["Retract"],
            "animations": [{"name": "landing_gear_extend", "speed": -1}],
        }

        # Forward fragment (Deploy): snap_first -> start, snap_last -> end.
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(deploy, "snap_first"),
            "transition_start",
        )
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(deploy, "snap_last"),
            "transition_end",
        )
        # Reverse-playback fragment (Retract, speed=-1): mapping flips.
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(retract, "snap_first"),
            "transition_end",
        )
        self.assertEqual(
            self.package_ops._fragment_endpoint_policy(retract, "snap_last"),
            "transition_start",
        )

    def test_target_frame_snap_opens_from_bind_anchored_start(self) -> None:
        obj = self._make_object("Canopy_Front", (0.0, 0.0, 0.0))
        channel = {
            "position": [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.1, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=-1,
            endpoint_policy="literal",
            target_frame=36.5,
            anchor_frame=36.5,
        )

        self.assertEqual(obj.location, (0.0, 2.0, 0.0))

    def test_target_frame_snap_closes_to_bind_when_target_is_reference(self) -> None:
        obj = self._make_object("Canopy_Front", (0.0, 0.0, 0.0))
        channel = {
            "position": [[0.0, 2.0, 0.0], [0.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=-1,
            endpoint_policy="literal",
            target_frame=36.5,
            anchor_frame=36.5,
        )

        self.assertEqual(obj.location, (0.0, 0.0, 0.0))

    def test_snap_first_can_use_target_frame_as_anchor_reference(self) -> None:
        obj = self._make_object("Canopy_Front", (0.0, 0.0, 0.0))
        channel = {
            "position": [[0.0, 2.0, 0.0], [0.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            "position_time": [0.0, 36.5, 75.0],
        }

        self.package_ops._apply_best_channel_transform(
            obj,
            self._bind_data((0.0, 0.0, 0.0)),
            channel,
            frame_index=0,
            endpoint_policy="literal",
            anchor_frame=36.5,
        )

        self.assertEqual(obj.location, (0.0, 2.0, 0.0))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
