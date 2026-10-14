"""Phase 63 — Addon preference gate unit tests.

Tests for the pure helper functions that control post-open behaviour.
These tests do not require bpy and run in a plain Python environment.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ADDON_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ADDON_ROOT))


def _load_preference_gates() -> tuple:
    """Extract pure preference gate functions from ui.py via AST, no bpy needed."""
    import ast as ast_mod

    ui_path = ADDON_ROOT / "starbreaker_addon" / "ui.py"
    source = ui_path.read_text()
    tree = ast_mod.parse(source)

    namespace: dict = {"PROP_PACKAGE_ROOT": "starbreaker_package_root"}
    for node in ast_mod.walk(tree):
        if isinstance(node, ast_mod.FunctionDef) and node.name in (
            "_should_auto_refresh_unloaded_materials_on_load",
            "_should_auto_refresh_package_root",
        ):
            func_source = ast_mod.get_source_segment(source, node)
            if func_source:
                exec(compile(ast_mod.parse(func_source), str(ui_path), "exec"), namespace)  # noqa: S102

    return (
        namespace["_should_auto_refresh_unloaded_materials_on_load"],
        namespace["_should_auto_refresh_package_root"],
    )


(
    _should_auto_refresh_unloaded_materials_on_load,
    _should_auto_refresh_package_root,
) = _load_preference_gates()


class _MockPrefs:
    """Simple stand-in for STARBREAKER_AP_preferences."""

    def __init__(
        self,
        auto_refresh_unloaded_materials_on_load: bool = True,
    ) -> None:
        self.auto_refresh_unloaded_materials_on_load = auto_refresh_unloaded_materials_on_load


class _MockObject(dict):
    pass


class TestPreferenceGates(unittest.TestCase):
    # ── _should_auto_refresh_unloaded_materials_on_load ─────────────────────

    def test_auto_refresh_on_load_default_true_when_prefs_none(self) -> None:
        self.assertTrue(_should_auto_refresh_unloaded_materials_on_load(None))

    def test_auto_refresh_on_load_true_when_pref_enabled(self) -> None:
        self.assertTrue(
            _should_auto_refresh_unloaded_materials_on_load(
                _MockPrefs(auto_refresh_unloaded_materials_on_load=True)
            )
        )

    def test_auto_refresh_on_load_false_when_pref_disabled(self) -> None:
        self.assertFalse(
            _should_auto_refresh_unloaded_materials_on_load(
                _MockPrefs(auto_refresh_unloaded_materials_on_load=False)
            )
        )

    def test_auto_refresh_on_load_true_when_attr_missing(self) -> None:
        self.assertTrue(_should_auto_refresh_unloaded_materials_on_load(object()))

    def test_auto_refresh_requires_package_root_and_refresh_need(self) -> None:
        package_root = _MockObject(starbreaker_package_root=True)
        plain_object = _MockObject()

        self.assertTrue(_should_auto_refresh_package_root(package_root, lambda _obj: True))
        self.assertFalse(_should_auto_refresh_package_root(package_root, lambda _obj: False))
        self.assertFalse(_should_auto_refresh_package_root(plain_object, lambda _obj: True))


if __name__ == "__main__":
    unittest.main()
