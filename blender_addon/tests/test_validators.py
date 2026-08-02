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

if "bpy" not in sys.modules:
    bpy = types.ModuleType("bpy")
    bpy.types = types.SimpleNamespace(Material=object)
    bpy.data = types.SimpleNamespace(node_groups=[], images=[], materials=[], actions=[])
    sys.modules["bpy"] = bpy


import bpy  # noqa: E402

from starbreaker_addon.runtime.validators import (  # noqa: E402
    _purge_orphaned_file_backed_images,
    _purge_orphaned_runtime_actions,
)


class FakeImage:
    def __init__(self, name: str, *, users: int, filepath: str, **props: object) -> None:
        self.name = name
        self.users = users
        self.filepath = filepath
        self.props = dict(props)

    def get(self, key, default=None):
        return self.props.get(key, default)


class FakeAction:
    def __init__(self, name: str, users: int) -> None:
        self.name = name
        self.users = users


class OrphanPurgeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.removed: list[object] = []
        bpy.data.batch_remove = lambda ids: self.removed.extend(ids)

    def test_only_addon_stamped_images_are_purged(self) -> None:
        addon_image = FakeImage(
            "ship_diff.dds", users=0, filepath="//ship_diff.dds",
            starbreaker_source_mtime_ns="123",
        )
        gobo_image = FakeImage(
            "cookie.dds", users=0, filepath="//cookie.dds",
            starbreaker_gobo_mean_luminance=0.5,
        )
        user_image = FakeImage("my_reference.png", users=0, filepath="//my_reference.png")
        in_use = FakeImage(
            "used.dds", users=2, filepath="//used.dds", starbreaker_source_mtime_ns="9",
        )
        bpy.data.images = [addon_image, gobo_image, user_image, in_use]

        self.assertEqual(_purge_orphaned_file_backed_images(), 2)
        self.assertEqual(self.removed, [addon_image, gobo_image])

    def test_purge_is_a_single_batch_remove(self) -> None:
        calls: list[int] = []
        bpy.data.batch_remove = lambda ids: calls.append(len(ids))
        bpy.data.actions = [
            FakeAction("HeightMapAction", 0),
            FakeAction("HeightMapAction.001", 0),
            FakeAction("WalkCycle", 0),
        ]
        self.assertEqual(_purge_orphaned_runtime_actions(), 2)
        self.assertEqual(calls, [2])


if __name__ == "__main__":
    unittest.main()
