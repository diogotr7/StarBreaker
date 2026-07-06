#!/usr/bin/env python3
"""Drift validator for the machine-readable UI screen dossier.

Cross-checks ``crates/starbreaker-ui/data/ui_screen_dossier_v1.json`` against
the authoritative markdown dossier table in
``crates/starbreaker-ui/docs/ui-reference.md`` §3 so the two representations
cannot silently diverge (registry pattern — provenance + field-extraction
contract in the sidecar ``ui_screen_dossier_v1.notes.md``).

Emits a RESULT MARKER on every exit path (``validate_ui_dossier: OK (N screens)``
/ ``validate_ui_dossier: FAILED (...)``) via an atexit handler — never rely on
a piped exit code (ui_check.sh convention).

Usage:
  python3 scripts/validate_ui_dossier.py             # validate the real files
  python3 scripts/validate_ui_dossier.py --self-test # hermetic self-check
"""
import atexit
import json
import os
import re
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOSSIER = os.path.join(REPO_ROOT, "crates/starbreaker-ui/data/ui_screen_dossier_v1.json")
UI_REFERENCE = os.path.join(REPO_ROOT, "crates/starbreaker-ui/docs/ui-reference.md")
PACKAGES_ROOT = os.path.expanduser("~/projects/scorg_tools/ships/Packages")

REQUIRED_KEYS = {
    "screen_id", "ship_folder", "reference_file", "scene_package", "helper",
    "lod", "canvas", "preset", "tier", "target_id", "open_issues",
}
VALID_TIERS = {"PLATINUM", "GOLD", None}
# Fields re-derived from §3 and compared JSON<->table (must not drift).
CROSSCHECK_FIELDS = ("screen_id", "helper", "preset", "tier", "target_id")

_STATE = {"marker": "validate_ui_dossier: FAILED (did not complete)"}
_BACKTICK = re.compile(r"`([^`]+)`")


@atexit.register
def _emit_marker():
    print(_STATE["marker"])


def first_backtick(cell):
    m = _BACKTICK.search(cell)
    return m.group(1) if m else None


def parse_table_rows(md_text):
    """Parse ui-reference §3's dossier table. Returns a list of dicts with the
    cross-check fields, or None if the table can't be located."""
    lines = md_text.splitlines()
    hdr = next(
        (i for i, l in enumerate(lines)
         if l.lstrip().startswith("|") and "Helper / scene" in l),
        None,
    )
    if hdr is None:
        return None
    i = hdr + 1
    if i < len(lines) and set(lines[i].strip()) <= set("-:| "):  # separator row
        i += 1
    rows = []
    while i < len(lines) and lines[i].lstrip().startswith("|"):
        cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
        if len(cells) >= 6:  # a stray '|' in the last (Open issues) column only adds trailing cells
            helper_bt = first_backtick(cells[1])
            canvas = first_backtick(cells[2])
            tier_cell = cells[5].upper()
            rows.append({
                "screen_id": helper_bt if helper_bt else canvas,
                "helper": helper_bt if helper_bt else cells[1].split("/")[0].strip(),
                "preset": first_backtick(cells[4]),
                "tier": "PLATINUM" if "PLATINUM" in tier_cell else ("GOLD" if "GOLD" in tier_cell else None),
                "target_id": first_backtick(cells[5]),
            })
        i += 1
    return rows


