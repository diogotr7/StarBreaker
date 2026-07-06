#!/usr/bin/env python3
"""Blocker-evidence validator (plan A5).

A parity "this can't be reproduced from data" (blocker) claim presented at the
major-item gate MUST attach an evidence file proving the in-data sources were
actually exhausted first (the recurring lesson: proven blockers kept dissolving
once someone searched the right family — compass ticks, master-mode size). This
validates that file: every required surface — datacore_families, p4k_assets,
localization, derivable_mechanisms — must carry >= 1 {query, result} pair with
non-empty strings, and `conclusion` must be `blocked` or `found`.

Schema + a worked example: crates/starbreaker-ui/docs/blocker-evidence-schema.md.

RESULT MARKER via atexit (registered in __main__ only, so importing this module
has no side effect): `ui_blocker_evidence: OK ...` / `ui_blocker_evidence:
FAILED (...)`. Never rely on a piped exit code.

Usage:
  python3 scripts/ui_blocker_evidence.py <evidence.json>
  python3 scripts/ui_blocker_evidence.py --self-test
"""
import atexit
import json
import sys

REQUIRED_SURFACES = ("datacore_families", "p4k_assets", "localization", "derivable_mechanisms")
VALID_CONCLUSIONS = ("blocked", "found")

_STATE = {"marker": "ui_blocker_evidence: FAILED (did not complete)"}


def _emit_marker():
    print(_STATE["marker"])


def _nonempty_str(v):
    return isinstance(v, str) and bool(v.strip())


def validate(evidence):
    """-> list of error strings (empty list == valid)."""
    if not isinstance(evidence, dict):
        return ["evidence must be a JSON object"]
    errors = []
    if not _nonempty_str(evidence.get("item")):
        errors.append("missing non-empty 'item'")
    if evidence.get("conclusion") not in VALID_CONCLUSIONS:
        errors.append(f"'conclusion' must be one of {list(VALID_CONCLUSIONS)}, got {evidence.get('conclusion')!r}")
    surfaces = evidence.get("surfaces")
    if not isinstance(surfaces, dict):
        return errors + ["missing 'surfaces' object"]
    for surface in REQUIRED_SURFACES:
        pairs = surfaces.get(surface)
        if not isinstance(pairs, list) or not pairs:
            errors.append(f"surface '{surface}': need >= 1 {{query, result}} pair")
            continue
        for i, pair in enumerate(pairs):
            if not isinstance(pair, dict):
                errors.append(f"surface '{surface}'[{i}]: not an object")
                continue
            if not _nonempty_str(pair.get("query")):
                errors.append(f"surface '{surface}'[{i}]: empty/missing 'query'")
            if not _nonempty_str(pair.get("result")):
                errors.append(f"surface '{surface}'[{i}]: empty/missing 'result'")
    return errors


def run_self_test():
    good = {
        "item": "compass live ticks",
        "surfaces": {s: [{"query": f"query {s}", "result": f"result {s}"}] for s in REQUIRED_SURFACES},
        "conclusion": "blocked",
    }
    assert validate(good) == [], f"well-formed rejected: {validate(good)}"

    # missing a required surface -> reject
    missing = json.loads(json.dumps(good))
    del missing["surfaces"]["p4k_assets"]
    assert validate(missing), "missing surface accepted"

    # empty result string -> reject
    empty = json.loads(json.dumps(good))
    empty["surfaces"]["localization"][0]["result"] = "   "
    assert validate(empty), "empty result accepted"

    # empty surface list -> reject
    empty_list = json.loads(json.dumps(good))
    empty_list["surfaces"]["datacore_families"] = []
    assert validate(empty_list), "empty surface list accepted"

    # bad conclusion -> reject
    bad_conc = json.loads(json.dumps(good))
    bad_conc["conclusion"] = "maybe"
    assert validate(bad_conc), "bad conclusion accepted"

    _STATE["marker"] = "ui_blocker_evidence: OK (self-test)"
    return 0


def main(argv):
    if "--self-test" in argv:
        return run_self_test()
    if not argv:
        _STATE["marker"] = "ui_blocker_evidence: FAILED (usage: <evidence.json> | --self-test)"
        return 2
    with open(argv[0], encoding="utf-8") as f:
        evidence = json.load(f)
    errors = validate(evidence)
    if errors:
        for e in errors:
            print(f"  error: {e}")
        _STATE["marker"] = f"ui_blocker_evidence: FAILED ({len(errors)} errors)"
        return 1
    _STATE["marker"] = f"ui_blocker_evidence: OK ({evidence['item']!r} — {evidence['conclusion']})"
    return 0


if __name__ == "__main__":
    atexit.register(_emit_marker)   # script-only: no import side effect
    try:
        rc = main(sys.argv[1:])
    except Exception as e:
        _STATE["marker"] = f"ui_blocker_evidence: FAILED ({type(e).__name__}: {e})"
        rc = 1
    sys.exit(rc)