def validate(dossier, md_text, packages_root):
    """(dossier dict, ui-reference markdown, ships Packages root) -> (errors, warnings)."""
    errors, warnings = [], []

    if dossier.get("version") != 1:
        errors.append(f"version must be 1, got {dossier.get('version')!r}")
    screens = dossier.get("screens")
    if not isinstance(screens, list):
        errors.append("'screens' must be a list")
        return errors, warnings

    for s in screens:
        sid = s.get("screen_id", "<no screen_id>")
        missing = REQUIRED_KEYS - set(s)
        if missing:
            errors.append(f"{sid}: missing keys {sorted(missing)}")
        if s.get("lod") not in (0, 1):
            errors.append(f"{sid}: lod must be 0 or 1, got {s.get('lod')!r}")
        if s.get("tier") not in VALID_TIERS:
            errors.append(f"{sid}: tier must be PLATINUM/GOLD/null, got {s.get('tier')!r}")

    table = parse_table_rows(md_text)
    if table is None:
        errors.append("ui-reference §3 dossier table not found (no 'Helper / scene' header)")
        return errors, warnings

    json_ids = {s.get("screen_id") for s in screens}
    table_ids = {r["screen_id"] for r in table}
    for sid in sorted(json_ids - table_ids, key=str):
        errors.append(f"{sid}: in JSON but not in §3 table")
    for sid in sorted(table_ids - json_ids, key=str):
        errors.append(f"{sid}: in §3 table but not in JSON")

    table_by_id = {r["screen_id"]: r for r in table}
    for s in screens:
        r = table_by_id.get(s.get("screen_id"))
        if not r:
            continue
        for f in CROSSCHECK_FIELDS:
            if s.get(f) != r.get(f):
                errors.append(f"{s['screen_id']}: {f} drift — JSON {s.get(f)!r} vs §3 {r.get(f)!r}")

    if packages_root and os.path.isdir(packages_root):
        for s in screens:
            pkg = s.get("scene_package")
            if pkg and not os.path.isdir(os.path.join(packages_root, pkg)):
                warnings.append(f"{s.get('screen_id')}: scene_package not exported: {pkg}")
    elif packages_root:
        warnings.append(f"ships Packages root absent ({packages_root}) — package existence unchecked (no game data)")

    return errors, warnings


def run_self_test():
    good_dossier = {
        "version": 1,
        "screens": [{
            "screen_id": "Screen_Demo", "ship_folder": "Clipper",
            "reference_file": "Screen_Demo.png", "scene_package": "PKG_LOD0",
            "helper": "Screen_Demo", "lod": 0, "canvas": "MC_S_Demo",
            "preset": "demo", "tier": "GOLD", "target_id": "demo_master",
            "open_issues": "none",
        }],
    }
    good_md = (
        "| Screen | Helper / scene | Canvas | Reference image | Preset | Tier / target id | Open issues |\n"
        "|---|---|---|---|---|---|---|\n"
        "| Demo | `Screen_Demo` / LOD0 scene | `MC_S_Demo` | `Screen_Demo.png` | `demo` | GOLD `demo_master` | none |\n"
    )
    errors, _ = validate(good_dossier, good_md, None)
    assert not errors, f"well-formed dossier rejected: {errors}"

    # bad tier (schema) + the mismatch drifts vs the table -> must reject
    bad_dossier = {"version": 1, "screens": [dict(good_dossier["screens"][0], tier="SILVER")]}
    errors, _ = validate(bad_dossier, good_md, None)
    assert errors, "malformed dossier (bad tier + table drift) accepted"

    # a JSON screen absent from the table must be caught as drift
    extra = {"version": 1, "screens": good_dossier["screens"] + [
        dict(good_dossier["screens"][0], screen_id="Screen_Ghost", helper="Screen_Ghost")]}
    errors, _ = validate(extra, good_md, None)
    assert any("Ghost" in e for e in errors), "extra JSON screen not flagged as drift"

    _STATE["marker"] = "validate_ui_dossier: OK (self-test)"
    return 0


def main(argv):
    if "--self-test" in argv:
        return run_self_test()
    with open(DOSSIER, encoding="utf-8") as f:
        dossier = json.load(f)
    with open(UI_REFERENCE, encoding="utf-8") as f:
        md_text = f.read()
    errors, warnings = validate(dossier, md_text, PACKAGES_ROOT)
    for w in warnings:
        print(f"  warn: {w}")
    if errors:
        for e in errors:
            print(f"  error: {e}")
        _STATE["marker"] = f"validate_ui_dossier: FAILED ({len(errors)} errors)"
        return 1
    _STATE["marker"] = f"validate_ui_dossier: OK ({len(dossier['screens'])} screens)"
    return 0


if __name__ == "__main__":
    try:
        rc = main(sys.argv[1:])
    except Exception as e:  # guarantee a clean FAILED marker on any error path
        _STATE["marker"] = f"validate_ui_dossier: FAILED ({type(e).__name__}: {e})"
        rc = 1
    sys.exit(rc)
